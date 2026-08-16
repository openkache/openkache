//! Application and per-worker configuration for the OpenKache server.
//!
//! Storage uses fixed 4 KiB Buckets inside Segments. Each worker owns paired
//! Segment and Blob files plus one in-memory lookup Table. The Segment file
//! embeds recovery control pages used to rebuild committed state at startup.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::BUCKET_BYTES;
use crate::error::{KvError, Result};
use crate::platform::allowed_cpu_ids;
use crate::store::BUCKET_READ_POOL_CAPACITY;

pub(crate) const DEFAULT_BUCKET_CHOICE_COUNT: usize = 4;
pub(crate) const MAX_BUCKET_READ_POOL_CAPACITY: usize = 4_096;

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
    pub large_value_path: PathBuf,
    pub segment_size: usize,
    pub blob_segment_size: usize,
    pub large_value_capacity: usize,
    pub mutable_segment_count: usize,
    pub max_flushes_in_flight: usize,
    pub stable_ram_segment_count: usize,
    pub bucket_read_pool_capacity: usize,
    pub copy_ssd_inline_value_once: bool,
    pub lease_ssd_read_buffer: bool,
    pub large_value_threshold: usize,
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
            large_value_path: PathBuf::from("target/kvkache-v1/kvkache.large"),
            segment_size: 16 * 1024 * 1024,
            blob_segment_size: 64 * 1024 * 1024,
            large_value_capacity: 64 * 1024 * 1024,
            mutable_segment_count: 3,
            max_flushes_in_flight: 2,
            stable_ram_segment_count: 2,
            bucket_read_pool_capacity: BUCKET_READ_POOL_CAPACITY,
            copy_ssd_inline_value_once: true,
            lease_ssd_read_buffer: false,
            large_value_threshold: 20 * 1024,
            max_item_bytes: DEFAULT_MAX_ITEM_BYTES,
            segment_count: 64,
            table_capacity: 10_000_000,
            table_target_load_percent: 88,
            fingerprint_bits: 8,
            unary_count: 32,
            front_back_ratio: 8,
            bucket_choice_count: DEFAULT_BUCKET_CHOICE_COUNT,
            bucket_selection_policy: BucketSelectionPolicy::LeastUsed,
            sg_index_bits: 9,
            fingerprint_hash_offset_bits: 64,
            read_max_time_us: 1_000,
            write_max_time_us: 5_000,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if !(1..=32).contains(&self.bucket_choice_count)
            || !self.bucket_choice_count.is_power_of_two()
        {
            return Err(KvError::InvalidConfig(
                "bucket-choice-count must be a power of two between 1 and 32".into(),
            ));
        }
        if self.sg_index_bits == 0
            || self
                .sg_index_bits
                .checked_add(self.bucket_choice_bits())
                .is_none_or(|bits| bits > u32::BITS as usize)
        {
            return Err(KvError::InvalidConfig(
                "SG index and Bucket-choice fields must fit in 32 bits".into(),
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
        if self.mutable_segment_count != 3 {
            return Err(KvError::InvalidConfig(
                "mutable Segment count must be 3".into(),
            ));
        }
        if self.max_flushes_in_flight != 2 {
            return Err(KvError::InvalidConfig(
                "maximum flushes in flight must be 2".into(),
            ));
        }
        if self.stable_ram_segment_count > self.segment_count {
            return Err(KvError::InvalidConfig(
                "stable RAM Segment count cannot exceed physical Segment count".into(),
            ));
        }
        if self.bucket_read_pool_capacity > MAX_BUCKET_READ_POOL_CAPACITY {
            return Err(KvError::InvalidConfig(format!(
                "Bucket-read pool capacity cannot exceed {MAX_BUCKET_READ_POOL_CAPACITY} buffers"
            )));
        }
        if self.large_value_threshold == 0 {
            return Err(KvError::InvalidConfig(
                "large-value threshold must be non-zero".into(),
            ));
        }
        if self.large_value_capacity == 0
            || !self.large_value_capacity.is_multiple_of(BUCKET_BYTES)
            || self.large_value_capacity < self.max_item_bytes
        {
            return Err(KvError::InvalidConfig(
                "large-value capacity must be a 4096-byte multiple at least as large as one maximum Item"
                    .into(),
            ));
        }
        if self.segment_size.checked_mul(self.segment_count).is_none() {
            return Err(KvError::InvalidConfig(
                "total Segment data size is too large".into(),
            ));
        }
        self.generation_file_bytes()?;
        if self.blob_segment_size == 0
            || !self.blob_segment_size.is_multiple_of(BUCKET_BYTES)
            || self.blob_segment_size > u32::MAX as usize
        {
            return Err(KvError::InvalidConfig(
                "Blob Segment size must be a 4096-byte multiple no larger than u32::MAX".into(),
            ));
        }
        if self.max_item_bytes == 0
            || self.max_item_bytes > self.large_value_capacity
            || self.max_item_bytes > openkache_protocol::MAX_VALUE_BYTES
        {
            return Err(KvError::InvalidConfig(format!(
                "maximum item size must be between 1 byte and {} bytes",
                self.large_value_capacity
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
        let logical_sg_capacity = self.logical_sg_capacity()?;
        if logical_sg_capacity > (1usize << self.sg_index_bits) {
            return Err(KvError::InvalidConfig(format!(
                "logical SG capacity {logical_sg_capacity} exceeds {} configured SG-index bits",
                self.sg_index_bits
            )));
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

    pub fn generation_file_bytes(&self) -> Result<u64> {
        self.segment_size
            .checked_add(self.blob_segment_size)
            .and_then(|bytes| bytes.checked_mul(self.segment_count))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or_else(|| KvError::InvalidConfig("generation file size is too large".into()))
    }

    pub fn logical_sg_capacity(&self) -> Result<usize> {
        let physical = usize::try_from(self.generation_file_bytes()?)
            .map_err(|_| KvError::InvalidConfig("generation file does not fit usize".into()))?
            / self.segment_size;
        physical
            .checked_add(self.mutable_segment_count)
            .and_then(|count| count.checked_add(self.max_flushes_in_flight))
            .ok_or_else(|| KvError::InvalidConfig("logical SG capacity overflowed".into()))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub version: u32,
    pub quic: QuicConfig,
    pub tls: TlsConfig,
    pub network: NetworkConfig,
    pub observability: ObservabilityConfig,
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
            observability: ObservabilityConfig::default(),
            runtime: RuntimeConfig::with_cpu_ids(storage_cpu_ids),
            io_uring: IoUringConfig::default(),
            timeouts: TimeoutConfig::default(),
            storage: StorageConfig::default(),
            table: TableConfig::default(),
        }
    }
}

/// Low-cardinality management endpoint and request-latency settings.
///
/// The endpoint is disabled by default. When enabled, it serves `/metrics`,
/// `/livez`, and `/readyz` on a separate standard-library TCP listener so
/// scrapes cannot consume data-plane worker capacity.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ObservabilityConfig {
    /// Local address for the Prometheus-compatible management endpoint.
    pub metrics_listen: Option<SocketAddr>,
    /// Allow binding the management endpoint to a non-loopback address.
    pub metrics_allow_remote: bool,
    /// Requests at or above this duration emit a structured warning.
    pub slow_request_us: u64,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            metrics_listen: None,
            metrics_allow_remote: false,
            slow_request_us: 10_000,
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
    /// Aggregate request/GET-response bytes admitted concurrently across the server.
    pub max_inflight_value_mib: usize,
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
            max_inflight_value_mib: 256,
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
    /// Delay before each simulated storage read or write completes.
    pub simulated_io_latency_us: u64,
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
            simulated_io_latency_us: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct IoUringConfig {
    pub sqpoll: bool,
    /// Optional worker-indexed CPU affinity for storage SQPOLL kernel threads.
    pub sqpoll_cpu_ids: Vec<usize>,
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
            sqpoll_cpu_ids: Vec::new(),
            iopoll: false,
            entries_per_worker: 256,
            max_inflight_per_worker: 64,
            batch_size: 64,
            batch_max_wait_us: 10,
        }
    }
}

impl IoUringConfig {
    pub(crate) fn waiting_capacity(&self) -> Result<usize> {
        let capacity = self
            .batch_size
            .checked_mul(self.max_inflight_per_worker)
            .ok_or_else(|| {
                KvError::InvalidConfig(
                    "io_uring.batch_size * max_inflight_per_worker exceeds usize".into(),
                )
            })?;
        u32::try_from(capacity).map_err(|_| {
            KvError::InvalidConfig(
                "io_uring worker waiting capacity exceeds the supported slot index range".into(),
            )
        })?;
        Ok(capacity)
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
    pub large_value_file_pattern: String,
    pub segments_per_thread: usize,
    pub segment_size_mib: usize,
    pub blob_segment_size_mib: usize,
    /// Directly writable RAM SGs retained by each storage worker.
    pub mutable_segments_per_thread: usize,
    /// Concurrent generation flushes admitted by each storage worker.
    pub max_flushes_in_flight_per_thread: usize,
    /// Recently flushed read-only SGs retained in RAM by each storage worker.
    pub stable_ram_segments_per_thread: usize,
    /// Fixed 4 KiB direct-read buffers preallocated by each storage worker.
    pub bucket_read_pool_capacity_per_thread: usize,
    /// Copy inline values out of direct-read buffers before completing a read.
    pub copy_ssd_inline_value_once: bool,
    /// Retain pooled direct-read buffers in completed values instead of copying.
    pub lease_ssd_read_buffer: bool,
    /// Values larger than this threshold use the large-value tier.
    pub large_value_threshold_kib: usize,
    /// Preallocated worker-local large-value circular file capacity.
    pub large_value_capacity_mib_per_thread: usize,
    /// Maximum encoded cache-item size accepted by the server.
    pub max_item_size_mib: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("target/kvkache-v1"),
            data_file_pattern: "worker-{thread_id:02}.data".into(),
            large_value_file_pattern: "worker-{thread_id:02}.large".into(),
            segments_per_thread: 4,
            segment_size_mib: 16,
            blob_segment_size_mib: 64,
            mutable_segments_per_thread: 3,
            max_flushes_in_flight_per_thread: 2,
            stable_ram_segments_per_thread: 2,
            bucket_read_pool_capacity_per_thread: BUCKET_READ_POOL_CAPACITY,
            copy_ssd_inline_value_once: true,
            lease_ssd_read_buffer: false,
            large_value_threshold_kib: 20,
            large_value_capacity_mib_per_thread: 64,
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
            bucket_choice_count: DEFAULT_BUCKET_CHOICE_COUNT,
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
            || self.network.max_inflight_value_mib == 0
            || self
                .network
                .max_inflight_value_mib
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
        if self.observability.slow_request_us == 0 {
            return Err(KvError::InvalidConfig(
                "observability.slow_request_us must be non-zero".into(),
            ));
        }
        if let Some(address) = self.observability.metrics_listen
            && !self.observability.metrics_allow_remote
            && !address.ip().is_loopback()
        {
            return Err(KvError::InvalidConfig(
                "observability.metrics_listen must be loopback unless observability.metrics_allow_remote is true"
                    .into(),
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
        #[cfg(not(feature = "storage-runtime-simulated"))]
        if self.runtime.simulated_io_latency_us != 0 {
            return Err(KvError::InvalidConfig(
                "runtime.simulated_io_latency_us requires storage-runtime-simulated".into(),
            ));
        }
        #[cfg(feature = "storage-runtime-simulated")]
        if self.io_uring.sqpoll {
            return Err(KvError::InvalidConfig(
                "simulated storage workers do not support io_uring SQPOLL".into(),
            ));
        }
        if self.io_uring.iopoll {
            return Err(KvError::InvalidConfig(
                "io_uring IOPOLL must remain false".into(),
            ));
        }
        if self.io_uring.sqpoll {
            if !self.io_uring.sqpoll_cpu_ids.is_empty()
                && self.io_uring.sqpoll_cpu_ids.len() != self.runtime.thread_count
            {
                return Err(KvError::InvalidConfig(
                    "io_uring.sqpoll_cpu_ids length must equal runtime.thread_count when provided"
                        .into(),
                ));
            }
            if let Some(cpu) = self
                .io_uring
                .sqpoll_cpu_ids
                .iter()
                .find(|cpu| !allowed.contains(cpu) || u32::try_from(**cpu).is_err())
            {
                return Err(KvError::InvalidConfig(format!(
                    "SQPOLL CPU {cpu} is not an allowed io_uring CPU"
                )));
            }
        } else if !self.io_uring.sqpoll_cpu_ids.is_empty() {
            return Err(KvError::InvalidConfig(
                "io_uring.sqpoll_cpu_ids requires io_uring.sqpoll=true".into(),
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
        self.io_uring.waiting_capacity()?;
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
        if self.storage.mutable_segments_per_thread != 3
            || self.storage.max_flushes_in_flight_per_thread != 2
            || self.storage.large_value_threshold_kib == 0
            || self.storage.large_value_capacity_mib_per_thread == 0
        {
            return Err(KvError::InvalidConfig(
                "storage requires 3 mutable SGs, 2 flushes in flight, and non-zero large-value threshold and capacity"
                    .into(),
            ));
        }
        if self.storage.bucket_read_pool_capacity_per_thread > MAX_BUCKET_READ_POOL_CAPACITY {
            return Err(KvError::InvalidConfig(format!(
                "storage.bucket_read_pool_capacity_per_thread cannot exceed {MAX_BUCKET_READ_POOL_CAPACITY} buffers"
            )));
        }
        if self.storage.max_item_size_mib == 0
            || self.storage.max_item_size_mib > self.storage.large_value_capacity_mib_per_thread
            || self
                .storage
                .max_item_size_mib
                .checked_mul(1024 * 1024)
                .is_none_or(|bytes| bytes > openkache_protocol::MAX_VALUE_BYTES)
        {
            return Err(KvError::InvalidConfig(format!(
                "storage.max_item_size_mib must be between 1 and {}",
                self.storage
                    .large_value_capacity_mib_per_thread
                    .min(openkache_protocol::MAX_VALUE_BYTES / (1024 * 1024))
            )));
        }
        if self.network.max_inflight_value_mib < self.storage.max_item_size_mib {
            return Err(KvError::InvalidConfig(
                "network.max_inflight_value_mib must be at least storage.max_item_size_mib".into(),
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
        let large_value_names = (0..self.runtime.thread_count)
            .map(|thread_id| {
                expand_thread_pattern(&self.storage.large_value_file_pattern, thread_id)
            })
            .collect::<HashSet<_>>();
        if large_value_names.len() != self.runtime.thread_count {
            return Err(KvError::InvalidConfig(
                "storage.large_value_file_pattern must expand uniquely for every thread".into(),
            ));
        }
        self.worker_config(0).validate()
    }

    pub fn worker_config(&self, thread_id: usize) -> Config {
        let data_name = expand_thread_pattern(&self.storage.data_file_pattern, thread_id);
        let large_value_name =
            expand_thread_pattern(&self.storage.large_value_file_pattern, thread_id);
        let segment_size = self.storage.segment_size_mib * 1024 * 1024;
        let blob_segment_size = self.storage.blob_segment_size_mib * 1024 * 1024;
        let logical_sg_capacity = self
            .storage
            .segments_per_thread
            .saturating_mul(segment_size.saturating_add(blob_segment_size))
            .checked_div(segment_size)
            .unwrap_or_default()
            .saturating_add(self.storage.mutable_segments_per_thread)
            .saturating_add(self.storage.max_flushes_in_flight_per_thread);
        Config {
            data_path: self.storage.directory.join(data_name),
            large_value_path: self.storage.directory.join(large_value_name),
            segment_size,
            blob_segment_size,
            large_value_capacity: self.storage.large_value_capacity_mib_per_thread * 1024 * 1024,
            mutable_segment_count: self.storage.mutable_segments_per_thread,
            max_flushes_in_flight: self.storage.max_flushes_in_flight_per_thread,
            stable_ram_segment_count: self.storage.stable_ram_segments_per_thread,
            bucket_read_pool_capacity: self.storage.bucket_read_pool_capacity_per_thread,
            copy_ssd_inline_value_once: self.storage.copy_ssd_inline_value_once,
            lease_ssd_read_buffer: self.storage.lease_ssd_read_buffer,
            large_value_threshold: self.storage.large_value_threshold_kib * 1024,
            max_item_bytes: self.storage.max_item_size_mib * 1024 * 1024,
            segment_count: self.storage.segments_per_thread,
            table_capacity: self.table.capacity_per_thread,
            table_target_load_percent: self.table.target_load_percent,
            fingerprint_bits: self.table.fingerprint_bits,
            unary_count: self.table.unary_count,
            front_back_ratio: self.table.front_back_ratio,
            bucket_choice_count: self.table.bucket_choice_count,
            bucket_selection_policy: self.table.bucket_selection_policy,
            sg_index_bits: bits_for_count(logical_sg_capacity),
            fingerprint_hash_offset_bits: self.table.fingerprint_hash_offset_bits,
            read_max_time_us: self.timeouts.read_max_time_us,
            write_max_time_us: self.timeouts.write_max_time_us,
        }
    }
}
