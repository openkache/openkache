//! Direct mutable SG store backed by a worker-local variable-generation ring.

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::future::Future;
use std::io::{Read, Write};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use super::{
    BLOB_ITEM_THRESHOLD_BYTES, BlobArena, BlobHandle, BlobRef, BucketHashSequence,
    CAPACITY_CHECK_INTERVAL, CommittedGenerationState, DirectIoBuffer, DirectIoBufferLease,
    DirectIoBufferPool, GenerationLocation, GenerationLog, GenerationReservation, INLINE_VALUE_TAG,
    ITEM_EXPIRATION_BYTES, ITEM_FIXED_BYTES, Item, ItemState, JobPin, LargeValueLocation,
    LargeValueLog, MutableSegment, RamBacking, ReadBacking, ResourceGuard, STORED_BLOB_REF_BYTES,
    STORED_LARGE_VALUE_REF_BYTES, STORED_VALUE_TAG_BYTES, SegmentFlushReason, SetOutcome,
    SgDirectory, StoredValue, Table, TableLocation, bucket_hash, decode_stored_value,
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
const CHECKPOINT_MAGIC: &[u8; 8] = b"OKCPV2\0\0";
const CHECKPOINT_VERSION: u32 = 1;
const CHECKPOINT_MAX_RECORDS: usize = 1_000_000;

struct CheckpointGeneration {
    sequence: u64,
    location: GenerationLocation,
    large_value_location: Option<LargeValueLocation>,
}

struct Checkpoint {
    generations: Vec<CommittedGenerationState>,
    index: Vec<(StorageKey, TableLocation)>,
}

fn checkpoint_path(config: &Config) -> std::path::PathBuf {
    config.data_path.with_extension("checkpoint")
}

fn load_checkpoint(config: &Config) -> Result<Option<Checkpoint>> {
    let path = checkpoint_path(config);
    let mut file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(startup_io_error("opening the storage checkpoint", error)),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| startup_io_error("reading the storage checkpoint", error))?;
    decode_checkpoint(config, &bytes)
        .map(Some)
        .map_err(|error| startup_storage_error("decoding the storage checkpoint", error))
}

fn encode_checkpoint(
    config: &Config,
    generations: &[CheckpointGeneration],
    index: &HashMap<StorageKey, TableLocation>,
) -> Result<Vec<u8>> {
    let generation_count = u32::try_from(generations.len())
        .map_err(|_| KvError::Worker("storage checkpoint generation count exceeds u32".into()))?;
    let index_count = u64::try_from(index.len())
        .map_err(|_| KvError::Worker("storage checkpoint Item count exceeds u64".into()))?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(CHECKPOINT_MAGIC);
    bytes.extend_from_slice(&CHECKPOINT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(config.segment_size as u64).to_le_bytes());
    bytes.extend_from_slice(&(config.blob_segment_size as u64).to_le_bytes());
    bytes.extend_from_slice(&(config.large_value_capacity as u64).to_le_bytes());
    bytes.extend_from_slice(&(config.segment_count as u64).to_le_bytes());
    bytes.extend_from_slice(&generation_count.to_le_bytes());
    bytes.extend_from_slice(&index_count.to_le_bytes());
    for generation in generations {
        bytes.extend_from_slice(&generation.sequence.to_le_bytes());
        encode_generation_location(&mut bytes, generation.location);
        match generation.large_value_location {
            Some(location) => {
                bytes.push(1);
                encode_large_value_location(&mut bytes, location);
            }
            None => bytes.push(0),
        }
    }
    let mut entries = index.iter().collect::<Vec<_>>();
    entries.sort_unstable_by_key(|(storage_key, _)| storage_key.into_bytes());
    for (storage_key, location) in entries {
        bytes.extend_from_slice(storage_key.as_bytes());
        bytes.extend_from_slice(&location.sg_index.to_le_bytes());
        bytes.push(location.bucket_hash_index);
        bytes.extend_from_slice(&[0; 3]);
    }
    let checksum = crc32fast::hash(&bytes);
    bytes.extend_from_slice(&checksum.to_le_bytes());
    Ok(bytes)
}

fn decode_checkpoint(config: &Config, bytes: &[u8]) -> Result<Checkpoint> {
    if bytes.len() < CHECKPOINT_MAGIC.len() + 4 + 8 * 4 + 4 + 8 + 4
        || &bytes[..CHECKPOINT_MAGIC.len()] != CHECKPOINT_MAGIC
    {
        return Err(KvError::Worker(
            "storage checkpoint header is invalid".into(),
        ));
    }
    let checksum_start = bytes.len() - 4;
    let expected = u32::from_le_bytes(
        bytes[checksum_start..]
            .try_into()
            .map_err(|_| KvError::Worker("storage checkpoint checksum is truncated".into()))?,
    );
    if crc32fast::hash(&bytes[..checksum_start]) != expected {
        return Err(KvError::Worker(
            "storage checkpoint checksum does not match".into(),
        ));
    }
    let mut cursor = Cursor::new(&bytes[..checksum_start]);
    if cursor.take(8)? != CHECKPOINT_MAGIC {
        return Err(KvError::Worker(
            "storage checkpoint magic is invalid".into(),
        ));
    }
    if cursor.u32()? != CHECKPOINT_VERSION {
        return Err(KvError::Worker(
            "storage checkpoint version is unsupported".into(),
        ));
    }
    for (label, expected) in [
        ("Segment size", config.segment_size as u64),
        ("Blob Segment size", config.blob_segment_size as u64),
        ("large-value capacity", config.large_value_capacity as u64),
        ("Segment count", config.segment_count as u64),
    ] {
        if cursor.u64()? != expected {
            return Err(KvError::Worker(format!(
                "storage checkpoint {label} does not match the configured geometry"
            )));
        }
    }
    let generation_count = usize::try_from(cursor.u32()?).map_err(|_| {
        KvError::Worker("storage checkpoint generation count overflows usize".into())
    })?;
    let index_count = usize::try_from(cursor.u64()?)
        .map_err(|_| KvError::Worker("storage checkpoint Item count overflows usize".into()))?;
    if generation_count > CHECKPOINT_MAX_RECORDS || index_count > CHECKPOINT_MAX_RECORDS {
        return Err(KvError::Worker(
            "storage checkpoint contains too many records".into(),
        ));
    }
    let mut generations = Vec::with_capacity(generation_count);
    for _ in 0..generation_count {
        let sequence = cursor.u64()?;
        let location = decode_generation_location(&mut cursor)?;
        let large_value_location = match cursor.byte()? {
            0 => None,
            1 => Some(decode_large_value_location(&mut cursor)?),
            _ => {
                return Err(KvError::Worker(
                    "storage checkpoint large-value flag is invalid".into(),
                ));
            }
        };
        generations.push(CommittedGenerationState {
            sequence,
            location,
            large_value_location,
        });
    }
    let mut index = Vec::with_capacity(index_count);
    for _ in 0..index_count {
        let storage_key = StorageKey::new(cursor.array_32()?);
        let sg_index = cursor.u32()?;
        let bucket_hash_index = cursor.byte()?;
        let _ = cursor.take(3)?;
        index.push((
            storage_key,
            TableLocation {
                sg_index,
                bucket_hash_index,
            },
        ));
    }
    if !cursor.is_empty() {
        return Err(KvError::Worker(
            "storage checkpoint contains trailing bytes".into(),
        ));
    }
    Ok(Checkpoint { generations, index })
}

fn encode_generation_location(bytes: &mut Vec<u8>, location: GenerationLocation) {
    bytes.extend_from_slice(&location.logical_sg_id.to_le_bytes());
    bytes.extend_from_slice(&location.record_start.to_le_bytes());
    bytes.extend_from_slice(&location.blob_logical_len.to_le_bytes());
    bytes.extend_from_slice(&location.blob_padded_len.to_le_bytes());
    bytes.extend_from_slice(&location.sg_base.to_le_bytes());
    bytes.extend_from_slice(&location.record_len.to_le_bytes());
}

fn decode_generation_location(cursor: &mut Cursor<'_>) -> Result<GenerationLocation> {
    Ok(GenerationLocation {
        logical_sg_id: cursor.u32()?,
        record_start: cursor.u64()?,
        blob_logical_len: cursor.u32()?,
        blob_padded_len: cursor.u32()?,
        sg_base: cursor.u64()?,
        record_len: cursor.u64()?,
    })
}

fn encode_large_value_location(bytes: &mut Vec<u8>, location: LargeValueLocation) {
    bytes.extend_from_slice(&location.logical_sg_id.to_le_bytes());
    bytes.extend_from_slice(&location.record_start.to_le_bytes());
    bytes.extend_from_slice(&location.logical_len.to_le_bytes());
    bytes.extend_from_slice(&location.padded_len.to_le_bytes());
}

fn decode_large_value_location(cursor: &mut Cursor<'_>) -> Result<LargeValueLocation> {
    Ok(LargeValueLocation {
        logical_sg_id: cursor.u32()?,
        record_start: cursor.u64()?,
        logical_len: cursor.u32()?,
        padded_len: cursor.u32()?,
    })
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| KvError::Worker("storage checkpoint offset overflowed".into()))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| KvError::Worker("storage checkpoint is truncated".into()))?;
        self.offset = end;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn array_32(&mut self) -> Result<[u8; 32]> {
        Ok(self.take(32)?.try_into().unwrap())
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

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
    reason: SegmentFlushReason,
    fill_used_bytes: u64,
    blob_logical_len: usize,
    result: Result<(CommittedGenerationState, u64)>,
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
    reader_guard: Option<Rc<CommittedGenerationState>>,
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
    allow_checkpoint: bool,
    persistent_index: Option<HashMap<StorageKey, TableLocation>>,
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
        allow_checkpoint: bool,
    ) -> Result<Self> {
        config.validate()?;
        Self::open_with_validated_config(config, [0; 16], resource_guard, allow_checkpoint).await
    }

    pub(crate) async fn open_with_validated_config(
        config: Config,
        _storage_key_id: [u8; 16],
        resource_guard: Arc<ResourceGuard>,
        allow_checkpoint: bool,
    ) -> Result<Self> {
        // The simulated backend has no durable files. Keep its completion-only
        // workers ephemeral even though startup grants the checkpoint seam.
        let allow_checkpoint = allow_checkpoint && storage_backend::USES_PHYSICAL_STORAGE;
        if storage_backend::USES_PHYSICAL_STORAGE && !allow_checkpoint {
            let running_marker = config
                .data_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(storage_backend::RUNNING_MARKER_FILE);
            if running_marker.exists() {
                return Err(KvError::Worker(format!(
                    "unclean storage run detected at {}; committed-SG recovery is unavailable, so refusing to expose an empty Table; remove or repopulate the storage only after an offline recovery decision",
                    running_marker.display()
                )));
            }
        }
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
        let mut table = Table::new(&config)?;
        let logical_id_capacity = config.logical_sg_capacity()?;
        let mut directory = SgDirectory::new(logical_id_capacity)?;
        let mut generation_log =
            GenerationLog::new(capacity, config.segment_size, config.blob_segment_size)?;
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
        let mut large_value_log = LargeValueLog::new(config.large_value_capacity)?;
        let checkpoint = allow_checkpoint
            .then(|| load_checkpoint(&config))
            .transpose()?
            .flatten();
        let mut persistent_index = allow_checkpoint.then(HashMap::new);
        let mut next_sequence = 0_u64;
        let mut recovered_live_keys = 0_usize;
        if let Some(checkpoint) = checkpoint {
            for committed in checkpoint.generations {
                generation_log.restore(committed.location)?;
                if let Some(location) = committed.large_value_location {
                    large_value_log.restore(location)?;
                }
                next_sequence = next_sequence.max(committed.sequence.saturating_add(1));
                directory.restore_stable(committed)?;
            }
            for (storage_key, table_location) in checkpoint.index {
                if !directory.is_stable(table_location.sg_index) {
                    return Err(KvError::Worker(format!(
                        "checkpoint Item points at non-stable logical SG {}",
                        table_location.sg_index
                    )));
                }
                table.insert(&storage_key, table_location)?;
                persistent_index
                    .as_mut()
                    .expect("persistent checkpoint index is enabled")
                    .insert(storage_key, table_location);
                recovered_live_keys = recovered_live_keys.saturating_add(1);
            }
        }
        let mut mutable = Vec::with_capacity(config.mutable_segment_count);
        for lane in 0..config.mutable_segment_count {
            let logical_sg_id = directory.allocate_mutable(lane)?;
            let sequence = next_sequence;
            next_sequence = next_sequence.saturating_add(1);
            mutable.push(Some(MutableGeneration {
                logical_sg_id,
                sequence,
                segment: MutableSegment::new(&config, logical_sg_id as usize),
                blob_arena: BlobArena::new(config.blob_segment_size),
                large_value_arena: BlobArena::new(config.large_value_capacity),
            }));
        }
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
            live_keys: recovered_live_keys,
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
            allow_checkpoint,
            persistent_index,
        })
    }

    pub(crate) async fn checkpoint(&self) -> Result<()> {
        if !self.allow_checkpoint {
            return Ok(());
        }
        let Some(index) = &self.persistent_index else {
            return Ok(());
        };
        let mut generations = Vec::new();
        for location in self.generation_log.locations() {
            let (_, state) = self
                .directory
                .stable_states()
                .find(|(logical_sg_id, _)| *logical_sg_id == location.logical_sg_id)
                .ok_or_else(|| {
                    KvError::Worker(format!(
                        "cannot checkpoint logical SG {} before it is stable",
                        location.logical_sg_id
                    ))
                })?;
            generations.push(CheckpointGeneration {
                sequence: state.sequence,
                location: state.location,
                large_value_location: state.large_value_location,
            });
        }
        let bytes = encode_checkpoint(&self.config, &generations, index)?;
        let path = checkpoint_path(&self.config);
        let temporary = path.with_extension("checkpoint.tmp");
        let write_result = (|| -> std::io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)?;
            if let Some(parent) = path.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary);
            return Err(startup_io_error("writing the storage checkpoint", error));
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
