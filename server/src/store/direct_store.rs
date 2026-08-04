//! Direct mutable SG store backed by a worker-local variable-generation ring.

use std::cell::Cell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::future::FutureExt;
use futures_util::stream::{FuturesUnordered, StreamExt};
use openkache_protocol::{EvictionMode, SetCondition, SetOptions};

use super::*;
use crate::types::StoredItemValue;

const MAX_LEASED_SSD_VALUE_READ_BYTES: usize = 6 * BUCKET_BYTES;

struct MutableGeneration {
    logical_sg_id: u32,
    sequence: u64,
    segment: MutableSegment,
    blob_arena: BlobArena,
    large_value_arena: BlobArena,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum MutableValueHandle {
    Blob {
        lane: usize,
        logical_sg_id: u32,
        handle: BlobHandle,
    },
    Large {
        lane: usize,
        logical_sg_id: u32,
        handle: BlobHandle,
    },
}

struct MutablePlacement {
    table_location: TableLocation,
    mutable_value: Option<MutableValueHandle>,
    in_place: bool,
}

struct LocatedItem {
    table_location: TableLocation,
    item: Item,
    backing: ReadBacking,
    sequence: u64,
}

pub(crate) enum KeyedOperation {
    Get,
    Set {
        value: StoredItemValue,
        options: SetOptions,
    },
    Delete,
}

pub(crate) enum KeyedOutcome {
    Value(Option<StoredItemValue>),
    Set(SetOutcome),
    Deleted(bool),
}

#[derive(Clone)]
pub(crate) enum KeyedVisibleState {
    Missing,
    Present(StoredItemValue),
}

pub(crate) struct KeyedFinish {
    pub(crate) outcome: Result<KeyedOutcome>,
    pub(crate) visible_state: Option<KeyedVisibleState>,
    pub(crate) flush_required: bool,
}

enum PreparedKeyedOperation {
    Get,
    Set {
        value: StoredItemValue,
        options: SetOptions,
    },
    Delete,
}

#[derive(Clone, Copy)]
struct LocatedKeyState {
    table_location: TableLocation,
    item_state: ItemState,
    mutable_value: Option<MutableValueHandle>,
}

enum KeyedObservation {
    Value(Option<StoredItemValue>),
    State(Option<LocatedKeyState>),
}

enum KeyedObservationPlan {
    Read(DirectReadPlan, ReadPurpose),
    Error(KvError),
}

#[derive(Clone, Copy)]
enum ReadPurpose {
    Value,
    State,
}

pub(crate) struct KeyedJob {
    storage_key: StorageKey,
    operation: PreparedKeyedOperation,
    observation: KeyedObservationPlan,
}

pub(crate) struct CompletedKeyedJob {
    storage_key: StorageKey,
    operation: PreparedKeyedOperation,
    observation: Result<KeyedObservation>,
}

impl KeyedJob {
    pub(crate) async fn run(self) -> CompletedKeyedJob {
        let observation = match self.observation {
            KeyedObservationPlan::Read(plan, purpose) => plan.read(purpose).await,
            KeyedObservationPlan::Error(error) => Err(error),
        };
        CompletedKeyedJob {
            storage_key: self.storage_key,
            operation: self.operation,
            observation,
        }
    }
}

enum PreparedReadBacking {
    Mutable {
        item: Option<Item>,
        value: Option<Result<StoredItemValue>>,
        mutable_value: Option<MutableValueHandle>,
    },
    Ram {
        backing: Rc<RamBacking>,
        _retirement_guard: Option<Rc<SsdBacking>>,
    },
    Ssd(Rc<SsdBacking>),
}

struct PreparedReadCandidate {
    table_location: TableLocation,
    sequence: u64,
    backing: PreparedReadBacking,
}

enum ObservedValue {
    None,
    Encoded(Vec<u8>),
    OwnedInline(Vec<u8>),
    DirectInline {
        buffer: DirectIoBufferLease,
        range: std::ops::Range<usize>,
    },
    RamInline {
        segment: Arc<DirectIoBuffer>,
        range: std::ops::Range<usize>,
    },
}

struct ObservedItem {
    state: ItemState,
    value: ObservedValue,
}

struct DirectReadPlan {
    data: File,
    large_values: File,
    config: Config,
    storage_key: StorageKey,
    candidates: Vec<PreparedReadCandidate>,
    io: Rc<DirectStoreIo>,
    _job_pins: Vec<JobPin>,
}

impl DirectReadPlan {
    async fn read(self, purpose: ReadPurpose) -> Result<KeyedObservation> {
        let mut newest = None;
        for candidate in self.candidates {
            let item = candidate
                .read_item(
                    &self.data,
                    &self.config,
                    self.storage_key,
                    purpose,
                    &self.io,
                )
                .await?;
            let Some(item) = item else {
                continue;
            };
            if newest
                .as_ref()
                .is_none_or(|(current, _): &(PreparedReadCandidate, ObservedItem)| {
                    candidate.sequence > current.sequence
                })
            {
                newest = Some((candidate, item));
            }
        }
        let Some((candidate, item)) = newest else {
            return Ok(match purpose {
                ReadPurpose::Value => KeyedObservation::Value(None),
                ReadPurpose::State => KeyedObservation::State(None),
            });
        };
        match purpose {
            ReadPurpose::Value => {
                if !item_state_is_live_now(item.state) {
                    return Ok(KeyedObservation::Value(None));
                }
                candidate
                    .read_value(
                        &self.data,
                        &self.large_values,
                        &self.config,
                        item.value,
                        &self.io,
                    )
                    .await
                    .map(|value| KeyedObservation::Value(Some(value)))
            }
            ReadPurpose::State => Ok(KeyedObservation::State(Some(LocatedKeyState {
                table_location: candidate.table_location,
                item_state: item.state,
                mutable_value: candidate.mutable_value(),
            }))),
        }
    }
}

impl PreparedReadCandidate {
    async fn read_item(
        &self,
        data: &File,
        config: &Config,
        storage_key: StorageKey,
        purpose: ReadPurpose,
        io: &DirectStoreIo,
    ) -> Result<Option<ObservedItem>> {
        match &self.backing {
            PreparedReadBacking::Mutable { item, .. } => {
                Ok(item.as_ref().map(|item| ObservedItem {
                    state: ItemState {
                        is_tombstone: item.is_tombstone,
                        expires_at_ms: item.expires_at_ms,
                        eviction_protected: item.eviction_protected,
                    },
                    value: ObservedValue::None,
                }))
            }
            PreparedReadBacking::Ram { backing, .. } => {
                let bucket_index = bucket_hash(
                    &storage_key,
                    self.table_location.bucket_hash_index,
                    config.bucket_count(),
                );
                let start = bucket_index * BUCKET_BYTES;
                let bucket = &backing.segment[start..start + BUCKET_BYTES];
                let Some((state, range)) = find_item_state_and_value_range(bucket, &storage_key)
                else {
                    return Ok(None);
                };
                let range = start + range.start..start + range.end;
                let value = match purpose {
                    ReadPurpose::State => ObservedValue::None,
                    ReadPurpose::Value => {
                        match decode_stored_value(&backing.segment[range.clone()])? {
                            StoredValue::Inline(_) => ObservedValue::RamInline {
                                segment: Arc::clone(&backing.segment),
                                range: range.start + STORED_VALUE_TAG_BYTES..range.end,
                            },
                            StoredValue::Blob(_) | StoredValue::Large(_) => {
                                ObservedValue::Encoded(backing.segment[range].to_vec())
                            }
                        }
                    }
                };
                Ok(Some(ObservedItem { state, value }))
            }
            PreparedReadBacking::Ssd(backing) => {
                let bucket_index = bucket_hash(
                    &storage_key,
                    self.table_location.bucket_hash_index,
                    config.bucket_count(),
                );
                let bytes = read_exact_direct(
                    data,
                    io.bucket_read_pool.take_bucket().await,
                    backing.location.sg_base + (bucket_index * BUCKET_BYTES) as u64,
                    BUCKET_BYTES,
                    config.read_max_time_us,
                    "generation Bucket read",
                )
                .await?;
                io.data_read.set(io.data_read.get() + BUCKET_BYTES as u64);
                let observed =
                    find_item_state_and_value_range(&bytes, &storage_key).map(|(state, range)| {
                        let value = match purpose {
                            ReadPurpose::State => ObservedValue::None,
                            ReadPurpose::Value
                                if config.lease_ssd_read_buffer
                                    && bytes.get(range.start) == Some(&INLINE_VALUE_TAG) =>
                            {
                                ObservedValue::DirectInline {
                                    buffer: bytes,
                                    range: range.start + STORED_VALUE_TAG_BYTES..range.end,
                                }
                            }
                            ReadPurpose::Value
                                if config.copy_ssd_inline_value_once
                                    && bytes.get(range.start) == Some(&INLINE_VALUE_TAG) =>
                            {
                                ObservedValue::OwnedInline(
                                    bytes[range.start + STORED_VALUE_TAG_BYTES..range.end].to_vec(),
                                )
                            }
                            ReadPurpose::Value => ObservedValue::Encoded(bytes[range].to_vec()),
                        };
                        ObservedItem { state, value }
                    });
                Ok(observed)
            }
        }
    }

    fn mutable_value(&self) -> Option<MutableValueHandle> {
        match &self.backing {
            PreparedReadBacking::Mutable { mutable_value, .. } => *mutable_value,
            PreparedReadBacking::Ram { .. } | PreparedReadBacking::Ssd(_) => None,
        }
    }

    async fn read_value(
        self,
        data: &File,
        large_values: &File,
        config: &Config,
        value: ObservedValue,
        io: &DirectStoreIo,
    ) -> Result<StoredItemValue> {
        match self.backing {
            PreparedReadBacking::Mutable { value, .. } => value.ok_or_else(|| {
                KvError::Worker("mutable keyed read has no value snapshot".into())
            })?,
            PreparedReadBacking::Ram { backing, .. } => match value {
                ObservedValue::RamInline { segment, range } => {
                    Ok(StoredItemValue::from_segment(segment, range))
                }
                ObservedValue::Encoded(encoded) => {
                    let bytes = read_ram_value(encoded, &backing)?;
                    Ok(StoredItemValue::new(bytes))
                }
                ObservedValue::OwnedInline(_) => Err(KvError::Worker(
                    "RAM keyed read has an incompatible owned inline snapshot".into(),
                )),
                ObservedValue::DirectInline { .. } => Err(KvError::Worker(
                    "RAM keyed read has an incompatible direct-read snapshot".into(),
                )),
                ObservedValue::None => Err(KvError::Worker(
                    "RAM keyed read has no value snapshot".into(),
                )),
            },
            PreparedReadBacking::Ssd(backing) => match value {
                ObservedValue::OwnedInline(bytes) => Ok(StoredItemValue::new(bytes)),
                ObservedValue::DirectInline { buffer, range } => {
                    Ok(StoredItemValue::from_direct_read(buffer, range))
                }
                ObservedValue::Encoded(mut encoded) => {
                    read_ssd_value(data, large_values, config, &backing, &mut encoded, io).await
                }
                ObservedValue::None | ObservedValue::RamInline { .. } => Err(KvError::Worker(
                    "SSD keyed read has no value snapshot".into(),
                )),
            },
        }
    }
}

enum PendingKeyedMutation {
    Set {
        storage_key: StorageKey,
        value: StoredItemValue,
        expires_at_ms: u64,
        eviction_protected: bool,
        previous: Option<TableLocation>,
        previous_mutable_value: Option<MutableValueHandle>,
        previous_live: bool,
    },
}

struct DirectStoreIo {
    data_written: Cell<u64>,
    data_read: Cell<u64>,
    bucket_read_pool: DirectIoBufferPool,
    value_read_pool: DirectIoBufferPool,
}

impl DirectStoreIo {
    fn new(bucket_read_pool_capacity: usize, lease_ssd_read_buffer: bool) -> Self {
        let value_read_pool_capacity = lease_ssd_read_buffer
            .then_some(bucket_read_pool_capacity)
            .unwrap_or(0);
        Self {
            data_written: Cell::new(0),
            data_read: Cell::new(0),
            bucket_read_pool: DirectIoBufferPool::with_capacity(bucket_read_pool_capacity),
            value_read_pool: DirectIoBufferPool::with_capacity_and_buffer_bytes(
                value_read_pool_capacity,
                MAX_LEASED_SSD_VALUE_READ_BYTES,
            ),
        }
    }
}

struct FlushCompletion {
    logical_sg_id: u32,
    location: GenerationLocation,
    large_value_location: Option<LargeValueLocation>,
    reason: SegmentFlushReason,
    fill_used_bytes: u64,
    blob_logical_len: usize,
    result: Result<u64>,
}

type FlushFuture = Pin<Box<dyn Future<Output = FlushCompletion>>>;
type EvictionReadFuture = Pin<Box<dyn Future<Output = (usize, Result<DirectIoBuffer>)>>>;

struct PreparedFlush {
    logical_sg_id: u32,
    reason: SegmentFlushReason,
    fill_used_bytes: u64,
    blob_logical_len: usize,
    blob_write: Option<DirectIoBuffer>,
    blob_physical_len: usize,
    large_value_logical_len: usize,
    large_value_write: Option<DirectIoBuffer>,
    large_value_physical_len: usize,
    segment_write: DirectIoBuffer,
}

struct ClosingFlush {
    logical_sg_id: u32,
    reason: SegmentFlushReason,
    fill_used_bytes: u64,
}

struct EvictionExtent {
    offset: usize,
    buffer: DirectIoBuffer,
    next_bucket: usize,
}

struct EvictionWork {
    victim: GenerationLocation,
    reader_guard: Option<Rc<SsdBacking>>,
    current: Option<EvictionExtent>,
    prefetched: Option<EvictionExtent>,
    read: Option<EvictionReadFuture>,
    next_read_offset: usize,
    now_ms: u64,
    retiring: bool,
    has_large_value_extent: bool,
    protected_item_found: bool,
}

pub(crate) struct Kvkache {
    config: Config,
    data: File,
    large_values: File,
    pub(crate) table: Table,
    directory: SgDirectory,
    mutable: Vec<Option<MutableGeneration>>,
    generation_log: GenerationLog,
    large_value_log: LargeValueLog,
    pending_keyed_mutations: VecDeque<PendingKeyedMutation>,
    closing_flushes: VecDeque<ClosingFlush>,
    sealed_flushes: VecDeque<PreparedFlush>,
    inflight_flushes: FuturesUnordered<FlushFuture>,
    stable_ram_segments: VecDeque<u32>,
    eviction: Option<EvictionWork>,
    next_sequence: u64,
    live_keys: usize,
    resource_guard: Arc<ResourceGuard>,
    next_memory_capacity_check: Instant,
    io: Rc<DirectStoreIo>,
    pub(crate) segment_flushes: u64,
    pub(crate) segment_capacity_flushes: u64,
    pub(crate) segment_sync_flushes: u64,
    pub(crate) segment_reuses: u64,
    pub(crate) eviction_read_timeouts: u64,
    pub(crate) generation_fill_used_bytes: u64,
    pub(crate) generation_fill_capacity_bytes: u64,
}

impl Kvkache {
    #[allow(dead_code)]
    pub(crate) async fn open(config: Config) -> Result<Self> {
        let resource_guard = Arc::new(ResourceGuard::for_worker_config(&config)?);
        Self::open_with_resource_guard(config, [0; 16], resource_guard, false).await
    }

    #[allow(dead_code)]
    pub(crate) async fn open_with_storage_key_id(
        config: Config,
        storage_key_id: [u8; 16],
    ) -> Result<Self> {
        let resource_guard = Arc::new(ResourceGuard::for_worker_config(&config)?);
        Self::open_with_resource_guard(config, storage_key_id, resource_guard, false).await
    }

    pub(crate) async fn open_with_resource_guard(
        config: Config,
        _storage_key_id: [u8; 16],
        resource_guard: Arc<ResourceGuard>,
        _allow_checkpoint: bool,
    ) -> Result<Self> {
        config.validate()?;
        Self::open_with_validated_config(config, [0; 16], resource_guard, false).await
    }

    pub(crate) async fn open_with_validated_config(
        config: Config,
        _storage_key_id: [u8; 16],
        resource_guard: Arc<ResourceGuard>,
        _allow_checkpoint: bool,
    ) -> Result<Self> {
        if let Some(parent) = config.data_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = open_direct_file(&config.data_path).await?;
        let capacity = config.generation_file_bytes()?;
        data.set_len(capacity).await?;
        reserve_file_range(&data, 0, capacity)
            .await
            .map_err(|error| storage_io_error(&resource_guard, error))?;
        resource_guard.observe_storage_reservation()?;
        let table = Table::new(&config)?;
        let logical_id_capacity = config.logical_sg_capacity()?;
        let mut directory = SgDirectory::new(logical_id_capacity)?;
        let generation_log =
            GenerationLog::new(capacity, config.segment_size, config.blob_segment_size)?;
        let mut mutable = Vec::with_capacity(config.mutable_segment_count);
        for lane in 0..config.mutable_segment_count {
            let logical_sg_id = directory.allocate_mutable(lane)?;
            mutable.push(Some(MutableGeneration {
                logical_sg_id,
                sequence: lane as u64,
                segment: MutableSegment::new(&config, logical_sg_id as usize),
                blob_arena: BlobArena::new(config.blob_segment_size),
                large_value_arena: BlobArena::new(config.large_value_capacity),
            }));
        }
        let next_sequence = config.mutable_segment_count as u64;
        if let Some(parent) = config.large_value_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let large_values = open_direct_file(&config.large_value_path).await?;
        large_values
            .set_len(config.large_value_capacity as u64)
            .await?;
        reserve_file_range(&large_values, 0, config.large_value_capacity as u64)
            .await
            .map_err(|error| storage_io_error(&resource_guard, error))?;
        resource_guard.observe_storage_reservation()?;
        let large_value_log = LargeValueLog::new(config.large_value_capacity)?;
        let bucket_read_pool_capacity = config.bucket_read_pool_capacity;
        let lease_ssd_read_buffer = config.lease_ssd_read_buffer;
        Ok(Self {
            config,
            data,
            large_values,
            table,
            directory,
            mutable,
            generation_log,
            large_value_log,
            pending_keyed_mutations: VecDeque::new(),
            closing_flushes: VecDeque::new(),
            sealed_flushes: VecDeque::new(),
            inflight_flushes: FuturesUnordered::new(),
            stable_ram_segments: VecDeque::new(),
            eviction: None,
            next_sequence,
            live_keys: 0,
            resource_guard,
            next_memory_capacity_check: Instant::now(),
            io: Rc::new(DirectStoreIo::new(
                bucket_read_pool_capacity,
                lease_ssd_read_buffer,
            )),
            segment_flushes: 0,
            segment_capacity_flushes: 0,
            segment_sync_flushes: 0,
            segment_reuses: 0,
            eviction_read_timeouts: 0,
            generation_fill_used_bytes: 0,
            generation_fill_capacity_bytes: 0,
        })
    }

    pub(crate) async fn checkpoint(&self) -> Result<()> {
        Ok(())
    }

    pub(crate) fn prepare_keyed(
        &mut self,
        storage_key: StorageKey,
        operation: KeyedOperation,
    ) -> KeyedJob {
        let (operation, observation) = match operation {
            KeyedOperation::Get => (
                PreparedKeyedOperation::Get,
                self.keyed_read_plan(storage_key, ReadPurpose::Value),
            ),
            KeyedOperation::Set { value, options } => {
                let observation = if options.ttl_ms == Some(0) {
                    KeyedObservationPlan::Error(KvError::InvalidRequest(
                        "SET TTL must be greater than zero milliseconds".into(),
                    ))
                } else if let Err(error) =
                    self.validate_value(&value.bytes, options.ttl_ms.is_some())
                {
                    KeyedObservationPlan::Error(error)
                } else {
                    let now = Instant::now();
                    let refresh_memory = now >= self.next_memory_capacity_check;
                    if refresh_memory {
                        self.next_memory_capacity_check = now + CAPACITY_CHECK_INTERVAL;
                    }
                    match self.resource_guard.admit_set(refresh_memory) {
                        Ok(()) => self.keyed_read_plan(storage_key, ReadPurpose::State),
                        Err(error) => KeyedObservationPlan::Error(error),
                    }
                };
                (
                    PreparedKeyedOperation::Set {
                        value,
                        options,
                    },
                    observation,
                )
            }
            KeyedOperation::Delete => (
                PreparedKeyedOperation::Delete,
                self.keyed_read_plan(storage_key, ReadPurpose::State),
            ),
        };
        KeyedJob {
            storage_key,
            operation,
            observation,
        }
    }

    fn keyed_read_plan(
        &self,
        storage_key: StorageKey,
        purpose: ReadPurpose,
    ) -> KeyedObservationPlan {
        let mut candidates = Vec::new();
        let mut job_pins = Vec::new();
        for table_location in self.table.candidate_locations(&storage_key) {
            let Some(backing) = self.directory.read_backing(table_location.sg_index) else {
                continue;
            };
            let (sequence, backing) = match backing {
                ReadBacking::Mutable { lane, _job_pin } => {
                    job_pins.push(_job_pin);
                    let Some(generation) = self.mutable[lane].as_ref() else {
                        continue;
                    };
                    let bucket_index = bucket_hash(
                        &storage_key,
                        table_location.bucket_hash_index,
                        self.config.bucket_count(),
                    );
                    let start = bucket_index * BUCKET_BYTES;
                    let item = find_item_in_bucket(
                        &generation.segment.bytes[start..start + BUCKET_BYTES],
                        &storage_key,
                    );
                    let mutable_value = item.as_ref().and_then(|item| {
                        mutable_value_handle_for(lane, generation.logical_sg_id, &item.value)
                    });
                    let value = match (purpose, item.as_ref()) {
                        (ReadPurpose::Value, Some(item)) if !item.is_tombstone => Some(
                            read_mutable_value(item.value.clone(), generation)
                                .map(StoredItemValue::new),
                        ),
                        _ => None,
                    };
                    (
                        generation.sequence,
                        PreparedReadBacking::Mutable {
                            item,
                            value,
                            mutable_value,
                        },
                    )
                }
                ReadBacking::Ram {
                    backing,
                    retirement_guard,
                } => (
                    backing.sequence,
                    PreparedReadBacking::Ram {
                        backing,
                        _retirement_guard: retirement_guard,
                    },
                ),
                ReadBacking::Ssd(backing) => (backing.sequence, PreparedReadBacking::Ssd(backing)),
            };
            candidates.push(PreparedReadCandidate {
                table_location,
                sequence,
                backing,
            });
        }
        KeyedObservationPlan::Read(
            DirectReadPlan {
                data: self.data.clone(),
                large_values: self.large_values.clone(),
                config: self.config.clone(),
                storage_key,
                candidates,
                io: Rc::clone(&self.io),
                _job_pins: job_pins,
            },
            purpose,
        )
    }

    pub(crate) fn finish_keyed(
        &mut self,
        completed: CompletedKeyedJob,
        include_visible_state: bool,
    ) -> KeyedFinish {
        let observation = match completed.observation {
            Ok(observation) => observation,
            Err(error) => {
                return KeyedFinish {
                    outcome: Err(error),
                    visible_state: None,
                    flush_required: false,
                };
            }
        };
        let (outcome, visible_state, flush_required) = match (completed.operation, observation) {
            (PreparedKeyedOperation::Get, KeyedObservation::Value(mut value)) => {
                let visible_state = include_visible_state.then(|| match &mut value {
                    Some(value) => KeyedVisibleState::Present(value.clone_for_visible_state()),
                    None => KeyedVisibleState::Missing,
                });
                (Ok(KeyedOutcome::Value(value)), visible_state, false)
            }
            (
                PreparedKeyedOperation::Set {
                    value,
                    options,
                },
                KeyedObservation::State(previous),
            ) => {
                let visible_value = (options == SetOptions::NONE).then(|| value.clone());
                let evaluated_at_ms = unix_time_ms();
                let expires_at_ms = match options
                    .ttl_ms
                    .map(|ttl_ms| {
                        evaluated_at_ms.checked_add(ttl_ms).ok_or_else(|| {
                            KvError::InvalidRequest(
                                "SET TTL exceeds the supported time range".into(),
                            )
                        })
                    })
                    .transpose()
                {
                    Ok(expires_at_ms) => expires_at_ms.unwrap_or_default(),
                    Err(error) => {
                        return KeyedFinish {
                            outcome: Err(error),
                            visible_state: None,
                            flush_required: false,
                        };
                    }
                };
                match self.finish_keyed_set(
                    completed.storage_key,
                    value,
                    options,
                    evaluated_at_ms,
                    expires_at_ms,
                    previous,
                ) {
                    Ok((outcome, flush_required)) => {
                        let visible_state = visible_value.and_then(|value| match outcome {
                            SetOutcome::Created | SetOutcome::Replaced => {
                                Some(KeyedVisibleState::Present(value))
                            }
                            SetOutcome::NotStored => None,
                        });
                        (
                            Ok(KeyedOutcome::Set(outcome)),
                            visible_state,
                            flush_required,
                        )
                    }
                    Err(error) => (Err(error), None, false),
                }
            }
            (
                PreparedKeyedOperation::Delete,
                KeyedObservation::State(previous),
            ) => match self.finish_keyed_delete(completed.storage_key, unix_time_ms(), previous) {
                Ok((deleted, flush_required)) => (
                    Ok(KeyedOutcome::Deleted(deleted)),
                    Some(KeyedVisibleState::Missing),
                    flush_required,
                ),
                Err(error) => (Err(error), None, false),
            },
            _ => (
                Err(KvError::Worker(
                    "keyed operation completed with an incompatible observation".into(),
                )),
                None,
                false,
            ),
        };
        KeyedFinish {
            outcome,
            visible_state,
            flush_required,
        }
    }

    pub(crate) fn can_collapse_set(&self, value: &StoredItemValue) -> bool {
        self.validate_value(&value.bytes, false).is_ok()
    }

    fn finish_keyed_set(
        &mut self,
        storage_key: StorageKey,
        value: StoredItemValue,
        options: SetOptions,
        evaluated_at_ms: u64,
        expires_at_ms: u64,
        previous: Option<LocatedKeyState>,
    ) -> Result<(SetOutcome, bool)> {
        let previous_live = previous
            .as_ref()
            .is_some_and(|located| item_state_is_live_at(located.item_state, evaluated_at_ms));
        if !set_condition_allows(options.condition, previous_live) {
            return Ok((SetOutcome::NotStored, false));
        }
        let previous_location = previous.as_ref().map(|located| located.table_location);
        let previous_mutable_value = previous.and_then(|located| located.mutable_value);
        let outcome = if previous_live {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        };
        if let Some(replacement) = self.try_append_value(
            storage_key,
            &value.bytes,
            expires_at_ms,
            matches!(options.eviction_mode, EvictionMode::EvictionProtected),
            previous_location,
            previous_mutable_value,
        )? {
            let previous_disappeared = self.publish_table_location(
                storage_key,
                previous_location,
                previous_mutable_value,
                replacement,
            )?;
            if !previous_live || previous_disappeared {
                self.live_keys += 1;
            }
            return Ok((outcome, false));
        }
        self.pending_keyed_mutations
            .push_back(PendingKeyedMutation::Set {
                storage_key,
                value,
                expires_at_ms,
                eviction_protected: matches!(
                    options.eviction_mode,
                    EvictionMode::EvictionProtected
                ),
                previous: previous_location,
                previous_mutable_value,
                previous_live,
            });
        Ok((outcome, true))
    }

    fn finish_keyed_delete(
        &mut self,
        storage_key: StorageKey,
        evaluated_at_ms: u64,
        previous: Option<LocatedKeyState>,
    ) -> Result<(bool, bool)> {
        let Some(previous) = previous else {
            return Ok((false, false));
        };
        if !item_state_is_live_at(previous.item_state, evaluated_at_ms) {
            return Ok((false, false));
        }
        self.remove_table_location(storage_key, previous.table_location, previous.mutable_value)?;
        self.live_keys = self.live_keys.saturating_sub(1);
        Ok((true, false))
    }

    pub(crate) fn progress_capacity(&mut self) -> Result<bool> {
        self.advance_closings()?;
        self.advance_flushes()?;
        while let Some(mutation) = self.pending_keyed_mutations.pop_front() {
            if let Some(mutation) = self.try_apply_pending_keyed_mutation(mutation)? {
                self.pending_keyed_mutations.push_front(mutation);
                if self.active_flush_count() >= self.config.max_flushes_in_flight {
                    return Ok(false);
                }
                let lane = self.fullest_mutable_lane()?;
                self.close_lane(lane, SegmentFlushReason::Capacity)?;
                self.advance_closings()?;
                self.advance_flushes()?;
            }
        }
        Ok(true)
    }

    fn try_apply_pending_keyed_mutation(
        &mut self,
        mutation: PendingKeyedMutation,
    ) -> Result<Option<PendingKeyedMutation>> {
        match mutation {
            PendingKeyedMutation::Set {
                storage_key,
                value,
                expires_at_ms,
                eviction_protected,
                previous,
                previous_mutable_value,
                previous_live,
            } => {
                let Some(replacement) = self.try_append_value(
                    storage_key,
                    &value.bytes,
                    expires_at_ms,
                    eviction_protected,
                    previous,
                    previous_mutable_value,
                )?
                else {
                    return Ok(Some(PendingKeyedMutation::Set {
                        storage_key,
                        value,
                        expires_at_ms,
                        eviction_protected,
                        previous,
                        previous_mutable_value,
                        previous_live,
                    }));
                };
                let previous_disappeared = self.publish_table_location(
                    storage_key,
                    previous,
                    previous_mutable_value,
                    replacement,
                )?;
                if !previous_live || previous_disappeared {
                    self.live_keys += 1;
                }
            }
        }
        Ok(None)
    }

    #[allow(dead_code)]
    pub(crate) async fn get(&self, storage_key: &StorageKey) -> Result<Option<Vec<u8>>> {
        Ok(self
            .get_encoded(storage_key)
            .await?
            .map(StoredItemValue::into_bytes))
    }

    pub(crate) async fn get_encoded(
        &self,
        storage_key: &StorageKey,
    ) -> Result<Option<StoredItemValue>> {
        let Some(located) = self.locate_item(storage_key).await? else {
            return Ok(None);
        };
        if !item_is_live_now(&located.item) {
            return Ok(None);
        }
        let bytes = self.read_value(located.item.value, located.backing).await?;
        Ok(Some(StoredItemValue::new(bytes)))
    }

    #[allow(dead_code)]
    pub(crate) async fn set(
        &mut self,
        storage_key: StorageKey,
        value: &[u8],
    ) -> Result<SetOutcome> {
        self.set_encoded(storage_key, StoredItemValue::new(value.to_vec()))
            .await
    }

    pub(crate) async fn set_encoded(
        &mut self,
        storage_key: StorageKey,
        value: StoredItemValue,
    ) -> Result<SetOutcome> {
        self.set_encoded_with_options(storage_key, value, SetOptions::NONE)
            .await
    }

    pub(crate) async fn set_encoded_with_options(
        &mut self,
        storage_key: StorageKey,
        value: StoredItemValue,
        options: SetOptions,
    ) -> Result<SetOutcome> {
        self.drive_background_once().await?;
        if options.ttl_ms == Some(0) {
            return Err(KvError::InvalidRequest(
                "SET TTL must be greater than zero milliseconds".into(),
            ));
        }
        self.validate_value(&value.bytes, options.ttl_ms.is_some())?;
        let now = Instant::now();
        let refresh_memory = now >= self.next_memory_capacity_check;
        if refresh_memory {
            self.next_memory_capacity_check = now + CAPACITY_CHECK_INTERVAL;
        }
        self.resource_guard.admit_set(refresh_memory)?;
        let now_ms = unix_time_ms();
        let expires_at_ms = options
            .ttl_ms
            .map(|ttl_ms| {
                now_ms.checked_add(ttl_ms).ok_or_else(|| {
                    KvError::InvalidRequest("SET TTL exceeds the supported time range".into())
                })
            })
            .transpose()?
            .unwrap_or_default();
        let previous = self.locate_item(&storage_key).await?;
        let previous_live = previous
            .as_ref()
            .is_some_and(|located| located.item.is_live_at(now_ms));
        if !set_condition_allows(options.condition, previous_live) {
            return Ok(SetOutcome::NotStored);
        }
        let previous_location = previous.as_ref().map(|located| located.table_location);
        let previous_mutable_value = previous
            .as_ref()
            .and_then(|located| self.mutable_value_handle(located));

        let new_location = loop {
            if let Some(location) = self.try_append_value(
                storage_key,
                &value.bytes,
                expires_at_ms,
                matches!(options.eviction_mode, EvictionMode::EvictionProtected),
                previous_location,
                previous_mutable_value,
            )? {
                break location;
            }
            let lane = self.fullest_mutable_lane()?;
            self.flush_lane(lane, SegmentFlushReason::Capacity).await?;
        };
        let previous_disappeared = self.publish_table_location(
            storage_key,
            previous_location,
            previous_mutable_value,
            new_location,
        )?;
        match (previous_live, true) {
            (false, true) => self.live_keys += 1,
            (true, true) => {}
            _ => unreachable!(),
        }
        if previous_live && previous_disappeared {
            self.live_keys += 1;
        }
        Ok(if previous_live {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        })
    }

    #[allow(dead_code)]
    pub(crate) async fn delete(&mut self, storage_key: &StorageKey) -> Result<bool> {
        self.drive_background_once().await?;
        let now_ms = unix_time_ms();
        let previous = self.locate_item(storage_key).await?;
        let Some(previous) = previous else {
            return Ok(false);
        };
        if !previous.item.is_live_at(now_ms) {
            return Ok(false);
        }
        let previous_location = previous.table_location;
        let previous_mutable_value = self.mutable_value_handle(&previous);
        self.remove_table_location(*storage_key, previous_location, previous_mutable_value)?;
        self.live_keys = self.live_keys.saturating_sub(1);
        Ok(true)
    }

    pub(crate) async fn sync(&mut self) -> Result<()> {
        for lane in 0..self.mutable.len() {
            let should_flush = self.mutable[lane]
                .as_ref()
                .is_some_and(|generation| generation.segment.item_count != 0);
            if should_flush {
                self.flush_lane(lane, SegmentFlushReason::Sync).await?;
            }
        }
        while self.has_background_work() {
            self.wait_for_background_progress().await?;
        }
        Ok(())
    }

    fn try_append_value(
        &mut self,
        storage_key: StorageKey,
        value: &[u8],
        expires_at_ms: u64,
        eviction_protected: bool,
        previous_location: Option<TableLocation>,
        previous_mutable_value: Option<MutableValueHandle>,
    ) -> Result<Option<MutablePlacement>> {
        let large = value.len() > self.config.large_value_threshold
            || value.len() > self.config.blob_segment_size;
        let blob = !large && value.len() > BLOB_ITEM_THRESHOLD_BYTES;
        let encoded_len = if large {
            STORED_LARGE_VALUE_REF_BYTES
        } else if blob {
            STORED_BLOB_REF_BYTES
        } else {
            STORED_VALUE_TAG_BYTES + value.len()
        };
        for lane in 0..self.mutable.len() {
            let Some(generation) = self.mutable[lane].as_mut() else {
                continue;
            };
            let fixed_item_bytes = ITEM_FIXED_BYTES
                + if expires_at_ms == 0 {
                    0
                } else {
                    ITEM_EXPIRATION_BYTES
                }
                + encoded_len;
            let previous_in_generation =
                previous_location.filter(|location| location.sg_index == generation.logical_sg_id);
            if let Some(previous_location) = previous_in_generation
                && let InPlaceValue::Replaced(mutable_value) = try_replace_value_in_place(
                    generation,
                    lane,
                    previous_location,
                    storage_key,
                    value,
                    expires_at_ms,
                    eviction_protected,
                    large,
                    blob,
                    previous_mutable_value,
                )?
            {
                return Ok(Some(MutablePlacement {
                    table_location: previous_location,
                    mutable_value,
                    in_place: true,
                }));
            }
            if generation
                .segment
                .choose_bucket(&storage_key, fixed_item_bytes)
                .is_none()
            {
                continue;
            }
            let Some(staged) = stage_mutable_value(generation, lane, value, large, blob)? else {
                continue;
            };
            let item = live_item(
                storage_key,
                staged.encoded,
                expires_at_ms,
                eviction_protected,
            );
            let location = generation.segment.append(item, true).ok_or_else(|| {
                KvError::Worker("chosen mutable SG Bucket rejected an Item".into())
            })?;
            if previous_in_generation.is_some_and(|previous| {
                same_physical_bucket(&storage_key, previous, location, self.config.bucket_count())
            }) {
                generation.segment.remove(location, &storage_key);
                clear_mutable_value(generation, lane, staged.mutable_value);
                continue;
            }
            return Ok(Some(MutablePlacement {
                table_location: location,
                mutable_value: staged.mutable_value,
                in_place: false,
            }));
        }
        Ok(None)
    }

    fn mutable_value_handle(&self, located: &LocatedItem) -> Option<MutableValueHandle> {
        let ReadBacking::Mutable { lane, .. } = &located.backing else {
            return None;
        };
        mutable_value_handle_for(*lane, located.table_location.sg_index, &located.item.value)
    }

    fn publish_table_location(
        &mut self,
        storage_key: StorageKey,
        previous: Option<TableLocation>,
        previous_mutable_value: Option<MutableValueHandle>,
        replacement: MutablePlacement,
    ) -> Result<bool> {
        if replacement.in_place {
            debug_assert_eq!(previous, Some(replacement.table_location));
            return Ok(false);
        }
        let previous_disappeared = match previous {
            Some(previous)
                if self.table.replace_location(
                    &storage_key,
                    previous,
                    replacement.table_location,
                ) =>
            {
                false
            }
            Some(previous) => {
                if self
                    .table
                    .candidate_locations(&storage_key)
                    .contains(&previous)
                {
                    self.rollback_mutable_placement(&storage_key, &replacement);
                    return Err(KvError::Worker(
                        "Table location changed before the mutable replacement was published"
                            .into(),
                    ));
                }
                if let Err(error) = self.table.insert(&storage_key, replacement.table_location) {
                    self.rollback_mutable_placement(&storage_key, &replacement);
                    return Err(error);
                }
                true
            }
            None => {
                if let Err(error) = self.table.insert(&storage_key, replacement.table_location) {
                    self.rollback_mutable_placement(&storage_key, &replacement);
                    return Err(error);
                }
                false
            }
        };
        if let Some(previous) = previous {
            self.remove_previous_mutable_item(&storage_key, previous, previous_mutable_value);
        }
        Ok(previous_disappeared)
    }

    fn rollback_mutable_placement(
        &mut self,
        storage_key: &StorageKey,
        placement: &MutablePlacement,
    ) {
        let Some((lane, generation)) =
            mutable_generation_for_location(&mut self.mutable, placement.table_location)
        else {
            return;
        };
        if generation
            .segment
            .remove(placement.table_location, storage_key)
        {
            clear_mutable_value(generation, lane, placement.mutable_value);
        }
    }

    fn remove_previous_mutable_item(
        &mut self,
        storage_key: &StorageKey,
        previous: TableLocation,
        previous_mutable_value: Option<MutableValueHandle>,
    ) {
        let Some((lane, generation)) = mutable_generation_for_location(&mut self.mutable, previous)
        else {
            return;
        };
        if generation.segment.remove(previous, storage_key) {
            clear_mutable_value(generation, lane, previous_mutable_value);
        }
    }

    fn remove_table_location(
        &mut self,
        storage_key: StorageKey,
        previous: TableLocation,
        previous_mutable_value: Option<MutableValueHandle>,
    ) -> Result<()> {
        if !self.table.remove(&storage_key, previous) {
            return Err(KvError::Worker(
                "Table location changed before DELETE was published".into(),
            ));
        }
        self.remove_previous_mutable_item(&storage_key, previous, previous_mutable_value);
        Ok(())
    }

    fn fullest_mutable_lane(&self) -> Result<usize> {
        self.mutable
            .iter()
            .enumerate()
            .filter_map(|(lane, generation)| {
                generation.as_ref().map(|generation| {
                    (
                        lane,
                        generation.segment.used_bytes()
                            + generation.blob_arena.allocated_bytes()
                            + generation.large_value_arena.allocated_bytes(),
                    )
                })
            })
            .max_by_key(|(_, bytes)| *bytes)
            .map(|(lane, _)| lane)
            .ok_or_else(|| KvError::Worker("worker has no mutable SG to seal".into()))
    }

    async fn flush_lane(&mut self, lane: usize, reason: SegmentFlushReason) -> Result<()> {
        while self.active_flush_count() >= self.config.max_flushes_in_flight {
            self.wait_for_background_progress().await?;
        }
        self.close_lane(lane, reason)?;
        self.advance_closings()?;
        self.advance_flushes()?;
        self.drive_background_once().await?;
        Ok(())
    }

    fn close_lane(&mut self, lane: usize, reason: SegmentFlushReason) -> Result<()> {
        let generation = self
            .mutable
            .get_mut(lane)
            .and_then(Option::take)
            .ok_or_else(|| KvError::Worker(format!("mutable SG lane {lane} is unavailable")))?;
        let logical_sg_id = generation.logical_sg_id;
        let fill_used_bytes = generation.segment.used_bytes() as u64;
        self.directory.close(
            logical_sg_id,
            RamBacking {
                sequence: generation.sequence,
                segment: Arc::new(generation.segment.bytes),
                blob_arena: generation.blob_arena,
                large_value_arena: generation.large_value_arena,
            },
        )?;
        self.closing_flushes.push_back(ClosingFlush {
            logical_sg_id,
            reason,
            fill_used_bytes,
        });
        let new_logical_sg_id = self.directory.allocate_mutable(lane)?;
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.mutable[lane] = Some(MutableGeneration {
            logical_sg_id: new_logical_sg_id,
            sequence,
            segment: MutableSegment::new(&self.config, new_logical_sg_id as usize),
            blob_arena: BlobArena::new(self.config.blob_segment_size),
            large_value_arena: BlobArena::new(self.config.large_value_capacity),
        });
        Ok(())
    }

    fn advance_closings(&mut self) -> Result<bool> {
        let mut ready = None;
        for (index, closing) in self.closing_flushes.iter().enumerate() {
            if let Some(readable) = self
                .directory
                .closing_backing_if_ready(closing.logical_sg_id)?
            {
                ready = Some((index, readable));
                break;
            }
        }
        let Some((index, readable)) = ready else {
            return Ok(false);
        };
        let closing = self
            .closing_flushes
            .remove(index)
            .expect("the ready Closing SG was inspected above");
        let packed = readable.blob_arena.pack()?;
        let packed_large_values = readable.large_value_arena.pack()?;
        let mut segment_write = readable.segment.as_ref().clone();
        rewrite_segment_values(&mut segment_write, |encoded| {
            match decode_stored_value(encoded)? {
                StoredValue::Inline(_) => Ok(None),
                StoredValue::Blob(blob_ref) => {
                    let handle = BlobHandle {
                        slot: blob_ref.value_offset,
                        value_len: blob_ref.value_len,
                    };
                    let blob_ref = packed.blob_ref(handle).unwrap_or(BlobRef {
                        value_offset: 0,
                        value_len: 0,
                    });
                    Ok(Some(encode_blob_ref(blob_ref)))
                }
                StoredValue::Large(value_ref) => {
                    let handle = BlobHandle {
                        slot: value_ref.value_offset,
                        value_len: value_ref.value_len,
                    };
                    let value_ref = packed_large_values.blob_ref(handle).unwrap_or(BlobRef {
                        value_offset: 0,
                        value_len: 0,
                    });
                    Ok(Some(encode_large_value_ref(value_ref)))
                }
            }
        })?;
        let blob_logical_len = packed.bytes.len();
        let blob_write = direct_buffer_from_bytes(&packed.bytes)?;
        let blob_physical_len = blob_write.as_ref().map_or(0, DirectIoBuffer::capacity);
        let large_value_logical_len = packed_large_values.bytes.len();
        let large_value_write = direct_buffer_from_bytes(&packed_large_values.bytes)?;
        let large_value_physical_len = large_value_write
            .as_ref()
            .map_or(0, DirectIoBuffer::capacity);
        self.sealed_flushes.push_back(PreparedFlush {
            logical_sg_id: closing.logical_sg_id,
            reason: closing.reason,
            fill_used_bytes: closing.fill_used_bytes,
            blob_logical_len,
            blob_write,
            blob_physical_len,
            large_value_logical_len,
            large_value_write,
            large_value_physical_len,
            segment_write,
        });
        Ok(true)
    }

    async fn drive_background_once(&mut self) -> Result<()> {
        std::future::poll_fn(|context| match self.poll_background(context) {
            Poll::Ready(result) => Poll::Ready(result.map(|_| ())),
            Poll::Pending => Poll::Ready(Ok(())),
        })
        .await
    }

    async fn wait_for_background_progress(&mut self) -> Result<()> {
        std::future::poll_fn(|context| {
            self.poll_background(context)
                .map(|result| result.map(|_| ()))
        })
        .await
    }

    fn complete_flush(&mut self, completion: FlushCompletion) -> Result<()> {
        let physical_bytes = completion
            .result
            .map_err(|error| storage_operation_error(&self.resource_guard, error))?;
        self.io
            .data_written
            .set(self.io.data_written.get() + physical_bytes);
        let retain_ram = self.config.stable_ram_segment_count != 0;
        self.directory.publish_stable(
            completion.logical_sg_id,
            completion.location,
            completion.large_value_location,
            retain_ram,
        )?;
        if retain_ram {
            self.stable_ram_segments.push_back(completion.logical_sg_id);
            while self.stable_ram_segments.len() > self.config.stable_ram_segment_count {
                let logical_sg_id = self
                    .stable_ram_segments
                    .pop_front()
                    .expect("the stable RAM cache exceeded its configured capacity");
                self.directory.drop_stable_ram(logical_sg_id)?;
            }
        }
        self.segment_flushes += 1;
        match completion.reason {
            SegmentFlushReason::Capacity => self.segment_capacity_flushes += 1,
            SegmentFlushReason::Sync => self.segment_sync_flushes += 1,
        }
        self.generation_fill_used_bytes += completion.fill_used_bytes
            + completion.blob_logical_len as u64
            + completion
                .large_value_location
                .map_or(0, |location| u64::from(location.logical_len));
        self.generation_fill_capacity_bytes += completion.location.record_len
            + completion
                .large_value_location
                .map_or(0, |location| u64::from(location.padded_len));
        Ok(())
    }

    pub(crate) fn has_background_work(&self) -> bool {
        !self.closing_flushes.is_empty()
            || !self.sealed_flushes.is_empty()
            || !self.inflight_flushes.is_empty()
            || self.eviction.is_some()
    }

    pub(crate) fn poll_background(&mut self, context: &mut Context<'_>) -> Poll<Result<bool>> {
        let mut progressed = false;
        if let Poll::Ready(Some(completion)) = self.inflight_flushes.poll_next_unpin(context) {
            if let Err(error) = self.complete_flush(completion) {
                return Poll::Ready(Err(error));
            }
            progressed = true;
        }
        match self.poll_eviction(context) {
            Poll::Ready(Ok(eviction_progress)) => progressed |= eviction_progress,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Pending => {}
        }
        match self.advance_closings() {
            Ok(closing_progress) => progressed |= closing_progress,
            Err(error) => return Poll::Ready(Err(error)),
        }
        match self.advance_flushes() {
            Ok(flush_progress) => progressed |= flush_progress,
            Err(error) => return Poll::Ready(Err(error)),
        }
        if progressed {
            Poll::Ready(Ok(true))
        } else {
            Poll::Pending
        }
    }

    fn active_flush_count(&self) -> usize {
        self.closing_flushes.len() + self.sealed_flushes.len() + self.inflight_flushes.len()
    }

    fn advance_flushes(&mut self) -> Result<bool> {
        if self.eviction.is_some() {
            return Ok(false);
        }
        let Some(prepared) = self.sealed_flushes.front() else {
            return Ok(false);
        };
        let generation_fits = self.generation_log.can_reserve(prepared.blob_logical_len)?;
        let large_values_fit = self
            .large_value_log
            .can_reserve(prepared.large_value_logical_len)?;
        if !generation_fits || !large_values_fit {
            let victim = self.generation_log.oldest_location().ok_or_else(|| {
                KvError::Worker("large-value space is exhausted without an SG victim".into())
            })?;
            if !self.directory.is_stable(victim.logical_sg_id) {
                return Ok(false);
            }
            self.start_eviction(victim)?;
            return Ok(true);
        }
        let GenerationReservation::Reserved(location) = self
            .generation_log
            .reserve(prepared.logical_sg_id, prepared.blob_logical_len)?
        else {
            return Err(KvError::Worker(
                "generation reservation changed after its successful preview".into(),
            ));
        };
        let large_value_location = self
            .large_value_log
            .reserve(prepared.logical_sg_id, prepared.large_value_logical_len)?;
        let prepared = self
            .sealed_flushes
            .pop_front()
            .expect("the prepared flush was inspected above");
        self.submit_flush(prepared, location, large_value_location)?;
        Ok(true)
    }

    fn submit_flush(
        &mut self,
        prepared: PreparedFlush,
        location: GenerationLocation,
        large_value_location: Option<LargeValueLocation>,
    ) -> Result<()> {
        self.directory
            .publish_inflight(prepared.logical_sg_id, location, large_value_location)?;
        let file = self.data.clone();
        let large_values = self.large_values.clone();
        let config = self.config.clone();
        self.inflight_flushes.push(
            async move {
                let result = write_generation(
                    file,
                    large_values,
                    config,
                    location,
                    large_value_location,
                    prepared.blob_write,
                    prepared.blob_physical_len,
                    prepared.large_value_write,
                    prepared.large_value_physical_len,
                    prepared.segment_write,
                )
                .await;
                FlushCompletion {
                    logical_sg_id: prepared.logical_sg_id,
                    location,
                    large_value_location,
                    reason: prepared.reason,
                    fill_used_bytes: prepared.fill_used_bytes,
                    blob_logical_len: prepared.blob_logical_len,
                    result,
                }
            }
            .boxed_local(),
        );
        Ok(())
    }

    fn start_eviction(&mut self, victim: GenerationLocation) -> Result<()> {
        let logical_sg_id = victim.logical_sg_id;
        self.stable_ram_segments
            .retain(|cached| *cached != logical_sg_id);
        let guard = self.directory.begin_eviction(logical_sg_id)?;
        let has_large_value_extent = guard.large_value_location.is_some();
        let mut eviction = EvictionWork {
            victim,
            reader_guard: Some(guard),
            current: None,
            prefetched: None,
            read: None,
            next_read_offset: 0,
            now_ms: unix_time_ms(),
            retiring: false,
            has_large_value_extent,
            protected_item_found: false,
        };
        schedule_eviction_read(&self.data, &self.config, &mut eviction);
        self.eviction = Some(eviction);
        Ok(())
    }

    fn poll_eviction(&mut self, context: &mut Context<'_>) -> Poll<Result<bool>> {
        let Some(mut eviction) = self.eviction.take() else {
            return Poll::Pending;
        };
        if eviction.retiring {
            if !self
                .directory
                .try_free_retiring(eviction.victim.logical_sg_id)?
            {
                self.eviction = Some(eviction);
                context.waker().wake_by_ref();
                return Poll::Pending;
            }
            self.generation_log
                .release_oldest(eviction.victim.logical_sg_id)?;
            if eviction.has_large_value_extent {
                self.large_value_log
                    .release_oldest(eviction.victim.logical_sg_id)?;
            }
            self.segment_reuses += 1;
            return Poll::Ready(Ok(true));
        }

        if let Some(read) = eviction.read.as_mut()
            && let Poll::Ready((offset, result)) = read.as_mut().poll(context)
        {
            let buffer = match result {
                Ok(buffer) => buffer,
                Err(KvError::Timeout(_)) => {
                    eviction.read = None;
                    eviction.next_read_offset = offset;
                    self.eviction_read_timeouts += 1;
                    schedule_eviction_read(&self.data, &self.config, &mut eviction);
                    self.eviction = Some(eviction);
                    context.waker().wake_by_ref();
                    return Poll::Ready(Ok(true));
                }
                Err(error) => {
                    return Poll::Ready(Err(storage_operation_error(&self.resource_guard, error)));
                }
            };
            self.io
                .data_read
                .set(self.io.data_read.get() + buffer.len() as u64);
            let extent = EvictionExtent {
                offset,
                buffer,
                next_bucket: 0,
            };
            if eviction.current.is_none() {
                eviction.current = Some(extent);
            } else {
                eviction.prefetched = Some(extent);
            }
            eviction.read = None;
            schedule_eviction_read(&self.data, &self.config, &mut eviction);
        }

        if eviction.current.is_none() {
            eviction.current = eviction.prefetched.take();
        }
        schedule_eviction_read(&self.data, &self.config, &mut eviction);
        if let Some(extent) = eviction.current.as_mut() {
            let deadline = Instant::now() + Duration::from_micros(50);
            let mut cleaned = 0usize;
            while extent.next_bucket * BUCKET_BYTES < extent.buffer.len()
                && (cleaned == 0 || (cleaned < 64 && Instant::now() < deadline))
            {
                self.clean_eviction_bucket(
                    eviction.victim.logical_sg_id,
                    eviction.now_ms,
                    extent,
                    &mut eviction.protected_item_found,
                )?;
                extent.next_bucket += 1;
                cleaned += 1;
            }
            if extent.next_bucket * BUCKET_BYTES == extent.buffer.len() {
                eviction.current = eviction.prefetched.take();
            }
            self.eviction = Some(eviction);
            context.waker().wake_by_ref();
            return Poll::Ready(Ok(true));
        }

        if eviction.read.is_some() {
            self.eviction = Some(eviction);
            return Poll::Pending;
        }

        if eviction.protected_item_found {
            return Poll::Ready(Err(KvError::NoCapacity));
        }

        self.directory
            .begin_retiring(eviction.victim.logical_sg_id)?;
        eviction.reader_guard.take();
        eviction.retiring = true;
        self.eviction = Some(eviction);
        context.waker().wake_by_ref();
        Poll::Ready(Ok(true))
    }

    fn clean_eviction_bucket(
        &mut self,
        logical_sg_id: u32,
        now_ms: u64,
        extent: &EvictionExtent,
        protected_item_found: &mut bool,
    ) -> Result<()> {
        let bucket_offset = extent.next_bucket * BUCKET_BYTES;
        let bucket_index = (extent.offset + bucket_offset) / BUCKET_BYTES;
        let bucket = &extent.buffer[bucket_offset..bucket_offset + BUCKET_BYTES];
        for item in items(bucket) {
            if find_item_in_bucket(bucket, &item.storage_key).as_ref() != Some(&item) {
                continue;
            }
            let Some(bucket_hash_index) = bucket_hash_index_for_bucket(
                &item.storage_key,
                bucket_index,
                self.config.bucket_count(),
                self.config.bucket_choice_count,
            ) else {
                continue;
            };
            let location = TableLocation {
                sg_index: logical_sg_id,
                bucket_hash_index,
            };
            // Ignore stale records that no longer back the table entry. This is important for
            // protected items that were explicitly deleted or replaced while their generation
            // was stable.
            if !self
                .table
                .candidate_locations(&item.storage_key)
                .contains(&location)
            {
                continue;
            }
            let replacing_protected_item = self.pending_keyed_mutations.iter().any(|mutation| {
                matches!(
                    mutation,
                    PendingKeyedMutation::Set {
                        storage_key,
                        previous: Some(previous),
                        previous_live: true,
                        ..
                    } if *storage_key == item.storage_key && previous.sg_index == logical_sg_id
                )
            });
            if item.eviction_protected && item.is_live_at(now_ms) && !replacing_protected_item {
                // Capacity eviction may remove only resolved Evictable items. Keep protected
                // records in the generation and report NoCapacity once the victim has been
                // scanned; the generation log remains pinned so the protected value stays
                // readable.
                *protected_item_found = true;
                continue;
            }
            if self.table.remove(&item.storage_key, location) {
                if item.is_live_at(now_ms) {
                    self.live_keys = self.live_keys.saturating_sub(1);
                }
            }
        }
        Ok(())
    }

    async fn locate_item(&self, storage_key: &StorageKey) -> Result<Option<LocatedItem>> {
        let mut newest = None;
        for table_location in self.table.candidate_locations(storage_key) {
            let Some(backing) = self.directory.read_backing(table_location.sg_index) else {
                continue;
            };
            let sequence = match &backing {
                ReadBacking::Mutable { lane, .. } => self.mutable[*lane]
                    .as_ref()
                    .map_or(0, |generation| generation.sequence),
                ReadBacking::Ram { backing, .. } => backing.sequence,
                ReadBacking::Ssd(backing) => backing.sequence,
            };
            let Some(item) = self
                .read_candidate(storage_key, table_location, &backing)
                .await?
            else {
                continue;
            };
            if newest
                .as_ref()
                .is_none_or(|located: &LocatedItem| sequence > located.sequence)
            {
                newest = Some(LocatedItem {
                    table_location,
                    item,
                    backing,
                    sequence,
                });
            }
        }
        Ok(newest)
    }

    async fn read_candidate(
        &self,
        storage_key: &StorageKey,
        table_location: TableLocation,
        backing: &ReadBacking,
    ) -> Result<Option<Item>> {
        let bucket_index = bucket_hash(
            storage_key,
            table_location.bucket_hash_index,
            self.config.bucket_count(),
        );
        match backing {
            ReadBacking::Mutable { lane, .. } => {
                let Some(generation) = self.mutable[*lane].as_ref() else {
                    return Ok(None);
                };
                let start = bucket_index * BUCKET_BYTES;
                Ok(find_item_in_bucket(
                    &generation.segment.bytes[start..start + BUCKET_BYTES],
                    storage_key,
                ))
            }
            ReadBacking::Ram { backing, .. } => {
                let start = bucket_index * BUCKET_BYTES;
                Ok(find_item_in_bucket(
                    &backing.segment[start..start + BUCKET_BYTES],
                    storage_key,
                ))
            }
            ReadBacking::Ssd(backing) => {
                let bytes = read_exact_direct(
                    &self.data,
                    self.io.bucket_read_pool.take_bucket().await,
                    backing.location.sg_base + (bucket_index * BUCKET_BYTES) as u64,
                    BUCKET_BYTES,
                    self.config.read_max_time_us,
                    "generation Bucket read",
                )
                .await?;
                self.io
                    .data_read
                    .set(self.io.data_read.get() + BUCKET_BYTES as u64);
                let item = find_item_in_bucket(&bytes, storage_key);
                Ok(item)
            }
        }
    }

    async fn read_value(&self, mut encoded: Vec<u8>, backing: ReadBacking) -> Result<Vec<u8>> {
        match backing {
            ReadBacking::Mutable { lane, .. } => match decode_stored_value(&encoded)? {
                StoredValue::Inline(_) => {
                    remove_stored_value_tag(&mut encoded);
                    Ok(encoded)
                }
                StoredValue::Blob(blob_ref) => self.mutable[lane]
                    .as_ref()
                    .and_then(|generation| {
                        generation.blob_arena.get(BlobHandle {
                            slot: blob_ref.value_offset,
                            value_len: blob_ref.value_len,
                        })
                    })
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| KvError::Worker("mutable Blob handle is invalid".into())),
                StoredValue::Large(value_ref) => self.mutable[lane]
                    .as_ref()
                    .and_then(|generation| {
                        generation.large_value_arena.get(BlobHandle {
                            slot: value_ref.value_offset,
                            value_len: value_ref.value_len,
                        })
                    })
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| KvError::Worker("mutable large-value handle is invalid".into())),
            },
            ReadBacking::Ram { backing, .. } => match decode_stored_value(&encoded)? {
                StoredValue::Inline(_) => {
                    remove_stored_value_tag(&mut encoded);
                    Ok(encoded)
                }
                StoredValue::Blob(blob_ref) => backing
                    .blob_arena
                    .get(BlobHandle {
                        slot: blob_ref.value_offset,
                        value_len: blob_ref.value_len,
                    })
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| KvError::Worker("sealed Blob handle is invalid".into())),
                StoredValue::Large(value_ref) => backing
                    .large_value_arena
                    .get(BlobHandle {
                        slot: value_ref.value_offset,
                        value_len: value_ref.value_len,
                    })
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| KvError::Worker("sealed large-value handle is invalid".into())),
            },
            ReadBacking::Ssd(backing) => match decode_stored_value(&encoded)? {
                StoredValue::Inline(value) => Ok(value.to_vec()),
                StoredValue::Blob(blob_ref) => self.read_blob(&backing.location, blob_ref).await,
                StoredValue::Large(value_ref) => {
                    let location = backing.large_value_location.as_ref().ok_or_else(|| {
                        KvError::Worker("large-value Item has no SSD extent".into())
                    })?;
                    self.read_large_value(location, value_ref).await
                }
            },
        }
    }

    async fn read_blob(&self, location: &GenerationLocation, blob_ref: BlobRef) -> Result<Vec<u8>> {
        let logical_end = u64::from(blob_ref.value_offset)
            .checked_add(u64::from(blob_ref.value_len))
            .ok_or_else(|| KvError::Worker("BlobRef range overflowed".into()))?;
        if logical_end > u64::from(location.blob_logical_len) {
            return Err(KvError::Worker(
                "BlobRef exceeds its generation Blob extent".into(),
            ));
        }
        if blob_ref.value_len == 0 {
            return Ok(Vec::new());
        }
        let absolute = location.record_start + u64::from(blob_ref.value_offset);
        let aligned_start = absolute / BUCKET_BYTES as u64 * BUCKET_BYTES as u64;
        let prefix = (absolute - aligned_start) as usize;
        let read_len = prefix
            .checked_add(blob_ref.value_len as usize)
            .and_then(|len| len.checked_next_multiple_of(BUCKET_BYTES))
            .ok_or_else(|| KvError::Worker("Blob direct-read extent overflowed".into()))?;
        let bytes = read_exact_direct(
            &self.data,
            DirectIoBuffer::for_read(read_len),
            aligned_start,
            read_len,
            self.config.read_max_time_us,
            "generation Blob read",
        )
        .await?;
        self.io
            .data_read
            .set(self.io.data_read.get() + read_len as u64);
        Ok(bytes[prefix..prefix + blob_ref.value_len as usize].to_vec())
    }

    async fn read_large_value(
        &self,
        location: &LargeValueLocation,
        value_ref: BlobRef,
    ) -> Result<Vec<u8>> {
        let logical_end = u64::from(value_ref.value_offset)
            .checked_add(u64::from(value_ref.value_len))
            .ok_or_else(|| KvError::Worker("large-value ref range overflowed".into()))?;
        if logical_end > u64::from(location.logical_len) {
            return Err(KvError::Worker(
                "large-value ref exceeds its SSD extent".into(),
            ));
        }
        if value_ref.value_len == 0 {
            return Ok(Vec::new());
        }
        let absolute = location.record_start + u64::from(value_ref.value_offset);
        let aligned_start = absolute / BUCKET_BYTES as u64 * BUCKET_BYTES as u64;
        let prefix = (absolute - aligned_start) as usize;
        let read_len = prefix
            .checked_add(value_ref.value_len as usize)
            .and_then(|len| len.checked_next_multiple_of(BUCKET_BYTES))
            .ok_or_else(|| KvError::Worker("large-value direct-read extent overflowed".into()))?;
        let bytes = read_exact_direct(
            &self.large_values,
            DirectIoBuffer::for_read(read_len),
            aligned_start,
            read_len,
            self.config.read_max_time_us,
            "large-value read",
        )
        .await?;
        self.io
            .data_read
            .set(self.io.data_read.get() + read_len as u64);
        Ok(bytes[prefix..prefix + value_ref.value_len as usize].to_vec())
    }

    fn validate_value(&self, value: &[u8], expiring: bool) -> Result<()> {
        if value.len() > self.config.max_item_bytes {
            return Err(KvError::ItemTooLarge {
                bytes: value.len(),
                capacity: self.config.max_item_bytes,
            });
        }
        let large = value.len() > self.config.large_value_threshold
            || value.len() > self.config.blob_segment_size;
        let stored_len = if large {
            STORED_LARGE_VALUE_REF_BYTES
        } else if value.len() > BLOB_ITEM_THRESHOLD_BYTES {
            STORED_BLOB_REF_BYTES
        } else {
            STORED_VALUE_TAG_BYTES + value.len()
        };
        let item_len =
            ITEM_FIXED_BYTES + if expiring { ITEM_EXPIRATION_BYTES } else { 0 } + stored_len;
        if item_len + item_offsets_bytes(1) + 1 > BUCKET_BYTES {
            return Err(KvError::ItemTooLarge {
                bytes: value.len(),
                capacity: self.config.max_item_bytes,
            });
        }
        if large && value.len() > self.config.large_value_capacity {
            return Err(KvError::ItemTooLarge {
                bytes: value.len(),
                capacity: self.config.large_value_capacity,
            });
        }
        Ok(())
    }

    pub(crate) fn stats(&self) -> String {
        let fill = self.generation_fill_used_bytes as f64 * 100.0
            / self.generation_fill_capacity_bytes.max(1) as f64;
        let blob_staging_live_bytes = self
            .mutable
            .iter()
            .filter_map(Option::as_ref)
            .map(|generation| generation.blob_arena.live_bytes())
            .sum::<usize>();
        let large_value_staging_live_bytes = self
            .mutable
            .iter()
            .filter_map(Option::as_ref)
            .map(|generation| generation.large_value_arena.live_bytes())
            .sum::<usize>();
        format!(
            "keys={} stable_keys={} pending_items=0 pending_value_bytes=0 mutable_sgs={} inflight_flushes={} max_flushes_in_flight={} blob_staging_live_bytes={} large_value_staging_live_bytes={} table_load={:.2}% table_memory={:.2}MiB modeled_resident={:.2}MiB flushes={} capacity_flushes={} sync_flushes={} segment_reuses={} eviction_read_timeouts={} generation_fill_percent={:.3}% generation_fill_used_bytes={} generation_fill_capacity_bytes={} memory_stop_writes={} storage_stop_writes={} rejected_writes={} data_read={} data_written={} blob_data_read={} blob_data_written=0 bucket_read_pool_capacity={} bucket_read_pool_allocations={} bucket_read_pool_reuses={} bucket_read_pool_idle={} bucket_read_pool_high_water={} value_read_pool_capacity={} value_read_pool_allocations={} value_read_pool_reuses={} value_read_pool_idle={} value_read_pool_high_water={}",
            self.live_keys,
            self.live_keys,
            self.mutable.len(),
            self.active_flush_count(),
            self.config.max_flushes_in_flight,
            blob_staging_live_bytes,
            large_value_staging_live_bytes,
            self.table.load_factor() * 100.0,
            self.table.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.segment_flushes,
            self.segment_capacity_flushes,
            self.segment_sync_flushes,
            self.segment_reuses,
            self.eviction_read_timeouts,
            fill,
            self.generation_fill_used_bytes,
            self.generation_fill_capacity_bytes,
            self.resource_guard.memory_stop_writes(),
            self.resource_guard.storage_stop_writes(),
            self.resource_guard.rejected_writes(),
            self.io.data_read.get(),
            self.io.data_written.get(),
            self.io.data_read.get(),
            self.io.bucket_read_pool.capacity(),
            self.io.bucket_read_pool.allocations(),
            self.io.bucket_read_pool.reuses(),
            self.io.bucket_read_pool.idle(),
            self.io.bucket_read_pool.high_water(),
            self.io.value_read_pool.capacity(),
            self.io.value_read_pool.allocations(),
            self.io.value_read_pool.reuses(),
            self.io.value_read_pool.idle(),
            self.io.value_read_pool.high_water(),
        )
    }

    pub(super) fn memory_bytes(&self) -> usize {
        self.table.memory_bytes()
            + self.directory.ram_bytes()
            + self
                .mutable
                .iter()
                .filter_map(Option::as_ref)
                .map(|generation| {
                    generation.segment.bytes.capacity()
                        + generation.blob_arena.allocated_bytes()
                        + generation.large_value_arena.allocated_bytes()
                })
                .sum::<usize>()
    }
}

fn item_state_is_live_at(state: ItemState, now_ms: u64) -> bool {
    !state.is_tombstone && (state.expires_at_ms == 0 || state.expires_at_ms > now_ms)
}

fn item_state_is_live_now(state: ItemState) -> bool {
    !state.is_tombstone && (state.expires_at_ms == 0 || state.expires_at_ms > unix_time_ms())
}

fn item_is_live_now(item: &Item) -> bool {
    item_state_is_live_now(ItemState {
        is_tombstone: item.is_tombstone,
        expires_at_ms: item.expires_at_ms,
        eviction_protected: item.eviction_protected,
    })
}

fn mutable_value_handle_for(
    lane: usize,
    logical_sg_id: u32,
    encoded: &[u8],
) -> Option<MutableValueHandle> {
    match decode_stored_value(encoded).ok()? {
        StoredValue::Inline(_) => None,
        StoredValue::Blob(blob_ref) => Some(MutableValueHandle::Blob {
            lane,
            logical_sg_id,
            handle: BlobHandle {
                slot: blob_ref.value_offset,
                value_len: blob_ref.value_len,
            },
        }),
        StoredValue::Large(value_ref) => Some(MutableValueHandle::Large {
            lane,
            logical_sg_id,
            handle: BlobHandle {
                slot: value_ref.value_offset,
                value_len: value_ref.value_len,
            },
        }),
    }
}

fn read_mutable_value(mut encoded: Vec<u8>, generation: &MutableGeneration) -> Result<Vec<u8>> {
    match decode_stored_value(&encoded)? {
        StoredValue::Inline(_) => {
            remove_stored_value_tag(&mut encoded);
            Ok(encoded)
        }
        StoredValue::Blob(blob_ref) => generation
            .blob_arena
            .get(BlobHandle {
                slot: blob_ref.value_offset,
                value_len: blob_ref.value_len,
            })
            .map(ToOwned::to_owned)
            .ok_or_else(|| KvError::Worker("mutable Blob handle is invalid".into())),
        StoredValue::Large(value_ref) => generation
            .large_value_arena
            .get(BlobHandle {
                slot: value_ref.value_offset,
                value_len: value_ref.value_len,
            })
            .map(ToOwned::to_owned)
            .ok_or_else(|| KvError::Worker("mutable large-value handle is invalid".into())),
    }
}

fn read_ram_value(mut encoded: Vec<u8>, backing: &RamBacking) -> Result<Vec<u8>> {
    match decode_stored_value(&encoded)? {
        StoredValue::Inline(_) => {
            remove_stored_value_tag(&mut encoded);
            Ok(encoded)
        }
        StoredValue::Blob(blob_ref) => backing
            .blob_arena
            .get(BlobHandle {
                slot: blob_ref.value_offset,
                value_len: blob_ref.value_len,
            })
            .map(ToOwned::to_owned)
            .ok_or_else(|| KvError::Worker("sealed Blob handle is invalid".into())),
        StoredValue::Large(value_ref) => backing
            .large_value_arena
            .get(BlobHandle {
                slot: value_ref.value_offset,
                value_len: value_ref.value_len,
            })
            .map(ToOwned::to_owned)
            .ok_or_else(|| KvError::Worker("sealed large-value handle is invalid".into())),
    }
}

async fn read_ssd_value(
    data: &File,
    large_values: &File,
    config: &Config,
    backing: &SsdBacking,
    encoded: &mut Vec<u8>,
    io: &DirectStoreIo,
) -> Result<StoredItemValue> {
    match decode_stored_value(encoded)? {
        StoredValue::Inline(value) => Ok(StoredItemValue::new(value.to_vec())),
        StoredValue::Blob(blob_ref) => {
            let logical_end = u64::from(blob_ref.value_offset)
                .checked_add(u64::from(blob_ref.value_len))
                .ok_or_else(|| KvError::Worker("BlobRef range overflowed".into()))?;
            if logical_end > u64::from(backing.location.blob_logical_len) {
                return Err(KvError::Worker(
                    "BlobRef exceeds its generation Blob extent".into(),
                ));
            }
            read_ssd_extent(
                data,
                backing.location.record_start,
                blob_ref,
                config.read_max_time_us,
                "generation Blob read",
                io,
                config.lease_ssd_read_buffer,
            )
            .await
        }
        StoredValue::Large(value_ref) => {
            let location = backing
                .large_value_location
                .as_ref()
                .ok_or_else(|| KvError::Worker("large-value Item has no SSD extent".into()))?;
            let logical_end = u64::from(value_ref.value_offset)
                .checked_add(u64::from(value_ref.value_len))
                .ok_or_else(|| KvError::Worker("large-value ref range overflowed".into()))?;
            if logical_end > u64::from(location.logical_len) {
                return Err(KvError::Worker(
                    "large-value ref exceeds its SSD extent".into(),
                ));
            }
            read_ssd_extent(
                large_values,
                location.record_start,
                value_ref,
                config.read_max_time_us,
                "large-value read",
                io,
                config.lease_ssd_read_buffer,
            )
            .await
        }
    }
}

async fn read_ssd_extent(
    file: &File,
    record_start: u64,
    value_ref: BlobRef,
    read_max_time_us: u64,
    operation: &'static str,
    io: &DirectStoreIo,
    lease_response: bool,
) -> Result<StoredItemValue> {
    if value_ref.value_len == 0 {
        return Ok(StoredItemValue::new(Vec::new()));
    }
    let absolute = record_start + u64::from(value_ref.value_offset);
    let aligned_start = absolute / BUCKET_BYTES as u64 * BUCKET_BYTES as u64;
    let prefix = (absolute - aligned_start) as usize;
    let read_len = prefix
        .checked_add(value_ref.value_len as usize)
        .and_then(|len| len.checked_next_multiple_of(BUCKET_BYTES))
        .ok_or_else(|| KvError::Worker("direct-read extent overflowed".into()))?;
    if lease_response && read_len <= MAX_LEASED_SSD_VALUE_READ_BYTES {
        let bytes = read_exact_direct(
            file,
            io.value_read_pool.take_buffer(read_len).await,
            aligned_start,
            read_len,
            read_max_time_us,
            operation,
        )
        .await?;
        io.data_read.set(io.data_read.get() + read_len as u64);
        return Ok(StoredItemValue::from_direct_read(
            bytes,
            prefix..prefix + value_ref.value_len as usize,
        ));
    }
    let bytes = read_exact_direct(
        file,
        DirectIoBuffer::for_read(read_len),
        aligned_start,
        read_len,
        read_max_time_us,
        operation,
    )
    .await?;
    io.data_read.set(io.data_read.get() + read_len as u64);
    Ok(StoredItemValue::new(
        bytes[prefix..prefix + value_ref.value_len as usize].to_vec(),
    ))
}

enum InPlaceValue {
    Replaced(Option<MutableValueHandle>),
    NotReplaced,
}

struct StagedMutableValue {
    encoded: Vec<u8>,
    mutable_value: Option<MutableValueHandle>,
}

#[allow(clippy::too_many_arguments)]
fn try_replace_value_in_place(
    generation: &mut MutableGeneration,
    lane: usize,
    previous_location: TableLocation,
    storage_key: StorageKey,
    value: &[u8],
    expires_at_ms: u64,
    eviction_protected: bool,
    large: bool,
    blob: bool,
    previous: Option<MutableValueHandle>,
) -> Result<InPlaceValue> {
    let value_len = u32::try_from(value.len())
        .map_err(|_| KvError::Usage("mutable value length does not fit in u32".into()))?;
    let same_slot = match previous {
        Some(MutableValueHandle::Blob {
            lane: previous_lane,
            logical_sg_id,
            handle,
        }) if previous_lane == lane
            && logical_sg_id == generation.logical_sg_id
            && blob
            && generation.blob_arena.can_replace(handle, value.len()) =>
        {
            Some((
                MutableValueHandle::Blob {
                    lane,
                    logical_sg_id,
                    handle: BlobHandle {
                        slot: handle.slot,
                        value_len,
                    },
                },
                encode_blob_handle(BlobHandle {
                    slot: handle.slot,
                    value_len,
                }),
            ))
        }
        Some(MutableValueHandle::Large {
            lane: previous_lane,
            logical_sg_id,
            handle,
        }) if previous_lane == lane
            && logical_sg_id == generation.logical_sg_id
            && large
            && generation
                .large_value_arena
                .can_replace(handle, value.len()) =>
        {
            Some((
                MutableValueHandle::Large {
                    lane,
                    logical_sg_id,
                    handle: BlobHandle {
                        slot: handle.slot,
                        value_len,
                    },
                },
                encode_large_value_handle(BlobHandle {
                    slot: handle.slot,
                    value_len,
                }),
            ))
        }
        _ => None,
    };
    if let Some((mutable_value, encoded)) = same_slot {
        let item = live_item(storage_key, encoded, expires_at_ms, eviction_protected);
        if generation.segment.replace(previous_location, item, true) {
            match (previous, mutable_value) {
                (
                    Some(MutableValueHandle::Blob {
                        handle: previous, ..
                    }),
                    MutableValueHandle::Blob { handle, .. },
                ) => {
                    let replaced = generation.blob_arena.replace(previous, value)?;
                    debug_assert_eq!(replaced, handle);
                }
                (
                    Some(MutableValueHandle::Large {
                        handle: previous, ..
                    }),
                    MutableValueHandle::Large { handle, .. },
                ) => {
                    let replaced = generation.large_value_arena.replace(previous, value)?;
                    debug_assert_eq!(replaced, handle);
                }
                _ => unreachable!("same-slot replacement preserves the mutable value tier"),
            }
            return Ok(InPlaceValue::Replaced(Some(mutable_value)));
        }
    }

    let Some(staged) = stage_mutable_value(generation, lane, value, large, blob)? else {
        return Ok(InPlaceValue::NotReplaced);
    };
    let item = live_item(
        storage_key,
        staged.encoded,
        expires_at_ms,
        eviction_protected,
    );
    if !generation.segment.replace(previous_location, item, true) {
        clear_mutable_value(generation, lane, staged.mutable_value);
        return Ok(InPlaceValue::NotReplaced);
    }
    clear_mutable_value(
        generation,
        lane,
        previous.filter(|previous| Some(*previous) != staged.mutable_value),
    );
    Ok(InPlaceValue::Replaced(staged.mutable_value))
}

fn stage_mutable_value(
    generation: &mut MutableGeneration,
    lane: usize,
    value: &[u8],
    large: bool,
    blob: bool,
) -> Result<Option<StagedMutableValue>> {
    if large {
        let handle = match generation.large_value_arena.insert(value) {
            Ok(handle) => handle,
            Err(KvError::BlobSegmentFull { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let mutable_value = MutableValueHandle::Large {
            lane,
            logical_sg_id: generation.logical_sg_id,
            handle,
        };
        return Ok(Some(StagedMutableValue {
            encoded: encode_large_value_handle(handle),
            mutable_value: Some(mutable_value),
        }));
    }
    if blob {
        let handle = match generation.blob_arena.insert(value) {
            Ok(handle) => handle,
            Err(KvError::BlobSegmentFull { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let mutable_value = MutableValueHandle::Blob {
            lane,
            logical_sg_id: generation.logical_sg_id,
            handle,
        };
        return Ok(Some(StagedMutableValue {
            encoded: encode_blob_handle(handle),
            mutable_value: Some(mutable_value),
        }));
    }
    Ok(Some(StagedMutableValue {
        encoded: encode_inline_value(value),
        mutable_value: None,
    }))
}

fn clear_mutable_value(
    generation: &mut MutableGeneration,
    lane: usize,
    mutable_value: Option<MutableValueHandle>,
) {
    match mutable_value {
        Some(MutableValueHandle::Blob {
            lane: value_lane,
            logical_sg_id,
            handle,
        }) if value_lane == lane && logical_sg_id == generation.logical_sg_id => {
            generation.blob_arena.remove(handle);
        }
        Some(MutableValueHandle::Large {
            lane: value_lane,
            logical_sg_id,
            handle,
        }) if value_lane == lane && logical_sg_id == generation.logical_sg_id => {
            generation.large_value_arena.remove(handle);
        }
        _ => {}
    }
}

fn live_item(
    storage_key: StorageKey,
    encoded: Vec<u8>,
    expires_at_ms: u64,
    eviction_protected: bool,
) -> Item {
    if expires_at_ms == 0 {
        Item::live_with_eviction(storage_key, encoded, eviction_protected)
    } else {
        Item::live_expiring_with_eviction(storage_key, encoded, expires_at_ms, eviction_protected)
    }
}

fn same_physical_bucket(
    storage_key: &StorageKey,
    first: TableLocation,
    second: TableLocation,
    bucket_count: usize,
) -> bool {
    first.sg_index == second.sg_index
        && bucket_hash(storage_key, first.bucket_hash_index, bucket_count)
            == bucket_hash(storage_key, second.bucket_hash_index, bucket_count)
}

fn mutable_generation_for_location(
    mutable: &mut [Option<MutableGeneration>],
    location: TableLocation,
) -> Option<(usize, &mut MutableGeneration)> {
    mutable
        .iter_mut()
        .enumerate()
        .find_map(|(lane, generation)| {
            generation
                .as_mut()
                .filter(|generation| generation.logical_sg_id == location.sg_index)
                .map(|generation| (lane, generation))
        })
}

fn direct_buffer_from_bytes(bytes: &[u8]) -> Result<Option<DirectIoBuffer>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let len = bytes
        .len()
        .checked_next_multiple_of(BUCKET_BYTES)
        .ok_or_else(|| KvError::Usage("Blob write padding overflowed".into()))?;
    let mut buffer = DirectIoBuffer::zeroed(len);
    buffer[..bytes.len()].copy_from_slice(bytes);
    Ok(Some(buffer))
}

fn schedule_eviction_read(data: &File, config: &Config, eviction: &mut EvictionWork) {
    const EXTENT_BYTES: usize = 1024 * 1024;
    if eviction.read.is_some()
        || eviction.prefetched.is_some()
        || eviction.next_read_offset >= config.segment_size
    {
        return;
    }
    let offset = eviction.next_read_offset;
    let len = EXTENT_BYTES.min(config.segment_size - offset);
    eviction.next_read_offset += len;
    let file = data.clone();
    let file_offset = eviction.victim.sg_base + offset as u64;
    let read_max_time_us = config.read_max_time_us.max(config.write_max_time_us);
    eviction.read = Some(
        async move {
            let result = read_exact_direct(
                &file,
                DirectIoBuffer::for_read(len),
                file_offset,
                len,
                read_max_time_us,
                "eviction SG extent read",
            )
            .await;
            (offset, result)
        }
        .boxed_local(),
    );
}

async fn write_generation(
    data: File,
    large_values: File,
    config: Config,
    location: GenerationLocation,
    large_value_location: Option<LargeValueLocation>,
    blob_write: Option<DirectIoBuffer>,
    blob_physical_len: usize,
    large_value_write: Option<DirectIoBuffer>,
    large_value_physical_len: usize,
    segment_write: DirectIoBuffer,
) -> Result<u64> {
    let blob_future = async {
        match blob_write {
            Some(buffer) => write_all_direct(
                &data,
                buffer,
                location.record_start,
                blob_physical_len,
                config.write_max_time_us,
                "generation Blob write",
            )
            .await
            .map(Some),
            None => Ok(None),
        }
    };
    let segment_future = write_all_direct(
        &data,
        segment_write,
        location.sg_base,
        config.segment_size,
        config.write_max_time_us,
        "generation SG write",
    );
    let large_value_future = async {
        match (large_value_write, large_value_location) {
            (Some(buffer), Some(location)) => write_all_direct(
                &large_values,
                buffer,
                location.record_start,
                large_value_physical_len,
                config.write_max_time_us,
                "large-value write",
            )
            .await
            .map(Some),
            (None, None) => Ok(None),
            _ => Err(KvError::Worker(
                "large-value buffer and reservation disagree".into(),
            )),
        }
    };
    let (blob_result, segment_result, large_value_result) =
        futures_util::join!(blob_future, segment_future, large_value_future);
    let _blob_buffer = blob_result?;
    let _segment_buffer = segment_result?;
    let _large_value_buffer = large_value_result?;
    Ok(blob_physical_len as u64 + config.segment_size as u64 + large_value_physical_len as u64)
}

fn bucket_hash_index_for_bucket(
    storage_key: &StorageKey,
    bucket_index: usize,
    bucket_count: usize,
    bucket_choice_count: usize,
) -> Option<u8> {
    let hashes = BucketHashSequence::new(storage_key, bucket_count);
    (0..bucket_choice_count as u8).find(|index| hashes.get(*index) == bucket_index)
}

fn set_condition_allows(condition: SetCondition, current_live: bool) -> bool {
    match condition {
        SetCondition::None => true,
        SetCondition::IfAbsent => !current_live,
        SetCondition::IfPresent => current_live,
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(u64::MAX)
}
