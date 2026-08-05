//! Optional OpenTelemetry metrics export.
//!
//! The exporter is deliberately attached to an SDK-owned management thread.
//! Worker-local atomics are copied into a [`TelemetrySnapshot`] only when the
//! SDK performs an export callback; no OpenTelemetry instrument or global
//! provider is touched by the request path.

use std::sync::Arc;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Meter, MeterProvider};
use opentelemetry_otlp::MetricExporter;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;

use super::service::{OPERATION_NAMES, ObservabilityState, STATUS_NAMES, TelemetrySnapshot};

const OTEL_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
const OTEL_METRICS_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT";

pub(super) struct OtlpMetrics {
    provider: SdkMeterProvider,
}

impl OtlpMetrics {
    pub(super) fn configured() -> bool {
        std::env::var_os(OTEL_METRICS_ENDPOINT_ENV).is_some()
            || std::env::var_os(OTEL_ENDPOINT_ENV).is_some()
    }

    /// Builds an OTLP/HTTP metrics provider when an OTLP endpoint is configured.
    ///
    /// `opentelemetry-otlp` follows the standard environment variable rules for
    /// endpoint, headers, timeout, and export interval. The feature remains
    /// dormant when neither the signal-specific nor generic endpoint is set.
    pub(super) fn from_environment(state: Arc<ObservabilityState>) -> Option<Self> {
        if !Self::configured() {
            return None;
        }

        let exporter = match MetricExporter::builder().with_http().build() {
            Ok(exporter) => exporter,
            Err(error) => {
                tracing::warn!(
                    target: "openkache::observability",
                    error = %error,
                    "OpenTelemetry metrics exporter disabled because it could not be built"
                );
                return None;
            }
        };
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter)
            .with_resource(Resource::builder().with_service_name("openkache").build())
            .build();
        register_metrics(&provider, state);
        Some(Self { provider })
    }

    pub(super) fn shutdown(self) {
        if let Err(error) = self.provider.shutdown() {
            tracing::debug!(
                target: "openkache::observability",
                error = %error,
                "OpenTelemetry metrics provider shutdown failed"
            );
        }
    }
}

fn register_metrics(provider: &SdkMeterProvider, state: Arc<ObservabilityState>) {
    let meter = provider.meter("openkache.observability");
    register_gauge(&meter, "openkache.ready", &state, |snapshot| {
        u64::from(snapshot.lifecycle.is_ready())
    });
    register_gauge(&meter, "openkache.degraded", &state, |snapshot| {
        u64::from(snapshot.lifecycle.is_degraded())
    });
    register_gauge(&meter, "openkache.draining", &state, |snapshot| {
        u64::from(snapshot.lifecycle.is_draining())
    });
    register_gauge(&meter, "openkache.failed", &state, |snapshot| {
        u64::from(snapshot.lifecycle.is_failed())
    });
    register_gauge(&meter, "openkache.uptime", &state, |snapshot| {
        snapshot.uptime_seconds.max(0.0) as u64
    });
    register_gauge(&meter, "openkache.active_connections", &state, |snapshot| {
        snapshot
            .network
            .iter()
            .map(|metrics| metrics.active_connections)
            .sum()
    });
    register_gauge(&meter, "openkache.active_streams", &state, |snapshot| {
        snapshot
            .network
            .iter()
            .map(|metrics| metrics.active_streams)
            .sum()
    });

    register_counter(&meter, "openkache.handshakes", &state, |snapshot| {
        snapshot
            .network
            .iter()
            .map(|metrics| metrics.handshakes_total)
            .sum()
    });
    register_counter(&meter, "openkache.handshake_failures", &state, |snapshot| {
        snapshot
            .network
            .iter()
            .map(|metrics| metrics.handshake_failures_total)
            .sum()
    });
    register_counter(&meter, "openkache.protocol_errors", &state, |snapshot| {
        snapshot
            .network
            .iter()
            .map(|metrics| metrics.protocol_errors_total)
            .sum()
    });
    register_counter(
        &meter,
        "openkache.request_read_timeouts",
        &state,
        |snapshot| {
            snapshot
                .network
                .iter()
                .map(|metrics| metrics.request_read_timeouts_total)
                .sum()
        },
    );
    register_counter(
        &meter,
        "openkache.response_write_failures",
        &state,
        |snapshot| {
            snapshot
                .network
                .iter()
                .map(|metrics| metrics.response_write_failures_total)
                .sum()
        },
    );
    register_counter(&meter, "openkache.abandoned_requests", &state, |snapshot| {
        snapshot
            .network
            .iter()
            .map(|metrics| metrics.abandoned_requests_total)
            .sum()
    });
    register_counter(&meter, "openkache.slow_requests", &state, |snapshot| {
        snapshot
            .network
            .iter()
            .map(|metrics| metrics.slow_requests_total)
            .sum()
    });

    let state_for_requests = Arc::clone(&state);
    meter
        .u64_observable_counter("openkache.requests")
        .with_callback(move |observer| {
            let snapshot = state_for_requests.snapshot();
            for (operation, operation_name) in OPERATION_NAMES.iter().enumerate() {
                for (status, status_name) in STATUS_NAMES.iter().enumerate() {
                    let attributes = [
                        KeyValue::new("operation", *operation_name),
                        KeyValue::new("status", *status_name),
                    ];
                    observer.observe(
                        snapshot
                            .network
                            .iter()
                            .map(|metrics| metrics.request_totals[operation][status])
                            .sum(),
                        &attributes,
                    );
                }
            }
        })
        .build();

    let state_for_storage = Arc::clone(&state);
    meter
        .u64_observable_counter("openkache.storage.operations")
        .with_callback(move |observer| {
            let snapshot = state_for_storage.snapshot();
            for (worker, metrics) in snapshot.storage.iter().enumerate() {
                for (operation, operation_name) in OPERATION_NAMES.iter().enumerate() {
                    let attributes = [
                        KeyValue::new("worker", worker.to_string()),
                        KeyValue::new("cpu_id", metrics.cpu_id.to_string()),
                        KeyValue::new("operation", *operation_name),
                    ];
                    observer.observe(metrics.operations_total[operation], &attributes);
                }
            }
        })
        .build();

    let state_for_storage_queue = Arc::clone(&state);
    meter
        .u64_observable_counter("openkache.storage.queue_full")
        .with_callback(move |observer| {
            let snapshot = state_for_storage_queue.snapshot();
            for (worker, metrics) in snapshot.storage.iter().enumerate() {
                let queue_full = snapshot
                    .network
                    .iter()
                    .filter_map(|network| network.storage_queue_full_total.get(worker))
                    .sum();
                let attributes = [
                    KeyValue::new("worker", worker.to_string()),
                    KeyValue::new("cpu_id", metrics.cpu_id.to_string()),
                ];
                observer.observe(queue_full, &attributes);
            }
        })
        .build();

    let state_for_storage_health = Arc::clone(&state);
    meter
        .u64_observable_gauge("openkache.storage.worker_up")
        .with_callback(move |observer| {
            let snapshot = state_for_storage_health.snapshot();
            for (worker, metrics) in snapshot.storage.iter().enumerate() {
                let attributes = [
                    KeyValue::new("worker", worker.to_string()),
                    KeyValue::new("cpu_id", metrics.cpu_id.to_string()),
                ];
                observer.observe(u64::from(metrics.lifecycle.is_up()), &attributes);
            }
        })
        .build();

    let state_for_network_health = Arc::clone(&state);
    meter
        .u64_observable_gauge("openkache.network.worker_up")
        .with_callback(move |observer| {
            let snapshot = state_for_network_health.snapshot();
            for (worker, metrics) in snapshot.network.iter().enumerate() {
                let attributes = [KeyValue::new("worker", worker.to_string())];
                observer.observe(u64::from(metrics.lifecycle.is_up()), &attributes);
            }
        })
        .build();
}

fn register_gauge(
    meter: &Meter,
    name: &'static str,
    state: &Arc<ObservabilityState>,
    value: impl Fn(&TelemetrySnapshot) -> u64 + Send + Sync + 'static,
) {
    let state = Arc::clone(state);
    meter
        .u64_observable_gauge(name)
        .with_callback(move |observer| {
            let snapshot = state.snapshot();
            observer.observe(value(&snapshot), &[]);
        })
        .build();
}

fn register_counter(
    meter: &Meter,
    name: &'static str,
    state: &Arc<ObservabilityState>,
    value: impl Fn(&TelemetrySnapshot) -> u64 + Send + Sync + 'static,
) {
    let state = Arc::clone(state);
    meter
        .u64_observable_counter(name)
        .with_callback(move |observer| {
            let snapshot = state.snapshot();
            observer.observe(value(&snapshot), &[]);
        })
        .build();
}
