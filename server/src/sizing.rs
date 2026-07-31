//! Resource-budget sizing for common cache value distributions.
//!
//! The planner deliberately favors predictable headroom over exhaustive hardware
//! tuning. It selects power-of-two SG counts, limits the Table to half of the
//! process RAM budget, and leaves five percent of the SSD budget unassigned.
//! Automatic RAM discovery also leaves at least twenty percent of currently
//! available memory outside the process budget. Budgets are advisory inputs.
//! Discovery recognizes common Linux cgroup memory limits, macOS Mach VM
//! statistics, and filesystem availability, but not device performance or
//! filesystem quotas.

use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::fs;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::store::{
    BLOB_ITEM_THRESHOLD_BYTES, ITEM_FIXED_BYTES, STORED_BLOB_REF_BYTES, STORED_VALUE_TAG_BYTES,
};
use crate::table::Table;
use crate::types::STORAGE_KEY_BYTES;
use crate::{AppConfig, BUCKET_BYTES, KvError, Result, allowed_cpu_ids, bits_for_count};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * 1024 * 1024;
const MEMORY_USE_PERCENT: u64 = 80;
const MEMORY_RESERVE_TOTAL_PERCENT: u64 = 5;
const STORAGE_USE_PERCENT: u64 = 95;
const TABLE_RAM_PERCENT: u64 = 50;
const LIVE_KEY_PERCENT: u64 = 75;

/// A coarse value-size distribution used to choose the SG and Blob layout.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SizingProfile {
    /// Predominantly 100-byte values stored inline in SG Buckets.
    Light,
    /// Predominantly 1 KiB values stored inline in SG Buckets.
    #[default]
    Balanced,
    /// Predominantly 2 KiB values stored densely in Blob Segments.
    Heavy,
    /// Predominantly 16 KiB values stored in per-SG Blob staging.
    Blob,
}

impl SizingProfile {
    /// Returns the stable CLI and diagnostic label.
    ///
    /// # Returns
    ///
    /// One of `light`, `balanced`, or `heavy`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Balanced => "balanced",
            Self::Heavy => "heavy",
            Self::Blob => "blob",
        }
    }

    /// Returns the encoded value size modeled by this profile.
    ///
    /// # Returns
    ///
    /// The encoded value length in bytes.
    pub const fn value_bytes(self) -> usize {
        match self {
            Self::Light => 100,
            Self::Balanced => 1024,
            Self::Heavy => 2048,
            Self::Blob => 16 * 1024,
        }
    }

    const fn segment_size_mib(self) -> usize {
        match self {
            Self::Light | Self::Balanced | Self::Blob => 16,
            Self::Heavy => 2,
        }
    }

    const fn blob_segment_size_mib(self) -> usize {
        match self {
            Self::Light | Self::Balanced => 1,
            Self::Heavy | Self::Blob => 64,
        }
    }
}

/// Hardware limits and storage location used to derive one server configuration.
#[derive(Clone, Debug)]
pub struct SizingRequest {
    /// Total CPUs available to the server process.
    pub cpu_count: usize,
    /// Maximum resident memory budget in bytes.
    pub memory_bytes: u64,
    /// Maximum SSD address-space budget in bytes.
    pub storage_bytes: u64,
    /// Directory that owns the generated worker shard files.
    pub directory: PathBuf,
    /// Coarse encoded-value distribution; defaults to 1 KiB values.
    pub profile: SizingProfile,
}

impl SizingRequest {
    /// Detects the process CPU affinity, available RAM, and filesystem space.
    ///
    /// # Arguments
    ///
    /// * `directory` - Storage directory, or a path whose nearest existing
    ///   ancestor identifies the target filesystem.
    /// * `profile` - Encoded-value distribution used for capacity modeling.
    ///
    /// # Returns
    ///
    /// Resource budgets ready for optional caller overrides and planning.
    ///
    /// # Errors
    ///
    /// Returns an error when CPU topology, host memory, or target filesystem
    /// capacity cannot be inspected.
    pub fn detect(directory: PathBuf, profile: SizingProfile) -> Result<Self> {
        Ok(Self {
            cpu_count: allowed_cpu_ids()?.len(),
            memory_bytes: detected_memory_capacity()?.budget_bytes,
            storage_bytes: filesystem_available_bytes(&directory)?,
            directory,
            profile,
        })
    }

    /// Derives an [`AppConfig`] and its capacity estimates from the resource budgets.
    ///
    /// # Returns
    ///
    /// A plan using permitted CPU IDs and the largest power-of-two SG count that
    /// stays within the SSD and Table-memory budgets.
    ///
    /// # Errors
    ///
    /// Returns an error when a budget is zero, fewer CPUs are available than
    /// requested, arithmetic overflows, or even one SG per worker cannot fit.
    ///
    /// The returned plan does not reserve filesystem blocks or enforce the
    /// process memory limit. Callers must compare the requested budgets with
    /// deployment limits before starting the server.
    pub fn plan(self) -> Result<SizingPlan> {
        let mut cpu_ids = allowed_cpu_ids()?.into_iter().collect::<Vec<_>>();
        cpu_ids.sort_unstable();
        if self.cpu_count > cpu_ids.len() {
            return Err(KvError::InvalidConfig(format!(
                "sizing requested {} CPUs but the process affinity set permits {}",
                self.cpu_count,
                cpu_ids.len()
            )));
        }
        cpu_ids.truncate(self.cpu_count);
        let plan = self.plan_with_cpu_ids(cpu_ids)?;
        plan.config.validate()?;
        Ok(plan)
    }

    pub(crate) fn plan_with_cpu_ids(self, cpu_ids: Vec<usize>) -> Result<SizingPlan> {
        if self.cpu_count == 0 {
            return Err(KvError::InvalidConfig(
                "sizing CPU count must be non-zero".into(),
            ));
        }
        if self.memory_bytes == 0 || self.storage_bytes == 0 {
            return Err(KvError::InvalidConfig(
                "sizing memory and storage budgets must be non-zero".into(),
            ));
        }
        if cpu_ids.len() != self.cpu_count {
            return Err(KvError::InvalidConfig(
                "sizing CPU ID count must equal the requested CPU count".into(),
            ));
        }

        let storage_budget_bytes =
            percent_of(self.storage_bytes, STORAGE_USE_PERCENT, "SSD sizing budget")?;
        let table_memory_budget_bytes =
            percent_of(self.memory_bytes, TABLE_RAM_PERCENT, "Table sizing budget")?;
        let mut config = AppConfig::with_cpu_ids(cpu_ids);
        let storage_worker_count = config.runtime.thread_count;
        config.storage.directory = self.directory;
        config.storage.segment_size_mib = self.profile.segment_size_mib();
        config.storage.blob_segment_size_mib = self.profile.blob_segment_size_mib();
        config.storage.max_item_size_mib = config.storage.blob_segment_size_mib.min(16);
        config.storage.large_value_capacity_mib_per_thread = config.storage.max_item_size_mib;

        for exponent in (0..=16).rev() {
            let segments_per_thread = 1usize << exponent;
            config.storage.segments_per_thread = segments_per_thread;
            let keys_per_segment = keys_per_segment(self.profile)?;
            let raw_key_capacity = checked_product(
                [
                    storage_worker_count as u64,
                    segments_per_thread as u64,
                    keys_per_segment,
                ],
                "raw key capacity",
            )?;
            let planned_key_capacity =
                percent_of(raw_key_capacity, LIVE_KEY_PERCENT, "planned key capacity")?;
            let capacity_per_thread_u64 =
                planned_key_capacity.div_ceil(storage_worker_count as u64);
            let Ok(capacity_per_thread) = usize::try_from(capacity_per_thread_u64) else {
                continue;
            };
            config.table.capacity_per_thread = capacity_per_thread.max(1);

            let storage_file_bytes = storage_file_bytes(&config)?;
            if storage_file_bytes > storage_budget_bytes {
                continue;
            }
            let worker_config = config.worker_config(0);
            worker_config.validate()?;
            let table_memory_bytes = (Table::modeled_memory_bytes(&worker_config)? as u64)
                .checked_mul(storage_worker_count as u64)
                .ok_or_else(|| {
                    KvError::InvalidConfig("modeled Table memory size overflowed".into())
                })?;
            if table_memory_bytes > table_memory_budget_bytes {
                continue;
            }

            return Ok(SizingPlan {
                config,
                profile: self.profile,
                value_bytes: self.profile.value_bytes(),
                process_memory_budget_bytes: self.memory_bytes,
                sg_index_bits: bits_for_count(segments_per_thread),
                raw_key_capacity,
                planned_key_capacity,
                table_memory_bytes,
                table_memory_budget_bytes,
                storage_file_bytes,
                storage_budget_bytes,
            });
        }

        Err(KvError::InvalidConfig(
            "resource budgets cannot fit one SG and its modeled Table; increase RAM or SSD".into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MemorySnapshot {
    pub(crate) total_bytes: u64,
    pub(crate) available_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MemoryCapacity {
    pub(crate) available_bytes: u64,
    pub(crate) reserve_bytes: u64,
    pub(crate) budget_bytes: u64,
}

#[cfg(target_os = "linux")]
pub(crate) fn detected_memory_snapshot() -> Result<MemorySnapshot> {
    let meminfo = fs::read_to_string("/proc/meminfo")?;
    let cgroup_v2_max = fs::read_to_string("/sys/fs/cgroup/memory.max").ok();
    let cgroup_v2_high = fs::read_to_string("/sys/fs/cgroup/memory.high").ok();
    let cgroup_v2_current = fs::read_to_string("/sys/fs/cgroup/memory.current").ok();
    let cgroup_v1_limit = fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes").ok();
    let cgroup_v1_usage = fs::read_to_string("/sys/fs/cgroup/memory/memory.usage_in_bytes").ok();
    memory_snapshot_from_sources(
        cgroup_v2_max.as_deref(),
        cgroup_v2_high.as_deref(),
        cgroup_v2_current.as_deref(),
        cgroup_v1_limit.as_deref(),
        cgroup_v1_usage.as_deref(),
        &meminfo,
    )
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub(crate) fn detected_memory_snapshot() -> Result<MemorySnapshot> {
    let total_bytes = macos_sysctl_u64(c"hw.memsize")?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    let mut statistics = MaybeUninit::<libc::vm_statistics64>::zeroed();
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let required_count = (std::mem::offset_of!(libc::vm_statistics64, speculative_count)
        + std::mem::size_of::<libc::natural_t>())
        / std::mem::size_of::<libc::integer_t>();
    let required_count = libc::mach_msg_type_number_t::try_from(required_count)
        .expect("vm_statistics64 field count fits mach_msg_type_number_t");
    let result = unsafe {
        libc::host_statistics64(
            libc::mach_host_self(),
            libc::HOST_VM_INFO64,
            statistics.as_mut_ptr().cast(),
            &mut count,
        )
    };
    if result != libc::KERN_SUCCESS {
        return Err(std::io::Error::other(format!(
            "host_statistics64 failed with kern_return_t {result}"
        ))
        .into());
    }
    if count < required_count {
        return Err(std::io::Error::other(format!(
            "host_statistics64 returned {count} values, expected at least {required_count}"
        ))
        .into());
    }
    let statistics = unsafe { statistics.assume_init() };
    memory_snapshot_from_macos_sources(
        total_bytes,
        page_size as u64,
        statistics.free_count as u64,
        statistics.inactive_count as u64,
        statistics.speculative_count as u64,
    )
}

#[cfg(target_os = "macos")]
fn macos_sysctl_u64(name: &std::ffi::CStr) -> Result<u64> {
    let mut value = 0u64;
    let mut value_len = std::mem::size_of::<u64>();
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut value_len,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if value_len != std::mem::size_of::<u64>() {
        return Err(std::io::Error::other(format!(
            "{} returned {value_len} bytes, expected {}",
            name.to_string_lossy(),
            std::mem::size_of::<u64>()
        ))
        .into());
    }
    Ok(value)
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn memory_snapshot_from_macos_sources(
    total_bytes: u64,
    page_size: u64,
    free_pages: u64,
    inactive_pages: u64,
    speculative_pages: u64,
) -> Result<MemorySnapshot> {
    if total_bytes == 0 || page_size == 0 {
        return Err(KvError::InvalidConfig(
            "macOS memory totals and page size must be non-zero".into(),
        ));
    }
    let available_pages = free_pages
        .checked_add(inactive_pages)
        .and_then(|pages| pages.checked_add(speculative_pages))
        .ok_or_else(|| KvError::InvalidConfig("macOS available page count overflowed".into()))?;
    let available_bytes = available_pages
        .checked_mul(page_size)
        .ok_or_else(|| KvError::InvalidConfig("macOS available memory size overflowed".into()))?
        .min(total_bytes);
    Ok(MemorySnapshot {
        total_bytes,
        available_bytes,
    })
}

fn detected_memory_capacity() -> Result<MemoryCapacity> {
    automatic_memory_capacity(detected_memory_snapshot()?)
}

#[cfg(target_os = "linux")]
pub(crate) fn memory_snapshot_from_sources(
    cgroup_v2_max: Option<&str>,
    cgroup_v2_high: Option<&str>,
    cgroup_v2_current: Option<&str>,
    cgroup_v1_limit: Option<&str>,
    cgroup_v1_usage: Option<&str>,
    meminfo: &str,
) -> Result<MemorySnapshot> {
    let total_bytes = meminfo_bytes(meminfo, "MemTotal:")?;
    let host_available_bytes = meminfo_bytes(meminfo, "MemAvailable:")?;
    let mut effective_total_bytes = total_bytes;
    let mut available_bytes = host_available_bytes;

    if cgroup_v2_max.is_some() || cgroup_v2_high.is_some() || cgroup_v2_current.is_some() {
        let mut limit = None;
        for value in [cgroup_v2_max, cgroup_v2_high].into_iter().flatten() {
            if let Some(value) = parse_memory_limit(value)? {
                limit = Some(limit.map_or(value, |smallest: u64| smallest.min(value)));
            }
        }
        if let Some(limit) = limit {
            let current = parse_required_memory_value(cgroup_v2_current, "memory.current")?;
            effective_total_bytes = effective_total_bytes.min(limit);
            available_bytes = available_bytes.min(limit.saturating_sub(current));
        }
    } else if (cgroup_v1_limit.is_some() || cgroup_v1_usage.is_some())
        && let Some(limit) = cgroup_v1_limit
            .map(parse_memory_limit)
            .transpose()?
            .flatten()
    {
        let current = parse_required_memory_value(cgroup_v1_usage, "memory.usage_in_bytes")?;
        effective_total_bytes = effective_total_bytes.min(limit);
        available_bytes = available_bytes.min(limit.saturating_sub(current));
    }

    if effective_total_bytes == 0 {
        return Err(KvError::InvalidConfig(
            "effective memory limit must be non-zero".into(),
        ));
    }
    Ok(MemorySnapshot {
        total_bytes: effective_total_bytes,
        available_bytes,
    })
}

pub(crate) fn automatic_memory_capacity(snapshot: MemorySnapshot) -> Result<MemoryCapacity> {
    if snapshot.available_bytes == 0 {
        return Err(KvError::InvalidConfig(
            "no memory is available within the host and cgroup limits".into(),
        ));
    }
    let proportional_reserve = percent_of(
        snapshot.available_bytes,
        100 - MEMORY_USE_PERCENT,
        "available memory reserve",
    )?;
    let total_reserve = percent_of(
        snapshot.total_bytes,
        MEMORY_RESERVE_TOTAL_PERCENT,
        "total memory reserve",
    )?
    .min(GIB)
    .min(snapshot.available_bytes / 2);
    let reserve_bytes = proportional_reserve.max(total_reserve);
    let budget_bytes = snapshot.available_bytes.saturating_sub(reserve_bytes);
    if budget_bytes == 0 {
        return Err(KvError::InvalidConfig(
            "automatic memory reserve leaves no process budget".into(),
        ));
    }
    Ok(MemoryCapacity {
        available_bytes: snapshot.available_bytes,
        reserve_bytes,
        budget_bytes,
    })
}

#[cfg(target_os = "linux")]
fn meminfo_bytes(meminfo: &str, field: &'static str) -> Result<u64> {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .and_then(|value| value.split_whitespace().next())
        .ok_or_else(|| KvError::InvalidConfig(format!("{field} is absent from /proc/meminfo")))?
        .parse::<u64>()
        .map_err(|_| KvError::InvalidConfig(format!("{field} is not an integer")))?
        .checked_mul(1024)
        .ok_or_else(|| KvError::InvalidConfig(format!("{field} byte size overflowed")))
}

#[cfg(target_os = "linux")]
fn parse_memory_limit(value: &str) -> Result<Option<u64>> {
    let value = value.trim();
    if value == "max" {
        return Ok(None);
    }
    let limit = value
        .parse::<u64>()
        .map_err(|_| KvError::InvalidConfig("cgroup memory limit is not an integer".into()))?;
    if limit == 0 {
        return Err(KvError::InvalidConfig(
            "cgroup memory limit must be non-zero".into(),
        ));
    }
    Ok(Some(limit))
}

#[cfg(target_os = "linux")]
fn parse_required_memory_value(value: Option<&str>, name: &'static str) -> Result<u64> {
    value
        .ok_or_else(|| KvError::InvalidConfig(format!("{name} is unavailable")))?
        .trim()
        .parse::<u64>()
        .map_err(|_| KvError::InvalidConfig(format!("{name} is not an integer")))
}

pub(crate) fn filesystem_available_bytes(path: &Path) -> Result<u64> {
    let mut probe = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    while !probe.exists() {
        if !probe.pop() {
            return Err(KvError::InvalidConfig(format!(
                "no existing filesystem ancestor for {}",
                path.display()
            )));
        }
    }

    let path = CString::new(probe.as_os_str().as_bytes())
        .map_err(|_| KvError::InvalidConfig("storage path contains a NUL byte".into()))?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let stats = unsafe { stats.assume_init() };
    let fragment_bytes = if stats.f_frsize == 0 {
        stats.f_bsize
    } else {
        stats.f_frsize
    };
    stats
        .f_bavail
        .checked_mul(fragment_bytes)
        .ok_or_else(|| KvError::InvalidConfig("available filesystem size overflowed".into()))
}

/// Effective configuration and capacity estimates produced by [`SizingRequest`].
#[derive(Clone, Debug)]
pub struct SizingPlan {
    /// Server configuration ready for validation and startup.
    pub config: AppConfig,
    /// Selected workload profile.
    pub profile: SizingProfile,
    /// Encoded value size used by the storage model.
    pub value_bytes: usize,
    /// Maximum whole-process memory budget supplied to the planner.
    pub process_memory_budget_bytes: u64,
    /// Packed bits used for one worker-local SG index.
    pub sg_index_bits: usize,
    /// Theoretical unique-key capacity at perfect SG packing.
    pub raw_key_capacity: u64,
    /// Recommended live-key capacity after the 25% SG churn reserve.
    pub planned_key_capacity: u64,
    /// Modeled host-wide Table allocation.
    pub table_memory_bytes: u64,
    /// Maximum Table allocation admitted by the planner.
    pub table_memory_budget_bytes: u64,
    /// Host-wide Segment, control-page, and Blob file address space.
    pub storage_file_bytes: u64,
    /// Maximum storage address space admitted by the planner.
    pub storage_budget_bytes: u64,
}

fn percent_of(value: u64, percent: u64, name: &str) -> Result<u64> {
    value
        .checked_mul(percent)
        .map(|scaled| scaled / 100)
        .ok_or_else(|| KvError::InvalidConfig(format!("{name} overflowed")))
}

fn checked_product<const N: usize>(values: [u64; N], name: &str) -> Result<u64> {
    values.into_iter().try_fold(1u64, |product, value| {
        product
            .checked_mul(value)
            .ok_or_else(|| KvError::InvalidConfig(format!("{name} overflowed")))
    })
}

fn storage_file_bytes(config: &AppConfig) -> Result<u64> {
    let per_worker = config.worker_config(0);
    per_worker
        .generation_file_bytes()?
        .checked_add(per_worker.large_value_capacity as u64)
        .ok_or_else(|| KvError::InvalidConfig("sized per-worker storage length overflowed".into()))?
        .checked_mul(config.runtime.thread_count as u64)
        .ok_or_else(|| KvError::InvalidConfig("sized storage file length overflowed".into()))
}

fn keys_per_segment(profile: SizingProfile) -> Result<u64> {
    let value_bytes = profile.value_bytes();
    let segment_bytes = checked_product(
        [profile.segment_size_mib() as u64, MIB],
        "Segment byte size",
    )?;
    let bucket_count = segment_bytes / BUCKET_BYTES as u64;
    if STORAGE_KEY_BYTES.saturating_add(value_bytes) <= BLOB_ITEM_THRESHOLD_BYTES {
        let item_bytes = ITEM_FIXED_BYTES + STORED_VALUE_TAG_BYTES + value_bytes;
        return bucket_count
            .checked_mul(items_per_bucket(item_bytes) as u64)
            .ok_or_else(|| KvError::InvalidConfig("inline SG key capacity overflowed".into()));
    }

    let reference_capacity = bucket_count
        .checked_mul(items_per_bucket(ITEM_FIXED_BYTES + STORED_BLOB_REF_BYTES) as u64)
        .ok_or_else(|| KvError::InvalidConfig("BlobRef SG capacity overflowed".into()))?;
    let blob_bytes = checked_product(
        [profile.blob_segment_size_mib() as u64, MIB],
        "Blob Segment byte size",
    )?;
    Ok(reference_capacity.min(blob_bytes / value_bytes as u64))
}

fn items_per_bucket(item_bytes: usize) -> usize {
    (1..=u8::MAX as usize)
        .take_while(|count| 1 + (*count * 20).div_ceil(8) + *count * item_bytes <= BUCKET_BYTES)
        .last()
        .unwrap_or_default()
}
