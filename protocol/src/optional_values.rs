//! Generic compact optional-value field layout.
//!
//! Protocol-v1 compatibility re-exports these primitives for source
//! compatibility; descriptor-selected generic layouts use the same codec.

use crate::{validate_value_length, MAX_VALUE_BYTES, ProtocolError, Result};

pub const OPTIONAL_VALUE_LENGTH_BYTES: usize = 4;
pub const OPTIONAL_VALUE_MISSING: u32 = u32::MAX;

fn invalid(message: &'static str) -> ProtocolError {
    ProtocolError::InvalidFieldSequence(message)
}

/// Encodes one optional-value length word.
///
/// `None` is the missing sentinel; `Some(0)` is a present-empty value. Keeping
/// this distinction in the shared codec prevents server/client adapters from
/// duplicating the compact framing constants.
pub fn optional_value_prefix(value_len: Option<usize>) -> Result<[u8; OPTIONAL_VALUE_LENGTH_BYTES]> {
    let length = match value_len {
        None => OPTIONAL_VALUE_MISSING,
        Some(length) if length < OPTIONAL_VALUE_MISSING as usize => length as u32,
        Some(length) => {
            return Err(ProtocolError::ValueTooLarge {
                size: length,
                maximum: (OPTIONAL_VALUE_MISSING - 1) as usize,
            });
        }
    };
    Ok(length.to_be_bytes())
}

pub fn encode_optional_values(values: &[Option<&[u8]>]) -> Result<Vec<u8>> {
    let mut encoder = OptionalValuesEncoder::new(values.len());
    for value in values {
        encoder.push(*value)?;
    }
    encoder.finish()
}

pub fn optional_values_encoded_len(values: &[Option<&[u8]>]) -> Result<usize> {
    let mut encoded_len = 0usize;
    for value in values {
        let value_len = value.map(<[u8]>::len).unwrap_or(0);
        if value_len >= OPTIONAL_VALUE_MISSING as usize {
            return Err(ProtocolError::ValueTooLarge {
                size: value_len,
                maximum: (OPTIONAL_VALUE_MISSING - 1) as usize,
            });
        }
        encoded_len = encoded_len
            .checked_add(OPTIONAL_VALUE_LENGTH_BYTES)
            .and_then(|length| length.checked_add(value_len))
            .ok_or(ProtocolError::FrameLengthOverflow)?;
    }
    validate_value_length(encoded_len)?;
    Ok(encoded_len)
}

pub fn optional_values_encoded_len_from_lengths(lengths: &[Option<usize>]) -> Result<usize> {
    let mut encoded_len = 0usize;
    for value_len in lengths.iter().copied().map(|length| length.unwrap_or(0)) {
        if value_len >= OPTIONAL_VALUE_MISSING as usize {
            return Err(ProtocolError::ValueTooLarge {
                size: value_len,
                maximum: (OPTIONAL_VALUE_MISSING - 1) as usize,
            });
        }
        encoded_len = encoded_len
            .checked_add(OPTIONAL_VALUE_LENGTH_BYTES)
            .and_then(|length| length.checked_add(value_len))
            .ok_or(ProtocolError::FrameLengthOverflow)?;
    }
    validate_value_length(encoded_len)?;
    Ok(encoded_len)
}

#[derive(Debug)]
pub struct OptionalValuesEncoder {
    payload: Vec<u8>,
    expected_fields: usize,
    written_fields: usize,
}

impl OptionalValuesEncoder {
    pub fn new(field_count: usize) -> Self {
        Self {
            payload: Vec::with_capacity(
                field_count
                    .saturating_mul(OPTIONAL_VALUE_LENGTH_BYTES)
                    .min(MAX_VALUE_BYTES),
            ),
            expected_fields: field_count,
            written_fields: 0,
        }
    }

    pub fn push(&mut self, value: Option<&[u8]>) -> Result<()> {
        if self.written_fields >= self.expected_fields {
            return Err(invalid("optional-value encoder received too many fields"));
        }
        let value_len = value.map(<[u8]>::len);
        let prefix = optional_value_prefix(value_len)?;
        let next_len = self
            .payload
            .len()
            .checked_add(OPTIONAL_VALUE_LENGTH_BYTES)
            .and_then(|length| length.checked_add(value_len.unwrap_or(0)))
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        validate_value_length(next_len)?;
        self.payload.extend_from_slice(&prefix);
        if let Some(value) = value {
            self.payload.extend_from_slice(value);
        }
        self.written_fields += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<Vec<u8>> {
        if self.written_fields != self.expected_fields {
            return Err(invalid("optional-value encoder received too few fields"));
        }
        Ok(self.payload)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionalValues<'a, 'b> {
    payload: &'a [u8],
    offsets: &'b [(usize, usize)],
    field_count: usize,
}

impl<'a, 'b> OptionalValues<'a, 'b> {
    pub fn decode(
        payload: &'a [u8],
        value_count: usize,
        offsets: &'b mut [(usize, usize)],
    ) -> Result<Self> {
        if offsets.len() < value_count {
            return Err(invalid(
                "optional-value offset storage is smaller than the modeled field count",
            ));
        }
        validate_value_length(payload.len())?;
        let minimum = value_count
            .checked_mul(OPTIONAL_VALUE_LENGTH_BYTES)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        if payload.len() < minimum {
            return Err(invalid("optional-value payload is missing an entry length"));
        }
        let mut cursor = 0usize;
        for index in 0..value_count {
            let prefix_end = cursor
                .checked_add(OPTIONAL_VALUE_LENGTH_BYTES)
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            let length = u32::from_be_bytes(
                payload[cursor..prefix_end]
                    .try_into()
                    .expect("optional-value length prefix has fixed width"),
            );
            cursor = prefix_end;
            if length == OPTIONAL_VALUE_MISSING {
                offsets[index] = (usize::MAX, usize::MAX);
                continue;
            }
            let length = usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
            let end = cursor
                .checked_add(length)
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            if end > payload.len() {
                return Err(invalid("optional-value payload entry is truncated"));
            }
            offsets[index] = (cursor, end);
            cursor = end;
        }
        if cursor != payload.len() {
            return Err(invalid("optional-value payload contains trailing bytes"));
        }
        Ok(Self {
            payload,
            offsets: &offsets[..value_count],
            field_count: value_count,
        })
    }

    pub const fn len(self) -> usize {
        self.field_count
    }

    pub fn get(self, index: usize) -> Option<&'a [u8]> {
        let (start, end) = *self.offsets.get(index)?;
        (start != usize::MAX).then(|| &self.payload[start..end])
    }
}

pub fn decode_optional_values(payload: &[u8], value_count: usize) -> Result<Vec<Option<Vec<u8>>>> {
    let mut offsets = vec![(usize::MAX, usize::MAX); value_count];
    let view = OptionalValues::decode(payload, value_count, &mut offsets)?;
    Ok((0..view.len())
        .map(|index| view.get(index).map(ToOwned::to_owned))
        .collect())
}

pub fn optional_values_max_encoded_len(
    value_count: usize,
    max_value_bytes: usize,
) -> Option<usize> {
    value_count.checked_mul(OPTIONAL_VALUE_LENGTH_BYTES.checked_add(max_value_bytes)?)
}
