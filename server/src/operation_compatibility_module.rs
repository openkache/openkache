//! Compatibility API registration and startup state installation.
//!
//! Request decoding and behavior remain in their API-owned modules. This
//! module contributes the compatibility operations to the generic server
//! composition and resolves their exact capability state once at startup.

use std::sync::Arc;

use super::operation_authorization::{authorization_administrator, authorization_none};
use super::operation_capabilities::{CapabilityCatalog, downcast_capability};
use super::operation_compatibility_handlers as handlers;
use super::operation_compatibility_prepare as prepare;
use super::operation_compatibility_services::{
    DeleteState, GetState, NamespaceDeleteState, NamespaceOpenState, NamespaceUpdateState,
    SetState, StatsState, SyncState,
};
use super::operation_composition::ApiModule;
use super::operation_contract::OperationId;
use super::operation_execution_state::OperationStateBindings;
use super::operation_ports::{NAMESPACE_PORT, OBSERVABILITY_PORT};
use super::operation_registration::{RegistrationBuilder, ServerOperationRegistration};
use super::storage_port::{STORAGE_PORT, StorageDataPort};

fn initialize_module(
    states: &mut OperationStateBindings<'_>,
    bootstrap: &dyn CapabilityCatalog,
) -> Result<(), &'static str> {
    let storage = downcast_capability(bootstrap, STORAGE_PORT)
        .ok_or("compatibility storage port is unavailable")?;
    let namespaces =
        downcast_capability(bootstrap, NAMESPACE_PORT).ok_or("namespace port is unavailable")?;
    let observability = downcast_capability(bootstrap, OBSERVABILITY_PORT)
        .ok_or("observability port is unavailable")?;
    let max_item_bytes = storage.max_item_bytes();

    states.bind(
        OperationId::Get,
        Arc::new(GetState {
            storage: storage.clone(),
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        OperationId::Set,
        Arc::new(SetState {
            storage: storage.clone(),
            namespaces: Arc::clone(namespaces),
            max_item_bytes,
        }),
    )?;
    states.bind(
        OperationId::Delete,
        Arc::new(DeleteState {
            storage: storage.clone(),
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        OperationId::Stats,
        Arc::new(StatsState {
            storage: storage.clone(),
            namespaces: Arc::clone(namespaces),
            observability: Arc::clone(observability),
        }),
    )?;
    states.bind(
        OperationId::Sync,
        Arc::new(SyncState {
            storage: storage.clone(),
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        OperationId::NamespaceOpen,
        Arc::new(NamespaceOpenState {
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        OperationId::NamespaceUpdatePolicy,
        Arc::new(NamespaceUpdateState {
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    states.bind(
        OperationId::NamespaceDelete,
        Arc::new(NamespaceDeleteState {
            storage: storage.clone(),
            namespaces: Arc::clone(namespaces),
        }),
    )?;
    Ok(())
}

const OPERATIONS: &[ServerOperationRegistration] = &[
    RegistrationBuilder::new(OperationId::Get, handlers::get_handler)
        .state::<GetState>()
        .prepare(prepare::prepare_get_namespace)
        .authorize(authorization_none)
        .read_only()
        .build(),
    RegistrationBuilder::new(OperationId::Set, handlers::set_handler)
        .state::<SetState>()
        .admit_header(prepare::admit_set_header)
        .prepare(prepare::prepare_set)
        .authorize(authorization_none)
        .mutation()
        .build(),
    RegistrationBuilder::new(OperationId::Delete, handlers::delete_handler)
        .state::<DeleteState>()
        .prepare(prepare::prepare_delete_namespace)
        .authorize(authorization_none)
        .mutation()
        .build(),
    RegistrationBuilder::new(OperationId::Stats, handlers::stats_handler)
        .state::<StatsState>()
        .prepare(prepare::prepare_stats_namespace)
        .authorize(authorization_administrator)
        .read_only()
        .build(),
    RegistrationBuilder::new(OperationId::Sync, handlers::sync_handler)
        .state::<SyncState>()
        .prepare(prepare::prepare_sync_namespace)
        .authorize(authorization_administrator)
        .mutation()
        .build(),
    RegistrationBuilder::new(OperationId::NamespaceOpen, handlers::namespace_open_handler)
        .state::<NamespaceOpenState>()
        .prepare(prepare::prepare_namespace_open)
        .authorize(authorization_none)
        .mutation()
        .build(),
    RegistrationBuilder::new(
        OperationId::NamespaceUpdatePolicy,
        handlers::namespace_update_policy_handler,
    )
    .state::<NamespaceUpdateState>()
    .prepare(prepare::prepare_namespace_update)
    .authorize(authorization_none)
    .mutation()
    .build(),
    RegistrationBuilder::new(OperationId::NamespaceDelete, handlers::namespace_delete_handler)
        .state::<NamespaceDeleteState>()
        .prepare(prepare::prepare_namespace_delete)
        .authorize(authorization_none)
        .mutation()
        .build(),
];

pub(super) const API: ApiModule =
    ApiModule::new(OPERATIONS).install_operation_state(initialize_module);
