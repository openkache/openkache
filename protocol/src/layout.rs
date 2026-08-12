//! Generic generated field-layout dispatch.
//!
//! The layout module translates generated field metadata into one of the
//! shared payload primitives. It does not inspect operation names, semantic
//! roles, or protocol-v1 routes; those concerns stay in generated/API-owned
//! adapters.

use crate::{
    MAX_OPERATION_FIELDS, OperationFieldLayout, OperationFieldPlan, ProtocolError, ResponseSegment,
    Result, encode_field_sequence, encode_varuint, optional_value_prefix,
    optional_values_encoded_len, validate_value_length,
};
use smallvec::SmallVec;

const INLINE_OPERATION_FIELDS: usize = 8;

/// Value abstraction used by ownership-preserving layout encoders.
pub trait LayoutValue {
    /// Returns the exact encoded length of this value.
    fn encoded_len(&self) -> usize;
}

/// Encodes a generated field layout into ownership-preserving response
/// segments. Framing bytes are created here while the caller moves each
/// application/storage value into a segment, avoiding a coalescing copy.
pub fn encode_layout_segments<T, F>(
    values: SmallVec<[Option<T>; INLINE_OPERATION_FIELDS]>,
    layout: OperationFieldLayout,
    mut append_value: F,
) -> Result<SmallVec<[ResponseSegment; INLINE_OPERATION_FIELDS]>>
where
    T: LayoutValue,
    F: FnMut(&mut SmallVec<[ResponseSegment; INLINE_OPERATION_FIELDS]>, T),
{
    if values.len() > MAX_OPERATION_FIELDS {
        return Err(ProtocolError::InvalidFieldSequence(
            "field values exceed the generated operation bound",
        ));
    }
    let mut segments = SmallVec::<[ResponseSegment; INLINE_OPERATION_FIELDS]>::new();
    match layout {
        OperationFieldLayout::Dense => {
            if values.iter().any(Option::is_none) {
                return Err(ProtocolError::InvalidFieldSequence(
                    "dense layout cannot omit a required field",
                ));
            }
            for value in values.into_iter().flatten() {
                append_value(&mut segments, value);
            }
        }
        OperationFieldLayout::Sequence => {
            let mut mask = SmallVec::<[u8; 32]>::new();
            mask.resize(values.len().saturating_add(7) / 8, 0);
            let final_present = values.iter().rposition(Option::is_some);
            for (index, value) in values.iter().enumerate() {
                if value.is_some() {
                    mask[index / 8] |= 1 << (index % 8);
                }
            }
            segments.push(ResponseSegment::Inline(mask));
            for (index, value) in values.into_iter().enumerate() {
                let Some(value) = value else {
                    continue;
                };
                if Some(index) != final_present {
                    let length = u64::try_from(value.encoded_len())
                        .map_err(|_| ProtocolError::FrameLengthOverflow)?;
                    let (encoded, encoded_len) = encode_varuint(length);
                    segments.push(ResponseSegment::inline(&encoded[..encoded_len]));
                }
                append_value(&mut segments, value);
            }
        }
        OperationFieldLayout::OptionalValues => {
            for value in values {
                let prefix = optional_value_prefix(value.as_ref().map(LayoutValue::encoded_len))?;
                segments.push(ResponseSegment::inline(&prefix));
                if let Some(value) = value {
                    append_value(&mut segments, value);
                }
            }
        }
        OperationFieldLayout::Empty
        | OperationFieldLayout::Opaque
        | OperationFieldLayout::AdapterOwned => {
            return Err(ProtocolError::InvalidFieldSequence(
                "selected layout does not encode ordered fields",
            ));
        }
    }
    Ok(segments)
}

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
/// This is the generic wire-shape dispatch point shared by clients, servers,
/// and private adapters. It accepts only generic ordered opaque field bytes;
/// semantic roles, operation names, and compatibility policies stay outside
/// the protocol primitive. Adapter-owned layouts are rejected here and must be
/// projected by the adapter that declared them.
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
        OperationFieldLayout::OptionalValues => optional_values_encoded_len(values)
            .and_then(|encoded_len| {
                let mut output = Vec::with_capacity(encoded_len);
                for value in values {
                    let prefix = optional_value_prefix(value.map(<[u8]>::len))?;
                    output.extend_from_slice(&prefix);
                    if let Some(value) = value {
                        output.extend_from_slice(value);
                    }
                }
                Ok(output)
            }),
        OperationFieldLayout::Dense => encode_dense_fields(values, widths),
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
        OperationFieldLayout::AdapterOwned => Err(ProtocolError::InvalidFieldSequence(
            "adapter-owned layout requires its response adapter",
        )),
    }
}

/// Decodes modeled fields using the layout selected by the generated shape
/// plan, writing borrowed offsets into caller-owned storage.
///
/// Keeping this dispatch in the shared protocol crate prevents server and
/// client implementations from drifting on presence masks, field lengths,
/// and fixed-width tuples. Adapter-owned layouts are fail-closed here.
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
        OperationFieldLayout::OptionalValues => {
            let decoded = crate::OptionalValues::decode(payload, required.len(), offsets)?;
            if required
                .iter()
                .enumerate()
                .any(|(index, is_required)| *is_required && decoded.get(index).is_none())
            {
                return Err(ProtocolError::InvalidFieldSequence(
                    "optional-value layout is missing a required field",
                ));
            }
            Ok(())
        }
        OperationFieldLayout::Dense => {
            if required.iter().any(|is_required| !is_required) {
                return Err(ProtocolError::InvalidFieldSequence(
                    "dense layout cannot contain optional fields",
                ));
            }
            DenseFields::decode(payload, widths, offsets).map(|_| ())
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
        OperationFieldLayout::AdapterOwned => Err(ProtocolError::InvalidFieldSequence(
            "adapter-owned layout requires its response adapter",
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
