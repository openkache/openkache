//! Core cache engine and its Segment lifecycle.
//!
//! A bounded pending map coalesces the latest writes before flush. Live Items
//! and Tombstones form a circular SG log; each SG is reused with its paired
//! dense Blob generation.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use compio::BufResult;
use compio::buf::{IntoInner, IoBuf, IoBufMut, SetLen};
use compio::driver::AsRawFd;
use compio::fs::{File, OpenOptions};
use compio::io::{AsyncReadAt, AsyncWriteAt};
use futures_util::future::join_all;
use openkache_protocol::{SetCondition, SetOptions};

use crate::types::EncodedValue;
use crate::*;

mod blob;
mod bucket;
mod recovery;
mod segment_io;

pub(crate) use self::blob::*;
pub(crate) use self::bucket::*;
pub(crate) use self::recovery::*;

const CAPACITY_CHECK_INTERVAL: Duration = Duration::from_millis(100);
const FILE_RESERVATION_RETRY_DELAYS: [Duration; 6] = [
    Duration::from_millis(1),
    Duration::from_millis(2),
    Duration::from_millis(4),
    Duration::from_millis(8),
    Duration::from_millis(16),
    Duration::from_millis(32),
];
const STORAGE_RESERVE_PERCENT: u64 = 5;
/// Retains at most 1 MiB of idle 4 KiB read buffers per storage worker.
pub(crate) const BUCKET_READ_POOL_CAPACITY: usize = 256;

#[repr(C, align(4096))]
#[derive(Clone, Debug, Eq, PartialEq)]
struct DirectIoPage([u8; BUCKET_BYTES]);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirectIoBuffer {
    pages: Vec<DirectIoPage>,
    initialized_len: usize,
}

impl DirectIoBuffer {
    pub(crate) fn zeroed(len: usize) -> Self {
        assert!(len > 0 && len.is_multiple_of(BUCKET_BYTES));
        Self {
            pages: (0..len / BUCKET_BYTES)
                .map(|_| DirectIoPage([0; BUCKET_BYTES]))
                .collect(),
            initialized_len: len,
        }
    }

    pub(crate) fn for_read(len: usize) -> Self {
        let mut buffer = Self::zeroed(len);
        buffer.initialized_len = 0;
        buffer
    }

    fn capacity(&self) -> usize {
        self.pages.len() * BUCKET_BYTES
    }

    fn as_ptr(&self) -> *const u8 {
        self.pages.as_ptr().cast()
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.pages.as_mut_ptr().cast()
    }
}

impl Deref for DirectIoBuffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        // SAFETY: every DirectIoPage byte is initialized when allocated, and
        // initialized_len never exceeds the allocation.
        unsafe { std::slice::from_raw_parts(self.as_ptr(), self.initialized_len) }
    }
}

impl DerefMut for DirectIoBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: the allocation is exclusively borrowed and initialized_len
        // never exceeds its capacity.
        unsafe { std::slice::from_raw_parts_mut(self.as_mut_ptr(), self.initialized_len) }
    }
}

impl IoBuf for DirectIoBuffer {
    fn as_init(&self) -> &[u8] {
        self
    }
}

impl IoBufMut for DirectIoBuffer {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let capacity = self.capacity();
        // SAFETY: the contiguous page allocation contains capacity bytes.
        // Treating initialized bytes as MaybeUninit is permitted.
        unsafe {
            std::slice::from_raw_parts_mut(self.as_mut_ptr().cast::<MaybeUninit<u8>>(), capacity)
        }
    }
}

impl SetLen for DirectIoBuffer {
    unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= self.capacity());
        self.initialized_len = len;
    }
}

#[derive(Default)]
pub(crate) struct DirectIoBufferPool {
    pub(crate) buffers: RefCell<Vec<DirectIoBuffer>>,
}

impl DirectIoBufferPool {
    pub(crate) fn take_bucket(&self) -> DirectIoBuffer {
        self.buffers
            .borrow_mut()
            .pop()
            .unwrap_or_else(|| DirectIoBuffer::for_read(BUCKET_BYTES))
    }

    pub(crate) fn recycle_bucket(&self, mut buffer: DirectIoBuffer) {
        if buffer.capacity() != BUCKET_BYTES {
            return;
        }
        buffer.initialized_len = 0;
        let mut buffers = self.buffers.borrow_mut();
        if buffers.len() < BUCKET_READ_POOL_CAPACITY {
            buffers.push(buffer);
        }
    }
}

pub(crate) async fn open_direct_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .custom_flags(libc::O_DIRECT)
        .open(path)
        .await
}

pub(crate) struct ResourceGuard {
    directory: std::path::PathBuf,
    memory_stop_available_bytes: u64,
    memory_resume_available_bytes: u64,
    storage_stop_available_bytes: u64,
    storage_resume_available_bytes: u64,
    memory_stop_writes: AtomicBool,
    storage_stop_writes: AtomicBool,
    rejected_writes: AtomicU64,
}

impl ResourceGuard {
    pub(crate) fn for_app_config(config: &AppConfig) -> Result<Self> {
        let workers = (0..config.runtime.thread_count)
            .map(|thread_id| config.worker_config(thread_id))
            .collect::<Vec<_>>();
        Self::new(&config.storage.directory, &workers)
    }

    pub(crate) fn for_worker_config(config: &Config) -> Result<Self> {
        let directory = config
            .data_path
            .parent()
            .ok_or_else(|| {
                KvError::InvalidConfig("storage data path has no parent directory".into())
            })?
            .to_path_buf();
        Self::new(&directory, std::slice::from_ref(config))
    }

    fn new(directory: &Path, workers: &[Config]) -> Result<Self> {
        let memory =
            crate::sizing::automatic_memory_capacity(crate::sizing::detected_memory_snapshot()?)?;
        let table_memory_bytes = workers.iter().try_fold(0u64, |total, worker| {
            total
                .checked_add(Table::modeled_memory_bytes(worker)? as u64)
                .ok_or_else(|| KvError::InvalidConfig("host Table memory size overflowed".into()))
        })?;
        let table_memory_budget_bytes = memory.budget_bytes / 2;
        if table_memory_bytes > table_memory_budget_bytes {
            return Err(KvError::InvalidConfig(format!(
                "modeled Table requires {table_memory_bytes} bytes but current safe Table budget is {table_memory_budget_bytes} bytes"
            )));
        }
        let storage_available_bytes = crate::sizing::filesystem_available_bytes(directory)?;
        let storage_allocated_bytes = workers.iter().try_fold(0u64, |total, worker| {
            let data = allocated_file_bytes(&worker.data_path)?;
            let blob = allocated_file_bytes(&worker.blob_path())?;
            total
                .checked_add(data)
                .and_then(|value| value.checked_add(blob))
                .ok_or_else(|| KvError::InvalidConfig("allocated storage size overflowed".into()))
        })?;
        let generation_bytes = workers.iter().try_fold(0u64, |total, worker| {
            let worker_generation = (worker.segment_size as u64)
                .checked_add(worker.blob_segment_size as u64)
                .and_then(|value| value.checked_add(BUCKET_BYTES as u64))
                .ok_or_else(|| {
                    KvError::InvalidConfig("storage generation size overflowed".into())
                })?;
            total.checked_add(worker_generation).ok_or_else(|| {
                KvError::InvalidConfig("host storage generation size overflowed".into())
            })
        })?;
        let (storage_stop_available_bytes, storage_resume_available_bytes) =
            storage_capacity_thresholds(
                storage_available_bytes,
                storage_allocated_bytes,
                generation_bytes,
            )?;

        Ok(Self {
            directory: directory.to_path_buf(),
            memory_stop_available_bytes: memory.reserve_bytes,
            memory_resume_available_bytes: memory
                .reserve_bytes
                .saturating_add(memory.reserve_bytes / 4)
                .min(memory.available_bytes),
            storage_stop_available_bytes,
            storage_resume_available_bytes,
            memory_stop_writes: AtomicBool::new(false),
            storage_stop_writes: AtomicBool::new(
                storage_available_bytes <= storage_stop_available_bytes,
            ),
            rejected_writes: AtomicU64::new(0),
        })
    }

    fn admit_set(&self, refresh_memory: bool) -> Result<()> {
        let memory_stopped = self.memory_stop_writes.load(Ordering::Acquire);
        if refresh_memory || memory_stopped {
            let available = crate::sizing::detected_memory_snapshot()?.available_bytes;
            self.memory_stop_writes.store(
                capacity_stop_writes(
                    memory_stopped,
                    available,
                    self.memory_stop_available_bytes,
                    self.memory_resume_available_bytes,
                ),
                Ordering::Release,
            );
        }
        if self.memory_stop_writes.load(Ordering::Acquire) {
            self.rejected_writes.fetch_add(1, Ordering::Relaxed);
            return Err(KvError::CapacityExhausted { resource: "memory" });
        }

        if self.storage_stop_writes.load(Ordering::Acquire) {
            let available = crate::sizing::filesystem_available_bytes(&self.directory)?;
            let stopped = capacity_stop_writes(
                true,
                available,
                self.storage_stop_available_bytes,
                self.storage_resume_available_bytes,
            );
            self.storage_stop_writes.store(stopped, Ordering::Release);
            if stopped {
                self.rejected_writes.fetch_add(1, Ordering::Relaxed);
                return Err(KvError::CapacityExhausted {
                    resource: "storage",
                });
            }
        }
        Ok(())
    }

    fn observe_storage_reservation(&self) -> Result<()> {
        let available = crate::sizing::filesystem_available_bytes(&self.directory)?;
        if available <= self.storage_stop_available_bytes {
            self.storage_stop_writes.store(true, Ordering::Release);
        }
        Ok(())
    }

    fn mark_storage_exhausted(&self) {
        self.storage_stop_writes.store(true, Ordering::Release);
    }

    fn memory_stop_writes(&self) -> bool {
        self.memory_stop_writes.load(Ordering::Acquire)
    }

    fn storage_stop_writes(&self) -> bool {
        self.storage_stop_writes.load(Ordering::Acquire)
    }

    fn rejected_writes(&self) -> u64 {
        self.rejected_writes.load(Ordering::Relaxed)
    }
}

pub(crate) const fn capacity_stop_writes(
    stopped: bool,
    available_bytes: u64,
    stop_available_bytes: u64,
    resume_available_bytes: u64,
) -> bool {
    if stopped {
        available_bytes < resume_available_bytes
    } else {
        available_bytes <= stop_available_bytes
    }
}

pub(crate) fn storage_capacity_thresholds(
    available_bytes: u64,
    allocated_bytes: u64,
    generation_bytes: u64,
) -> Result<(u64, u64)> {
    let managed_storage_bytes = available_bytes
        .checked_add(allocated_bytes)
        .ok_or_else(|| KvError::InvalidConfig("managed storage size overflowed".into()))?;
    let proportional_reserve = managed_storage_bytes
        .checked_mul(STORAGE_RESERVE_PERCENT)
        .map(|bytes| bytes / 100)
        .ok_or_else(|| KvError::InvalidConfig("storage reserve size overflowed".into()))?;
    let generation_reserve = generation_bytes
        .checked_mul(2)
        .ok_or_else(|| KvError::InvalidConfig("storage generation reserve overflowed".into()))?;
    let reserve_bytes = proportional_reserve.max(generation_reserve);
    if reserve_bytes >= managed_storage_bytes {
        return Err(KvError::InvalidConfig(
            "available storage cannot preserve two complete Segment generations".into(),
        ));
    }
    Ok(((reserve_bytes / 2).max(generation_bytes), reserve_bytes))
}

fn allocated_file_bytes(path: &Path) -> Result<u64> {
    match fs::metadata(path) {
        Ok(metadata) => metadata
            .blocks()
            .checked_mul(512)
            .ok_or_else(|| KvError::InvalidConfig("allocated file size overflowed".into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn reserve_file_range(file: &File, offset: u64, len: u64) -> std::io::Result<()> {
    if len == 0 {
        return Ok(());
    }
    let offset = i64::try_from(offset).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "offset is too large")
    })?;
    let len = i64::try_from(len).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "length is too large")
    })?;
    // Blocking tasks continue after their join handle is dropped, so retain an
    // independently owned descriptor in case the awaiting operation is cancelled.
    let file_descriptor =
        unsafe { std::os::fd::BorrowedFd::borrow_raw(file.as_raw_fd()) }.try_clone_to_owned()?;
    compio::runtime::spawn_blocking(move || {
        reserve_with_transient_retry(
            || unsafe { libc::posix_fallocate(file_descriptor.as_raw_fd(), offset, len) },
            std::thread::sleep,
        )
    })
    .await
    .map_err(std::io::Error::from)?
}

pub(crate) fn reserve_with_transient_retry(
    mut reserve: impl FnMut() -> i32,
    mut wait: impl FnMut(Duration),
) -> std::io::Result<()> {
    for delay in FILE_RESERVATION_RETRY_DELAYS {
        let result = reserve();
        if result == 0 {
            return Ok(());
        }
        if result != libc::EAGAIN && result != libc::EINTR {
            return Err(std::io::Error::from_raw_os_error(result));
        }
        wait(delay);
    }
    let result = reserve();
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(result))
    }
}

fn storage_io_error(guard: &ResourceGuard, error: std::io::Error) -> KvError {
    if error
        .raw_os_error()
        .is_some_and(|code| code == libc::ENOSPC || code == libc::EDQUOT)
    {
        guard.mark_storage_exhausted();
        KvError::CapacityExhausted {
            resource: "storage",
        }
    } else {
        error.into()
    }
}

fn storage_operation_error(guard: &ResourceGuard, error: KvError) -> KvError {
    match error {
        KvError::Io(error) => storage_io_error(guard, error),
        error => error,
    }
}

fn validate_direct_io_progress(operation: &str, completed: usize, remaining: usize) -> Result<()> {
    if completed == 0 || completed > remaining {
        return Err(KvError::Worker(format!(
            "invalid direct {operation} progress: completed {completed} with {remaining} bytes remaining"
        )));
    }
    if completed < remaining && !completed.is_multiple_of(BUCKET_BYTES) {
        return Err(KvError::Worker(format!(
            "unaligned short direct {operation}: completed {completed} with {remaining} bytes remaining"
        )));
    }
    Ok(())
}

fn is_transient_io_error(error: &std::io::Error) -> bool {
    error
        .raw_os_error()
        .is_some_and(|code| code == libc::EAGAIN || code == libc::EINTR)
}

fn direct_io_timeout(
    started: Instant,
    timeout: Duration,
    operation: &'static str,
) -> Result<Duration> {
    let remaining = timeout.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        Err(KvError::Timeout(operation))
    } else {
        Ok(remaining)
    }
}

pub(crate) async fn read_exact_direct(
    file: &File,
    mut buffer: DirectIoBuffer,
    offset: u64,
    len: usize,
    timeout_us: u64,
    operation: &'static str,
) -> Result<DirectIoBuffer> {
    let timeout = Duration::from_micros(timeout_us);
    let started = Instant::now();
    let mut completed = 0usize;
    while completed < len {
        let read_offset = offset
            .checked_add(completed as u64)
            .ok_or_else(|| KvError::Worker(format!("{operation} offset overflowed")))?;
        let read = file.read_at(buffer.slice(completed..len), read_offset);
        let BufResult(result, returned) =
            compio::runtime::time::timeout(direct_io_timeout(started, timeout, operation)?, read)
                .await
                .map_err(|_| KvError::Timeout(operation))?;
        let read_bytes = match result {
            Ok(read_bytes) => read_bytes,
            Err(error) if is_transient_io_error(&error) => {
                buffer = returned.into_inner();
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        validate_direct_io_progress(operation, read_bytes, len - completed)?;
        completed += read_bytes;
        buffer = returned.into_inner();
    }
    Ok(buffer)
}

pub(crate) async fn write_all_direct(
    mut file: &File,
    mut buffer: DirectIoBuffer,
    offset: u64,
    len: usize,
    timeout_us: u64,
    operation: &'static str,
) -> Result<DirectIoBuffer> {
    let timeout = Duration::from_micros(timeout_us);
    let started = Instant::now();
    let mut completed = 0usize;
    while completed < len {
        let write_offset = offset
            .checked_add(completed as u64)
            .ok_or_else(|| KvError::Worker(format!("{operation} offset overflowed")))?;
        let write = file.write_at(buffer.slice(completed..len), write_offset);
        let BufResult(result, returned) =
            compio::runtime::time::timeout(direct_io_timeout(started, timeout, operation)?, write)
                .await
                .map_err(|_| KvError::Timeout(operation))?;
        let written = match result {
            Ok(written) => written,
            Err(error) if is_transient_io_error(&error) => {
                buffer = returned.into_inner();
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        validate_direct_io_progress(operation, written, len - completed)?;
        completed += written;
        buffer = returned.into_inner();
    }
    Ok(buffer)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetOutcome {
    Created,
    Replaced,
    NotStored,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct KvkacheIoStats {
    pub(crate) data_written: u64,
    pub(crate) data_read: u64,
    pub(crate) blob_data_written: u64,
    pub(crate) blob_data_read: u64,
}

#[derive(Default)]
struct IoCounters {
    data_written: Cell<u64>,
    data_read: Cell<u64>,
    blob_data_written: Cell<u64>,
    blob_data_read: Cell<u64>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SegmentFlushReason {
    Capacity,
    Sync,
}

struct LocatedItem {
    table_location: TableLocation,
    item: Item,
}

struct LocatedItemState {
    table_location: TableLocation,
    state: ItemState,
}

#[derive(Debug)]
pub(crate) struct PendingItem {
    pub(crate) value: Option<EncodedValue>,
    pub(crate) expires_at_ms: u64,
    pub(crate) previous: Option<TableLocation>,
    pub(crate) previous_live: bool,
}

struct FlushRecord {
    storage_key: StorageKey,
    value: Option<EncodedValue>,
    expires_at_ms: u64,
    previous: Option<TableLocation>,
    previous_live: bool,
    table_location: Option<TableLocation>,
    blob_ref: Option<BlobRef>,
}

struct FlushLane {
    segment: MutableSegment,
    records: Vec<FlushRecord>,
    blob_used: usize,
}

impl FlushLane {
    fn new(segment: MutableSegment, record_capacity: usize) -> Self {
        Self {
            segment,
            records: Vec::with_capacity(record_capacity),
            blob_used: 0,
        }
    }

    fn try_append(&mut self, record: &mut FlushRecord, blob_capacity: usize) -> Result<bool> {
        let (item, blob_ref) = match record.value.as_ref() {
            None => (Item::tombstone(record.storage_key), None),
            Some(value) if is_blob_item(&value.bytes) => {
                let Some(blob_end) = self.blob_used.checked_add(value.bytes.len()) else {
                    return Ok(false);
                };
                if blob_end > blob_capacity {
                    return Ok(false);
                }
                let blob_ref = BlobRef::new(self.blob_used, value.bytes.len())?;
                let encoded = encode_blob_ref(blob_ref, value.flags);
                let item = if record.expires_at_ms == 0 {
                    Item::live(record.storage_key, encoded)
                } else {
                    Item::live_expiring(record.storage_key, encoded, record.expires_at_ms)
                };
                (item, Some(blob_ref))
            }
            Some(value) => {
                let encoded = encode_inline_value(&value.bytes, value.flags);
                let item = if record.expires_at_ms == 0 {
                    Item::live(record.storage_key, encoded)
                } else {
                    Item::live_expiring(record.storage_key, encoded, record.expires_at_ms)
                };
                (item, None)
            }
        };
        let Some(table_location) = self.segment.append(item, false) else {
            return Ok(false);
        };
        if blob_ref.is_some() {
            self.blob_used += record
                .value
                .as_ref()
                .expect("Blob record has a live value")
                .bytes
                .len();
        }
        if let Some(value) = &record.value {
            self.segment.accepted_item_bytes +=
                (crate::types::STORAGE_KEY_BYTES + value.bytes.len()) as u64;
        }
        record.table_location = Some(table_location);
        record.blob_ref = blob_ref;
        Ok(true)
    }
}

pub(crate) struct Kvkache {
    config: Config,
    data: File,
    pub(crate) table: Table,
    pub(crate) blob_segment: BlobSegment,
    pub(crate) pending: HashMap<StorageKey, PendingItem>,
    pub(crate) pending_sg_bytes: usize,
    pub(crate) pending_blob_bytes: usize,
    pub(crate) occupied_segments: Vec<bool>,
    segment_commits: Vec<Option<SegmentCommit>>,
    stable_live_keys: usize,
    next_segment_index: usize,
    next_generation: u64,
    storage_key_id: [u8; 16],
    pub(crate) segment_flushes: u64,
    pub(crate) segment_capacity_flushes: u64,
    pub(crate) segment_sync_flushes: u64,
    pub(crate) segment_fill_used_bytes: u64,
    pub(crate) segment_fill_capacity_bytes: u64,
    pub(crate) segment_capacity_fill_used_bytes: u64,
    pub(crate) segment_capacity_fill_capacity_bytes: u64,
    pub(crate) segment_sync_fill_used_bytes: u64,
    pub(crate) segment_sync_fill_capacity_bytes: u64,
    pub(crate) segment_reuses: u64,
    resource_guard: Arc<ResourceGuard>,
    next_memory_capacity_check: Instant,
    io: IoCounters,
    bucket_read_pool: DirectIoBufferPool,
    pub(crate) mutable_segment_buffers: Vec<DirectIoBuffer>,
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
        storage_key_id: [u8; 16],
        resource_guard: Arc<ResourceGuard>,
        allow_checkpoint: bool,
    ) -> Result<Self> {
        config.validate()?;
        Self::open_with_validated_config(config, storage_key_id, resource_guard, allow_checkpoint)
            .await
    }

    pub(crate) async fn open_with_validated_config(
        config: Config,
        storage_key_id: [u8; 16],
        resource_guard: Arc<ResourceGuard>,
        allow_checkpoint: bool,
    ) -> Result<Self> {
        if let Some(parent) = config.data_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data_exists = config.data_path.exists();
        let blob_exists = config.blob_path().exists();
        if !data_exists && blob_exists {
            return Err(KvError::Worker(
                "Blob storage exists but the Segment file is missing".into(),
            ));
        }
        if data_exists {
            let actual = fs::metadata(&config.data_path)?.len();
            let expected = config.segment_file_bytes()?;
            if actual != expected {
                return Err(KvError::Worker(format!(
                    "Segment file has length {actual}, expected {expected}; legacy files require repopulation"
                )));
            }
        }
        if blob_exists {
            let actual = fs::metadata(config.blob_path())?.len();
            let expected = config.blob_bytes();
            if actual != expected {
                return Err(KvError::Worker(format!(
                    "Blob file has length {actual}, expected {expected}"
                )));
            }
        }
        let mut data = open_direct_file(&config.data_path).await?;
        if !data_exists {
            initialize_segment_file(&mut data, &config, storage_key_id).await?;
        }
        let checkpoint = if allow_checkpoint && data_exists {
            load_table_checkpoint(&config, storage_key_id).await?
        } else {
            None
        };
        let (table, recovery_state, stable_live_keys, recovered_from_checkpoint) =
            if let Some(checkpoint) = checkpoint {
                validate_segment_file(&data, &config, storage_key_id).await?;
                (
                    checkpoint.table,
                    checkpoint.recovery_state,
                    checkpoint.stable_live_keys,
                    true,
                )
            } else {
                (
                    Table::new(&config)?,
                    recover_state(&data, &config, storage_key_id).await?,
                    0,
                    false,
                )
            };
        if !blob_exists && !recovery_state.commits.is_empty() {
            return Err(KvError::Worker(
                "committed Segment state exists but the Blob file is missing".into(),
            ));
        }
        let blob_segment = BlobSegment::open(&config).await?;
        let next_segment_index = recovery_state.next_segment_index;
        let next_generation = recovery_state.next_generation;
        let mutable_segment_buffers = Vec::with_capacity(config.ram_segment_count);

        let mut cache = Self {
            table,
            blob_segment,
            pending: HashMap::new(),
            pending_sg_bytes: 0,
            pending_blob_bytes: 0,
            occupied_segments: vec![false; config.segment_count],
            segment_commits: vec![None; config.segment_count],
            stable_live_keys,
            next_segment_index,
            next_generation,
            storage_key_id,
            segment_flushes: 0,
            segment_capacity_flushes: 0,
            segment_sync_flushes: 0,
            segment_fill_used_bytes: 0,
            segment_fill_capacity_bytes: 0,
            segment_capacity_fill_used_bytes: 0,
            segment_capacity_fill_capacity_bytes: 0,
            segment_sync_fill_used_bytes: 0,
            segment_sync_fill_capacity_bytes: 0,
            segment_reuses: 0,
            resource_guard,
            next_memory_capacity_check: Instant::now(),
            io: IoCounters::default(),
            bucket_read_pool: DirectIoBufferPool::default(),
            mutable_segment_buffers,
            config,
            data,
        };
        if recovered_from_checkpoint {
            cache.restore_commit_state(&recovery_state.commits)?;
        } else {
            cache.recover(recovery_state.commits).await?;
        }
        Ok(cache)
    }

    async fn recover(&mut self, commits: Vec<SegmentCommit>) -> Result<()> {
        self.restore_commit_state(&commits)?;
        let mut buffer = None;
        for commit in commits.into_iter().rev() {
            buffer = Some(self.recover_segment(commit, buffer.take()).await?);
        }
        Ok(())
    }

    fn restore_commit_state(&mut self, commits: &[SegmentCommit]) -> Result<()> {
        for commit in commits {
            self.occupied_segments[commit.sg_index] = true;
            self.segment_commits[commit.sg_index] = Some(*commit);
            self.blob_segment
                .recover_segment(commit.sg_index, commit.blob_logical_len)?;
        }
        Ok(())
    }

    async fn recovered_key_exists(&self, storage_key: &StorageKey) -> Result<bool> {
        Ok(self.locate_stable_state(storage_key).await?.is_some())
    }

    pub(crate) async fn checkpoint(&self) -> Result<()> {
        if !self.pending.is_empty() {
            return Err(KvError::Worker(
                "Table checkpoint requires an empty pending map".into(),
            ));
        }
        write_table_checkpoint(
            &self.config,
            self.storage_key_id,
            &self.table,
            self.stable_live_keys,
            &self.segment_commits,
            self.next_segment_index,
            self.next_generation,
        )
        .await
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
    ) -> Result<Option<EncodedValue>> {
        if let Some(pending) = self.pending.get(storage_key) {
            return Ok(pending
                .is_live_at(unix_time_ms())
                .then(|| pending.value.clone())
                .flatten());
        }
        let Some(located) = self.locate_stable_record(storage_key).await? else {
            return Ok(None);
        };
        if !located.item.is_live_at(unix_time_ms()) {
            return Ok(None);
        }
        self.read_stored_value(located.table_location, located.item.value)
            .await
            .map(Some)
    }

    #[allow(dead_code)]
    pub(crate) async fn set(
        &mut self,
        storage_key: StorageKey,
        value: &[u8],
    ) -> Result<SetOutcome> {
        self.set_encoded(storage_key, EncodedValue::plain(value.to_vec()))
            .await
    }

    pub(crate) async fn set_encoded(
        &mut self,
        storage_key: StorageKey,
        value: EncodedValue,
    ) -> Result<SetOutcome> {
        self.set_encoded_with_options(storage_key, value, SetOptions::NONE)
            .await
    }

    pub(crate) async fn set_encoded_with_options(
        &mut self,
        storage_key: StorageKey,
        value: EncodedValue,
        options: SetOptions,
    ) -> Result<SetOutcome> {
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
        let expires_at_ms = match options.ttl_ms {
            Some(ttl_ms) => now_ms.checked_add(ttl_ms).ok_or_else(|| {
                KvError::InvalidRequest("SET TTL exceeds the supported time range".into())
            })?,
            None => 0,
        };
        let (previous, previous_live, outcome) =
            if let Some(current) = self.pending.get(&storage_key) {
                let current_live = current.is_live_at(now_ms);
                if !set_condition_allows(options.condition, current_live) {
                    return Ok(SetOutcome::NotStored);
                }
                let pending = self
                    .take_pending(&storage_key)
                    .expect("pending Item remains present");
                (
                    pending.previous,
                    pending.previous_live,
                    if current_live {
                        SetOutcome::Replaced
                    } else {
                        SetOutcome::Created
                    },
                )
            } else {
                let previous = self.locate_stable_state(&storage_key).await?;
                let current_live = previous
                    .as_ref()
                    .is_some_and(|located| located.state.is_live_at(now_ms));
                let previous_stable_live = previous
                    .as_ref()
                    .is_some_and(|located| !located.state.is_tombstone());
                if !set_condition_allows(options.condition, current_live) {
                    return Ok(SetOutcome::NotStored);
                }
                let location = previous.as_ref().map(|located| located.table_location);
                (
                    location,
                    previous_stable_live,
                    if current_live {
                        SetOutcome::Replaced
                    } else {
                        SetOutcome::Created
                    },
                )
            };
        self.insert_pending(
            storage_key,
            PendingItem {
                value: Some(value),
                expires_at_ms,
                previous,
                previous_live,
            },
        );
        if self.pending_should_flush() {
            self.flush_pending(SegmentFlushReason::Capacity).await?;
        }
        Ok(outcome)
    }

    pub(crate) async fn delete(&mut self, storage_key: &StorageKey) -> Result<bool> {
        let now_ms = unix_time_ms();
        if let Some(current) = self.pending.get(storage_key)
            && !current.is_live_at(now_ms)
        {
            let mut pending = self
                .take_pending(storage_key)
                .expect("pending Item remains present");
            if pending.previous.is_some() {
                pending.value = None;
                pending.expires_at_ms = 0;
                self.insert_pending(*storage_key, pending);
            }
            return Ok(false);
        }
        if let Some(mut pending) = self.take_pending(storage_key) {
            if pending.value.is_none() {
                self.insert_pending(*storage_key, pending);
                return Ok(false);
            }
            if pending.previous.is_some() {
                pending.value = None;
                pending.expires_at_ms = 0;
                self.insert_pending(*storage_key, pending);
            }
            if self.pending_should_flush() {
                self.flush_pending(SegmentFlushReason::Capacity).await?;
            }
            return Ok(true);
        }
        let Some(previous) = self.locate_stable_state(storage_key).await? else {
            return Ok(false);
        };
        if !previous.state.is_live_at(now_ms) {
            return Ok(false);
        }
        self.insert_pending(
            *storage_key,
            PendingItem {
                value: None,
                expires_at_ms: 0,
                previous: Some(previous.table_location),
                previous_live: true,
            },
        );
        if self.pending_should_flush() {
            self.flush_pending(SegmentFlushReason::Capacity).await?;
        }
        Ok(true)
    }

    pub(crate) async fn sync(&mut self) -> Result<()> {
        // Every flushed generation persists its data before its commit page;
        // an empty pending map therefore has no outstanding durable work.
        self.flush_pending(SegmentFlushReason::Sync).await
    }

    async fn locate_stable_record(&self, storage_key: &StorageKey) -> Result<Option<LocatedItem>> {
        self.locate_stable_record_with_bucket(storage_key, None)
            .await
    }

    async fn locate_stable_record_with_bucket(
        &self,
        storage_key: &StorageKey,
        scanned: Option<(TableLocation, &[u8])>,
    ) -> Result<Option<LocatedItem>> {
        let locations = self.table.candidate_locations(storage_key);
        if let [table_location] = locations.as_slice() {
            let Some(item) = self
                .read_record_candidate(storage_key, *table_location, scanned)
                .await?
            else {
                return Ok(None);
            };
            return Ok(Some(LocatedItem {
                table_location: *table_location,
                item,
            }));
        }
        let mut newest: Option<(usize, LocatedItem)> = None;
        let reads = locations.into_iter().map(|table_location| async move {
            self.read_record_candidate(storage_key, table_location, scanned)
                .await
                .map(|item| (table_location, item))
        });
        for result in join_all(reads).await {
            let (table_location, item) = result?;
            let Some(item) = item else {
                continue;
            };
            let age = self.ssd_segment_age(table_location.sg_index as usize);
            if newest
                .as_ref()
                .is_none_or(|(newest_age, _)| age < *newest_age)
            {
                newest = Some((
                    age,
                    LocatedItem {
                        table_location,
                        item,
                    },
                ));
            }
        }
        Ok(newest.map(|(_, located)| located))
    }

    async fn read_record_candidate(
        &self,
        storage_key: &StorageKey,
        table_location: TableLocation,
        scanned: Option<(TableLocation, &[u8])>,
    ) -> Result<Option<Item>> {
        if let Some((scanned_location, bucket)) = scanned
            && table_location == scanned_location
        {
            Ok(find_item_in_bucket(bucket, storage_key))
        } else {
            self.read_location(storage_key, table_location).await
        }
    }

    async fn locate_stable_state(
        &self,
        storage_key: &StorageKey,
    ) -> Result<Option<LocatedItemState>> {
        let locations = self.table.candidate_locations(storage_key);
        if let [table_location] = locations.as_slice() {
            let Some(state) = self
                .read_location_state(storage_key, *table_location)
                .await?
            else {
                return Ok(None);
            };
            return Ok(Some(LocatedItemState {
                table_location: *table_location,
                state,
            }));
        }
        let mut newest: Option<(usize, LocatedItemState)> = None;
        let reads = locations.into_iter().map(|table_location| async move {
            self.read_location_state(storage_key, table_location)
                .await
                .map(|state| (table_location, state))
        });
        for result in join_all(reads).await {
            let (table_location, state) = result?;
            let Some(state) = state else {
                continue;
            };
            let age = self.ssd_segment_age(table_location.sg_index as usize);
            if newest
                .as_ref()
                .is_none_or(|(newest_age, _)| age < *newest_age)
            {
                newest = Some((
                    age,
                    LocatedItemState {
                        table_location,
                        state,
                    },
                ));
            }
        }
        Ok(newest.map(|(_, located)| located))
    }

    async fn read_stored_value(
        &self,
        table_location: TableLocation,
        mut encoded: Vec<u8>,
    ) -> Result<EncodedValue> {
        let decoded = decode_stored_value(&encoded)?;
        let flags = decoded.flags;
        let blob_ref = match decoded.value {
            StoredValue::Inline(value) => {
                debug_assert_eq!(
                    value.len() + STORED_VALUE_TAG_BYTES,
                    encoded.len(),
                    "decoded inline value excludes only its tag"
                );
                None
            }
            StoredValue::Blob(blob_ref) => Some(blob_ref),
        };
        let bytes = match blob_ref {
            None => {
                remove_stored_value_tag(&mut encoded);
                encoded
            }
            Some(blob_ref) => {
                let value = self
                    .blob_segment
                    .read(table_location.sg_index as usize, blob_ref)
                    .await?;
                let physical_bytes = self.blob_segment.physical_read_bytes(blob_ref);
                self.io
                    .data_read
                    .set(self.io.data_read.get() + physical_bytes);
                self.io
                    .blob_data_read
                    .set(self.io.blob_data_read.get() + physical_bytes);
                value
            }
        };
        Ok(EncodedValue::new(bytes, flags))
    }

    fn ssd_segment_age(&self, sg_index: usize) -> usize {
        let count = self.config.segment_count;
        let newest_ssd = (self.next_segment_index + count - 1) % count;
        (newest_ssd + count - sg_index) % count
    }

    async fn read_location(
        &self,
        storage_key: &StorageKey,
        table_location: TableLocation,
    ) -> Result<Option<Item>> {
        let sg_index = table_location.sg_index as usize;
        if !self
            .occupied_segments
            .get(sg_index)
            .copied()
            .unwrap_or(false)
        {
            return Ok(None);
        }
        let bucket_index = bucket_hash(
            storage_key,
            table_location.bucket_hash_index,
            self.config.bucket_count(),
        );
        let bytes = self.read_bucket(sg_index, bucket_index).await?;
        let item = find_item_in_bucket(&bytes, storage_key);
        self.bucket_read_pool.recycle_bucket(bytes);
        Ok(item)
    }

    async fn read_location_state(
        &self,
        storage_key: &StorageKey,
        table_location: TableLocation,
    ) -> Result<Option<ItemState>> {
        let sg_index = table_location.sg_index as usize;
        if !self
            .occupied_segments
            .get(sg_index)
            .copied()
            .unwrap_or(false)
        {
            return Ok(None);
        }
        let bucket_index = bucket_hash(
            storage_key,
            table_location.bucket_hash_index,
            self.config.bucket_count(),
        );
        let bytes = self.read_bucket(sg_index, bucket_index).await?;
        let state = find_item_state_in_bucket(&bytes, storage_key);
        self.bucket_read_pool.recycle_bucket(bytes);
        Ok(state)
    }

    async fn flush_pending(&mut self, reason: SegmentFlushReason) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let now_ms = unix_time_ms();
        self.pending_sg_bytes = 0;
        self.pending_blob_bytes = 0;
        let mut remaining = Vec::with_capacity(self.pending.len());
        remaining.extend(
            self.pending
                .drain()
                .filter_map(|(storage_key, mut pending)| {
                    if !pending.is_live_at(now_ms) {
                        pending.value = None;
                        pending.expires_at_ms = 0;
                    }
                    (pending.value.is_some() || pending.previous.is_some()).then_some(FlushRecord {
                        storage_key,
                        value: pending.value,
                        expires_at_ms: pending.expires_at_ms,
                        previous: pending.previous,
                        previous_live: pending.previous_live,
                        table_location: None,
                        blob_ref: None,
                    })
                }),
        );
        remaining.sort_unstable_by(|left, right| {
            right
                .value
                .as_ref()
                .map_or(0, |value| value.bytes.len())
                .cmp(&left.value.as_ref().map_or(0, |value| value.bytes.len()))
                .then_with(|| left.storage_key.cmp(&right.storage_key))
        });

        let mut blob_buffer = None;
        let mut control_buffer = None;
        while !remaining.is_empty() {
            let next_generation = match next_sg_generation(self.next_generation) {
                Ok(next_generation) => next_generation,
                Err(error) => {
                    self.restore_flush_records(remaining);
                    return Err(error);
                }
            };
            let sg_index = self.next_segment_index;
            let lane_count = self.config.ram_segment_count;
            let lane_record_capacity = remaining.len().div_ceil(lane_count);
            // The extra lanes are packing lookahead. Only lane zero becomes the next
            // durable generation; overflow remains readable through `pending`.
            let mut lanes = Vec::with_capacity(lane_count);
            for _ in 0..lane_count {
                let segment = match self.mutable_segment_buffers.pop() {
                    Some(bytes) => MutableSegment::reuse(&self.config, sg_index, bytes),
                    None => MutableSegment::new(&self.config, sg_index),
                };
                lanes.push(FlushLane::new(segment, lane_record_capacity));
            }
            let mut deferred = Vec::new();
            for mut record in remaining.drain(..) {
                let mut placed_lane = None;
                let mut planning_error = None;
                for (lane_index, lane) in lanes.iter_mut().enumerate() {
                    match lane.try_append(&mut record, self.config.blob_segment_size) {
                        Ok(true) => {
                            placed_lane = Some(lane_index);
                            break;
                        }
                        Ok(false) => {}
                        Err(error) => {
                            planning_error = Some(error);
                            break;
                        }
                    }
                }
                if let Some(error) = planning_error {
                    let mut restore = Vec::with_capacity(
                        1 + deferred.len()
                            + lanes.iter().map(|lane| lane.records.len()).sum::<usize>(),
                    );
                    restore.push(record);
                    restore.append(&mut deferred);
                    for lane in lanes {
                        restore.extend(lane.records);
                        self.mutable_segment_buffers.push(lane.segment.bytes);
                    }
                    self.restore_flush_records(restore);
                    return Err(error);
                }
                if let Some(lane_index) = placed_lane {
                    lanes[lane_index].records.push(record);
                } else {
                    deferred.push(record);
                }
            }

            let selected = lanes.remove(0);
            for mut lane in lanes {
                for mut record in lane.records.drain(..) {
                    record.table_location = None;
                    record.blob_ref = None;
                    deferred.push(record);
                }
                self.mutable_segment_buffers.push(lane.segment.bytes);
            }
            let active = selected.segment;
            let mut planned = selected.records;
            let blob_used = selected.blob_used;
            if planned.is_empty() {
                self.mutable_segment_buffers.push(active.bytes);
                self.restore_flush_records(deferred);
                return Err(KvError::Worker(
                    "pending Item cannot fit in an empty Segment generation".into(),
                ));
            }
            if let Err(error) = self.reserve_segment_generation(sg_index, blob_used).await {
                self.mutable_segment_buffers.push(active.bytes);
                self.restore_flush_records(planned.into_iter().chain(deferred));
                return Err(error);
            }
            if self.occupied_segments[sg_index] {
                let Some(reclaimed_commit) = self.segment_commits[sg_index] else {
                    self.mutable_segment_buffers.push(active.bytes);
                    self.restore_flush_records(planned.into_iter().chain(deferred));
                    return Err(KvError::Worker(
                        "occupied Segment is missing its commit metadata".into(),
                    ));
                };
                let invalidated = match invalidate_segment(
                    &mut self.data,
                    &self.config,
                    sg_index,
                    control_buffer.take(),
                )
                .await
                {
                    Ok((bytes, buffer)) => {
                        control_buffer = Some(buffer);
                        bytes
                    }
                    Err(error) => {
                        self.mutable_segment_buffers.push(active.bytes);
                        self.restore_flush_records(planned.into_iter().chain(deferred));
                        return Err(storage_operation_error(&self.resource_guard, error));
                    }
                };
                self.io
                    .data_written
                    .set(self.io.data_written.get() + invalidated);
                if let Err(error) = self.prepare_segment_for_reuse(reclaimed_commit).await {
                    self.mutable_segment_buffers.push(active.bytes);
                    self.restore_flush_records(planned.into_iter().chain(deferred));
                    return Err(error);
                }
                self.segment_commits[sg_index] = None;
                for record in planned.iter_mut().chain(&mut deferred) {
                    if record
                        .previous
                        .is_some_and(|location| location.sg_index as usize == sg_index)
                    {
                        // Reuse invalidates every previous location in this
                        // Segment, regardless of whether the key is present in
                        // the newly planned generation.
                        record.previous = None;
                        record.previous_live = false;
                    }
                }
            }

            let blob_values = planned
                .iter()
                .filter(|record| record.blob_ref.is_some())
                .map(|record| {
                    record
                        .value
                        .as_ref()
                        .expect("Blob record has a live value")
                        .bytes
                        .as_slice()
                });
            let blob_physical_bytes = match self
                .blob_segment
                .write_segment(sg_index, blob_values, blob_buffer.take())
                .await
            {
                Ok((bytes, buffer)) => {
                    blob_buffer = buffer;
                    bytes
                }
                Err(error) => {
                    self.mutable_segment_buffers.push(active.bytes);
                    self.restore_flush_records(planned.into_iter().chain(deferred));
                    return Err(storage_operation_error(&self.resource_guard, error));
                }
            };
            self.io
                .data_written
                .set(self.io.data_written.get() + blob_physical_bytes);
            self.io
                .blob_data_written
                .set(self.io.blob_data_written.get() + blob_physical_bytes);

            let (returned_segment_buffer, returned_control_buffer) = match self
                .write_segment(
                    active,
                    reason,
                    blob_used,
                    next_generation,
                    control_buffer.take(),
                )
                .await
            {
                Ok(buffers) => buffers,
                Err(error) => {
                    self.restore_flush_records(planned.into_iter().chain(deferred));
                    return Err(storage_operation_error(&self.resource_guard, error));
                }
            };
            self.mutable_segment_buffers.push(returned_segment_buffer);
            control_buffer = Some(returned_control_buffer);

            let mut published = 0usize;
            let mut publish_error = None;
            for record in &planned {
                let table_location = record.table_location.unwrap();
                let new_live = record.value.is_some();
                let replaced = record.previous.is_some_and(|previous| {
                    self.table
                        .replace_location(&record.storage_key, previous, table_location)
                });
                if replaced {
                    match (record.previous_live, new_live) {
                        (true, false) => {
                            self.stable_live_keys = self.stable_live_keys.saturating_sub(1);
                        }
                        (false, true) => self.stable_live_keys += 1,
                        _ => {}
                    }
                } else if let Err(error) = self.table.insert(&record.storage_key, table_location) {
                    publish_error = Some(error);
                    break;
                } else if new_live {
                    self.stable_live_keys += 1;
                }
                published += 1;
            }
            if let Some(error) = publish_error {
                self.restore_flush_records(planned.into_iter().skip(published).chain(deferred));
                return Err(error);
            }
            if reason == SegmentFlushReason::Capacity && self.config.ram_segment_count > 1 {
                self.restore_flush_records(deferred);
                return Ok(());
            }
            remaining = deferred;
            remaining.sort_unstable_by(|left, right| {
                right
                    .value
                    .as_ref()
                    .map_or(0, |value| value.bytes.len())
                    .cmp(&left.value.as_ref().map_or(0, |value| value.bytes.len()))
                    .then_with(|| left.storage_key.cmp(&right.storage_key))
            });
        }
        Ok(())
    }

    async fn reserve_segment_generation(
        &self,
        sg_index: usize,
        blob_logical_bytes: usize,
    ) -> Result<()> {
        let data_offset = self.config.segment_control_offset(sg_index);
        let data_bytes = (self.config.segment_size as u64)
            .checked_add(BUCKET_BYTES as u64)
            .ok_or_else(|| KvError::InvalidConfig("Segment reservation size overflowed".into()))?;
        // Reserve both sparse extents before an occupied control page is
        // invalidated, so allocation failure cannot discard the old generation.
        reserve_file_range(&self.data, data_offset, data_bytes)
            .await
            .map_err(|error| storage_io_error(&self.resource_guard, error))?;
        self.blob_segment
            .reserve_segment(sg_index, blob_logical_bytes, &self.resource_guard)
            .await?;
        self.resource_guard.observe_storage_reservation()
    }

    async fn write_segment(
        &mut self,
        active: MutableSegment,
        reason: SegmentFlushReason,
        blob_logical_len: usize,
        next_generation: u64,
        control_buffer: Option<DirectIoBuffer>,
    ) -> Result<(DirectIoBuffer, DirectIoBuffer)> {
        let fill_used_bytes = active.used_bytes() as u64;
        let sg_index = active.sg_index;
        let offset = self.config.segment_data_offset(sg_index);
        let expected = active.bytes.len();
        let bytes = write_all_direct(
            &self.data,
            active.bytes,
            offset,
            expected,
            self.config.write_max_time_us,
            "Segment write",
        )
        .await?;
        self.io
            .data_written
            .set(self.io.data_written.get() + bytes.len() as u64);
        if blob_logical_len != 0 {
            self.blob_segment.sync().await?;
        }
        self.data.sync_data().await?;
        let commit = SegmentCommit {
            sg_index,
            generation: self.next_generation,
            blob_logical_len,
            bucket_choice_count: self.config.bucket_choice_count,
        };
        let (control_bytes, control_buffer) = commit_segment(
            &mut self.data,
            &self.config,
            self.storage_key_id,
            commit,
            control_buffer,
        )
        .await?;
        self.io
            .data_written
            .set(self.io.data_written.get() + control_bytes);
        self.occupied_segments[sg_index] = true;
        self.segment_commits[sg_index] = Some(commit);
        self.next_segment_index = (sg_index + 1) % self.config.segment_count;
        self.next_generation = next_generation;
        self.segment_flushes += 1;
        match reason {
            SegmentFlushReason::Capacity => {
                self.segment_capacity_flushes += 1;
                self.segment_capacity_fill_used_bytes += fill_used_bytes;
                self.segment_capacity_fill_capacity_bytes += self.config.segment_size as u64;
            }
            SegmentFlushReason::Sync => {
                self.segment_sync_flushes += 1;
                self.segment_sync_fill_used_bytes += fill_used_bytes;
                self.segment_sync_fill_capacity_bytes += self.config.segment_size as u64;
            }
        }
        self.segment_fill_used_bytes += fill_used_bytes;
        self.segment_fill_capacity_bytes += self.config.segment_size as u64;
        Ok((bytes, control_buffer))
    }

    fn validate_value(&self, value: &[u8], expiring: bool) -> Result<()> {
        if value.len() > self.config.max_item_bytes {
            return Err(KvError::ItemTooLarge {
                bytes: value.len(),
                capacity: self.config.max_item_bytes,
            });
        }
        if is_blob_item(value) {
            if value.len() > self.config.blob_segment_size || value.len() > u32::MAX as usize {
                return Err(KvError::BlobSegmentFull {
                    required_bytes: value.len() as u64,
                    remaining_bytes: self.config.blob_segment_size as u64,
                });
            }
            return Ok(());
        }
        let item_bytes = item_offsets_bytes(1)
            + ITEM_FIXED_BYTES
            + usize::from(expiring) * ITEM_EXPIRATION_BYTES
            + STORED_VALUE_TAG_BYTES
            + value.len();
        let capacity = BUCKET_BYTES - 1;
        if item_bytes > capacity {
            return Err(KvError::ItemTooLarge {
                bytes: item_bytes,
                capacity,
            });
        }
        Ok(())
    }

    fn insert_pending(&mut self, storage_key: StorageKey, pending: PendingItem) {
        let (sg_bytes, blob_bytes) = pending_accounted_bytes(
            pending.value.as_ref().map(|value| value.bytes.as_slice()),
            pending.expires_at_ms != 0,
        );
        self.pending_sg_bytes = self.pending_sg_bytes.saturating_add(sg_bytes);
        self.pending_blob_bytes = self.pending_blob_bytes.saturating_add(blob_bytes);
        let replaced = self.pending.insert(storage_key, pending);
        debug_assert!(replaced.is_none());
    }

    fn take_pending(&mut self, storage_key: &StorageKey) -> Option<PendingItem> {
        let pending = self.pending.remove(storage_key)?;
        let (sg_bytes, blob_bytes) = pending_accounted_bytes(
            pending.value.as_ref().map(|value| value.bytes.as_slice()),
            pending.expires_at_ms != 0,
        );
        self.pending_sg_bytes = self.pending_sg_bytes.saturating_sub(sg_bytes);
        self.pending_blob_bytes = self.pending_blob_bytes.saturating_sub(blob_bytes);
        Some(pending)
    }

    fn pending_should_flush(&self) -> bool {
        self.pending_sg_bytes >= self.config.segment_size * self.config.ram_segment_count
            || self.pending_blob_bytes
                >= self.config.blob_segment_size * self.config.ram_segment_count
    }

    fn restore_flush_records<I>(&mut self, records: I)
    where
        I: IntoIterator<Item = FlushRecord>,
    {
        for record in records {
            self.insert_pending(
                record.storage_key,
                PendingItem {
                    value: record.value,
                    expires_at_ms: record.expires_at_ms,
                    previous: record.previous,
                    previous_live: record.previous_live,
                },
            );
        }
    }

    pub(crate) fn stats(&self) -> String {
        let io = self.io_stats();
        let segment_fill_percent = self.segment_fill_used_bytes as f64 * 100.0
            / self.segment_fill_capacity_bytes.max(1) as f64;
        let segment_capacity_fill_percent = self.segment_capacity_fill_used_bytes as f64 * 100.0
            / self.segment_capacity_fill_capacity_bytes.max(1) as f64;
        let segment_sync_fill_percent = self.segment_sync_fill_used_bytes as f64 * 100.0
            / self.segment_sync_fill_capacity_bytes.max(1) as f64;
        format!(
            "keys={} stable_keys={} pending_items={} pending_value_bytes={} ram_segments={} table_load={:.2}% table_memory={:.2}MiB ({:.3}B/planned-key) modeled_resident={:.2}MiB front_subtables={} front_capacity={} back_subtables={} back_capacity={} bucket_choices={} bucket_selection={} blob_used={} blob_logical_used={} blob_capacity={} next_segment_index={} occupied_segments={} flushes={} capacity_flushes={} sync_flushes={} segment_reuses={} memory_stop_writes={} memory_stop_available_bytes={} memory_resume_available_bytes={} storage_stop_writes={} storage_stop_available_bytes={} storage_resume_available_bytes={} rejected_writes={} sg_fill_percent={:.3}% sg_fill_used_bytes={} sg_fill_capacity_bytes={} capacity_sg_fill_percent={:.3}% capacity_sg_fill_used_bytes={} capacity_sg_fill_capacity_bytes={} sync_sg_fill_percent={:.3}% sync_sg_fill_used_bytes={} sync_sg_fill_capacity_bytes={} data_read={} data_written={} blob_data_read={} blob_data_written={}",
            self.logical_key_count(),
            self.stable_live_keys,
            self.pending.len(),
            self.pending_value_bytes(),
            self.config.ram_segment_count,
            self.table.load_factor() * 100.0,
            self.table.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.table.memory_bytes() as f64 / self.config.table_capacity as f64,
            self.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.table.front_table.len(),
            self.table.front_subtable_layout.entry_capacity,
            self.table.back_table.len(),
            self.table.back_subtable_layout.entry_capacity,
            self.config.bucket_choice_count,
            self.config.bucket_selection_policy.as_str(),
            self.blob_segment.used_bytes(),
            self.blob_segment.logical_used_bytes(),
            self.blob_segment.capacity_bytes(),
            self.next_segment_index,
            self.occupied_segments
                .iter()
                .filter(|value| **value)
                .count(),
            self.segment_flushes,
            self.segment_capacity_flushes,
            self.segment_sync_flushes,
            self.segment_reuses,
            self.resource_guard.memory_stop_writes(),
            self.resource_guard.memory_stop_available_bytes,
            self.resource_guard.memory_resume_available_bytes,
            self.resource_guard.storage_stop_writes(),
            self.resource_guard.storage_stop_available_bytes,
            self.resource_guard.storage_resume_available_bytes,
            self.resource_guard.rejected_writes(),
            segment_fill_percent,
            self.segment_fill_used_bytes,
            self.segment_fill_capacity_bytes,
            segment_capacity_fill_percent,
            self.segment_capacity_fill_used_bytes,
            self.segment_capacity_fill_capacity_bytes,
            segment_sync_fill_percent,
            self.segment_sync_fill_used_bytes,
            self.segment_sync_fill_capacity_bytes,
            io.data_read,
            io.data_written,
            io.blob_data_read,
            io.blob_data_written,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn reset_io_stats(&self) {
        self.io.data_written.set(0);
        self.io.data_read.set(0);
        self.io.blob_data_written.set(0);
        self.io.blob_data_read.set(0);
    }

    pub(super) fn io_stats(&self) -> KvkacheIoStats {
        KvkacheIoStats {
            data_written: self.io.data_written.get(),
            data_read: self.io.data_read.get(),
            blob_data_written: self.io.blob_data_written.get(),
            blob_data_read: self.io.blob_data_read.get(),
        }
    }

    pub(super) fn memory_bytes(&self) -> usize {
        self.table.memory_bytes()
            + self.pending.capacity()
                * (std::mem::size_of::<StorageKey>() + std::mem::size_of::<PendingItem>())
            + self.pending_value_bytes()
            + self.occupied_segments.capacity() * std::mem::size_of::<bool>()
            + self.blob_segment.memory_bytes()
            + self.config.segment_size * self.config.ram_segment_count
    }

    fn logical_key_count(&self) -> usize {
        let mut count = self.stable_live_keys;
        let now_ms = unix_time_ms();
        for pending in self.pending.values() {
            match (pending.previous_live, pending.is_live_at(now_ms)) {
                (true, false) => count = count.saturating_sub(1),
                (false, true) => count += 1,
                _ => {}
            }
        }
        count
    }

    fn pending_value_bytes(&self) -> usize {
        self.pending
            .values()
            .filter_map(|pending| pending.value.as_ref())
            .map(|value| value.bytes.capacity())
            .sum()
    }
}

pub(crate) fn validate_recovered_blob_ref(
    blob_ref: BlobRef,
    blob_logical_len: usize,
) -> Result<()> {
    let value_end = (blob_ref.value_offset as usize)
        .checked_add(blob_ref.value_len as usize)
        .ok_or_else(|| KvError::Worker("recovered BlobRef range overflow".into()))?;
    if blob_ref.value_len == 0 || value_end > blob_logical_len {
        return Err(KvError::Worker(
            "recovered BlobRef is invalid or exceeds the committed Blob length".into(),
        ));
    }
    Ok(())
}

fn stored_payload_len(value: &[u8]) -> usize {
    if is_blob_item(value) {
        STORED_BLOB_REF_BYTES
    } else {
        STORED_VALUE_TAG_BYTES + value.len()
    }
}

fn pending_accounted_bytes(value: Option<&[u8]>, expiring: bool) -> (usize, usize) {
    let Some(value) = value else {
        return (item_offsets_bytes(1) + ITEM_FIXED_BYTES, 0);
    };
    let sg_bytes = item_offsets_bytes(1)
        + ITEM_FIXED_BYTES
        + usize::from(expiring) * ITEM_EXPIRATION_BYTES
        + stored_payload_len(value);
    let blob_bytes = if is_blob_item(value) { value.len() } else { 0 };
    (sg_bytes, blob_bytes)
}

impl PendingItem {
    fn is_live_at(&self, now_ms: u64) -> bool {
        self.value.is_some() && (self.expires_at_ms == 0 || self.expires_at_ms > now_ms)
    }
}

fn set_condition_allows(condition: SetCondition, current_live: bool) -> bool {
    match condition {
        SetCondition::None => true,
        SetCondition::IfAbsent => !current_live,
        SetCondition::IfPresent => current_live,
    }
}

fn unix_time_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
