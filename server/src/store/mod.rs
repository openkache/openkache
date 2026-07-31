//! Direct mutable SG cache with worker-local variable generation files.

use std::cell::RefCell;
use std::fs;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::*;
use compio::BufResult;
use compio::buf::{IntoInner, IoBuf, IoBufMut, SetLen};
use compio::driver::AsRawFd;
use compio::fs::{File, OpenOptions};
use compio::io::{AsyncReadAt, AsyncWriteAt};

mod blob;
mod blob_arena;
mod bucket;
mod direct_store;
#[allow(dead_code)]
mod generation_log;
mod large_value_log;
mod sg_directory;

pub(crate) use self::blob::*;
pub(crate) use self::blob_arena::*;
pub(crate) use self::bucket::*;
pub(crate) use self::direct_store::*;
#[allow(unused_imports)]
pub(crate) use self::generation_log::*;
pub(crate) use self::large_value_log::*;
pub(crate) use self::sg_directory::*;

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
    open_direct_file_with_flags(path, true, true, 0).await
}

async fn open_direct_file_with_flags(
    path: &Path,
    create: bool,
    write: bool,
    flags: i32,
) -> std::io::Result<File> {
    let file = OpenOptions::new()
        .create(create)
        .truncate(false)
        .read(true)
        .write(write)
        .custom_flags(flags | direct_io_open_flag())
        .open(path)
        .await?;
    configure_direct_io(file.as_raw_fd())?;
    Ok(file)
}

#[cfg(target_os = "linux")]
pub(super) const fn direct_io_open_flag() -> i32 {
    libc::O_DIRECT
}

#[cfg(target_os = "macos")]
pub(super) const fn direct_io_open_flag() -> i32 {
    0
}

#[cfg(target_os = "linux")]
pub(super) fn configure_direct_io(_file_descriptor: i32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn configure_direct_io(file_descriptor: i32) -> std::io::Result<()> {
    if unsafe { libc::fcntl(file_descriptor, libc::F_NOCACHE, 1) } == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
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
            let large_values = allocated_file_bytes(&worker.large_value_path)?;
            total
                .checked_add(data)
                .and_then(|bytes| bytes.checked_add(large_values))
                .ok_or_else(|| KvError::InvalidConfig("allocated storage size overflowed".into()))
        })?;
        let generation_bytes = workers.iter().try_fold(0u64, |total, worker| {
            let worker_generation = (worker.segment_size as u64)
                .checked_add(worker.blob_segment_size as u64)
                .and_then(|value| value.checked_add(worker.max_item_bytes as u64))
                .and_then(|value| value.checked_add(2 * BUCKET_BYTES as u64))
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
            || reserve_file_range_once(file_descriptor.as_raw_fd(), offset, len),
            std::thread::sleep,
        )
    })
    .await
    .map_err(std::io::Error::from)?
}

#[cfg(target_os = "linux")]
fn reserve_file_range_once(file_descriptor: i32, offset: i64, len: i64) -> i32 {
    unsafe { libc::posix_fallocate(file_descriptor, offset, len) }
}

#[cfg(target_os = "macos")]
fn reserve_file_range_once(file_descriptor: i32, _offset: i64, len: i64) -> i32 {
    let mut reservation = libc::fstore_t {
        fst_flags: libc::F_ALLOCATECONTIG,
        fst_posmode: libc::F_PEOFPOSMODE,
        fst_offset: 0,
        fst_length: len,
        fst_bytesalloc: 0,
    };
    let contiguous = unsafe { libc::fcntl(file_descriptor, libc::F_PREALLOCATE, &mut reservation) };
    if contiguous == 0 {
        return 0;
    }

    reservation.fst_flags = libc::F_ALLOCATEALL;
    if unsafe { libc::fcntl(file_descriptor, libc::F_PREALLOCATE, &mut reservation) } == 0 {
        0
    } else {
        std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EIO)
    }
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

#[derive(Clone, Copy)]
enum SegmentFlushReason {
    Capacity,
    Sync,
}
