//! Protocol response projection for transport-neutral operation outcomes.
//!
//! Operation behavior and dispatch live in `operation_handlers`; this module
//! is the operation-layer code that turns an outcome into the protocol
//! `Response`. The generated descriptor selects generic response framing and
//! compact field layout for every API.

use openkache_protocol::{
    encode_layout_segments, Opcode, ResponseParts, ResponseSegment, Status,
};
use smallvec::SmallVec;

use super::operation_capabilities::CapabilityCatalog;
use super::operation_handlers::{OperationContext, OperationInputView};
use super::operation_outcome::{
    OperationBody, OperationError, OperationOutcome, OperationValue, StatusToken,
};
use super::operation_registry::OperationHandler;
use crate::operation_contract as contract;

/// One fully framed operation response ready for ownership-preserving writes.
pub(super) struct OperationResponse {
    status: Status,
    parts: ResponseParts,
}

impl OperationResponse {
    pub(super) const fn from_parts(status: Status, parts: ResponseParts) -> Self {
        Self { status, parts }
    }

    pub(super) const fn status(&self) -> Status {
        self.status
    }

    pub(super) fn into_parts(self) -> ResponseParts {
        self.parts
    }
}

/// Builds a bounded response for a request that timed out before its opcode
/// could be decoded.
pub(super) fn request_read_timeout_response() -> OperationResponse {
    response_bytes(Status::Timeout, b"request read timed out")
}

/// Builds a bounded response for a request that exceeded the admission limit
/// before an operation registration could be selected.
pub(super) fn request_too_large_response() -> OperationResponse {
    response_bytes(Status::TooLarge, b"request exceeds the protocol limit")
}

/// Builds an operation-scoped timeout response through its generated contract.
pub(super) fn timeout_response(opcode: Opcode, message: &[u8]) -> OperationResponse {
    contract_error_response(opcode, Status::Timeout, message)
}

/// Builds an operation-scoped overload response through its generated contract.
pub(super) fn overloaded_response(opcode: Opcode, message: &[u8]) -> OperationResponse {
    contract_error_response(opcode, Status::Overloaded, message)
}

/// Projects a server protocol-adapter error into the wire status vocabulary.
///
/// Framing errors are discovered before an operation handler exists, so this
/// adapter intentionally uses the common protocol status set instead of a
/// generated operation contract.
pub(super) fn protocol_error_response(error: crate::protocol::ProtocolError) -> OperationResponse {
    let status = match error {
        crate::protocol::ProtocolError::UnknownOpcode(_) => Status::UnsupportedOpcode,
        crate::protocol::ProtocolError::ValueTooLarge { .. } => Status::TooLarge,
        _ => Status::InvalidRequest,
    };
    response_display(status, error)
}

/// Projects a shared wire-parser error into the same bounded response shape.
pub(super) fn wire_protocol_error_response(
    error: openkache_protocol::ProtocolError,
) -> OperationResponse {
    let status = match error {
        openkache_protocol::ProtocolError::UnknownOpcode(_) => Status::UnsupportedOpcode,
        openkache_protocol::ProtocolError::ValueTooLarge { .. } => Status::TooLarge,
        _ => Status::InvalidRequest,
    };
    response_display(status, error)
}

/// Returns the generated response admission budget for one operation.
///
/// Generated wire metadata is consumed by the transport adapter. API
/// registration only receives this opaque budget and does not import the wire
/// contract directly.
pub(super) const fn response_budget(opcode: Opcode) -> usize {
    contract::response_budget(opcode)
}

/// Executes one API handler and projects its outcome through the generated
/// response contract.
///
/// Every registration uses this same boundary. The handler decides domain
/// behavior; this module alone maps the transport-neutral outcome through the
/// generated response contract.
pub(super) async fn execute(
    capabilities: &dyn CapabilityCatalog,
    input: OperationInputView,
    handler: OperationHandler,
) -> Option<OperationResponse> {
    let opcode = input.opcode();
    let outcome = handler(OperationContext {
        capabilities,
        input,
    })
    .await;
    encode_operation_outcome(opcode, outcome)
}

/// Selects an error status that the generated operation contract permits.
///
/// A generic adapter may discover a malformed domain result or a denied
/// capability before an API-owned handler runs. Returning a hard-coded status
/// here would make a future operation violate its own contract.
pub(super) fn contract_error_status(opcode: Opcode, preferred: Status) -> Status {
    select_error_status(contract::spec(opcode), preferred)
}

/// Builds a contract-valid error response for a generated operation.
pub(super) fn contract_error_response(
    opcode: Opcode,
    preferred: Status,
    message: &[u8],
) -> OperationResponse {
    let status = contract_error_status(opcode, preferred);
    bounded_bytes_response(
        status,
        message,
        b"operation error exceeds the protocol limit",
    )
}

/// Builds a contract-valid error response from an API-owned opaque status.
pub(super) fn contract_error_response_token(
    opcode: Opcode,
    preferred: StatusToken,
    message: &[u8],
) -> OperationResponse {
    contract_error_response(
        opcode,
        wire_status(preferred).unwrap_or(Status::InternalError),
        message,
    )
}

/// Builds a contract-valid response for a common infrastructure failure.
///
/// These helpers keep wire status selection inside the transport adapter. The
/// dispatcher only reports the failure category and never imports the wire
/// enum.
pub(super) fn invalid_request_response(opcode: Opcode, message: &[u8]) -> OperationResponse {
    contract_error_response(opcode, Status::InvalidRequest, message)
}

pub(super) fn unsupported_operation_response(
    opcode: Opcode,
    message: &[u8],
) -> OperationResponse {
    contract_error_response(opcode, Status::UnsupportedOpcode, message)
}

pub(super) fn forbidden_response(opcode: Opcode, message: &[u8]) -> OperationResponse {
    contract_error_response(opcode, Status::Forbidden, message)
}

/// Encodes a transport-neutral outcome through the shared generated response
/// layout selected by the operation descriptor.
pub(super) fn encode_operation_outcome(
    opcode: Opcode,
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
                    if wire.generic_response_framing()
                        != Some(contract::OperationResponseFraming::Empty)
                    {
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
                    if value.len() > response_budget(opcode) {
                        return Some(contract_error_response(
                            opcode,
                            Status::TooLarge,
                            b"operation response exceeds the protocol limit",
                        ));
                    }
                    Some(generic_opaque_response(opcode, status, value))
                }
                OperationBody::Fields(values) => {
                    Some(planned_fields_response(opcode, status, values))
                }
            }
        }
        OperationOutcome::Error(error) => operation_error_response(opcode, error),
        OperationOutcome::Abandoned => None,
    }
}

/// Encodes a generated ordered-field response through its descriptor-selected
/// shared layout.
pub(super) fn operation_fields_response(
    opcode: Opcode,
    status: Status,
    values: SmallVec<[Option<OperationValue>; 8]>,
) -> OperationResponse {
    planned_fields_response(opcode, status, values)
}

/// Encodes any descriptor-planned field response.
fn planned_fields_response(
    opcode: Opcode,
    status: Status,
    values: SmallVec<[Option<OperationValue>; 8]>,
) -> OperationResponse {
    let wire = contract::spec(opcode);
    if !matches!(
        wire.generic_response_framing(),
        Some(contract::OperationResponseFraming::FieldSequence)
        | Some(contract::OperationResponseFraming::OptionalValues)
    ) || values.len() != wire.response.fields.len()
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
    match segmented_response_fields(values, wire.response.layout) {
        Ok(segments) => response_parts_with_budget(
            status,
            segments,
            response_budget(opcode),
        )
            .map(|parts| OperationResponse::from_parts(status, parts))
            .unwrap_or_else(|_| {
                contract_error_response(
                    opcode,
                    Status::TooLarge,
                    b"operation response fields exceed the protocol limit",
                )
            }),
        Err(()) => contract_error_response(
            opcode,
            Status::TooLarge,
            b"operation response fields exceed the protocol limit",
        ),
    }
}

fn segmented_response_fields(
    values: SmallVec<[Option<OperationValue>; 8]>,
    layout: contract::OperationFieldLayout,
) -> Result<SmallVec<[ResponseSegment; 8]>, ()> {
    encode_layout_segments(values, layout, append_operation_value).map_err(|_| ())
}

pub(super) fn append_operation_value(
    segments: &mut SmallVec<[ResponseSegment; 8]>,
    value: OperationValue,
) {
    match value {
        OperationValue::Inline(value) => segments.push(ResponseSegment::Inline(value)),
        OperationValue::Owned(value) => segments.push(ResponseSegment::Payload(value)),
        OperationValue::Segmented(value) => value.append_segments(segments),
    }
}

pub(super) fn validate_response_fields(
    values: &[Option<OperationValue>],
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
        if field.codecs.is_empty() {
            continue;
        }
        let valid = match value {
            OperationValue::Segmented(value) => {
                super::operation_fields::validate_segmented_field(field, value).is_ok()
            }
            _ => value.contiguous().is_some_and(|bytes| {
                super::operation_fields::validate_field_bytes(field, bytes).is_ok()
            }),
        };
        if !valid {
            return Err(b"operation response field does not satisfy its generated codec");
        }
    }
    Ok(())
}

fn generic_opaque_response(
    opcode: Opcode,
    status: Status,
    value: OperationValue,
) -> OperationResponse {
    if !valid_opaque_response(opcode, &value) {
        return contract_error_response(
            opcode,
            Status::InternalError,
            b"opaque operation payload does not match its response framing",
        );
    }
    operation_response(opcode, status, value)
}

fn response_parts_with_budget(
    status: Status,
    segments: SmallVec<[ResponseSegment; 8]>,
    budget: usize,
) -> Result<ResponseParts, ()> {
    let payload_len = segments
        .iter()
        .try_fold(0usize, |total, segment| total.checked_add(segment.len()))
        .ok_or(())?;
    if payload_len > budget {
        return Err(());
    }
    ResponseParts::from_segments(status, segments).map_err(|_| ())
}

fn valid_opaque_response(opcode: openkache_protocol::Opcode, value: &OperationValue) -> bool {
    let wire = contract::spec(opcode);
    if wire.generic_response_framing() != Some(contract::OperationResponseFraming::Opaque) {
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
    match value {
        OperationValue::Segmented(value) => {
            super::operation_fields::validate_segmented_field(field, value).is_ok()
        }
        _ => value.contiguous().is_some_and(|bytes| {
            super::operation_fields::validate_field_bytes(field, bytes).is_ok()
        }),
    }
}

fn operation_success_status(
    opcode: openkache_protocol::Opcode,
    status: StatusToken,
) -> Option<Status> {
    let status = wire_status(status)?;
    if contract::spec(opcode).success_statuses.contains(&status) {
        Some(status)
    } else {
        None
    }
}

fn wire_status(token: StatusToken) -> Option<Status> {
    Status::try_from(token.code()).ok()
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
            Status::InvalidRequest,
            OperationValue::inline(message),
        ),
        OperationError::Status { status, message } => operation_status_response(
            opcode,
            contract,
            wire_status(status).unwrap_or(Status::InternalError),
            OperationValue::inline(message),
        ),
        OperationError::OwnedStatus { status, message } => operation_status_response(
            opcode,
            contract,
            wire_status(status).unwrap_or(Status::InternalError),
            OperationValue::from(message),
        ),
    }
}

fn operation_status_response(
    opcode: openkache_protocol::Opcode,
    contract: contract::OperationWireSpec,
    requested: Status,
    message: OperationValue,
) -> Option<OperationResponse> {
    let status = select_error_status(contract, requested);
    Some(operation_response(opcode, status, message))
}

/// Chooses the closest contract-valid error status for an adapter preference.
///
/// All pre-dispatch and post-behavior errors use this one projection. Keeping
/// the fallback policy in one helper prevents a new operation from observing
/// different status selection depending on which boundary detected the error.
fn select_error_status(contract: contract::OperationWireSpec, requested: Status) -> Status {
    contract
        .error_statuses
        .iter()
        .copied()
        .find(|candidate| *candidate == requested)
        .or_else(|| {
            contract
                .error_statuses
                .iter()
                .copied()
                .find(|candidate| *candidate == Status::InternalError)
        })
        .or_else(|| contract.error_statuses.first().copied())
        .unwrap_or(Status::InternalError)
}

/// Encodes one operation response without allowing an API-owned payload to
/// panic the server when it exceeds the protocol frame limit.
fn operation_response(
    opcode: openkache_protocol::Opcode,
    status: Status,
    payload: impl Into<OperationValue>,
) -> OperationResponse {
    match response_parts(status, payload.into()) {
        Ok(response) => response,
        Err(()) => contract_error_response(
            opcode,
            Status::TooLarge,
            b"operation response exceeds the protocol limit",
        ),
    }
}

fn response_parts(status: Status, payload: OperationValue) -> Result<OperationResponse, ()> {
    let mut segments = SmallVec::<[ResponseSegment; 8]>::new();
    append_operation_value(&mut segments, payload);
    ResponseParts::from_segments(status, segments)
        .map(|parts| OperationResponse::from_parts(status, parts))
        .map_err(|_| ())
}

fn response_display(status: Status, value: impl std::fmt::Display) -> OperationResponse {
    let mut payload = String::with_capacity(
        openkache_protocol::RESPONSE_FIXED_BYTES + openkache_protocol::MAX_VARUINT_BYTES + 64,
    );
    use std::fmt::Write as _;
    write!(payload, "{value}").expect("writing to a String cannot fail");
    response_parts(status, OperationValue::from(payload.into_bytes()))
        .unwrap_or_else(|_| response_bytes(status, b"operation error exceeds the protocol limit"))
}

fn response_bytes(status: Status, payload: &[u8]) -> OperationResponse {
    bounded_bytes_response(
        status,
        payload,
        b"operation response exceeds the protocol limit",
    )
}

fn bounded_bytes_response(status: Status, payload: &[u8], fallback: &[u8]) -> OperationResponse {
    let payload = if payload.len() <= openkache_protocol::MAX_VALUE_BYTES {
        payload
    } else {
        fallback
    };
    response_parts(status, OperationValue::inline(payload))
        .expect("bounded server response must remain within protocol limits")
}
