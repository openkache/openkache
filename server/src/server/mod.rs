//! QUIC server backed by the sharded SSD-first cache runtime.

use std::fmt::Write as _;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::Status;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::channel::{self, AsyncReceiver};
use crate::network_runtime;
use crate::observability::{
    NetworkShard, NetworkWorkerId, ObservabilityService, ObservabilityState, Operation,
};
use crate::platform::StorageDeviceKind;
use crate::protocol::{ItemId, NamespaceDescriptor, NamespacePolicy, Response};
use crate::transport::{
    Connection as TransportConnection, Endpoint as TransportEndpoint,
    Incoming as TransportIncoming, ReceiveStream, RequestBudget, SendStream, ServerEndpoint,
    ServerTlsConfig, StreamReadError, TransportError,
};
use crate::{AppConfig, NetworkConfig, NetworkWorkerCache, QuicBackend, ThreadedKvkache};
#[allow(unused_imports)]
use crate::KvError;
#[allow(unused_imports)]
pub(crate) use crate::{contract, protocol};
// Operation modules are nested below this composition root, while the
// generated operation contract lives at the crate root. Re-exporting the
// facade here keeps the nested modules independent of the crate layout and
// lets the private source-copy test harness use the same path.
pub(crate) use crate::operation_contract;
// Keep the compatibility contract available through the same composition
// facade for white-box tests and compatibility bindings. It remains
// `pub(crate)`, so this does not add a public server API or make generic
// dispatch depend on v1 vocabulary.
#[allow(unused_imports)]
pub(crate) use super::operation_compatibility_contract;

#[path = "../operation_api.rs"]
mod operation_api;
#[path = "../operation_authorization.rs"]
mod operation_authorization;
#[path = "../operation_capabilities.rs"]
mod operation_capabilities;
#[path = "../operation_codecs.rs"]
pub(crate) mod operation_codecs;
#[path = "../operation_compatibility_behavior.rs"]
mod operation_compatibility_behavior;
#[path = "../operation_compatibility_bindings.rs"]
mod operation_compatibility_bindings;
#[path = "../operation_compatibility_services.rs"]
mod operation_compatibility_services;
pub(crate) use operation_compatibility_services::storage_write_options;
#[path = "../operation_dispatch.rs"]
pub(crate) mod operation_dispatch;
#[path = "../operation_fields.rs"]
mod operation_fields;
#[path = "../operation_generic_bindings.rs"]
mod operation_generic_bindings;
#[path = "../operation_handlers.rs"]
mod operation_handlers;
#[path = "../operation_outcome.rs"]
mod operation_outcome;
#[path = "../operation_registrations.rs"]
mod operation_registrations;
#[path = "../operation_registry.rs"]
mod operation_registry;
#[path = "../operation_runtime_capabilities.rs"]
mod operation_runtime_capabilities;
#[path = "../operation_transport.rs"]
mod operation_transport;
#[path = "../request_projection.rs"]
mod request_projection;
#[path = "../storage_port.rs"]
mod storage_port;

pub use operation_api::CapabilityKey;
pub use operation_capabilities::{
    CapabilityCatalog, CapabilityEntry, CapabilityList, CapabilityRegistry, EmptyCapabilityCatalog,
};

pub(crate) const fn request_descriptor(
    opcode: openkache_protocol::Opcode,
) -> &'static crate::protocol::RequestDescriptor {
    operation_registrations::request_descriptor(opcode)
}

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


#[path = "namespace_registry.rs"]
mod namespace_registry;
pub(crate) use namespace_registry::{
    NamespaceError, NamespaceOpenResult, NamespaceRegistry, SetReservation,
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
    local_addr: SocketAddr,
    quic_backend: QuicBackend,
    tls: Arc<ServerTlsConfig>,
    access_policy: Arc<AccessPolicy>,
    cache: Arc<ThreadedKvkache>,
    namespaces: Arc<Mutex<NamespaceRegistry>>,
    network: NetworkConfig,
    request_timeout: Duration,
    max_item_bytes: usize,
    observability: ObservabilityService,
    capabilities: Arc<dyn CapabilityCatalog>,
}
