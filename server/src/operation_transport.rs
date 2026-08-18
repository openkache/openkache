//! Protocol response projection for transport-neutral operation outcomes.
//!
//! Operation behavior and dispatch live in `operation_handlers`; this module
//! is the only operation-layer code that turns an outcome into the protocol
//! `Response`. Keeping this adapter separate prevents API behavior from
//! acquiring wire status or framing branches.

use openkache_protocol::{Opcode, ProtocolError, Response, ResponseParts, ResponseSegment, Status};
use smallvec::SmallVec;

use super::operation_contract::OperationStatus;
use super::operation_outcome::{
    OperationBody, OperationError, OperationOutcome, OperationSuccessStatus, OperationValue,
};
use crate::operation_contract as contract;

/// One fully framed operation response ready for ownership-preserving writes.
pub(super) struct OperationResponse {
    status: Status,
    parts: ResponseParts,
}

impl OperationResponse {
    pub(super) const fn status(&self) -> Status {
        self.status
    }

    pub(super) fn into_parts(self) -> ResponseParts {
        self.parts
    }
}

impl From<Response> for OperationResponse {
    fn from(response: Response) -> Self {
        let status = response.status;
        let parts = response
            .into_parts()
            .expect("validated response remains within the protocol limit");
        Self { status, parts }
    }
}

/// Returns the generated response admission budget for one operation.
///
/// Generated wire metadata is consumed by the transport adapter. API
/// registration only receives this opaque budget and does not import the wire
/// contract directly.
pub(super) const fn response_budget(opcode: Opcode) -> usize {
    contract::response_budget(opcode)
}

/// Resolves a generated response budget at the explicit wire-adapter
/// boundary. Runtime dispatch remains keyed by the neutral operation ID.
pub(super) const fn response_budget_for_operation(operation_id: contract::OperationId) -> usize {
    response_budget(contract::opcode_for_operation_id(operation_id))
}

/// Selects an error status that the generated operation contract permits.
///
/// A generic adapter may discover a malformed domain result or a denied
/// capability before an API-owned handler runs. Returning a hard-coded status
/// here would make a future operation violate its own contract.
pub(super) fn contract_error_status(opcode: Opcode, preferred: Status) -> Status {
    let wire = contract::spec(opcode);
    wire.error_statuses
        .iter()
        .copied()
        .find(|status| *status == preferred)
        .or_else(|| {
            wire.error_statuses
                .iter()
                .copied()
                .find(|status| *status == Status::InternalError)
        })
        .or_else(|| wire.error_statuses.first().copied())
        .unwrap_or(Status::InternalError)
}

/// Builds a contract-valid error response for a generated operation.
pub(super) fn contract_error_response(
    opcode: Opcode,
    preferred: Status,
    message: &[u8],
) -> OperationResponse {
    let status = contract_error_status(opcode, preferred);
    let parts = ResponseParts::segmented(
        status,
        [operation_value_segment(OperationValue::inline(message))],
    )
    .unwrap_or_else(|_| {
        // Error diagnostics are API-owned bytes. A malformed adapter must not
        // turn an oversized diagnostic into a server panic; retain the
        // contract-valid status and send a bounded generic diagnostic instead.
        ResponseParts::segmented(
            status,
            [operation_value_segment(OperationValue::inline(
                b"operation error exceeds the protocol limit",
            ))],
        )
        .expect("bounded error response remains within the protocol limit")
    });
    OperationResponse { status, parts }
}

/// Builds a wire error response for a neutral operation selected by runtime
/// dispatch. This is the only conversion needed by callers that already have
/// a protocol status rather than a generated semantic status.
pub(super) fn contract_error_response_for_operation(
    operation_id: contract::OperationId,
    preferred: Status,
    message: &[u8],
) -> OperationResponse {
    contract_error_response(
        contract::opcode_for_operation_id(operation_id),
        preferred,
        message,
    )
}

/// Builds a contract-valid error response from an API-owned semantic status.
pub(super) fn contract_error_response_status(
    opcode: Opcode,
    preferred: OperationStatus,
    message: &[u8],
) -> OperationResponse {
    contract_error_response(opcode, preferred.wire_status(), message)
}

/// Builds a semantic error response after the neutral runtime has selected an
/// operation. Wire status and framing remain private to this adapter.
pub(super) fn contract_error_response_status_for_operation(
    operation_id: contract::OperationId,
    preferred: OperationStatus,
    message: &[u8],
) -> OperationResponse {
    contract_error_response_status(
        contract::opcode_for_operation_id(operation_id),
        preferred,
        message,
    )
}

/// Encodes a generated ordered-field response without consulting a semantic
/// route name. Compatibility framing is delegated to its adapter, while this
/// function owns the generic presence-mask representation.
pub(super) fn operation_fields_response(
    opcode: Opcode,
    status: Status,
    values: SmallVec<[Option<OperationValue>; 8]>,
) -> OperationResponse {
    planned_fields_response(opcode, status, values)
}

/// Encodes any descriptor-planned field response.
///
/// The layout and field codecs come from the generated operation descriptor.
/// Both the presence-mask sequence and the explicit fixed-width optional-value table
/// use this same boundary; operation behavior never chooses a layout by
/// operation name.
fn planned_fields_response(
    opcode: Opcode,
    status: Status,
    values: SmallVec<[Option<OperationValue>; 8]>,
) -> OperationResponse {
    let wire = contract::spec(opcode);
    if values.len() > contract::MAX_FIELDS
        || !matches!(
            wire.response.framing,
            contract::OperationLayoutFraming::FieldSequence
                | contract::OperationLayoutFraming::OptionalValues
        )
        || values.len() != wire.response.fields.len()
    {
        return contract_error_response(
            opcode,
            Status::InternalError,
            b"operation response framing does not match its generated plan",
        );
    }
    if let Err(message) = validate_response_fields(&values, wire.response.fields) {
        return contract_error_response(opcode, Status::InternalError, message);
    }
    let parts = match ResponseParts::planned_fields(
        status,
        values
            .into_iter()
            .map(|value| value.map(operation_value_segment)),
        wire.response,
    ) {
        Ok(parts) => parts,
        Err(error) => {
            let (status, message) = match error {
                ProtocolError::ValueTooLarge { .. } | ProtocolError::FrameLengthOverflow => (
                    Status::TooLarge,
                    b"operation response fields exceed the protocol limit".as_slice(),
                ),
                _ => (
                    Status::InternalError,
                    b"operation response fields do not match their generated layout".as_slice(),
                ),
            };
            return contract_error_response(opcode, status, message);
        }
    };
    OperationResponse { status, parts }
}

fn operation_value_segment(value: OperationValue) -> ResponseSegment {
    value.into_segment()
}

pub(super) fn validate_response_fields<T: AsRef<[u8]>>(
    values: &[Option<T>],
    plan: &'static [contract::OperationFieldPlan],
) -> Result<(), &'static [u8]> {
    if values.len() != plan.len() {
        return Err(b"operation response fields do not match the generated plan");
    }
    if values
        .iter()
        .zip(plan)
        .any(|(value, field)| field.required && value.is_none())
    {
        return Err(b"required operation response field is missing");
    }
    for (value, field) in values.iter().zip(plan) {
        let Some(value) = value.as_ref() else {
            continue;
        };
        if super::operation_fields::validate_field_bytes(field, value.as_ref()).is_err() {
            return Err(b"operation response field does not satisfy its generated codec");
        }
    }
    Ok(())
}

/// Converts a transport-neutral operation outcome through the generated
/// status and response framing contract. An abandoned outcome deliberately
/// produces no response when a mutation's commit state is unknowable.
pub(super) fn encode_operation_outcome(
    opcode: openkache_protocol::Opcode,
    outcome: OperationOutcome,
) -> Option<OperationResponse> {
    match outcome {
        OperationOutcome::Success {
            status,
            body: payload,
        } => {
            let Some(status) = operation_success_status(opcode, status) else {
                return Some(contract_error_response(
                    opcode,
                    Status::InternalError,
                    b"operation returned a success status outside its contract",
                ));
            };
            match payload {
                OperationBody::Empty => {
                    let wire = contract::spec(opcode);
                    if wire.response.framing != contract::OperationLayoutFraming::Empty {
                        return Some(contract_error_response(
                            opcode,
                            Status::InternalError,
                            b"empty operation payload does not match its response framing",
                        ));
                    }
                    Some(operation_response(
                        opcode,
                        status,
                        OperationValue::inline(b""),
                    ))
                }
                OperationBody::Opaque(value) => {
                    // Size admission is checked before codec validation. A
                    // payload that exceeds the generated response budget must
                    // always produce the contract's TooLarge response, even
                    // when its bytes would also fail an application codec
                    // (for example, an oversized invalid UTF-8 payload).
                    if value.len() > response_budget(opcode) {
                        return Some(contract_error_response(
                            opcode,
                            Status::TooLarge,
                            b"operation response exceeds the protocol limit",
                        ));
                    }
                    if !valid_opaque_response(opcode, value.as_ref()) {
                        return Some(contract_error_response(
                            opcode,
                            Status::InternalError,
                            b"opaque operation payload does not match its response framing",
                        ));
                    }
                    Some(operation_response(opcode, status, value))
                }
                OperationBody::Fields(values) => {
                    Some(operation_fields_response(opcode, status, values))
                }
            }
        }
        OperationOutcome::Error(error) => operation_error_response(opcode, error),
        OperationOutcome::Abandoned => None,
    }
}

/// Projects a neutral operation outcome at the one wire response boundary.
pub(super) fn encode_operation_outcome_for_operation(
    operation_id: contract::OperationId,
    outcome: OperationOutcome,
) -> Option<OperationResponse> {
    encode_operation_outcome(contract::opcode_for_operation_id(operation_id), outcome)
}

fn valid_opaque_response(opcode: openkache_protocol::Opcode, value: &[u8]) -> bool {
    let wire = contract::spec(opcode);
    if wire.response.framing != contract::OperationLayoutFraming::Opaque {
        return false;
    }
    // A composite opaque payload is valid only when the model explicitly marks
    // it as an adapter-owned aggregate. Generic composite opaque operations
    // fail generation and can never bypass codec validation accidentally.
    let Some(field) = wire.response.fields.first() else {
        return wire.response.opaque_aggregate;
    };
    if wire.response.fields.len() != 1 {
        return wire.response.opaque_aggregate;
    }
    super::operation_fields::validate_field_bytes(field, value).is_ok()
}

fn operation_success_status(
    opcode: openkache_protocol::Opcode,
    status: OperationSuccessStatus,
) -> Option<Status> {
    let status = status.wire_status();
    if contract::spec(opcode).success_statuses.contains(&status) {
        Some(status)
    } else {
        None
    }
}

/// Maps a transport-neutral operation failure through the generated status
/// contract. A behavior cannot accidentally return a status that its Smithy
/// operation did not declare.
fn operation_error_response(
    opcode: openkache_protocol::Opcode,
    error: OperationError,
) -> Option<OperationResponse> {
    let contract = contract::spec(opcode);
    match error {
        OperationError::InvalidRequest(message) => operation_status_response(
            opcode,
            contract,
            OperationStatus::InvalidRequest,
            OperationValue::inline(message),
        ),
        OperationError::Status { status, message } => {
            operation_status_response(opcode, contract, status, OperationValue::inline(message))
        }
        OperationError::OwnedStatus { status, message } => {
            operation_status_response(opcode, contract, status, OperationValue::from(message))
        }
    }
}

fn operation_status_response(
    opcode: openkache_protocol::Opcode,
    contract: contract::OperationWireSpec,
    requested: OperationStatus,
    message: impl Into<OperationValue>,
) -> Option<OperationResponse> {
    let requested = requested.wire_status();
    let status = Some(requested)
        .filter(|status| contract.error_statuses.contains(status))
        .or_else(|| {
            contract
                .error_statuses
                .iter()
                .copied()
                .find(|candidate| *candidate == Status::InternalError)
        })
        .or_else(|| contract.error_statuses.first().copied())
        .unwrap_or(Status::InternalError);
    Some(operation_response(opcode, status, message))
}

/// Encodes one operation response without allowing an API-owned payload to
/// panic the server when it exceeds the protocol frame limit.
fn operation_response(
    opcode: openkache_protocol::Opcode,
    status: Status,
    payload: impl Into<OperationValue>,
) -> OperationResponse {
    match ResponseParts::segmented(status, [operation_value_segment(payload.into())]) {
        Ok(parts) => OperationResponse { status, parts },
        Err(_) => contract_error_response(
            opcode,
            Status::TooLarge,
            b"operation response exceeds the protocol limit",
        ),
    }
}
