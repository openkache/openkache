//! API-owned bindings for generic operations.
//!
//! These bindings deliberately avoid namespace, item, and SET vocabulary.
//! Request and response shapes come from the generated operation contract;
//! only application semantics live here.

use openkache_protocol::Opcode;

use super::operation_api::{self, ApiModule};
use super::operation_contract::OperationStatus;
use super::operation_handlers::OperationContext;
use super::operation_outcome::{OperationOutcome, OperationValue};
use super::operation_registry::OperationFuture;

fn ping_handler<'a>(_context: OperationContext<'a>) -> OperationFuture<'a> {
    OperationFuture::ready(OperationOutcome::opaque(
        OperationStatus::Ok,
        OperationValue::inline(b"PONG"),
    ))
}

pub(super) const API: ApiModule = ApiModule::new(
    crate::protocol::generic_request_descriptor(),
    &[
        operation_api::RegistrationBuilder::new(Opcode::Ping, ping_handler)
            .read_only()
            .build(),
    ],
);
