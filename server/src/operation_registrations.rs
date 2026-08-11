//! API binding registrations.
//!
//! The generic registry module validates and looks up these entries, while
//! this module owns the API-specific function pointers. Keeping the table out
//! of the registry foundation makes the composition boundary explicit. Every
//! `handler` field uses one generic `OperationHandler` future boundary; this
//! table never introduces a transport- or protocol-specific handler family.

use std::sync::{Arc, Mutex};

use openkache_protocol::Opcode;

use super::operation_api::{OperationCatalog, ServerOperationRegistration};
use super::{
    NamespaceRegistry, NetworkWorkerCache, ObservabilityState,
    operation_capabilities::{CapabilityCatalog, CapabilityRegistry},
    operation_compatibility_bindings as compatibility, operation_generic_bindings as generic,
};

/// Composition-root operation catalog assembled from API-owned modules.
pub(super) const SERVER_OPERATIONS: OperationCatalog = OperationCatalog::new()
    .register_module(generic::API)
    .register_module(compatibility::API);

pub(super) fn server_operation(opcode: Opcode) -> Option<&'static ServerOperationRegistration> {
    SERVER_OPERATIONS.get(opcode)
}

pub(super) fn registered_operations() -> impl Iterator<Item = &'static ServerOperationRegistration>
{
    SERVER_OPERATIONS.iter()
}

/// Installs the capabilities owned by the currently registered API modules.
///
/// This is deliberately the only server composition function that knows which
/// concrete runtime handles are needed by the compatibility adapters. The network
/// loop passes opaque capability state onward and remains independent of
/// operation names, wire layouts, and client projections.
pub(super) fn install_runtime_capabilities(
    base: Arc<dyn CapabilityCatalog>,
    cache: Arc<NetworkWorkerCache>,
    namespaces: Arc<Mutex<NamespaceRegistry>>,
    observability: Arc<ObservabilityState>,
) -> Arc<dyn CapabilityCatalog> {
    let storage_port: super::storage_port::StoragePortHandle = cache.clone();
    let mut registry = CapabilityRegistry::overlay(base);
    generic::install_storage_port(&mut registry, storage_port);
    generic::install_resource_store(&mut registry);
    compatibility::install_compatibility_services(
        &mut registry,
        cache,
        namespaces,
        observability,
    );
    Arc::new(registry)
}

/// Validates the complete server composition in one place.
///
/// The network server should only decide whether the operation catalog is
/// usable; it should not know that one module happens to be a protocol-v1
/// compatibility projection. Keeping the compatibility-route check beside the
/// registration catalog makes adding another adapter a local composition
/// change.
pub(super) fn validate() -> Result<(), &'static str> {
    super::operation_handlers::validate_handler_registry()
}
