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
use openkache_protocol::{SetCondition, SetOptions};

use super::*;
use crate::types::StoredItemValue;

struct MutableGeneration {
    logical_sg_id: u32,
    sequence: u64,
    segment: MutableSegment,
    blob_arena: BlobArena,
    large_value_arena: BlobArena,
}

#[derive(Clone, Copy)]
enum MutableValueHandle {
    Blob { lane: usize, handle: BlobHandle },
    Large { lane: usize, handle: BlobHandle },
}

struct LocatedItem {
    table_location: TableLocation,
    item: Item,
    backing: ReadBacking,
    sequence: u64,
}

#[derive(Default)]
struct DirectStoreIo {
    data_written: Cell<u64>,
    data_read: Cell<u64>,
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
    sealed_flushes: VecDeque<PreparedFlush>,
    inflight_flushes: FuturesUnordered<FlushFuture>,
    eviction: Option<EvictionWork>,
    next_sequence: u64,
    live_keys: usize,
    resource_guard: Arc<ResourceGuard>,
    next_memory_capacity_check: Instant,
    io: DirectStoreIo,
    bucket_read_pool: DirectIoBufferPool,
    pub(crate) segment_flushes: u64,
    pub(crate) segment_capacity_flushes: u64,
    pub(crate) segment_sync_flushes: u64,
    pub(crate) segment_reuses: u64,
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
        Ok(Self {
            config,
            data,
            large_values,
            table,
            directory,
            mutable,
            generation_log,
            large_value_log,
            sealed_flushes: VecDeque::new(),
            inflight_flushes: FuturesUnordered::new(),
            eviction: None,
            next_sequence,
            live_keys: 0,
            resource_guard,
            next_memory_capacity_check: Instant::now(),
            io: DirectStoreIo::default(),
            bucket_read_pool: DirectIoBufferPool::default(),
            segment_flushes: 0,
            segment_capacity_flushes: 0,
            segment_sync_flushes: 0,
            segment_reuses: 0,
            generation_fill_used_bytes: 0,
            generation_fill_capacity_bytes: 0,
        })
    }

    pub(crate) async fn checkpoint(&self) -> Result<()> {
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) async fn get(&self, storage_key: &StorageKey) -> Result<Option<Vec<u8>>> {
        Ok(self
            .get_encoded(storage_key)
            .await?
            .map(|value| value.bytes))
    }

    pub(crate) async fn get_encoded(
        &self,
        storage_key: &StorageKey,
    ) -> Result<Option<StoredItemValue>> {
        let Some(located) = self.locate_item(storage_key).await? else {
            return Ok(None);
        };
        if !located.item.is_live_at(unix_time_ms()) {
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
                previous_mutable_value,
            )? {
                break location;
            }
            let lane = self.fullest_mutable_lane()?;
            self.flush_lane(lane, SegmentFlushReason::Capacity).await?;
        };
        self.publish_table_location(storage_key, previous_location, new_location)?;
        match (previous_live, true) {
            (false, true) => self.live_keys += 1,
            (true, true) => {}
            _ => unreachable!(),
        }
        Ok(if previous_live {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        })
    }

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
        let new_location = loop {
            if let Some(location) =
                self.try_append_tombstone(*storage_key, previous_mutable_value)?
            {
                break location;
            }
            let lane = self.fullest_mutable_lane()?;
            self.flush_lane(lane, SegmentFlushReason::Capacity).await?;
        };
        self.publish_table_location(*storage_key, Some(previous_location), new_location)?;
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
        previous_mutable_value: Option<MutableValueHandle>,
    ) -> Result<Option<TableLocation>> {
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
            if generation
                .segment
                .choose_bucket(&storage_key, fixed_item_bytes)
                .is_none()
            {
                continue;
            }
            let encoded = if large {
                let handle = if let Some(MutableValueHandle::Large {
                    lane: previous_lane,
                    handle: previous,
                }) = previous_mutable_value
                    && previous_lane == lane
                {
                    generation.large_value_arena.replace(previous, value)?
                } else {
                    match generation.large_value_arena.insert(value) {
                        Ok(handle) => handle,
                        Err(KvError::BlobSegmentFull { .. }) => continue,
                        Err(error) => return Err(error),
                    }
                };
                invalidate_mutable_value(generation, lane, previous_mutable_value, Some(true));
                encode_large_value_handle(handle)
            } else if blob {
                let handle = if let Some(MutableValueHandle::Blob {
                    lane: previous_lane,
                    handle: previous,
                }) = previous_mutable_value
                    && previous_lane == lane
                {
                    generation.blob_arena.replace(previous, value)?
                } else {
                    match generation.blob_arena.insert(value) {
                        Ok(handle) => handle,
                        Err(KvError::BlobSegmentFull { .. }) => continue,
                        Err(error) => return Err(error),
                    }
                };
                invalidate_mutable_value(generation, lane, previous_mutable_value, Some(false));
                encode_blob_handle(handle)
            } else {
                invalidate_mutable_value(generation, lane, previous_mutable_value, None);
                encode_inline_value(value)
            };
            let item = if expires_at_ms == 0 {
                Item::live(storage_key, encoded)
            } else {
                Item::live_expiring(storage_key, encoded, expires_at_ms)
            };
            let location = generation.segment.append(item, true).ok_or_else(|| {
                KvError::Worker("chosen mutable SG Bucket rejected an Item".into())
            })?;
            return Ok(Some(location));
        }
        Ok(None)
    }

    fn try_append_tombstone(
        &mut self,
        storage_key: StorageKey,
        previous_mutable_value: Option<MutableValueHandle>,
    ) -> Result<Option<TableLocation>> {
        for lane in 0..self.mutable.len() {
            let Some(generation) = self.mutable[lane].as_mut() else {
                continue;
            };
            if generation
                .segment
                .choose_bucket(&storage_key, ITEM_FIXED_BYTES)
                .is_none()
            {
                continue;
            }
            invalidate_mutable_value(generation, lane, previous_mutable_value, None);
            return generation
                .segment
                .append(Item::tombstone(storage_key), true)
                .map(Some)
                .ok_or_else(|| {
                    KvError::Worker("chosen mutable SG Bucket rejected a Tombstone".into())
                });
        }
        Ok(None)
    }

    fn mutable_value_handle(&self, located: &LocatedItem) -> Option<MutableValueHandle> {
        let ReadBacking::Mutable { lane } = &located.backing else {
            return None;
        };
        match decode_stored_value(&located.item.value).ok()? {
            StoredValue::Inline(_) => None,
            StoredValue::Blob(blob_ref) => Some(MutableValueHandle::Blob {
                lane: *lane,
                handle: BlobHandle {
                    slot: blob_ref.value_offset,
                    value_len: blob_ref.value_len,
                },
            }),
            StoredValue::Large(value_ref) => Some(MutableValueHandle::Large {
                lane: *lane,
                handle: BlobHandle {
                    slot: value_ref.value_offset,
                    value_len: value_ref.value_len,
                },
            }),
        }
    }

    fn publish_table_location(
        &mut self,
        storage_key: StorageKey,
        previous: Option<TableLocation>,
        replacement: TableLocation,
    ) -> Result<()> {
        match previous {
            Some(previous) if previous == replacement => Ok(()),
            Some(previous)
                if self
                    .table
                    .replace_location(&storage_key, previous, replacement) =>
            {
                Ok(())
            }
            Some(_) | None => self.table.insert(&storage_key, replacement),
        }
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
        while self.active_flush_count() >= self.config.max_flushes_in_flight
            || !self.sealed_flushes.is_empty()
        {
            self.wait_for_background_progress().await?;
        }
        let generation = self
            .mutable
            .get_mut(lane)
            .and_then(Option::take)
            .ok_or_else(|| KvError::Worker(format!("mutable SG lane {lane} is unavailable")))?;
        let logical_sg_id = generation.logical_sg_id;
        let fill_used_bytes = generation.segment.used_bytes() as u64;
        self.directory.seal(
            logical_sg_id,
            RamBacking {
                sequence: generation.sequence,
                segment: generation.segment.bytes,
                blob_arena: generation.blob_arena,
                large_value_arena: generation.large_value_arena,
            },
        )?;
        let ReadBacking::Ram(readable) = self
            .directory
            .read_backing(logical_sg_id)
            .ok_or_else(|| KvError::Worker("sealed SG lost its RAM backing".into()))?
        else {
            unreachable!("a sealed SG is RAM-backed")
        };
        let packed = readable.blob_arena.pack()?;
        let packed_large_values = readable.large_value_arena.pack()?;
        let mut segment_write = readable.segment.clone();
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
            logical_sg_id,
            reason,
            fill_used_bytes,
            blob_logical_len,
            blob_write,
            blob_physical_len,
            large_value_logical_len,
            large_value_write,
            large_value_physical_len,
            segment_write,
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
        self.advance_flushes()?;
        self.drive_background_once().await?;
        Ok(())
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
        self.directory.publish_stable(
            completion.logical_sg_id,
            completion.location,
            completion.large_value_location,
        )?;
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
        !self.sealed_flushes.is_empty()
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
        self.sealed_flushes.len() + self.inflight_flushes.len()
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
            let buffer =
                result.map_err(|error| storage_operation_error(&self.resource_guard, error))?;
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
                self.clean_eviction_bucket(eviction.victim.logical_sg_id, eviction.now_ms, extent)?;
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
            if self.table.remove(&item.storage_key, location) && item.is_live_at(now_ms) {
                self.live_keys = self.live_keys.saturating_sub(1);
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
                ReadBacking::Mutable { lane } => self.mutable[*lane]
                    .as_ref()
                    .map_or(0, |generation| generation.sequence),
                ReadBacking::Ram(backing) => backing.sequence,
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
            ReadBacking::Mutable { lane } => {
                let Some(generation) = self.mutable[*lane].as_ref() else {
                    return Ok(None);
                };
                let start = bucket_index * BUCKET_BYTES;
                Ok(find_item_in_bucket(
                    &generation.segment.bytes[start..start + BUCKET_BYTES],
                    storage_key,
                ))
            }
            ReadBacking::Ram(backing) => {
                let start = bucket_index * BUCKET_BYTES;
                Ok(find_item_in_bucket(
                    &backing.segment[start..start + BUCKET_BYTES],
                    storage_key,
                ))
            }
            ReadBacking::Ssd(backing) => {
                let bytes = read_exact_direct(
                    &self.data,
                    self.bucket_read_pool.take_bucket(),
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
                self.bucket_read_pool.recycle_bucket(bytes);
                Ok(item)
            }
        }
    }

    async fn read_value(&self, mut encoded: Vec<u8>, backing: ReadBacking) -> Result<Vec<u8>> {
        match backing {
            ReadBacking::Mutable { lane } => match decode_stored_value(&encoded)? {
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
            ReadBacking::Ram(backing) => match decode_stored_value(&encoded)? {
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
            "keys={} stable_keys={} pending_items=0 pending_value_bytes=0 mutable_sgs={} inflight_flushes={} max_flushes_in_flight={} blob_staging_live_bytes={} large_value_staging_live_bytes={} table_load={:.2}% table_memory={:.2}MiB modeled_resident={:.2}MiB flushes={} capacity_flushes={} sync_flushes={} segment_reuses={} generation_fill_percent={:.3}% generation_fill_used_bytes={} generation_fill_capacity_bytes={} memory_stop_writes={} storage_stop_writes={} rejected_writes={} data_read={} data_written={} blob_data_read={} blob_data_written=0",
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
            fill,
            self.generation_fill_used_bytes,
            self.generation_fill_capacity_bytes,
            self.resource_guard.memory_stop_writes(),
            self.resource_guard.storage_stop_writes(),
            self.resource_guard.rejected_writes(),
            self.io.data_read.get(),
            self.io.data_written.get(),
            self.io.data_read.get(),
        )
    }

    pub(super) fn memory_bytes(&self) -> usize {
        self.table.memory_bytes()
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

fn invalidate_mutable_value(
    generation: &mut MutableGeneration,
    lane: usize,
    previous: Option<MutableValueHandle>,
    retained_large_tier: Option<bool>,
) {
    match previous {
        Some(MutableValueHandle::Blob {
            lane: previous_lane,
            handle,
        }) if previous_lane == lane && retained_large_tier != Some(false) => {
            generation.blob_arena.invalidate(handle);
        }
        Some(MutableValueHandle::Large {
            lane: previous_lane,
            handle,
        }) if previous_lane == lane && retained_large_tier != Some(true) => {
            generation.large_value_arena.invalidate(handle);
        }
        _ => {}
    }
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
    let read_max_time_us = config.read_max_time_us;
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
