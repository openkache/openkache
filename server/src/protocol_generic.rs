//! Operation-neutral request framing and field validation.
//!
//! This module deliberately knows only generated wire metadata. Historical
//! compatibility layouts stay in `protocol_compat_v1`; the public
//! `Request` facade may compose both projections, but the generic parser does
//! not select or interpret a compatibility route.

use super::super::operation_contract as contract;
use super::{
    ProtocolError, Result, WireRequestLayout, WireResult, decode_varuint, validate_value_length,
};
use openkache_protocol::{
    OPCODE_BYTES, Opcode, OperationLayoutPlan, decode_planned_fields, encode_planned_fields,
};
use smallvec::SmallVec;

const INLINE_OPERATION_FIELDS: usize = 8;

pub(super) static REQUEST_DESCRIPTOR: super::RequestDescriptor = super::RequestDescriptor::new(
    "generated",
    request_frame_layout,
    decode_header,
    encode_request_prefix,
    validate_request,
    decode_request,
    decode_owned_request,
    decode_server_request,
);

pub(super) fn request_frame_layout(opcode: Opcode) -> WireResult<WireRequestLayout> {
    Ok(contract::wire_request_layout(opcode))
}

pub(super) fn decode_header(
    prefix: &[u8],
    opcode: Opcode,
    descriptor: &'static super::RequestDescriptor,
) -> Result<Option<super::RequestHeader>> {
    let plan = contract::operation_wire_spec(opcode).request;
    if plan.frame == contract::OperationFramePolicy::FixedBody {
        let body_len = plan.exact_width;
        validate_value_length(body_len)?;
        let encoded_end = OPCODE_BYTES
            .checked_add(body_len)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        if prefix.len() < encoded_end {
            return Ok(None);
        }
        return Ok(Some(super::RequestHeader::generic(
            descriptor,
            opcode,
            OPCODE_BYTES,
            body_len,
        )));
    }
    let framing = plan.framing;
    match framing {
        contract::OperationLayoutFraming::Empty => Ok(Some(super::RequestHeader::generic(
            descriptor,
            opcode,
            OPCODE_BYTES,
            0,
        ))),
        contract::OperationLayoutFraming::Opaque
        | contract::OperationLayoutFraming::OrderedFields
        | contract::OperationLayoutFraming::FieldSequence
        | contract::OperationLayoutFraming::OptionalValues => {
            let context = match framing {
                contract::OperationLayoutFraming::Opaque => "application value length",
                contract::OperationLayoutFraming::OrderedFields
                | contract::OperationLayoutFraming::FieldSequence
                | contract::OperationLayoutFraming::OptionalValues => "field sequence length",
                contract::OperationLayoutFraming::Empty => unreachable!(),
            };
            let Some((value_len, value_len_bytes)) =
                decode_varuint(&prefix[OPCODE_BYTES..], context)?
            else {
                return Ok(None);
            };
            let value_len =
                usize::try_from(value_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
            validate_value_length(value_len)?;
            Ok(Some(super::RequestHeader::generic(
                descriptor,
                opcode,
                OPCODE_BYTES + value_len_bytes,
                value_len,
            )))
        }
    }
}

pub(super) fn encode_request_prefix(
    request: &super::Request,
    output: &mut Vec<u8>,
) -> Result<bool> {
    let contract = contract::operation_wire_spec(request.opcode).request;
    match contract.framing {
        contract::OperationLayoutFraming::Empty => {}
        contract::OperationLayoutFraming::Opaque
        | contract::OperationLayoutFraming::OrderedFields
        | contract::OperationLayoutFraming::FieldSequence => {
            if contract.frame == contract::OperationFramePolicy::FixedBody {
                if request.value.len() != contract.exact_width {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "fixed generic request width does not match contract",
                    ));
                }
            } else {
                let (encoded, length) = super::encode_varuint(request.value.len() as u64);
                output.extend_from_slice(&encoded[..length]);
            }
        }
        contract::OperationLayoutFraming::OptionalValues => {
            return Err(ProtocolError::InvalidFieldSequence(
                "optional-values framing is response-only",
            ));
        }
    }
    Ok(true)
}

/// Encodes a generic request from its descriptor-shaped field values.
///
/// This keeps callers from reimplementing presence-mask/dense layout selection
/// when constructing a new route-less API request.
pub(super) fn encode_fields(opcode: Opcode, values: Vec<Option<Vec<u8>>>) -> Result<Vec<u8>> {
    encode_fields_with_plan(values, contract::spec(opcode).request)
}

/// Encodes field values using only operation-neutral generated layout metadata.
///
/// The adapter validates each field against its generated width and codec
/// metadata before handing the payload to the operation-neutral request
/// facade.
pub(super) fn encode_fields_with_plan(
    values: Vec<Option<Vec<u8>>>,
    plan: OperationLayoutPlan,
) -> Result<Vec<u8>> {
    if !matches!(
        plan.framing,
        contract::OperationLayoutFraming::Empty
            | contract::OperationLayoutFraming::OrderedFields
            | contract::OperationLayoutFraming::FieldSequence
    ) {
        return Err(ProtocolError::InvalidFieldSequence(
            "generic fields require an encodable request framing",
        ));
    }
    let fields = plan.fields;
    if values.len() != fields.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "generic field values do not match the generated request plan",
        ));
    }
    if fields.len() > contract::MAX_OPERATION_REQUEST_FIELDS {
        return Err(ProtocolError::InvalidFieldSequence(
            "generic request field plan exceeds generated bounds",
        ));
    }
    for (field, value) in fields.iter().zip(values.iter()) {
        let Some(value) = value.as_deref() else {
            if field.required {
                return Err(ProtocolError::InvalidFieldSequence(
                    "required generic request field is missing",
                ));
            }
            continue;
        };
        if field.encoded_width != 0 && value.len() != field.encoded_width {
            return Err(ProtocolError::InvalidFieldSequence(
                "generic request field does not match its declared width",
            ));
        }
        openkache_protocol::codec::validate_field_codecs_with_nested_widths(
            value,
            field.codecs,
            field.nested_codecs,
            field.nested_widths,
            field.nested_enum_values,
            field.nested_union_tags,
            field.enum_values,
            field.union_tags,
            contract::wire_codec_kind,
        )
        .map_err(|error| {
            ProtocolError::InvalidFieldSequence(
                std::str::from_utf8(error.message())
                    .unwrap_or("generic request field codec is invalid"),
            )
        })?;
    }
    let borrowed: SmallVec<[Option<&[u8]>; INLINE_OPERATION_FIELDS]> =
        values.iter().map(|value| value.as_deref()).collect();
    encode_planned_fields(&borrowed, fields, plan.layout).map_err(Into::into)
}

/// Validates a semantic request against the generated generic shape.
///
/// The request facade exposes a generic payload view at this boundary. That
/// keeps this module independent from compatibility-only semantic fields while
/// allowing another framing adapter to provide its own validator.
pub(super) fn validate_request(request: &super::Request) -> Result<()> {
    let payload = request.generic_payload()?;
    validate_value_length(payload.len())?;
    match contract::spec(request.opcode).request.framing {
        contract::OperationLayoutFraming::OrderedFields
        | contract::OperationLayoutFraming::FieldSequence => {
            validate_fields(request.opcode, payload)?;
            Ok(())
        }
        contract::OperationLayoutFraming::Empty if !payload.is_empty() => Err(
            ProtocolError::InvalidFieldSequence("empty generic request contains a payload"),
        ),
        contract::OperationLayoutFraming::Empty | contract::OperationLayoutFraming::Opaque => {
            Ok(())
        }
        contract::OperationLayoutFraming::OptionalValues => Err(
            ProtocolError::InvalidFieldSequence("optional-values framing is response-only"),
        ),
    }
}

/// Decodes a borrowed generic frame into the public semantic facade.
///
/// The adapter owns the body boundary. The shared protocol module only
/// validates the frame length before calling this function.
pub(super) fn decode_request(frame: &[u8], header: super::RequestHeader) -> Result<super::Request> {
    super::Request::from_generic_parts(header.opcode, frame[header.encoded_len..].to_vec())
}

/// Decodes a generic frame while reusing its allocation for the body.
pub(super) fn decode_owned_request(
    mut frame: Vec<u8>,
    header: super::RequestHeader,
) -> Result<super::Request> {
    frame.copy_within(header.encoded_len.., 0);
    frame.truncate(header.value_len);
    super::Request::from_generic_parts(header.opcode, frame)
}

/// Keeps a generic request frame owned until the generated operation view has
/// decoded its fields. No semantic compatibility projection is materialized.
pub(super) fn decode_server_request(
    frame: Vec<u8>,
    header: super::RequestHeader,
) -> Result<super::ServerRequest> {
    Ok(super::ServerRequest::Frame { frame, header })
}

/// Validates only generated shape metadata for a generic request.
pub(super) fn validate_fields(opcode: Opcode, payload: &[u8]) -> Result<()> {
    let plan = contract::operation_wire_spec(opcode).request.fields;
    if plan.len() > contract::MAX_OPERATION_REQUEST_FIELDS {
        return Err(ProtocolError::InvalidFieldSequence(
            "generic request field plan exceeds generated bounds",
        ));
    }
    let mut offsets =
        SmallVec::<[(usize, usize); INLINE_OPERATION_FIELDS]>::with_capacity(plan.len());
    offsets.resize(plan.len(), (usize::MAX, usize::MAX));
    decode_planned_fields(
        payload,
        plan,
        contract::operation_wire_spec(opcode).request.layout,
        &mut offsets,
    )?;
    Ok(())
}
