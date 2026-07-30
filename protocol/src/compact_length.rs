//! Canonical one-or-two-byte encoding for lengths no greater than 4 KiB.

/// Largest value representable by the compact-length encoding.
pub const MAX_COMPACT_LENGTH: u16 = 4 * 1024;
/// Largest encoded compact-length representation.
pub const MAX_COMPACT_LENGTH_BYTES: usize = 2;

const TWO_BYTE_TAG: u8 = 0xf0;
const ONE_BYTE_VALUES: u16 = TWO_BYTE_TAG as u16;

/// A validated compact-length byte sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodedCompactLength {
    bytes: [u8; MAX_COMPACT_LENGTH_BYTES],
    len: u8,
}

impl EncodedCompactLength {
    /// Returns the canonical encoded bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    /// Returns the encoded length in bytes.
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Returns whether the encoded representation is empty.
    pub const fn is_empty(&self) -> bool {
        false
    }
}

/// Compact-length codec failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CompactLengthError {
    /// An application value exceeds the format ceiling.
    #[error("compact length {value} exceeds the maximum {maximum}")]
    ValueOutOfRange { value: usize, maximum: u16 },
    /// The input ends before a complete encoded value.
    #[error("compact length is truncated: expected {expected} bytes, got {actual}")]
    Truncated { expected: usize, actual: usize },
    /// A two-byte representation uses the reserved range above 4 KiB.
    #[error("compact length encoding resolves to reserved value {value}")]
    ReservedEncoding { value: u16 },
}

/// Encodes one length in its canonical one-or-two-byte representation.
///
/// Values below 240 encode directly as one byte. Larger values encode their
/// offset from 240 as a 12-bit big-endian payload below an `0xf0` tag nibble.
///
/// # Arguments
///
/// * `value` - Length in bytes, in the inclusive range `0..=4096`.
///
/// # Returns
///
/// A validated representation whose [`EncodedCompactLength::as_bytes`] method
/// exposes the complete canonical encoding.
///
/// # Errors
///
/// Returns [`CompactLengthError::ValueOutOfRange`] when `value` exceeds 4096.
pub fn encode_compact_length(value: usize) -> Result<EncodedCompactLength, CompactLengthError> {
    if value > usize::from(MAX_COMPACT_LENGTH) {
        return Err(CompactLengthError::ValueOutOfRange {
            value,
            maximum: MAX_COMPACT_LENGTH,
        });
    }
    let value = value as u16;
    if value < ONE_BYTE_VALUES {
        return Ok(EncodedCompactLength {
            bytes: [value as u8, 0],
            len: 1,
        });
    }

    let payload = value - ONE_BYTE_VALUES;
    Ok(EncodedCompactLength {
        bytes: [TWO_BYTE_TAG | ((payload >> 8) as u8), payload as u8],
        len: 2,
    })
}

/// Decodes the first compact length in a byte slice.
///
/// # Arguments
///
/// * `input` - Bytes beginning with one canonical compact-length value.
///
/// # Returns
///
/// The decoded length and the number of input bytes consumed.
///
/// # Errors
///
/// Returns [`CompactLengthError::Truncated`] when `input` does not contain a
/// complete value, or [`CompactLengthError::ReservedEncoding`] when the bytes
/// represent a value above 4096.
pub fn decode_compact_length(input: &[u8]) -> Result<(u16, usize), CompactLengthError> {
    let Some(&first) = input.first() else {
        return Err(CompactLengthError::Truncated {
            expected: 1,
            actual: 0,
        });
    };
    if first < TWO_BYTE_TAG {
        return Ok((u16::from(first), 1));
    }
    if input.len() < MAX_COMPACT_LENGTH_BYTES {
        return Err(CompactLengthError::Truncated {
            expected: MAX_COMPACT_LENGTH_BYTES,
            actual: input.len(),
        });
    }

    let payload = (u16::from(first & !TWO_BYTE_TAG) << 8) | u16::from(input[1]);
    let value = ONE_BYTE_VALUES + payload;
    if value > MAX_COMPACT_LENGTH {
        return Err(CompactLengthError::ReservedEncoding { value });
    }
    Ok((value, MAX_COMPACT_LENGTH_BYTES))
}
