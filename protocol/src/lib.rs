//! Binary request and response framing shared by OpenKache clients and servers.

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

const MIN_VARUINT_BYTES: usize = 1;
const MIN_REQUEST_FRAME_BYTES: usize = REQUEST_FIXED_BYTES + MIN_VARUINT_BYTES * 2;
const MIN_RESPONSE_FRAME_BYTES: usize = RESPONSE_FIXED_BYTES + MIN_VARUINT_BYTES;
const MAX_REQUEST_PREFIX_BYTES: usize =
    REQUEST_FIXED_BYTES + MAX_VARUINT_BYTES * 3 + ITEM_KEY_BYTES;
const MAX_RESPONSE_PREFIX_BYTES: usize = RESPONSE_FIXED_BYTES + MAX_VARUINT_BYTES;
const KNOWN_SET_FLAGS: u8 = SET_TTL_FLAG | SET_IF_ABSENT_FLAG | SET_IF_PRESENT_FLAG;

/// Conservative maximum complete request frame size.
pub const MAX_REQUEST_FRAME_BYTES: usize = MAX_REQUEST_PREFIX_BYTES + MAX_VALUE_BYTES;
/// Conservative maximum complete response frame size.
pub const MAX_RESPONSE_FRAME_BYTES: usize = MAX_RESPONSE_PREFIX_BYTES + MAX_VALUE_BYTES;

impl Status {
    /// Returns whether this status represents a server-side error.
    pub fn is_error(self) -> bool {
        (self as u8) >= Status::InvalidRequest as u8
    }
}

/// The exact fixed-size item identifier carried by the protocol.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ItemKey([u8; ITEM_KEY_BYTES]);

impl ItemKey {
    /// Wraps an exact 32-byte item key.
    pub const fn new(bytes: [u8; ITEM_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the complete key bytes.
    pub const fn as_bytes(&self) -> &[u8; ITEM_KEY_BYTES] {
        &self.0
    }

    /// Consumes the key and returns its bytes.
    pub const fn into_bytes(self) -> [u8; ITEM_KEY_BYTES] {
        self.0
    }
}

impl AsRef<[u8]> for ItemKey {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Condition applied atomically by a `SET` request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SetCondition {
    /// Store regardless of whether the key exists.
    #[default]
    None,
    /// Store only when the key does not exist.
    IfAbsent,
    /// Store only when the key already exists.
    IfPresent,
}

/// Optional behavior for one `SET` request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SetOptions {
    /// Atomic existence condition.
    pub condition: SetCondition,
    /// Relative lifetime in milliseconds. `None` stores a persistent item.
    pub ttl_ms: Option<u64>,
}

impl SetOptions {
    /// Creates persistent, unconditional `SET` behavior.
    pub const NONE: Self = Self {
        condition: SetCondition::None,
        ttl_ms: None,
    };

    /// Creates options from an existence condition and optional positive TTL.
    pub const fn new(condition: SetCondition, ttl_ms: Option<u64>) -> Self {
        Self { condition, ttl_ms }
    }

    fn flags(self) -> u8 {
        let condition = match self.condition {
            SetCondition::None => 0,
            SetCondition::IfAbsent => SET_IF_ABSENT_FLAG,
            SetCondition::IfPresent => SET_IF_PRESENT_FLAG,
        };
        condition
            | if self.ttl_ms.is_some() {
                SET_TTL_FLAG
            } else {
                0
            }
    }
}

/// A validated variable-length request header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestHeader {
    opcode: Opcode,
    encoded_len: usize,
    key_len: usize,
    value_len: usize,
    condition: SetCondition,
    has_ttl: bool,
}

impl RequestHeader {
    /// Returns the decoded operation.
    pub const fn opcode(self) -> Opcode {
        self.opcode
    }

    /// Returns the number of encoded header bytes before the key.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the encoded key length.
    pub const fn key_len(self) -> usize {
        self.key_len
    }

    /// Returns the opaque value length.
    pub const fn value_len(self) -> usize {
        self.value_len
    }

    /// Returns whether a TTL varuint follows the key.
    pub const fn has_ttl(self) -> bool {
        self.has_ttl
    }

    /// Reports the complete frame length once enough key and TTL prefix bytes are present.
    ///
    /// # Arguments
    ///
    /// * `prefix` - Frame bytes beginning at the opcode and extending through the TTL when present.
    ///
    /// # Returns
    ///
    /// `Ok(Some(length))` when the complete frame length is known, or `Ok(None)` when more prefix
    /// bytes are required.
    ///
    /// # Errors
    ///
    /// Returns an error for a malformed, non-canonical, zero, or overflowing TTL.
    pub fn frame_len(self, prefix: &[u8]) -> Result<Option<usize>> {
        let key_end = self
            .encoded_len
            .checked_add(self.key_len)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        if prefix.len() < key_end {
            return Ok(None);
        }
        let ttl_len = if self.has_ttl {
            let Some((ttl_ms, encoded_len)) = decode_varuint(&prefix[key_end..], "SET TTL")? else {
                return Ok(None);
            };
            if ttl_ms == 0 {
                return Err(ProtocolError::InvalidSetTtl);
            }
            encoded_len
        } else {
            0
        };
        key_end
            .checked_add(ttl_len)
            .and_then(|size| size.checked_add(self.value_len))
            .map(Some)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }
}

/// A decoded OpenKache request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub opcode: Opcode,
    pub key: Option<ItemKey>,
    pub set_options: SetOptions,
    pub value: Vec<u8>,
}

impl Request {
    /// Decodes and validates a request header when enough prefix bytes are available.
    ///
    /// # Arguments
    ///
    /// * `prefix` - Frame bytes beginning at the opcode.
    ///
    /// # Returns
    ///
    /// `Ok(Some(header))` after both canonical length fields are present, or `Ok(None)` when more
    /// bytes are required.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown opcode, invalid flags, malformed lengths, an unsupported
    /// key length, or an oversized value.
    pub fn decode_header(prefix: &[u8]) -> Result<Option<RequestHeader>> {
        if prefix.len() < REQUEST_FIXED_BYTES {
            return Ok(None);
        }
        let opcode = Opcode::try_from(prefix[0])?;
        let (condition, has_ttl) = decode_request_flags(opcode, prefix[1])?;
        let Some((key_len, key_len_bytes)) =
            decode_varuint(&prefix[REQUEST_FIXED_BYTES..], "request key length")?
        else {
            return Ok(None);
        };
        let value_len_start = REQUEST_FIXED_BYTES + key_len_bytes;
        let Some((value_len, value_len_bytes)) =
            decode_varuint(&prefix[value_len_start..], "request value length")?
        else {
            return Ok(None);
        };
        let key_len = usize::try_from(key_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
        let value_len =
            usize::try_from(value_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
        validate_item_key_length(opcode, key_len)?;
        validate_value_length(value_len)?;
        validate_request_shape(
            opcode,
            key_len != 0,
            SetOptions::new(condition, None),
            value_len,
        )?;
        Ok(Some(RequestHeader {
            opcode,
            encoded_len: value_len_start + value_len_bytes,
            key_len,
            value_len,
            condition,
            has_ttl,
        }))
    }

    /// Reports the complete request frame length once its variable prefix is available.
    ///
    /// # Arguments
    ///
    /// * `prefix` - Frame bytes beginning at the opcode.
    ///
    /// # Returns
    ///
    /// `Ok(Some(length))` when the key and optional TTL prefix are complete, or `Ok(None)` when
    /// more prefix bytes are required.
    ///
    /// # Errors
    ///
    /// Returns an error when the header, key length, value length, flags, or TTL is invalid.
    pub fn frame_len(prefix: &[u8]) -> Result<Option<usize>> {
        let Some(header) = Self::decode_header(prefix)? else {
            return Ok(None);
        };
        header.frame_len(prefix)
    }

    /// Creates and validates a request.
    pub fn new(opcode: Opcode, key: Option<ItemKey>, value: Vec<u8>) -> Result<Self> {
        Self::new_set(opcode, key, SetOptions::NONE, value)
    }

    /// Creates and validates a request with explicit `SET` options.
    pub fn new_set(
        opcode: Opcode,
        key: Option<ItemKey>,
        set_options: SetOptions,
        value: Vec<u8>,
    ) -> Result<Self> {
        validate_value_length(value.len())?;
        validate_request_shape(opcode, key.is_some(), set_options, value.len())?;
        Ok(Self {
            opcode,
            key,
            set_options,
            value,
        })
    }

    /// Encodes this request into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let prefix = self.encode_prefix()?;
        let mut frame = Vec::with_capacity(prefix.len + self.value.len());
        frame.extend_from_slice(prefix.as_slice());
        frame.extend_from_slice(&self.value);
        Ok(frame)
    }

    /// Encodes this request while reusing its value allocation when practical.
    pub fn into_encoded(mut self) -> Result<Vec<u8>> {
        let prefix = self.encode_prefix()?;
        let prefix_len = prefix.len;
        let value_len = self.value.len();
        self.value.reserve(prefix_len);
        self.value.resize(prefix_len + value_len, 0);
        self.value.copy_within(0..value_len, prefix_len);
        self.value[..prefix_len].copy_from_slice(prefix.as_slice());
        Ok(self.value)
    }

    fn encode_prefix(&self) -> Result<RequestPrefix> {
        validate_value_length(self.value.len())?;
        validate_request_shape(
            self.opcode,
            self.key.is_some(),
            self.set_options,
            self.value.len(),
        )?;
        let key_len = self.key.map_or(0, |_| ITEM_KEY_BYTES);
        let mut key_len_encoded = [0; MAX_VARUINT_BYTES];
        let key_len_bytes = vu128::encode_u64(&mut key_len_encoded, key_len as u64);
        let mut value_len_encoded = [0; MAX_VARUINT_BYTES];
        let value_len_bytes = vu128::encode_u64(&mut value_len_encoded, self.value.len() as u64);
        let mut ttl_encoded = [0; MAX_VARUINT_BYTES];
        let ttl_bytes = self
            .set_options
            .ttl_ms
            .map_or(0, |ttl_ms| vu128::encode_u64(&mut ttl_encoded, ttl_ms));
        let len = REQUEST_FIXED_BYTES + key_len_bytes + value_len_bytes + key_len + ttl_bytes;
        let mut bytes = [0; MAX_REQUEST_PREFIX_BYTES];
        bytes[0] = self.opcode as u8;
        bytes[1] = self.set_options.flags();
        let mut offset = REQUEST_FIXED_BYTES;
        bytes[offset..offset + key_len_bytes].copy_from_slice(&key_len_encoded[..key_len_bytes]);
        offset += key_len_bytes;
        bytes[offset..offset + value_len_bytes]
            .copy_from_slice(&value_len_encoded[..value_len_bytes]);
        offset += value_len_bytes;
        if let Some(key) = self.key {
            bytes[offset..offset + key_len].copy_from_slice(key.as_bytes());
            offset += key_len;
        }
        bytes[offset..offset + ttl_bytes].copy_from_slice(&ttl_encoded[..ttl_bytes]);
        Ok(RequestPrefix { bytes, len })
    }

    /// Decodes and validates one complete request frame.
    pub fn decode(frame: &[u8]) -> Result<Self> {
        let decoded = decode_request_frame(frame)?;
        Ok(Self {
            opcode: decoded.opcode,
            key: decoded.key,
            set_options: decoded.set_options,
            value: frame[decoded.value_start..].to_vec(),
        })
    }

    /// Decodes a request while reusing the frame allocation for its value.
    pub fn decode_owned(mut frame: Vec<u8>) -> Result<Self> {
        let decoded = decode_request_frame(&frame)?;
        frame.copy_within(decoded.value_start.., 0);
        frame.truncate(decoded.value_len);
        Ok(Self {
            opcode: decoded.opcode,
            key: decoded.key,
            set_options: decoded.set_options,
            value: frame,
        })
    }
}

struct RequestPrefix {
    bytes: [u8; MAX_REQUEST_PREFIX_BYTES],
    len: usize,
}

impl RequestPrefix {
    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

struct DecodedRequestFrame {
    opcode: Opcode,
    key: Option<ItemKey>,
    set_options: SetOptions,
    value_start: usize,
    value_len: usize,
}

fn decode_request_frame(frame: &[u8]) -> Result<DecodedRequestFrame> {
    let header = Request::decode_header(frame)?.ok_or(ProtocolError::FrameTooShort {
        expected: MIN_REQUEST_FRAME_BYTES,
        actual: frame.len(),
    })?;
    let expected = header
        .frame_len(frame)?
        .ok_or(ProtocolError::FrameTooShort {
            expected: header.encoded_len + header.key_len + usize::from(header.has_ttl),
            actual: frame.len(),
        })?;
    if frame.len() != expected {
        return Err(ProtocolError::FrameLength {
            expected,
            actual: frame.len(),
        });
    }
    let key_end = header.encoded_len + header.key_len;
    let key = if header.key_len == 0 {
        None
    } else {
        Some(ItemKey::new(
            frame[header.encoded_len..key_end]
                .try_into()
                .expect("validated item key length"),
        ))
    };
    let mut set_options = SetOptions::new(header.condition, None);
    let value_start = if header.has_ttl {
        let (ttl_ms, ttl_len) = decode_varuint(&frame[key_end..], "SET TTL")?
            .expect("frame length requires a complete TTL");
        set_options.ttl_ms = Some(ttl_ms);
        key_end + ttl_len
    } else {
        key_end
    };
    validate_request_shape(
        header.opcode,
        header.key_len != 0,
        set_options,
        header.value_len,
    )?;
    Ok(DecodedRequestFrame {
        opcode: header.opcode,
        key,
        set_options,
        value_start,
        value_len: header.value_len,
    })
}

/// A validated variable-length response header.
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

    /// Returns the number of encoded header bytes before the payload.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the response payload length.
    pub const fn payload_len(self) -> usize {
        self.payload_len
    }

    /// Returns the complete response frame length.
    ///
    /// # Returns
    ///
    /// The encoded header length plus the opaque payload length.
    ///
    /// # Errors
    ///
    /// Returns an error when the complete length cannot be represented as a `usize`.
    pub fn frame_len(self) -> Result<usize> {
        self.encoded_len
            .checked_add(self.payload_len)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }
}

/// A decoded OpenKache response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    pub status: Status,
    pub payload: Vec<u8>,
}

impl Response {
    /// Decodes and validates a response header when enough prefix bytes are available.
    ///
    /// # Arguments
    ///
    /// * `prefix` - Frame bytes beginning at the status.
    ///
    /// # Returns
    ///
    /// `Ok(Some(header))` after the canonical payload length is present, or `Ok(None)` when more
    /// bytes are required.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown status, malformed length, or oversized payload.
    pub fn decode_header(prefix: &[u8]) -> Result<Option<ResponseHeader>> {
        if prefix.len() < RESPONSE_FIXED_BYTES {
            return Ok(None);
        }
        let status = Status::try_from(prefix[0])?;
        let Some((payload_len, payload_len_bytes)) =
            decode_varuint(&prefix[RESPONSE_FIXED_BYTES..], "response payload length")?
        else {
            return Ok(None);
        };
        let payload_len =
            usize::try_from(payload_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
        validate_value_length(payload_len)?;
        Ok(Some(ResponseHeader {
            status,
            encoded_len: RESPONSE_FIXED_BYTES + payload_len_bytes,
            payload_len,
        }))
    }

    /// Reports the complete response frame length once its variable prefix is available.
    ///
    /// # Arguments
    ///
    /// * `prefix` - Frame bytes beginning at the status.
    ///
    /// # Returns
    ///
    /// `Ok(Some(length))` when the canonical payload length is complete, or `Ok(None)` when more
    /// prefix bytes are required.
    ///
    /// # Errors
    ///
    /// Returns an error when the status or payload length is invalid or the complete length
    /// overflows.
    pub fn frame_len(prefix: &[u8]) -> Result<Option<usize>> {
        Self::decode_header(prefix)?
            .map(ResponseHeader::frame_len)
            .transpose()
    }

    /// Creates a response after checking the payload limit.
    pub fn new(status: Status, payload: Vec<u8>) -> Result<Self> {
        validate_value_length(payload.len())?;
        Ok(Self { status, payload })
    }

    /// Encodes this response into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let prefix = self.encode_prefix()?;
        let mut frame = Vec::with_capacity(prefix.len + self.payload.len());
        frame.extend_from_slice(prefix.as_slice());
        frame.extend_from_slice(&self.payload);
        Ok(frame)
    }

    /// Consumes and encodes this response, reusing its payload allocation when practical.
    ///
    /// # Returns
    ///
    /// The complete encoded response frame.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload exceeds the protocol limit.
    pub fn into_encoded(mut self) -> Result<Vec<u8>> {
        let prefix = self.encode_prefix()?;
        let payload_len = self.payload.len();
        let prefix_len = prefix.len;
        self.payload.reserve(prefix_len);
        self.payload.resize(prefix_len + payload_len, 0);
        self.payload.copy_within(0..payload_len, prefix_len);
        self.payload[..prefix_len].copy_from_slice(prefix.as_slice());
        Ok(self.payload)
    }

    fn encode_prefix(&self) -> Result<ResponsePrefix> {
        validate_value_length(self.payload.len())?;
        let mut length = [0; MAX_VARUINT_BYTES];
        let length_bytes = vu128::encode_u64(&mut length, self.payload.len() as u64);
        let len = RESPONSE_FIXED_BYTES + length_bytes;
        let mut bytes = [0; MAX_RESPONSE_PREFIX_BYTES];
        bytes[0] = self.status as u8;
        bytes[RESPONSE_FIXED_BYTES..len].copy_from_slice(&length[..length_bytes]);
        Ok(ResponsePrefix { bytes, len })
    }

    /// Decodes and validates one complete response frame.
    pub fn decode(frame: &[u8]) -> Result<Self> {
        let header = Self::decode_header(frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: MIN_RESPONSE_FRAME_BYTES,
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

    /// Decodes a response while reusing the frame allocation for its payload.
    pub fn decode_owned(mut frame: Vec<u8>) -> Result<Self> {
        let header = Self::decode_header(&frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: MIN_RESPONSE_FRAME_BYTES,
            actual: frame.len(),
        })?;
        let expected = header.frame_len()?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        frame.copy_within(header.encoded_len.., 0);
        frame.truncate(header.payload_len);
        Ok(Self {
            status: header.status,
            payload: frame,
        })
    }
}

struct ResponsePrefix {
    bytes: [u8; MAX_RESPONSE_PREFIX_BYTES],
    len: usize,
}

impl ResponsePrefix {
    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Protocol framing and validation errors.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("unknown opcode 0x{0:02x}")]
    UnknownOpcode(u8),
    #[error("unknown status 0x{0:02x}")]
    UnknownStatus(u8),
    #[error("request flags contain unknown bits 0x{0:02x}")]
    UnknownRequestFlags(u8),
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
    #[error("{opcode:?} requires a {expected}-byte item key, received {actual} key bytes")]
    InvalidItemKeyLength {
        opcode: Opcode,
        expected: usize,
        actual: usize,
    },
    #[error("value is too large: {size} bytes exceeds {maximum}")]
    ValueTooLarge { size: usize, maximum: usize },
    #[error("{opcode:?} requires key_len={expected_key} and value_len={expected_value}")]
    InvalidRequestShape {
        opcode: Opcode,
        expected_key: usize,
        expected_value: &'static str,
    },
    #[error("if-absent and if-present conditions cannot be combined")]
    ConflictingSetConditions,
    #[error("SET TTL must be greater than zero milliseconds")]
    InvalidSetTtl,
    #[error("SET options are not valid for {opcode:?}")]
    InvalidSetOptions { opcode: Opcode },
}

/// Convenience result type for protocol operations.
pub type Result<T> = std::result::Result<T, ProtocolError>;

fn decode_varuint(input: &[u8], context: &'static str) -> Result<Option<(u64, usize)>> {
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
    debug_assert_eq!(decoded_len, encoded_len);

    let mut canonical = [0; MAX_VARUINT_BYTES];
    let canonical_len = vu128::encode_u64(&mut canonical, value);
    if canonical_len != encoded_len || canonical[..canonical_len] != input[..encoded_len] {
        return Err(ProtocolError::NonCanonicalVaruint { context });
    }
    Ok(Some((value, encoded_len)))
}

fn decode_request_flags(opcode: Opcode, flags: u8) -> Result<(SetCondition, bool)> {
    if flags & !KNOWN_SET_FLAGS != 0 {
        return Err(ProtocolError::UnknownRequestFlags(flags & !KNOWN_SET_FLAGS));
    }
    if opcode != Opcode::Set && flags != 0 {
        return Err(ProtocolError::InvalidSetOptions { opcode });
    }
    let condition = match (
        flags & SET_IF_ABSENT_FLAG != 0,
        flags & SET_IF_PRESENT_FLAG != 0,
    ) {
        (false, false) => SetCondition::None,
        (true, false) => SetCondition::IfAbsent,
        (false, true) => SetCondition::IfPresent,
        (true, true) => return Err(ProtocolError::ConflictingSetConditions),
    };
    Ok((condition, flags & SET_TTL_FLAG != 0))
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

fn validate_item_key_length(opcode: Opcode, key_len: usize) -> Result<()> {
    let expected = match opcode {
        Opcode::Ping | Opcode::Stats | Opcode::Sync => 0,
        Opcode::Get | Opcode::Set | Opcode::Delete => ITEM_KEY_BYTES,
    };
    if key_len == expected {
        Ok(())
    } else {
        Err(ProtocolError::InvalidItemKeyLength {
            opcode,
            expected,
            actual: key_len,
        })
    }
}

fn validate_request_shape(
    opcode: Opcode,
    has_client_key: bool,
    set_options: SetOptions,
    value_len: usize,
) -> Result<()> {
    if set_options.ttl_ms == Some(0) {
        return Err(ProtocolError::InvalidSetTtl);
    }
    if opcode != Opcode::Set && set_options != SetOptions::NONE {
        return Err(ProtocolError::InvalidSetOptions { opcode });
    }
    let valid = match opcode {
        Opcode::Ping | Opcode::Stats | Opcode::Sync => !has_client_key && value_len == 0,
        Opcode::Get | Opcode::Delete => has_client_key && value_len == 0,
        Opcode::Set => has_client_key,
    };
    if valid {
        return Ok(());
    }
    let (expected_key, expected_value) = match opcode {
        Opcode::Ping | Opcode::Stats | Opcode::Sync => (0, "0"),
        Opcode::Get | Opcode::Delete => (ITEM_KEY_BYTES, "0"),
        Opcode::Set => (ITEM_KEY_BYTES, "any"),
    };
    Err(ProtocolError::InvalidRequestShape {
        opcode,
        expected_key,
        expected_value,
    })
}
