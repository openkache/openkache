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
    REQUEST_FIXED_BYTES + MAX_VARUINT_BYTES * 3 + ITEM_ID_BYTES + MUTATION_ID_BYTES;
const MAX_RESPONSE_PREFIX_BYTES: usize = RESPONSE_FIXED_BYTES + MAX_VARUINT_BYTES;
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
pub struct ItemId([u8; ITEM_ID_BYTES]);

impl ItemId {
    /// Wraps an exact 32-byte item ID.
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

/// Exact idempotency token carried by a mutating v1 request.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MutationId([u8; MUTATION_ID_BYTES]);

impl MutationId {
    /// Wraps an exact mutation token.
    pub const fn new(bytes: [u8; MUTATION_ID_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the complete mutation-token bytes.
    pub const fn as_bytes(&self) -> &[u8; MUTATION_ID_BYTES] {
        &self.0
    }

    /// Consumes the token and returns its bytes.
    pub const fn into_bytes(self) -> [u8; MUTATION_ID_BYTES] {
        self.0
    }
}

impl AsRef<[u8]> for MutationId {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Condition applied atomically by a `SET` request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SetCondition {
    /// Store regardless of whether the item ID exists.
    #[default]
    None,
    /// Store only when the item ID does not exist.
    IfAbsent,
    /// Store only when the item ID already exists.
    IfPresent,
}

/// Optional behavior for one `SET` request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SetOptions {
    /// Atomic existence condition.
    pub condition: SetCondition,
    /// Relative lifetime in milliseconds. `None` stores a persistent item.
    pub ttl_ms: Option<u64>,
    /// Optional idempotency token for this mutation.
    pub mutation_id: Option<MutationId>,
}

impl SetOptions {
    /// Creates persistent, unconditional `SET` behavior.
    pub const NONE: Self = Self {
        condition: SetCondition::None,
        ttl_ms: None,
        mutation_id: None,
    };

    /// Creates options from an existence condition and optional positive TTL.
    pub const fn new(condition: SetCondition, ttl_ms: Option<u64>) -> Self {
        Self {
            condition,
            ttl_ms,
            mutation_id: None,
        }
    }

    /// Adds an idempotency token to this mutation.
    pub const fn with_mutation_id(mut self, mutation_id: MutationId) -> Self {
        self.mutation_id = Some(mutation_id);
        self
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
            | if self.mutation_id.is_some() {
                SET_MUTATION_ID_FLAG
            } else {
                0
            }
    }
}

fn encode_varuint(value: u64) -> ([u8; MAX_VARUINT_BYTES], usize) {
    let mut encoded = [0; MAX_VARUINT_BYTES];
    let length = vu128::encode_u64(&mut encoded, value);
    (encoded, length)
}

fn prepend_prefix(mut payload: Vec<u8>, prefix: &[u8]) -> Vec<u8> {
    let payload_len = payload.len();
    payload.reserve(prefix.len());
    payload.resize(prefix.len() + payload_len, 0);
    payload.copy_within(0..payload_len, prefix.len());
    payload[..prefix.len()].copy_from_slice(prefix);
    payload
}

/// A validated variable-length request header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestHeader {
    opcode: Opcode,
    encoded_len: usize,
    item_id_len: usize,
    value_len: usize,
    condition: SetCondition,
    has_ttl: bool,
    has_mutation_id: bool,
}

impl RequestHeader {
    /// Returns the decoded operation.
    pub const fn opcode(self) -> Opcode {
        self.opcode
    }

    /// Returns the number of encoded header bytes before the item ID.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the encoded item ID length.
    pub const fn item_id_len(self) -> usize {
        self.item_id_len
    }

    /// Returns the opaque value length.
    pub const fn value_len(self) -> usize {
        self.value_len
    }

    /// Returns whether a TTL varuint follows the item ID.
    pub const fn has_ttl(self) -> bool {
        self.has_ttl
    }

    /// Returns whether a fixed-size mutation token follows the item ID.
    pub const fn has_mutation_id(self) -> bool {
        self.has_mutation_id
    }

    /// Reports the complete frame length once enough item ID and TTL prefix bytes are present.
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
        let item_id_end = self.item_id_end()?;
        if prefix.len() < item_id_end {
            return Ok(None);
        }
        let mutation_end = item_id_end
            .checked_add(if self.has_mutation_id {
                MUTATION_ID_BYTES
            } else {
                0
            })
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        if prefix.len() < mutation_end {
            return Ok(None);
        }
        let ttl_len = if self.has_ttl {
            let Some((ttl_ms, encoded_len)) = decode_varuint(&prefix[mutation_end..], "SET TTL")?
            else {
                return Ok(None);
            };
            if ttl_ms == 0 {
                return Err(ProtocolError::InvalidSetTtl);
            }
            encoded_len
        } else {
            0
        };
        mutation_end
            .checked_add(ttl_len)
            .and_then(|size| size.checked_add(self.value_len))
            .map(Some)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }

    fn item_id_end(self) -> Result<usize> {
        self.encoded_len
            .checked_add(self.item_id_len)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }
}

/// A decoded OpenKache request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub opcode: Opcode,
    pub item_id: Option<ItemId>,
    pub set_options: SetOptions,
    pub mutation_id: Option<MutationId>,
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
    /// item ID length, or an oversized value.
    pub fn decode_header(prefix: &[u8]) -> Result<Option<RequestHeader>> {
        if prefix.len() < REQUEST_FIXED_BYTES {
            return Ok(None);
        }
        let opcode = Opcode::try_from(prefix[0])?;
        let (condition, has_ttl, has_mutation_id) = decode_request_flags(opcode, prefix[1])?;
        let Some((item_id_len, item_id_len_bytes)) =
            decode_varuint(&prefix[REQUEST_FIXED_BYTES..], "request item ID length")?
        else {
            return Ok(None);
        };
        let value_len_start = REQUEST_FIXED_BYTES + item_id_len_bytes;
        let Some((value_len, value_len_bytes)) =
            decode_varuint(&prefix[value_len_start..], "request value length")?
        else {
            return Ok(None);
        };
        let item_id_len =
            usize::try_from(item_id_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
        let value_len =
            usize::try_from(value_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
        validate_item_id_length(opcode, item_id_len)?;
        validate_value_length(value_len)?;
        let mut set_options = SetOptions::new(condition, None);
        // The header does not contain the token bytes yet, but shape
        // validation still needs to know whether the mutation flag is
        // present. A zero token is only a validation marker; the complete
        // decoder replaces it with the bytes from the frame.
        if has_mutation_id {
            set_options.mutation_id = Some(MutationId::new([0; MUTATION_ID_BYTES]));
        }
        validate_request_shape(
            opcode,
            item_id_len != 0,
            set_options,
            has_mutation_id,
            value_len,
        )?;
        Ok(Some(RequestHeader {
            opcode,
            encoded_len: value_len_start + value_len_bytes,
            item_id_len,
            value_len,
            condition,
            has_ttl,
            has_mutation_id,
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
    /// `Ok(Some(length))` when the item ID and optional TTL prefix are complete, or `Ok(None)` when
    /// more prefix bytes are required.
    ///
    /// # Errors
    ///
    /// Returns an error when the header, item ID length, value length, flags, or TTL is invalid.
    pub fn frame_len(prefix: &[u8]) -> Result<Option<usize>> {
        let Some(header) = Self::decode_header(prefix)? else {
            return Ok(None);
        };
        header.frame_len(prefix)
    }

    /// Creates and validates a request.
    pub fn new(opcode: Opcode, item_id: Option<ItemId>, value: Vec<u8>) -> Result<Self> {
        Self::new_set(opcode, item_id, SetOptions::NONE, value)
    }

    /// Creates and validates a request with explicit `SET` options.
    pub fn new_set(
        opcode: Opcode,
        item_id: Option<ItemId>,
        set_options: SetOptions,
        value: Vec<u8>,
    ) -> Result<Self> {
        validate_value_length(value.len())?;
        validate_request_shape(
            opcode,
            item_id.is_some(),
            set_options,
            set_options.mutation_id.is_some(),
            value.len(),
        )?;
        Ok(Self {
            opcode,
            item_id,
            set_options,
            mutation_id: set_options.mutation_id,
            value,
        })
    }

    /// Creates and validates a request with an explicit mutation token.
    pub fn new_with_mutation(
        opcode: Opcode,
        item_id: Option<ItemId>,
        mutation_id: Option<MutationId>,
        value: Vec<u8>,
    ) -> Result<Self> {
        let set_options =
            mutation_id.map_or(SetOptions::NONE, |id| SetOptions::NONE.with_mutation_id(id));
        Self::new_set(opcode, item_id, set_options, value)
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
        Ok(prepend_prefix(
            std::mem::take(&mut self.value),
            prefix.as_slice(),
        ))
    }

    fn encode_prefix(&self) -> Result<RequestPrefix> {
        validate_value_length(self.value.len())?;
        validate_request_shape(
            self.opcode,
            self.item_id.is_some(),
            self.set_options,
            self.mutation_id.is_some(),
            self.value.len(),
        )?;
        let item_id_len = self.item_id.map_or(0, |_| ITEM_ID_BYTES);
        let (item_id_len_encoded, item_id_len_bytes) = encode_varuint(item_id_len as u64);
        let (value_len_encoded, value_len_bytes) = encode_varuint(self.value.len() as u64);
        let (ttl_encoded, ttl_bytes) = self
            .set_options
            .ttl_ms
            .map_or(([0; MAX_VARUINT_BYTES], 0), encode_varuint);
        let mutation_bytes = self
            .mutation_id
            .map_or([0; MUTATION_ID_BYTES], MutationId::into_bytes);
        let mutation_len = self.mutation_id.map_or(0, |_| MUTATION_ID_BYTES);
        let len = REQUEST_FIXED_BYTES
            + item_id_len_bytes
            + value_len_bytes
            + item_id_len
            + mutation_len
            + ttl_bytes;
        let mut bytes = [0; MAX_REQUEST_PREFIX_BYTES];
        bytes[0] = self.opcode as u8;
        bytes[1] = self.set_options.flags();
        let mut offset = REQUEST_FIXED_BYTES;
        bytes[offset..offset + item_id_len_bytes]
            .copy_from_slice(&item_id_len_encoded[..item_id_len_bytes]);
        offset += item_id_len_bytes;
        bytes[offset..offset + value_len_bytes]
            .copy_from_slice(&value_len_encoded[..value_len_bytes]);
        offset += value_len_bytes;
        if let Some(item_id) = self.item_id {
            bytes[offset..offset + item_id_len].copy_from_slice(item_id.as_bytes());
            offset += item_id_len;
        }
        bytes[offset..offset + mutation_len].copy_from_slice(&mutation_bytes[..mutation_len]);
        offset += mutation_len;
        bytes[offset..offset + ttl_bytes].copy_from_slice(&ttl_encoded[..ttl_bytes]);
        Ok(RequestPrefix { bytes, len })
    }

    /// Decodes and validates one complete request frame.
    pub fn decode(frame: &[u8]) -> Result<Self> {
        let decoded = decode_request_frame(frame)?;
        Ok(Self {
            opcode: decoded.opcode,
            item_id: decoded.item_id,
            set_options: decoded.set_options,
            mutation_id: decoded.mutation_id,
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
            item_id: decoded.item_id,
            set_options: decoded.set_options,
            mutation_id: decoded.mutation_id,
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
    item_id: Option<ItemId>,
    set_options: SetOptions,
    mutation_id: Option<MutationId>,
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
            expected: header.encoded_len
                + header.item_id_len
                + if header.has_mutation_id {
                    MUTATION_ID_BYTES
                } else {
                    0
                }
                + usize::from(header.has_ttl),
            actual: frame.len(),
        })?;
    if frame.len() != expected {
        return Err(ProtocolError::FrameLength {
            expected,
            actual: frame.len(),
        });
    }
    let item_id_end = header.encoded_len + header.item_id_len;
    let item_id = if header.item_id_len == 0 {
        None
    } else {
        Some(ItemId::new(
            frame[header.encoded_len..item_id_end]
                .try_into()
                .map_err(|_| ProtocolError::InvalidItemIdLength {
                    opcode: header.opcode,
                    expected: ITEM_ID_BYTES,
                    actual: header.item_id_len,
                })?,
        ))
    };
    let mutation_start = item_id_end;
    let mutation_end = mutation_start
        + if header.has_mutation_id {
            MUTATION_ID_BYTES
        } else {
            0
        };
    let mutation_id = if header.has_mutation_id {
        Some(MutationId::new(
            frame[mutation_start..mutation_end]
                .try_into()
                .map_err(|_| ProtocolError::InvalidMutationIdLength {
                    opcode: header.opcode,
                    expected: MUTATION_ID_BYTES,
                    actual: mutation_end.saturating_sub(mutation_start),
                })?,
        ))
    } else {
        None
    };
    let mut set_options = SetOptions::new(header.condition, None);
    set_options.mutation_id = mutation_id;
    let value_start = if header.has_ttl {
        let (ttl_ms, ttl_len) = decode_varuint(&frame[mutation_end..], "SET TTL")?.ok_or(
            ProtocolError::FrameTooShort {
                expected: mutation_end + MIN_VARUINT_BYTES,
                actual: frame.len(),
            },
        )?;
        set_options.ttl_ms = Some(ttl_ms);
        mutation_end + ttl_len
    } else {
        mutation_end
    };
    validate_request_shape(
        header.opcode,
        header.item_id_len != 0,
        set_options,
        mutation_id.is_some(),
        header.value_len,
    )?;
    Ok(DecodedRequestFrame {
        opcode: header.opcode,
        item_id,
        set_options,
        mutation_id,
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
        Ok(prepend_prefix(
            std::mem::take(&mut self.payload),
            prefix.as_slice(),
        ))
    }

    fn encode_prefix(&self) -> Result<ResponsePrefix> {
        validate_value_length(self.payload.len())?;
        let (length, length_bytes) = encode_varuint(self.payload.len() as u64);
        let len = RESPONSE_FIXED_BYTES + length_bytes;
        let mut bytes = [0; MAX_RESPONSE_PREFIX_BYTES];
        bytes[0] = self.status as u8;
        bytes[RESPONSE_FIXED_BYTES..len].copy_from_slice(&length[..length_bytes]);
        Ok(ResponsePrefix { bytes, len })
    }

    /// Decodes and validates one complete response frame.
    pub fn decode(frame: &[u8]) -> Result<Self> {
        let header = decode_response_frame(frame)?;
        Ok(Self {
            status: header.status,
            payload: frame[header.encoded_len..].to_vec(),
        })
    }

    /// Decodes a response while reusing the frame allocation for its payload.
    pub fn decode_owned(mut frame: Vec<u8>) -> Result<Self> {
        let header = decode_response_frame(&frame)?;
        frame.copy_within(header.encoded_len.., 0);
        frame.truncate(header.payload_len);
        Ok(Self {
            status: header.status,
            payload: frame,
        })
    }
}

fn decode_response_frame(frame: &[u8]) -> Result<ResponseHeader> {
    let header = Response::decode_header(frame)?.ok_or(ProtocolError::FrameTooShort {
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
    Ok(header)
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
    #[error("{opcode:?} requires a {expected}-byte item ID, received {actual} item ID bytes")]
    InvalidItemIdLength {
        opcode: Opcode,
        expected: usize,
        actual: usize,
    },
    #[error(
        "{opcode:?} carries an invalid mutation ID length: expected {expected}, received {actual}"
    )]
    InvalidMutationIdLength {
        opcode: Opcode,
        expected: usize,
        actual: usize,
    },
    #[error("value is too large: {size} bytes exceeds {maximum}")]
    ValueTooLarge { size: usize, maximum: usize },
    #[error("{opcode:?} requires item_id_len={expected_item_id} and value_len={expected_value}")]
    InvalidRequestShape {
        opcode: Opcode,
        expected_item_id: usize,
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

fn decode_request_flags(opcode: Opcode, flags: u8) -> Result<(SetCondition, bool, bool)> {
    let spec = operation_spec(opcode);
    let unsupported_flags = flags & !spec.allowed_flags;
    // Known SET flags on a non-SET operation are a semantic option error,
    // while bits outside the protocol's assigned flag set are malformed
    // request flags. Keeping the distinction makes diagnostics stable for
    // both callers and protocol conformance tests.
    let known_set_flags =
        SET_TTL_FLAG | SET_IF_ABSENT_FLAG | SET_IF_PRESENT_FLAG | SET_MUTATION_ID_FLAG;
    if unsupported_flags & known_set_flags != 0 {
        return Err(ProtocolError::InvalidSetOptions { opcode });
    }
    if unsupported_flags != 0 {
        return Err(ProtocolError::UnknownRequestFlags(unsupported_flags));
    }
    let mutation_id = flags & SET_MUTATION_ID_FLAG != 0;
    if flags & SET_TTL_FLAG != 0 && !spec.ttl_allowed {
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
    Ok((condition, flags & SET_TTL_FLAG != 0, mutation_id))
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

fn validate_item_id_length(opcode: Opcode, item_id_len: usize) -> Result<()> {
    let expected = operation_spec(opcode).item_id_bytes;
    if item_id_len == expected {
        Ok(())
    } else {
        Err(ProtocolError::InvalidItemIdLength {
            opcode,
            expected,
            actual: item_id_len,
        })
    }
}

fn validate_request_shape(
    opcode: Opcode,
    has_item_id: bool,
    set_options: SetOptions,
    has_mutation_id: bool,
    value_len: usize,
) -> Result<()> {
    let spec = operation_spec(opcode);
    if set_options.ttl_ms == Some(0) {
        return Err(ProtocolError::InvalidSetTtl);
    }
    if set_options.ttl_ms.is_some() && !spec.ttl_allowed {
        return Err(ProtocolError::InvalidSetOptions { opcode });
    }
    if set_options.condition != SetCondition::None && opcode != Opcode::Set {
        return Err(ProtocolError::InvalidSetOptions { opcode });
    }
    if has_mutation_id != set_options.mutation_id.is_some() || (has_mutation_id && !spec.mutation) {
        return Err(ProtocolError::InvalidSetOptions { opcode });
    }
    let valid = (has_item_id == (spec.item_id_bytes != 0))
        && value_len >= spec.value_min_bytes
        && value_len <= spec.value_max_bytes;
    if valid {
        return Ok(());
    }
    Err(ProtocolError::InvalidRequestShape {
        opcode,
        expected_item_id: spec.item_id_bytes,
        expected_value: if spec.value_min_bytes == spec.value_max_bytes {
            "0"
        } else {
            "any"
        },
    })
}
