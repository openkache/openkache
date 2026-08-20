//! QUIC server backed by the sharded SSD-first cache runtime.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::Status;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

#[allow(unused_imports)]
use crate::KvError;
use crate::channel::{self, AsyncReceiver, Sender};
use crate::network_runtime;
use crate::network_runtime::{TcpListener, TcpStream};
use crate::observability::{
    NetworkShard, NetworkWorkerId, ObservabilityService, ObservabilityState, ObservabilityStats,
};
use crate::platform::StorageDeviceKind;
use crate::protocol::{NamespaceDescriptor, NamespacePolicy};
use crate::transport::tcp::TlsTcpLane;
use crate::transport::{
    Connection as TransportConnection, Endpoint as TransportEndpoint,
    Incoming as TransportIncoming, ReceiveStream, RequestBudget, SendStream, ServerEndpoint,
    RequestBudgetPermit, RequestFrame, RequestRead, ServerTlsConfig, StreamReadError,
    TransportError, strict_server_config,
};
use crate::{AppConfig, NetworkConfig, NetworkWorkerCache, QuicBackend, ThreadedKvkache};
#[allow(unused_imports)]
pub(crate) use crate::{contract, protocol};
// Operation modules are nested below this composition root, while the
// generated operation contract lives at the crate root. Re-exporting the
// contract here keeps the nested modules independent of the crate layout.
pub(crate) use crate::operation_contract;
// Keep the compatibility contract available to compatibility bindings. It
// remains `pub(crate)`, so this does not add a public server API or make
// generic dispatch depend on v1 vocabulary.
#[allow(unused_imports)]
pub(crate) use super::operation_compatibility_contract;

#[path = "../operation_authorization.rs"]
mod operation_authorization;
#[path = "../operation_capabilities.rs"]
mod operation_capabilities;
#[path = "../operation_codecs.rs"]
pub(crate) mod operation_codecs;
#[path = "../operation_compatibility_behavior.rs"]
mod operation_compatibility_behavior;
#[path = "../operation_compatibility_decode.rs"]
mod operation_compatibility_decode;
#[path = "../operation_compatibility_handlers.rs"]
mod operation_compatibility_handlers;
#[path = "../operation_compatibility_module.rs"]
mod operation_compatibility_module;
#[path = "../operation_compatibility_prepare.rs"]
mod operation_compatibility_prepare;
#[path = "../operation_compatibility_services.rs"]
mod operation_compatibility_services;
#[path = "../operation_composition.rs"]
mod operation_composition;
#[path = "../operation_dispatch.rs"]
pub(crate) mod operation_dispatch;
#[path = "../operation_execution_state.rs"]
mod operation_execution_state;
#[path = "../operation_fields.rs"]
mod operation_fields;
#[path = "../operation_generic_bindings.rs"]
mod operation_generic_bindings;
#[path = "../operation_handlers.rs"]
mod operation_handlers;
#[path = "../operation_outcome.rs"]
mod operation_outcome;
#[path = "../operation_ports.rs"]
mod operation_ports;
#[path = "../operation_preparation.rs"]
mod operation_preparation;
#[path = "../operation_registration.rs"]
mod operation_registration;
#[path = "../operation_registrations.rs"]
mod operation_registrations;
#[path = "../operation_registry.rs"]
mod operation_registry;
#[path = "../operation_transport.rs"]
mod operation_transport;
#[path = "../request_projection.rs"]
mod request_projection;
#[path = "../storage_port.rs"]
mod storage_port;

pub use operation_capabilities::{
    CapabilityCatalog, CapabilityEntry, CapabilityKey, CapabilityList, CapabilityRegistry,
    EmptyCapabilityCatalog,
};

#[path = "network_roles.rs"]
mod network_roles;
#[allow(unused_imports)]
pub(crate) use network_roles::{
    NetworkRolePlacement, NetworkTaskReporter, NetworkWorkerCompletion, NetworkWorkerHandle,
    NetworkWorkerReporter, launch_network_role,
};

#[path = "tls.rs"]
mod tls;
use tls::{AccessPolicy, load_production_tls};
#[path = "errors.rs"]
mod errors;
pub use errors::{Result, ServerError};

#[path = "namespace_journal.rs"]
mod namespace_journal;
#[path = "namespace_metadata.rs"]
pub(crate) mod namespace_metadata;
#[path = "namespace_registry.rs"]
mod namespace_registry;
#[allow(unused_imports)]
pub(crate) use namespace_journal::{JournalEvent, NamespaceJournal};
pub(crate) use namespace_registry::{
    NamespaceError, NamespaceOpenResult, NamespaceOperationLock, NamespaceRegistry, SetReservation,
};

#[path = "connection.rs"]
mod connection;
#[path = "lifecycle.rs"]
mod lifecycle;
#[allow(unused_imports)]
pub(crate) use lifecycle::{join_network_threads, shutdown_network_workers_and_cache};

/// Bound reuse-port sockets and the sharded SSD-backed cache they serve.
pub struct KacheServer {
    sockets: Vec<std::net::UdpSocket>,
    tcp_sockets: Vec<std::net::TcpListener>,
    local_addr: SocketAddr,
    tcp_local_addr: SocketAddr,
    quic_backend: QuicBackend,
    tls: Arc<ServerTlsConfig>,
    access_policy: Arc<AccessPolicy>,
    cache: Arc<ThreadedKvkache>,
    namespaces: Arc<Mutex<NamespaceRegistry>>,
    network: NetworkConfig,
    request_timeout: Duration,
    experimental_api_enabled: bool,
    experimental_api_revision: Option<String>,
    observability: ObservabilityService,
    capabilities: Arc<dyn CapabilityCatalog>,
}

/// Result of resolving a name through the out-of-band namespace gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamespaceOpenOutcome {
    Existing,
    Created,
}

/// Errors returned by the out-of-band namespace lifecycle seam.
#[derive(Debug, thiserror::Error)]
pub enum NamespaceGateError {
    #[error("namespace request is invalid")]
    InvalidRequest,
    #[error("namespace does not exist")]
    NotFound,
    #[error("namespace revision conflicts with the current descriptor")]
    Conflict,
    #[error("namespace still contains live items")]
    NotEmpty,
    #[error("namespace metadata is unavailable")]
    Internal,
}

/// Explicit control-plane seam for server-assigned namespace identities.
///
/// Stable v1 data-plane requests never create, look up, update, or delete
/// namespaces. A control-plane adapter obtains this handle from
/// [`KacheServer::namespace_gate`], resolves a name to a positive server-owned
/// ID, and then carries that ID in data-plane requests. Policy is immutable:
/// opening an existing name returns its original descriptor and ignores any
/// replacement policy.
#[derive(Clone)]
pub struct NamespaceGate {
    registry: Arc<Mutex<NamespaceRegistry>>,
    storage: Option<Arc<ThreadedKvkache>>,
}

impl NamespaceGate {
    #[allow(dead_code)]
    fn new(registry: Arc<Mutex<NamespaceRegistry>>) -> Self {
        Self {
            registry,
            storage: None,
        }
    }

    pub(crate) fn with_storage(
        registry: Arc<Mutex<NamespaceRegistry>>,
        storage: Arc<ThreadedKvkache>,
    ) -> Self {
        Self {
            registry,
            storage: Some(storage),
        }
    }

    /// Resolves or creates a namespace under the serialized lifecycle gate.
    pub async fn open(
        &self,
        name: &[u8],
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> std::result::Result<(NamespaceOpenOutcome, NamespaceDescriptor), NamespaceGateError> {
        if std::str::from_utf8(name).is_err() {
            return Err(NamespaceGateError::InvalidRequest);
        }
        let lifecycle = self
            .registry
            .lock()
            .map_err(|_| NamespaceGateError::Internal)?
            .lifecycle_lock();
        let _guard = lifecycle.lock().await;
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| NamespaceGateError::Internal)?;
        let (outcome, descriptor) = registry
            .open(
                openkache_protocol::OwnedRange::whole(name.to_vec()),
                create_if_missing,
                policy,
            )
            .map_err(NamespaceGateError::from)?;
        let outcome = match outcome {
            NamespaceOpenResult::Existing => NamespaceOpenOutcome::Existing,
            NamespaceOpenResult::Created => NamespaceOpenOutcome::Created,
        };
        Ok((outcome, descriptor))
    }

    /// Deletes an empty namespace after checking its current descriptor
    /// revision. The production gate probes each conservatively tracked item
    /// through storage first, pruning expired or already-absent values before
    /// enforcing the live-membership check.
    pub async fn delete(
        &self,
        namespace_id: u64,
        expected_revision: u64,
    ) -> std::result::Result<(), NamespaceGateError> {
        let lifecycle = self
            .registry
            .lock()
            .map_err(|_| NamespaceGateError::Internal)?
            .lifecycle_lock();
        let _guard = lifecycle.lock().await;
        let operation = self
            .registry
            .lock()
            .map_err(|_| NamespaceGateError::Internal)?
            .operation_lock(namespace_id)
            .ok_or(NamespaceGateError::NotFound)?;
        let (operation_lock, _active) = operation.into_parts();
        let _operation_guard = operation_lock.lock().await;
        if let Some(storage) = &self.storage {
            let item_keys = self
                .registry
                .lock()
                .map_err(|_| NamespaceGateError::Internal)?
                .item_keys(namespace_id)
                .ok_or(NamespaceGateError::NotFound)?;
            for storage_key in item_keys {
                let value = storage
                    .get_stored(storage_key)
                    .await
                    .map_err(|_| NamespaceGateError::Internal)?;
                if value.is_none() {
                    self.registry
                        .lock()
                        .map_err(|_| NamespaceGateError::Internal)?
                        .prune_item(namespace_id, storage_key)
                        .map_err(NamespaceGateError::from)?;
                }
            }
        }
        self.registry
            .lock()
            .map_err(|_| NamespaceGateError::Internal)?
            .delete(namespace_id, expected_revision)
            .map_err(NamespaceGateError::from)
    }
}

impl From<NamespaceError> for NamespaceGateError {
    fn from(error: NamespaceError) -> Self {
        match error {
            NamespaceError::InvalidRequest => Self::InvalidRequest,
            NamespaceError::NotFound => Self::NotFound,
            NamespaceError::Conflict => Self::Conflict,
            NamespaceError::NotEmpty => Self::NotEmpty,
            NamespaceError::PolicyConflict | NamespaceError::Internal => Self::Internal,
        }
    }
}
