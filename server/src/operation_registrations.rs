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
    operation_compatibility_services::{
        COMPATIBILITY_RUNTIME, CompatibilityRuntime,
    },
    operation_compatibility_registrations as compatibility,
    operation_generic_registrations as generic,
    storage_port::STORAGE_PORT,
};

/// Composition-root operation catalog assembled from API-owned modules.
pub(super) const API_MODULES: &[super::operation_api::ApiModule] = &[
    generic::API,
    compatibility::API,
];

pub(super) const SERVER_OPERATIONS: OperationCatalog =
    OperationCatalog::new().register_modules(API_MODULES);

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
    let mut source = CapabilityRegistry::overlay(Arc::clone(&base));
    let storage_port: super::storage_port::StoragePortHandle = cache.clone();
    source.insert(STORAGE_PORT, storage_port);
    source.insert(
        COMPATIBILITY_RUNTIME,
        CompatibilityRuntime::new(
            Arc::clone(&cache),
            Arc::clone(&namespaces),
            Arc::clone(&observability),
        ),
    );
    let source: Arc<dyn CapabilityCatalog> = Arc::new(source);
    let mut registry = CapabilityRegistry::overlay(base);
    for module in API_MODULES {
        module.install_capabilities(&mut registry, source.as_ref());
    }
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
