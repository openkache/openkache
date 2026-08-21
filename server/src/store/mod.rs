//! Direct mutable SG cache with worker-local variable generation files.

#[cfg(feature = "storage-runtime-compio")]
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::channel::{AsyncReceiver, Sender, TrySendError};
use crate::*;
#[cfg(feature = "storage-runtime-compio")]
use compio::buf::{IoBuf, IoBufMut, SetLen};

use crate::storage_backend;
use crate::storage_runtime::{self, File};

mod blob;
mod blob_arena;
mod bucket;
mod committed_generation;
mod direct_store;
#[allow(dead_code)]
mod generation_log;
mod format;
mod large_value_log;
mod sg_directory;

pub(crate) use self::blob::*;
pub(crate) use self::blob_arena::*;
pub(crate) use self::bucket::*;
pub(crate) use self::committed_generation::*;
pub(crate) use self::direct_store::*;
#[allow(unused_imports)]
pub(crate) use self::generation_log::*;
pub(crate) use self::format::*;
pub(crate) use self::large_value_log::*;
pub(crate) use self::sg_directory::*;

const CAPACITY_CHECK_INTERVAL: Duration = Duration::from_millis(100);
/// Preallocates one 4 KiB read buffer for each default in-flight worker job.
pub(crate) const BUCKET_READ_POOL_CAPACITY: usize = 64;

#[allow(unused_imports)]
pub(crate) use storage_backend::{
    capacity_stop_writes, reserve_with_transient_retry, storage_capacity_thresholds,
};

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

#[cfg(feature = "storage-runtime-compio")]
impl IoBuf for DirectIoBuffer {
    fn as_init(&self) -> &[u8] {
        self
    }
}

#[cfg(feature = "storage-runtime-compio")]
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

#[cfg(feature = "storage-runtime-compio")]
impl SetLen for DirectIoBuffer {
    unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= self.capacity());
        self.initialized_len = len;
    }
}

struct DirectIoBufferPoolStats {
    active: AtomicUsize,
    allocations: AtomicU64,
    reuses: AtomicU64,
    high_water: AtomicUsize,
}

pub(crate) struct DirectIoBufferLease {
    buffer: Option<DirectIoBuffer>,
    recycle: Option<Sender<DirectIoBuffer>>,
    stats: Option<Arc<DirectIoBufferPoolStats>>,
}

impl DirectIoBufferLease {
    fn unpooled(buffer: DirectIoBuffer) -> Self {
        Self {
            buffer: Some(buffer),
            recycle: None,
            stats: None,
        }
    }

    fn pooled(
        buffer: DirectIoBuffer,
        recycle: Sender<DirectIoBuffer>,
        stats: Arc<DirectIoBufferPoolStats>,
    ) -> Self {
        Self {
            buffer: Some(buffer),
            recycle: Some(recycle),
            stats: Some(stats),
        }
    }

    fn buffer(&self) -> &DirectIoBuffer {
        self.buffer.as_ref().expect("direct-I/O lease has a buffer")
    }

    fn buffer_mut(&mut self) -> &mut DirectIoBuffer {
        self.buffer.as_mut().expect("direct-I/O lease has a buffer")
    }
}

impl Drop for DirectIoBufferLease {
    fn drop(&mut self) {
        let Some(mut buffer) = self.buffer.take() else {
            return;
        };
        let Some(recycle) = self.recycle.as_ref() else {
            return;
        };
        buffer.initialized_len = 0;
        match recycle.try_send(buffer) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                debug_assert!(
                    false,
                    "direct-I/O recycle queue exceeded its fixed capacity"
                );
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
        if let Some(stats) = &self.stats {
            stats.active.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Deref for DirectIoBufferLease {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.buffer()
    }
}

impl DerefMut for DirectIoBufferLease {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.buffer_mut()
    }
}

#[cfg(feature = "storage-runtime-kimojio")]
impl storage_runtime::ReadBuffer for DirectIoBuffer {
    fn read_capacity_mut(&mut self) -> &mut [u8] {
        let capacity = self.capacity();
        // SAFETY: the contiguous page allocation contains `capacity` initialized bytes.
        unsafe { std::slice::from_raw_parts_mut(self.as_mut_ptr(), capacity) }
    }

    fn set_read_len(&mut self, initialized_len: usize) {
        debug_assert!(initialized_len <= self.capacity());
        self.initialized_len = self.initialized_len.max(initialized_len);
    }
}

#[cfg(feature = "storage-runtime-simulated")]
impl storage_runtime::ReadBuffer for DirectIoBuffer {
    fn set_read_len(&mut self, initialized_len: usize) {
        debug_assert!(initialized_len <= self.capacity());
        self.initialized_len = self.initialized_len.max(initialized_len);
    }
}

#[cfg(feature = "storage-runtime-kimojio")]
impl storage_runtime::WriteBuffer for DirectIoBuffer {
    fn initialized(&self) -> &[u8] {
        self
    }
}

#[cfg(feature = "storage-runtime-simulated")]
impl storage_runtime::WriteBuffer for DirectIoBuffer {}

#[cfg(feature = "storage-runtime-kimojio")]
impl storage_runtime::ReadBuffer for DirectIoBufferLease {
    fn read_capacity_mut(&mut self) -> &mut [u8] {
        self.buffer_mut().read_capacity_mut()
    }

    fn set_read_len(&mut self, initialized_len: usize) {
        self.buffer_mut().set_read_len(initialized_len);
    }
}

#[cfg(feature = "storage-runtime-simulated")]
impl storage_runtime::ReadBuffer for DirectIoBufferLease {
    fn set_read_len(&mut self, initialized_len: usize) {
        self.buffer_mut().set_read_len(initialized_len);
    }
}

#[cfg(feature = "storage-runtime-kimojio")]
impl storage_runtime::WriteBuffer for DirectIoBufferLease {
    fn initialized(&self) -> &[u8] {
        self
    }
}

#[cfg(feature = "storage-runtime-simulated")]
impl storage_runtime::WriteBuffer for DirectIoBufferLease {}

#[cfg(feature = "storage-runtime-compio")]
impl IoBuf for DirectIoBufferLease {
    fn as_init(&self) -> &[u8] {
        self
    }
}

#[cfg(feature = "storage-runtime-compio")]
impl IoBufMut for DirectIoBufferLease {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        self.buffer_mut().as_uninit()
    }
}

#[cfg(feature = "storage-runtime-compio")]
impl SetLen for DirectIoBufferLease {
    unsafe fn set_len(&mut self, len: usize) {
        // SAFETY: the caller upholds `SetLen`'s initialization contract.
        unsafe { self.buffer_mut().set_len(len) };
    }
}

#[cfg(feature = "storage-runtime-monoio")]
unsafe impl monoio::buf::IoBuf for DirectIoBuffer {
    fn read_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn bytes_init(&self) -> usize {
        self.initialized_len
    }
}

#[cfg(feature = "storage-runtime-monoio")]
unsafe impl monoio::buf::IoBufMut for DirectIoBuffer {
    fn write_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }

    fn bytes_total(&mut self) -> usize {
        self.capacity()
    }

    unsafe fn set_init(&mut self, initialized_len: usize) {
        debug_assert!(initialized_len <= self.capacity());
        self.initialized_len = self.initialized_len.max(initialized_len);
    }
}

#[cfg(feature = "storage-runtime-monoio")]
unsafe impl monoio::buf::IoBuf for DirectIoBufferLease {
    fn read_ptr(&self) -> *const u8 {
        self.buffer().as_ptr()
    }

    fn bytes_init(&self) -> usize {
        self.buffer().initialized_len
    }
}

#[cfg(feature = "storage-runtime-monoio")]
unsafe impl monoio::buf::IoBufMut for DirectIoBufferLease {
    fn write_ptr(&mut self) -> *mut u8 {
        self.buffer_mut().as_mut_ptr()
    }

    fn bytes_total(&mut self) -> usize {
        self.buffer_mut().capacity()
    }

    unsafe fn set_init(&mut self, initialized_len: usize) {
        let buffer = self.buffer_mut();
        debug_assert!(initialized_len <= buffer.capacity());
        buffer.initialized_len = buffer.initialized_len.max(initialized_len);
    }
}

pub(crate) struct DirectIoBufferPool {
    capacity: usize,
    recycle: Option<Sender<DirectIoBuffer>>,
    available: Option<AsyncReceiver<DirectIoBuffer>>,
    stats: Arc<DirectIoBufferPoolStats>,
}

impl Default for DirectIoBufferPool {
    fn default() -> Self {
        Self::with_capacity(BUCKET_READ_POOL_CAPACITY)
    }
}

impl DirectIoBufferPool {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_buffer_bytes(capacity, BUCKET_BYTES)
    }

    pub(crate) fn with_capacity_and_buffer_bytes(capacity: usize, buffer_bytes: usize) -> Self {
        assert!(buffer_bytes > 0 && buffer_bytes.is_multiple_of(BUCKET_BYTES));
        let stats = Arc::new(DirectIoBufferPoolStats {
            active: AtomicUsize::new(0),
            allocations: AtomicU64::new(0),
            reuses: AtomicU64::new(0),
            high_water: AtomicUsize::new(0),
        });
        if capacity == 0 {
            return Self {
                capacity,
                recycle: None,
                available: None,
                stats,
            };
        }
        let (recycle, available) = crate::channel::bounded_async(capacity);
        for _ in 0..capacity {
            recycle
                .try_send(DirectIoBuffer::for_read(buffer_bytes))
                .expect("new direct-I/O pool has capacity for every preallocated buffer");
        }
        Self {
            capacity,
            recycle: Some(recycle),
            available: Some(available),
            stats,
        }
    }

    pub(crate) async fn take_bucket(&self) -> DirectIoBufferLease {
        self.take_buffer(BUCKET_BYTES).await
    }

    pub(crate) async fn take_buffer(&self, read_len: usize) -> DirectIoBufferLease {
        debug_assert!(read_len > 0 && read_len.is_multiple_of(BUCKET_BYTES));
        let Some(available) = &self.available else {
            self.stats.allocations.fetch_add(1, Ordering::Relaxed);
            return DirectIoBufferLease::unpooled(DirectIoBuffer::for_read(read_len));
        };
        let buffer = available
            .recv_async_storage()
            .await
            .expect("direct-I/O pool sender remains owned by the pool");
        debug_assert!(buffer.capacity() >= read_len);
        self.stats.reuses.fetch_add(1, Ordering::Relaxed);
        let active = self.stats.active.fetch_add(1, Ordering::Relaxed) + 1;
        self.stats.high_water.fetch_max(active, Ordering::Relaxed);
        DirectIoBufferLease::pooled(
            buffer,
            self.recycle
                .as_ref()
                .expect("enabled direct-I/O pool has a recycle sender")
                .clone(),
            Arc::clone(&self.stats),
        )
    }

    pub(crate) fn allocations(&self) -> u64 {
        self.stats.allocations.load(Ordering::Relaxed)
    }

    pub(crate) fn reuses(&self) -> u64 {
        self.stats.reuses.load(Ordering::Relaxed)
    }

    pub(crate) fn idle(&self) -> usize {
        if self.capacity == 0 {
            return 0;
        }
        self.capacity
            .saturating_sub(self.stats.active.load(Ordering::Relaxed))
    }

    pub(crate) fn high_water(&self) -> usize {
        self.stats.high_water.load(Ordering::Relaxed)
    }

    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }
}

pub(crate) async fn open_direct_file(path: &Path) -> std::io::Result<File> {
    open_direct_file_with_flags(path, true, true, 0).await
}

async fn open_direct_file_with_flags(
    path: &Path,
    create: bool,
    write: bool,
    flags: i32,
) -> std::io::Result<File> {
    let file =
        storage_runtime::open_file(path, create, write, flags | direct_io_open_flag()).await?;
    storage_backend::configure_direct_io(&file)?;
    Ok(file)
}

pub(super) const fn direct_io_open_flag() -> i32 {
    storage_backend::direct_io_open_flag()
}

#[allow(dead_code)]
pub(super) fn configure_direct_io(file_descriptor: i32) -> std::io::Result<()> {
    storage_backend::configure_direct_io_descriptor(file_descriptor)
}

pub(crate) struct ResourceGuard {
    memory_stop_available_bytes: u64,
    memory_resume_available_bytes: u64,
    storage_capacity: storage_backend::StorageCapacity,
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
        let storage_capacity = storage_backend::storage_capacity(directory, workers)?;

        Ok(Self {
            memory_stop_available_bytes: memory.reserve_bytes,
            memory_resume_available_bytes: memory
                .reserve_bytes
                .saturating_add(memory.reserve_bytes / 4)
                .min(memory.available_bytes),
            storage_capacity,
            memory_stop_writes: AtomicBool::new(false),
            storage_stop_writes: AtomicBool::new(false),
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
            let stopped = self.storage_capacity.refresh_stop_writes(true)?;
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
        if self.storage_capacity.observe_reservation()? {
            self.storage_stop_writes.store(true, Ordering::Release);
        }
        Ok(())
    }

    fn mark_storage_exhausted(&self) {
        if self.storage_capacity.is_enabled() {
            self.storage_stop_writes.store(true, Ordering::Release);
        }
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

pub(crate) async fn reserve_file_range(file: &File, offset: u64, len: u64) -> std::io::Result<()> {
    storage_backend::reserve_file_range(file, offset, len).await
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

pub(crate) async fn read_exact_direct<B>(
    file: &File,
    mut buffer: B,
    offset: u64,
    len: usize,
    timeout_us: u64,
    operation: &'static str,
) -> Result<B>
where
    B: storage_runtime::ReadBuffer,
{
    let timeout = Duration::from_micros(timeout_us);
    let started = Instant::now();
    let mut completed = 0usize;
    while completed < len {
        let read_offset = offset
            .checked_add(completed as u64)
            .ok_or_else(|| KvError::Worker(format!("{operation} offset overflowed")))?;
        let (result, returned) = storage_runtime::timeout(
            direct_io_timeout(started, timeout, operation)?,
            file.read_at(buffer, completed..len, read_offset),
        )
        .await
        .map_err(|_| KvError::Timeout(operation))?;
        let read_bytes = match result {
            Ok(read_bytes) => read_bytes,
            Err(error) if is_transient_io_error(&error) => {
                buffer = returned;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        validate_direct_io_progress(operation, read_bytes, len - completed)?;
        completed += read_bytes;
        buffer = returned;
    }
    Ok(buffer)
}

pub(crate) async fn write_all_direct(
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
        let write_offset = offset
            .checked_add(completed as u64)
            .ok_or_else(|| KvError::Worker(format!("{operation} offset overflowed")))?;
        let (result, returned) = storage_runtime::timeout(
            direct_io_timeout(started, timeout, operation)?,
            file.write_at(buffer, completed..len, write_offset),
        )
        .await
        .map_err(|_| KvError::Timeout(operation))?;
        let written = match result {
            Ok(written) => written,
            Err(error) if is_transient_io_error(&error) => {
                buffer = returned;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        validate_direct_io_progress(operation, written, len - completed)?;
        completed += written;
        buffer = returned;
    }
    Ok(buffer)
}

/// Flushes file data through the selected storage runtime within the write
/// deadline. A generation must not be published until every paired file has
/// completed this barrier.
pub(crate) async fn sync_data(file: &File, timeout_us: u64, operation: &'static str) -> Result<()> {
    storage_runtime::timeout(Duration::from_micros(timeout_us), file.sync_data())
        .await
        .map_err(|_| KvError::Timeout(operation))?
        .map_err(Into::into)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetOutcome {
    Created,
    Replaced,
    NotStored,
}

#[derive(Clone, Copy)]
enum SegmentFlushReason {
    Capacity,
    Sync,
}
