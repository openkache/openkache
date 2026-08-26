//! Generic field-layout primitives.
//!
//! An API owns its ordered field list, requiredness, and semantic codecs. This
//! module only handles byte-level layouts and lets callers select the exact
//! primitive for each operation.

use crate::protocol::{
    OperationFieldLayout, OperationFieldPlan, OwnedFrame, ProtocolError, Result, WireSegment,
    encode_field_sequence,
};
use smallvec::{Array, SmallVec};

const INLINE_FIELDS: usize = 8;

pub(crate) const OPTIONAL_VALUE_CODEC: crate::protocol::OptionalValueCodec =
    match crate::protocol::OptionalValueCodec::new(
        crate::protocol::OPTIONAL_VALUE_LENGTH_BYTES,
        crate::protocol::OPTIONAL_VALUE_MISSING as u64,
    ) {
        Ok(codec) => codec,
        Err(_) => panic!("generated optional-value wire constants are invalid"),
    };

/// Encodes required fixed-width fields without per-field prefixes.
pub fn encode_dense_fields(values: &[&[u8]], widths: &[usize]) -> Result<Vec<u8>> {
    if values.len() != widths.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "dense values and widths have different lengths",
        ));
    }
    let capacity = widths.iter().try_fold(0usize, |total, width| {
        total
            .checked_add(*width)
            .ok_or(ProtocolError::FrameLengthOverflow)
    })?;
    crate::protocol::validate_value_length(capacity)?;
    let mut output = Vec::with_capacity(capacity);
    for (value, width) in values.iter().zip(widths) {
        if value.len() != *width {
            return Err(ProtocolError::InvalidFieldSequence(
                "dense value does not match its declared width",
            ));
        }
        output.extend_from_slice(value);
    }
    Ok(output)
}

/// Borrowed view over a dense fixed-width field tuple.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DenseFields<'a, 'b> {
    payload: &'a [u8],
    offsets: &'b [(usize, usize)],
}

impl<'a, 'b> DenseFields<'a, 'b> {
    /// Decodes exact widths into caller-owned offsets.
    pub fn decode(
        payload: &'a [u8],
        widths: &[usize],
        offsets: &'b mut [(usize, usize)],
    ) -> Result<Self> {
        if offsets.len() < widths.len() {
            return Err(ProtocolError::InvalidFieldSequence(
                "dense offset storage is too small",
            ));
        }
        let mut cursor = 0usize;
        for (index, width) in widths.iter().enumerate() {
            let end = cursor
                .checked_add(*width)
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            if end > payload.len() {
                return Err(ProtocolError::InvalidFieldSequence(
                    "dense payload is truncated",
                ));
            }
            offsets[index] = (cursor, end);
            cursor = end;
        }
        if cursor != payload.len() {
            return Err(ProtocolError::InvalidFieldSequence(
                "dense payload contains trailing bytes",
            ));
        }
        Ok(Self {
            payload,
            offsets: &offsets[..widths.len()],
        })
    }

    /// Returns the number of fields.
    pub const fn len(self) -> usize {
        self.offsets.len()
    }

    /// Returns one borrowed field.
    pub fn get(self, index: usize) -> Option<&'a [u8]> {
        let (start, end) = *self.offsets.get(index)?;
        self.payload.get(start..end)
    }
}

/// Encodes a field sequence from API-owned presence decisions.
pub fn encode_field_sequence_fields(values: &[Option<&[u8]>]) -> Result<Vec<u8>> {
    encode_field_sequence(values)
}

/// Encodes fields according to generated, operation-neutral layout metadata.
pub fn encode_layout_fields(
    values: &[Option<&[u8]>],
    layout: OperationFieldLayout,
    widths: &[usize],
) -> Result<Vec<u8>> {
    if values.len() != widths.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "field metadata length mismatch",
        ));
    }
    match layout {
        OperationFieldLayout::Sequence => encode_field_sequence(values),
        OperationFieldLayout::OptionalValues => {
            crate::protocol::encode_optional_values(OPTIONAL_VALUE_CODEC, values)
        }
        OperationFieldLayout::Dense => {
            if values.iter().any(Option::is_none) {
                return Err(ProtocolError::InvalidFieldSequence(
                    "dense field is missing",
                ));
            }
            let refs: SmallVec<[&[u8]; INLINE_FIELDS]> =
                values.iter().map(|v| v.expect("checked above")).collect();
            encode_dense_fields(&refs, widths)
        }
        OperationFieldLayout::Empty if values.is_empty() => Ok(Vec::new()),
        OperationFieldLayout::Empty => Err(ProtocolError::InvalidFieldSequence(
            "empty layout has fields",
        )),
        OperationFieldLayout::Opaque => Err(ProtocolError::InvalidFieldSequence(
            "opaque layout is not field-addressable",
        )),
    }
}

/// Decodes fields according to generated layout metadata into caller-owned offsets.
pub fn decode_layout_fields(
    payload: &[u8],
    layout: OperationFieldLayout,
    required: &[bool],
    widths: &[usize],
    offsets: &mut [(usize, usize)],
) -> Result<()> {
    if required.len() != widths.len() || offsets.len() < required.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "field metadata length mismatch",
        ));
    }
    match layout {
        OperationFieldLayout::Sequence => {
            crate::protocol::FieldSequence::decode_with_required(payload, required, offsets)
                .map(|_| ())
        }
        OperationFieldLayout::OptionalValues => OPTIONAL_VALUE_CODEC
            .decode(payload, required.len(), offsets)
            .map(|_| ()),
        OperationFieldLayout::Dense => {
            if required.iter().any(|r| !r) {
                return Err(ProtocolError::InvalidFieldSequence(
                    "dense layout has optional fields",
                ));
            }
            DenseFields::decode(payload, widths, offsets).map(|_| ())
        }
        OperationFieldLayout::Empty if required.is_empty() && payload.is_empty() => Ok(()),
        OperationFieldLayout::Empty => Err(ProtocolError::InvalidFieldSequence(
            "empty layout has fields",
        )),
        OperationFieldLayout::Opaque => Err(ProtocolError::InvalidFieldSequence(
            "opaque layout is not field-addressable",
        )),
    }
}

pub fn encode_planned_fields(
    values: &[Option<&[u8]>],
    plan: &[OperationFieldPlan],
    layout: OperationFieldLayout,
) -> Result<Vec<u8>> {
    if values.len() != plan.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "field values do not match operation plan",
        ));
    }
    let widths: SmallVec<[usize; INLINE_FIELDS]> = plan.iter().map(|f| f.encoded_width).collect();
    encode_layout_fields(values, layout, &widths)
}

pub fn decode_planned_fields(
    payload: &[u8],
    plan: &[OperationFieldPlan],
    layout: OperationFieldLayout,
    offsets: &mut [(usize, usize)],
) -> Result<()> {
    let required: SmallVec<[bool; INLINE_FIELDS]> = plan.iter().map(|f| f.required).collect();
    let widths: SmallVec<[usize; INLINE_FIELDS]> = plan.iter().map(|f| f.encoded_width).collect();
    decode_layout_fields(payload, layout, &required, &widths, offsets)
}

/// Encodes generated field-layout metadata while retaining every value owner.
///
/// The operation plan owns field order, requiredness, and fixed widths. This
/// primitive owns only structural layout bytes and never branches on an
/// opcode, API name, or semantic field role. Optional-value layouts require
/// the API-selected codec; other layouts ignore it.
pub fn encode_planned_field_segments<I, T>(
    values: I,
    plan: &[OperationFieldPlan],
    layout: OperationFieldLayout,
    optional_codec: Option<&crate::protocol::OptionalValueCodec>,
) -> Result<OwnedFrame>
where
    I: IntoIterator<Item = Option<T>>,
    T: Into<WireSegment>,
{
    encode_planned_field_segments_in::<
        [WireSegment; crate::protocol::segments::INLINE_SEGMENTS],
        _,
        _,
    >(values, plan, layout, optional_codec)
}

/// Encodes a generated field plan with caller-selected inline segment storage.
///
/// Selecting storage changes only ownership metadata capacity. The encoded
/// bytes, validation, field ordering, and payload owners are identical.
pub(crate) fn encode_planned_field_segments_in<A, I, T>(
    values: I,
    plan: &[OperationFieldPlan],
    layout: OperationFieldLayout,
    optional_codec: Option<&crate::protocol::OptionalValueCodec>,
) -> Result<crate::protocol::SegmentFrame<A>>
where
    A: Array<Item = WireSegment>,
    I: IntoIterator<Item = Option<T>>,
    T: Into<WireSegment>,
{
    if plan.len() > crate::protocol::MAX_OPERATION_FIELDS {
        return Err(ProtocolError::InvalidFieldSequence(
            "operation field count exceeds the generated bound",
        ));
    }
    let values: SmallVec<[Option<WireSegment>; INLINE_FIELDS]> = values
        .into_iter()
        .take(plan.len().saturating_add(1))
        .map(|value| value.map(Into::into))
        .collect();
    if values.len() != plan.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "field values do not match operation plan",
        ));
    }
    for (value, field) in values.iter().zip(plan) {
        if layout == OperationFieldLayout::Dense {
            if !field.required {
                return Err(ProtocolError::InvalidFieldSequence(
                    "dense operation field is optional",
                ));
            }
            if field.encoded_width == 0 {
                return Err(ProtocolError::InvalidFieldSequence(
                    "dense operation field has no declared fixed width",
                ));
            }
        }
        if field.required && value.is_none() {
            return Err(ProtocolError::InvalidFieldSequence(
                "required operation field is missing",
            ));
        }
        if let Some(value) = value
            && field.encoded_width != 0
            && value.len() != field.encoded_width
        {
            return Err(ProtocolError::InvalidFieldSequence(
                "operation field does not match its declared fixed width",
            ));
        }
    }

    let mut segments = SmallVec::<A>::new();
    match layout {
        OperationFieldLayout::Dense => {
            if values.iter().any(Option::is_none) {
                return Err(ProtocolError::InvalidFieldSequence(
                    "dense field is missing",
                ));
            }
            segments.extend(values.into_iter().flatten());
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
            if !mask.is_empty() {
                segments.push(WireSegment::Inline(mask));
            }
            for (index, value) in values.into_iter().enumerate() {
                let Some(value) = value else {
                    continue;
                };
                if Some(index) != final_present {
                    let length = u64::try_from(value.len())
                        .map_err(|_| ProtocolError::FrameLengthOverflow)?;
                    let (encoded, encoded_len) = crate::protocol::encode_varuint(length);
                    segments.push(WireSegment::inline(&encoded[..encoded_len]));
                }
                segments.push(value);
            }
        }
        OperationFieldLayout::OptionalValues => {
            let optional_codec = optional_codec.ok_or(ProtocolError::InvalidOptionalValues(
                "optional-value layout requires an explicit codec",
            ))?;
            for value in values {
                let prefix = optional_codec.prefix(value.as_ref().map(WireSegment::len))?;
                segments.push(WireSegment::inline(
                    &prefix[8 - optional_codec.length_bytes()..],
                ));
                if let Some(value) = value {
                    segments.push(value);
                }
            }
        }
        OperationFieldLayout::Empty if values.is_empty() => {}
        OperationFieldLayout::Empty => {
            return Err(ProtocolError::InvalidFieldSequence(
                "empty layout has fields",
            ));
        }
        OperationFieldLayout::Opaque => {
            return Err(ProtocolError::InvalidFieldSequence(
                "opaque layout is not field-addressable",
            ));
        }
    }

    let frame = crate::protocol::SegmentFrame::from_segments(segments)?;
    if frame.len() > crate::protocol::MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: frame.len(),
            maximum: crate::protocol::MAX_VALUE_BYTES,
        });
    }
    Ok(frame)
}
