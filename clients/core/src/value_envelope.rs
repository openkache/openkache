//! Canonical framing for self-describing, cross-language values.

/// Magic bytes and version written at the start of every value envelope.
pub const MAGIC_AND_VERSION: [u8; 4] = [0x4f, 0x4b, 0x56, 0x01];

/// Maximum UTF-8 byte length of an encoding identifier.
pub const MAX_ENCODING_BYTES: usize = 64;

/// Maximum UTF-8 byte length of a logical type name.
pub const MAX_TYPE_NAME_BYTES: usize = u16::MAX as usize;

const HEADER_BYTES: usize = 8;

/// A decoded OpenKache value envelope borrowing its source bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValueEnvelope<'a> {
    /// Stable codec identifier, such as `json`, `protobuf`, or `flatbuffers`.
    pub encoding: &'a str,
    /// Codec-defined logical type, such as a fully qualified Protobuf message name.
    pub type_name: &'a str,
    /// Exact codec-specific payload bytes.
    pub payload: &'a [u8],
}

/// Value-envelope validation and encoding errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The encoding identifier does not follow the portable identifier grammar.
    #[error("invalid value encoding {encoding:?}")]
    InvalidEncoding {
        /// Rejected encoding identifier.
        encoding: String,
    },
    /// The logical type name cannot fit in the envelope header.
    #[error("value type name contains {size} bytes, maximum is {maximum}")]
    TypeNameTooLong {
        /// Actual UTF-8 byte length.
        size: usize,
        /// Maximum representable byte length.
        maximum: usize,
    },
    /// The envelope length cannot be represented by the current platform.
    #[error("value envelope length overflow")]
    LengthOverflow,
    /// The output allocation could not be reserved.
    #[error("failed to allocate {size} bytes for value envelope")]
    Allocation {
        /// Requested allocation size.
        size: usize,
    },
    /// The input does not contain a complete envelope header.
    #[error("value envelope contains {size} bytes, header requires {minimum}")]
    HeaderTruncated {
        /// Actual input size.
        size: usize,
        /// Minimum complete header size.
        minimum: usize,
    },
    /// The input uses a different magic value or envelope version.
    #[error("unsupported value envelope magic or version {found:02x?}")]
    UnsupportedMagicOrVersion {
        /// Four-byte prefix found in the input.
        found: [u8; 4],
    },
    /// The declared encoding and type-name bytes are not present.
    #[error("value envelope metadata requires {required} bytes, input contains {actual}")]
    MetadataTruncated {
        /// Bytes required by the declared metadata lengths.
        required: usize,
        /// Bytes available in the input.
        actual: usize,
    },
    /// The encoding identifier is not valid UTF-8.
    #[error("value encoding is not valid UTF-8: {0}")]
    EncodingUtf8(#[source] std::str::Utf8Error),
    /// The logical type name is not valid UTF-8.
    #[error("value type name is not valid UTF-8: {0}")]
    TypeNameUtf8(#[source] std::str::Utf8Error),
}

/// Convenience alias for value-envelope results.
pub type Result<T> = std::result::Result<T, Error>;

/// Encodes codec metadata and payload bytes into the canonical value envelope.
///
/// # Arguments
///
/// * `encoding` - Portable codec identifier matching `[a-z][a-z0-9.-]{0,63}`.
/// * `type_name` - Codec-defined UTF-8 logical type name.
/// * `payload` - Exact codec-specific bytes.
///
/// # Returns
///
/// An owned envelope using big-endian metadata lengths.
///
/// # Errors
///
/// Returns an error when the encoding identifier is invalid, the type name is
/// longer than 65,535 bytes, the total length overflows, or allocation fails.
pub fn encode(encoding: &str, type_name: &str, payload: &[u8]) -> Result<Vec<u8>> {
    validate_encoding(encoding)?;
    let type_name_length = u16::try_from(type_name.len()).map_err(|_| Error::TypeNameTooLong {
        size: type_name.len(),
        maximum: MAX_TYPE_NAME_BYTES,
    })?;
    let total_length = HEADER_BYTES
        .checked_add(encoding.len())
        .and_then(|length| length.checked_add(type_name.len()))
        .and_then(|length| length.checked_add(payload.len()))
        .ok_or(Error::LengthOverflow)?;

    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(total_length)
        .map_err(|_| Error::Allocation { size: total_length })?;
    bytes.extend_from_slice(&MAGIC_AND_VERSION);
    bytes.extend_from_slice(
        &u16::try_from(encoding.len())
            .expect("validated encoding length fits in u16")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(&type_name_length.to_be_bytes());
    bytes.extend_from_slice(encoding.as_bytes());
    bytes.extend_from_slice(type_name.as_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// Decodes and validates a canonical value envelope without copying its payload.
///
/// # Arguments
///
/// * `bytes` - Complete value bytes returned by a raw cache operation.
///
/// # Returns
///
/// Borrowed encoding, logical type, and payload slices.
///
/// # Errors
///
/// Returns an error when the header, version, declared lengths, UTF-8 metadata,
/// or encoding identifier is invalid.
pub fn decode(bytes: &[u8]) -> Result<ValueEnvelope<'_>> {
    if bytes.len() < HEADER_BYTES {
        return Err(Error::HeaderTruncated {
            size: bytes.len(),
            minimum: HEADER_BYTES,
        });
    }

    let found = <[u8; 4]>::try_from(&bytes[..MAGIC_AND_VERSION.len()])
        .expect("validated envelope header contains magic bytes");
    if found != MAGIC_AND_VERSION {
        return Err(Error::UnsupportedMagicOrVersion { found });
    }

    let encoding_length = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let type_name_length = u16::from_be_bytes([bytes[6], bytes[7]]) as usize;
    let payload_offset = HEADER_BYTES
        .checked_add(encoding_length)
        .and_then(|length| length.checked_add(type_name_length))
        .ok_or(Error::LengthOverflow)?;
    if payload_offset > bytes.len() {
        return Err(Error::MetadataTruncated {
            required: payload_offset,
            actual: bytes.len(),
        });
    }

    let encoding_end = HEADER_BYTES + encoding_length;
    let encoding =
        std::str::from_utf8(&bytes[HEADER_BYTES..encoding_end]).map_err(Error::EncodingUtf8)?;
    validate_encoding(encoding)?;
    let type_name =
        std::str::from_utf8(&bytes[encoding_end..payload_offset]).map_err(Error::TypeNameUtf8)?;

    Ok(ValueEnvelope {
        encoding,
        type_name,
        payload: &bytes[payload_offset..],
    })
}

fn validate_encoding(encoding: &str) -> Result<()> {
    let bytes = encoding.as_bytes();
    let valid = (1..=MAX_ENCODING_BYTES).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..].iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        });
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidEncoding {
            encoding: encoding.to_owned(),
        })
    }
}
