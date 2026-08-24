//! Compile-time-selected storage policy and lifecycle backend.
//!
//! [`storage_runtime`](crate::storage_runtime) owns the event-loop and file
//! operation implementations. This module owns the policy decisions that
//! surround those operations: whether a backend has physical persistence,
//! how startup state is created, how namespace metadata is stored, and which
//! capacity and device checks apply. Keeping those decisions here lets the
//! cache and server paths use one backend contract instead of branching on the
//! simulated feature at every call site.

use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(not(feature = "storage-runtime-simulated"))]
use crate::BUCKET_BYTES;
#[cfg(not(feature = "storage-runtime-simulated"))]
use crate::platform;
use crate::platform::StorageDeviceKind;
use crate::runtime::ServerSecret;
use crate::sizing;
#[cfg(not(feature = "storage-runtime-simulated"))]
use crate::storage_runtime;
use crate::storage_runtime::File;
use crate::{AppConfig, Config, KvError, Result};

#[allow(dead_code)]
pub(crate) const SERVER_KEY_FILE: &str = ".openkache-key";
#[allow(dead_code)]
pub(crate) const RUNNING_MARKER_FILE: &str = ".openkache-running";
#[allow(dead_code)]
pub(crate) const NAMESPACE_METADATA_FILE: &str = ".openkache-namespaces";
#[allow(dead_code)]
pub(crate) const STORAGE_FORMAT_FILE: &str = ".openkache-format";
pub(crate) const USES_PHYSICAL_STORAGE: bool = backend::USES_PHYSICAL_STORAGE;

#[allow(dead_code)]
const STORAGE_RESERVE_PERCENT: u64 = 5;
#[allow(dead_code)]
const FILE_RESERVATION_RETRY_DELAYS: [Duration; 6] = [
    Duration::from_millis(1),
    Duration::from_millis(2),
    Duration::from_millis(4),
    Duration::from_millis(8),
    Duration::from_millis(16),
    Duration::from_millis(32),
];

/// Startup state shared by the worker pool and the namespace registry.
pub(crate) struct Startup {
    pub(crate) server_secret: ServerSecret,
    pub(crate) allow_checkpoint: bool,
}

/// Describes where namespace metadata is kept for the selected backend.
#[allow(dead_code)]
pub(crate) enum NamespacePersistence {
    Durable(PathBuf),
    Ephemeral,
}

/// Storage capacity policy used by [`crate::store::ResourceGuard`].
///
/// Native backends carry the directory used for filesystem checks. The
/// simulated backend carries no path and therefore never touches the
/// configured storage directory.
pub(crate) struct StorageCapacity {
    directory: Option<PathBuf>,
    stop_available_bytes: u64,
    resume_available_bytes: u64,
}

impl StorageCapacity {
    #[allow(dead_code)]
    fn disabled() -> Self {
        Self {
            directory: None,
            stop_available_bytes: 0,
            resume_available_bytes: 0,
        }
    }

    #[allow(dead_code)]
    fn physical(directory: &Path, stop_available_bytes: u64, resume_available_bytes: u64) -> Self {
        Self {
            directory: Some(directory.to_path_buf()),
            stop_available_bytes,
            resume_available_bytes,
        }
    }

    pub(crate) fn refresh_stop_writes(&self, stopped: bool) -> Result<bool> {
        let Some(directory) = &self.directory else {
            return Ok(false);
        };
        let available = sizing::filesystem_available_bytes(directory)?;
        Ok(capacity_stop_writes(
            stopped,
            available,
            self.stop_available_bytes,
            self.resume_available_bytes,
        ))
    }

    pub(crate) fn observe_reservation(&self) -> Result<bool> {
        let Some(directory) = &self.directory else {
            return Ok(false);
        };
        Ok(sizing::filesystem_available_bytes(directory)? <= self.stop_available_bytes)
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.directory.is_some()
    }
}

/// Starts the selected storage backend's lifecycle.
pub(crate) fn startup(config: &AppConfig) -> Result<Startup> {
    backend::startup(config)
}

/// Starts a storage run when a caller supplies the server key explicitly.
pub(crate) fn startup_with_server_key(config: &AppConfig) -> Result<bool> {
    backend::startup_with_server_key(config)
}

/// Begins a storage run for lifecycle-focused callers.
#[allow(dead_code)]
pub(crate) fn begin_storage_run(directory: &Path) -> Result<bool> {
    backend::begin_storage_run(directory)
}

/// Finishes the selected storage backend's lifecycle.
pub(crate) fn finish(directory: &Path) -> Result<()> {
    backend::finish(directory)
}

/// Finishes a storage run for lifecycle-focused callers.
#[allow(dead_code)]
pub(crate) fn finish_storage_run(directory: &Path) -> Result<()> {
    backend::finish_storage_run(directory)
}

/// Loads or creates the durable server secret, or returns the backend's
/// ephemeral secret when persistence is not part of its contract.
#[allow(dead_code)]
pub(crate) fn load_or_create_server_secret(
    directory: &Path,
    existing_storage: bool,
) -> Result<ServerSecret> {
    backend::load_or_create_server_secret(directory, existing_storage)
}

/// Selects DomainV1 or fails closed before worker storage is opened.
#[allow(dead_code)]
pub(crate) fn load_or_create_storage_format(
    directory: &Path,
    existing_storage: bool,
) -> Result<()> {
    backend::load_or_create_storage_format(directory, existing_storage)
}

/// Opens the parent directory when the selected backend uses physical files.
pub(crate) fn ensure_parent_directory(path: &Path) -> std::io::Result<()> {
    backend::ensure_parent_directory(path)
}

/// Reports whether the selected backend has storage that can already exist.
pub(crate) fn existing_storage(config: &AppConfig) -> bool {
    backend::existing_storage(config)
}

/// Selects persistent or ephemeral namespace metadata for the backend.
pub(crate) fn namespace_persistence(directory: &Path) -> NamespacePersistence {
    backend::namespace_persistence(directory)
}

/// Builds the capacity policy for a group of workers.
pub(crate) fn storage_capacity(directory: &Path, workers: &[Config]) -> Result<StorageCapacity> {
    backend::storage_capacity(directory, workers)
}

/// Classifies the device backing an opened file.
pub(crate) fn file_device_kind(file: &File) -> StorageDeviceKind {
    backend::file_device_kind(file)
}

/// Applies the backend's direct-I/O mode to an opened file.
pub(crate) fn configure_direct_io(file: &File) -> std::io::Result<()> {
    backend::configure_direct_io(file)
}

/// Applies the platform's direct-I/O mode to an already-open descriptor.
///
/// This descriptor form remains available to focused platform tests; storage
/// production code uses [`configure_direct_io`] so simulated files need no
/// descriptor-specific branch at their call site.
#[allow(dead_code)]
pub(crate) fn configure_direct_io_descriptor(file_descriptor: i32) -> std::io::Result<()> {
    backend::configure_direct_io_descriptor(file_descriptor)
}

/// Returns the open flag used for direct-I/O storage files.
#[cfg(target_os = "linux")]
pub(crate) const fn direct_io_open_flag() -> i32 {
    libc::O_DIRECT
}

/// macOS uses `F_NOCACHE` after opening a file rather than an open flag.
#[cfg(target_os = "macos")]
pub(crate) const fn direct_io_open_flag() -> i32 {
    0
}

/// Reserves a physical file range, or completes immediately for an ephemeral
/// backend.
pub(crate) async fn reserve_file_range(file: &File, offset: u64, len: u64) -> std::io::Result<()> {
    backend::reserve_file_range(file, offset, len).await
}

/// Retries only transient file reservation failures.
#[allow(dead_code)]
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

/// Calculates the hysteresis thresholds used by native storage admission.
#[allow(dead_code)]
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

#[cfg(feature = "storage-runtime-simulated")]
mod backend {
    use super::*;

    pub(crate) const USES_PHYSICAL_STORAGE: bool = false;

    pub(crate) fn startup(_config: &AppConfig) -> Result<Startup> {
        Ok(Startup {
            server_secret: ServerSecret {
                // Completion-only storage is intentionally ephemeral. A fixed
                // secret keeps CPU benchmarks reproducible without an entropy
                // or persistence dependency.
                id: [0; 16],
                key: [0; 32],
            },
            allow_checkpoint: true,
        })
    }

    pub(crate) fn startup_with_server_key(_config: &AppConfig) -> Result<bool> {
        Ok(true)
    }

    pub(crate) fn begin_storage_run(_directory: &Path) -> Result<bool> {
        Ok(true)
    }

    pub(crate) fn finish(_directory: &Path) -> Result<()> {
        Ok(())
    }

    pub(crate) fn finish_storage_run(_directory: &Path) -> Result<()> {
        Ok(())
    }

    pub(crate) fn load_or_create_server_secret(
        _directory: &Path,
        _existing_storage: bool,
    ) -> Result<ServerSecret> {
        Ok(ServerSecret {
            id: [0; 16],
            key: [0; 32],
        })
    }

    pub(crate) fn load_or_create_storage_format(
        _directory: &Path,
        _existing_storage: bool,
    ) -> Result<()> {
        Ok(())
    }

    pub(crate) fn ensure_parent_directory(_path: &Path) -> std::io::Result<()> {
        Ok(())
    }

    pub(crate) fn existing_storage(_config: &AppConfig) -> bool {
        false
    }

    pub(crate) fn namespace_persistence(_directory: &Path) -> NamespacePersistence {
        NamespacePersistence::Ephemeral
    }

    pub(crate) fn storage_capacity(
        _directory: &Path,
        _workers: &[Config],
    ) -> Result<StorageCapacity> {
        Ok(StorageCapacity::disabled())
    }

    pub(crate) fn file_device_kind(_file: &File) -> StorageDeviceKind {
        StorageDeviceKind::NotApplicable
    }

    pub(crate) fn configure_direct_io(_file: &File) -> std::io::Result<()> {
        Ok(())
    }

    pub(crate) fn configure_direct_io_descriptor(_file_descriptor: i32) -> std::io::Result<()> {
        Ok(())
    }

    pub(crate) async fn reserve_file_range(
        _file: &File,
        _offset: u64,
        _len: u64,
    ) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(not(feature = "storage-runtime-simulated"))]
mod backend {
    use std::fs;
    use std::io::{ErrorKind, Read, Write};
    #[cfg(not(feature = "storage-runtime-kimojio"))]
    use std::os::fd::{AsRawFd, BorrowedFd};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    use super::*;

    const SERVER_KEY_MAGIC: &[u8; 8] = b"OKKEY\0\0\0";
    const SERVER_KEY_VERSION: u32 = 1;
    const SERVER_KEY_FILE_BYTES: usize = 64;
    const STORAGE_FORMAT_MAGIC: &[u8; 8] = b"OKFORMAT";
    const STORAGE_FORMAT_VERSION: u32 = 1;
    const STORAGE_FORMAT_FILE_BYTES: usize = 16;
    const RUNNING_MARKER_MAGIC: &[u8; 8] = b"OKRUNNIN";
    pub(crate) const USES_PHYSICAL_STORAGE: bool = true;

    pub(crate) fn startup(config: &AppConfig) -> Result<Startup> {
        fs::create_dir_all(&config.storage.directory)?;
        let existing_storage = existing_storage(config);
        load_or_create_storage_format(&config.storage.directory, existing_storage)?;
        let server_secret =
            load_or_create_server_secret(&config.storage.directory, existing_storage)?;
        let allow_checkpoint = begin_storage_run(&config.storage.directory)?;
        Ok(Startup {
            server_secret,
            allow_checkpoint,
        })
    }

    pub(crate) fn startup_with_server_key(config: &AppConfig) -> Result<bool> {
        fs::create_dir_all(&config.storage.directory)?;
        load_or_create_storage_format(&config.storage.directory, existing_storage(config))?;
        begin_storage_run(&config.storage.directory)
    }

    pub(crate) fn finish(directory: &Path) -> Result<()> {
        finish_storage_run(directory)
    }

    pub(crate) fn load_or_create_server_secret(
        directory: &Path,
        existing_storage: bool,
    ) -> Result<ServerSecret> {
        let path = directory.join(SERVER_KEY_FILE);
        match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
        {
            Ok(mut file) => {
                let metadata = file.metadata()?;
                if !metadata.file_type().is_file() {
                    return Err(KvError::Worker(format!(
                        "server key file {} must be a regular file",
                        path.display()
                    )));
                }
                let permissions = metadata.permissions().mode() & 0o777;
                if permissions & 0o077 != 0 {
                    return Err(KvError::Worker(format!(
                        "server key file {} must not be accessible by group or other users",
                        path.display()
                    )));
                }
                let mut bytes = Vec::with_capacity(SERVER_KEY_FILE_BYTES);
                file.read_to_end(&mut bytes)?;
                return decode_server_secret(&bytes);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if existing_storage {
            return Err(KvError::Worker(format!(
                "server key file {} is missing for existing storage",
                path.display()
            )));
        }

        let secret = ServerSecret {
            id: rand::random(),
            key: rand::random(),
        };
        let bytes = encode_server_secret(secret);
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::OpenOptions::new()
            .read(true)
            .open(directory)?
            .sync_all()?;
        Ok(secret)
    }

    pub(crate) fn load_or_create_storage_format(
        directory: &Path,
        existing_storage: bool,
    ) -> Result<()> {
        let path = directory.join(STORAGE_FORMAT_FILE);
        match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
        {
            Ok(mut file) => {
                let metadata = file.metadata()?;
                if !metadata.file_type().is_file() {
                    return Err(KvError::Worker(format!(
                        "storage format marker {} must be a regular file",
                        path.display()
                    )));
                }
                let mut bytes = Vec::with_capacity(STORAGE_FORMAT_FILE_BYTES);
                file.read_to_end(&mut bytes)?;
                if !valid_storage_format(&bytes) {
                    return Err(KvError::Worker(format!(
                        "storage format marker {} is invalid or unsupported",
                        path.display()
                    )));
                }
                return Ok(());
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if existing_storage {
            return Err(KvError::Worker(format!(
                "storage format marker {} is missing for existing storage; DomainV1 cannot open an unmarked legacy store",
                path.display()
            )));
        }

        let bytes = encode_storage_format();
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::File::open(directory)?.sync_all()?;
        Ok(())
    }

    pub(crate) fn ensure_parent_directory(path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    pub(crate) fn existing_storage(config: &AppConfig) -> bool {
        (0..config.runtime.thread_count).any(|thread_id| {
            let worker = config.worker_config(thread_id);
            worker.data_path.exists() || worker.large_value_path.exists()
        })
    }

    pub(crate) fn namespace_persistence(directory: &Path) -> NamespacePersistence {
        NamespacePersistence::Durable(directory.join(NAMESPACE_METADATA_FILE))
    }

    pub(crate) fn storage_capacity(
        directory: &Path,
        workers: &[Config],
    ) -> Result<StorageCapacity> {
        let available = super::sizing::filesystem_available_bytes(directory)?;
        let allocated = workers.iter().try_fold(0u64, |total, worker| {
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
                .and_then(|value| value.checked_add(2 * super::BUCKET_BYTES as u64))
                .ok_or_else(|| {
                    KvError::InvalidConfig("storage generation size overflowed".into())
                })?;
            total.checked_add(worker_generation).ok_or_else(|| {
                KvError::InvalidConfig("host storage generation size overflowed".into())
            })
        })?;
        let (stop_available_bytes, resume_available_bytes) =
            storage_capacity_thresholds(available, allocated, generation_bytes)?;
        Ok(StorageCapacity::physical(
            directory,
            stop_available_bytes,
            resume_available_bytes,
        ))
    }

    pub(crate) fn file_device_kind(file: &File) -> StorageDeviceKind {
        super::platform::storage_device_kind_from_fd(file.raw_fd())
    }

    pub(crate) fn configure_direct_io(file: &File) -> std::io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            configure_direct_io_descriptor(file.raw_fd())
        }
        #[cfg(target_os = "macos")]
        {
            configure_direct_io_descriptor(file.raw_fd())
        }
    }

    pub(crate) fn configure_direct_io_descriptor(file_descriptor: i32) -> std::io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            let _ = file_descriptor;
            Ok(())
        }
        #[cfg(target_os = "macos")]
        {
            if unsafe { libc::fcntl(file_descriptor, libc::F_NOCACHE, 1) } == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }

    pub(crate) async fn reserve_file_range(
        file: &File,
        offset: u64,
        len: u64,
    ) -> std::io::Result<()> {
        if len == 0 {
            return Ok(());
        }
        #[cfg(feature = "storage-runtime-kimojio")]
        {
            for delay in FILE_RESERVATION_RETRY_DELAYS {
                match file.reserve_range(offset, len).await {
                    Ok(()) => return Ok(()),
                    Err(error) if is_transient_io_error(&error) => {
                        super::storage_runtime::sleep(delay).await?
                    }
                    Err(error) => return Err(error),
                }
            }
            file.reserve_range(offset, len).await
        }
        #[cfg(not(feature = "storage-runtime-kimojio"))]
        {
            let offset = i64::try_from(offset).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "offset is too large")
            })?;
            let len = i64::try_from(len).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "length is too large")
            })?;
            // Blocking tasks continue after their join handle is dropped, so
            // retain an independently owned descriptor in case the awaiting
            // operation is cancelled.
            let file_descriptor =
                unsafe { BorrowedFd::borrow_raw(file.raw_fd()) }.try_clone_to_owned()?;
            super::storage_runtime::spawn_blocking(move || {
                reserve_with_transient_retry(
                    || reserve_file_range_once(file_descriptor.as_raw_fd(), offset, len),
                    std::thread::sleep,
                )
            })
            .await
            .map_err(std::io::Error::from)?
        }
    }

    #[cfg(all(target_os = "linux", not(feature = "storage-runtime-kimojio")))]
    fn reserve_file_range_once(file_descriptor: i32, offset: i64, len: i64) -> i32 {
        unsafe { libc::posix_fallocate(file_descriptor, offset, len) }
    }

    #[cfg(all(target_os = "macos", not(feature = "storage-runtime-kimojio")))]
    fn reserve_file_range_once(file_descriptor: i32, _offset: i64, len: i64) -> i32 {
        let mut reservation = libc::fstore_t {
            fst_flags: libc::F_ALLOCATECONTIG,
            fst_posmode: libc::F_PEOFPOSMODE,
            fst_offset: 0,
            fst_length: len,
            fst_bytesalloc: 0,
        };
        let contiguous =
            unsafe { libc::fcntl(file_descriptor, libc::F_PREALLOCATE, &mut reservation) };
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

    fn allocated_file_bytes(path: &Path) -> Result<u64> {
        use std::os::unix::fs::MetadataExt;

        match fs::metadata(path) {
            Ok(metadata) => metadata
                .blocks()
                .checked_mul(512)
                .ok_or_else(|| KvError::InvalidConfig("allocated file size overflowed".into())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
            Err(error) => Err(error.into()),
        }
    }

    #[allow(dead_code)]
    fn is_transient_io_error(error: &std::io::Error) -> bool {
        error
            .raw_os_error()
            .is_some_and(|code| code == libc::EAGAIN || code == libc::EINTR)
    }

    pub(crate) fn begin_storage_run(directory: &Path) -> Result<bool> {
        let path = directory.join(RUNNING_MARKER_FILE);
        let allow_checkpoint = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => false,
            Ok(_) => {
                return Err(KvError::Worker(format!(
                    "running marker {} must be a regular file",
                    path.display()
                )));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => true,
            Err(error) => return Err(error.into()),
        };
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        file.write_all(RUNNING_MARKER_MAGIC)?;
        file.sync_all()?;
        fs::File::open(directory)?.sync_all()?;
        Ok(allow_checkpoint)
    }

    pub(crate) fn finish_storage_run(directory: &Path) -> Result<()> {
        fs::remove_file(directory.join(RUNNING_MARKER_FILE))?;
        fs::File::open(directory)?.sync_all()?;
        Ok(())
    }

    fn encode_server_secret(secret: ServerSecret) -> [u8; SERVER_KEY_FILE_BYTES] {
        let mut bytes = [0; SERVER_KEY_FILE_BYTES];
        bytes[..8].copy_from_slice(SERVER_KEY_MAGIC);
        bytes[8..12].copy_from_slice(&SERVER_KEY_VERSION.to_le_bytes());
        bytes[12..28].copy_from_slice(&secret.id);
        bytes[28..60].copy_from_slice(&secret.key);
        let checksum = server_key_checksum(&bytes[..60]);
        bytes[60..64].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    fn decode_server_secret(bytes: &[u8]) -> Result<ServerSecret> {
        if bytes.len() != SERVER_KEY_FILE_BYTES
            || &bytes[..8] != SERVER_KEY_MAGIC
            || u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != SERVER_KEY_VERSION
            || server_key_checksum(&bytes[..60])
                != u32::from_le_bytes(bytes[60..64].try_into().unwrap())
        {
            return Err(KvError::Worker("server key file is invalid".into()));
        }
        Ok(ServerSecret {
            id: bytes[12..28].try_into().unwrap(),
            key: bytes[28..60].try_into().unwrap(),
        })
    }

    fn server_key_checksum(bytes: &[u8]) -> u32 {
        crc32fast::hash(bytes)
    }

    fn encode_storage_format() -> [u8; STORAGE_FORMAT_FILE_BYTES] {
        let mut bytes = [0; STORAGE_FORMAT_FILE_BYTES];
        bytes[..8].copy_from_slice(STORAGE_FORMAT_MAGIC);
        bytes[8..12].copy_from_slice(&STORAGE_FORMAT_VERSION.to_le_bytes());
        let checksum = crc32fast::hash(&bytes[..12]);
        bytes[12..].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    fn valid_storage_format(bytes: &[u8]) -> bool {
        bytes.len() == STORAGE_FORMAT_FILE_BYTES
            && &bytes[..8] == STORAGE_FORMAT_MAGIC
            && u32::from_le_bytes(bytes[8..12].try_into().unwrap()) == STORAGE_FORMAT_VERSION
            && u32::from_le_bytes(bytes[12..].try_into().unwrap())
                == crc32fast::hash(&bytes[..12])
    }
}
