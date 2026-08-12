//! Compact-v1 wire error projection and packed-step inspection.
//!
//! The generated request decoder reports structural errors. This adapter-only
//! module maps them to the historical public error vocabulary without making
//! the generic protocol codec aware of SET or namespace semantics.

use openkache_protocol::{Opcode, RequestWirePackedField, RequestWireStep};

use super::super::ProtocolError;
use super::contract;

/// Projects operation-neutral generated-plan failures into compatibility
/// errors.
pub(super) fn map_wire_decode_error(
    opcode: Opcode,
    prefix: &[u8],
    error: openkache_protocol::ProtocolError,
) -> ProtocolError {
    match error {
        openkache_protocol::ProtocolError::NonCanonicalVaruint {
            context: "request wire integer" | "request integer",
        } if opcode == Opcode::Set => ProtocolError::NonCanonicalVaruint { context: "SET TTL" },
        openkache_protocol::ProtocolError::InvalidFieldSequence(_) => {
            classify_wire_prefix(opcode, prefix).unwrap_or_else(|| {
                ProtocolError::InvalidFieldSequence("request wire plan rejected the compact prefix")
            })
        }
        error => error.into(),
    }
}

const MAX_COMPACT_FIELDS: usize = 64;

pub(super) fn classify_wire_prefix(opcode: Opcode, prefix: &[u8]) -> Option<ProtocolError> {
    let plan = contract::request_wire_plan(opcode)?;
    let mut cursor = openkache_protocol::OPCODE_BYTES;
    let mut selectors = [None; MAX_COMPACT_FIELDS];
    classify_wire_steps(opcode, prefix, &mut cursor, plan.steps, &mut selectors)
}

fn classify_wire_steps(
    opcode: Opcode,
    prefix: &[u8],
    cursor: &mut usize,
    steps: &[RequestWireStep],
    selectors: &mut [Option<&'static [u8]>; MAX_COMPACT_FIELDS],
) -> Option<ProtocolError> {
    for step in steps {
        match *step {
            RequestWireStep::FixedField { bytes, .. } => {
                *cursor = cursor.checked_add(bytes)?;
            }
            RequestWireStep::Packed {
                fields,
                reserved_mask,
                ..
            } => {
                let byte = *prefix.get(*cursor)?;
                if byte & reserved_mask != 0 {
                    return Some(packed_flags_error(opcode, fields, byte & reserved_mask));
                }
                for field in fields {
                    let selected = byte & field.mask;
                    let Some(mapping) =
                        field.values.iter().find(|mapping| mapping.bits == selected)
                    else {
                        return Some(packed_field_error(opcode, field.field));
                    };
                    *selectors.get_mut(field.field)? = Some(mapping.value);
                }
                *cursor = cursor.checked_add(1)?;
            }
            RequestWireStep::ByteLengthField { .. } => {
                let length = usize::from(*prefix.get(*cursor)?);
                *cursor = cursor.checked_add(1)?.checked_add(length)?;
            }
            RequestWireStep::VarUIntField { .. } => {
                let (_, encoded_len) = openkache_protocol::decode_varuint(
                    prefix.get(*cursor..).unwrap_or_default(),
                    "request wire integer",
                )
                .ok()??;
                *cursor = cursor.checked_add(encoded_len)?;
            }
            RequestWireStep::Conditional {
                field,
                equals,
                steps,
            } => {
                if selectors.get(field).copied().flatten() == Some(equals)
                    && let Some(error) =
                        classify_wire_steps(opcode, prefix, cursor, steps, selectors)
                {
                    return Some(error);
                }
            }
            RequestWireStep::Bytes { expected } => {
                let end = cursor.checked_add(expected.len())?;
                if prefix.get(*cursor..end) != Some(expected) {
                    let actual = *prefix.get(*cursor)?;
                    return Some(ProtocolError::UnknownRequestFlags(actual));
                }
                *cursor = end;
            }
            RequestWireStep::TrailingField { .. } => {
                let (_, encoded_len) = openkache_protocol::decode_varuint(
                    prefix.get(*cursor..).unwrap_or_default(),
                    "request trailing field length",
                )
                .ok()??;
                *cursor = cursor.checked_add(encoded_len)?;
            }
        }
    }
    None
}

fn packed_flags_error(
    opcode: Opcode,
    fields: &[RequestWirePackedField],
    bits: u8,
) -> ProtocolError {
    if fields.iter().any(|field| {
        matches!(
            contract::spec(opcode)
                .request
                .fields
                .get(field.field)
                .map(|field| field.role),
            Some(
                "default_expiration"
                    | "default_ttl_milliseconds"
                    | "expiration_override"
                    | "default_eviction"
                    | "eviction_override"
            )
        )
    }) {
        ProtocolError::InvalidNamespacePolicy("namespace policy contains reserved bits")
    } else {
        ProtocolError::UnknownRequestFlags(bits)
    }
}

fn packed_field_error(opcode: Opcode, field: usize) -> ProtocolError {
    match contract::spec(opcode)
        .request
        .fields
        .get(field)
        .map(|field| field.role)
    {
        Some("condition") => ProtocolError::ConflictingSetConditions,
        Some("expiration_mode" | "eviction_mode") => ProtocolError::InvalidSetOptions { opcode },
        Some(
            "default_expiration"
            | "default_ttl_milliseconds"
            | "expiration_override"
            | "default_eviction"
            | "eviction_override",
        ) => ProtocolError::InvalidNamespacePolicy("namespace policy contains an unknown mode"),
        _ => ProtocolError::InvalidFieldSequence("request packed field has an unknown value"),
    }
}
