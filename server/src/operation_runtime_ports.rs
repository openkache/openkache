//! Server-owned ports available while API modules install capabilities.
//!
//! Each handle has an independent typed identity. The borrowed bootstrap
//! entries never enter the request catalog; module installers can derive or
//! retain only the request capabilities they need from their values.

use std::sync::{Arc, Mutex};

use super::operation_api::CapabilityKey;
use super::{NamespaceRegistry, NetworkWorkerCache, ObservabilityState};

pub(super) const NETWORK_WORKER_CACHE: CapabilityKey<Arc<NetworkWorkerCache>> =
    CapabilityKey::new("openkache.server.network_worker_cache");
pub(super) const NAMESPACE_REGISTRY: CapabilityKey<Arc<Mutex<NamespaceRegistry>>> =
    CapabilityKey::new("openkache.server.namespace_registry");
pub(super) const OBSERVABILITY_STATE: CapabilityKey<Arc<ObservabilityState>> =
    CapabilityKey::new("openkache.server.observability_state");
