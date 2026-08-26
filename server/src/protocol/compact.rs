//! Operation-neutral compact `vu128` and borrowed value primitives.

use crate::protocol::{MAX_VARUINT_BYTES, ProtocolError, Result};

/// Encodes one canonical unsigned 64-bit `vu128`.
pub fn encode_vu128(value: u64) -> ([u8; MAX_VARUINT_BYTES], usize) {
    let mut encoded = [0; MAX_VARUINT_BYTES];
    let length = vu128::encode_u64(&mut encoded, value);
    (encoded, length)
}

/// Decodes one canonical unsigned 64-bit `vu128`.
///
/// `Ok(None)` means that the prefix is valid but incomplete.
pub fn decode_vu128(input: &[u8], context: &'static str) -> Result<Option<(u64, usize)>> {
    let Some(&first) = input.first() else {
        return Ok(None);
    };
    let encoded_len = vu128::encoded_len(first);
    if encoded_len > MAX_VARUINT_BYTES {
        return Err(ProtocolError::VaruintOverflow { context });
    }
    if input.len() < encoded_len {
        return Ok(None);
    }
    let mut encoded = [0; MAX_VARUINT_BYTES];
    encoded[..encoded_len].copy_from_slice(&input[..encoded_len]);
    let (value, decoded_len) = vu128::decode_u64(&encoded);
    if decoded_len != encoded_len {
        return Err(ProtocolError::NonCanonicalVaruint { context });
    }
    let (canonical, canonical_len) = encode_vu128(value);
    if canonical_len != encoded_len || canonical[..canonical_len] != input[..encoded_len] {
        return Err(ProtocolError::NonCanonicalVaruint { context });
    }
    Ok(Some((value, encoded_len)))
}

/// Backward-compatible wire-level spelling.
pub use decode_vu128 as decode_varuint;
/// Backward-compatible wire-level spelling.
pub use encode_vu128 as encode_varuint;

/// A borrowed value and the cursor immediately after its canonical length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LengthDelimitedView<'a> {
    /// Value bytes borrowed from the original payload.
    pub value: &'a [u8],
    /// Offset at which the next value begins.
    pub next: usize,
}

/// Reads one canonical length-delimited value without allocating.
pub fn read_length_delimited(
    payload: &[u8],
    cursor: usize,
) -> Result<Option<LengthDelimitedView<'_>>> {
    let Some((length, encoded_len)) = decode_vu128(
        payload.get(cursor..).unwrap_or_default(),
        "compact value length",
    )?
    else {
        return Ok(None);
    };
    let value_start = cursor
        .checked_add(encoded_len)
        .ok_or(ProtocolError::FrameLengthOverflow)?;
    let value_len = usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
    let value_end = value_start
        .checked_add(value_len)
        .ok_or(ProtocolError::FrameLengthOverflow)?;
    let Some(value) = payload.get(value_start..value_end) else {
        return Ok(None);
    };
    Ok(Some(LengthDelimitedView {
        value,
        next: value_end,
    }))
}

/// Borrowed view over the generic field-sequence framing implementation.
pub type FieldSequenceView<'a, 'b> = crate::protocol::FieldSequence<'a, 'b>;
