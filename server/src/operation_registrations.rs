//! API module composition.
//!
//! API-owned modules contribute behavior and capabilities. Runtime frame
//! admission uses the generated wire layout directly and reaches this
//! composition only after projection.

use std::sync::{Arc, Mutex};

use openkache_protocol::Opcode;

use super::operation_composition::ServerComposition;
use super::operation_contract::{self as contract, OperationId, operation_id_for_opcode};
use super::operation_execution_state::OperationRuntime;
use super::operation_registration::ServerOperationRegistration;
use super::{
    NamespaceRegistry, NetworkWorkerCache, ObservabilityState,
    operation_capabilities::{CapabilityCatalog, CapabilityEntry, CapabilityList},
    operation_compatibility_module as compatibility, operation_generic_bindings as generic,
    operation_ports::{
        NAMESPACE_CATALOG_PORT, NAMESPACE_COORDINATION_PORT, NAMESPACE_MEMBERSHIP_PORT,
        NamespaceCatalogCapabilityHandle, NamespaceCoordinationCapabilityHandle,
        NamespaceMembershipCapabilityHandle, OBSERVABILITY_PORT, ObservabilityCapabilityHandle,
    },
};

/// Composition-root catalogs assembled from API-owned modules.
pub(super) const SERVER_COMPOSITION: ServerComposition = ServerComposition::new()
    .register_module(generic::API)
    .register_module(compatibility::API);

pub(super) fn server_operation(opcode: Opcode) -> Option<&'static ServerOperationRegistration> {
    SERVER_COMPOSITION.operation(operation_id_for_opcode(opcode))
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
    let storage_port = super::storage_port::StoragePort::new(Arc::clone(&cache));
    let coordination_registry = Arc::clone(&namespaces);
    let catalog_registry = Arc::clone(&namespaces);
    let namespace_coordination_port: NamespaceCoordinationCapabilityHandle = coordination_registry;
    let namespace_catalog_port: NamespaceCatalogCapabilityHandle = catalog_registry;
    let namespace_membership_port: NamespaceMembershipCapabilityHandle = namespaces;
    let observability_port: ObservabilityCapabilityHandle = observability;
    let bootstrap_entries = [
        CapabilityEntry::new(super::storage_port::STORAGE_PORT, &storage_port),
        CapabilityEntry::new(NAMESPACE_COORDINATION_PORT, &namespace_coordination_port),
        CapabilityEntry::new(NAMESPACE_CATALOG_PORT, &namespace_catalog_port),
        CapabilityEntry::new(NAMESPACE_MEMBERSHIP_PORT, &namespace_membership_port),
        CapabilityEntry::new(OBSERVABILITY_PORT, &observability_port),
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
    let mut seen = [false; OperationId::COUNT];
    for registration in registered_operations() {
        let index = registration.operation_id.index();
        if seen[index] {
            return Err("server operation policy registry contains a duplicate operation ID");
        }
        seen[index] = true;
    }
    for entry in contract::operation_registry() {
        let Some(_registration) = server_operation(entry.opcode) else {
            return Err("modeled operation has no server registration");
        };
        let wire = entry.wire;
        if wire.request.fields.len() > contract::MAX_OPERATION_REQUEST_FIELDS {
            return Err("modeled operation request plan exceeds generated bounds");
        }
        if matches!(
            wire.response.framing,
            contract::OperationLayoutFraming::OptionalValues
                | contract::OperationLayoutFraming::FieldSequence
        ) && wire.response.fields.is_empty()
        {
            return Err("ordered response operation has no generated fields");
        }
    }

    super::operation_codecs::validate_contract_codecs()?;
    for entry in contract::operation_registry() {
        if server_operation(entry.opcode).is_none() {
            return Err("modeled operation has no registered server handler");
        }
    }
    Ok(())
}
