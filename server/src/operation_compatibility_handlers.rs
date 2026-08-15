//! Typed compatibility handlers connecting decoded input to API behavior.

use super::operation_compatibility_behavior as behavior;
use super::operation_compatibility_decode as decode;
use super::operation_compatibility_services::{
    DeleteState, GetState, NamespaceDeleteState, NamespaceOpenState, NamespaceUpdateState,
    SetState, StatsState, SyncState,
};
use super::operation_contract::OperationStatus;
use super::operation_handlers::OperationContext;
use super::operation_outcome::{OperationError, OperationOutcome};
use super::operation_registry::OperationFuture;

fn invalid_input<'a>(message: &'static [u8]) -> OperationFuture<'a> {
    OperationFuture::ready(OperationOutcome::invalid_request(message))
}

fn missing_module_state<'a>() -> OperationFuture<'a> {
    OperationFuture::ready(OperationOutcome::error(OperationError::status(
        OperationStatus::InternalError,
        b"compatibility module state is unavailable",
    )))
}

/// Builds a typed API handler from a generated field decoder and an API-owned
/// behavior function.
macro_rules! typed_handler {
    ($name:ident, $state:ty, mut $decode:ident, $behavior:path; $($port:ident),+) => {
        pub(super) fn $name<'a>(context: OperationContext<'a>) -> OperationFuture<'a> {
            let Some(state) = context.state::<$state>() else {
                return missing_module_state();
            };
            let OperationContext { mut input, .. } = context;
            let decoded = match decode::$decode(&mut input) {
                Ok(input) => input,
                Err(message) => return invalid_input(message),
            };
            OperationFuture::pending($behavior($(state.$port.as_ref(),)+ decoded))
        }
    };
    ($name:ident, $state:ty, $decode:ident, $behavior:path; $($port:ident),+) => {
        pub(super) fn $name<'a>(context: OperationContext<'a>) -> OperationFuture<'a> {
            let Some(state) = context.state::<$state>() else {
                return missing_module_state();
            };
            let OperationContext { input, .. } = context;
            let decoded = match decode::$decode(&input) {
                Ok(input) => input,
                Err(message) => return invalid_input(message),
            };
            OperationFuture::pending($behavior($(state.$port.as_ref(),)+ decoded))
        }
    };
}

typed_handler!(
    get_handler,
    GetState,
    decode_get,
    behavior::get;
    storage,
    namespaces
);
typed_handler!(
    namespace_open_handler,
    NamespaceOpenState,
    mut decode_namespace_open,
    behavior::namespace_open;
    namespaces
);
typed_handler!(
    namespace_update_policy_handler,
    NamespaceUpdateState,
    decode_namespace_revision,
    behavior::namespace_update_policy;
    namespaces
);
typed_handler!(
    namespace_delete_handler,
    NamespaceDeleteState,
    decode_namespace_delete,
    behavior::namespace_delete;
    storage,
    namespaces
);
typed_handler!(
    set_handler,
    SetState,
    mut decode_set,
    behavior::set;
    storage,
    namespaces
);
typed_handler!(
    delete_handler,
    DeleteState,
    decode_delete,
    behavior::delete;
    storage,
    namespaces
);
typed_handler!(
    stats_handler,
    StatsState,
    decode_stats,
    behavior::stats;
    storage,
    namespaces,
    observability
);
typed_handler!(
    sync_handler,
    SyncState,
    decode_sync,
    behavior::sync;
    storage,
    namespaces
);
