//! Server-owned resources available while API modules install capabilities.
//!
//! This bootstrap bundle is separate from the immutable request capability
//! catalog. Modules consume only the resources they need, and bootstrap-only
//! handles never enter request execution.

use std::sync::{Arc, Mutex};

use super::operation_api::CapabilityKey;
use super::{NamespaceRegistry, NetworkWorkerCache, ObservabilityState};

pub(super) const SERVER_RUNTIME_RESOURCES: CapabilityKey<ServerRuntimeResources> =
    CapabilityKey::new("openkache.server.runtime_resources");

pub(super) struct ServerRuntimeResources {
    pub(super) cache: Arc<NetworkWorkerCache>,
    pub(super) namespaces: Arc<Mutex<NamespaceRegistry>>,
    pub(super) observability: Arc<ObservabilityState>,
}
