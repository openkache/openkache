//! Storage engine: hash table + segment groups + io_uring-backed flush to SSD.
//!
//! A RAM+SSD hybrid store built for high-throughput concurrent GET/SET/DELETE.
//! Core design choices:
//!
//! - **Fixed RAM budget**: a few segment groups (SGs) stay mutable in RAM; when
//!   the working set fills them, the oldest is flushed to SSD and becomes
//!   read-only, so total RAM use stays bounded.
//! - **Multi-choice bucket placement**: each key maps to several candidate
//!   bucket locations per SG; the hash table records the chosen one, so a lookup
//!   routes directly instead of scanning every choice.
//! - **Async lookups, synchronous commits**: reads may await a bucket load from
//!   SSD; the commit step is await-free and, if it triggers a rotation, spawns
//!   the flush as a detached task.
//! - **Single-owner, lock-free**: this worker is the sole owner of all storage
//!   state. It coordinates with the network thread only through SPSC queues, so
//!   no locks are taken on the request path.

use std::collections::VecDeque;
use std::io;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use compio::buf::IoBuf;
#[cfg(target_os = "linux")]
use compio::driver::AsRawFd;
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

pub(crate) use bucket::BUCKET_BYTES;
use bucket::{Bucket, BucketValue};
use sg::MutableSg;
use table::{Table, TableConfig, TableCreateError};

use crate::config::StorageConfig;

const STORAGE_KEY_BYTES: usize = 32;
/// Number of segment groups kept resident in RAM. Fixed: this multiplied by the
/// SG size is the fixed RAM budget the memory-efficiency story depends on.
pub(crate) const MUTABLE_SG_COUNT: usize = 3;
/// How many candidate bucket locations each key has within an SG. More choices
/// lower the collision rate but raise lookup cost; 4 balances the two.
const BUCKET_CHOICE_COUNT: u8 = 4;
pub(crate) const BUCKET_CHOICE_BITS: u32 = BUCKET_CHOICE_COUNT.ilog2();

/// A fixed-size key used by Storage and Bucket.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StorageKey([u8; STORAGE_KEY_BYTES]);

impl StorageKey {
    /// Derives the fixed-size storage key by hashing the client key with BLAKE3.
    /// Hashing gives a uniform distribution across buckets and a constant key size
    /// regardless of client key length, which the fixed bucket layout relies on.
    pub(crate) fn from_key(key: &[u8]) -> Self {
        Self(*blake3::hash(key).as_bytes())
    }

    pub(crate) const fn as_bytes(&self) -> &[u8; STORAGE_KEY_BYTES] {
        &self.0
    }

    /// The 128-bit slice of the hash used to index the table. Different byte
    /// ranges feed the table index and the per-choice bucket offsets, keeping
    /// those derivations statistically independent.
    pub(crate) fn table_hash(&self) -> u128 {
        u128::from_le_bytes(self.0[8..24].try_into().unwrap())
    }
}

/// All storage state, owned exclusively by the storage worker thread.
struct Storage {
    /// Maps each key's table hash to its encoded (SG, bucket_choice) location.
    table: Table,
    /// Every segment group in a fixed-length ring. Each is Unused, Mutable (RAM),
    /// Flushing (writing to SSD), or Ssd (read-only on disk).
    sgs: Box<[SgState]>,
    /// Index of the oldest of the MUTABLE_SG_COUNT mutable SGs. Rotation advances
    /// this, treating `sgs` as a circular buffer.
    oldest_mutable_sg_index: usize,
    /// One pre-allocated spare SG reused across flushes. Holding exactly one caps
    /// in-flight flushes at one and avoids allocating on every rotation.
    spare_mutable_sg: Option<MutableSg>,
    /// Buckets per SG, from config; used for bucket index and file offsets.
    buckets_per_sg: usize,
    /// Bytes per SG, from config; used for file offsets.
    sg_bytes: u64,
}

/// Lifecycle of a segment group. An SG moves Unused -> Mutable -> Flushing ->
/// Ssd, and its buffer returns as a reusable spare once the flush completes.
enum SgState {
    /// Not in use; available to become Mutable on the next rotation.
    Unused,
    /// Resident in RAM and accepting writes.
    Mutable(MutableSg),
    /// Being written to SSD; still readable through the shared buffer.
    Flushing(FlushBuffer),
    /// Written out and read-only; reads go through the storage file.
    Ssd,
}

/// A ref-counted view of a mutable SG that is being flushed. `Rc` lets concurrent
/// reads see the SG while the async write copies the same bytes to SSD.
#[derive(Clone)]
struct FlushBuffer(Rc<MutableSg>);

impl IoBuf for FlushBuffer {
    fn as_init(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// A pending write of one SG's bytes to its fixed offset in the storage file.
struct FlushJob {
    sg_index: usize,
    file_offset: u64,
    buffer: FlushBuffer,
}

/// Candidate locations for a key captured before an await. The commit step
/// compares against this snapshot to detect concurrent changes and retry
/// (optimistic concurrency control).
struct SetPlan {
    candidates: Box<[u32]>,
}

/// Where a SET found the key during lookup. `None` means the key was absent, so
/// the SET is an insert rather than a replace.
struct SetObservation {
    previous: Option<u32>,
}

/// Like `SetObservation`, plus whether a live value (not a tombstone) existed —
/// which decides the DELETE reply (`:1` vs `:0`).
struct DeleteObservation {
    previous: Option<u32>,
    existed: bool,
}

/// Outcome of inspecting one candidate location during a lookup.
enum Lookup {
    /// A live value found in RAM.
    Value(Arc<[u8]>),
    /// A tombstone (deletion marker) found in RAM.
    Tombstone,
    /// This candidate does not hold the key; try the next one.
    CandidateMiss,
    /// The candidate is on SSD; the caller must read the bucket at this offset.
    ReadBucket { file_offset: u64 },
}

/// Result of a commit attempt for SET. `Retry` means state changed under us
/// during the await, so the whole SET restarts from a fresh plan.
enum CommitSet {
    Retry,
    Finished {
        result: Result<(), SetError>,
        /// A flush to run asynchronously if this SET triggered a rotation.
        flush: Option<FlushJob>,
    },
}

/// Result of a commit attempt for DELETE. `Finished(bool)` reports whether a
/// live value existed.
enum CommitDelete {
    Retry,
    Finished(bool),
}

#[derive(Debug)]
enum SetError {
    /// The value is too large to fit in a single bucket.
    ValueTooLarge,
    /// The hash table has no free slot for this key.
    TableFull,
    /// A previous flush has not completed, so no spare SG is available.
    FlushStillInFlight,
    /// Every SG slot is occupied; SSD-backed capacity is exhausted.
    SsdCapacityReached,
    /// The location we expected to update disappeared before commit.
    TableLocationMissing,
}

impl Storage {
    fn new(config: &StorageConfig) -> Result<Self, TableCreateError> {
        let table = Table::new(TableConfig {
            max_entries: config.table_max_entries,
            value_bits: config.table_value_bits,
            fingerprint_bits: 8,
        })?;
        let mut sgs = (0..config.storage_sg_count)
            .map(|_| SgState::Unused)
            .collect::<Vec<_>>()
            .into_boxed_slice();

        for state in &mut sgs[..MUTABLE_SG_COUNT] {
            *state = SgState::Mutable(Self::new_mutable_sg(config));
        }

        Ok(Self {
            table,
            sgs,
            oldest_mutable_sg_index: 0,
            spare_mutable_sg: Some(Self::new_mutable_sg(config)),
            buckets_per_sg: config.buckets_per_sg,
            sg_bytes: config.sg_bytes,
        })
    }

    fn new_mutable_sg(config: &StorageConfig) -> MutableSg {
        MutableSg::new(config.buckets_per_sg, BUCKET_CHOICE_COUNT)
    }

    /// Snapshots a key's current candidate locations before an await. The commit
    /// step later diffs against this snapshot to detect concurrent modification.
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

    /// Inspects one candidate location and reports what is there. Decodes the
    /// location, then dispatches on the SG's state: RAM-resident SGs (Mutable or
    /// Flushing) are read in place; an SSD SG yields the file offset for the
    /// caller to read asynchronously.
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
                file_offset: self.bucket_file_offset(sg_index, key, bucket_choice),
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

    /// An await-free section that conditionally updates the observed location and
    /// flushes the oldest SG when needed. Returns `Retry` when concurrent state
    /// changed since the plan was captured; otherwise `Finished` with the result
    /// and an optional flush task. Being await-free is what makes the commit
    /// atomic with respect to other operations on this single-threaded worker.
    fn commit_set(
        &mut self,
        key: &StorageKey,
        value: &[u8],
        plan: SetPlan,
        observation: SetObservation,
    ) -> CommitSet {
        // Reject over-large values up front: cheap, and it guards the append path
        // from attempting an impossible insert.
        if !Bucket::new().can_append(BucketValue::Value(value)) {
            return CommitSet::Finished {
                result: Err(SetError::ValueTooLarge),
                flush: None,
            };
        }

        if let Some(previous) = observation.previous {
            // Replace path: the location we observed must still be a live candidate.
            // If it vanished during the await, another op changed it — retry.
            if !self
                .table
                .values(key.table_hash())
                .any(|candidate| candidate == previous)
            {
                return CommitSet::Retry;
            }

            // If the previous value lives in a mutable SG, try to overwrite it in
            // place, which avoids allocating a new bucket slot entirely.
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
            // Insert path: bail out if the candidate set changed since the plan was
            // captured, meaning a concurrent op may have inserted the same key.
            let current_candidates = self.table.values(key.table_hash()).collect::<Vec<_>>();
            if current_candidates.as_slice() != plan.candidates.as_ref() {
                return CommitSet::Retry;
            }
        }

        // In-place replace was not possible (or this is an insert): append into a
        // mutable SG. If all mutable SGs are full, rotate the oldest out to SSD to
        // free a fresh SG, then insert there.
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

        // Point the table at the new location. On replace this is a CAS from the
        // observed value; on insert it adds a fresh entry.
        let table_updated = match observation.previous {
            Some(previous) => self.table.replace(key.table_hash(), previous, replacement),
            None => self.table.insert(key.table_hash(), replacement).is_ok(),
        };

        if !table_updated {
            // The table changed under us (replace) or is full (insert). Undo the
            // append so the SG does not leak the orphaned value, and report why.
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

        // On a successful replace, reclaim the old value's bucket space now that
        // the table no longer points at it.
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

    /// Tries to append `value` into one of the mutable SGs, newest first, and
    /// returns its encoded location. Returns `None` if every mutable SG is full,
    /// signalling the caller to rotate. Newest-first keeps the freshest writes in
    /// the SG that will be flushed last, extending their RAM residency.
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

            // If the new slot maps to the same physical bucket as the value we are
            // replacing, keeping both would leave two entries for one key in that
            // bucket. Undo this insert and try another SG so the replace stays clean.
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

    /// Rotates the oldest mutable SG out to SSD and opens a fresh one in its place.
    /// The old SG becomes `Flushing` (wrapped in an `Rc` so reads continue during
    /// the write) and the pre-allocated spare becomes the new mutable SG. Returns
    /// the flush job plus the index of the new mutable SG, or an error if there is
    /// no free SSD slot or a flush is already in flight.
    fn rotate_mutable(&mut self) -> Result<(FlushJob, usize), SetError> {
        let old_index = self.oldest_mutable_sg_index;
        let new_index = (old_index + MUTABLE_SG_COUNT) % self.sgs.len();

        // The slot the new mutable SG will occupy must be free (not still on SSD).
        if !matches!(self.sgs[new_index], SgState::Unused) {
            return Err(SetError::SsdCapacityReached);
        }
        // Reusing the single spare bounds in-flight flushes to one; if it is gone,
        // the previous flush has not completed yet.
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
                // Each SG occupies a fixed slice of the file at index * sg_bytes.
                file_offset: old_index as u64 * self.sg_bytes,
                buffer,
            },
            new_index,
        ))
    }

    /// Finalizes a flush: marks the SG `Ssd` and reclaims its buffer as the spare.
    /// The `Rc::try_unwrap` must succeed because the flush completing means no
    /// concurrent reader still holds the buffer — that invariant is asserted.
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
        // Recover sole ownership of the SG buffer and clear it for reuse. This is
        // the reused spare, so no allocation happens on the flush completion path.
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

    // A table value packs the SG index in the high bits and the bucket choice in
    // the low `BUCKET_CHOICE_BITS`, so one u32 fully identifies a location.
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

    /// Computes the byte offset of a key's bucket within the storage file for an
    /// SSD-resident SG. Choices 0 and 1 use two independent 64-bit halves of the
    /// key hash directly; higher choices mix them so each choice lands in a
    /// different bucket, spreading collisions across the SG.
    fn bucket_file_offset(&self, sg_index: usize, key: &StorageKey, bucket_choice: u8) -> u64 {
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
        let bucket_index = hash as usize % self.buckets_per_sg;
        sg_index as u64 * self.sg_bytes + bucket_index as u64 * BUCKET_BYTES as u64
    }
}

/// Holds the storage engine and the completed-responses queue.
struct WorkerState {
    storage: Storage,
    /// Responses that have finished execution but await SPSC capacity to send back
    /// to the network thread.
    completed: VecDeque<StorageResponse>,
    /// The first fatal I/O error encountered, cached until the main loop checks it.
    fatal_io_error: Option<io::Error>,
}

/// A raw pointer to `WorkerState` that async tasks use to reach back into the
/// synchronous storage engine without needing lifetimes or `Arc`. Safe because
/// WorkerState is pinned in a Box for the thread's lifetime, and the raw pointer
/// never outlives the runtime that owns it.
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
    // Fetch the candidate set once, then iterate. If every candidate is a miss,
    // the key does not exist; otherwise we return on the first hit.
    let candidates = worker.access(|worker| worker.storage.candidates(&key));

    for candidate in candidates {
        match worker.access(|worker| worker.storage.lookup(&key, candidate)) {
            Lookup::Value(value) => return Ok(Reply::Get(Some(value))),
            Lookup::Tombstone => return Ok(Reply::Get(None)),
            Lookup::CandidateMiss => {}
            // SSD-resident bucket: issue the async read, await it, then search the
            // loaded bucket for the key. No await happens for RAM-resident buckets,
            // so a pure-RAM GET is fully synchronous.
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

/// Resolves the observation for a SET by scanning the captured candidates. If a
/// candidate holds the key (in RAM or on SSD), the returned `SetObservation`
/// records its location so `commit_set` can replace or CAS it. If no candidate
/// holds the key, `previous` is `None`, meaning the SET is an insert.
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

/// Executes a SET as an optimistic loop: capture a plan, resolve the current
/// location (possibly awaiting an SSD read), then commit. `commit_set` returns
/// `Retry` if the state changed under us during the await, so we loop until it
/// finishes. A flush triggered by the commit is spawned as a detached task so
/// the SET reply is not blocked on disk I/O.
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

/// Forces the oldest mutable SG to flush to SSD and awaits its completion, so a
/// benchmark can push data to disk without waiting for the SGs to fill. Rotate
/// errors (SSD full, a flush already in flight) become a graceful reply rather
/// than a fatal worker error.
async fn execute_flush(worker: WorkerHandle, file: Rc<File>) -> io::Result<Reply> {
    let rotation = worker.access(|worker| worker.storage.rotate_mutable());
    match rotation {
        Ok((flush_job, _new_mutable_sg_index)) => {
            // Await completion: complete_flush restores the single spare SG, so
            // the reply means "on SSD" and the next FLUSH can proceed.
            flush_sg(worker, file, flush_job).await;
            Ok(Reply::Flush(Ok(())))
        }
        Err(SetError::SsdCapacityReached) => Ok(Reply::Flush(Err("SSD capacity reached"))),
        Err(SetError::FlushStillInFlight) => Ok(Reply::Flush(Err("a flush is already in flight"))),
        Err(_) => Ok(Reply::Flush(Err("cannot flush"))),
    }
}

/// Executes a DELETE with the same optimistic capture/resolve/commit loop as
/// SET, but no flush can be triggered (a delete never grows storage).
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

/// Writes one SG's bytes to the storage file and finalizes it. A write failure is
/// stored as the worker's fatal error, since a lost flush means data on SSD would
/// be inconsistent — the worker cannot safely continue.
async fn flush_sg(worker: WorkerHandle, file: Rc<File>, flush: FlushJob) {
    let file_offset = flush.file_offset;
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

/// Dispatches one request to the matching executor and packages the reply with
/// the client id and sequence so the network thread can route it back in order.
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
        Command::Flush => execute_flush(worker, file).await?,
    };

    Ok(StorageResponse {
        client_id,
        sequence,
        reply,
    })
}

/// Builds the compio io_uring runtime for this storage thread. The proactor is
/// configured like the network thread (single_issuer, defer_taskrun) but with
/// a fixed queue depth matching the config, since we control the concurrency.
fn create_runtime(config: &StorageConfig) -> io::Result<Runtime> {
    let mut proactor = ProactorBuilder::new();
    proactor
        .capacity(config.io_queue_entries)
        .single_issuer(true)
        .defer_taskrun(true)
        .taskrun_flag(true);

    let mut runtime = Runtime::builder();
    runtime.with_proactor(proactor);
    runtime.build()
}

/// Opens the storage file with O_DIRECT (Linux) to bypass the page cache and
/// pre-allocates physical blocks if configured. Bypassing the cache avoids
/// double-buffering (we already hold data in mutable SGs), and pre-allocation
/// ensures the file is contiguous on NVMe for sequential-write benchmarks.
fn open_storage_file(runtime: &Runtime, config: &StorageConfig) -> io::Result<Rc<File>> {
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
            .open(&config.storage_file_path)
            .await?;
        if config.preallocate_file {
            // Reserve physical blocks so the backing file is contiguous, for
            // sequential-write and DLWA measurements on real NVMe.
            preallocate(&file, config.storage_file_bytes)?;
            file.set_len(config.storage_file_bytes).await?;
        } else {
            // Sparse: sets the logical size only; blocks are allocated on first
            // write to each SG offset.
            file.set_len(config.storage_file_bytes).await?;
        }
        Ok(Rc::new(file))
    })
}

#[cfg(target_os = "linux")]
fn preallocate(file: &File, len: u64) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: `fd` is a valid open descriptor for the lifetime of this call.
    let result = unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, len as libc::off_t) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn preallocate(file: &File, len: u64) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    let mut store = libc::fstore_t {
        fst_flags: libc::F_ALLOCATECONTIG,
        fst_posmode: libc::F_PEOFPOSMODE,
        fst_offset: 0,
        fst_length: len as libc::off_t,
        fst_bytesalloc: 0,
    };

    // SAFETY: `fd` is a valid open descriptor and `store` points to writable
    // storage for the duration of this synchronous fcntl call.
    let mut result = unsafe {
        libc::fcntl(
            file.as_raw_fd(),
            libc::F_PREALLOCATE,
            &mut store as *mut libc::fstore_t,
        )
    };
    if result == -1 {
        // Contiguous allocation is best-effort on APFS; fall back to any
        // available extents before reporting a genuine allocation failure.
        store.fst_flags = libc::F_ALLOCATEALL;
        result = unsafe {
            libc::fcntl(
                file.as_raw_fd(),
                libc::F_PREALLOCATE,
                &mut store as *mut libc::fstore_t,
            )
        };
    }
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// `IORING_ENTER_GETEVENTS`.
#[cfg(target_os = "linux")]
const IORING_ENTER_GETEVENTS: libc::c_uint = 1;
/// `__NR_io_uring_enter`.
#[cfg(target_os = "linux")]
const SYS_IO_URING_ENTER: libc::c_long = 426;

/// Enters the io_uring with `GETEVENTS` but submits nothing, forcing the kernel
/// to run deferred task work and post any ready completions to the CQ. compio's
/// non-blocking `poll_with` cannot do this once the TASKRUN flag is set (it
/// omits `GETEVENTS` when it thinks completions are already available), so we
/// reap directly on the ring fd. Errors are non-fatal — a failed reap just means
/// nothing was posted this iteration and the next one retries.
#[cfg(target_os = "linux")]
fn reap_completions(ring_fd: libc::c_int) {
    // SAFETY: `ring_fd` is the runtime's live io_uring fd; to_submit=0 means no
    // SQEs are consumed, so this never races compio's submission bookkeeping.
    unsafe {
        libc::syscall(
            SYS_IO_URING_ENTER,
            ring_fd,
            0,                      // to_submit
            0,                      // min_complete (non-blocking)
            IORING_ENTER_GETEVENTS, // flush deferred completions into the CQ
            std::ptr::null::<libc::c_void>(),
            0,
        );
    }
}

/// The storage worker's main loop. Each iteration: drains the request queue and
/// spawns an executor task for each, advances all async tasks until they would
/// block, force-reaps any deferred completions (Linux only; see `reap_completions`),
/// moves completed responses into the response queue, then checks for fatal errors.
/// This loop is synchronous and runs on the compio runtime.
fn run_storage_worker(
    runtime: Runtime,
    mut worker: Box<WorkerState>,
    file: Rc<File>,
    mut request_receiver: Consumer<StorageRequest, STORAGE_QUEUE_SLOTS>,
    mut response_sender: Producer<StorageResponse, STORAGE_QUEUE_SLOTS>,
) -> io::Result<()> {
    let worker_handle = WorkerHandle(NonNull::from(worker.as_mut()));
    #[cfg(target_os = "linux")]
    let ring_fd = runtime.as_raw_fd();

    runtime.enter(|| {
        loop {
            let mut popped_request = false;
            while let Some(request) = request_receiver.pop() {
                popped_request = true;
                let file = Rc::clone(&file);
                // Spawn an async task for the request; it runs until its first await
                // (usually an SSD read or flush write), then yields. `detach` means
                // the worker loop is not blocked by the spawn — it continues and polls
                // all spawned tasks collectively later.
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

            let _ = popped_request;
            // Advance every spawned task until they all hit an await.
            runtime.run();
            runtime.flush();
            #[cfg(target_os = "linux")]
            {
                // Force-reap deferred completions before compio reads the CQ.
                //
                // Under `IORING_SETUP_DEFER_TASKRUN`, a completion only lands in the
                // CQ when the ring is entered with `GETEVENTS`. compio's
                // `poll_with(Duration::ZERO)` drops `GETEVENTS` whenever its internal
                // `want_sqe == 0` (which is exactly the case once the kernel sets the
                // TASKRUN flag), so a flush write that completes while the request
                // stream is pure-CPU (SET-only) or idle is never reaped and the
                // worker wedges. A bare `io_uring_enter(fd, 0, 0, GETEVENTS)` submits
                // nothing and reaps everything, matching the network worker's path.
                reap_completions(ring_fd);
            }
            // Poll the ring for completions (non-blocking), which unblocks tasks.
            runtime.poll_with(Some(Duration::ZERO));
            runtime.run();

            // Drain completed responses back to the network thread.
            while response_sender.has_capacity() {
                let Some(response) = worker_handle.access(|worker| worker.completed.pop_front())
                else {
                    break;
                };
                let Ok(()) = response_sender.push(response) else {
                    unreachable!("response queue reported capacity for its sole producer");
                };
            }

            // Check for a fatal I/O error (flush failure, mainly) and bail out if
            // one occurred. Continuing after a failed flush risks data loss.
            if let Some(error) = worker_handle.access(|worker| worker.fatal_io_error.take()) {
                return Err(error);
            }
        }
    })
}

/// Entry point for the storage thread. Sets up the runtime, storage file, storage
/// engine, and worker state, then enters the main loop. Returns only on a fatal
/// error, which the main thread logs and terminates on.
pub(crate) fn run(
    config: StorageConfig,
    request_receiver: Consumer<StorageRequest, STORAGE_QUEUE_SLOTS>,
    response_sender: Producer<StorageResponse, STORAGE_QUEUE_SLOTS>,
) -> io::Result<()> {
    let runtime = create_runtime(&config)?;
    let file = open_storage_file(&runtime, &config)?;
    let storage = Storage::new(&config)
        .map_err(|error| io::Error::other(format!("failed to create storage: {error:?}")))?;
    let worker = Box::new(WorkerState {
        storage,
        completed: VecDeque::new(),
        fatal_io_error: None,
    });

    run_storage_worker(runtime, worker, file, request_receiver, response_sender)
}
