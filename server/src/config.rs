//! Configuration types for the KV cache server: [`AppConfig`] (top-level, deserialized from
//! TOML), [`Config`] (per-worker resolved config), and inner config structs for runtime,
//! I/O uring, timeouts, storage, index, durability, and recovery. Includes validation
//! logic and helpers such as [`bits_for_count()`] and [`expand_thread_pattern()`].

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{KvError, Result};

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
        return Err(io::Error::last_os_error().into());
    }
    Ok((0..libc::CPU_SETSIZE as usize)
        .filter(|cpu| unsafe { libc::CPU_ISSET(*cpu, &set) })
        .collect())
}

pub fn expand_thread_pattern(pattern: &str, thread_id: usize) -> String {
    pattern
        .replace("{thread_id:02}", &format!("{thread_id:02}"))
        .replace("{thread_id}", &thread_id.to_string())
}

pub fn bits_for_count(count: usize) -> usize {
    (usize::BITS as usize - count.saturating_sub(1).leading_zeros() as usize).max(1)
}

#[derive(Clone, Debug)]
pub struct Config {
    pub data_path: PathBuf,
    pub index_path: PathBuf,
    pub sg_size: usize,
    pub sg_count: usize,
    pub page_size: usize,
    pub index_capacity: usize,
    pub index_target_load_percent: usize,
    pub fingerprint_bits: usize,
    pub mini_buckets: usize,
    pub front_back_ratio: usize,
    pub region_bits: usize,
    pub checkpoint_on_sg_flush: bool,
    pub recovery_enabled: bool,
    pub fallback_to_sg_scan: bool,
    pub fingerprint_hash_offset_bits: usize,
    pub read_max_time_us: u64,
    pub write_max_time_us: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_path: PathBuf::from("target/kvkache-v1/kvkache.data"),
            index_path: PathBuf::from("target/kvkache-v1/kvkache.index"),
            sg_size: 16 * 1024 * 1024,
            sg_count: 64,
            page_size: 4096,
            index_capacity: 10_000_000,
            index_target_load_percent: 88,
            fingerprint_bits: 8,
            mini_buckets: 32,
            front_back_ratio: 8,
            region_bits: 6,
            checkpoint_on_sg_flush: true,
            recovery_enabled: true,
            fallback_to_sg_scan: true,
            fingerprint_hash_offset_bits: 0,
            read_max_time_us: 1_000,
            write_max_time_us: 5_000,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.sg_count == 0 || self.sg_count > (1usize << self.region_bits) {
            return Err(KvError::InvalidConfig(format!(
                "sg-count must be in 1..={} for {} region bits",
                1usize << self.region_bits,
                self.region_bits
            )));
        }
        if self.fingerprint_bits > 16 {
            return Err(KvError::InvalidConfig(
                "fingerprint-bits must be between 0 and 16".into(),
            ));
        }
        if self.region_bits == 0 || self.region_bits > 8 {
            return Err(KvError::InvalidConfig(
                "region-bits must be between 1 and 8".into(),
            ));
        }
        if self.page_size < 512 || self.page_size > u16::MAX as usize {
            return Err(KvError::InvalidConfig(
                "page-bytes must be between 512 and 65535".into(),
            ));
        }
        if self.sg_size == 0 || !self.sg_size.is_multiple_of(self.page_size) {
            return Err(KvError::InvalidConfig(
                "SG size must be a non-zero multiple of page size".into(),
            ));
        }
        if self.sg_size.checked_mul(self.sg_count).is_none() {
            return Err(KvError::InvalidConfig(
                "total SG data size is too large".into(),
            ));
        }
        if self.index_capacity == 0 {
            return Err(KvError::InvalidConfig(
                "index-capacity must be non-zero".into(),
            ));
        }
        if !(1..=95).contains(&self.index_target_load_percent) {
            return Err(KvError::InvalidConfig(
                "index-load-percent must be between 1 and 95".into(),
            ));
        }
        if self.mini_buckets < 2 || self.mini_buckets > 96 {
            return Err(KvError::InvalidConfig(
                "mini-buckets must be between 2 and 96".into(),
            ));
        }
        if self.front_back_ratio < 2
            || self.front_back_ratio > 16
            || !self.front_back_ratio.is_power_of_two()
        {
            return Err(KvError::InvalidConfig(
                "front-back-ratio must be a power of two between 2 and 16".into(),
            ));
        }
        if self.fingerprint_hash_offset_bits > 64 {
            return Err(KvError::InvalidConfig(
                "fingerprint hash offset must be at most 64 bits".into(),
            ));
        }
        if self.read_max_time_us == 0 || self.write_max_time_us == 0 {
            return Err(KvError::InvalidConfig(
                "read/write timeouts must be non-zero".into(),
            ));
        }
        Ok(())
    }

    pub fn page_count(&self) -> usize {
        self.sg_size / self.page_size
    }

    pub fn data_bytes(&self) -> u64 {
        (self.sg_size * self.sg_count) as u64
    }

    pub fn signature(&self) -> [u64; 10] {
        [
            self.sg_size as u64,
            self.sg_count as u64,
            self.page_size as u64,
            self.index_capacity as u64,
            self.index_target_load_percent as u64,
            self.fingerprint_bits as u64,
            self.mini_buckets as u64,
            self.front_back_ratio as u64,
            self.region_bits as u64,
            self.fingerprint_hash_offset_bits as u64,
        ]
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub version: u32,
    pub runtime: RuntimeConfig,
    pub io_uring: IoUringConfig,
    pub timeouts: TimeoutConfig,
    pub storage: StorageConfig,
    pub index: IndexConfig,
    pub durability: DurabilityConfig,
    pub recovery: RecoveryConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 1,
            runtime: RuntimeConfig::default(),
            io_uring: IoUringConfig::default(),
            timeouts: TimeoutConfig::default(),
            storage: StorageConfig::default(),
            index: IndexConfig::default(),
            durability: DurabilityConfig::default(),
            recovery: RecoveryConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub thread_count: usize,
    pub cpu_ids: Vec<usize>,
    pub event_interval: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let thread_count = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        Self {
            thread_count,
            cpu_ids: (0..thread_count).collect(),
            event_interval: 31,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct IoUringConfig {
    pub sqpoll: bool,
    pub entries_per_worker: u32,
    pub max_inflight_per_worker: usize,
    pub batch_size: usize,
    pub batch_max_wait_us: u64,
}

impl Default for IoUringConfig {
    fn default() -> Self {
        Self {
            sqpoll: false,
            entries_per_worker: 256,
            max_inflight_per_worker: 8,
            batch_size: 16,
            batch_max_wait_us: 10,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct TimeoutConfig {
    pub input_max_time_us: u64,
    pub output_max_time_us: u64,
    pub read_max_time_us: u64,
    pub write_max_time_us: u64,
    pub request_max_time_us: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            input_max_time_us: 10_000,
            output_max_time_us: 1_000_000,
            read_max_time_us: 100_000,
            write_max_time_us: 1_000_000,
            request_max_time_us: 2_000_000,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub directory: PathBuf,
    pub data_file_pattern: String,
    pub index_file_pattern: String,
    pub sg_per_thread: usize,
    pub sg_size_mib: usize,
    pub page_size_bytes: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("target/kvkache-v1"),
            data_file_pattern: "data-{thread_id:02}.sg".into(),
            index_file_pattern: "index-{thread_id:02}.chk".into(),
            sg_per_thread: 4,
            sg_size_mib: 16,
            page_size_bytes: 4096,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct IndexConfig {
    pub capacity_per_thread: usize,
    pub target_load_percent: usize,
    pub fingerprint_bits: usize,
    pub mini_buckets: usize,
    pub front_back_ratio: usize,
    pub fingerprint_hash_offset_bits: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            capacity_per_thread: 625_000,
            target_load_percent: 88,
            fingerprint_bits: 8,
            mini_buckets: 32,
            front_back_ratio: 8,
            fingerprint_hash_offset_bits: 64,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct DurabilityConfig {
    pub checkpoint_on_sg_flush: bool,
}

impl Default for DurabilityConfig {
    fn default() -> Self {
        Self {
            checkpoint_on_sg_flush: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RecoveryConfig {
    pub enabled: bool,
    pub fallback_to_sg_scan: bool,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fallback_to_sg_scan: true,
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(KvError::InvalidConfig(format!(
                "unsupported config version {}",
                self.version
            )));
        }
        if self.runtime.thread_count == 0 {
            return Err(KvError::InvalidConfig(
                "runtime.thread_count must be non-zero".into(),
            ));
        }
        if self.runtime.cpu_ids.len() != self.runtime.thread_count {
            return Err(KvError::InvalidConfig(
                "runtime.cpu_ids length must equal runtime.thread_count".into(),
            ));
        }
        let unique = self.runtime.cpu_ids.iter().copied().collect::<HashSet<_>>();
        if unique.len() != self.runtime.cpu_ids.len() {
            return Err(KvError::InvalidConfig(
                "runtime.cpu_ids must not contain duplicates".into(),
            ));
        }
        let allowed = allowed_cpu_ids()?;
        if let Some(cpu) = self
            .runtime
            .cpu_ids
            .iter()
            .find(|cpu| !allowed.contains(cpu))
        {
            return Err(KvError::InvalidConfig(format!(
                "CPU {cpu} is not in the process affinity set {allowed:?}"
            )));
        }
        if self.runtime.event_interval == 0 {
            return Err(KvError::InvalidConfig(
                "runtime.event_interval must be non-zero".into(),
            ));
        }
        if self.io_uring.sqpoll {
            return Err(KvError::InvalidConfig(
                "io_uring.sqpoll must remain false".into(),
            ));
        }
        if self.io_uring.entries_per_worker == 0
            || self.io_uring.max_inflight_per_worker == 0
            || self.io_uring.max_inflight_per_worker > self.io_uring.entries_per_worker as usize
        {
            return Err(KvError::InvalidConfig(
                "io_uring max_inflight must be in 1..=entries_per_worker".into(),
            ));
        }
        if self.io_uring.batch_size == 0 {
            return Err(KvError::InvalidConfig(
                "io_uring.batch_size must be non-zero".into(),
            ));
        }
        if self.timeouts.input_max_time_us == 0
            || self.timeouts.output_max_time_us == 0
            || self.timeouts.read_max_time_us == 0
            || self.timeouts.write_max_time_us == 0
            || self.timeouts.request_max_time_us == 0
        {
            return Err(KvError::InvalidConfig(
                "all timeout values must be non-zero".into(),
            ));
        }
        if self.storage.sg_per_thread == 0 || self.storage.sg_per_thread > 256 {
            return Err(KvError::InvalidConfig(
                "storage.sg_per_thread must be between 1 and 256".into(),
            ));
        }
        if self.storage.sg_size_mib.checked_mul(1024 * 1024).is_none() {
            return Err(KvError::InvalidConfig(
                "storage.sg_size_mib is too large".into(),
            ));
        }
        let data_names = (0..self.runtime.thread_count)
            .map(|thread_id| expand_thread_pattern(&self.storage.data_file_pattern, thread_id))
            .collect::<HashSet<_>>();
        let index_names = (0..self.runtime.thread_count)
            .map(|thread_id| expand_thread_pattern(&self.storage.index_file_pattern, thread_id))
            .collect::<HashSet<_>>();
        if data_names.len() != self.runtime.thread_count
            || index_names.len() != self.runtime.thread_count
        {
            return Err(KvError::InvalidConfig(
                "storage file patterns must expand to a unique file for every thread".into(),
            ));
        }
        if (0..self.runtime.thread_count).any(|thread_id| {
            expand_thread_pattern(&self.storage.data_file_pattern, thread_id)
                == expand_thread_pattern(&self.storage.index_file_pattern, thread_id)
        }) {
            return Err(KvError::InvalidConfig(
                "data and index file patterns must not resolve to the same file".into(),
            ));
        }
        self.worker_config(0).validate()
    }

    pub fn for_trace_benchmark(
        directory: PathBuf,
        cpu_ids: Vec<usize>,
        total_sg_count: usize,
        total_index_capacity: usize,
    ) -> Result<Self> {
        if cpu_ids.is_empty() || !total_sg_count.is_multiple_of(cpu_ids.len()) {
            return Err(KvError::InvalidConfig(
                "trace benchmark total SG count must divide evenly across workers".into(),
            ));
        }
        let thread_count = cpu_ids.len();
        let sg_per_thread = total_sg_count / thread_count;
        let capacity_per_thread = total_index_capacity.div_ceil(thread_count);
        let config = Self {
            version: 1,
            runtime: RuntimeConfig {
                thread_count,
                cpu_ids,
                event_interval: 31,
            },
            io_uring: IoUringConfig::default(),
            timeouts: TimeoutConfig {
                input_max_time_us: 10_000,
                output_max_time_us: 1_000_000,
                read_max_time_us: 100_000,
                write_max_time_us: 1_000_000,
                request_max_time_us: 2_000_000,
            },
            storage: StorageConfig {
                directory,
                data_file_pattern: "data-{thread_id:02}.sg".into(),
                index_file_pattern: "index-{thread_id:02}.chk".into(),
                sg_per_thread,
                sg_size_mib: 16,
                page_size_bytes: 4096,
            },
            index: IndexConfig {
                capacity_per_thread,
                target_load_percent: 88,
                fingerprint_bits: 8,
                mini_buckets: 32,
                front_back_ratio: 8,
                fingerprint_hash_offset_bits: 64,
            },
            durability: DurabilityConfig {
                checkpoint_on_sg_flush: true,
            },
            recovery: RecoveryConfig {
                enabled: true,
                fallback_to_sg_scan: true,
            },
        };
        config.validate()?;
        Ok(config)
    }

    pub fn worker_config(&self, thread_id: usize) -> Config {
        let data_name = expand_thread_pattern(&self.storage.data_file_pattern, thread_id);
        let index_name = expand_thread_pattern(&self.storage.index_file_pattern, thread_id);
        Config {
            data_path: self.storage.directory.join(data_name),
            index_path: self.storage.directory.join(index_name),
            sg_size: self.storage.sg_size_mib * 1024 * 1024,
            sg_count: self.storage.sg_per_thread,
            page_size: self.storage.page_size_bytes,
            index_capacity: self.index.capacity_per_thread,
            index_target_load_percent: self.index.target_load_percent,
            fingerprint_bits: self.index.fingerprint_bits,
            mini_buckets: self.index.mini_buckets,
            front_back_ratio: self.index.front_back_ratio,
            region_bits: bits_for_count(self.storage.sg_per_thread),
            checkpoint_on_sg_flush: self.durability.checkpoint_on_sg_flush,
            recovery_enabled: self.recovery.enabled,
            fallback_to_sg_scan: self.recovery.fallback_to_sg_scan,
            fingerprint_hash_offset_bits: self.index.fingerprint_hash_offset_bits,
            read_max_time_us: self.timeouts.read_max_time_us,
            write_max_time_us: self.timeouts.write_max_time_us,
        }
    }
}
