//! Compatibility API registration and startup state installation.
//!
//! Request decoding and behavior remain in their API-owned modules. This
//! module contributes the compatibility operations to the generic server
//! composition and resolves their exact capability state once at startup.

use std::sync::Arc;

use openkache_protocol::Opcode;

use super::operation_api::{ApiModule, RegistrationBuilder, ServerOperationRegistration};
use super::operation_capabilities::CapabilityCatalog;
use super::operation_compatibility_bindings as bindings;
use super::operation_compatibility_services::{
    COMPATIBILITY_NAMESPACE_PORT, COMPATIBILITY_OBSERVABILITY_PORT, COMPATIBILITY_STORAGE_PORT,
    DeleteState, GetState, NamespaceDeleteState, NamespaceOpenState, NamespaceUpdateState,
    SetState, StatsState, SyncState,
};
use super::operation_execution_state::OperationStateBindings;
use super::operation_handlers;

fn initialize_module(
    states: &mut OperationStateBindings<'_>,
    bootstrap: &dyn CapabilityCatalog,
) -> Result<(), &'static str> {
    let storage = super::operation_api::downcast_capability(bootstrap, COMPATIBILITY_STORAGE_PORT)
        .ok_or("compatibility storage port is unavailable")?;
    let namespaces =
        super::operation_api::downcast_capability(bootstrap, COMPATIBILITY_NAMESPACE_PORT)
            .ok_or("compatibility namespace port is unavailable")?;
    let observability =
        super::operation_api::downcast_capability(bootstrap, COMPATIBILITY_OBSERVABILITY_PORT)
            .ok_or("compatibility observability port is unavailable")?;
    let max_item_bytes = storage.max_item_bytes();

    states.bind(
        Opcode::Get,
        Arc::new(GetState {
            storage: Arc::clone(storage),
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        Opcode::Set,
        Arc::new(SetState {
            storage: Arc::clone(storage),
            namespaces: Arc::clone(namespaces),
            max_item_bytes,
        }),
    )?;
    states.bind(
        Opcode::Delete,
        Arc::new(DeleteState {
            storage: Arc::clone(storage),
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        Opcode::Stats,
        Arc::new(StatsState {
            storage: Arc::clone(storage),
            namespaces: Arc::clone(namespaces),
            observability: Arc::clone(observability),
        }),
    )?;
    states.bind(
        Opcode::Sync,
        Arc::new(SyncState {
            storage: Arc::clone(storage),
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        Opcode::NamespaceOpen,
        Arc::new(NamespaceOpenState {
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        Opcode::NamespaceUpdatePolicy,
        Arc::new(NamespaceUpdateState {
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        Opcode::NamespaceDelete,
        Arc::new(NamespaceDeleteState {
            storage: Arc::clone(storage),
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    Ok(())
}

const OPERATIONS: &[ServerOperationRegistration] = &[
    RegistrationBuilder::new(Opcode::Get, bindings::get_handler)
        .state::<GetState>()
        .prepare(bindings::prepare_get_namespace)
        .authorize(operation_handlers::authorization_none)
        .read_only()
        .build(),
    RegistrationBuilder::new(Opcode::Set, bindings::set_handler)
        .state::<SetState>()
        .admit_header(bindings::admit_set_header)
        .prepare(bindings::prepare_set)
        .authorize(operation_handlers::authorization_none)
        .mutation()
        .build(),
    RegistrationBuilder::new(Opcode::Delete, bindings::delete_handler)
        .state::<DeleteState>()
        .prepare(bindings::prepare_delete_namespace)
        .authorize(operation_handlers::authorization_none)
        .mutation()
        .build(),
    RegistrationBuilder::new(Opcode::Stats, bindings::stats_handler)
        .state::<StatsState>()
        .prepare(bindings::prepare_stats_namespace)
        .authorize(operation_handlers::authorization_administrator)
        .read_only()
        .build(),
    RegistrationBuilder::new(Opcode::Sync, bindings::sync_handler)
        .state::<SyncState>()
        .prepare(bindings::prepare_sync_namespace)
        .authorize(operation_handlers::authorization_administrator)
        .mutation()
        .build(),
    RegistrationBuilder::new(Opcode::NamespaceOpen, bindings::namespace_open_handler)
        .state::<NamespaceOpenState>()
        .prepare(bindings::prepare_namespace_open)
        .authorize(operation_handlers::authorization_none)
        .mutation()
        .build(),
    RegistrationBuilder::new(
        Opcode::NamespaceUpdatePolicy,
        bindings::namespace_update_policy_handler,
    )
    .state::<NamespaceUpdateState>()
    .prepare(bindings::prepare_namespace_update)
    .authorize(operation_handlers::authorization_none)
    .mutation()
    .build(),
    RegistrationBuilder::new(
        Opcode::NamespaceDelete,
        bindings::namespace_delete_handler,
    )
    .state::<NamespaceDeleteState>()
    .prepare(bindings::prepare_namespace_delete)
    .authorize(operation_handlers::authorization_none)
    .mutation()
    .build(),
];

pub(super) const API: ApiModule =
    ApiModule::new(OPERATIONS).install_operation_state(initialize_module);
