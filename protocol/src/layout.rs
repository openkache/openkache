//! Generic field-layout primitives.
//!
//! An API owns its ordered field list, requiredness, and semantic codecs. This
//! module only handles byte-level layouts and lets callers select the exact
//! primitive for each operation.

use crate::{ProtocolError, ResponseSegment, Result, encode_field_sequence};
use smallvec::SmallVec;

const INLINE_FIELDS: usize = 8;

/// Value that can be appended to a segmented response without coalescing it.
pub trait LayoutValue {
    /// Returns the number of bytes written for this value.
    fn encoded_len(&self) -> usize;
}

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
    crate::validate_value_length(capacity)?;
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

/// Encodes a field sequence while retaining ownership of value segments.
pub fn encode_field_sequence_segments<T, F>(
    values: SmallVec<[Option<T>; INLINE_FIELDS]>,
    mut append_value: F,
) -> Result<SmallVec<[ResponseSegment; INLINE_FIELDS]>>
where
    T: LayoutValue,
    F: FnMut(&mut SmallVec<[ResponseSegment; INLINE_FIELDS]>, T),
{
    let mut mask = SmallVec::<[u8; 32]>::new();
    mask.resize(values.len().saturating_add(7) / 8, 0);
    let final_present = values.iter().rposition(Option::is_some);
    for (index, value) in values.iter().enumerate() {
        if value.is_some() {
            mask[index / 8] |= 1 << (index % 8);
        }
    }
    let mut segments = SmallVec::new();
    segments.push(ResponseSegment::Inline(mask));
    for (index, value) in values.into_iter().enumerate() {
        let Some(value) = value else {
            continue;
        };
        if Some(index) != final_present {
            let length = u64::try_from(value.encoded_len())
                .map_err(|_| ProtocolError::FrameLengthOverflow)?;
            let (encoded, encoded_len) = crate::encode_varuint(length);
            segments.push(ResponseSegment::inline(&encoded[..encoded_len]));
        }
        append_value(&mut segments, value);
    }
    Ok(segments)
}

/// Encodes an API-configured optional-value sequence while retaining value
/// ownership.
pub fn encode_optional_value_segments<T, F>(
    values: SmallVec<[Option<T>; INLINE_FIELDS]>,
    codec: &crate::OptionalValueCodec,
    mut append_value: F,
) -> Result<SmallVec<[ResponseSegment; INLINE_FIELDS]>>
where
    T: LayoutValue,
    F: FnMut(&mut SmallVec<[ResponseSegment; INLINE_FIELDS]>, T),
{
    let mut segments = SmallVec::new();
    for value in values {
        let prefix = codec.prefix(value.as_ref().map(LayoutValue::encoded_len))?;
        segments.push(ResponseSegment::inline(&prefix[..codec.length_bytes()]));
        if let Some(value) = value {
            append_value(&mut segments, value);
        }
    }
    Ok(segments)
}
