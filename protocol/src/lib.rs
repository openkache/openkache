//! Binary request and response framing shared by OpenKache clients and servers.

use sha2::{Digest, Sha256};

/// QUIC application protocol identifier for the first OpenKache wire format.
pub const ALPN: &[u8] = b"openkache/1";
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

/// Operations supported by protocol v1.
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

/// A decoded OpenKache request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub opcode: Opcode,
    pub client_key_digest: Option<ClientKeyDigest>,
    pub value: Vec<u8>,
}

impl Request {
    /// Creates and validates a request.
    pub fn new(
        opcode: Opcode,
        client_key_digest: Option<ClientKeyDigest>,
        value: Vec<u8>,
    ) -> Result<Self> {
        validate_lengths(value.len())?;
        validate_request_shape(opcode, client_key_digest.is_some(), value.len())?;
        Ok(Self {
            opcode,
            client_key_digest,
            value,
        })
    }

    /// Encodes this request into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        validate_lengths(self.value.len())?;
        validate_request_shape(
            self.opcode,
            self.client_key_digest.is_some(),
            self.value.len(),
        )?;
        let key_len = self
            .client_key_digest
            .map_or(0, |_| CLIENT_KEY_DIGEST_BYTES);
        let mut frame = Vec::with_capacity(REQUEST_HEADER_BYTES + key_len + self.value.len());
        frame.push(self.opcode as u8);
        frame.extend_from_slice(&(key_len as u32).to_be_bytes());
        frame.extend_from_slice(&(self.value.len() as u32).to_be_bytes());
        if let Some(client_key_digest) = self.client_key_digest {
            frame.extend_from_slice(client_key_digest.as_bytes());
        }
        frame.extend_from_slice(&self.value);
        Ok(frame)
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
        let value_len = u32::from_be_bytes(frame[5..9].try_into().unwrap()) as usize;
        validate_lengths(value_len)?;
        validate_wire_key_length(opcode, key_len)?;
        let expected = REQUEST_HEADER_BYTES
            .checked_add(key_len)
            .and_then(|size| size.checked_add(value_len))
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        validate_request_shape(opcode, key_len != 0, value_len)?;
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
            value: frame[key_end..].to_vec(),
        })
    }
}

/// A decoded OpenKache response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    pub status: Status,
    pub payload: Vec<u8>,
}

impl Response {
    /// Creates a response after checking the payload limit.
    pub fn new(status: Status, payload: Vec<u8>) -> Result<Self> {
        if payload.len() > MAX_VALUE_BYTES {
            return Err(ProtocolError::ValueTooLarge {
                size: payload.len(),
                maximum: MAX_VALUE_BYTES,
            });
        }
        Ok(Self { status, payload })
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
        frame.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
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
        let payload_len = u32::from_be_bytes(frame[1..5].try_into().unwrap()) as usize;
        if payload_len > MAX_VALUE_BYTES {
            return Err(ProtocolError::ValueTooLarge {
                size: payload_len,
                maximum: MAX_VALUE_BYTES,
            });
        }
        let expected = RESPONSE_HEADER_BYTES
            .checked_add(payload_len)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        Ok(Self {
            status,
            payload: frame[RESPONSE_HEADER_BYTES..].to_vec(),
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

fn validate_request_shape(opcode: Opcode, has_client_key: bool, value_len: usize) -> Result<()> {
    let valid = match opcode {
        Opcode::Ping | Opcode::Stats | Opcode::Sync => !has_client_key && value_len == 0,
        Opcode::Get | Opcode::Delete => has_client_key && value_len == 0,
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
