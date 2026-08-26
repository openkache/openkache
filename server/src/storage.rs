use std::collections::VecDeque;
use std::io;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use compio::buf::IoBuf;
use compio::driver::ProactorBuilder;
use compio::fs::{File, OpenOptions};
use compio::io::{AsyncReadAtExt, AsyncWriteAtExt};
use compio::runtime::Runtime;

use crate::spsc::{Consumer, Producer};
use crate::storage_message::{
    Command, Reply, STORAGE_QUEUE_SLOTS, StorageRequest, StorageResponse,
};

pub(crate) mod bucket;
pub(crate) mod sg;
pub(crate) mod table;

use bucket::{BUCKET_BYTES, Bucket, BucketValue};
use sg::MutableSg;
use table::{Table, TableConfig, TableCreateError};

const STORAGE_KEY_BYTES: usize = 32;
const MUTABLE_SG_COUNT: usize = 3;
const BUCKETS_PER_SG: usize = 65_536;
const BUCKET_CHOICE_COUNT: u8 = 4;
const BUCKET_CHOICE_BITS: u32 = BUCKET_CHOICE_COUNT.ilog2();
const STORAGE_SG_COUNT: usize = 64;
const TABLE_VALUE_BITS: u8 = 8;
const TABLE_MAX_ENTRIES: usize = 625_000;
const STORAGE_IO_QUEUE_ENTRIES: u32 = 4_096;
const STORAGE_FILE_PATH: &str = "openkache.data";
const SG_BYTES: u64 = (BUCKETS_PER_SG * BUCKET_BYTES) as u64;
const STORAGE_FILE_BYTES: u64 = STORAGE_SG_COUNT as u64 * SG_BYTES;

/// A fixed-size key used by Storage and Bucket.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StorageKey([u8; STORAGE_KEY_BYTES]);

impl StorageKey {
    pub(crate) fn from_key(key: &[u8]) -> Self {
        Self(*blake3::hash(key).as_bytes())
    }

    pub(crate) const fn as_bytes(&self) -> &[u8; STORAGE_KEY_BYTES] {
        &self.0
    }

    pub(crate) fn table_hash(&self) -> u128 {
        u128::from_le_bytes(self.0[8..24].try_into().unwrap())
    }
}

struct Storage {
    table: Table,
    sgs: Box<[SgState]>,
    oldest_mutable_sg_index: usize,
    spare_mutable_sg: Option<MutableSg>,
}

enum SgState {
    Unused,
    Mutable(MutableSg),
    Flushing(FlushBuffer),
    Ssd,
}

#[derive(Clone)]
struct FlushBuffer(Rc<MutableSg>);

impl IoBuf for FlushBuffer {
    fn as_init(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

struct FlushJob {
    sg_index: usize,
    buffer: FlushBuffer,
}

struct SetPlan {
    candidates: Box<[u32]>,
}

struct SetObservation {
    previous: Option<u32>,
}

struct DeleteObservation {
    previous: Option<u32>,
    existed: bool,
}

enum Lookup {
    Value(Arc<[u8]>),
    Tombstone,
    CandidateMiss,
    ReadBucket { file_offset: u64 },
}

enum CommitSet {
    Retry,
    Finished {
        result: Result<(), SetError>,
        flush: Option<FlushJob>,
    },
}

enum CommitDelete {
    Retry,
    Finished(bool),
}

#[derive(Debug)]
enum SetError {
    ValueTooLarge,
    TableFull,
    FlushStillInFlight,
    SsdCapacityReached,
    TableLocationMissing,
}

impl Storage {
    fn new() -> Result<Self, TableCreateError> {
        let table = Table::new(TableConfig {
            max_entries: TABLE_MAX_ENTRIES,
            value_bits: TABLE_VALUE_BITS,
            fingerprint_bits: 8,
        })?;
        let mut sgs = (0..STORAGE_SG_COUNT)
            .map(|_| SgState::Unused)
            .collect::<Vec<_>>()
            .into_boxed_slice();

        for state in &mut sgs[..MUTABLE_SG_COUNT] {
            *state = SgState::Mutable(Self::new_mutable_sg());
        }

        Ok(Self {
            table,
            sgs,
            oldest_mutable_sg_index: 0,
            spare_mutable_sg: Some(Self::new_mutable_sg()),
        })
    }

    fn new_mutable_sg() -> MutableSg {
        MutableSg::new(BUCKETS_PER_SG, BUCKET_CHOICE_COUNT)
    }

    fn prepare_set(&self, key: &StorageKey) -> SetPlan {
        SetPlan {
            candidates: self
                .table
                .values(key.table_hash())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }

    fn candidates(&self, key: &StorageKey) -> Box<[u32]> {
        self.table
            .values(key.table_hash())
            .collect::<Vec<_>>()
            .into_boxed_slice()
    }

    fn lookup(&self, key: &StorageKey, table_value: u32) -> Lookup {
        let (sg_index, bucket_choice) = Self::decode_table_value(table_value);
        let Some(state) = self.sgs.get(sg_index) else {
            return Lookup::CandidateMiss;
        };
        if bucket_choice >= BUCKET_CHOICE_COUNT {
            return Lookup::CandidateMiss;
        }

        match state {
            SgState::Mutable(sg) => Self::lookup_ram(sg, key, bucket_choice),
            SgState::Flushing(buffer) => Self::lookup_ram(&buffer.0, key, bucket_choice),
            SgState::Ssd => Lookup::ReadBucket {
                file_offset: Self::bucket_file_offset(sg_index, key, bucket_choice),
            },
            SgState::Unused => Lookup::CandidateMiss,
        }
    }

    fn lookup_ram(sg: &MutableSg, key: &StorageKey, bucket_choice: u8) -> Lookup {
        match sg.get(key, bucket_choice) {
            Some(BucketValue::Value(value)) => Lookup::Value(Arc::from(value)),
            Some(BucketValue::Tombstone) => Lookup::Tombstone,
            None => Lookup::CandidateMiss,
        }
    }

    /// An await-free section that conditionally updates the observed location and flushes the oldest SG when needed.
    fn commit_set(
        &mut self,
        key: &StorageKey,
        value: &[u8],
        plan: SetPlan,
        observation: SetObservation,
    ) -> CommitSet {
        if !Bucket::new().can_append(BucketValue::Value(value)) {
            return CommitSet::Finished {
                result: Err(SetError::ValueTooLarge),
                flush: None,
            };
        }

        if let Some(previous) = observation.previous {
            if !self
                .table
                .values(key.table_hash())
                .any(|candidate| candidate == previous)
            {
                return CommitSet::Retry;
            }

            let (previous_sg_index, previous_bucket_choice) = Self::decode_table_value(previous);
            if let Some(SgState::Mutable(sg)) = self.sgs.get_mut(previous_sg_index) {
                if sg.replace(key, previous_bucket_choice, BucketValue::Value(value)) {
                    return CommitSet::Finished {
                        result: Ok(()),
                        flush: None,
                    };
                }
            }
        } else {
            let current_candidates = self.table.values(key.table_hash()).collect::<Vec<_>>();
            if current_candidates.as_slice() != plan.candidates.as_ref() {
                return CommitSet::Retry;
            }
        }

        let mut flush = None;
        let replacement =
            match self.append_to_mutable_sg(key, BucketValue::Value(value), observation.previous) {
                Some(replacement) => replacement,
                None => {
                    let (flush_job, new_mutable_sg_index) = match self.rotate_mutable() {
                        Ok(rotation) => rotation,
                        Err(error) => {
                            return CommitSet::Finished {
                                result: Err(error),
                                flush: None,
                            };
                        }
                    };
                    let SgState::Mutable(new_mutable) = &mut self.sgs[new_mutable_sg_index] else {
                        unreachable!("rotate_mutable must open a Mutable SG");
                    };
                    let bucket_choice = new_mutable
                        .insert(key, BucketValue::Value(value))
                        .expect("a value checked against an empty Bucket must fit");
                    flush = Some(flush_job);
                    Self::encode_table_value(new_mutable_sg_index, bucket_choice)
                }
            };

        let table_updated = match observation.previous {
            Some(previous) => self.table.replace(key.table_hash(), previous, replacement),
            None => self.table.insert(key.table_hash(), replacement).is_ok(),
        };

        if !table_updated {
            self.rollback_append(replacement, key);
            return CommitSet::Finished {
                result: Err(if observation.previous.is_some() {
                    SetError::TableLocationMissing
                } else {
                    SetError::TableFull
                }),
                flush,
            };
        }

        if let Some(previous) = observation.previous {
            self.remove_from_mutable_sg(previous, key);
        }

        CommitSet::Finished {
            result: Ok(()),
            flush,
        }
    }

    /// Removes one entry only when its table location is unchanged from before the await.
    fn commit_delete(
        &mut self,
        key: &StorageKey,
        plan: SetPlan,
        observation: DeleteObservation,
    ) -> CommitDelete {
        if let Some(previous) = observation.previous {
            if !self.table.remove(key.table_hash(), previous) {
                return CommitDelete::Retry;
            }

            // Reclaim bytes from a Mutable SG so a later SET cannot conflict with
            // the stale value in the same bucket. SSD and Flushing SGs only lose the table entry.
            self.remove_from_mutable_sg(previous, key);
            return CommitDelete::Finished(observation.existed);
        }

        let current_candidates = self.table.values(key.table_hash()).collect::<Vec<_>>();
        if current_candidates.as_slice() != plan.candidates.as_ref() {
            return CommitDelete::Retry;
        }

        CommitDelete::Finished(false)
    }

    fn append_to_mutable_sg(
        &mut self,
        key: &StorageKey,
        value: BucketValue<'_>,
        previous: Option<u32>,
    ) -> Option<u32> {
        for mutable_offset in (0..MUTABLE_SG_COUNT).rev() {
            let sg_index = (self.oldest_mutable_sg_index + mutable_offset) % self.sgs.len();
            let SgState::Mutable(sg) = &mut self.sgs[sg_index] else {
                unreachable!("the three SGs after oldest must be Mutable");
            };
            let Some(bucket_choice) = sg.insert(key, value) else {
                continue;
            };

            let same_physical_bucket = previous.is_some_and(|previous| {
                let (previous_sg_index, previous_bucket_choice) =
                    Self::decode_table_value(previous);
                previous_sg_index == sg_index
                    && sg.bucket_index_for_choice(key, previous_bucket_choice)
                        == sg.bucket_index_for_choice(key, bucket_choice)
            });
            if same_physical_bucket {
                let removed = sg.remove(key, bucket_choice);
                debug_assert!(removed);
                continue;
            }

            return Some(Self::encode_table_value(sg_index, bucket_choice));
        }
        None
    }

    fn rotate_mutable(&mut self) -> Result<(FlushJob, usize), SetError> {
        let old_index = self.oldest_mutable_sg_index;
        let new_index = (old_index + MUTABLE_SG_COUNT) % self.sgs.len();

        if !matches!(self.sgs[new_index], SgState::Unused) {
            return Err(SetError::SsdCapacityReached);
        }
        let Some(new_mutable) = self.spare_mutable_sg.take() else {
            return Err(SetError::FlushStillInFlight);
        };
        let old_state = std::mem::replace(&mut self.sgs[old_index], SgState::Unused);
        let SgState::Mutable(old_mutable) = old_state else {
            unreachable!("oldest_mutable_sg_index must select a Mutable SG");
        };

        let buffer = FlushBuffer(Rc::new(old_mutable));
        self.sgs[old_index] = SgState::Flushing(buffer.clone());
        self.sgs[new_index] = SgState::Mutable(new_mutable);
        self.oldest_mutable_sg_index = (old_index + 1) % self.sgs.len();

        Ok((
            FlushJob {
                sg_index: old_index,
                buffer,
            },
            new_index,
        ))
    }

    fn complete_flush(
        &mut self,
        sg_index: usize,
        result: io::Result<()>,
        returned_buffer: FlushBuffer,
    ) -> io::Result<()> {
        result?;
        drop(returned_buffer);

        let old_state = std::mem::replace(&mut self.sgs[sg_index], SgState::Ssd);
        let SgState::Flushing(buffer) = old_state else {
            unreachable!("completed SG must be Flushing");
        };
        let mut reusable = Rc::try_unwrap(buffer.0)
            .unwrap_or_else(|_| panic!("Flushing state must be the only buffer owner"));
        reusable.clear();

        assert!(
            self.spare_mutable_sg.is_none(),
            "only one flush may be in flight"
        );
        self.spare_mutable_sg = Some(reusable);
        Ok(())
    }

    fn rollback_append(&mut self, table_value: u32, key: &StorageKey) {
        let removed = self.remove_from_mutable_sg(table_value, key);
        debug_assert!(removed);
    }

    fn remove_from_mutable_sg(&mut self, table_value: u32, key: &StorageKey) -> bool {
        let (sg_index, bucket_choice) = Self::decode_table_value(table_value);
        match self.sgs.get_mut(sg_index) {
            Some(SgState::Mutable(sg)) => sg.remove(key, bucket_choice),
            _ => false,
        }
    }

    fn encode_table_value(sg_index: usize, bucket_choice: u8) -> u32 {
        ((sg_index as u32) << BUCKET_CHOICE_BITS) | u32::from(bucket_choice)
    }

    fn decode_table_value(table_value: u32) -> (usize, u8) {
        let bucket_choice_mask = (1 << BUCKET_CHOICE_BITS) - 1;
        (
            (table_value >> BUCKET_CHOICE_BITS) as usize,
            (table_value & bucket_choice_mask) as u8,
        )
    }

    fn bucket_file_offset(sg_index: usize, key: &StorageKey, bucket_choice: u8) -> u64 {
        let key_bytes = key.as_bytes();
        let first = u64::from_le_bytes(key_bytes[16..24].try_into().unwrap());
        let second = u64::from_le_bytes(key_bytes[24..32].try_into().unwrap());
        let hash = match bucket_choice {
            0 => first,
            1 => second,
            choice => {
                first.wrapping_add(u64::from(choice).wrapping_mul(second.rotate_left(32) | 1))
            }
        };
        let bucket_index = hash as usize % BUCKETS_PER_SG;
        sg_index as u64 * SG_BYTES + bucket_index as u64 * BUCKET_BYTES as u64
    }
}

struct WorkerState {
    storage: Storage,
    completed: VecDeque<StorageResponse>,
    fatal_io_error: Option<io::Error>,
}

#[derive(Clone, Copy)]
struct WorkerHandle(NonNull<WorkerState>);

impl WorkerHandle {
    fn access<R>(self, operation: impl FnOnce(&mut WorkerState) -> R) -> R {
        // SAFETY: WorkerState remains in a pinned Box and is accessed only by its storage thread.
        unsafe { operation(&mut *self.0.as_ptr()) }
    }
}

async fn read_bucket(file: Rc<File>, file_offset: u64) -> io::Result<Box<Bucket>> {
    let (result, bucket) = file
        .read_exact_at(Box::new(Bucket::new()), file_offset)
        .await
        .into_parts();
    result?;
    Ok(bucket)
}

async fn execute_get(worker: WorkerHandle, file: Rc<File>, key: StorageKey) -> io::Result<Reply> {
    let candidates = worker.access(|worker| worker.storage.candidates(&key));

    for candidate in candidates {
        match worker.access(|worker| worker.storage.lookup(&key, candidate)) {
            Lookup::Value(value) => return Ok(Reply::Get(Some(value))),
            Lookup::Tombstone => return Ok(Reply::Get(None)),
            Lookup::CandidateMiss => {}
            Lookup::ReadBucket { file_offset } => {
                let bucket = read_bucket(Rc::clone(&file), file_offset).await?;
                match bucket.get(&key) {
                    Some(BucketValue::Value(value)) => {
                        return Ok(Reply::Get(Some(Arc::from(value))));
                    }
                    Some(BucketValue::Tombstone) => return Ok(Reply::Get(None)),
                    None => {}
                }
            }
        }
    }
    Ok(Reply::Get(None))
}

async fn resolve_set_plan(
    worker: WorkerHandle,
    file: Rc<File>,
    key: &StorageKey,
    plan: &SetPlan,
) -> io::Result<SetObservation> {
    for candidate in plan.candidates.iter().copied() {
        match worker.access(|worker| worker.storage.lookup(key, candidate)) {
            Lookup::Value(_) | Lookup::Tombstone => {
                return Ok(SetObservation {
                    previous: Some(candidate),
                });
            }
            Lookup::CandidateMiss => {}
            Lookup::ReadBucket { file_offset } => {
                let bucket = read_bucket(Rc::clone(&file), file_offset).await?;
                if bucket.get(key).is_some() {
                    return Ok(SetObservation {
                        previous: Some(candidate),
                    });
                }
            }
        }
    }
    Ok(SetObservation { previous: None })
}

async fn resolve_delete_plan(
    worker: WorkerHandle,
    file: Rc<File>,
    key: &StorageKey,
    plan: &SetPlan,
) -> io::Result<DeleteObservation> {
    for candidate in plan.candidates.iter().copied() {
        match worker.access(|worker| worker.storage.lookup(key, candidate)) {
            Lookup::Value(_) => {
                return Ok(DeleteObservation {
                    previous: Some(candidate),
                    existed: true,
                });
            }
            Lookup::Tombstone => {
                return Ok(DeleteObservation {
                    previous: Some(candidate),
                    existed: false,
                });
            }
            Lookup::CandidateMiss => {}
            Lookup::ReadBucket { file_offset } => {
                let bucket = read_bucket(Rc::clone(&file), file_offset).await?;
                match bucket.get(key) {
                    Some(BucketValue::Value(_)) => {
                        return Ok(DeleteObservation {
                            previous: Some(candidate),
                            existed: true,
                        });
                    }
                    Some(BucketValue::Tombstone) => {
                        return Ok(DeleteObservation {
                            previous: Some(candidate),
                            existed: false,
                        });
                    }
                    None => {}
                }
            }
        }
    }

    Ok(DeleteObservation {
        previous: None,
        existed: false,
    })
}

async fn execute_set(
    worker: WorkerHandle,
    file: Rc<File>,
    key: StorageKey,
    value: Arc<[u8]>,
) -> io::Result<Reply> {
    loop {
        let plan = worker.access(|worker| worker.storage.prepare_set(&key));
        let observation = resolve_set_plan(worker, Rc::clone(&file), &key, &plan).await?;
        let commit =
            worker.access(|worker| worker.storage.commit_set(&key, &value, plan, observation));

        match commit {
            CommitSet::Retry => continue,
            CommitSet::Finished { result, flush } => {
                if let Some(flush) = flush {
                    compio::runtime::spawn(flush_sg(worker, Rc::clone(&file), flush)).detach();
                }
                result
                    .map_err(|error| io::Error::other(format!("storage SET failed: {error:?}")))?;
                return Ok(Reply::SetOk);
            }
        }
    }
}

async fn execute_delete(
    worker: WorkerHandle,
    file: Rc<File>,
    key: StorageKey,
) -> io::Result<Reply> {
    loop {
        let plan = worker.access(|worker| worker.storage.prepare_set(&key));
        let observation = resolve_delete_plan(worker, Rc::clone(&file), &key, &plan).await?;
        match worker.access(|worker| worker.storage.commit_delete(&key, plan, observation)) {
            CommitDelete::Retry => continue,
            CommitDelete::Finished(existed) => return Ok(Reply::Delete(existed)),
        }
    }
}

async fn flush_sg(worker: WorkerHandle, file: Rc<File>, flush: FlushJob) {
    let file_offset = flush.sg_index as u64 * SG_BYTES;
    let mut file = &*file;
    let (result, returned_buffer) = file
        .write_all_at(flush.buffer, file_offset)
        .await
        .into_parts();

    worker.access(|worker| {
        if let Err(error) = worker
            .storage
            .complete_flush(flush.sg_index, result, returned_buffer)
        {
            worker.fatal_io_error = Some(error);
        }
    });
}

async fn execute_request(
    worker: WorkerHandle,
    file: Rc<File>,
    request: StorageRequest,
) -> io::Result<StorageResponse> {
    let StorageRequest {
        client_id,
        sequence,
        command,
    } = request;
    let reply = match command {
        Command::Get { key } => execute_get(worker, file, StorageKey::from_key(&key)).await?,
        Command::Set { key, value } => {
            execute_set(worker, file, StorageKey::from_key(&key), value).await?
        }
        Command::Delete { key } => execute_delete(worker, file, StorageKey::from_key(&key)).await?,
    };

    Ok(StorageResponse {
        client_id,
        sequence,
        reply,
    })
}

fn create_runtime() -> io::Result<Runtime> {
    let mut proactor = ProactorBuilder::new();
    proactor
        .capacity(STORAGE_IO_QUEUE_ENTRIES)
        .single_issuer(true)
        .defer_taskrun(true)
        .taskrun_flag(true);

    let mut runtime = Runtime::builder();
    runtime.with_proactor(proactor);
    runtime.build()
}

fn open_storage_file(runtime: &Runtime) -> io::Result<Rc<File>> {
    #[cfg(target_os = "linux")]
    let direct_io_flags = libc::O_DIRECT;
    #[cfg(target_os = "macos")]
    let direct_io_flags = 0;

    runtime.block_on(async {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(direct_io_flags)
            .open(STORAGE_FILE_PATH)
            .await?;
        file.set_len(STORAGE_FILE_BYTES).await?;
        Ok(Rc::new(file))
    })
}

fn run_storage_worker(
    runtime: Runtime,
    mut worker: Box<WorkerState>,
    file: Rc<File>,
    mut request_receiver: Consumer<StorageRequest, STORAGE_QUEUE_SLOTS>,
    mut response_sender: Producer<StorageResponse, STORAGE_QUEUE_SLOTS>,
) -> io::Result<()> {
    let worker_handle = WorkerHandle(NonNull::from(worker.as_mut()));

    runtime.enter(|| {
        loop {
            while let Some(request) = request_receiver.pop() {
                let file = Rc::clone(&file);
                runtime
                    .spawn(async move {
                        match execute_request(worker_handle, file, request).await {
                            Ok(response) => {
                                worker_handle.access(|worker| worker.completed.push_back(response))
                            }
                            Err(error) => {
                                worker_handle.access(|worker| worker.fatal_io_error = Some(error));
                            }
                        }
                    })
                    .detach();
            }

            runtime.run();
            runtime.flush();
            runtime.poll_with(Some(Duration::ZERO));
            runtime.run();

            while response_sender.has_capacity() {
                let Some(response) = worker_handle.access(|worker| worker.completed.pop_front())
                else {
                    break;
                };
                let Ok(()) = response_sender.push(response) else {
                    unreachable!("response queue reported capacity for its sole producer");
                };
            }

            if let Some(error) = worker_handle.access(|worker| worker.fatal_io_error.take()) {
                return Err(error);
            }
        }
    })
}

pub(crate) fn run(
    request_receiver: Consumer<StorageRequest, STORAGE_QUEUE_SLOTS>,
    response_sender: Producer<StorageResponse, STORAGE_QUEUE_SLOTS>,
) -> io::Result<()> {
    let runtime = create_runtime()?;
    let file = open_storage_file(&runtime)?;
    let storage = Storage::new()
        .map_err(|error| io::Error::other(format!("failed to create storage: {error:?}")))?;
    let worker = Box::new(WorkerState {
        storage,
        completed: VecDeque::new(),
        fatal_io_error: None,
    });

    run_storage_worker(runtime, worker, file, request_receiver, response_sender)
}
