//! Shared application-value codec adapters.
//!
//! Operation implementations deal in decoded values and transformations. The
//! byte order, UTF-8 validation, and payload-shape checks live here so an API
//! extension never needs to duplicate protocol framing details.

#![allow(dead_code)]

/// Decodes one application payload as UTF-8 without allocating.
pub(super) fn decode_utf8(payload: &[u8]) -> Result<&str, &'static [u8]> {
    std::str::from_utf8(payload).map_err(|_| b"application value must be valid UTF-8" as _)
}

/// Decodes a fixed-width big-endian unsigned integer from a generic field.
pub(super) fn decode_u64_be(payload: &[u8]) -> Result<u64, &'static [u8]> {
    let bytes: [u8; std::mem::size_of::<u64>()] = payload
        .try_into()
        .map_err(|_| b"u64_be field must contain exactly eight bytes" as &'static [u8])?;
    Ok(u64::from_be_bytes(bytes))
}

/// Decodes one fixed-width big-endian signed 32-bit integer.
pub(super) fn decode_i32_be(payload: &[u8]) -> Result<i32, &'static [u8]> {
    let bytes: [u8; std::mem::size_of::<i32>()] = payload
        .try_into()
        .map_err(|_| b"i32_be field must contain exactly four bytes" as &'static [u8])?;
    Ok(i32::from_be_bytes(bytes))
}

/// Decodes one fixed-width big-endian binary64 value.
pub(super) fn decode_f64_be(payload: &[u8]) -> Result<f64, &'static [u8]> {
    let bytes: [u8; std::mem::size_of::<f64>()] = payload
        .try_into()
        .map_err(|_| b"f64_be field must contain exactly eight bytes" as &'static [u8])?;
    Ok(f64::from_be_bytes(bytes))
}

/// Decodes the canonical one-byte boolean used by generic field sequences.
pub(super) fn decode_bool(payload: &[u8]) -> Result<bool, &'static [u8]> {
    match payload {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(b"boolean field must contain exactly one byte, either 0 or 1"),
    }
}

/// Encodes one fixed-width big-endian unsigned 64-bit integer.
pub(super) fn encode_u64_be(value: u64) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

/// Encodes one fixed-width big-endian signed 32-bit integer.
pub(super) fn encode_i32_be(value: i32) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

/// Encodes one fixed-width big-endian binary64 value.
pub(super) fn encode_f64_be(value: f64) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

/// Encodes the canonical one-byte boolean used by generic field sequences.
pub(super) fn encode_bool(value: bool) -> Vec<u8> {
    vec![u8::from(value)]
}

/// Encodes one UTF-8 application value.
pub(super) fn encode_utf8(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}

/// Applies a server-owned transform to a packed big-endian binary64 array.
///
/// The transform receives host-endian `f64` values and returns `None` when the
/// domain result is invalid (for example, a non-finite overflow). This keeps
/// endian conversion and output framing on the shared codec path while
/// preserving a single allocation for the transformed payload.
pub(super) fn transform_packed_f64_be(
    payload: &[u8],
    mut transform: impl FnMut(f64) -> Option<f64>,
) -> Result<Vec<u8>, &'static [u8]> {
    const F64_BYTES: usize = std::mem::size_of::<f64>();
    if payload.len() % F64_BYTES != 0 {
        return Err(b"packed_f64_be payload length must be a multiple of eight");
    }
    let mut output = Vec::with_capacity(payload.len());
    for chunk in payload.chunks_exact(F64_BYTES) {
        let input = f64::from_be_bytes(
            chunk
                .try_into()
                .expect("chunks_exact returned a fixed-width chunk"),
        );
        let value = transform(input)
            .filter(|value| value.is_finite())
            .ok_or(b"packed_f64_be transform produced a non-finite value" as &'static [u8])?;
        output.extend_from_slice(&value.to_be_bytes());
    }
    Ok(output)
}
