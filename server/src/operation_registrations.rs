//! API module composition.
//!
//! API-owned modules contribute behavior and capabilities. Runtime frame
//! admission uses the generated wire layout directly and reaches this
//! composition only after projection.

use std::sync::{Arc, Mutex};

use openkache_protocol::Opcode;

use super::operation_api::{ServerComposition, ServerOperationRegistration};
use super::operation_execution_state::OperationRuntime;
use super::{
    NamespaceRegistry, NetworkWorkerCache, ObservabilityState,
    operation_capabilities::{CapabilityCatalog, CapabilityEntry, CapabilityList},
    operation_compatibility_module as compatibility, operation_generic_bindings as generic,
    operation_compatibility_services::{
        COMPATIBILITY_NAMESPACE_PORT, COMPATIBILITY_OBSERVABILITY_PORT,
        COMPATIBILITY_STORAGE_PORT, NamespaceCapabilityHandle, ObservabilityCapabilityHandle,
        StorageCapabilityHandle,
    },
};

/// Composition-root catalogs assembled from API-owned modules.
pub(super) const SERVER_COMPOSITION: ServerComposition = ServerComposition::new()
    .register_module(generic::API)
    .register_module(compatibility::API);

pub(super) fn server_operation(opcode: Opcode) -> Option<&'static ServerOperationRegistration> {
    SERVER_COMPOSITION.operation(opcode)
}

pub(super) fn registered_operations() -> impl Iterator<Item = &'static ServerOperationRegistration>
{
    SERVER_COMPOSITION.operations()
}

/// Installs the capabilities owned by the currently registered API modules.
///
/// This is deliberately the only server composition function that knows which
/// concrete runtime handles are available to API installers. Request execution
/// receives only the resulting dense operation runtime.
pub(super) fn build_operation_runtime(
    base: &dyn CapabilityCatalog,
    cache: Arc<NetworkWorkerCache>,
    namespaces: Arc<Mutex<NamespaceRegistry>>,
    observability: Arc<ObservabilityState>,
) -> Result<Arc<OperationRuntime>, &'static str> {
    let storage_port: super::storage_port::StoragePortHandle = cache.clone();
    let compatibility_storage: StorageCapabilityHandle = cache;
    let compatibility_namespaces: NamespaceCapabilityHandle = namespaces;
    let compatibility_observability: ObservabilityCapabilityHandle = observability;
    let bootstrap_entries = [
        CapabilityEntry::new(super::storage_port::STORAGE_PORT, &storage_port),
        CapabilityEntry::new(COMPATIBILITY_STORAGE_PORT, &compatibility_storage),
        CapabilityEntry::new(
            COMPATIBILITY_NAMESPACE_PORT,
            &compatibility_namespaces,
        ),
        CapabilityEntry::new(
            COMPATIBILITY_OBSERVABILITY_PORT,
            &compatibility_observability,
        ),
    ];
    let bootstrap = CapabilityList::overlay(base, &bootstrap_entries);
    let runtime = SERVER_COMPOSITION.initialize_modules(&bootstrap)?;
    Ok(Arc::new(runtime))
}

/// Validates the complete server composition in one place.
///
/// The network server only decides whether every modeled operation has a
/// usable behavior registration and codec binding.
pub(super) fn validate() -> Result<(), &'static str> {
    super::operation_handlers::validate_handler_registry()
}
