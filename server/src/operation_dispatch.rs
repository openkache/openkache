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

use openkache_protocol::Opcode;
use smallvec::SmallVec;

use super::ServerRequest;
use super::operation_api;
use super::operation_handlers;
use super::operation_transport;

/// Returns whether a request may have crossed a mutation commit point.
///
/// The stream loop uses this only to decide whether a timeout may safely
/// produce an error response. Mutation policy remains part of the API
/// registration and never becomes a network-loop opcode match.
pub(super) fn may_mutate(opcode: Opcode) -> bool {
    operation_api::server_operation(opcode).is_some_and(|registration| {
        matches!(
            registration.policy,
            operation_api::OperationCommitDisposition::MayBeCommitted
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

/// Builds the contract-valid response used when the response budget cannot be
/// acquired before dispatch.
pub(super) fn overloaded_response(
    opcode: Opcode,
    message: &'static [u8],
) -> operation_transport::OperationResponse {
    operation_transport::contract_error_response_status(
        opcode,
        super::operation_contract::OperationStatus::Overloaded,
        message,
    )
}

/// Returns the generated response-memory reservation for one operation.
///
/// The stream loop uses this opaque budget before dispatch. Wire payload
/// bounds remain owned by the generated contract adapter rather than being
/// re-derived by the network server.
pub(crate) fn response_budget_bytes(opcode: Opcode) -> Option<usize> {
    operation_api::server_operation(opcode).and_then(|_| {
        let budget = operation_transport::response_budget(opcode);
        (budget > 0).then_some(budget)
    })
}

/// Executes a decoded request through the generic operation boundary.
///
/// The caller only needs a response or an abandoned mutation. All protocol
/// status selection and response framing stay in `operation_transport`.
pub(super) async fn execute_request(
    request: ServerRequest,
    authorization: operation_handlers::AuthorizationContext,
    capabilities: &dyn super::operation_capabilities::CapabilityCatalog,
) -> Option<operation_transport::OperationResponse> {
    let opcode = request.opcode();
    let Some(registration) = operation_api::server_operation(opcode) else {
        return Some(operation_transport::contract_error_response_status(
            opcode,
            super::operation_contract::OperationStatus::UnsupportedOpcode,
            b"modeled operation has no server registration",
        ));
    };
    if !operation_handlers::authorization_allowed(registration, authorization.clone()) {
        return Some(operation_transport::contract_error_response_status(
            opcode,
            super::operation_contract::OperationStatus::Forbidden,
            b"operation authorization capability is not satisfied",
        ));
    }

    // Every modeled request is decoded exactly once into the generated field
    // view. Immediate operations use the same preparation and resource path
    // as asynchronous ones.
    let input = (registration.decode)(request);
    if !input.is_valid() {
        return Some(operation_transport::contract_error_response_status(
            opcode,
            super::operation_contract::OperationStatus::InvalidRequest,
            b"operation field sequence is invalid",
        ));
    }
    if let Err(message) = input.validate_codecs() {
        return Some(operation_transport::contract_error_response_status(
            opcode,
            super::operation_contract::OperationStatus::InvalidRequest,
            message,
        ));
    }

    let preparation =
        match (registration.prepare)(&input, operation_api::PrepareContext { capabilities }) {
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

    operation_transport::execute(capabilities, input, registration.handler).await
}
