//! Worker-local observability and its management-plane exporter.

mod http;
mod lifecycle;
#[cfg(feature = "opentelemetry")]
mod otlp;
mod prometheus;
#[path = "observability/service.rs"]
mod service;
mod shard;

#[allow(unused_imports)]
pub(crate) use http::MetricsEndpoint;
pub(crate) use service::{ObservabilityService, ObservabilityState, ObservabilityStats, Operation};
pub(crate) use shard::{NetworkShard, NetworkWorkerId, StorageWorkerId};
