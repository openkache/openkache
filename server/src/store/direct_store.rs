//! Direct mutable SG store backed by a worker-local variable-generation ring.

use std::cell::Cell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use super::{
    BLOB_ITEM_THRESHOLD_BYTES, BlobArena, BlobHandle, BlobRef, BucketHashSequence,
    CAPACITY_CHECK_INTERVAL, DirectIoBuffer, DirectIoBufferLease, DirectIoBufferPool,
    GenerationLocation, GenerationLog, GenerationReservation, INLINE_VALUE_TAG,
    ITEM_EXPIRATION_BYTES, ITEM_FIXED_BYTES, Item, ItemState, JobPin, LargeValueLocation,
    LargeValueLog, MutableSegment, RamBacking, ReadBacking, ResourceGuard, STORED_BLOB_REF_BYTES,
    STORED_LARGE_VALUE_REF_BYTES, STORED_VALUE_TAG_BYTES, SegmentFlushReason, SetOutcome,
    SgDirectory, SsdBacking, StoredValue, Table, TableLocation, bucket_hash, decode_stored_value,
    encode_blob_handle, encode_blob_ref, encode_inline_value, encode_large_value_handle,
    encode_large_value_ref, find_item_in_bucket, find_item_state_and_value_range,
    item_offsets_bytes, items, open_direct_file, read_exact_direct, remove_stored_value_tag,
    reserve_file_range, rewrite_segment_values, storage_io_error, storage_operation_error,
    write_all_direct,
};
use crate::storage_backend;
use crate::storage_runtime::File;
use crate::{BUCKET_BYTES, Config, KvError, Result, StorageKey};
use futures_util::stream::FuturesUnordered;

const MAX_LEASED_SSD_VALUE_READ_BYTES: usize = 6 * BUCKET_BYTES;

mod eviction;
mod flush;
mod keyed;
mod mutations;
mod pending;
mod placement;
mod policy;
mod read_plan;
mod reads;
mod value_reads;
mod values;
use self::keyed::PendingKeyedMutation;
pub(crate) use self::keyed::{
    CompletedKeyedJob, KeyedFinish, KeyedJob, KeyedOperation, KeyedOutcome, KeyedVisibleState,
    PendingKeyedResult,
};

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

fn startup_io_error(operation: &str, error: std::io::Error) -> KvError {
    KvError::Io(std::io::Error::new(
        error.kind(),
        format!("{operation}: {error}"),
    ))
}

fn startup_storage_error(operation: &str, error: KvError) -> KvError {
    match error {
        KvError::Io(error) => startup_io_error(operation, error),
        KvError::CapacityExhausted { resource } => KvError::Worker(format!(
            "{operation}: {resource} capacity is exhausted; writes are temporarily stopped"
        )),
        error => error,
    }
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

struct EvictableLocation {
    storage_key: StorageKey,
    table_location: TableLocation,
    live: bool,
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
    evictable_items: Vec<EvictableLocation>,
}

pub(crate) struct Kvkache {
    config: Config,
    data: File,
    large_values: File,
    pub(crate) storage_device_kind: crate::platform::StorageDeviceKind,
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
        storage_backend::ensure_parent_directory(&config.data_path)
            .map_err(|error| startup_io_error("creating the data directory", error))?;
        let data = open_direct_file(&config.data_path)
            .await
            .map_err(|error| startup_io_error("opening the data file", error))?;
        let capacity = config.generation_file_bytes()?;
        data.set_len(capacity)
            .await
            .map_err(|error| startup_io_error("setting the data file length", error))?;
        reserve_file_range(&data, 0, capacity)
            .await
            .map_err(|error| storage_io_error(&resource_guard, error))
            .map_err(|error| startup_storage_error("reserving the data file range", error))?;
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
        storage_backend::ensure_parent_directory(&config.large_value_path)
            .map_err(|error| startup_io_error("creating the large-value directory", error))?;
        let large_values = open_direct_file(&config.large_value_path)
            .await
            .map_err(|error| startup_io_error("opening the large-value file", error))?;
        large_values
            .set_len(config.large_value_capacity as u64)
            .await
            .map_err(|error| startup_io_error("setting the large-value file length", error))?;
        reserve_file_range(&large_values, 0, config.large_value_capacity as u64)
            .await
            .map_err(|error| storage_io_error(&resource_guard, error))
            .map_err(|error| {
                startup_storage_error("reserving the large-value file range", error)
            })?;
        resource_guard.observe_storage_reservation()?;
        let storage_device_kind = storage_backend::file_device_kind(&data)
            .combine(storage_backend::file_device_kind(&large_values));
        let large_value_log = LargeValueLog::new(config.large_value_capacity)?;
        let bucket_read_pool_capacity = config.bucket_read_pool_capacity;
        let lease_ssd_read_buffer = config.lease_ssd_read_buffer;
        Ok(Self {
            config,
            data,
            large_values,
            storage_device_kind,
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
