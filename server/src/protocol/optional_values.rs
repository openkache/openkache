//! Configurable fixed-width optional-value codec.

use crate::protocol::{ProtocolError, Result};

/// Optional-value framing configured by the API that owns the wire contract.
///
/// `missing` is reserved for absent values. A present value may use every
/// representable length except that one sentinel, including zero when the
/// sentinel is non-zero. The width and sentinel are runtime configuration so
/// each API can choose the compact contract it actually specifies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionalValueCodec {
    length_bytes: usize,
    missing: u64,
}

impl OptionalValueCodec {
    /// Creates a fixed-width codec with an API-selected missing sentinel.
    pub const fn new(length_bytes: usize, missing: u64) -> Result<Self> {
        if length_bytes == 0 || length_bytes > 8 {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value length width must be between one and eight bytes",
            ));
        }
        let maximum = if length_bytes == 8 {
            u64::MAX
        } else {
            (1u64 << (length_bytes * 8)) - 1
        };
        if missing > maximum {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value missing sentinel does not fit its prefix width",
            ));
        }
        Ok(Self {
            length_bytes,
            missing,
        })
    }

    /// Returns the configured prefix width.
    pub const fn length_bytes(self) -> usize {
        self.length_bytes
    }

    /// Returns the reserved missing sentinel.
    pub const fn missing(self) -> u64 {
        self.missing
    }

    /// Encodes one length prefix into the low bytes of a big-endian word.
    pub fn prefix(self, value_len: Option<usize>) -> Result<[u8; 8]> {
        let length = match value_len {
            None => self.missing,
            Some(length) => {
                let length =
                    u64::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
                let maximum = if self.length_bytes == 8 {
                    u64::MAX
                } else {
                    (1u64 << (self.length_bytes * 8)) - 1
                };
                if length > maximum {
                    return Err(ProtocolError::ValueTooLarge {
                        size: usize::try_from(length).unwrap_or(usize::MAX),
                        maximum: usize::try_from(maximum).unwrap_or(usize::MAX),
                    });
                }
                if length == self.missing {
                    return Err(ProtocolError::ValueTooLarge {
                        size: usize::try_from(length).unwrap_or(usize::MAX),
                        maximum: usize::try_from(maximum).unwrap_or(usize::MAX),
                    });
                }
                length
            }
        };
        Ok(length.to_be_bytes())
    }

    fn decode_length(self, prefix: &[u8]) -> Result<Option<usize>> {
        if prefix.len() != self.length_bytes {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value prefix has the wrong width",
            ));
        }
        let mut bytes = [0u8; 8];
        bytes[8 - self.length_bytes..].copy_from_slice(prefix);
        let length = u64::from_be_bytes(bytes);
        if length == self.missing {
            return Ok(None);
        }
        Ok(Some(
            usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?,
        ))
    }

    /// Returns the exact encoded size for values.
    pub fn encoded_len(self, values: &[Option<&[u8]>]) -> Result<usize> {
        let mut total = 0usize;
        for value in values {
            self.prefix(value.map(<[u8]>::len))?;
            total = total
                .checked_add(self.length_bytes)
                .and_then(|total| total.checked_add(value.map_or(0, <[u8]>::len)))
                .ok_or(ProtocolError::FrameLengthOverflow)?;
        }
        crate::protocol::validate_value_length(total)?;
        Ok(total)
    }

    /// Encodes an ordered optional-value sequence.
    pub fn encode(self, values: &[Option<&[u8]>]) -> Result<Vec<u8>> {
        let expected = self.encoded_len(values)?;
        let mut encoder = OptionalValuesEncoder::with_codec(self, values.len());
        for value in values {
            encoder.push(*value)?;
        }
        let output = encoder.finish()?;
        debug_assert_eq!(expected, output.len());
        Ok(output)
    }

    /// Decodes a borrowed optional-value sequence into caller-owned offsets.
    pub fn decode<'a, 'b>(
        self,
        payload: &'a [u8],
        value_count: usize,
        offsets: &'b mut [(usize, usize)],
    ) -> Result<OptionalValues<'a, 'b>> {
        if offsets.len() < value_count {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value offset storage is too small",
            ));
        }
        crate::protocol::validate_value_length(payload.len())?;
        let minimum = value_count
            .checked_mul(self.length_bytes)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        if payload.len() < minimum {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value payload is missing an entry length",
            ));
        }
        let mut cursor = 0usize;
        for index in 0..value_count {
            let prefix_end = cursor
                .checked_add(self.length_bytes)
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            let length = self.decode_length(&payload[cursor..prefix_end])?;
            cursor = prefix_end;
            let Some(length) = length else {
                offsets[index] = (usize::MAX, usize::MAX);
                continue;
            };
            crate::protocol::validate_value_length(length)?;
            let end = cursor
                .checked_add(length)
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            if end > payload.len() {
                return Err(ProtocolError::InvalidOptionalValues(
                    "optional-value payload entry is truncated",
                ));
            }
            offsets[index] = (cursor, end);
            cursor = end;
        }
        if cursor != payload.len() {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value payload contains trailing bytes",
            ));
        }
        Ok(OptionalValues {
            payload,
            offsets: &offsets[..value_count],
        })
    }
}

/// Incremental encoder for one configured optional-value layout.
#[derive(Debug)]
pub struct OptionalValuesEncoder {
    codec: OptionalValueCodec,
    payload: Vec<u8>,
    expected_fields: usize,
    written_fields: usize,
}

impl OptionalValuesEncoder {
    /// Creates an encoder with an API-selected codec.
    pub fn with_codec(codec: OptionalValueCodec, field_count: usize) -> Self {
        Self {
            codec,
            payload: Vec::with_capacity(
                field_count
                    .saturating_mul(codec.length_bytes())
                    .min(crate::protocol::MAX_VALUE_BYTES),
            ),
            expected_fields: field_count,
            written_fields: 0,
        }
    }

    /// Appends one present or missing field.
    pub fn push(&mut self, value: Option<&[u8]>) -> Result<()> {
        if self.written_fields >= self.expected_fields {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value encoder received too many fields",
            ));
        }
        let prefix = self.codec.prefix(value.map(<[u8]>::len))?;
        let next_len = self
            .payload
            .len()
            .checked_add(self.codec.length_bytes())
            .and_then(|length| length.checked_add(value.map_or(0, <[u8]>::len)))
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        crate::protocol::validate_value_length(next_len)?;
        self.payload
            .extend_from_slice(&prefix[8 - self.codec.length_bytes()..]);
        if let Some(value) = value {
            self.payload.extend_from_slice(value);
        }
        self.written_fields += 1;
        Ok(())
    }

    /// Completes the sequence after all fields have been appended.
    pub fn finish(self) -> Result<Vec<u8>> {
        if self.written_fields != self.expected_fields {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value encoder received too few fields",
            ));
        }
        Ok(self.payload)
    }
}

/// Borrowed view over a configured optional-value sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionalValues<'a, 'b> {
    payload: &'a [u8],
    offsets: &'b [(usize, usize)],
}

impl<'a, 'b> OptionalValues<'a, 'b> {
    /// Returns the number of values.
    pub const fn len(self) -> usize {
        self.offsets.len()
    }

    /// Returns one present value, preserving present-empty as `Some(&[])`.
    pub fn get(self, index: usize) -> Option<&'a [u8]> {
        let (start, end) = *self.offsets.get(index)?;
        (start != usize::MAX).then(|| &self.payload[start..end])
    }
}

/// Encodes optional values with a caller-supplied layout.
pub fn encode_optional_values(
    codec: OptionalValueCodec,
    values: &[Option<&[u8]>],
) -> Result<Vec<u8>> {
    codec.encode(values)
}

/// Computes an exact optional-value payload size from values.
pub fn optional_values_encoded_len(
    codec: OptionalValueCodec,
    values: &[Option<&[u8]>],
) -> Result<usize> {
    codec.encoded_len(values)
}

/// Computes an exact optional-value payload size from lengths.
pub fn optional_values_encoded_len_from_lengths(
    codec: OptionalValueCodec,
    lengths: &[Option<usize>],
) -> Result<usize> {
    let mut total = 0usize;
    for length in lengths {
        codec.prefix(*length)?;
        total = total
            .checked_add(codec.length_bytes())
            .and_then(|total| total.checked_add(length.unwrap_or(0)))
            .ok_or(ProtocolError::FrameLengthOverflow)?;
    }
    crate::protocol::validate_value_length(total)?;
    Ok(total)
}

/// Returns an upper bound for a configured optional-value payload.
pub fn optional_values_max_encoded_len(
    codec: OptionalValueCodec,
    value_count: usize,
    max_value_bytes: usize,
) -> Option<usize> {
    value_count.checked_mul(codec.length_bytes().checked_add(max_value_bytes)?)
}

/// Decodes an owned optional-value sequence.
pub fn decode_optional_values(
    codec: OptionalValueCodec,
    payload: &[u8],
    value_count: usize,
) -> Result<Vec<Option<Vec<u8>>>> {
    let mut offsets = vec![(usize::MAX, usize::MAX); value_count];
    let view = codec.decode(payload, value_count, &mut offsets)?;
    Ok((0..view.len())
        .map(|index| view.get(index).map(ToOwned::to_owned))
        .collect())
}
