//! Primitive wire values and response framing shared by clients and servers.
//!
//! Request bodies are intentionally outside this crate. Version 1 has no
//! common request length field: the opcode selects an operation-specific
//! layout. The server adapter owns request delimiting and semantic validation;
//! client adapters own request construction and response payload decoding.

macro_rules! wire_enum {
    (
        $(#[$metadata:meta])*
        pub enum $name:ident {
            $($variant:ident = $value:expr),+ $(,)?
        }
        unknown => $unknown:ident
    ) => {
        $(#[$metadata])*
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(u8)]
        pub enum $name {
            $($variant = $value),+
        }

        impl TryFrom<u8> for $name {
            type Error = ProtocolError;

            fn try_from(value: u8) -> Result<Self> {
                match value {
                    $(value if value == Self::$variant as u8 => Ok(Self::$variant),)+
                    _ => Err(ProtocolError::$unknown(value)),
                }
            }
        }
    };
}

include!(concat!(env!("OUT_DIR"), "/wire_values.rs"));

/// The exact fixed-size item identifier carried by the wire protocol.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ItemId([u8; ITEM_ID_BYTES]);

impl ItemId {
    /// Wraps an exact item ID without interpreting its bytes.
    pub const fn new(bytes: [u8; ITEM_ID_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the complete item ID bytes.
    pub const fn as_bytes(&self) -> &[u8; ITEM_ID_BYTES] {
        &self.0
    }

    /// Consumes the item ID and returns its bytes.
    pub const fn into_bytes(self) -> [u8; ITEM_ID_BYTES] {
        self.0
    }
}

impl AsRef<[u8]> for ItemId {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Status {
    /// Returns whether this status represents a server-side error.
    pub const fn is_error(self) -> bool {
        (self as u8) >= ERROR_STATUS_MINIMUM
    }
}

/// Metadata required to delimit one response with an opaque payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseHeader {
    status: Status,
    encoded_len: usize,
    payload_len: usize,
}

impl ResponseHeader {
    /// Returns the decoded status.
    pub const fn status(self) -> Status {
        self.status
    }

    /// Returns the number of bytes before the opaque payload.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the payload length.
    pub const fn payload_len(self) -> usize {
        self.payload_len
    }

    /// Returns the complete response frame length.
    pub fn frame_len(self) -> Result<usize> {
        self.encoded_len
            .checked_add(self.payload_len)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }
}

fn decode_response_header(prefix: &[u8]) -> Result<Option<ResponseHeader>> {
    let Some(&status_byte) = prefix.first() else {
        return Ok(None);
    };
    let status = Status::try_from(status_byte)?;
    let Some((payload_len, encoded_len)) = decode_varuint(
        prefix.get(RESPONSE_FIXED_BYTES..).unwrap_or_default(),
        "response payload length",
    )?
    else {
        return Ok(None);
    };
    let payload_len =
        usize::try_from(payload_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
    validate_value_length(payload_len)?;
    Ok(Some(ResponseHeader {
        status,
        encoded_len: RESPONSE_FIXED_BYTES + encoded_len,
        payload_len,
    }))
}

/// A complete response viewed as an opaque status and payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseFrame<'a> {
    status: Status,
    frame: &'a [u8],
    payload_offset: usize,
}

impl<'a> ResponseFrame<'a> {
    /// Decodes one complete response without interpreting its payload.
    pub fn decode(frame: &'a [u8]) -> Result<Self> {
        let header = decode_response_header(frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: RESPONSE_FIXED_BYTES + MIN_VARUINT_BYTES,
            actual: frame.len(),
        })?;
        let expected = header.frame_len()?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        Ok(Self {
            status: header.status,
            frame,
            payload_offset: header.encoded_len,
        })
    }

    /// Returns the response status.
    pub const fn status(self) -> Status {
        self.status
    }

    /// Returns the opaque response payload.
    pub fn payload(self) -> &'a [u8] {
        &self.frame[self.payload_offset..]
    }

    /// Returns the original complete encoded frame.
    pub const fn encoded(self) -> &'a [u8] {
        self.frame
    }
}

/// Generic response frame encoder/decoder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    pub status: Status,
    pub payload: Vec<u8>,
}

impl Response {
    /// Creates a response after checking the wire payload ceiling.
    pub fn new(status: Status, payload: Vec<u8>) -> Result<Self> {
        validate_value_length(payload.len())?;
        Ok(Self { status, payload })
    }

    /// Encodes this response into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        validate_value_length(self.payload.len())?;
        let (length, length_bytes) = encode_varuint(self.payload.len() as u64);
        let mut frame =
            Vec::with_capacity(RESPONSE_FIXED_BYTES + length_bytes + self.payload.len());
        frame.push(self.status as u8);
        frame.extend_from_slice(&length[..length_bytes]);
        frame.extend_from_slice(&self.payload);
        Ok(frame)
    }

    /// Consumes and encodes this response.
    pub fn into_encoded(self) -> Result<Vec<u8>> {
        self.encode()
    }

    /// Decodes a response header when enough bytes are available.
    pub fn decode_header(prefix: &[u8]) -> Result<Option<ResponseHeader>> {
        decode_response_header(prefix)
    }

    /// Reports the complete response frame length once the header is available.
    pub fn frame_len(prefix: &[u8]) -> Result<Option<usize>> {
        Self::decode_header(prefix)?
            .map(ResponseHeader::frame_len)
            .transpose()
    }

    /// Decodes and validates one complete response frame.
    pub fn decode(frame: &[u8]) -> Result<Self> {
        let header = Self::decode_header(frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: RESPONSE_FIXED_BYTES + MIN_VARUINT_BYTES,
            actual: frame.len(),
        })?;
        let expected = header.frame_len()?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        Ok(Self {
            status: header.status,
            payload: frame[header.encoded_len..].to_vec(),
        })
    }

    /// Decodes a response while retaining a conventional owned payload.
    pub fn decode_owned(frame: Vec<u8>) -> Result<Self> {
        Self::decode(&frame)
    }
}

/// Encodes one canonical unsigned 64-bit `vu128`.
pub fn encode_varuint(value: u64) -> ([u8; MAX_VARUINT_BYTES], usize) {
    let mut encoded = [0; MAX_VARUINT_BYTES];
    let length = vu128::encode_u64(&mut encoded, value);
    (encoded, length)
}

/// Decodes one canonical unsigned 64-bit `vu128`.
///
/// `Ok(None)` means that the prefix is valid but incomplete. The context is
/// included only in malformed-input diagnostics; it does not assign semantic
/// meaning to the field.
pub fn decode_varuint(input: &[u8], context: &'static str) -> Result<Option<(u64, usize)>> {
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
    let mut canonical = [0; MAX_VARUINT_BYTES];
    let canonical_len = vu128::encode_u64(&mut canonical, value);
    if canonical_len != encoded_len || canonical[..canonical_len] != input[..encoded_len] {
        return Err(ProtocolError::NonCanonicalVaruint { context });
    }
    Ok(Some((value, encoded_len)))
}

fn validate_value_length(value_len: usize) -> Result<()> {
    if value_len > MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: value_len,
            maximum: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

/// Errors that belong to the common wire boundary.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("unknown opcode 0x{0:02x}")]
    UnknownOpcode(u8),
    #[error("unknown status 0x{0:02x}")]
    UnknownStatus(u8),
    #[error("frame is too short: expected at least {expected} bytes, got {actual}")]
    FrameTooShort { expected: usize, actual: usize },
    #[error("frame length does not match header: expected {expected} bytes, got {actual}")]
    FrameLength { expected: usize, actual: usize },
    #[error("frame length overflow")]
    FrameLengthOverflow,
    #[error("{context} uses a non-canonical vu128 encoding")]
    NonCanonicalVaruint { context: &'static str },
    #[error("{context} exceeds the supported 64-bit vu128 range")]
    VaruintOverflow { context: &'static str },
    #[error("value is too large: {size} bytes exceeds {maximum}")]
    ValueTooLarge { size: usize, maximum: usize },
}

/// Convenience result type for common wire operations.
pub type Result<T> = std::result::Result<T, ProtocolError>;
