//! Generic generated field-layout dispatch.
//!
//! The layout module translates generated field metadata into one of the
//! shared payload primitives. It does not inspect operation names, semantic
//! roles, or protocol-v1 routes; those concerns stay in generated/API-owned
//! adapters.

use crate::{
    MAX_OPERATION_FIELDS, OperationFieldLayout, OperationFieldPlan, ProtocolError, Result,
    encode_field_sequence, encode_optional_values, validate_value_length,
};
use smallvec::SmallVec;

const INLINE_OPERATION_FIELDS: usize = 8;

/// Encodes a flat tuple of required fixed-width fields without per-field
/// presence or length prefixes.
///
/// The generated layout compiler selects this primitive only when every field
/// has a known width and is required. Optional, variable, repeated, and
/// nested shapes continue to use [`crate::encode_field_sequence`].
pub fn encode_dense_fields(values: &[Option<&[u8]>], widths: &[usize]) -> Result<Vec<u8>> {
    if values.len() != widths.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "dense field values do not match the generated width plan",
        ));
    }
    let capacity = widths.iter().try_fold(0usize, |total, width| {
        total
            .checked_add(*width)
            .ok_or(ProtocolError::FrameLengthOverflow)
    })?;
    validate_value_length(capacity)?;
    let mut payload = Vec::with_capacity(capacity);
    for (value, width) in values.iter().zip(widths) {
        let value = value.ok_or(ProtocolError::InvalidFieldSequence(
            "dense layout cannot omit a required field",
        ))?;
        if value.len() != *width {
            return Err(ProtocolError::InvalidFieldSequence(
                "dense field width does not match the generated plan",
            ));
        }
        payload.extend_from_slice(value);
    }
    Ok(payload)
}

/// Borrowed view over a dense fixed-width field tuple.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DenseFields<'a, 'b> {
    payload: &'a [u8],
    offsets: &'b [(usize, usize)],
    field_count: usize,
}

impl<'a, 'b> DenseFields<'a, 'b> {
    /// Validates exact field widths and stores borrowed offsets.
    pub fn decode(
        payload: &'a [u8],
        widths: &[usize],
        offsets: &'b mut [(usize, usize)],
    ) -> Result<Self> {
        if offsets.len() < widths.len() {
            return Err(ProtocolError::InvalidFieldSequence(
                "dense field offset storage is smaller than the generated plan",
            ));
        }
        let mut cursor = 0usize;
        for (index, width) in widths.iter().enumerate() {
            let end = cursor
                .checked_add(*width)
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            if end > payload.len() {
                return Err(ProtocolError::InvalidFieldSequence(
                    "dense field payload is truncated",
                ));
            }
            offsets[index] = (cursor, end);
            cursor = end;
        }
        if cursor != payload.len() {
            return Err(ProtocolError::InvalidFieldSequence(
                "dense field payload contains trailing bytes",
            ));
        }
        Ok(Self {
            payload,
            offsets: &offsets[..widths.len()],
            field_count: widths.len(),
        })
    }

    /// Returns the number of dense fields.
    pub const fn len(self) -> usize {
        self.field_count
    }

    /// Returns one borrowed dense field.
    pub fn get(self, index: usize) -> Option<&'a [u8]> {
        let (start, end) = *self.offsets.get(index)?;
        Some(&self.payload[start..end])
    }
}

/// Encodes modeled fields using the layout selected by the generated shape
/// plan.
///
/// This is the single wire-shape dispatch point shared by clients, servers,
/// and private adapters. It accepts only ordered opaque field bytes; semantic
/// roles, operation names, and compatibility policies stay outside the
/// protocol primitive.
pub fn encode_layout_fields(
    values: &[Option<&[u8]>],
    layout: OperationFieldLayout,
    widths: &[usize],
) -> Result<Vec<u8>> {
    if values.len() != widths.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "field values do not match the generated layout plan",
        ));
    }
    match layout {
        OperationFieldLayout::Sequence => encode_field_sequence(values),
        OperationFieldLayout::Dense => encode_dense_fields(values, widths),
        OperationFieldLayout::OptionalValues => encode_optional_values(values),
        OperationFieldLayout::Empty => {
            if values.is_empty() {
                Ok(Vec::new())
            } else {
                Err(ProtocolError::InvalidFieldSequence(
                    "empty layout cannot contain ordered fields",
                ))
            }
        }
        OperationFieldLayout::Opaque => Err(ProtocolError::InvalidFieldSequence(
            "opaque layout cannot encode ordered fields",
        )),
    }
}

/// Decodes modeled fields using the layout selected by the generated shape
/// plan, writing borrowed offsets into caller-owned storage.
///
/// Keeping this dispatch in the shared protocol crate prevents server and
/// client implementations from drifting on presence masks, field lengths,
/// fixed-width tuples, or the explicitly selected optional-value layout.
pub fn decode_layout_fields(
    payload: &[u8],
    layout: OperationFieldLayout,
    required: &[bool],
    widths: &[usize],
    offsets: &mut [(usize, usize)],
) -> Result<()> {
    if required.len() != widths.len() || offsets.len() < required.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "field metadata does not match the generated layout plan",
        ));
    }
    match layout {
        OperationFieldLayout::Sequence => {
            crate::FieldSequence::decode_with_required(payload, required, offsets).map(|_| ())
        }
        OperationFieldLayout::Dense => {
            if required.iter().any(|is_required| !is_required) {
                return Err(ProtocolError::InvalidFieldSequence(
                    "dense layout cannot contain optional fields",
                ));
            }
            DenseFields::decode(payload, widths, offsets).map(|_| ())
        }
        OperationFieldLayout::OptionalValues => {
            crate::OptionalValues::decode(payload, required.len(), offsets).map(|_| ())
        }
        OperationFieldLayout::Empty => {
            if required.is_empty() && payload.is_empty() {
                Ok(())
            } else {
                Err(ProtocolError::InvalidFieldSequence(
                    "empty layout cannot contain ordered fields",
                ))
            }
        }
        OperationFieldLayout::Opaque => Err(ProtocolError::InvalidFieldSequence(
            "opaque layout cannot decode ordered fields",
        )),
    }
}

/// Encodes fields directly from one generated operation plan.
///
/// This convenience layer owns bounded inline metadata storage for the
/// generated operation contract. Callers only provide the plan and already
/// validated field bytes; they do not need to duplicate requiredness/width
/// extraction in every language or transport adapter.
pub fn encode_planned_fields(
    values: &[Option<&[u8]>],
    plan: &[OperationFieldPlan],
    layout: OperationFieldLayout,
) -> Result<Vec<u8>> {
    if plan.len() > MAX_OPERATION_FIELDS || values.len() != plan.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "field values do not match the generated operation plan",
        ));
    }
    let widths: SmallVec<[usize; INLINE_OPERATION_FIELDS]> =
        plan.iter().map(|field| field.encoded_width).collect();
    encode_layout_fields(values, layout, &widths)
}

/// Decodes fields directly from one generated operation plan into borrowed
/// caller-owned offsets.
pub fn decode_planned_fields(
    payload: &[u8],
    plan: &[OperationFieldPlan],
    layout: OperationFieldLayout,
    offsets: &mut [(usize, usize)],
) -> Result<()> {
    if plan.len() > MAX_OPERATION_FIELDS || offsets.len() < plan.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "field offsets do not match the generated operation plan",
        ));
    }
    let required: SmallVec<[bool; INLINE_OPERATION_FIELDS]> =
        plan.iter().map(|field| field.required).collect();
    let widths: SmallVec<[usize; INLINE_OPERATION_FIELDS]> =
        plan.iter().map(|field| field.encoded_width).collect();
    decode_layout_fields(
        payload,
        layout,
        &required[..plan.len()],
        &widths[..plan.len()],
        offsets,
    )
}
