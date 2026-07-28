//! Binary request and response framing shared by OpenKache clients and servers.

use sha2::{Digest, Sha256};

/// QUIC application protocol identifier for persistent request lanes.
pub const ALPN: &[u8] = b"openkache/2";
/// Bytes in an encoded request header.
pub const REQUEST_HEADER_BYTES: usize = 9;
/// Bytes in an encoded response header.
pub const RESPONSE_HEADER_BYTES: usize = 5;
/// Bytes in every client-computed SHA-256 key digest.
pub const CLIENT_KEY_DIGEST_BYTES: usize = 32;
/// Maximum value or response payload size accepted by the smoke server.
pub const MAX_VALUE_BYTES: usize = 16 * 1024 * 1024;
/// Maximum complete request frame size.
pub const MAX_REQUEST_FRAME_BYTES: usize =
    REQUEST_HEADER_BYTES + CLIENT_KEY_DIGEST_BYTES + MAX_VALUE_BYTES;
/// Maximum complete response frame size.
pub const MAX_RESPONSE_FRAME_BYTES: usize = RESPONSE_HEADER_BYTES + MAX_VALUE_BYTES;

const VALUE_LENGTH_MASK: u32 = (1 << 30) - 1;
const VALUE_COMPRESSED_BIT: u32 = 1 << 31;
const VALUE_ENCRYPTED_BIT: u32 = 1 << 30;

/// Operations supported by protocol v2.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Opcode {
    Ping = 0x01,
    Get = 0x02,
    Set = 0x03,
    Delete = 0x04,
    Stats = 0x05,
    Sync = 0x06,
}

impl TryFrom<u8> for Opcode {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0x01 => Ok(Self::Ping),
            0x02 => Ok(Self::Get),
            0x03 => Ok(Self::Set),
            0x04 => Ok(Self::Delete),
            0x05 => Ok(Self::Stats),
            0x06 => Ok(Self::Sync),
            _ => Err(ProtocolError::UnknownOpcode(value)),
        }
    }
}

/// Status returned in every protocol response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Status {
    Ok = 0x00,
    NotFound = 0x01,
    Created = 0x02,
    Replaced = 0x03,
    Deleted = 0x04,
    InvalidRequest = 0x40,
    UnsupportedOpcode = 0x41,
    TooLarge = 0x42,
    Overloaded = 0x43,
    Timeout = 0x44,
    InternalError = 0x7f,
}

impl Status {
    /// Returns whether this status represents a server-side error.
    pub fn is_error(self) -> bool {
        (self as u8) >= Status::InvalidRequest as u8
    }
}

impl TryFrom<u8> for Status {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0x00 => Ok(Self::Ok),
            0x01 => Ok(Self::NotFound),
            0x02 => Ok(Self::Created),
            0x03 => Ok(Self::Replaced),
            0x04 => Ok(Self::Deleted),
            0x40 => Ok(Self::InvalidRequest),
            0x41 => Ok(Self::UnsupportedOpcode),
            0x42 => Ok(Self::TooLarge),
            0x43 => Ok(Self::Overloaded),
            0x44 => Ok(Self::Timeout),
            0x7f => Ok(Self::InternalError),
            _ => Err(ProtocolError::UnknownStatus(value)),
        }
    }
}

/// The fixed-size SHA-256 digest sent by clients instead of a user-provided key.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClientKeyDigest([u8; CLIENT_KEY_DIGEST_BYTES]);

impl ClientKeyDigest {
    /// Hashes the exact user-key bytes into the protocol's canonical wire key.
    pub fn from_user_key(user_key: &[u8]) -> Self {
        Self(Sha256::digest(user_key).into())
    }

    /// Wraps an already-computed client key digest.
    pub const fn new(bytes: [u8; CLIENT_KEY_DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the complete digest bytes.
    pub const fn as_bytes(&self) -> &[u8; CLIENT_KEY_DIGEST_BYTES] {
        &self.0
    }

    /// Consumes the digest and returns its bytes.
    pub const fn into_bytes(self) -> [u8; CLIENT_KEY_DIGEST_BYTES] {
        self.0
    }
}

impl AsRef<[u8]> for ClientKeyDigest {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Transformation metadata packed into unused high bits of the wire value length.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ValueFlags {
    compressed: bool,
    encrypted: bool,
}

impl ValueFlags {
    /// No client-side value transformation.
    pub const NONE: Self = Self::new(false, false);

    /// Creates flags for one encoded value.
    pub const fn new(compressed: bool, encrypted: bool) -> Self {
        Self {
            compressed,
            encrypted,
        }
    }

    /// Returns whether the value body is a Zstandard frame before encryption.
    pub const fn is_compressed(self) -> bool {
        self.compressed
    }

    /// Returns whether the value body is authenticated ciphertext.
    pub const fn is_encrypted(self) -> bool {
        self.encrypted
    }

    /// Returns a stable byte representation suitable for authenticated metadata.
    pub const fn authentication_byte(self) -> u8 {
        self.compressed as u8 | ((self.encrypted as u8) << 1)
    }

    const fn wire_bits(self) -> u32 {
        (if self.compressed {
            VALUE_COMPRESSED_BIT
        } else {
            0
        }) | (if self.encrypted {
            VALUE_ENCRYPTED_BIT
        } else {
            0
        })
    }

    const fn from_wire_length(encoded_length: u32) -> Self {
        Self::new(
            encoded_length & VALUE_COMPRESSED_BIT != 0,
            encoded_length & VALUE_ENCRYPTED_BIT != 0,
        )
    }
}

/// A decoded OpenKache request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub opcode: Opcode,
    pub client_key_digest: Option<ClientKeyDigest>,
    pub value_flags: ValueFlags,
    pub value: Vec<u8>,
}

impl Request {
    /// Returns the complete frame length encoded by a fixed-size request header.
    pub fn frame_len_from_header(header: &[u8]) -> Result<usize> {
        if header.len() < REQUEST_HEADER_BYTES {
            return Err(ProtocolError::FrameTooShort {
                expected: REQUEST_HEADER_BYTES,
                actual: header.len(),
            });
        }
        let opcode = Opcode::try_from(header[0])?;
        let key_len = u32::from_be_bytes(header[1..5].try_into().unwrap()) as usize;
        let encoded_value_len = u32::from_be_bytes(header[5..9].try_into().unwrap());
        let value_len = (encoded_value_len & VALUE_LENGTH_MASK) as usize;
        validate_lengths(value_len)?;
        validate_wire_key_length(opcode, key_len)?;
        REQUEST_HEADER_BYTES
            .checked_add(key_len)
            .and_then(|size| size.checked_add(value_len))
            .ok_or(ProtocolError::FrameLengthOverflow)
    }

    /// Creates and validates a request.
    pub fn new(
        opcode: Opcode,
        client_key_digest: Option<ClientKeyDigest>,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_with_value_flags(opcode, client_key_digest, ValueFlags::NONE, value)
    }

    /// Creates and validates a request with explicit value transformation flags.
    pub fn new_with_value_flags(
        opcode: Opcode,
        client_key_digest: Option<ClientKeyDigest>,
        value_flags: ValueFlags,
        value: Vec<u8>,
    ) -> Result<Self> {
        validate_lengths(value.len())?;
        validate_request_shape(
            opcode,
            client_key_digest.is_some(),
            value_flags,
            value.len(),
        )?;
        Ok(Self {
            opcode,
            client_key_digest,
            value_flags,
            value,
        })
    }

    /// Encodes this request into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        validate_lengths(self.value.len())?;
        validate_request_shape(
            self.opcode,
            self.client_key_digest.is_some(),
            self.value_flags,
            self.value.len(),
        )?;
        let key_len = self
            .client_key_digest
            .map_or(0, |_| CLIENT_KEY_DIGEST_BYTES);
        let mut frame = Vec::with_capacity(REQUEST_HEADER_BYTES + key_len + self.value.len());
        frame.push(self.opcode as u8);
        frame.extend_from_slice(&(key_len as u32).to_be_bytes());
        frame.extend_from_slice(
            &encode_value_length(self.value.len(), self.value_flags).to_be_bytes(),
        );
        if let Some(client_key_digest) = self.client_key_digest {
            frame.extend_from_slice(client_key_digest.as_bytes());
        }
        frame.extend_from_slice(&self.value);
        Ok(frame)
    }

    /// Encodes this request while reusing its value allocation when practical.
    pub fn into_encoded(mut self) -> Result<Vec<u8>> {
        validate_lengths(self.value.len())?;
        validate_request_shape(
            self.opcode,
            self.client_key_digest.is_some(),
            self.value_flags,
            self.value.len(),
        )?;
        let key_len = self
            .client_key_digest
            .map_or(0, |_| CLIENT_KEY_DIGEST_BYTES);
        let prefix_len = REQUEST_HEADER_BYTES + key_len;
        let value_len = self.value.len();
        self.value.reserve(prefix_len);
        self.value.resize(prefix_len + value_len, 0);
        self.value.copy_within(0..value_len, prefix_len);
        self.value[0] = self.opcode as u8;
        self.value[1..5].copy_from_slice(&(key_len as u32).to_be_bytes());
        self.value[5..9]
            .copy_from_slice(&encode_value_length(value_len, self.value_flags).to_be_bytes());
        if let Some(client_key_digest) = self.client_key_digest {
            self.value[REQUEST_HEADER_BYTES..prefix_len]
                .copy_from_slice(client_key_digest.as_bytes());
        }
        Ok(self.value)
    }

    /// Decodes and validates one complete request frame.
    pub fn decode(frame: &[u8]) -> Result<Self> {
        if frame.len() < REQUEST_HEADER_BYTES {
            return Err(ProtocolError::FrameTooShort {
                expected: REQUEST_HEADER_BYTES,
                actual: frame.len(),
            });
        }
        let opcode = Opcode::try_from(frame[0])?;
        let key_len = u32::from_be_bytes(frame[1..5].try_into().unwrap()) as usize;
        let encoded_value_len = u32::from_be_bytes(frame[5..9].try_into().unwrap());
        let value_flags = ValueFlags::from_wire_length(encoded_value_len);
        let value_len = (encoded_value_len & VALUE_LENGTH_MASK) as usize;
        validate_lengths(value_len)?;
        validate_wire_key_length(opcode, key_len)?;
        let expected = Self::frame_len_from_header(&frame[..REQUEST_HEADER_BYTES])?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        validate_request_shape(opcode, key_len != 0, value_flags, value_len)?;
        let key_end = REQUEST_HEADER_BYTES + key_len;
        let client_key_digest = if key_len == 0 {
            None
        } else {
            Some(ClientKeyDigest::new(
                frame[REQUEST_HEADER_BYTES..key_end]
                    .try_into()
                    .expect("validated client key digest length"),
            ))
        };
        Ok(Self {
            opcode,
            client_key_digest,
            value_flags,
            value: frame[key_end..].to_vec(),
        })
    }
}

/// A decoded OpenKache response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    pub status: Status,
    pub value_flags: ValueFlags,
    pub payload: Vec<u8>,
}

impl Response {
    /// Returns the complete frame length encoded by a fixed-size response header.
    pub fn frame_len_from_header(header: &[u8]) -> Result<usize> {
        if header.len() < RESPONSE_HEADER_BYTES {
            return Err(ProtocolError::FrameTooShort {
                expected: RESPONSE_HEADER_BYTES,
                actual: header.len(),
            });
        }
        Status::try_from(header[0])?;
        let encoded_payload_len = u32::from_be_bytes(header[1..5].try_into().unwrap());
        let payload_len = (encoded_payload_len & VALUE_LENGTH_MASK) as usize;
        validate_lengths(payload_len)?;
        RESPONSE_HEADER_BYTES
            .checked_add(payload_len)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }

    /// Creates a response after checking the payload limit.
    pub fn new(status: Status, payload: Vec<u8>) -> Result<Self> {
        Self::new_with_value_flags(status, ValueFlags::NONE, payload)
    }

    /// Creates a response with explicit value transformation flags.
    pub fn new_with_value_flags(
        status: Status,
        value_flags: ValueFlags,
        payload: Vec<u8>,
    ) -> Result<Self> {
        if payload.len() > MAX_VALUE_BYTES {
            return Err(ProtocolError::ValueTooLarge {
                size: payload.len(),
                maximum: MAX_VALUE_BYTES,
            });
        }
        validate_response_flags(status, value_flags, payload.len())?;
        Ok(Self {
            status,
            value_flags,
            payload,
        })
    }

    /// Encodes this response into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.payload.len() > MAX_VALUE_BYTES {
            return Err(ProtocolError::ValueTooLarge {
                size: self.payload.len(),
                maximum: MAX_VALUE_BYTES,
            });
        }
        let mut frame = Vec::with_capacity(RESPONSE_HEADER_BYTES + self.payload.len());
        frame.push(self.status as u8);
        validate_response_flags(self.status, self.value_flags, self.payload.len())?;
        frame.extend_from_slice(
            &encode_value_length(self.payload.len(), self.value_flags).to_be_bytes(),
        );
        frame.extend_from_slice(&self.payload);
        Ok(frame)
    }

    /// Decodes and validates one complete response frame.
    pub fn decode(frame: &[u8]) -> Result<Self> {
        if frame.len() < RESPONSE_HEADER_BYTES {
            return Err(ProtocolError::FrameTooShort {
                expected: RESPONSE_HEADER_BYTES,
                actual: frame.len(),
            });
        }
        let status = Status::try_from(frame[0])?;
        let encoded_payload_len = u32::from_be_bytes(frame[1..5].try_into().unwrap());
        let value_flags = ValueFlags::from_wire_length(encoded_payload_len);
        let payload_len = (encoded_payload_len & VALUE_LENGTH_MASK) as usize;
        if payload_len > MAX_VALUE_BYTES {
            return Err(ProtocolError::ValueTooLarge {
                size: payload_len,
                maximum: MAX_VALUE_BYTES,
            });
        }
        let expected = Self::frame_len_from_header(&frame[..RESPONSE_HEADER_BYTES])?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        validate_response_flags(status, value_flags, payload_len)?;
        Ok(Self {
            status,
            value_flags,
            payload: frame[RESPONSE_HEADER_BYTES..].to_vec(),
        })
    }

    /// Decodes a response while reusing the frame allocation for its payload.
    pub fn decode_owned(mut frame: Vec<u8>) -> Result<Self> {
        if frame.len() < RESPONSE_HEADER_BYTES {
            return Err(ProtocolError::FrameTooShort {
                expected: RESPONSE_HEADER_BYTES,
                actual: frame.len(),
            });
        }
        let status = Status::try_from(frame[0])?;
        let encoded_payload_len = u32::from_be_bytes(frame[1..5].try_into().unwrap());
        let value_flags = ValueFlags::from_wire_length(encoded_payload_len);
        let payload_len = (encoded_payload_len & VALUE_LENGTH_MASK) as usize;
        if payload_len > MAX_VALUE_BYTES {
            return Err(ProtocolError::ValueTooLarge {
                size: payload_len,
                maximum: MAX_VALUE_BYTES,
            });
        }
        let expected = Self::frame_len_from_header(&frame[..RESPONSE_HEADER_BYTES])?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        validate_response_flags(status, value_flags, payload_len)?;
        frame.copy_within(RESPONSE_HEADER_BYTES.., 0);
        frame.truncate(payload_len);
        Ok(Self {
            status,
            value_flags,
            payload: frame,
        })
    }
}

/// Protocol framing and validation errors.
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
    #[error("{opcode:?} requires a {expected}-byte client key digest, received {actual} key bytes")]
    InvalidClientKeyLength {
        opcode: Opcode,
        expected: usize,
        actual: usize,
    },
    #[error("value is too large: {size} bytes exceeds {maximum}")]
    ValueTooLarge { size: usize, maximum: usize },
    #[error("{opcode:?} requires key_len={expected_key} and value_len={expected_value}")]
    InvalidRequestShape {
        opcode: Opcode,
        expected_key: &'static str,
        expected_value: &'static str,
    },
    #[error("value transformation flags are not valid for {context}")]
    InvalidValueFlags { context: &'static str },
}

/// Convenience result type for protocol operations.
pub type Result<T> = std::result::Result<T, ProtocolError>;

fn validate_lengths(value_len: usize) -> Result<()> {
    if value_len > MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: value_len,
            maximum: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

fn encode_value_length(value_len: usize, value_flags: ValueFlags) -> u32 {
    value_len as u32 | value_flags.wire_bits()
}

fn validate_wire_key_length(opcode: Opcode, key_len: usize) -> Result<()> {
    let expected = match opcode {
        Opcode::Ping | Opcode::Stats | Opcode::Sync => 0,
        Opcode::Get | Opcode::Set | Opcode::Delete => CLIENT_KEY_DIGEST_BYTES,
    };
    if key_len == expected {
        Ok(())
    } else {
        Err(ProtocolError::InvalidClientKeyLength {
            opcode,
            expected,
            actual: key_len,
        })
    }
}

fn validate_request_shape(
    opcode: Opcode,
    has_client_key: bool,
    value_flags: ValueFlags,
    value_len: usize,
) -> Result<()> {
    let valid = match opcode {
        Opcode::Ping | Opcode::Stats | Opcode::Sync => {
            !has_client_key && value_len == 0 && value_flags == ValueFlags::NONE
        }
        Opcode::Get | Opcode::Delete => {
            has_client_key && value_len == 0 && value_flags == ValueFlags::NONE
        }
        Opcode::Set => has_client_key,
    };
    if valid {
        return Ok(());
    }
    let (expected_key, expected_value) = match opcode {
        Opcode::Ping | Opcode::Stats | Opcode::Sync => ("0", "0"),
        Opcode::Get | Opcode::Delete => ("32", "0"),
        Opcode::Set => ("32", "any"),
    };
    Err(ProtocolError::InvalidRequestShape {
        opcode,
        expected_key,
        expected_value,
    })
}

fn validate_response_flags(
    status: Status,
    value_flags: ValueFlags,
    payload_len: usize,
) -> Result<()> {
    if value_flags == ValueFlags::NONE || (status == Status::Ok && payload_len > 0) {
        Ok(())
    } else {
        Err(ProtocolError::InvalidValueFlags {
            context: "response status or empty payload",
        })
    }
}
