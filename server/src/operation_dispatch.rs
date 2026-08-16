//! Composition-root dispatch for modeled operations.
//!
//! The network loop owns stream lifetime, timeout policy, and response writes.
//! This module owns the operation-specific dispatch boundary between those
//! concerns: generated decoding, API preparation, capability authorization,
//! resource acquisition, and transport-neutral outcome projection.
//!
//! Keeping this code outside `server.rs` prevents the server lifecycle from
//! becoming a second operation registry. Adding an API changes its generated
//! contract and API-owned registration/binding, not the network loop.

use openkache_protocol::{Opcode, RequestFrameHeader};
use smallvec::SmallVec;

use super::operation_authorization::AuthorizationContext;
use super::operation_execution_state::OperationRuntime;
use super::operation_handlers;
use super::operation_preparation;
use super::operation_registration::OperationCommitDisposition;
use super::operation_registry::OperationTaskStorage;
use super::operation_transport;

pub(super) struct HeaderAdmissionRejection {
    opcode: Opcode,
    response: operation_transport::OperationResponse,
    elapsed: std::time::Duration,
}

impl HeaderAdmissionRejection {
    pub(super) const fn opcode(&self) -> Opcode {
        self.opcode
    }

    pub(super) const fn status(&self) -> openkache_protocol::Status {
        self.response.status()
    }

    pub(super) const fn elapsed(&self) -> std::time::Duration {
        self.elapsed
    }

    pub(super) fn into_response(self) -> operation_transport::OperationResponse {
        self.response
    }
}

/// Runs an API-owned request-header admission hook before the transport reads
/// the declared body.
///
/// The dense registration lookup is the only operation selection here.
/// Generated numeric body-field identity keeps the dispatcher independent of
/// field roles, storage ceilings, and compatibility API names.
pub(super) fn admit_request_header(
    header: RequestFrameHeader,
    prefix: &[u8],
    runtime: &OperationRuntime,
) -> Result<(), HeaderAdmissionRejection> {
    let Some((registration, state)) = runtime.operation_for_opcode(header.opcode()) else {
        return Ok(());
    };
    let Some(admit) = registration.admit_header else {
        return Ok(());
    };
    let view = operation_preparation::OperationHeaderView::new(
        header.body_len(),
        header.body_field(),
        prefix,
    );
    let started = std::time::Instant::now();
    admit(
        &view,
        operation_preparation::HeaderAdmissionContext {
            state,
        },
    )
    .map_err(|error| {
        let response = operation_transport::contract_error_response_status(
            header.opcode(), error.status, error.message,
        );
        HeaderAdmissionRejection {
            opcode: header.opcode(),
            response,
            elapsed: started.elapsed(),
        }
    })
}

/// Returns whether a request may have crossed a mutation commit point.
///
/// The stream loop uses this only to decide whether a timeout may safely
/// produce an error response. Mutation policy remains part of the API
/// registration and never becomes a network-loop opcode match.
pub(super) fn may_mutate(runtime: &OperationRuntime, opcode: Opcode) -> bool {
    runtime
        .registration_for_opcode(opcode)
        .is_some_and(|registration| {
        matches!(
            registration.policy,
            OperationCommitDisposition::MayBeCommitted
        )
        })
}

/// Builds the timeout response through the operation's generated status
/// contract. The network loop does not assume every future API uses the
/// historical `timeout` token.
pub(super) fn timeout_response(
    opcode: Opcode,
    message: &'static [u8],
) -> operation_transport::OperationResponse {
    operation_transport::contract_error_response_status(
        opcode,
        super::operation_contract::OperationStatus::Timeout,
        message,
    )
}

/// Returns the generated response-memory reservation for one operation.
///
/// The stream loop uses this opaque budget before dispatch. Wire payload
/// bounds remain owned by the generated contract adapter rather than being
/// re-derived by the network server.
pub(super) fn response_budget_bytes(runtime: &OperationRuntime, opcode: Opcode) -> Option<usize> {
    runtime.registration_for_opcode(opcode).and_then(|_| {
        let budget = operation_transport::response_budget(opcode);
        (budget > 0).then_some(budget)
    })
}

/// Executes a decoded request through the generic operation boundary.
///
/// The caller only needs a response or an abandoned mutation. All protocol
/// status selection and response framing stay in `operation_transport`.
pub(super) async fn execute_request(
    input: operation_handlers::OperationInputView,
    authorization: &AuthorizationContext,
    runtime: &OperationRuntime,
    task_storage: &mut OperationTaskStorage,
) -> Option<operation_transport::OperationResponse> {
    let opcode = input.opcode();
    let Some((registration, state)) = runtime.operation_for_opcode(opcode) else {
        return Some(operation_transport::contract_error_response_status(
            opcode,
            super::operation_contract::OperationStatus::UnsupportedOpcode,
            b"modeled operation has no server registration",
        ));
    };
    if !(registration.authorization)(authorization) {
        return Some(operation_transport::contract_error_response_status(
            opcode,
            super::operation_contract::OperationStatus::Forbidden,
            b"operation authorization capability is not satisfied",
        ));
    }

    if let Err(message) = input.validate_codecs() {
        return Some(operation_transport::contract_error_response_status(
            opcode,
            super::operation_contract::OperationStatus::InvalidRequest,
            message,
        ));
    }

    let preparation = match (registration.prepare)(
        &input,
        operation_preparation::PrepareContext {
            state,
        },
    ) {
        Ok(preparation) => preparation,
        Err(error) => {
            return Some(operation_transport::contract_error_response_status(
                opcode,
                error.status,
                error.message,
            ));
        }
    };

    // Preparation resolves opaque lock handles. The dispatcher acquires them
    // in the generated deterministic order and never interprets resource
    // identity.
    let mut _resource_guards = SmallVec::<[_; 8]>::new();
    for resource in preparation.resources() {
        _resource_guards.push(resource.lock().lock().await);
    }
    for resource in preparation.resources() {
        if let Some(error) = resource.inactive_error() {
            return Some(operation_transport::contract_error_response_status(
                opcode,
                error.status,
                error.message,
            ));
        }
    }

    let outcome = (registration.handler)(
        operation_handlers::OperationContext {
            state,
            input,
        },
        task_storage,
    )
    .await;
    operation_transport::encode_operation_outcome(opcode, outcome)
}
