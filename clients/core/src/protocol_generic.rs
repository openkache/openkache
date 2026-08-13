//! Generic client request and response-field adapter.
//!
//! This module consumes only the generated operation contract and reusable
//! field codecs. Protocol-v1 namespace/item/SET projections stay in
//! [`super::compat_v1`], while the transport core calls the small functions
//! re-exported by `protocol.rs`.

use openkache_protocol::{decode_planned_fields, encode_planned_fields};
use smallvec::SmallVec;

use super::{Opcode, ProtocolError, Request, Result};
use crate::contract::{MAX_OPERATION_FIELDS, MAX_OPERATION_REQUEST_FIELDS};

pub(crate) const INLINE_OPERATION_FIELDS: usize = 8;

/// Borrowed view over a decoded ordered response field sequence.
///
/// The view owns only one bounded offset table and borrows every field from
/// the response payload. Callers that need owned values can use
/// [`OperationFields::to_owned`].
#[derive(Debug)]
pub struct OperationFields<'a> {
    pub(super) payload: &'a [u8],
    pub(super) offsets: SmallVec<[(usize, usize); INLINE_OPERATION_FIELDS]>,
    pub(super) field_len: usize,
}

impl<'a> OperationFields<'a> {
    /// Returns the number of modeled fields.
    pub fn len(&self) -> usize {
        self.field_len
    }

    /// Returns whether the modeled field is present.
    pub fn is_present(&self, index: usize) -> bool {
        self.get(index).is_some()
    }

    /// Returns one borrowed field value, preserving present-empty as `Some`.
    pub fn get(&self, index: usize) -> Option<&'a [u8]> {
        let (start, end) = *self.offsets.get(index).filter(|_| index < self.field_len)?;
        (start != usize::MAX).then(|| &self.payload[start..end])
    }

    /// Copies the view into the historical owned representation.
    pub fn to_owned(&self) -> Vec<Option<Vec<u8>>> {
        (0..self.len())
            .map(|index| self.get(index).map(ToOwned::to_owned))
            .collect()
    }

    pub(crate) fn from_parts(
        payload: &'a [u8],
        offsets: SmallVec<[(usize, usize); INLINE_OPERATION_FIELDS]>,
        field_len: usize,
    ) -> Self {
        Self {
            payload,
            offsets,
            field_len,
        }
    }
}

/// Builds a request from a raw generic operation body.
///
/// This is deliberately a body-only boundary. Historical request projections
/// remain compatibility-adapter inputs and never enter this module.
pub(crate) fn request_from_contract_body(
    operation: Opcode,
    value: Vec<u8>,
) -> crate::Result<Request> {
    let contract = crate::contract::operation_wire_spec(operation);
    match contract.generic_request_framing() {
        Some(crate::contract::OperationRequestFraming::Empty) => {
            Request::new_generic(operation, value).map_err(crate::Error::protocol)
        }
        Some(crate::contract::OperationRequestFraming::Opaque) => {
            Request::new_generic(operation, value).map_err(crate::Error::protocol)
        }
        Some(crate::contract::OperationRequestFraming::OrderedFields) => {
            ordered_request(operation, value)
        }
        None => Err(crate::Error::configuration(
            "operation",
            "operation request framing is not generic",
        )),
    }
}

/// Validates a generic request body against its generated framing and codecs.
///
/// Compatibility fields are intentionally absent from this boundary. The
/// client request facade calls this helper after its legacy adapter has
/// rejected namespace, item, and SET projections, so generic validation has
/// one implementation shared by construction and re-validation.
pub(crate) fn validate_request_body(operation: Opcode, value: &[u8]) -> Result<()> {
    let contract = crate::contract::operation_wire_spec(operation);
    match contract.generic_request_framing() {
        Some(crate::contract::OperationRequestFraming::Empty) => {
            if !contract.request.fields.is_empty() {
                return Err(ProtocolError::InvalidFieldSequence(
                    "empty request framing cannot model fields",
                ));
            }
            if value.is_empty() {
                Ok(())
            } else {
                Err(ProtocolError::InvalidFieldSequence(
                    "empty request framing cannot carry body bytes",
                ))
            }
        }
        Some(crate::contract::OperationRequestFraming::Opaque) => {
            if contract.request.fields.len() != 1 {
                return Err(ProtocolError::InvalidFieldSequence(
                    "opaque request framing requires one modeled field",
                ));
            }
            let field = &contract.request.fields[0];
            validate_operation_field(field, value)?;
            Ok(())
        }
        Some(crate::contract::OperationRequestFraming::OrderedFields) => {
            validate_ordered_body(operation, value)?;
            Ok(())
        }
        None => Err(ProtocolError::InvalidFieldSequence(
            "operation request framing is not generic",
        )),
    }
}

/// Builds one generic request from an already encoded body.
pub(crate) fn request_from_unary(operation: Opcode, body: Vec<u8>) -> crate::Result<Request> {
    request_from_contract_body(operation, body)
}

fn ordered_request(operation: Opcode, value: Vec<u8>) -> crate::Result<Request> {
    let offsets = validate_ordered_body(operation, &value).map_err(crate::Error::protocol)?;
    if openkache_protocol::operation::request_wire_plan(operation).is_some() {
        let mut fields = offsets
            .into_iter()
            .map(|(start, end)| (start != usize::MAX).then(|| value[start..end].to_vec()))
            .collect::<Vec<_>>();
        return exact_request(operation, &mut fields)?.ok_or_else(|| {
            crate::Error::configuration(
                "operation",
                "exact request plan disappeared after field validation",
            )
        });
    }
    Ok(Request::new_ordered_unchecked(operation, value))
}

fn validate_ordered_body(
    operation: Opcode,
    value: &[u8],
) -> Result<SmallVec<[(usize, usize); INLINE_OPERATION_FIELDS]>> {
    let contract = crate::contract::operation_wire_spec(operation);
    let plan = contract.request.fields;
    if plan.len() > MAX_OPERATION_REQUEST_FIELDS {
        return Err(ProtocolError::InvalidFieldSequence(
            "generated request field plan exceeds client bounds",
        ));
    }
    let mut offsets =
        SmallVec::<[(usize, usize); INLINE_OPERATION_FIELDS]>::with_capacity(plan.len());
    offsets.resize(plan.len(), (usize::MAX, usize::MAX));
    decode_planned_fields(value, plan, contract.request.layout, &mut offsets)?;
    for (index, field) in plan.iter().enumerate() {
        let (start, end) = offsets[index];
        let field_value = (start != usize::MAX).then(|| &value[start..end]);
        if field.required && field_value.is_none() {
            return Err(ProtocolError::InvalidFieldSequence(
                "required request field is missing",
            ));
        }
        if let Some(field_value) = field_value {
            validate_operation_field(field, field_value)?;
        }
    }
    Ok(offsets)
}

fn exact_request(
    operation: Opcode,
    fields: &mut [Option<Vec<u8>>],
) -> crate::Result<Option<Request>> {
    let Some(wire_plan) = openkache_protocol::operation::request_wire_plan(operation) else {
        return Ok(None);
    };
    let borrowed: SmallVec<[Option<&[u8]>; INLINE_OPERATION_FIELDS]> =
        fields.iter().map(|value| value.as_deref()).collect();
    let prefix = openkache_protocol::encode_request_wire_prefix(operation, &borrowed, wire_plan)
        .map_err(|error| crate::Error::protocol(error.to_string()))?;
    drop(borrowed);
    let body_field = wire_plan.steps.iter().find_map(|step| match *step {
        openkache_protocol::RequestWireStep::ValueLengthField { field }
        | openkache_protocol::RequestWireStep::TrailingField { field } => Some(field),
        _ => None,
    });
    let payload = match body_field {
        Some(index) => fields
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| {
                crate::Error::configuration("fields", "generated request value field is missing")
            })?,
        None => Vec::new(),
    };
    Ok(Some(Request::new_exact_unchecked(
        operation, prefix, payload,
    )))
}

/// Builds an ordered-field request from generated field values.
pub(crate) fn request_from_fields(
    operation: Opcode,
    mut fields: Vec<Option<Vec<u8>>>,
) -> crate::Result<Request> {
    let contract = crate::contract::operation_wire_spec(operation);
    let plan = contract.request.fields;
    if !matches!(
        contract.generic_request_framing(),
        Some(crate::contract::OperationRequestFraming::OrderedFields)
    ) {
        return Err(crate::Error::configuration(
            "operation",
            "ordered fields are only available for field-sequence requests",
        ));
    }
    if fields.len() != plan.len() {
        return Err(crate::Error::configuration(
            "fields",
            format!(
                "operation requires {} ordered request fields, got {}",
                plan.len(),
                fields.len()
            ),
        ));
    }
    if fields.len() > MAX_OPERATION_FIELDS {
        return Err(crate::Error::configuration(
            "fields",
            "generated request field plan exceeds client bounds",
        ));
    }
    for (field, value) in plan.iter().zip(fields.iter()) {
        if field.required && value.is_none() {
            return Err(crate::Error::configuration(
                "fields",
                format!("required request field {} is missing", field.path.join(".")),
            ));
        }
        if let Some(value) = value.as_deref() {
            validate_operation_field(field, value).map_err(crate::Error::protocol)?;
        }
    }

    if let Some(request) = exact_request(operation, &mut fields)? {
        return Ok(request);
    }

    let borrowed: SmallVec<[Option<&[u8]>; INLINE_OPERATION_FIELDS]> =
        fields.iter().map(|value| value.as_deref()).collect();
    let value = encode_planned_fields(&borrowed, plan, contract.request.layout)
        .map_err(|error| crate::Error::protocol(error.to_string()))?;
    Ok(Request::new_ordered_unchecked(operation, value))
}

/// Validates one generated field against its canonical client-side codec
/// descriptor.
pub(crate) fn validate_operation_field(
    field: &crate::contract::OperationFieldPlan,
    payload: &[u8],
) -> Result<()> {
    if field.encoded_width != 0 && payload.len() != field.encoded_width {
        return Err(ProtocolError::InvalidFieldSequence(
            "operation field does not match its declared fixed width",
        ));
    }
    openkache_protocol::codec::validate_field_codecs_with_nested_widths(
        payload,
        field.codecs,
        field.nested_codecs,
        field.nested_widths,
        field.nested_enum_values,
        field.nested_union_tags,
        field.enum_values,
        field.union_tags,
        crate::contract::wire_codec_kind,
    )
    .map_err(codec_error)
}

/// Decodes an ordered response into a bounded borrowed view.
pub(crate) fn decode_field_sequence_view<'a>(
    payload: &'a [u8],
    plan: &'static [crate::contract::OperationFieldPlan],
    layout: crate::contract::OperationFieldLayout,
) -> Result<OperationFields<'a>> {
    if plan.len() > MAX_OPERATION_FIELDS {
        return Err(ProtocolError::InvalidFieldSequence(
            "generated operation field bound is stale",
        ));
    }
    let mut offsets =
        SmallVec::<[(usize, usize); INLINE_OPERATION_FIELDS]>::with_capacity(plan.len());
    offsets.resize(plan.len(), (usize::MAX, usize::MAX));
    decode_planned_fields(payload, plan, layout, &mut offsets).map_err(ProtocolError::from)?;
    Ok(OperationFields::from_parts(payload, offsets, plan.len()))
}

/// Decodes a generated response field plan into the common borrowed view.
///
/// Generic field-sequence layouts use the common borrowed view. Adapter-owned
/// framings are rejected here because their representation is intentionally
/// outside this shared codec.
pub(crate) fn decode_response_fields_view<'a>(
    payload: &'a [u8],
    plan: &crate::contract::OperationLayoutPlan,
) -> Result<OperationFields<'a>> {
    let fields = match plan.framing {
        crate::contract::OperationLayoutFraming::FieldSequence
        | crate::contract::OperationLayoutFraming::OrderedFields
        => {
            decode_field_sequence_view(payload, plan.fields, plan.layout)
        }
        _ => Err(ProtocolError::InvalidFieldSequence(
            "operation response does not use generic ordered field framing",
        )),
    }?;
    for (index, field) in plan.fields.iter().enumerate() {
        let Some(value) = fields.get(index) else {
            if field.required {
                return Err(ProtocolError::InvalidFieldSequence(
                    "required operation response field is missing",
                ));
            }
            continue;
        };
        validate_operation_field(field, value)?;
    }
    Ok(fields)
}

fn codec_error(error: openkache_protocol::codec::CodecError) -> ProtocolError {
    ProtocolError::InvalidFieldCodec(
        std::str::from_utf8(error.message()).expect("codec diagnostics are UTF-8"),
    )
}
