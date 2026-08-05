//! Prometheus/OpenMetrics formatting helpers.

use std::fmt::Write as _;

use super::lifecycle::Lifecycle;
use super::service::{
    HISTOGRAM_BUCKETS_US, HistogramSnapshot, OPERATION_NAMES, STATUS_NAMES, TelemetrySnapshot,
};

fn render_snapshot_histograms<'a>(
    output: &mut String,
    metric_name: &str,
    labels: &str,
    operation: &str,
    histograms: impl Iterator<Item = &'a HistogramSnapshot>,
) {
    let mut buckets = [0_u64; HISTOGRAM_BUCKETS_US.len()];
    let mut count = 0;
    let mut sum_ns = 0;
    for histogram in histograms {
        for (index, bucket) in histogram.buckets.iter().enumerate() {
            buckets[index] += bucket;
        }
        count += histogram.count;
        sum_ns += histogram.sum_ns;
    }
    let mut cumulative = 0;
    for (index, boundary) in HISTOGRAM_BUCKETS_US.iter().enumerate() {
        cumulative += buckets[index];
        writeln!(
            output,
            "{metric_name}_bucket{{{labels}operation=\"{operation}\",le=\"{:.6}\"}} {cumulative}",
            *boundary as f64 / 1_000_000.0
        )
        .expect("writing a String cannot fail");
    }
    writeln!(
        output,
        "{metric_name}_bucket{{{labels}operation=\"{operation}\",le=\"+Inf\"}} {count}"
    )
    .expect("writing a String cannot fail");
    writeln!(
        output,
        "{metric_name}_count{{{labels}operation=\"{operation}\"}} {count}"
    )
    .expect("writing a String cannot fail");
    writeln!(
        output,
        "{metric_name}_sum{{{labels}operation=\"{operation}\"}} {:.9}",
        sum_ns as f64 / 1_000_000_000.0
    )
    .expect("writing a String cannot fail");
}

/// Renders one immutable management-plane snapshot in OpenMetrics text format.
pub(super) fn render(snapshot: &TelemetrySnapshot, scrapes: u64) -> String {
    let mut output = String::with_capacity(16 * 1024);
    let ready = u8::from(snapshot.lifecycle == Lifecycle::Ready);
    let degraded = u8::from(snapshot.lifecycle == Lifecycle::Degraded);
    let draining = u8::from(snapshot.lifecycle == Lifecycle::Draining);
    let failed = u8::from(snapshot.lifecycle == Lifecycle::Failed);
    let sum_network = |read: fn(&super::service::NetworkWorkerSnapshot) -> u64| {
        snapshot.network.iter().map(read).sum::<u64>()
    };

    writeln!(
        output,
        "# HELP openkache_info Build and runtime information."
    )
    .expect("writing a String cannot fail");
    writeln!(output, "# TYPE openkache_info gauge").expect("writing a String cannot fail");
    writeln!(
        output,
        "openkache_info{{version=\"{}\",storage_runtime=\"{}\"}} 1",
        env!("CARGO_PKG_VERSION"),
        crate::storage_runtime_name()
    )
    .expect("writing a String cannot fail");

    write_gauge(
        &mut output,
        "openkache_uptime_seconds",
        snapshot.uptime_seconds,
    );
    write_gauge(&mut output, "openkache_ready", f64::from(ready));
    write_gauge(&mut output, "openkache_degraded", f64::from(degraded));
    write_gauge(&mut output, "openkache_draining", f64::from(draining));
    write_gauge(&mut output, "openkache_failed", f64::from(failed));
    write_gauge(
        &mut output,
        "openkache_active_connections",
        sum_network(|metrics| metrics.active_connections) as f64,
    );
    write_gauge(
        &mut output,
        "openkache_active_streams",
        sum_network(|metrics| metrics.active_streams) as f64,
    );

    write_counter(
        &mut output,
        "openkache_handshakes_total",
        sum_network(|metrics| metrics.handshakes_total),
    );
    write_counter(
        &mut output,
        "openkache_handshake_failures_total",
        sum_network(|metrics| metrics.handshake_failures_total),
    );
    write_counter(
        &mut output,
        "openkache_protocol_errors_total",
        sum_network(|metrics| metrics.protocol_errors_total),
    );
    write_counter(
        &mut output,
        "openkache_request_read_timeouts_total",
        sum_network(|metrics| metrics.request_read_timeouts_total),
    );
    write_counter(
        &mut output,
        "openkache_response_write_failures_total",
        sum_network(|metrics| metrics.response_write_failures_total),
    );
    write_counter(
        &mut output,
        "openkache_abandoned_requests_total",
        sum_network(|metrics| metrics.abandoned_requests_total),
    );
    write_counter(
        &mut output,
        "openkache_slow_requests_total",
        sum_network(|metrics| metrics.slow_requests_total),
    );
    write_counter(&mut output, "openkache_metrics_scrapes_total", scrapes);

    writeln!(output, "# TYPE openkache_requests_total counter")
        .expect("writing a String cannot fail");
    for operation in 0..OPERATION_NAMES.len() {
        for (status, status_name) in STATUS_NAMES.iter().enumerate() {
            let count = snapshot
                .network
                .iter()
                .map(|metrics| metrics.request_totals[operation][status])
                .sum::<u64>();
            writeln!(
                output,
                "openkache_requests_total{{operation=\"{}\",status=\"{status_name}\"}} {count}",
                OPERATION_NAMES[operation]
            )
            .expect("writing a String cannot fail");
        }
    }

    writeln!(output, "# TYPE openkache_network_worker_up gauge")
        .expect("writing a String cannot fail");
    for (worker, metrics) in snapshot.network.iter().enumerate() {
        writeln!(
            output,
            "openkache_network_worker_up{{worker=\"{worker}\"}} {}",
            u8::from(metrics.lifecycle.is_up())
        )
        .expect("writing a String cannot fail");
    }

    writeln!(
        output,
        "# TYPE openkache_request_duration_seconds histogram"
    )
    .expect("writing a String cannot fail");
    for operation in 0..OPERATION_NAMES.len() {
        render_snapshot_histograms(
            &mut output,
            "openkache_request_duration_seconds",
            "",
            OPERATION_NAMES[operation],
            snapshot
                .network
                .iter()
                .map(|metrics| &metrics.request_durations[operation]),
        );
    }

    writeln!(output, "# TYPE openkache_storage_worker_up gauge")
        .expect("writing a String cannot fail");
    writeln!(output, "# TYPE openkache_storage_queue_full_total counter")
        .expect("writing a String cannot fail");
    writeln!(
        output,
        "# TYPE openkache_storage_wait_duration_seconds histogram"
    )
    .expect("writing a String cannot fail");
    writeln!(output, "# TYPE openkache_storage_operations_total counter")
        .expect("writing a String cannot fail");
    writeln!(
        output,
        "# TYPE openkache_storage_operation_duration_seconds histogram"
    )
    .expect("writing a String cannot fail");
    for (worker, metrics) in snapshot.storage.iter().enumerate() {
        let labels = format!("worker=\"{worker}\",cpu_id=\"{}\"", metrics.cpu_id);
        let histogram_labels = format!("{labels},");
        writeln!(
            output,
            "openkache_storage_worker_up{{{labels}}} {}",
            u8::from(metrics.lifecycle.is_up())
        )
        .expect("writing a String cannot fail");
        let queue_full = snapshot
            .network
            .iter()
            .filter_map(|network| network.storage_queue_full_total.get(worker))
            .sum::<u64>();
        writeln!(
            output,
            "openkache_storage_queue_full_total{{{labels}}} {queue_full}"
        )
        .expect("writing a String cannot fail");
        for operation in 0..OPERATION_NAMES.len() {
            writeln!(
                output,
                "openkache_storage_operations_total{{{labels},operation=\"{}\"}} {}",
                OPERATION_NAMES[operation], metrics.operations_total[operation]
            )
            .expect("writing a String cannot fail");
            render_snapshot_histograms(
                &mut output,
                "openkache_storage_operation_duration_seconds",
                &histogram_labels,
                OPERATION_NAMES[operation],
                std::iter::once(&metrics.durations[operation]),
            );
        }
    }
    for (network_worker, network) in snapshot.network.iter().enumerate() {
        for worker in 0..snapshot.storage.len() {
            let wait_labels =
                format!("network_worker=\"{network_worker}\",storage_worker=\"{worker}\",");
            for operation in 0..OPERATION_NAMES.len() {
                render_snapshot_histograms(
                    &mut output,
                    "openkache_storage_wait_duration_seconds",
                    &wait_labels,
                    OPERATION_NAMES[operation],
                    network.storage_wait[worker].get(operation).into_iter(),
                );
            }
        }
    }
    output
}

fn write_gauge(output: &mut String, name: &str, value: f64) {
    writeln!(output, "# TYPE {name} gauge").expect("writing a String cannot fail");
    writeln!(output, "{name} {value:.6}").expect("writing a String cannot fail");
}

fn write_counter(output: &mut String, name: &str, value: u64) {
    writeln!(output, "# TYPE {name} counter").expect("writing a String cannot fail");
    writeln!(output, "{name} {value}").expect("writing a String cannot fail");
}
