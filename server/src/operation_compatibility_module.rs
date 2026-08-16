//! Compatibility API registration and startup state installation.
//!
//! Request decoding and behavior remain in their API-owned modules. This
//! module contributes the compatibility operations to the generic server
//! composition and resolves their exact capability state once at startup.

use std::sync::Arc;

use openkache_protocol::Opcode;

use super::operation_authorization::{authorization_administrator, authorization_none};
use super::operation_capabilities::{CapabilityCatalog, downcast_capability};
use super::operation_compatibility_handlers as handlers;
use super::operation_compatibility_prepare as prepare;
use super::operation_compatibility_services::{
    COMPATIBILITY_NAMESPACE_PORT, COMPATIBILITY_OBSERVABILITY_PORT, DeleteState, GetState,
    NamespaceDeleteState, NamespaceOpenState, NamespaceUpdateState, SetState, StatsState,
    SyncState,
};
use super::operation_composition::ApiModule;
use super::operation_execution_state::OperationStateBindings;
use super::operation_registration::{RegistrationBuilder, ServerOperationRegistration};
use super::storage_port::{STORAGE_PORT, StorageDataPort};

fn initialize_module(
    states: &mut OperationStateBindings<'_>,
    bootstrap: &dyn CapabilityCatalog,
) -> Result<(), &'static str> {
    let storage = downcast_capability(bootstrap, STORAGE_PORT)
        .ok_or("compatibility storage port is unavailable")?;
    let namespaces = downcast_capability(bootstrap, COMPATIBILITY_NAMESPACE_PORT)
        .ok_or("compatibility namespace port is unavailable")?;
    let observability = downcast_capability(bootstrap, COMPATIBILITY_OBSERVABILITY_PORT)
        .ok_or("compatibility observability port is unavailable")?;
    let max_item_bytes = storage.max_item_bytes();

    states.bind(
        Opcode::Get,
        Arc::new(GetState {
            storage: storage.clone(),
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        Opcode::Set,
        Arc::new(SetState {
            storage: storage.clone(),
            namespaces: Arc::clone(namespaces),
            max_item_bytes,
        }),
    )?;
    states.bind(
        Opcode::Delete,
        Arc::new(DeleteState {
            storage: storage.clone(),
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        Opcode::Stats,
        Arc::new(StatsState {
            storage: storage.clone(),
            namespaces: Arc::clone(namespaces),
            observability: Arc::clone(observability),
        }),
    )?;
    states.bind(
        Opcode::Sync,
        Arc::new(SyncState {
            storage: storage.clone(),
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
            storage: storage.clone(),
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    Ok(())
}

const OPERATIONS: &[ServerOperationRegistration] = &[
    RegistrationBuilder::new(Opcode::Get, handlers::get_handler)
        .state::<GetState>()
        .prepare(prepare::prepare_get_namespace)
        .authorize(authorization_none)
        .read_only()
        .build(),
    RegistrationBuilder::new(Opcode::Set, handlers::set_handler)
        .state::<SetState>()
        .admit_header(prepare::admit_set_header)
        .prepare(prepare::prepare_set)
        .authorize(authorization_none)
        .mutation()
        .build(),
    RegistrationBuilder::new(Opcode::Delete, handlers::delete_handler)
        .state::<DeleteState>()
        .prepare(prepare::prepare_delete_namespace)
        .authorize(authorization_none)
        .mutation()
        .build(),
    RegistrationBuilder::new(Opcode::Stats, handlers::stats_handler)
        .state::<StatsState>()
        .prepare(prepare::prepare_stats_namespace)
        .authorize(authorization_administrator)
        .read_only()
        .build(),
    RegistrationBuilder::new(Opcode::Sync, handlers::sync_handler)
        .state::<SyncState>()
        .prepare(prepare::prepare_sync_namespace)
        .authorize(authorization_administrator)
        .mutation()
        .build(),
    RegistrationBuilder::new(Opcode::NamespaceOpen, handlers::namespace_open_handler)
        .state::<NamespaceOpenState>()
        .prepare(prepare::prepare_namespace_open)
        .authorize(authorization_none)
        .mutation()
        .build(),
    RegistrationBuilder::new(
        Opcode::NamespaceUpdatePolicy,
        handlers::namespace_update_policy_handler,
    )
    .state::<NamespaceUpdateState>()
    .prepare(prepare::prepare_namespace_update)
    .authorize(authorization_none)
    .mutation()
    .build(),
    RegistrationBuilder::new(Opcode::NamespaceDelete, handlers::namespace_delete_handler)
        .state::<NamespaceDeleteState>()
        .prepare(prepare::prepare_namespace_delete)
        .authorize(authorization_none)
        .mutation()
        .build(),
];

pub(super) const API: ApiModule =
    ApiModule::new(OPERATIONS).install_operation_state(initialize_module);
