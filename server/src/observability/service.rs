//! Low-overhead server metrics and a local Prometheus-compatible exporter.
//!
//! The data plane records fixed-cardinality counters and histograms.  Exporting
//! those values happens on a dedicated management listener so a scrape never
//! queues a request on a storage worker or waits for a persistence barrier.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use openkache_protocol::{Opcode, Status};

use super::http::{MetricsEndpoint, MetricsEndpointHandle};
use super::lifecycle::{Lifecycle, LifecycleCell};
use super::shard::{NetworkShard, NetworkWorkerId, StorageShard, StorageWorkerId};

pub(super) const HISTOGRAM_BUCKETS_US: [u64; 12] = [
    1_000, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000, 2_000_000,
    5_000_000, 10_000_000,
];

const fn operation_names() -> [&'static str; Opcode::COUNT + 1] {
    let mut names = ["unknown"; Opcode::COUNT + 1];
    let mut index = 0;
    while index < Opcode::COUNT {
        names[index] = Opcode::NAMES[index];
        index += 1;
    }
    names
}

pub(super) const OPERATION_NAMES: [&str; Opcode::COUNT + 1] = operation_names();

pub(super) const STATUS_NAMES: [&str; Status::COUNT] = Status::NAMES;

/// An operation metric key plus the transport-only unknown bucket used by
/// adapters that cannot resolve a modeled opcode.
///
/// The key wraps the generated `Opcode` instead of duplicating its variants.
/// Metrics arrays are indexed by `Opcode::index()`, so sparse wire assignments
/// and Smithy enum reordering cannot silently corrupt telemetry. Built-in
/// command names are deliberately not part of this generic observability
/// primitive; protocol adapters resolve their own command vocabulary before
/// creating a key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Operation(Option<Opcode>);

impl Operation {
    const COUNT: usize = OPERATION_NAMES.len();

    pub(crate) const fn unknown() -> Self {
        Self(None)
    }

    pub(crate) const fn name(self) -> &'static str {
        OPERATION_NAMES[self.index()]
    }

    pub(crate) const fn index(self) -> usize {
        match self.0 {
            Some(opcode) => opcode.index(),
            None => Opcode::COUNT,
        }
    }

    pub(crate) const fn from_opcode(opcode: Opcode) -> Self {
        Self(Some(opcode))
    }

    pub(crate) const fn storage_get() -> Self {
        Self::from_opcode(Opcode::Get)
    }

    pub(crate) const fn storage_set() -> Self {
        Self::from_opcode(Opcode::Set)
    }

    pub(crate) const fn storage_delete() -> Self {
        Self::from_opcode(Opcode::Delete)
    }
}

const fn status_index(status: Status) -> usize {
    status.index()
}

pub(super) struct Histogram {
    pub(super) buckets: [AtomicU64; HISTOGRAM_BUCKETS_US.len()],
    pub(super) count: AtomicU64,
    pub(super) sum_ns: AtomicU64,
}

impl Histogram {
    fn new() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            count: AtomicU64::new(0),
            sum_ns: AtomicU64::new(0),
        }
    }

    fn observe(&self, elapsed: Duration) {
        let micros = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
        if let Some(index) = HISTOGRAM_BUCKETS_US
            .iter()
            .position(|boundary| micros <= *boundary)
        {
            self.buckets[index].fetch_add(1, Ordering::Relaxed);
        }
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum_ns.fetch_add(
            elapsed.as_nanos().min(u128::from(u64::MAX)) as u64,
            Ordering::Relaxed,
        );
    }

    fn snapshot(&self) -> HistogramSnapshot {
        HistogramSnapshot {
            buckets: self
                .buckets
                .iter()
                .map(|bucket| bucket.load(Ordering::Relaxed))
                .collect::<Vec<_>>()
                .try_into()
                .expect("histogram bucket count is fixed"),
            count: self.count.load(Ordering::Relaxed),
            sum_ns: self.sum_ns.load(Ordering::Relaxed),
        }
    }
}

struct RequestMetrics {
    totals: [[AtomicU64; STATUS_NAMES.len()]; Operation::COUNT],
    durations: [Histogram; Operation::COUNT],
}

#[derive(Clone, Debug)]
pub(super) struct HistogramSnapshot {
    pub(super) buckets: [u64; HISTOGRAM_BUCKETS_US.len()],
    pub(super) count: u64,
    pub(super) sum_ns: u64,
}

#[derive(Clone, Debug)]
pub(super) struct NetworkWorkerSnapshot {
    pub(super) lifecycle: Lifecycle,
    pub(super) request_totals: [[u64; STATUS_NAMES.len()]; Operation::COUNT],
    pub(super) request_durations: [HistogramSnapshot; Operation::COUNT],
    pub(super) storage_queue_full_total: Vec<u64>,
    pub(super) storage_wait: Vec<[HistogramSnapshot; Operation::COUNT]>,
    pub(super) active_connections: u64,
    pub(super) active_streams: u64,
    pub(super) handshakes_total: u64,
    pub(super) handshake_failures_total: u64,
    pub(super) protocol_errors_total: u64,
    pub(super) request_read_timeouts_total: u64,
    pub(super) response_write_failures_total: u64,
    pub(super) abandoned_requests_total: u64,
    pub(super) slow_requests_total: u64,
}

#[derive(Clone, Debug)]
pub(super) struct StorageWorkerSnapshot {
    pub(super) lifecycle: Lifecycle,
    pub(super) cpu_id: u64,
    pub(super) operations_total: [u64; Operation::COUNT],
    pub(super) durations: [HistogramSnapshot; Operation::COUNT],
}

/// A management-plane copy of all worker-local telemetry.
///
/// Exporters consume this immutable value instead of reaching into atomics.
/// Taking a snapshot is intentionally a scrape/export operation; no data-plane
/// worker performs this allocation or aggregation.
#[derive(Clone, Debug)]
pub(super) struct TelemetrySnapshot {
    pub(super) uptime_seconds: f64,
    pub(super) lifecycle: Lifecycle,
    pub(super) network: Vec<NetworkWorkerSnapshot>,
    pub(super) storage: Vec<StorageWorkerSnapshot>,
}

impl RequestMetrics {
    fn new() -> Self {
        Self {
            totals: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            durations: std::array::from_fn(|_| Histogram::new()),
        }
    }
}

/// Storage metrics are owned by one storage worker. The alignment keeps two
/// workers from sharing a cache line when the management thread takes a
/// snapshot.
#[repr(align(64))]
struct StorageWorkerMetrics {
    lifecycle: LifecycleCell,
    cpu_id: AtomicU64,
    operations_total: [AtomicU64; Operation::COUNT],
    durations: [Histogram; Operation::COUNT],
}

impl StorageWorkerMetrics {
    fn new() -> Self {
        Self {
            lifecycle: LifecycleCell::new(),
            cpu_id: AtomicU64::new(0),
            operations_total: std::array::from_fn(|_| AtomicU64::new(0)),
            durations: std::array::from_fn(|_| Histogram::new()),
        }
    }
}

/// All hot-path metrics for one network worker.
///
/// A network worker is the sole writer for its shard. Storage queue and wait
/// metrics are intentionally kept here as well: the caller owns those writes,
/// while the management thread aggregates them by target storage worker.
#[repr(align(64))]
struct NetworkWorkerMetrics {
    lifecycle: LifecycleCell,
    request: RequestMetrics,
    storage_queue_full_total: Vec<AtomicU64>,
    storage_wait: Vec<[Histogram; Operation::COUNT]>,
    active_connections: AtomicU64,
    active_streams: AtomicU64,
    handshakes_total: AtomicU64,
    handshake_failures_total: AtomicU64,
    protocol_errors_total: AtomicU64,
    request_read_timeouts_total: AtomicU64,
    response_write_failures_total: AtomicU64,
    abandoned_requests_total: AtomicU64,
    slow_requests_total: AtomicU64,
}

impl NetworkWorkerMetrics {
    fn new(storage_worker_count: usize) -> Self {
        Self {
            lifecycle: LifecycleCell::new(),
            request: RequestMetrics::new(),
            storage_queue_full_total: (0..storage_worker_count)
                .map(|_| AtomicU64::new(0))
                .collect(),
            storage_wait: (0..storage_worker_count)
                .map(|_| std::array::from_fn(|_| Histogram::new()))
                .collect(),
            active_connections: AtomicU64::new(0),
            active_streams: AtomicU64::new(0),
            handshakes_total: AtomicU64::new(0),
            handshake_failures_total: AtomicU64::new(0),
            protocol_errors_total: AtomicU64::new(0),
            request_read_timeouts_total: AtomicU64::new(0),
            response_write_failures_total: AtomicU64::new(0),
            abandoned_requests_total: AtomicU64::new(0),
            slow_requests_total: AtomicU64::new(0),
        }
    }
}

/// Lock-free worker-local telemetry and a scrape-time lifecycle reducer.
///
/// The data plane never writes a process-wide counter. Every request and
/// connection update is directed to the network worker shard that owns it;
/// storage execution updates are directed to the storage worker shard. The
/// management listener reads those shards and reduces them into one bounded
/// snapshot for Prometheus, health probes, and authenticated `STATS`.
pub(crate) struct ObservabilityState {
    started_at: Instant,
    enabled: bool,
    network: Vec<NetworkWorkerMetrics>,
    storage: Vec<StorageWorkerMetrics>,
    slow_request_threshold: Duration,
}

/// Common management-plane ownership shared by QUIC and RESP servers.
///
/// The state is created once from topology/configuration. Transport-specific
/// servers borrow its `Arc` for worker startup, while this service owns the
/// optional management listener until serving begins.
pub(crate) struct ObservabilityService {
    state: std::sync::Arc<ObservabilityState>,
    endpoint: Option<MetricsEndpoint>,
    #[cfg(feature = "opentelemetry")]
    otlp: Option<super::otlp::OtlpMetrics>,
}

impl ObservabilityService {
    pub(crate) fn new(
        network_worker_count: usize,
        storage_worker_count: usize,
        config: &crate::config::ObservabilityConfig,
    ) -> std::io::Result<Self> {
        let state = std::sync::Arc::new(ObservabilityState::new_with_workers(
            network_worker_count,
            storage_worker_count,
            Duration::from_micros(config.slow_request_us),
            telemetry_enabled(config),
        ));
        let endpoint = MetricsEndpoint::bind(
            config.metrics_listen,
            config.metrics_allow_remote,
            std::sync::Arc::clone(&state),
        )?;
        #[cfg(feature = "opentelemetry")]
        let otlp = super::otlp::OtlpMetrics::from_environment(std::sync::Arc::clone(&state));
        Ok(Self {
            state,
            endpoint,
            #[cfg(feature = "opentelemetry")]
            otlp,
        })
    }

    pub(crate) fn state(&self) -> std::sync::Arc<ObservabilityState> {
        std::sync::Arc::clone(&self.state)
    }

    pub(crate) fn metrics_addr(&self) -> Option<SocketAddr> {
        self.endpoint.as_ref().map(MetricsEndpoint::local_addr)
    }

    pub(crate) fn start(self) -> ObservabilityHandle {
        ObservabilityHandle {
            endpoint: self.endpoint.map(MetricsEndpoint::start),
            #[cfg(feature = "opentelemetry")]
            otlp: self.otlp,
        }
    }
}

fn telemetry_enabled(config: &crate::config::ObservabilityConfig) -> bool {
    let enabled = config.metrics_listen.is_some();
    #[cfg(feature = "opentelemetry")]
    {
        return enabled || super::otlp::OtlpMetrics::configured();
    }
    #[cfg(not(feature = "opentelemetry"))]
    {
        enabled
    }
}

pub(crate) struct ObservabilityHandle {
    endpoint: Option<MetricsEndpointHandle>,
    #[cfg(feature = "opentelemetry")]
    otlp: Option<super::otlp::OtlpMetrics>,
}

impl ObservabilityHandle {
    pub(crate) fn stop(self) {
        if let Some(endpoint) = self.endpoint {
            endpoint.stop();
        }
        #[cfg(feature = "opentelemetry")]
        if let Some(otlp) = self.otlp {
            otlp.shutdown();
        }
    }
}

impl ObservabilityState {
    pub(crate) fn new_with_workers(
        network_worker_count: usize,
        storage_worker_count: usize,
        slow_request_threshold: Duration,
        enabled: bool,
    ) -> Self {
        Self {
            started_at: Instant::now(),
            enabled,
            network: (0..network_worker_count)
                .map(|_| NetworkWorkerMetrics::new(storage_worker_count))
                .collect(),
            storage: (0..storage_worker_count)
                .map(|_| StorageWorkerMetrics::new())
                .collect(),
            slow_request_threshold,
        }
    }

    pub(crate) fn network_shard(&self, worker: NetworkWorkerId) -> NetworkShard<'_> {
        NetworkShard::new(self, worker)
    }

    pub(crate) fn storage_shard(&self, worker: StorageWorkerId) -> StorageShard<'_> {
        StorageShard::new(self, worker)
    }

    pub(crate) fn set_draining(&self) {
        for metrics in &self.network {
            metrics.lifecycle.transition(Lifecycle::Draining);
        }
        for metrics in &self.storage {
            metrics.lifecycle.transition(Lifecycle::Draining);
        }
    }

    pub(crate) fn set_failed(&self) {
        for metrics in &self.network {
            metrics.lifecycle.transition(Lifecycle::Failed);
        }
        for metrics in &self.storage {
            metrics.lifecycle.transition(Lifecycle::Failed);
        }
    }

    pub(crate) fn network_worker_started(&self, worker: usize) {
        if let Some(metrics) = self.network.get(worker) {
            metrics.lifecycle.transition(Lifecycle::Ready);
        }
    }

    pub(crate) fn network_worker_failed(&self, worker: usize) {
        if let Some(metrics) = self.network.get(worker) {
            metrics.lifecycle.transition(Lifecycle::Failed);
        }
    }

    pub(crate) fn storage_worker_started(&self, worker: usize, cpu_id: usize) {
        if let Some(metrics) = self.storage.get(worker) {
            metrics.cpu_id.store(cpu_id as u64, Ordering::Relaxed);
            metrics.lifecycle.transition(Lifecycle::Ready);
        }
    }

    pub(crate) fn storage_worker_stopped(&self, worker: usize) {
        if let Some(metrics) = self.storage.get(worker) {
            metrics.lifecycle.transition(Lifecycle::Failed);
        }
    }

    fn lifecycle_value(&self) -> Lifecycle {
        if self
            .network
            .iter()
            .any(|metrics| metrics.lifecycle.load() == Lifecycle::Failed)
        {
            return Lifecycle::Failed;
        }
        if self
            .storage
            .iter()
            .any(|metrics| metrics.lifecycle.load() == Lifecycle::Failed)
        {
            return Lifecycle::Degraded;
        }
        if self
            .network
            .iter()
            .map(|metrics| metrics.lifecycle.load())
            .chain(self.storage.iter().map(|metrics| metrics.lifecycle.load()))
            .any(|value| value == Lifecycle::Draining)
        {
            return Lifecycle::Draining;
        }
        if self
            .network
            .iter()
            .any(|metrics| metrics.lifecycle.load() == Lifecycle::Degraded)
        {
            return Lifecycle::Degraded;
        }
        if self
            .network
            .iter()
            .any(|metrics| metrics.lifecycle.load() == Lifecycle::Starting)
            || self
                .storage
                .iter()
                .any(|metrics| metrics.lifecycle.load() == Lifecycle::Starting)
        {
            return Lifecycle::Starting;
        }
        if self.network.is_empty() || self.storage.is_empty() {
            return Lifecycle::Starting;
        }
        Lifecycle::Ready
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.lifecycle_value() == Lifecycle::Ready
    }

    pub(crate) fn connection_started_on(&self, worker: usize) {
        if self.enabled
            && let Some(metrics) = self.network.get(worker)
        {
            metrics.active_connections.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn handshake_succeeded_on(&self, worker: usize) {
        if self.enabled
            && let Some(metrics) = self.network.get(worker)
        {
            metrics.handshakes_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn connection_finished_on(&self, worker: usize) {
        if self.enabled
            && let Some(metrics) = self.network.get(worker)
        {
            metrics.active_connections.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn handshake_failed_on(&self, worker: usize) {
        if self.enabled
            && let Some(metrics) = self.network.get(worker)
        {
            metrics.handshakes_total.fetch_add(1, Ordering::Relaxed);
            metrics
                .handshake_failures_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn stream_started_on(&self, worker: usize) {
        if self.enabled
            && let Some(metrics) = self.network.get(worker)
        {
            metrics.active_streams.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn stream_finished_on(&self, worker: usize) {
        if self.enabled
            && let Some(metrics) = self.network.get(worker)
        {
            metrics.active_streams.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn protocol_error_on(&self, worker: usize) {
        if self.enabled
            && let Some(metrics) = self.network.get(worker)
        {
            metrics
                .protocol_errors_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn request_read_timeout_on(&self, worker: usize) {
        if self.enabled
            && let Some(metrics) = self.network.get(worker)
        {
            metrics
                .request_read_timeouts_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn response_write_failure_on(&self, worker: usize) {
        if self.enabled
            && let Some(metrics) = self.network.get(worker)
        {
            metrics
                .response_write_failures_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn abandoned_request_on(&self, worker: usize) {
        if self.enabled
            && let Some(metrics) = self.network.get(worker)
        {
            metrics
                .abandoned_requests_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_request_on(
        &self,
        worker: usize,
        operation: Operation,
        status: Status,
        elapsed: Duration,
    ) {
        if !self.enabled {
            return;
        }
        let Some(metrics) = self.network.get(worker) else {
            return;
        };
        let operation_index = operation.index();
        metrics.request.totals[operation_index][status_index(status)]
            .fetch_add(1, Ordering::Relaxed);
        metrics.request.durations[operation_index].observe(elapsed);
        if elapsed >= self.slow_request_threshold {
            metrics.slow_requests_total.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                target: "openkache::request",
                operation = operation.name(),
                status = STATUS_NAMES[status_index(status)],
                duration_us = elapsed.as_micros() as u64,
                "slow request"
            );
        }
    }

    pub(crate) fn storage_queue_full(&self, network_worker: usize, storage_worker: usize) {
        if !self.enabled {
            return;
        }
        if let Some(metrics) = self.network.get(network_worker)
            && let Some(counter) = metrics.storage_queue_full_total.get(storage_worker)
        {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_storage_wait(
        &self,
        network_worker: usize,
        storage_worker: usize,
        operation: Operation,
        elapsed: Duration,
    ) {
        if !self.enabled {
            return;
        }
        if let Some(metrics) = self.network.get(network_worker)
            && let Some(histograms) = metrics.storage_wait.get(storage_worker)
        {
            histograms[operation.index()].observe(elapsed);
        }
    }

    pub(crate) fn record_storage_operation(
        &self,
        worker: usize,
        operation: Operation,
        elapsed: Duration,
    ) {
        if !self.enabled {
            return;
        }
        if let Some(metrics) = self.storage.get(worker) {
            let index = operation.index();
            metrics.operations_total[index].fetch_add(1, Ordering::Relaxed);
            metrics.durations[index].observe(elapsed);
        }
    }

    pub(super) fn snapshot(&self) -> TelemetrySnapshot {
        let network = self
            .network
            .iter()
            .map(|metrics| NetworkWorkerSnapshot {
                lifecycle: metrics.lifecycle.load(),
                request_totals: std::array::from_fn(|operation| {
                    std::array::from_fn(|status| {
                        metrics.request.totals[operation][status].load(Ordering::Relaxed)
                    })
                }),
                request_durations: std::array::from_fn(|operation| {
                    metrics.request.durations[operation].snapshot()
                }),
                storage_queue_full_total: metrics
                    .storage_queue_full_total
                    .iter()
                    .map(|counter| counter.load(Ordering::Relaxed))
                    .collect(),
                storage_wait: metrics
                    .storage_wait
                    .iter()
                    .map(|histograms| {
                        std::array::from_fn(|operation| histograms[operation].snapshot())
                    })
                    .collect(),
                active_connections: metrics.active_connections.load(Ordering::Relaxed),
                active_streams: metrics.active_streams.load(Ordering::Relaxed),
                handshakes_total: metrics.handshakes_total.load(Ordering::Relaxed),
                handshake_failures_total: metrics.handshake_failures_total.load(Ordering::Relaxed),
                protocol_errors_total: metrics.protocol_errors_total.load(Ordering::Relaxed),
                request_read_timeouts_total: metrics
                    .request_read_timeouts_total
                    .load(Ordering::Relaxed),
                response_write_failures_total: metrics
                    .response_write_failures_total
                    .load(Ordering::Relaxed),
                abandoned_requests_total: metrics.abandoned_requests_total.load(Ordering::Relaxed),
                slow_requests_total: metrics.slow_requests_total.load(Ordering::Relaxed),
            })
            .collect();
        let storage = self
            .storage
            .iter()
            .map(|metrics| StorageWorkerSnapshot {
                lifecycle: metrics.lifecycle.load(),
                cpu_id: metrics.cpu_id.load(Ordering::Relaxed),
                operations_total: std::array::from_fn(|operation| {
                    metrics.operations_total[operation].load(Ordering::Relaxed)
                }),
                durations: std::array::from_fn(|operation| metrics.durations[operation].snapshot()),
            })
            .collect();
        TelemetrySnapshot {
            uptime_seconds: self.started_at.elapsed().as_secs_f64(),
            lifecycle: self.lifecycle_value(),
            network,
            storage,
        }
    }

    /// Renders a bounded OpenMetrics text snapshot without taking a data-plane lock.
    pub(crate) fn render_prometheus_with_scrapes(&self, scrapes: u64) -> String {
        super::prometheus::render(&self.snapshot(), scrapes)
    }

    /// Adds machine-readable fields to the authenticated `STATS` JSON response.
    pub(crate) fn stats_json_fields(&self) -> String {
        let snapshot = self.snapshot();
        let active_connections = snapshot
            .network
            .iter()
            .map(|metrics| metrics.active_connections)
            .sum::<u64>();
        let active_streams = snapshot
            .network
            .iter()
            .map(|metrics| metrics.active_streams)
            .sum::<u64>();
        let requests_total = snapshot
            .network
            .iter()
            .flat_map(|metrics| metrics.request_totals.iter())
            .flat_map(|statuses| statuses.iter())
            .sum::<u64>();
        format!(
            r#""schema_version":1,"uptime_seconds":{:.3},"ready":{},"degraded":{},"active_connections":{},"active_streams":{},"requests_total":{}"#,
            snapshot.uptime_seconds,
            snapshot.lifecycle == Lifecycle::Ready,
            matches!(snapshot.lifecycle, Lifecycle::Degraded),
            active_connections,
            active_streams,
            requests_total,
        )
    }
}
