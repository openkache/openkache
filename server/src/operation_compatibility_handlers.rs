//! Typed compatibility handlers connecting decoded input to API behavior.

use super::operation_compatibility_behavior as behavior;
use super::operation_compatibility_decode as decode;
use super::operation_compatibility_services::{
    DeleteState, GetState, SetState, StatsState, SyncState,
};
use super::operation_contract::OperationStatus;
use super::operation_handlers::OperationContext;
use super::operation_outcome::{OperationError, OperationOutcome};
use super::operation_registry::{OperationFuture, OperationTaskStorage};

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
    ($name:ident, $state:ty, mut $decode:ident, $behavior:path) => {
        pub(super) fn $name<'a>(
            context: OperationContext<'a>,
            task_storage: &'a mut OperationTaskStorage,
        ) -> OperationFuture<'a> {
            let Some(state) = context.state::<$state>() else {
                return missing_module_state();
            };
            let OperationContext { mut input, .. } = context;
            let decoded = match decode::$decode(&mut input) {
                Ok(input) => input,
                Err(message) => return invalid_input(message),
            };
            OperationFuture::pending(task_storage, $behavior(state, decoded))
        }
    };
    ($name:ident, $state:ty, $decode:ident, $behavior:path) => {
        pub(super) fn $name<'a>(
            context: OperationContext<'a>,
            task_storage: &'a mut OperationTaskStorage,
        ) -> OperationFuture<'a> {
            let Some(state) = context.state::<$state>() else {
                return missing_module_state();
            };
            let OperationContext { input, .. } = context;
            let decoded = match decode::$decode(&input) {
                Ok(input) => input,
                Err(message) => return invalid_input(message),
            };
            OperationFuture::pending(task_storage, $behavior(state, decoded))
        }
    };
}

typed_handler!(get_handler, GetState, decode_get, behavior::get);
typed_handler!(set_handler, SetState, mut decode_set, behavior::set);
typed_handler!(delete_handler, DeleteState, decode_delete, behavior::delete);
typed_handler!(stats_handler, StatsState, decode_stats, behavior::stats);
typed_handler!(sync_handler, SyncState, decode_sync, behavior::sync);
