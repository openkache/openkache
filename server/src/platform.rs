//! Host-specific runtime topology and diagnostics.

use std::collections::HashSet;

use crate::error::Result;

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
        feature = "storage-runtime-monoio"
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
