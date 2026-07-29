//! Application and per-worker configuration for the OpenKache server.
//!
//! Storage uses fixed 4 KiB Buckets inside Segments. Each worker owns paired
//! Segment and Blob files plus one in-memory lookup Table. The Segment file
//! embeds recovery control pages used to rebuild committed state at startup.

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::BUCKET_BYTES;
use crate::error::{KvError, Result};

const DEFAULT_MAX_ITEM_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BucketSelectionPolicy {
    #[default]
    LeastUsed,
    MostUsed,
}

/// QUIC protocol implementation used by the network server.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum QuicBackend {
    /// Quinn protocol state machine driven by Compio packet I/O.
    Quinn,
    /// Compio packet I/O backed by `noq-proto`.
    Noq,
    /// Compio packet I/O backed by Cloudflare quiche.
    Quiche,
    /// Reserved selection for a future Mozilla neqo adapter.
    Neqo,
}

impl QuicBackend {
    /// Returns the stable configuration and diagnostics label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quinn => "quinn",
            Self::Noq => "noq",
            Self::Quiche => "quiche",
            Self::Neqo => "neqo",
        }
    }
}

const COMPILED_QUIC_BACKENDS: &[QuicBackend] = &[
    #[cfg(feature = "quic-quinn")]
    QuicBackend::Quinn,
    #[cfg(feature = "quic-noq")]
    QuicBackend::Noq,
    #[cfg(feature = "quic-quiche")]
    QuicBackend::Quiche,
];

/// QUIC transport configuration shared by all protocol implementations.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct QuicConfig {
    /// Protocol implementation selected when the server binds.
    ///
    /// Exactly one compiled backend is selected automatically. Builds containing
    /// multiple backends must set this field explicitly.
    pub backend: Option<QuicBackend>,
}

impl QuicConfig {
    /// Resolves the configured backend or rejects an ambiguous multi-backend build.
    pub fn selected_backend(&self) -> Result<QuicBackend> {
        self.backend.ok_or_else(|| {
            KvError::InvalidConfig(
                "quic.backend must be specified when multiple QUIC backends are compiled".into(),
            )
        })
    }
}

impl Default for QuicConfig {
    fn default() -> Self {
        let backend = match COMPILED_QUIC_BACKENDS {
            [backend] => Some(*backend),
            _ => None,
        };
        Self { backend }
    }
}

impl BucketSelectionPolicy {
    /// Returns the stable configuration and statistics label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LeastUsed => "least_used",
            Self::MostUsed => "most_used",
        }
    }
}

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
    let mut expanded = if pattern.contains("{thread_id:02}") {
        pattern.replace("{thread_id:02}", &format!("{thread_id:02}"))
    } else {
        pattern.to_owned()
    };
    if expanded.contains("{thread_id}") {
        expanded = expanded.replace("{thread_id}", &thread_id.to_string());
    }
    expanded
}

pub fn bits_for_count(count: usize) -> usize {
    (usize::BITS as usize - count.saturating_sub(1).leading_zeros() as usize).max(1)
}

#[derive(Clone, Debug)]
pub struct Config {
    pub data_path: PathBuf,
    pub segment_size: usize,
    pub blob_segment_size: usize,
    pub max_item_bytes: usize,
    pub segment_count: usize,
    pub table_capacity: usize,
    pub table_target_load_percent: usize,
    pub fingerprint_bits: usize,
    pub unary_count: usize,
    pub front_back_ratio: usize,
    pub bucket_choice_count: usize,
    pub bucket_selection_policy: BucketSelectionPolicy,
    pub sg_index_bits: usize,
    pub fingerprint_hash_offset_bits: usize,
    pub read_max_time_us: u64,
    pub write_max_time_us: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_path: PathBuf::from("target/kvkache-v1/kvkache.data"),
            segment_size: 16 * 1024 * 1024,
            blob_segment_size: 64 * 1024 * 1024,
            max_item_bytes: DEFAULT_MAX_ITEM_BYTES,
            segment_count: 64,
            table_capacity: 10_000_000,
            table_target_load_percent: 88,
            fingerprint_bits: 8,
            unary_count: 32,
            front_back_ratio: 8,
            bucket_choice_count: 2,
            bucket_selection_policy: BucketSelectionPolicy::LeastUsed,
            sg_index_bits: 6,
            fingerprint_hash_offset_bits: 64,
            read_max_time_us: 1_000,
            write_max_time_us: 5_000,
        }
    }
}

impl Config {
    pub(crate) fn blob_path(&self) -> PathBuf {
        self.data_path.with_extension("blob")
    }

    pub(crate) fn checkpoint_path(&self) -> PathBuf {
        self.data_path.with_extension("checkpoint")
    }

    pub(crate) fn next_checkpoint_path(&self) -> PathBuf {
        self.data_path.with_extension("checkpoint.next")
    }

    pub fn validate(&self) -> Result<()> {
        let blob_path = self.blob_path();
        let checkpoint_path = self.checkpoint_path();
        let next_checkpoint_path = self.next_checkpoint_path();
        if blob_path == self.data_path
            || checkpoint_path == self.data_path
            || next_checkpoint_path == self.data_path
            || checkpoint_path == blob_path
            || next_checkpoint_path == blob_path
            || next_checkpoint_path == checkpoint_path
        {
            return Err(KvError::InvalidConfig(
                "derived Blob path and checkpoint paths must be distinct from Segment storage"
                    .into(),
            ));
        }
        if self.sg_index_bits == 0 || self.sg_index_bits > 16 {
            return Err(KvError::InvalidConfig(
                "sg-index-bits must be between 1 and 16".into(),
            ));
        }
        if self.segment_count == 0 || self.segment_count > (1usize << self.sg_index_bits) {
            return Err(KvError::InvalidConfig(format!(
                "segment-count must be in 1..={} for {} SG index bits",
                1usize << self.sg_index_bits,
                self.sg_index_bits
            )));
        }
        if self.segment_size == 0 || !self.segment_size.is_multiple_of(BUCKET_BYTES) {
            return Err(KvError::InvalidConfig(
                "Segment size must be a non-zero multiple of 4096 bytes".into(),
            ));
        }
        if self.segment_size.checked_mul(self.segment_count).is_none() {
            return Err(KvError::InvalidConfig(
                "total Segment data size is too large".into(),
            ));
        }
        self.segment_file_bytes()?;
        if self.blob_segment_size == 0
            || !self.blob_segment_size.is_multiple_of(BUCKET_BYTES)
            || self.blob_segment_size > u32::MAX as usize
        {
            return Err(KvError::InvalidConfig(
                "Blob Segment size must be a 4096-byte multiple no larger than u32::MAX".into(),
            ));
        }
        if self.max_item_bytes == 0
            || self.max_item_bytes > self.blob_segment_size
            || self.max_item_bytes > openkache_protocol::MAX_VALUE_BYTES
        {
            return Err(KvError::InvalidConfig(format!(
                "maximum item size must be between 1 byte and {} bytes",
                self.blob_segment_size
                    .min(openkache_protocol::MAX_VALUE_BYTES)
            )));
        }
        if self
            .blob_segment_size
            .checked_mul(self.segment_count)
            .is_none()
        {
            return Err(KvError::InvalidConfig(
                "total Blob Segment data size is too large".into(),
            ));
        }
        if self.table_capacity == 0 {
            return Err(KvError::InvalidConfig(
                "table-capacity must be non-zero".into(),
            ));
        }
        if !(1..=95).contains(&self.table_target_load_percent) {
            return Err(KvError::InvalidConfig(
                "table-load-percent must be between 1 and 95".into(),
            ));
        }
        if self.fingerprint_bits > 16 {
            return Err(KvError::InvalidConfig(
                "fingerprint-bits must be between 0 and 16".into(),
            ));
        }
        if !(2..=96).contains(&self.unary_count) {
            return Err(KvError::InvalidConfig(
                "unary-count must be between 2 and 96".into(),
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
        if !(1..=32).contains(&self.bucket_choice_count)
            || !self.bucket_choice_count.is_power_of_two()
        {
            return Err(KvError::InvalidConfig(
                "bucket-choice-count must be a power of two between 1 and 32".into(),
            ));
        }
        if self.fingerprint_hash_offset_bits == 0 || self.fingerprint_hash_offset_bits > 64 {
            return Err(KvError::InvalidConfig(
                "fingerprint hash offset must be between 1 and 64 bits".into(),
            ));
        }
        if self.read_max_time_us == 0 || self.write_max_time_us == 0 {
            return Err(KvError::InvalidConfig(
                "read/write timeouts must be non-zero".into(),
            ));
        }
        Ok(())
    }

    pub fn bucket_count(&self) -> usize {
        self.segment_size / BUCKET_BYTES
    }

    pub fn bucket_choice_bits(&self) -> usize {
        self.bucket_choice_count.ilog2() as usize
    }

    pub fn data_bytes(&self) -> u64 {
        (self.segment_size * self.segment_count) as u64
    }

    pub fn segment_file_bytes(&self) -> Result<u64> {
        let stride = self
            .segment_size
            .checked_add(BUCKET_BYTES)
            .ok_or_else(|| KvError::InvalidConfig("Segment file stride is too large".into()))?;
        let bytes = stride
            .checked_mul(self.segment_count)
            .and_then(|bytes| bytes.checked_add(BUCKET_BYTES))
            .ok_or_else(|| KvError::InvalidConfig("Segment file size is too large".into()))?;
        Ok(bytes as u64)
    }

    pub(crate) fn segment_control_offset(&self, sg_index: usize) -> u64 {
        (BUCKET_BYTES + sg_index * (BUCKET_BYTES + self.segment_size)) as u64
    }

    pub(crate) fn segment_data_offset(&self, sg_index: usize) -> u64 {
        self.segment_control_offset(sg_index) + BUCKET_BYTES as u64
    }

    pub fn blob_bytes(&self) -> u64 {
        (self.blob_segment_size * self.segment_count) as u64
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub version: u32,
    pub quic: QuicConfig,
    pub tls: TlsConfig,
    pub network: NetworkConfig,
    pub runtime: RuntimeConfig,
    pub io_uring: IoUringConfig,
    pub timeouts: TimeoutConfig,
    pub storage: StorageConfig,
    pub table: TableConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self::with_cpu_ids(sorted_allowed_cpu_ids())
    }
}

impl AppConfig {
    pub(crate) fn with_cpu_ids(mut cpu_ids: Vec<usize>) -> Self {
        let network_worker_count = default_network_worker_count(cpu_ids.len());
        let storage_cpu_ids = if cpu_ids.len() > network_worker_count {
            cpu_ids.split_off(network_worker_count)
        } else {
            cpu_ids.clone()
        };
        Self {
            version: 1,
            quic: QuicConfig::default(),
            tls: TlsConfig::default(),
            network: NetworkConfig::with_cpu_ids(cpu_ids),
            runtime: RuntimeConfig::with_cpu_ids(storage_cpu_ids),
            io_uring: IoUringConfig::default(),
            timeouts: TimeoutConfig::default(),
            storage: StorageConfig::default(),
            table: TableConfig::default(),
        }
    }
}

/// Server identity, client authentication, and administrative authorization paths.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TlsConfig {
    /// PEM or DER server certificate chain, with the leaf certificate first.
    pub certificate_chain: Option<PathBuf>,
    /// Unencrypted PEM or DER server private key.
    pub private_key: Option<PathBuf>,
    /// PEM or DER CA certificates trusted to authenticate clients.
    pub client_ca: Option<PathBuf>,
    /// Leaf certificates whose authenticated clients may execute administrative commands.
    pub admin_client_certificates: Vec<PathBuf>,
}

impl TlsConfig {
    /// Returns whether any production TLS setting is present.
    pub fn is_configured(&self) -> bool {
        self.certificate_chain.is_some()
            || self.private_key.is_some()
            || self.client_ca.is_some()
            || !self.admin_client_certificates.is_empty()
    }

    fn validate(&self) -> Result<()> {
        if !self.is_configured() {
            return Ok(());
        }
        for (name, path) in [
            ("tls.certificate_chain", &self.certificate_chain),
            ("tls.private_key", &self.private_key),
            ("tls.client_ca", &self.client_ca),
        ] {
            if path.is_none() {
                return Err(KvError::InvalidConfig(format!(
                    "{name} is required when production TLS is configured"
                )));
            }
        }
        if self.admin_client_certificates.is_empty() {
            return Err(KvError::InvalidConfig(
                "tls.admin_client_certificates must contain at least one administrator certificate"
                    .into(),
            ));
        }
        Ok(())
    }
}

fn sorted_allowed_cpu_ids() -> Vec<usize> {
    let mut cpu_ids = allowed_cpu_ids()
        .map(|allowed| allowed.into_iter().collect::<Vec<_>>())
        .unwrap_or_else(|_| vec![0]);
    cpu_ids.sort_unstable();
    cpu_ids
}

/// Preserves the four-CPU 2/2 baseline and reserves four front-end CPUs only
/// when at least eight storage CPUs remain.
pub(crate) const fn default_network_worker_count(cpu_count: usize) -> usize {
    match cpu_count {
        0 => 0,
        1 => 1,
        2..=11 => 2,
        _ => 4,
    }
}

/// Front-end network workers, each owning one socket and protocol connection set.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    pub worker_count: usize,
    pub cpu_ids: Vec<usize>,
    pub event_interval: usize,
    pub io_uring_entries_per_worker: u32,
    pub max_stream_lanes_per_connection: usize,
    /// Aggregate request/GET-response bytes admitted concurrently by each network worker.
    pub max_inflight_value_mib_per_worker: usize,
    pub sqpoll: bool,
    pub napi_busy_poll: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        let mut cpu_ids = sorted_allowed_cpu_ids();
        cpu_ids.truncate(default_network_worker_count(cpu_ids.len()));
        Self::with_cpu_ids(cpu_ids)
    }
}

impl NetworkConfig {
    fn with_cpu_ids(cpu_ids: Vec<usize>) -> Self {
        let worker_count = cpu_ids.len();
        Self {
            worker_count,
            cpu_ids,
            event_interval: 31,
            io_uring_entries_per_worker: 4_096,
            max_stream_lanes_per_connection: 256,
            max_inflight_value_mib_per_worker: 256,
            sqpoll: false,
            napi_busy_poll: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    pub thread_count: usize,
    pub cpu_ids: Vec<usize>,
    pub event_interval: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self::with_cpu_ids(sorted_allowed_cpu_ids())
    }
}

impl RuntimeConfig {
    fn with_cpu_ids(cpu_ids: Vec<usize>) -> Self {
        let thread_count = cpu_ids.len();
        Self {
            thread_count,
            cpu_ids,
            event_interval: 31,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct IoUringConfig {
    pub sqpoll: bool,
    pub iopoll: bool,
    pub entries_per_worker: u32,
    pub max_inflight_per_worker: usize,
    pub batch_size: usize,
    pub batch_max_wait_us: u64,
}

impl Default for IoUringConfig {
    fn default() -> Self {
        Self {
            sqpoll: false,
            iopoll: false,
            entries_per_worker: 256,
            max_inflight_per_worker: 8,
            batch_size: 16,
            batch_max_wait_us: 10,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    pub directory: PathBuf,
    pub data_file_pattern: String,
    pub segments_per_thread: usize,
    pub segment_size_mib: usize,
    pub blob_segment_size_mib: usize,
    /// Maximum encoded cache-item size accepted by the server.
    pub max_item_size_mib: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("target/kvkache-v1"),
            data_file_pattern: "data-{thread_id:02}.sg".into(),
            segments_per_thread: 4,
            segment_size_mib: 16,
            blob_segment_size_mib: 64,
            max_item_size_mib: DEFAULT_MAX_ITEM_BYTES / (1024 * 1024),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TableConfig {
    pub capacity_per_thread: usize,
    pub target_load_percent: usize,
    pub fingerprint_bits: usize,
    pub unary_count: usize,
    pub front_back_ratio: usize,
    pub bucket_choice_count: usize,
    pub bucket_selection_policy: BucketSelectionPolicy,
    pub fingerprint_hash_offset_bits: usize,
}

impl Default for TableConfig {
    fn default() -> Self {
        Self {
            capacity_per_thread: 625_000,
            target_load_percent: 88,
            fingerprint_bits: 8,
            unary_count: 32,
            front_back_ratio: 8,
            bucket_choice_count: 2,
            bucket_selection_policy: BucketSelectionPolicy::LeastUsed,
            fingerprint_hash_offset_bits: 64,
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
        self.quic.selected_backend()?;
        self.tls.validate()?;
        if self.network.worker_count == 0 {
            return Err(KvError::InvalidConfig(
                "network.worker_count must be non-zero".into(),
            ));
        }
        if self.network.cpu_ids.len() != self.network.worker_count {
            return Err(KvError::InvalidConfig(
                "network.cpu_ids length must equal network.worker_count".into(),
            ));
        }
        let unique = self.network.cpu_ids.iter().copied().collect::<HashSet<_>>();
        if unique.len() != self.network.cpu_ids.len() {
            return Err(KvError::InvalidConfig(
                "network.cpu_ids must not contain duplicates".into(),
            ));
        }
        if self.network.event_interval == 0
            || self.network.io_uring_entries_per_worker == 0
            || self.network.max_stream_lanes_per_connection == 0
            || self.network.max_stream_lanes_per_connection > u32::MAX as usize
            || self.network.max_inflight_value_mib_per_worker == 0
            || self
                .network
                .max_inflight_value_mib_per_worker
                .checked_mul(1024 * 1024)
                .is_none()
        {
            return Err(KvError::InvalidConfig(
                "network event interval, io_uring entries, stream lane limit, and in-flight value budget must be non-zero"
                    .into(),
            ));
        }
        if self.network.sqpoll || self.network.napi_busy_poll {
            return Err(KvError::InvalidConfig(
                "network SQPOLL and NAPI busy poll must remain false".into(),
            ));
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
            .network
            .cpu_ids
            .iter()
            .find(|cpu| !allowed.contains(cpu))
        {
            return Err(KvError::InvalidConfig(format!(
                "network CPU {cpu} is not in the process affinity set {allowed:?}"
            )));
        }
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
        if self.io_uring.sqpoll || self.io_uring.iopoll {
            return Err(KvError::InvalidConfig(
                "io_uring SQPOLL and IOPOLL must remain false".into(),
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
        if self.storage.segments_per_thread == 0
            || self.storage.segments_per_thread > (1usize << 16)
        {
            return Err(KvError::InvalidConfig(
                "storage.segments_per_thread must be between 1 and 65536".into(),
            ));
        }
        if self.storage.segment_size_mib == 0
            || self
                .storage
                .segment_size_mib
                .checked_mul(1024 * 1024)
                .is_none()
        {
            return Err(KvError::InvalidConfig(
                "storage.segment_size_mib is invalid".into(),
            ));
        }
        if self.storage.blob_segment_size_mib == 0
            || self
                .storage
                .blob_segment_size_mib
                .checked_mul(1024 * 1024)
                .is_none_or(|bytes| bytes > u32::MAX as usize)
        {
            return Err(KvError::InvalidConfig(
                "storage.blob_segment_size_mib is invalid".into(),
            ));
        }
        if self.storage.max_item_size_mib == 0
            || self.storage.max_item_size_mib > self.storage.blob_segment_size_mib
            || self
                .storage
                .max_item_size_mib
                .checked_mul(1024 * 1024)
                .is_none_or(|bytes| bytes > openkache_protocol::MAX_VALUE_BYTES)
        {
            return Err(KvError::InvalidConfig(format!(
                "storage.max_item_size_mib must be between 1 and {}",
                self.storage
                    .blob_segment_size_mib
                    .min(openkache_protocol::MAX_VALUE_BYTES / (1024 * 1024))
            )));
        }
        if self.network.max_inflight_value_mib_per_worker < self.storage.max_item_size_mib {
            return Err(KvError::InvalidConfig(
                "network.max_inflight_value_mib_per_worker must be at least storage.max_item_size_mib"
                    .into(),
            ));
        }
        let data_names = (0..self.runtime.thread_count)
            .map(|thread_id| expand_thread_pattern(&self.storage.data_file_pattern, thread_id))
            .collect::<HashSet<_>>();
        if data_names.len() != self.runtime.thread_count {
            return Err(KvError::InvalidConfig(
                "storage.data_file_pattern must expand uniquely for every thread".into(),
            ));
        }
        self.worker_config(0).validate()
    }

    pub fn for_trace_benchmark(
        directory: PathBuf,
        cpu_ids: Vec<usize>,
        total_segment_count: usize,
        total_table_capacity: usize,
    ) -> Result<Self> {
        if cpu_ids.is_empty() || !total_segment_count.is_multiple_of(cpu_ids.len()) {
            return Err(KvError::InvalidConfig(
                "trace benchmark total Segment count must divide evenly across workers".into(),
            ));
        }
        let thread_count = cpu_ids.len();
        let segments_per_thread = total_segment_count / thread_count;
        let capacity_per_thread = total_table_capacity.div_ceil(thread_count);
        let config = Self {
            version: 1,
            quic: QuicConfig::default(),
            tls: TlsConfig::default(),
            network: NetworkConfig::default(),
            runtime: RuntimeConfig {
                thread_count,
                cpu_ids,
                event_interval: 31,
            },
            io_uring: IoUringConfig {
                entries_per_worker: 256,
                max_inflight_per_worker: 8,
                batch_size: 16,
                ..IoUringConfig::default()
            },
            timeouts: TimeoutConfig {
                input_max_time_us: 30_000_000,
                output_max_time_us: 30_000_000,
                read_max_time_us: 5_000_000,
                write_max_time_us: 30_000_000,
                request_max_time_us: 30_000_000,
            },
            storage: StorageConfig {
                directory,
                data_file_pattern: "data-{thread_id:02}.sg".into(),
                segments_per_thread,
                segment_size_mib: 16,
                blob_segment_size_mib: 64,
                max_item_size_mib: 16,
            },
            table: TableConfig {
                capacity_per_thread,
                ..TableConfig::default()
            },
        };
        config.validate()?;
        Ok(config)
    }

    pub fn worker_config(&self, thread_id: usize) -> Config {
        let data_name = expand_thread_pattern(&self.storage.data_file_pattern, thread_id);
        Config {
            data_path: self.storage.directory.join(data_name),
            segment_size: self.storage.segment_size_mib * 1024 * 1024,
            blob_segment_size: self.storage.blob_segment_size_mib * 1024 * 1024,
            max_item_bytes: self.storage.max_item_size_mib * 1024 * 1024,
            segment_count: self.storage.segments_per_thread,
            table_capacity: self.table.capacity_per_thread,
            table_target_load_percent: self.table.target_load_percent,
            fingerprint_bits: self.table.fingerprint_bits,
            unary_count: self.table.unary_count,
            front_back_ratio: self.table.front_back_ratio,
            bucket_choice_count: self.table.bucket_choice_count,
            bucket_selection_policy: self.table.bucket_selection_policy,
            sg_index_bits: bits_for_count(self.storage.segments_per_thread),
            fingerprint_hash_offset_bits: self.table.fingerprint_hash_offset_bits,
            read_max_time_us: self.timeouts.read_max_time_us,
            write_max_time_us: self.timeouts.write_max_time_us,
        }
    }
}
