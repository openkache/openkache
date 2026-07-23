//! KVKACHE v1: a persistent NVMe-backed KV cache with a RAM location filter.
//!
//! The logical global index is partitioned by key across thread-per-core
//! workers. Each worker exclusively owns a Breadcrumb-style
//! front-yard/backyard filter, its independent io_uring, and its circular SG
//! queue, so the hot path has no shared mutable index. Unlike an ordinary
//! approximate-membership filter, every fingerprint carries a compact physical
//! location: an SG region and the one-bit page-hash choice. A single mutable SG
//! per worker balances every record between two candidate pages.
//!
//! Public semantics intentionally follow common cache APIs:
//! - `get` returns the current value or `None`.
//! - `set` is an in-memory-acknowledged upsert and appends a new version.
//! - `delete` is acknowledged after appending an in-memory tombstone.
//!
//! A mutable SG stays open across request batches. It is persisted only when
//! page placement fails because it is full, or when `sync`/graceful shutdown
//! explicitly flushes it. Reopening the same paths restores the checkpoint and
//! validates every SG generation. If no checkpoint exists, the data SGs are
//! scanned to build an initial index.

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use compio::BufResult;
use compio::driver::ProactorBuilder;
use compio::fs::{File, OpenOptions};
use compio::io::{AsyncReadAtExt, AsyncWriteAtExt};
use compio::runtime::RuntimeBuilder;
use futures_util::stream::{FuturesUnordered, StreamExt};
use openkache::Key;
use serde::Deserialize;

const BUCKET_BYTES: usize = 64;
const PAGE_MAGIC: u32 = 0x4b56_5031; // "KVP1"
const PAGE_VERSION: u16 = 1;
const PAGE_HEADER: usize = 32;
const RECORD_HEADER: usize = 16;
const RECORD_SET: u8 = 1;
const RECORD_DELETE: u8 = 2;
const CHECKPOINT_MAGIC: &[u8; 8] = b"KVKIDX01";
const CHECKPOINT_VERSION: u32 = 1;
const NONE_GENERATION: u64 = u64::MAX;

type Result<T> = std::result::Result<T, KvError>;

#[derive(Debug)]
pub(crate) enum KvError {
    Io(io::Error),
    InvalidConfig(String),
    Corrupt(String),
    IndexFull,
    RecordTooLarge { bytes: usize, capacity: usize },
    Timeout(&'static str),
    Worker(String),
    Usage(String),
}

impl fmt::Display for KvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::InvalidConfig(message) => write!(f, "invalid configuration: {message}"),
            Self::Corrupt(message) => write!(f, "corrupt KVKACHE data: {message}"),
            Self::IndexFull => f.write_str("global Breadcrumb location index is full"),
            Self::RecordTooLarge { bytes, capacity } => write!(
                f,
                "record requires {bytes} bytes but one empty page has {capacity} bytes"
            ),
            Self::Timeout(operation) => write!(f, "{operation} timed out"),
            Self::Worker(message) => write!(f, "worker error: {message}"),
            Self::Usage(message) => f.write_str(message),
        }
    }
}

impl Error for KvError {}

impl From<io::Error> for KvError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub(crate) struct AppConfig {
    version: u32,
    runtime: RuntimeConfig,
    io_uring: IoUringConfig,
    timeouts: TimeoutConfig,
    storage: StorageConfig,
    index: IndexConfig,
    durability: DurabilityConfig,
    recovery: RecoveryConfig,
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
struct RuntimeConfig {
    thread_count: usize,
    cpu_ids: Vec<usize>,
    event_interval: usize,
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
struct IoUringConfig {
    sqpoll: bool,
    entries_per_worker: u32,
    max_inflight_per_worker: usize,
    batch_size: usize,
    batch_max_wait_us: u64,
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
struct TimeoutConfig {
    input_max_time_us: u64,
    output_max_time_us: u64,
    read_max_time_us: u64,
    write_max_time_us: u64,
    request_max_time_us: u64,
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
struct StorageConfig {
    directory: PathBuf,
    data_file_pattern: String,
    index_file_pattern: String,
    sg_per_thread: usize,
    sg_size_mib: usize,
    page_size_bytes: usize,
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
struct IndexConfig {
    capacity_per_thread: usize,
    target_load_percent: usize,
    fingerprint_bits: usize,
    mini_buckets: usize,
    front_back_ratio: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            capacity_per_thread: 625_000,
            target_load_percent: 88,
            fingerprint_bits: 8,
            mini_buckets: 32,
            front_back_ratio: 8,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct DurabilityConfig {
    checkpoint_on_sg_flush: bool,
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
struct RecoveryConfig {
    enabled: bool,
    fallback_to_sg_scan: bool,
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
    fn parse() -> Result<(Self, Command)> {
        let arguments = env::args().skip(1).collect::<Vec<_>>();
        let config_path = arguments
            .windows(2)
            .find(|pair| pair[0] == "--config")
            .map(|pair| PathBuf::from(&pair[1]));
        let mut config = if let Some(path) = &config_path {
            let text = fs::read_to_string(path).map_err(|error| {
                KvError::InvalidConfig(format!("cannot read config {}: {error}", path.display()))
            })?;
            toml::from_str(&text).map_err(|error| {
                KvError::InvalidConfig(format!("cannot parse config {}: {error}", path.display()))
            })?
        } else {
            Self::default()
        };

        let mut args = arguments.into_iter().peekable();
        while let Some(argument) = args.peek() {
            if !argument.starts_with("--") {
                break;
            }
            let flag = args.next().unwrap();
            let value = |args: &mut std::iter::Peekable<_>, flag: &str| -> Result<String> {
                args.next()
                    .ok_or_else(|| KvError::Usage(format!("missing value for {flag}")))
            };
            match flag.as_str() {
                "--config" => {
                    let _ = value(&mut args, &flag)?;
                }
                "--threads" => {
                    config.runtime.thread_count = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--cpu-ids" => {
                    config.runtime.cpu_ids = value(&mut args, &flag)?
                        .split(',')
                        .map(|cpu| parse_usize(cpu.trim(), &flag))
                        .collect::<Result<Vec<_>>>()?
                }
                "--event-interval" => {
                    config.runtime.event_interval = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--io-uring-entries" => {
                    config.io_uring.entries_per_worker = value(&mut args, &flag)?
                        .parse()
                        .map_err(|_| KvError::Usage(format!("{flag} expects a u32")))?
                }
                "--max-inflight" => {
                    config.io_uring.max_inflight_per_worker =
                        parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--batch-size" => {
                    config.io_uring.batch_size = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--batch-wait-us" => {
                    config.io_uring.batch_max_wait_us = parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--input-max-us" => {
                    config.timeouts.input_max_time_us = parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--output-max-us" => {
                    config.timeouts.output_max_time_us =
                        parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--read-max-us" => {
                    config.timeouts.read_max_time_us = parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--write-max-us" => {
                    config.timeouts.write_max_time_us = parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--request-max-us" => {
                    config.timeouts.request_max_time_us =
                        parse_u64(&value(&mut args, &flag)?, &flag)?
                }
                "--directory" => config.storage.directory = value(&mut args, &flag)?.into(),
                "--sg-per-thread" => {
                    config.storage.sg_per_thread = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--sg-mib" => {
                    config.storage.sg_size_mib = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--page-bytes" => {
                    config.storage.page_size_bytes = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--index-capacity-per-thread" => {
                    config.index.capacity_per_thread =
                        parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--index-load-percent" => {
                    config.index.target_load_percent =
                        parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--fingerprint-bits" => {
                    config.index.fingerprint_bits = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--mini-buckets" => {
                    config.index.mini_buckets = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--front-back-ratio" => {
                    config.index.front_back_ratio = parse_usize(&value(&mut args, &flag)?, &flag)?
                }
                "--help" => return Err(KvError::Usage(usage())),
                _ => {
                    return Err(KvError::Usage(format!(
                        "unknown option {flag}\n{}",
                        usage()
                    )));
                }
            }
        }

        let command = parse_command(&mut args)?;
        config.validate()?;
        Ok((config, command))
    }

    fn validate(&self) -> Result<()> {
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

    fn worker_config(&self, thread_id: usize) -> Config {
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
            // Routing consumes hash[0..8]. The filter uses hash[8..16], so
            // arbitrary (including non-power-of-two) worker counts do not
            // correlate routing with the packed Breadcrumb fingerprint.
            fingerprint_hash_offset_bits: 64,
            read_max_time_us: self.timeouts.read_max_time_us,
            write_max_time_us: self.timeouts.write_max_time_us,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn for_trace_benchmark(
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
        let mut config = Self::default();
        config.runtime.thread_count = cpu_ids.len();
        config.runtime.cpu_ids = cpu_ids;
        config.io_uring.entries_per_worker = 256;
        config.io_uring.max_inflight_per_worker = 8;
        config.io_uring.batch_size = 16;
        config.timeouts = TimeoutConfig {
            input_max_time_us: 30_000_000,
            output_max_time_us: 30_000_000,
            read_max_time_us: 5_000_000,
            write_max_time_us: 30_000_000,
            request_max_time_us: 30_000_000,
        };
        config.storage.directory = directory;
        config.storage.sg_per_thread = total_sg_count / config.runtime.thread_count;
        config.index.capacity_per_thread =
            total_index_capacity.div_ceil(config.runtime.thread_count);
        config.validate()?;
        Ok(config)
    }
}

fn allowed_cpu_ids() -> Result<HashSet<usize>> {
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

fn expand_thread_pattern(pattern: &str, thread_id: usize) -> String {
    pattern
        .replace("{thread_id:02}", &format!("{thread_id:02}"))
        .replace("{thread_id}", &thread_id.to_string())
}

fn bits_for_count(count: usize) -> usize {
    (usize::BITS as usize - count.saturating_sub(1).leading_zeros() as usize).max(1)
}

#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub(crate) data_path: PathBuf,
    pub(crate) index_path: PathBuf,
    pub(crate) sg_size: usize,
    pub(crate) sg_count: usize,
    pub(crate) page_size: usize,
    pub(crate) index_capacity: usize,
    pub(crate) index_target_load_percent: usize,
    pub(crate) fingerprint_bits: usize,
    pub(crate) mini_buckets: usize,
    pub(crate) front_back_ratio: usize,
    pub(crate) region_bits: usize,
    pub(crate) checkpoint_on_sg_flush: bool,
    pub(crate) recovery_enabled: bool,
    pub(crate) fallback_to_sg_scan: bool,
    pub(crate) fingerprint_hash_offset_bits: usize,
    pub(crate) read_max_time_us: u64,
    pub(crate) write_max_time_us: u64,
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
    pub(crate) fn validate(&self) -> Result<()> {
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

    fn page_count(&self) -> usize {
        self.sg_size / self.page_size
    }

    fn data_bytes(&self) -> u64 {
        (self.sg_size * self.sg_count) as u64
    }

    fn signature(&self) -> [u64; 10] {
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

fn parse_usize(value: &str, flag: &str) -> Result<usize> {
    value
        .parse()
        .map_err(|_| KvError::Usage(format!("{flag} expects a non-negative integer")))
}

fn parse_u64(value: &str, flag: &str) -> Result<u64> {
    value
        .parse()
        .map_err(|_| KvError::Usage(format!("{flag} expects a non-negative integer")))
}

fn required_arg<I: Iterator<Item = String>>(args: &mut I, usage: &str) -> Result<String> {
    args.next()
        .ok_or_else(|| KvError::Usage(format!("usage: {usage}")))
}

fn parse_command<I: Iterator<Item = String>>(args: &mut I) -> Result<Command> {
    let command = match args.next().as_deref() {
        Some("get") => Command::Get(required_arg(args, "get KEY")?.into_bytes()),
        Some("set") => Command::Set(
            required_arg(args, "set KEY VALUE")?.into_bytes(),
            required_arg(args, "set KEY VALUE")?.into_bytes(),
        ),
        Some("delete") => Command::Delete(required_arg(args, "delete KEY")?.into_bytes()),
        Some("sync") => Command::Sync,
        Some("stats") => Command::Stats,
        Some(command) => {
            return Err(KvError::Usage(format!(
                "unknown command {command}\n{}",
                usage()
            )));
        }
        None => return Err(KvError::Usage(usage())),
    };
    if args.next().is_some() {
        return Err(KvError::Usage("too many command arguments".into()));
    }
    Ok(command)
}

fn usage() -> String {
    "KVKACHE-v1 [options] <get KEY | set KEY VALUE | delete KEY | sync | stats>\n\
     Options:\n\
       --config PATH                TOML configuration file\n\
       --threads N                  worker thread count\n\
       --cpu-ids LIST               zero-based CPU IDs, comma separated\n\
       --io-uring-entries N         io_uring entries per worker\n\
       --max-inflight N             maximum in-flight I/O per worker\n\
       --batch-size N               requests processed per worker batch\n\
       --batch-wait-us N            partial batch wait in microseconds\n\
       --directory PATH             worker data/index directory\n\
       --sg-per-thread N            circular SGs owned by each worker\n\
       --sg-mib N                   SG size in MiB [16]\n\
       --page-bytes N               page size [4096]\n\
       --index-capacity-per-thread N expected live keys per worker\n\
       --index-load-percent N       planned filter load [88]\n\
       --fingerprint-bits N         R bits; zero removes R [8]\n\
       --mini-buckets N             M values per bucket [32]\n\
       --front-back-ratio N         front/back bucket ratio [8]"
        .into()
}

enum Command {
    Get(Vec<u8>),
    Set(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
    Sync,
    Stats,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Location {
    pub(crate) region: u8,
    pub(crate) page_choice: u8,
}

impl Location {
    fn encode(self, region_bits: usize) -> u16 {
        debug_assert!((self.region as usize) < (1usize << region_bits));
        ((self.region as u16) << 1) | self.page_choice as u16
    }

    fn decode(value: u16) -> Self {
        Self {
            region: (value >> 1) as u8,
            page_choice: (value & 1) as u8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Fingerprint {
    front: usize,
    mini: usize,
    remainder: u16,
}

#[derive(Clone, Copy, Debug)]
struct BackLocation {
    bucket: usize,
    crumb: u8,
}

#[derive(Clone, Copy, Debug)]
struct PackedEntry {
    mini: usize,
    remainder: u16,
    location: u16,
    crumb: u8,
}

#[derive(Clone, Debug)]
struct BucketLayout {
    mini_buckets: usize,
    capacity: usize,
    remainder_bits: usize,
    location_bits: usize,
    crumb_bits: usize,
    metadata_bytes: usize,
    remainder_bit: usize,
    location_bit: usize,
    crumb_bit: usize,
}

impl BucketLayout {
    fn new(
        mini_buckets: usize,
        remainder_bits: usize,
        location_bits: usize,
        crumb_bits: usize,
    ) -> Result<Self> {
        let mut chosen = None;
        for capacity in 1..=64 {
            let metadata_bits = mini_buckets + capacity;
            if metadata_bits > 128 {
                break;
            }
            let metadata_bytes = metadata_bits.div_ceil(8);
            let remainder_bytes = (capacity * remainder_bits).div_ceil(8);
            let location_bytes = (capacity * location_bits).div_ceil(8);
            let crumb_bytes = (capacity * crumb_bits).div_ceil(8);
            if metadata_bytes + remainder_bytes + location_bytes + crumb_bytes <= BUCKET_BYTES {
                chosen = Some((capacity, metadata_bytes, remainder_bytes, location_bytes));
            }
        }
        let Some((capacity, metadata_bytes, remainder_bytes, location_bytes)) = chosen else {
            return Err(KvError::InvalidConfig(
                "fingerprint/location/mini-bucket fields do not fit in 64-byte buckets".into(),
            ));
        };
        let remainder_bit = metadata_bytes * 8;
        let location_bit = remainder_bit + remainder_bytes * 8;
        let crumb_bit = location_bit + location_bytes * 8;
        Ok(Self {
            mini_buckets,
            capacity,
            remainder_bits,
            location_bits,
            crumb_bits,
            metadata_bytes,
            remainder_bit,
            location_bit,
            crumb_bit,
        })
    }
}

#[repr(C, align(64))]
#[derive(Clone)]
struct PackedBucket {
    bytes: [u8; BUCKET_BYTES],
}

impl PackedBucket {
    fn new(layout: &BucketLayout) -> Self {
        let mut bucket = Self {
            bytes: [0; BUCKET_BYTES],
        };
        let separators = (1u128 << layout.mini_buckets) - 1;
        bucket.store_metadata(layout, separators);
        bucket
    }

    fn metadata(&self, layout: &BucketLayout) -> u128 {
        let mut bytes = [0u8; 16];
        bytes[..layout.metadata_bytes].copy_from_slice(&self.bytes[..layout.metadata_bytes]);
        u128::from_le_bytes(bytes)
    }

    fn store_metadata(&mut self, layout: &BucketLayout, value: u128) {
        self.bytes[..layout.metadata_bytes]
            .copy_from_slice(&value.to_le_bytes()[..layout.metadata_bytes]);
    }

    fn len(&self, layout: &BucketLayout) -> usize {
        let bits = self.metadata(layout);
        (128 - bits.leading_zeros() as usize) - layout.mini_buckets
    }

    fn bounds(&self, layout: &BucketLayout, mini: usize) -> (usize, usize) {
        let bits = self.metadata(layout);
        let end = select_one(bits, mini) - mini;
        let start = if mini == 0 {
            0
        } else {
            select_one(bits, mini - 1) - (mini - 1)
        };
        (start, end)
    }

    fn entry(&self, layout: &BucketLayout, slot: usize) -> PackedEntry {
        PackedEntry {
            mini: self.mini_at(layout, slot),
            remainder: get_bits(
                &self.bytes,
                layout.remainder_bit + slot * layout.remainder_bits,
                layout.remainder_bits,
            ) as u16,
            location: get_bits(
                &self.bytes,
                layout.location_bit + slot * layout.location_bits,
                layout.location_bits,
            ) as u16,
            crumb: if layout.crumb_bits == 0 {
                0
            } else {
                get_bits(
                    &self.bytes,
                    layout.crumb_bit + slot * layout.crumb_bits,
                    layout.crumb_bits,
                ) as u8
            },
        }
    }

    fn mini_at(&self, layout: &BucketLayout, slot: usize) -> usize {
        let bits = self.metadata(layout);
        for mini in 0..layout.mini_buckets {
            let (_, end) = metadata_bounds(bits, mini);
            if slot < end {
                return mini;
            }
        }
        layout.mini_buckets - 1
    }

    fn write_entry(&mut self, layout: &BucketLayout, slot: usize, entry: PackedEntry) {
        set_bits(
            &mut self.bytes,
            layout.remainder_bit + slot * layout.remainder_bits,
            layout.remainder_bits,
            entry.remainder as u64,
        );
        set_bits(
            &mut self.bytes,
            layout.location_bit + slot * layout.location_bits,
            layout.location_bits,
            entry.location as u64,
        );
        if layout.crumb_bits > 0 {
            set_bits(
                &mut self.bytes,
                layout.crumb_bit + slot * layout.crumb_bits,
                layout.crumb_bits,
                entry.crumb as u64,
            );
        }
    }

    fn clear_entry(&mut self, layout: &BucketLayout, slot: usize) {
        self.write_entry(
            layout,
            slot,
            PackedEntry {
                mini: 0,
                remainder: 0,
                location: 0,
                crumb: 0,
            },
        );
    }

    /// Front insertion retains the smallest mini-buckets and returns one overflow.
    fn insert_front(&mut self, layout: &BucketLayout, entry: PackedEntry) -> Option<PackedEntry> {
        let len = self.len(layout);
        let location = self.bounds(layout, entry.mini).0;
        if location == layout.capacity {
            return Some(entry);
        }
        let overflow = (len == layout.capacity).then(|| self.entry(layout, layout.capacity - 1));
        let shift_end = len.min(layout.capacity - 1);
        for slot in (location..shift_end).rev() {
            let moved = self.entry(layout, slot);
            self.write_entry(layout, slot + 1, moved);
        }
        self.write_entry(layout, location, entry);
        self.metadata_insert(layout, entry.mini, location);
        overflow
    }

    fn insert_back(&mut self, layout: &BucketLayout, entry: PackedEntry) -> bool {
        let len = self.len(layout);
        if len == layout.capacity {
            return false;
        }
        let location = self.bounds(layout, entry.mini).0;
        for slot in (location..len).rev() {
            let moved = self.entry(layout, slot);
            self.write_entry(layout, slot + 1, moved);
        }
        self.write_entry(layout, location, entry);
        self.metadata_insert(layout, entry.mini, location);
        true
    }

    fn matching_slots(
        &self,
        layout: &BucketLayout,
        mini: usize,
        remainder: u16,
        crumb: Option<u8>,
    ) -> Vec<usize> {
        let (start, end) = self.bounds(layout, mini);
        (start..end)
            .filter(|slot| {
                let entry = self.entry(layout, *slot);
                entry.remainder == remainder && crumb.is_none_or(|crumb| entry.crumb == crumb)
            })
            .collect()
    }

    fn first_with_crumb(&self, layout: &BucketLayout, crumb: u8) -> Option<(usize, PackedEntry)> {
        (0..self.len(layout)).find_map(|slot| {
            let entry = self.entry(layout, slot);
            (entry.crumb == crumb).then_some((slot, entry))
        })
    }

    fn remove_at(&mut self, layout: &BucketLayout, mini: usize, slot: usize) -> PackedEntry {
        let len = self.len(layout);
        let removed = self.entry(layout, slot);
        for index in slot..len - 1 {
            let moved = self.entry(layout, index + 1);
            self.write_entry(layout, index, moved);
        }
        self.clear_entry(layout, len - 1);
        self.metadata_remove(layout, mini, slot);
        removed
    }

    fn metadata_insert(&mut self, layout: &BucketLayout, mini: usize, location: usize) {
        let bits = self.metadata(layout);
        let full = self.len(layout) == layout.capacity;
        let bit_index = mini + location;
        let lower_mask = (1u128 << bit_index) - 1;
        let total_bits = layout.mini_buckets + layout.capacity;
        let active_mask = low_mask(total_bits);
        let mut shifted = ((bits & lower_mask) | ((bits & !lower_mask) << 1)) & active_mask;
        if full {
            let zeros = !shifted & active_mask;
            let last_zero = 127 - zeros.leading_zeros() as usize;
            shifted |= 1u128 << last_zero;
        }
        self.store_metadata(layout, shifted);
    }

    fn metadata_remove(&mut self, layout: &BucketLayout, mini: usize, location: usize) {
        let bits = self.metadata(layout);
        let bit_index = mini + location;
        let lower_mask = (1u128 << bit_index) - 1;
        let lower = bits & lower_mask;
        let upper = (bits >> (bit_index + 1)) << bit_index;
        self.store_metadata(layout, lower | upper);
    }
}

fn metadata_bounds(bits: u128, mini: usize) -> (usize, usize) {
    let end = select_one(bits, mini) - mini;
    let start = if mini == 0 {
        0
    } else {
        select_one(bits, mini - 1) - (mini - 1)
    };
    (start, end)
}

fn select_one(mut bits: u128, rank: usize) -> usize {
    for _ in 0..rank {
        bits &= bits - 1;
    }
    bits.trailing_zeros() as usize
}

fn low_mask(bits: usize) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

fn get_bits(bytes: &[u8], bit: usize, width: usize) -> u64 {
    let mut value = 0u64;
    for offset in 0..width {
        let source = bit + offset;
        value |= (((bytes[source / 8] >> (source % 8)) & 1) as u64) << offset;
    }
    value
}

fn set_bits(bytes: &mut [u8], bit: usize, width: usize, value: u64) {
    for offset in 0..width {
        let target = bit + offset;
        let mask = 1u8 << (target % 8);
        if value & (1u64 << offset) == 0 {
            bytes[target / 8] &= !mask;
        } else {
            bytes[target / 8] |= mask;
        }
    }
}

pub(crate) struct LocationBreadcrumb {
    front: Vec<PackedBucket>,
    back: Vec<PackedBucket>,
    front_layout: BucketLayout,
    back_layout: BucketLayout,
    back_group_count: usize,
    ratio: usize,
    fingerprint_bits: usize,
    fingerprint_hash_offset_bits: usize,
    region_bits: usize,
    len: usize,
}

impl LocationBreadcrumb {
    pub(crate) fn new(config: &Config) -> Result<Self> {
        let location_bits = config.region_bits + 1;
        let crumb_bits = (config.front_back_ratio * 2).ilog2() as usize;
        let front_layout = BucketLayout::new(
            config.mini_buckets,
            config.fingerprint_bits,
            location_bits,
            0,
        )?;
        let back_layout = BucketLayout::new(
            config.mini_buckets,
            config.fingerprint_bits,
            location_bits,
            crumb_bits,
        )?;
        let slots_per_front = front_layout.capacity as f64
            + back_layout.capacity as f64 / config.front_back_ratio as f64;
        let planned_per_front = slots_per_front * config.index_target_load_percent as f64 / 100.0;
        let front_count = (config.index_capacity as f64 / planned_per_front)
            .ceil()
            .max(1.0) as usize;
        let back_group_count = front_count
            .div_ceil(config.front_back_ratio * config.front_back_ratio)
            .max(1);
        let back_count = front_count
            .div_ceil(config.front_back_ratio)
            .max(config.front_back_ratio * back_group_count);
        Ok(Self {
            front: (0..front_count)
                .map(|_| PackedBucket::new(&front_layout))
                .collect(),
            back: (0..back_count)
                .map(|_| PackedBucket::new(&back_layout))
                .collect(),
            front_layout,
            back_layout,
            back_group_count,
            ratio: config.front_back_ratio,
            fingerprint_bits: config.fingerprint_bits,
            fingerprint_hash_offset_bits: config.fingerprint_hash_offset_bits,
            region_bits: config.region_bits,
            len: 0,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    fn slot_capacity(&self) -> usize {
        self.front.len() * self.front_layout.capacity + self.back.len() * self.back_layout.capacity
    }

    fn load_factor(&self) -> f64 {
        self.len as f64 / self.slot_capacity() as f64
    }

    fn memory_bytes(&self) -> usize {
        (self.front.len() + self.back.len()) * BUCKET_BYTES
    }

    fn fingerprint(&self, hash: &[u8; 32]) -> Fingerprint {
        let prefix = u128::from_le_bytes(hash[..16].try_into().unwrap())
            >> self.fingerprint_hash_offset_bits;
        let prefix = prefix as u64;
        let quotient_count = self.front.len() * self.front_layout.mini_buckets;
        let remainder_space = 1u64 << self.fingerprint_bits;
        let space = (quotient_count as u128) * remainder_space as u128;
        let fingerprint = (prefix as u128 % space) as u64;
        let quotient = (fingerprint / remainder_space) as usize;
        Fingerprint {
            front: quotient / self.front_layout.mini_buckets,
            mini: quotient % self.front_layout.mini_buckets,
            remainder: (fingerprint & (remainder_space - 1)) as u16,
        }
    }

    pub(crate) fn candidates(&self, hash: &[u8; 32]) -> Vec<Location> {
        let fingerprint = self.fingerprint(hash);
        let front = &self.front[fingerprint.front];
        let mut encoded = front
            .matching_slots(
                &self.front_layout,
                fingerprint.mini,
                fingerprint.remainder,
                None,
            )
            .into_iter()
            .map(|slot| front.entry(&self.front_layout, slot).location)
            .collect::<Vec<_>>();
        let (_, end) = front.bounds(&self.front_layout, fingerprint.mini);
        if end == self.front_layout.capacity {
            let [first, second] = self.back_locations(fingerprint.front);
            for location in [first, second] {
                let bucket = &self.back[location.bucket];
                encoded.extend(
                    bucket
                        .matching_slots(
                            &self.back_layout,
                            fingerprint.mini,
                            fingerprint.remainder,
                            Some(location.crumb),
                        )
                        .into_iter()
                        .map(|slot| bucket.entry(&self.back_layout, slot).location),
                );
            }
        }
        let mut seen = HashSet::new();
        encoded
            .into_iter()
            .map(Location::decode)
            .filter(|location| seen.insert(*location))
            .collect()
    }

    pub(crate) fn insert(&mut self, hash: &[u8; 32], location: Location) -> Result<()> {
        let fingerprint = self.fingerprint(hash);
        let entry = PackedEntry {
            mini: fingerprint.mini,
            remainder: fingerprint.remainder,
            location: location.encode(self.region_bits),
            crumb: 0,
        };
        let saved = self.front[fingerprint.front].clone();
        let overflow = self.front[fingerprint.front].insert_front(&self.front_layout, entry);
        let Some(mut overflow) = overflow else {
            self.len += 1;
            return Ok(());
        };
        let [first, second] = self.back_locations(fingerprint.front);
        let destination = if self.back[first.bucket].len(&self.back_layout)
            <= self.back[second.bucket].len(&self.back_layout)
        {
            first
        } else {
            second
        };
        overflow.crumb = destination.crumb;
        if !self.back[destination.bucket].insert_back(&self.back_layout, overflow) {
            self.front[fingerprint.front] = saved;
            return Err(KvError::IndexFull);
        }
        self.len += 1;
        Ok(())
    }

    pub(crate) fn remove(&mut self, hash: &[u8; 32], location: Location) -> bool {
        let fingerprint = self.fingerprint(hash);
        let encoded = location.encode(self.region_bits);
        let was_full =
            self.front[fingerprint.front].len(&self.front_layout) == self.front_layout.capacity;
        let front_slots = self.front[fingerprint.front].matching_slots(
            &self.front_layout,
            fingerprint.mini,
            fingerprint.remainder,
            None,
        );
        if let Some(slot) = front_slots.into_iter().find(|slot| {
            self.front[fingerprint.front]
                .entry(&self.front_layout, *slot)
                .location
                == encoded
        }) {
            self.front[fingerprint.front].remove_at(&self.front_layout, fingerprint.mini, slot);
            if was_full {
                self.promote(fingerprint.front);
            }
            self.len -= 1;
            return true;
        }
        if !was_full {
            return false;
        }
        for back in self.back_locations(fingerprint.front) {
            let slots = self.back[back.bucket].matching_slots(
                &self.back_layout,
                fingerprint.mini,
                fingerprint.remainder,
                Some(back.crumb),
            );
            if let Some(slot) = slots.into_iter().find(|slot| {
                self.back[back.bucket]
                    .entry(&self.back_layout, *slot)
                    .location
                    == encoded
            }) {
                self.back[back.bucket].remove_at(&self.back_layout, fingerprint.mini, slot);
                self.len -= 1;
                return true;
            }
        }
        false
    }

    fn replace_location(
        &mut self,
        hash: &[u8; 32],
        previous: Location,
        replacement: Location,
    ) -> bool {
        if previous == replacement {
            return true;
        }
        let fingerprint = self.fingerprint(hash);
        let previous = previous.encode(self.region_bits);
        let replacement = replacement.encode(self.region_bits);
        let front_slots = self.front[fingerprint.front].matching_slots(
            &self.front_layout,
            fingerprint.mini,
            fingerprint.remainder,
            None,
        );
        if let Some(slot) = front_slots.into_iter().find(|slot| {
            self.front[fingerprint.front]
                .entry(&self.front_layout, *slot)
                .location
                == previous
        }) {
            let mut entry = self.front[fingerprint.front].entry(&self.front_layout, slot);
            entry.location = replacement;
            self.front[fingerprint.front].write_entry(&self.front_layout, slot, entry);
            return true;
        }
        if self.front[fingerprint.front].len(&self.front_layout) < self.front_layout.capacity {
            return false;
        }
        for back in self.back_locations(fingerprint.front) {
            let slots = self.back[back.bucket].matching_slots(
                &self.back_layout,
                fingerprint.mini,
                fingerprint.remainder,
                Some(back.crumb),
            );
            if let Some(slot) = slots.into_iter().find(|slot| {
                self.back[back.bucket]
                    .entry(&self.back_layout, *slot)
                    .location
                    == previous
            }) {
                let mut entry = self.back[back.bucket].entry(&self.back_layout, slot);
                entry.location = replacement;
                self.back[back.bucket].write_entry(&self.back_layout, slot, entry);
                return true;
            }
        }
        false
    }

    fn promote(&mut self, front: usize) {
        let [first, second] = self.back_locations(front);
        let a = self.back[first.bucket].first_with_crumb(&self.back_layout, first.crumb);
        let b = self.back[second.bucket].first_with_crumb(&self.back_layout, second.crumb);
        let selected = match (a, b) {
            (None, None) => return,
            (Some(candidate), None) => (first, candidate),
            (None, Some(candidate)) => (second, candidate),
            (Some(a), Some(b)) if a.1.mini <= b.1.mini => (first, a),
            (Some(_), Some(b)) => (second, b),
        };
        let (back, (slot, mut entry)) = selected;
        self.back[back.bucket].remove_at(&self.back_layout, entry.mini, slot);
        entry.crumb = 0;
        let overflow = self.front[front].insert_front(&self.front_layout, entry);
        debug_assert!(overflow.is_none());
    }

    fn back_locations(&self, front: usize) -> [BackLocation; 2] {
        let upper = front / self.ratio;
        let low = front % self.ratio;
        let first = BackLocation {
            bucket: front / self.ratio,
            crumb: (self.ratio + low) as u8,
        };
        let second = BackLocation {
            bucket: upper / self.ratio + low * self.back_group_count,
            crumb: (upper % self.ratio) as u8,
        };
        debug_assert!(first.bucket < self.back.len());
        debug_assert!(second.bucket < self.back.len());
        [first, second]
    }
}

#[derive(Clone, Debug)]
struct Record {
    kind: u8,
    page_choice: u8,
    sequence: u64,
    key: Vec<u8>,
    value: Vec<u8>,
}

impl Record {
    fn encoded_len(&self) -> usize {
        RECORD_HEADER + self.key.len() + self.value.len()
    }
}

struct MutableSg {
    bytes: Vec<u8>,
    region: usize,
    generation: u64,
    page_size: usize,
    record_count: usize,
    logical_bytes: u64,
}

enum MutableReplace {
    NotFound,
    Replaced(Location),
    NoSpace,
}

impl MutableSg {
    fn new(config: &Config, region: usize, generation: u64) -> Self {
        let mut sg = Self {
            bytes: vec![0; config.sg_size],
            region,
            generation,
            page_size: config.page_size,
            record_count: 0,
            logical_bytes: 0,
        };
        for page in 0..config.page_count() {
            initialize_page(sg.page_mut(page), generation);
        }
        sg
    }

    fn page(&self, page: usize) -> &[u8] {
        let start = page * self.page_size;
        &self.bytes[start..start + self.page_size]
    }

    fn page_mut(&mut self, page: usize) -> &mut [u8] {
        let start = page * self.page_size;
        &mut self.bytes[start..start + self.page_size]
    }

    fn choose_page(&self, hash: &[u8; 32], record_len: usize) -> Option<(usize, u8)> {
        let pages = self.bytes.len() / self.page_size;
        let first = page_hash(hash, 0, pages);
        let second = page_hash(hash, 1, pages);
        let first_used = page_used(self.page(first));
        let second_used = page_used(self.page(second));
        let first_fits = first_used + record_len <= self.page_size;
        let second_fits = second_used + record_len <= self.page_size;
        match (first_fits, second_fits) {
            (false, false) => None,
            (true, false) => Some((first, 0)),
            (false, true) => Some((second, 1)),
            (true, true) if first_used <= second_used => Some((first, 0)),
            (true, true) => Some((second, 1)),
        }
    }

    fn append(&mut self, mut record: Record, count_logical: bool) -> Option<Location> {
        let (page, choice) = self.choose_page(
            &Key::from(record.key.as_slice()).hashed_key().into_bytes(),
            record.encoded_len(),
        )?;
        record.page_choice = choice;
        append_page(self.page_mut(page), &record);
        self.record_count += 1;
        if count_logical {
            self.logical_bytes += (record.key.len() + record.value.len()) as u64;
        }
        Some(Location {
            region: self.region as u8,
            page_choice: choice,
        })
    }

    fn replace(
        &mut self,
        hash: &[u8; 32],
        mut record: Record,
        count_logical: bool,
    ) -> MutableReplace {
        let page_count = self.bytes.len() / self.page_size;
        let first = page_hash(hash, 0, page_count);
        let second = page_hash(hash, 1, page_count);
        let mut candidate_pages = vec![first];
        if second != first {
            candidate_pages.push(second);
        }

        let mut matches = Vec::new();
        for &page in &candidate_pages {
            matches.extend(
                matching_record_spans(self.page(page), &record.key)
                    .into_iter()
                    .map(|span| (page, span)),
            );
        }
        let Some(&(page, span)) = matches.iter().max_by_key(|(_, span)| span.sequence) else {
            return MutableReplace::NotFound;
        };

        if matches.len() == 1
            && page_used(self.page(page)) - span.len() + record.encoded_len() <= self.page_size
        {
            record.page_choice = span.page_choice;
            replace_page_record(self.page_mut(page), span, &record);
            if count_logical {
                self.logical_bytes += (record.key.len() + record.value.len()) as u64;
            }
            return MutableReplace::Replaced(Location {
                region: self.region as u8,
                page_choice: record.page_choice,
            });
        }

        let saved_pages = candidate_pages
            .iter()
            .map(|&page| (page, self.page(page).to_vec()))
            .collect::<Vec<_>>();
        let saved_record_count = self.record_count;
        let saved_logical_bytes = self.logical_bytes;
        let removed = candidate_pages
            .iter()
            .map(|&page| remove_key_from_page(self.page_mut(page), &record.key))
            .sum::<usize>();
        self.record_count -= removed;

        if let Some(location) = self.append(record, count_logical) {
            return MutableReplace::Replaced(location);
        }

        for (page, bytes) in saved_pages {
            self.page_mut(page).copy_from_slice(&bytes);
        }
        self.record_count = saved_record_count;
        self.logical_bytes = saved_logical_bytes;
        MutableReplace::NoSpace
    }

    fn find(&self, hash: &[u8; 32], key: &[u8], choice: u8) -> Option<Record> {
        let page = page_hash(hash, choice, self.bytes.len() / self.page_size);
        latest_in_page(self.page(page), key)
    }

    fn finalize(&mut self) {
        let pages = self.bytes.len() / self.page_size;
        for page in 0..pages {
            finalize_page(self.page_mut(page));
        }
    }
}

fn initialize_page(page: &mut [u8], generation: u64) {
    page.fill(0);
    put_u32(page, 0, PAGE_MAGIC);
    put_u16(page, 4, PAGE_VERSION);
    put_u16(page, 6, PAGE_HEADER as u16);
    put_u64(page, 8, generation);
    put_u16(page, 16, PAGE_HEADER as u16);
    put_u16(page, 18, 0);
    put_u64(page, 20, 0);
}

fn page_used(page: &[u8]) -> usize {
    get_u16(page, 16) as usize
}

#[derive(Clone, Copy)]
struct RecordSpan {
    start: usize,
    end: usize,
    sequence: u64,
    page_choice: u8,
}

impl RecordSpan {
    fn len(self) -> usize {
        self.end - self.start
    }
}

fn append_page(page: &mut [u8], record: &Record) {
    let used = page_used(page);
    let end = used + record.encoded_len();
    write_record(page, used, record);
    put_u16(page, 16, end as u16);
    put_u16(page, 18, get_u16(page, 18) + 1);
    put_u64(page, 20, 0);
}

fn write_record(page: &mut [u8], offset: usize, record: &Record) {
    let end = offset + record.encoded_len();
    page[offset] = record.kind;
    page[offset + 1] = record.page_choice;
    put_u16(page, offset + 2, record.key.len() as u16);
    put_u32(page, offset + 4, record.value.len() as u32);
    put_u64(page, offset + 8, record.sequence);
    let key_end = offset + RECORD_HEADER + record.key.len();
    page[offset + RECORD_HEADER..key_end].copy_from_slice(&record.key);
    page[key_end..end].copy_from_slice(&record.value);
}

fn matching_record_spans(page: &[u8], key: &[u8]) -> Vec<RecordSpan> {
    let used = page_used(page).min(page.len());
    let count = get_u16(page, 18) as usize;
    let mut offset = PAGE_HEADER;
    let mut result = Vec::new();
    for _ in 0..count {
        if offset + RECORD_HEADER > used {
            break;
        }
        let key_len = get_u16(page, offset + 2) as usize;
        let value_len = get_u32(page, offset + 4) as usize;
        let end = offset + RECORD_HEADER + key_len + value_len;
        if end > used {
            break;
        }
        if &page[offset + RECORD_HEADER..offset + RECORD_HEADER + key_len] == key {
            result.push(RecordSpan {
                start: offset,
                end,
                sequence: get_u64(page, offset + 8),
                page_choice: page[offset + 1],
            });
        }
        offset = end;
    }
    result
}

fn replace_page_record(page: &mut [u8], span: RecordSpan, record: &Record) {
    let old_used = page_used(page);
    let new_end = span.start + record.encoded_len();
    let new_used = old_used - span.len() + record.encoded_len();
    page.copy_within(span.end..old_used, new_end);
    if new_used < old_used {
        page[new_used..old_used].fill(0);
    }
    write_record(page, span.start, record);
    put_u16(page, 16, new_used as u16);
    put_u64(page, 20, 0);
}

fn remove_key_from_page(page: &mut [u8], key: &[u8]) -> usize {
    let old_used = page_used(page).min(page.len());
    let count = get_u16(page, 18) as usize;
    let mut read = PAGE_HEADER;
    let mut write = PAGE_HEADER;
    let mut removed = 0usize;
    for _ in 0..count {
        if read + RECORD_HEADER > old_used {
            break;
        }
        let key_len = get_u16(page, read + 2) as usize;
        let value_len = get_u32(page, read + 4) as usize;
        let end = read + RECORD_HEADER + key_len + value_len;
        if end > old_used {
            break;
        }
        let matches = &page[read + RECORD_HEADER..read + RECORD_HEADER + key_len] == key;
        if matches {
            removed += 1;
        } else {
            if write != read {
                page.copy_within(read..end, write);
            }
            write += end - read;
        }
        read = end;
    }
    page[write..old_used].fill(0);
    put_u16(page, 16, write as u16);
    put_u16(page, 18, (count - removed) as u16);
    put_u64(page, 20, 0);
    removed
}

fn records(page: &[u8]) -> Vec<Record> {
    if page.len() < PAGE_HEADER
        || get_u32(page, 0) != PAGE_MAGIC
        || get_u16(page, 4) != PAGE_VERSION
    {
        return Vec::new();
    }
    let used = page_used(page).min(page.len());
    let count = get_u16(page, 18) as usize;
    let mut offset = PAGE_HEADER;
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        if offset + RECORD_HEADER > used {
            break;
        }
        let key_len = get_u16(page, offset + 2) as usize;
        let value_len = get_u32(page, offset + 4) as usize;
        let end = offset + RECORD_HEADER + key_len + value_len;
        if end > used {
            break;
        }
        let key_end = offset + RECORD_HEADER + key_len;
        result.push(Record {
            kind: page[offset],
            page_choice: page[offset + 1],
            sequence: get_u64(page, offset + 8),
            key: page[offset + RECORD_HEADER..key_end].to_vec(),
            value: page[key_end..end].to_vec(),
        });
        offset = end;
    }
    result
}

fn latest_in_page(page: &[u8], key: &[u8]) -> Option<Record> {
    records(page)
        .into_iter()
        .filter(|record| record.key == key)
        .max_by_key(|record| record.sequence)
}

fn finalize_page(page: &mut [u8]) {
    put_u64(page, 20, 0);
    let checksum = checksum64(page);
    put_u64(page, 20, checksum);
}

fn verify_page(page: &[u8]) -> bool {
    if page.len() < PAGE_HEADER || get_u32(page, 0) != PAGE_MAGIC {
        return false;
    }
    let expected = get_u64(page, 20);
    let mut copy = page.to_vec();
    put_u64(&mut copy, 20, 0);
    expected != 0 && expected == checksum64(&copy)
}

fn page_hash(hash: &[u8; 32], choice: u8, pages: usize) -> usize {
    // hash[0..8] routes to a worker and hash[8..16] feeds the Breadcrumb.
    // The two page choices use independent portions of the digest.
    let start = if choice == 0 { 16 } else { 24 };
    u64::from_le_bytes(hash[start..start + 8].try_into().unwrap()) as usize % pages
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetOutcome {
    Created,
    Replaced,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct KvkacheIoStats {
    pub(crate) data_written: u64,
    pub(crate) data_read: u64,
    pub(crate) index_written: u64,
    pub(crate) index_read: u64,
}

#[derive(Default)]
struct IoCounters {
    data_written: Cell<u64>,
    data_read: Cell<u64>,
    index_written: Cell<u64>,
    index_read: Cell<u64>,
}

#[derive(Clone)]
struct LocatedRecord {
    location: Location,
    record: Record,
}

pub(crate) struct Kvkache {
    config: Config,
    data: File,
    index: LocationBreadcrumb,
    active: Option<MutableSg>,
    slot_generations: Vec<Option<u64>>,
    next_slot: usize,
    next_generation: u64,
    next_sequence: u64,
    pub(crate) data_flushes: u64,
    evictions: u64,
    io: IoCounters,
}

impl Kvkache {
    pub(crate) async fn open(config: Config) -> Result<Self> {
        config.validate()?;
        if let Some(parent) = config.data_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(parent) = config.index_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&config.data_path)
            .await?;
        data.set_len(config.data_bytes()).await?;

        let mut cache = Self {
            index: LocationBreadcrumb::new(&config)?,
            active: None,
            slot_generations: vec![None; config.sg_count],
            next_slot: 0,
            next_generation: 0,
            next_sequence: 0,
            data_flushes: 0,
            evictions: 0,
            io: IoCounters::default(),
            config,
            data,
        };
        if cache.config.recovery_enabled && !cache.load_checkpoint().await? {
            if !cache.config.fallback_to_sg_scan {
                return Err(KvError::Corrupt(
                    "checkpoint is absent or invalid and SG fallback is disabled".into(),
                ));
            }
            cache.rebuild_from_data().await?;
            if cache.slot_generations.iter().any(Option::is_some) {
                cache.save_checkpoint().await?;
            }
        }
        Ok(cache)
    }

    pub(crate) async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let hash = Key::from(key).hashed_key().into_bytes();
        Ok(self
            .locate(&hash, key)
            .await?
            .filter(|located| located.record.kind == RECORD_SET)
            .map(|located| located.record.value))
    }

    async fn get_many(&self, keys: Vec<Vec<u8>>) -> Vec<Result<Option<Vec<u8>>>> {
        let count = keys.len();
        let mut pending = FuturesUnordered::new();
        for (index, key) in keys.into_iter().enumerate() {
            pending.push(async move { (index, self.get(&key).await) });
        }
        let mut results = (0..count).map(|_| None).collect::<Vec<_>>();
        while let Some((index, result)) = pending.next().await {
            results[index] = Some(result);
        }
        results
            .into_iter()
            .map(|result| result.expect("every get future completes"))
            .collect()
    }

    pub(crate) async fn set(&mut self, key: &[u8], value: &[u8]) -> Result<SetOutcome> {
        if key.len() > u16::MAX as usize || value.len() > u32::MAX as usize {
            return Err(KvError::RecordTooLarge {
                bytes: RECORD_HEADER + key.len() + value.len(),
                capacity: self.config.page_size - PAGE_HEADER,
            });
        }
        let record_len = RECORD_HEADER + key.len() + value.len();
        if record_len > self.config.page_size - PAGE_HEADER {
            return Err(KvError::RecordTooLarge {
                bytes: record_len,
                capacity: self.config.page_size - PAGE_HEADER,
            });
        }
        let hash = Key::from(key).hashed_key().into_bytes();
        let previous = self.locate(&hash, key).await?;
        let sequence = self.take_sequence();
        let record = Record {
            kind: RECORD_SET,
            page_choice: 0,
            sequence,
            key: key.to_vec(),
            value: value.to_vec(),
        };
        let previous_is_active = previous.as_ref().is_some_and(|previous| {
            self.active
                .as_ref()
                .is_some_and(|active| active.region == previous.location.region as usize)
        });
        let location = if previous_is_active {
            match self
                .active
                .as_mut()
                .unwrap()
                .replace(&hash, record.clone(), true)
            {
                MutableReplace::Replaced(location) => location,
                MutableReplace::NotFound | MutableReplace::NoSpace => {
                    self.append_with_retry(record, true).await?
                }
            }
        } else {
            self.append_with_retry(record, true).await?
        };
        if let Some(previous) = &previous {
            if !self
                .index
                .replace_location(&hash, previous.location, location)
            {
                return Err(KvError::Corrupt(
                    "updated key is missing from the Breadcrumb".into(),
                ));
            }
        } else {
            self.index.insert(&hash, location)?;
        }
        Ok(if previous.is_some() {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        })
    }

    pub(crate) async fn delete(&mut self, key: &[u8]) -> Result<bool> {
        let hash = Key::from(key).hashed_key().into_bytes();
        let Some(previous) = self.locate(&hash, key).await? else {
            return Ok(false);
        };
        let sequence = self.take_sequence();
        let tombstone = Record {
            kind: RECORD_DELETE,
            page_choice: 0,
            sequence,
            key: key.to_vec(),
            value: Vec::new(),
        };
        self.append_with_retry(tombstone, true).await?;
        let removed = self.index.remove(&hash, previous.location);
        debug_assert!(removed);
        Ok(true)
    }

    pub(crate) async fn sync(&mut self) -> Result<()> {
        let checkpointed_by_flush = self.active.is_some() && self.config.checkpoint_on_sg_flush;
        self.flush_active().await?;
        if !checkpointed_by_flush {
            self.save_checkpoint().await?;
        }
        Ok(())
    }

    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }

    async fn locate(&self, hash: &[u8; 32], key: &[u8]) -> Result<Option<LocatedRecord>> {
        let mut latest: Option<LocatedRecord> = None;
        for location in self.index.candidates(hash) {
            let record = if self
                .active
                .as_ref()
                .is_some_and(|active| active.region == location.region as usize)
            {
                self.active
                    .as_ref()
                    .unwrap()
                    .find(hash, key, location.page_choice)
            } else {
                self.read_location(hash, key, location).await?
            };
            if let Some(record) = record
                && latest
                    .as_ref()
                    .is_none_or(|current| record.sequence > current.record.sequence)
            {
                latest = Some(LocatedRecord { location, record });
            }
        }
        Ok(latest)
    }

    async fn read_location(
        &self,
        hash: &[u8; 32],
        key: &[u8],
        location: Location,
    ) -> Result<Option<Record>> {
        if self.slot_generations[location.region as usize].is_none() {
            return Ok(None);
        }
        let page = page_hash(hash, location.page_choice, self.config.page_count());
        let bytes = self.read_page(location.region as usize, page).await?;
        Ok(latest_in_page(&bytes, key))
    }

    async fn append_with_retry(&mut self, record: Record, count_logical: bool) -> Result<Location> {
        loop {
            self.ensure_active().await?;
            if let Some(location) = self
                .active
                .as_mut()
                .unwrap()
                .append(record.clone(), count_logical)
            {
                return Ok(location);
            }
            self.flush_active().await?;
        }
    }

    async fn ensure_active(&mut self) -> Result<()> {
        if self.active.is_some() {
            return Ok(());
        }
        let region = self.next_slot;
        if self.slot_generations[region].is_some() {
            self.evict_region(region).await?;
        }
        let generation = self.next_generation;
        self.next_generation += 1;
        self.active = Some(MutableSg::new(&self.config, region, generation));
        Ok(())
    }

    async fn flush_active(&mut self) -> Result<()> {
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        if active.record_count == 0 {
            return Ok(());
        }
        active.finalize();
        let offset = active.region as u64 * self.config.sg_size as u64;
        let bytes = active.bytes;
        let write = self.data.write_all_at(bytes, offset);
        let BufResult(result, bytes) = compio::runtime::time::timeout(
            Duration::from_micros(self.config.write_max_time_us),
            write,
        )
        .await
        .map_err(|_| KvError::Timeout("SG write"))?;
        result?;
        self.io
            .data_written
            .set(self.io.data_written.get() + bytes.len() as u64);
        self.data.sync_data().await?;
        self.slot_generations[active.region] = Some(active.generation);
        self.next_slot = (active.region + 1) % self.config.sg_count;
        self.data_flushes += 1;
        if self.config.checkpoint_on_sg_flush {
            self.save_checkpoint().await?;
        }
        Ok(())
    }

    async fn evict_region(&mut self, region: usize) -> Result<()> {
        let records = self.read_sg_records(region).await?;
        let mut newest = HashMap::<Vec<u8>, Record>::new();
        for record in records {
            if newest
                .get(&record.key)
                .is_none_or(|current| record.sequence > current.sequence)
            {
                newest.insert(record.key.clone(), record);
            }
        }
        for record in newest
            .into_values()
            .filter(|record| record.kind == RECORD_SET)
        {
            let hash = Key::from(record.key.as_slice()).hashed_key().into_bytes();
            let location = Location {
                region: region as u8,
                page_choice: record.page_choice,
            };
            if self
                .locate(&hash, &record.key)
                .await?
                .is_some_and(|current| {
                    current.location == location && current.record.sequence == record.sequence
                })
            {
                let _ = self.index.remove(&hash, location);
            }
        }
        self.slot_generations[region] = None;
        self.evictions += 1;
        Ok(())
    }

    async fn read_page(&self, region: usize, page: usize) -> Result<Vec<u8>> {
        let offset =
            region as u64 * self.config.sg_size as u64 + page as u64 * self.config.page_size as u64;
        let read = self
            .data
            .read_exact_at(Vec::with_capacity(self.config.page_size), offset);
        let BufResult(result, bytes) = compio::runtime::time::timeout(
            Duration::from_micros(self.config.read_max_time_us),
            read,
        )
        .await
        .map_err(|_| KvError::Timeout("page read"))?;
        result?;
        self.io
            .data_read
            .set(self.io.data_read.get() + bytes.len() as u64);
        Ok(bytes)
    }

    async fn read_sg_records(&self, region: usize) -> Result<Vec<Record>> {
        let mut result = Vec::new();
        for page in 0..self.config.page_count() {
            let bytes = self.read_page(region, page).await?;
            if verify_page(&bytes) {
                result.extend(records(&bytes));
            }
        }
        Ok(result)
    }

    async fn scan_slot_generation(&self, region: usize) -> Result<Option<u64>> {
        let first = self.read_page(region, 0).await?;
        if !verify_page(&first) {
            return Ok(None);
        }
        let generation = get_u64(&first, 8);
        for page in 1..self.config.page_count() {
            let bytes = self.read_page(region, page).await?;
            if !verify_page(&bytes) || get_u64(&bytes, 8) != generation {
                return Ok(None);
            }
        }
        Ok(Some(generation))
    }

    async fn rebuild_from_data(&mut self) -> Result<()> {
        let mut occupied = Vec::new();
        for region in 0..self.config.sg_count {
            if let Some(generation) = self.scan_slot_generation(region).await? {
                self.slot_generations[region] = Some(generation);
                occupied.push((generation, region));
            }
        }
        occupied.sort_unstable();
        let mut latest = HashMap::<Vec<u8>, (Record, Location)>::new();
        for (_, region) in &occupied {
            for record in self.read_sg_records(*region).await? {
                let location = Location {
                    region: *region as u8,
                    page_choice: record.page_choice,
                };
                if latest
                    .get(&record.key)
                    .is_none_or(|(current, _)| record.sequence > current.sequence)
                {
                    latest.insert(record.key.clone(), (record, location));
                }
            }
        }
        self.index = LocationBreadcrumb::new(&self.config)?;
        self.next_sequence = 0;
        for (record, location) in latest.into_values() {
            self.next_sequence = self.next_sequence.max(record.sequence + 1);
            if record.kind == RECORD_SET {
                let hash = Key::from(record.key.as_slice()).hashed_key().into_bytes();
                self.index.insert(&hash, location)?;
            }
        }
        if let Some((generation, region)) = occupied.last().copied() {
            self.next_generation = generation + 1;
            self.next_slot = (region + 1) % self.config.sg_count;
        }
        Ok(())
    }

    async fn save_checkpoint(&self) -> Result<()> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(CHECKPOINT_MAGIC);
        push_u32(&mut bytes, CHECKPOINT_VERSION);
        for value in self.config.signature() {
            push_u64(&mut bytes, value);
        }
        push_u64(&mut bytes, self.next_slot as u64);
        push_u64(&mut bytes, self.next_generation);
        push_u64(&mut bytes, self.next_sequence);
        push_u64(&mut bytes, self.index.len as u64);
        push_u64(&mut bytes, self.index.front.len() as u64);
        push_u64(&mut bytes, self.index.back.len() as u64);
        push_u64(&mut bytes, self.index.back_group_count as u64);
        for generation in &self.slot_generations {
            push_u64(&mut bytes, generation.unwrap_or(NONE_GENERATION));
        }
        for bucket in &self.index.front {
            bytes.extend_from_slice(&bucket.bytes);
        }
        for bucket in &self.index.back {
            bytes.extend_from_slice(&bucket.bytes);
        }
        let checksum = checksum64(&bytes);
        push_u64(&mut bytes, checksum);

        let temporary = self.config.index_path.with_extension("index.tmp");
        {
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&temporary)
                .await?;
            let write = file.write_all_at(bytes, 0);
            let BufResult(result, returned) = compio::runtime::time::timeout(
                Duration::from_micros(self.config.write_max_time_us),
                write,
            )
            .await
            .map_err(|_| KvError::Timeout("checkpoint write"))?;
            result?;
            bytes = returned;
            file.sync_all().await?;
            file.close().await?;
        }
        compio::fs::rename(&temporary, &self.config.index_path).await?;
        self.io
            .index_written
            .set(self.io.index_written.get() + bytes.len() as u64);
        if let Some(parent) = self.config.index_path.parent() {
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY)
                .open(parent)
                .await?;
            directory.sync_all().await?;
            directory.close().await?;
        }
        Ok(())
    }

    async fn load_checkpoint(&mut self) -> Result<bool> {
        let checkpoint_read = compio::fs::read(&self.config.index_path);
        let mut bytes = match compio::runtime::time::timeout(
            Duration::from_micros(self.config.read_max_time_us),
            checkpoint_read,
        )
        .await
        .map_err(|_| KvError::Timeout("checkpoint read"))?
        {
            Ok(bytes) => {
                self.io
                    .index_read
                    .set(self.io.index_read.get() + bytes.len() as u64);
                bytes
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        if bytes.len() < 16 {
            return Ok(false);
        }
        let stored_checksum = u64::from_le_bytes(bytes[bytes.len() - 8..].try_into().unwrap());
        bytes.truncate(bytes.len() - 8);
        if checksum64(&bytes) != stored_checksum {
            return Ok(false);
        }
        let mut cursor = Cursor::new(&bytes);
        if cursor.take(8)? != CHECKPOINT_MAGIC || cursor.u32()? != CHECKPOINT_VERSION {
            return Ok(false);
        }
        for expected in self.config.signature() {
            if cursor.u64()? != expected {
                return Ok(false);
            }
        }
        let next_slot = cursor.u64()? as usize;
        let next_generation = cursor.u64()?;
        let next_sequence = cursor.u64()?;
        let len = cursor.u64()? as usize;
        let front_count = cursor.u64()? as usize;
        let back_count = cursor.u64()? as usize;
        let back_group_count = cursor.u64()? as usize;
        if front_count != self.index.front.len()
            || back_count != self.index.back.len()
            || back_group_count != self.index.back_group_count
        {
            return Ok(false);
        }
        let mut generations = Vec::with_capacity(self.config.sg_count);
        for region in 0..self.config.sg_count {
            let value = cursor.u64()?;
            let generation = (value != NONE_GENERATION).then_some(value);
            if self.scan_slot_generation(region).await? != generation {
                return Ok(false);
            }
            generations.push(generation);
        }
        for bucket in &mut self.index.front {
            bucket.bytes.copy_from_slice(cursor.take(BUCKET_BYTES)?);
        }
        for bucket in &mut self.index.back {
            bucket.bytes.copy_from_slice(cursor.take(BUCKET_BYTES)?);
        }
        if !cursor.remaining().is_empty() {
            return Ok(false);
        }
        self.index.len = len;
        self.slot_generations = generations;
        self.next_slot = next_slot;
        self.next_generation = next_generation;
        self.next_sequence = next_sequence;
        Ok(true)
    }

    pub(crate) fn stats(&self) -> String {
        let io = self.io_stats();
        format!(
            "keys={} index_load={:.2}% index_memory={:.2}MiB ({:.3}B/planned-key) modeled_resident={:.2}MiB front_buckets={} front_capacity={} back_buckets={} back_capacity={} next_slot={} generations={} flushes={} evictions={} data_read={} data_written={} index_read={} index_written={}",
            self.index.len(),
            self.index.load_factor() * 100.0,
            self.index.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.index.memory_bytes() as f64 / self.config.index_capacity as f64,
            self.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.index.front.len(),
            self.index.front_layout.capacity,
            self.index.back.len(),
            self.index.back_layout.capacity,
            self.next_slot,
            self.next_generation,
            self.data_flushes,
            self.evictions,
            io.data_read,
            io.data_written,
            io.index_read,
            io.index_written,
        )
    }

    // Used by the cross-prototype benchmark; the standalone CLI only reports
    // cumulative counters and therefore does not reset them.
    #[allow(dead_code)]
    pub(crate) fn reset_io_stats(&self) {
        self.io.data_written.set(0);
        self.io.data_read.set(0);
        self.io.index_written.set(0);
        self.io.index_read.set(0);
    }

    pub(crate) fn io_stats(&self) -> KvkacheIoStats {
        KvkacheIoStats {
            data_written: self.io.data_written.get(),
            data_read: self.io.data_read.get(),
            index_written: self.io.index_written.get(),
            index_read: self.io.index_read.get(),
        }
    }

    pub(crate) fn memory_bytes(&self) -> usize {
        self.index.memory_bytes()
            // One mutable SG is the steady-state write buffer. It is released
            // after `sync`, but must be budgeted for active operation.
            + self.config.sg_size
            + self.slot_generations.capacity() * std::mem::size_of::<Option<u64>>()
    }
}

enum WorkerRequest {
    Get {
        key: Vec<u8>,
        response: flume::Sender<Result<WorkerResponse>>,
    },
    Set {
        key: Vec<u8>,
        value: Vec<u8>,
        response: flume::Sender<Result<WorkerResponse>>,
    },
    Delete {
        key: Vec<u8>,
        response: flume::Sender<Result<WorkerResponse>>,
    },
    Stats {
        response: flume::Sender<Result<WorkerResponse>>,
    },
    Sync {
        response: flume::Sender<Result<WorkerResponse>>,
    },
    Shutdown {
        response: flume::Sender<Result<WorkerResponse>>,
    },
}

#[derive(Debug)]
enum WorkerResponse {
    Value(Option<Vec<u8>>),
    Set(SetOutcome),
    Deleted(bool),
    Stats(String),
    Synced,
    Shutdown,
}

#[derive(Debug)]
pub(crate) enum BenchmarkOperation {
    Get(Vec<u8>),
    Set(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
}

impl BenchmarkOperation {
    fn key(&self) -> &[u8] {
        match self {
            Self::Get(key) | Self::Delete(key) | Self::Set(key, _) => key,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct BenchmarkBatchStats {
    pub(crate) operations: usize,
    pub(crate) gets: usize,
    pub(crate) hits: usize,
    pub(crate) sets: usize,
    pub(crate) creates: usize,
    pub(crate) replaces: usize,
    pub(crate) deletes: usize,
    pub(crate) deleted: usize,
    pub(crate) latency_ns: Vec<u64>,
}

impl BenchmarkBatchStats {
    pub(crate) fn merge(&mut self, mut other: Self) {
        self.operations += other.operations;
        self.gets += other.gets;
        self.hits += other.hits;
        self.sets += other.sets;
        self.creates += other.creates;
        self.replaces += other.replaces;
        self.deletes += other.deletes;
        self.deleted += other.deleted;
        self.latency_ns.append(&mut other.latency_ns);
    }
}

#[derive(Clone, Copy)]
enum BenchmarkResponseKind {
    Get,
    Set,
    Delete,
}

struct PendingBenchmarkRequest {
    response: flume::Receiver<Result<WorkerResponse>>,
    kind: BenchmarkResponseKind,
    started: std::time::Instant,
}

async fn worker_loop(
    mut cache: Kvkache,
    receiver: flume::Receiver<WorkerRequest>,
    io_config: IoUringConfig,
) -> Result<()> {
    loop {
        let first = receiver
            .recv_async()
            .await
            .map_err(|_| KvError::Worker("request queue disconnected".into()))?;
        let wait_us = io_config.batch_max_wait_us;
        let mut batch = VecDeque::with_capacity(io_config.batch_size);
        batch.push_back(first);

        if batch.len() < io_config.batch_size
            && wait_us > 0
            && let Ok(Ok(request)) = compio::runtime::time::timeout(
                Duration::from_micros(wait_us),
                receiver.recv_async(),
            )
            .await
        {
            batch.push_back(request);
        }
        while batch.len() < io_config.batch_size {
            match receiver.try_recv() {
                Ok(request) => batch.push_back(request),
                Err(flume::TryRecvError::Empty | flume::TryRecvError::Disconnected) => break,
            }
        }

        if process_worker_batch(&mut cache, batch, io_config.max_inflight_per_worker).await? {
            return Ok(());
        }
    }
}

async fn process_worker_batch(
    cache: &mut Kvkache,
    mut batch: VecDeque<WorkerRequest>,
    max_inflight: usize,
) -> Result<bool> {
    let mut shutdown_response = None;

    while let Some(request) = batch.pop_front() {
        match request {
            WorkerRequest::Get { key, response } => {
                let mut keys = vec![key];
                let mut responses = vec![response];
                while keys.len() < max_inflight {
                    let Some(WorkerRequest::Get { .. }) = batch.front() else {
                        break;
                    };
                    let WorkerRequest::Get { key, response } = batch.pop_front().unwrap() else {
                        unreachable!()
                    };
                    keys.push(key);
                    responses.push(response);
                }
                let results = cache.get_many(keys).await;
                for (response, result) in responses.into_iter().zip(results) {
                    let _ = response.send(result.map(WorkerResponse::Value));
                }
            }
            WorkerRequest::Set {
                key,
                value,
                response,
            } => match cache.set(&key, &value).await {
                Ok(outcome) => {
                    let _ = response.send(Ok(WorkerResponse::Set(outcome)));
                }
                Err(error) => {
                    let _ = response.send(Err(error));
                }
            },
            WorkerRequest::Delete { key, response } => match cache.delete(&key).await {
                Ok(deleted) => {
                    let _ = response.send(Ok(WorkerResponse::Deleted(deleted)));
                }
                Err(error) => {
                    let _ = response.send(Err(error));
                }
            },
            WorkerRequest::Stats { response } => {
                let cpu = unsafe { libc::sched_getcpu() };
                let _ = response.send(Ok(WorkerResponse::Stats(format!(
                    "cpu_id={cpu} {}",
                    cache.stats()
                ))));
            }
            WorkerRequest::Sync { response } => {
                let result = cache.sync().await.map(|()| WorkerResponse::Synced);
                let _ = response.send(result);
            }
            WorkerRequest::Shutdown { response } => {
                shutdown_response = Some(response);
                break;
            }
        }
    }

    if shutdown_response.is_some() {
        match cache.sync().await {
            Ok(()) => {}
            Err(error) => {
                let message = error.to_string();
                if let Some(response) = shutdown_response {
                    let _ = response.send(Err(KvError::Worker(message.clone())));
                }
                return Err(KvError::Worker(message));
            }
        }
    }

    if let Some(response) = shutdown_response {
        let _ = response.send(Ok(WorkerResponse::Shutdown));
        return Ok(true);
    }
    Ok(false)
}

struct WorkerHandle {
    sender: flume::Sender<WorkerRequest>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub(crate) struct ThreadedKvkache {
    config: AppConfig,
    workers: Vec<WorkerHandle>,
}

impl ThreadedKvkache {
    pub(crate) fn start(config: AppConfig) -> Result<Self> {
        config.validate()?;
        fs::create_dir_all(&config.storage.directory)?;
        let (started_tx, started_rx) =
            flume::bounded::<std::result::Result<(), String>>(config.runtime.thread_count);
        let queue_capacity = config
            .io_uring
            .batch_size
            .saturating_mul(config.io_uring.max_inflight_per_worker)
            .max(64);
        let mut workers = Vec::with_capacity(config.runtime.thread_count);

        for thread_id in 0..config.runtime.thread_count {
            let (sender, receiver) = flume::bounded(queue_capacity);
            let started_tx = started_tx.clone();
            let shard_config = config.worker_config(thread_id);
            let io_config = config.io_uring.clone();
            let cpu_id = config.runtime.cpu_ids[thread_id];
            let event_interval = config.runtime.event_interval;
            let thread = std::thread::Builder::new()
                .name(format!("kvkache-worker-{thread_id}"))
                .spawn(move || {
                    let mut proactor = ProactorBuilder::new();
                    proactor.capacity(io_config.entries_per_worker);
                    let cpus = HashSet::from([cpu_id]);
                    let runtime = RuntimeBuilder::new()
                        .with_proactor(proactor)
                        .thread_affinity(cpus)
                        .event_interval(event_interval)
                        .build();
                    let runtime = match runtime {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            let _ = started_tx.send(Err(error.to_string()));
                            return;
                        }
                    };
                    runtime.block_on(async move {
                        let actual_cpu = unsafe { libc::sched_getcpu() };
                        if actual_cpu < 0 || actual_cpu as usize != cpu_id {
                            let _ = started_tx.send(Err(format!(
                                "thread {thread_id} expected CPU {cpu_id}, running on CPU {actual_cpu}"
                            )));
                            return;
                        }
                        let cache = match Kvkache::open(shard_config).await {
                            Ok(cache) => cache,
                            Err(error) => {
                                let _ = started_tx.send(Err(error.to_string()));
                                return;
                            }
                        };
                        let _ = started_tx.send(Ok(()));
                        if let Err(error) = worker_loop(cache, receiver, io_config).await {
                            eprintln!("worker {thread_id} stopped: {error}");
                        }
                    });
                })?;
            workers.push(WorkerHandle {
                sender,
                thread: Some(thread),
            });
        }
        drop(started_tx);

        for _ in 0..config.runtime.thread_count {
            match started_rx
                .recv()
                .map_err(|_| KvError::Worker("worker startup channel closed".into()))?
            {
                Ok(()) => {}
                Err(message) => {
                    for worker in &workers {
                        let (response, _) = flume::bounded(1);
                        let _ = worker.sender.send(WorkerRequest::Shutdown { response });
                    }
                    for worker in &mut workers {
                        if let Some(thread) = worker.thread.take() {
                            let _ = thread.join();
                        }
                    }
                    return Err(KvError::Worker(message));
                }
            }
        }

        Ok(Self { config, workers })
    }

    pub(crate) fn owner(&self, key: &[u8]) -> usize {
        let hash = Key::from(key).hashed_key().into_bytes();
        u64::from_le_bytes(hash[..8].try_into().unwrap()) as usize % self.workers.len()
    }

    fn request(
        &self,
        worker: usize,
        build: impl FnOnce(flume::Sender<Result<WorkerResponse>>) -> WorkerRequest,
    ) -> Result<WorkerResponse> {
        let (response_tx, response_rx) = flume::bounded(1);
        let request_started = std::time::Instant::now();
        self.workers[worker]
            .sender
            .send_timeout(
                build(response_tx),
                Duration::from_micros(self.config.timeouts.input_max_time_us),
            )
            .map_err(|_| KvError::Timeout("request input"))?;
        let elapsed = request_started.elapsed();
        let request_limit = Duration::from_micros(self.config.timeouts.request_max_time_us);
        let output_limit = Duration::from_micros(self.config.timeouts.output_max_time_us);
        let remaining = request_limit.saturating_sub(elapsed).min(output_limit);
        response_rx
            .recv_timeout(remaining)
            .map_err(|_| KvError::Timeout("request output"))?
    }

    pub(crate) fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>> {
        let worker = self.owner(&key);
        match self.request(worker, |response| WorkerRequest::Get { key, response })? {
            WorkerResponse::Value(value) => Ok(value),
            response => Err(KvError::Worker(format!(
                "unexpected get response: {response:?}"
            ))),
        }
    }

    pub(crate) fn set(&self, key: Vec<u8>, value: Vec<u8>) -> Result<SetOutcome> {
        let worker = self.owner(&key);
        match self.request(worker, |response| WorkerRequest::Set {
            key,
            value,
            response,
        })? {
            WorkerResponse::Set(outcome) => Ok(outcome),
            response => Err(KvError::Worker(format!(
                "unexpected set response: {response:?}"
            ))),
        }
    }

    pub(crate) fn delete(&self, key: Vec<u8>) -> Result<bool> {
        let worker = self.owner(&key);
        match self.request(worker, |response| WorkerRequest::Delete { key, response })? {
            WorkerResponse::Deleted(deleted) => Ok(deleted),
            response => Err(KvError::Worker(format!(
                "unexpected delete response: {response:?}"
            ))),
        }
    }

    pub(crate) fn run_benchmark_batch(
        &self,
        operations: Vec<BenchmarkOperation>,
        max_outstanding_per_worker: usize,
    ) -> Result<BenchmarkBatchStats> {
        if max_outstanding_per_worker == 0 {
            return Err(KvError::InvalidConfig(
                "benchmark max outstanding per worker must be non-zero".into(),
            ));
        }
        let max_outstanding = max_outstanding_per_worker
            .checked_mul(self.workers.len())
            .ok_or_else(|| KvError::InvalidConfig("benchmark window is too large".into()))?;
        let mut pending = VecDeque::with_capacity(max_outstanding);
        let mut stats = BenchmarkBatchStats {
            latency_ns: Vec::with_capacity(operations.len()),
            ..BenchmarkBatchStats::default()
        };

        for operation in operations {
            if pending.len() == max_outstanding {
                self.finish_benchmark_request(pending.pop_front().unwrap(), &mut stats)?;
            }
            let worker = self.owner(operation.key());
            let (response_tx, response_rx) = flume::bounded(1);
            let (request, kind) = match operation {
                BenchmarkOperation::Get(key) => (
                    WorkerRequest::Get {
                        key,
                        response: response_tx,
                    },
                    BenchmarkResponseKind::Get,
                ),
                BenchmarkOperation::Set(key, value) => (
                    WorkerRequest::Set {
                        key,
                        value,
                        response: response_tx,
                    },
                    BenchmarkResponseKind::Set,
                ),
                BenchmarkOperation::Delete(key) => (
                    WorkerRequest::Delete {
                        key,
                        response: response_tx,
                    },
                    BenchmarkResponseKind::Delete,
                ),
            };
            let started = std::time::Instant::now();
            self.workers[worker]
                .sender
                .send_timeout(
                    request,
                    Duration::from_micros(self.config.timeouts.input_max_time_us),
                )
                .map_err(|_| KvError::Timeout("benchmark request input"))?;
            pending.push_back(PendingBenchmarkRequest {
                response: response_rx,
                kind,
                started,
            });
        }
        while let Some(request) = pending.pop_front() {
            self.finish_benchmark_request(request, &mut stats)?;
        }
        Ok(stats)
    }

    fn finish_benchmark_request(
        &self,
        pending: PendingBenchmarkRequest,
        stats: &mut BenchmarkBatchStats,
    ) -> Result<()> {
        let request_limit = Duration::from_micros(self.config.timeouts.request_max_time_us);
        let output_limit = Duration::from_micros(self.config.timeouts.output_max_time_us);
        let remaining = request_limit
            .saturating_sub(pending.started.elapsed())
            .min(output_limit);
        let response = pending
            .response
            .recv_timeout(remaining)
            .map_err(|_| KvError::Timeout("benchmark request output"))??;
        stats.operations += 1;
        stats
            .latency_ns
            .push(pending.started.elapsed().as_nanos() as u64);
        match (pending.kind, response) {
            (BenchmarkResponseKind::Get, WorkerResponse::Value(value)) => {
                stats.gets += 1;
                stats.hits += value.is_some() as usize;
            }
            (BenchmarkResponseKind::Set, WorkerResponse::Set(outcome)) => {
                stats.sets += 1;
                match outcome {
                    SetOutcome::Created => stats.creates += 1,
                    SetOutcome::Replaced => stats.replaces += 1,
                }
            }
            (BenchmarkResponseKind::Delete, WorkerResponse::Deleted(deleted)) => {
                stats.deletes += 1;
                stats.deleted += deleted as usize;
            }
            (_, response) => {
                return Err(KvError::Worker(format!(
                    "unexpected benchmark response: {response:?}"
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn stats(&self) -> Result<Vec<String>> {
        self.workers
            .iter()
            .enumerate()
            .map(|(thread_id, _)| {
                match self.request(thread_id, |response| WorkerRequest::Stats { response })? {
                    WorkerResponse::Stats(stats) => Ok(format!("thread={thread_id} {stats}")),
                    response => Err(KvError::Worker(format!(
                        "unexpected stats response: {response:?}"
                    ))),
                }
            })
            .collect()
    }

    pub(crate) fn sync(&self) -> Result<()> {
        for thread_id in 0..self.workers.len() {
            match self.request(thread_id, |response| WorkerRequest::Sync { response })? {
                WorkerResponse::Synced => {}
                response => {
                    return Err(KvError::Worker(format!(
                        "unexpected sync response: {response:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn shutdown(&mut self) -> Result<()> {
        for thread_id in 0..self.workers.len() {
            match self.request(thread_id, |response| WorkerRequest::Shutdown { response })? {
                WorkerResponse::Shutdown => {}
                response => {
                    return Err(KvError::Worker(format!(
                        "unexpected shutdown response: {response:?}"
                    )));
                }
            }
        }
        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                thread
                    .join()
                    .map_err(|_| KvError::Worker("worker thread panicked".into()))?;
            }
        }
        Ok(())
    }
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
        let end = self.offset.saturating_add(length);
        if end > self.bytes.len() {
            return Err(KvError::Corrupt("truncated index checkpoint".into()));
        }
        let result = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(result)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn checksum64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

fn get_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn main() -> std::result::Result<(), Box<dyn Error>> {
    let (config, command) = match AppConfig::parse() {
        Ok(parsed) => parsed,
        Err(KvError::Usage(message)) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
        Err(error) => return Err(error.into()),
    };
    let mut cache = ThreadedKvkache::start(config)?;
    let operation = (|| -> Result<()> {
        match command {
            Command::Get(key) => match cache.get(key)? {
                Some(value) => println!("{}", String::from_utf8_lossy(&value)),
                None => println!("(nil)"),
            },
            Command::Set(key, value) => println!("{:?}", cache.set(key, value)?),
            Command::Delete(key) => {
                println!(
                    "{}",
                    if cache.delete(key)? {
                        "Deleted"
                    } else {
                        "NotFound"
                    }
                );
            }
            Command::Sync => {
                cache.sync()?;
                println!("Synced");
            }
            Command::Stats => {
                for stats in cache.stats()? {
                    println!("{stats}");
                }
            }
        }
        Ok(())
    })();
    let shutdown = cache.shutdown();
    operation?;
    shutdown?;
    Ok(())
}
