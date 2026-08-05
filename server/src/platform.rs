//! Host-specific runtime topology and diagnostics.

use std::collections::HashSet;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

use crate::error::Result;

/// Best-effort classification of the devices backing the worker storage files.
///
/// NVMe is the intended production medium, but it is not a correctness
/// requirement. `Unknown` is deliberately separate from `NonNvme`: containers,
/// device-mapper stacks, and restricted macOS environments may not expose
/// enough metadata to identify the physical device. `NotApplicable` is used by
/// the simulated storage backend, which has no physical storage file to inspect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageDeviceKind {
    /// Every inspected storage file resolves directly to an NVMe block device.
    Nvme,
    /// At least one inspected storage file resolves to a known non-NVMe block
    /// device.
    NonNvme,
    /// At least one inspected storage file could not be classified, and no
    /// inspected file was known to be non-NVMe.
    Unknown,
    /// The selected storage backend does not use physical storage.
    NotApplicable,
}

impl StorageDeviceKind {
    /// Conservatively combines classifications from all files used by storage.
    ///
    /// A known non-NVMe file takes precedence because it is enough to make the
    /// intended all-NVMe recommendation inapplicable. Otherwise, an unknown
    /// file prevents claiming that every storage file is on NVMe. A
    /// `NotApplicable` value is neutral so aggregation can start without
    /// assuming that a physical device exists.
    pub(crate) const fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::NonNvme, _) | (_, Self::NonNvme) => Self::NonNvme,
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            (Self::NotApplicable, other) | (other, Self::NotApplicable) => other,
            (Self::Nvme, Self::Nvme) => Self::Nvme,
        }
    }
}

/// Classifies the block device owning an already-open storage file.
///
/// This is the authoritative path for server startup diagnostics: the
/// descriptor was opened by the same storage worker that will issue reads and
/// writes, so the result cannot drift from a separately inspected config path.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
pub(crate) fn storage_device_kind_from_fd(fd: std::os::fd::RawFd) -> StorageDeviceKind {
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe { libc::fstat(fd, &mut metadata) } != 0 {
        return StorageDeviceKind::Unknown;
    }
    linux_storage_device_kind_for_device(metadata.st_dev)
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub(crate) fn storage_device_kind_from_fd(_fd: std::os::fd::RawFd) -> StorageDeviceKind {
    // macOS does not expose a stable, unprivileged equivalent of Linux's
    // /sys/dev/block mapping. Keep startup best-effort instead of guessing.
    StorageDeviceKind::Unknown
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn linux_storage_device_kind_for_device(device: u64) -> StorageDeviceKind {
    let sysfs_device = PathBuf::from("/sys/dev/block").join(format!(
        "{}:{}",
        libc::major(device),
        libc::minor(device)
    ));
    let Ok(resolved) = std::fs::canonicalize(sysfs_device) else {
        return StorageDeviceKind::Unknown;
    };

    let Some(block_name) = resolved
        .components()
        .skip_while(|component| component.as_os_str() != "block")
        .nth(1)
        .and_then(|component| component.as_os_str().to_str())
    else {
        return StorageDeviceKind::Unknown;
    };

    if block_name.starts_with("nvme") {
        StorageDeviceKind::Nvme
    } else if ["fd", "hd", "mmcblk", "sd", "sr"]
        .iter()
        .any(|prefix| block_name.starts_with(prefix))
    {
        StorageDeviceKind::NonNvme
    } else {
        StorageDeviceKind::Unknown
    }
}

/// Returns the logical CPU identifiers accepted by the runtime affinity API.
///
/// # Returns
///
/// The CPU identifiers to use in worker configuration.
///
/// # Errors
///
/// Returns an error when the process affinity mask cannot be read.
#[cfg(target_os = "linux")]
pub fn allowed_cpu_ids() -> Result<HashSet<usize>> {
    let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
    let result = unsafe {
        libc::sched_getaffinity(
            0,
            std::mem::size_of::<libc::cpu_set_t>(),
            &mut set as *mut _,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok((0..libc::CPU_SETSIZE as usize)
        .filter(|cpu| unsafe { libc::CPU_ISSET(*cpu, &set) })
        .collect())
}

/// Returns affinity tags spanning every logical processor on macOS.
///
/// # Returns
///
/// Sequential affinity tags covering the host's logical processor count.
///
/// # Errors
///
/// Returns an error when macOS does not report its available parallelism.
#[cfg(target_os = "macos")]
pub fn allowed_cpu_ids() -> Result<HashSet<usize>> {
    let count = std::thread::available_parallelism()?.get();
    Ok((0..count).collect())
}

/// Reports an affinity violation on platforms that support strict CPU pinning.
#[cfg(target_os = "linux")]
pub(crate) fn cpu_assignment_error(role: &str, expected_cpu: usize) -> Option<String> {
    let actual_cpu = unsafe { libc::sched_getcpu() };
    (actual_cpu < 0 || actual_cpu as usize != expected_cpu)
        .then(|| format!("{role} expected CPU {expected_cpu}, running on CPU {actual_cpu}"))
}

/// macOS exposes affinity tags as cache-locality hints and cannot read back a CPU assignment.
#[cfg(target_os = "macos")]
pub(crate) fn cpu_assignment_error(_role: &str, _expected_cpu: usize) -> Option<String> {
    None
}

/// Pins the calling thread to one Linux logical CPU.
#[cfg(all(
    target_os = "linux",
    any(
        feature = "storage-runtime-kimojio",
        feature = "storage-runtime-monoio",
        feature = "network-runtime-kimojio",
        feature = "network-runtime-monoio"
    )
))]
pub(crate) fn pin_current_thread(cpu_id: usize) -> std::io::Result<()> {
    if cpu_id >= libc::CPU_SETSIZE as usize {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("CPU identifier {cpu_id} exceeds CPU_SETSIZE"),
        ));
    }
    let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
    unsafe { libc::CPU_SET(cpu_id, &mut set) };
    let result = unsafe {
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set as *const _)
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Formats the worker placement information available on the current host.
#[cfg(target_os = "linux")]
pub(crate) fn cpu_diagnostic(_affinity_id: usize) -> String {
    let cpu = unsafe { libc::sched_getcpu() };
    format!("cpu_id={cpu}")
}

/// Formats the configured Mach affinity tag, which is not a strict CPU identifier.
#[cfg(target_os = "macos")]
pub(crate) fn cpu_diagnostic(affinity_id: usize) -> String {
    format!("affinity_tag={affinity_id}")
}
