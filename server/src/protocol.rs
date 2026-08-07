//! Server-owned semantic request codec and cache policy types.

use crate::contract::{
    DELETE_FLAGS_BYTES, DELETE_IF_EMPTY, DELETE_MODE_MASK, DELETE_RESERVED_MASK,
    NAMESPACE_NAME_MAX_BYTES, OPEN_CREATE_IF_MISSING, OPEN_FLAGS_BYTES, OPEN_RESERVED_MASK,
    POLICY_DEFAULT_EXPIRATION_MASK, POLICY_EVICTION_OVERRIDE, POLICY_EVICTION_PROTECTED,
    POLICY_EXPIRATION_OVERRIDE, POLICY_FIXED_TTL, POLICY_FLAGS_BYTES, POLICY_NO_EXPIRY,
    POLICY_RESERVED_MASK, SET_CONDITION_ANY_BITS, SET_CONDITION_MASK, SET_CONDITION_RESERVED_BITS,
    SET_EVICTABLE_BITS, SET_EVICTION_MASK, SET_EVICTION_PROTECTED_BITS, SET_EXPIRATION_MASK,
    SET_EXPLICIT_TTL_BITS, SET_FLAGS_BYTES, SET_IF_ABSENT_BITS, SET_IF_PRESENT_BITS,
    SET_INHERIT_EVICTION_BITS, SET_INHERIT_EXPIRATION_BITS, SET_NO_EXPIRY_BITS, SET_RESERVED_MASK,
};
pub use crate::contract::{WireRequestLayout, WireRequestStep, wire_request_layout};
use openkache_protocol::*;
pub use openkache_protocol::{ItemId, Opcode, Response, Status};

const MAX_POLICY_BYTES: usize = POLICY_FLAGS_BYTES + openkache_protocol::MAX_VARUINT_BYTES;

/// Encodes an ordered response field sequence from its generated field count.
///
/// Server-owned handlers provide domain values and decide what they mean. This
/// helper owns only the shared response framing contract: field cardinality,
/// length width, missing sentinel, and protocol-size validation. Optional
/// values are one reusable field-sequence encoding; the operation contract
/// remains responsible for declaring the ordered fields and their requiredness.
pub(super) fn field_sequence_response<T: AsRef<[u8]>>(
    opcode: Opcode,
    values: &[Option<T>],
) -> Response {
    let contract = crate::contract::operation_contract(opcode);
    let expected = crate::contract::operation_response_field_count(opcode);
    if !matches!(
        contract.response_route,
        crate::contract::OperationResponseRoute::FieldSequence
            | crate::contract::OperationResponseRoute::Composite
    ) || expected == 0
        || values.len() != expected
    {
        return Response::new(
            Status::InternalError,
            b"operation response fields do not match their generated plan".to_vec(),
        )
        .expect("static response fits the protocol limit");
    }
    let plan = contract.response_plan;
    if plan.len() == expected
        && values
            .iter()
            .zip(plan)
            .any(|(value, field)| field.required && value.is_none())
    {
        return Response::new(
            Status::InternalError,
            b"required operation response field is missing".to_vec(),
        )
        .expect("static response fits the protocol limit");
    }
    let encoded_values = values
        .iter()
        .map(|value| value.as_ref().map(AsRef::as_ref))
        .collect::<Vec<_>>();
    match encode_optional_values(&encoded_values) {
        Ok(payload) => Response::new(Status::Ok, payload)
            .expect("optional-value payload was validated by the protocol encoder"),
        Err(_) => Response::new(
            Status::TooLarge,
            b"operation response fields exceed the protocol limit".to_vec(),
        )
        .expect("static response fits the protocol limit"),
    }
}

/// Condition applied atomically by a `SET` request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SetCondition {
    /// Store regardless of whether the item ID exists.
    #[default]
    Any,
    /// Store only when the item ID does not exist.
    IfAbsent,
    /// Store only when the item ID already exists.
    IfPresent,
}

/// Item-level expiration selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExpirationMode {
    /// Resolve the namespace's current default at the SET linearization point.
    #[default]
    Inherit,
    /// Store without a TTL deadline.
    NoExpiry,
    /// Carry a positive `ttl_ms` in the SET request.
    ExplicitTtl,
}

/// Item-level capacity-eviction selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EvictionMode {
    /// Resolve the namespace's current default at the SET linearization point.
    #[default]
    Inherit,
    /// Permit selection by the namespace eviction algorithm.
    Evictable,
    /// Do not select this item for capacity eviction.
    EvictionProtected,
}

/// Whether a namespace permits an item to override its default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverridePolicy {
    Allowed,
    Disallowed,
}

/// Namespace expiration default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpirationDefault {
    NoExpiry,
    FixedTtl { ttl_ms: u64 },
}

/// Namespace capacity-eviction default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvictionDefault {
    Evictable,
    EvictionProtected,
}

/// Policy applied to newly written items in one namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NamespacePolicy {
    pub default_expiration: ExpirationDefault,
    pub expiration_override: OverridePolicy,
    pub default_eviction: EvictionDefault,
    pub eviction_override: OverridePolicy,
}

impl Default for NamespacePolicy {
    fn default() -> Self {
        Self {
            default_expiration: ExpirationDefault::NoExpiry,
            expiration_override: OverridePolicy::Allowed,
            default_eviction: EvictionDefault::Evictable,
            eviction_override: OverridePolicy::Allowed,
        }
    }
}

/// Namespace identity and policy returned by namespace-management operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NamespaceDescriptor {
    pub namespace_id: u64,
    pub revision: u64,
    pub policy: NamespacePolicy,
}

impl NamespaceDescriptor {
    /// Encodes the descriptor payload returned by namespace-management requests.
    pub fn encode(self) -> Result<Vec<u8>> {
        if self.namespace_id == 0 {
            return Err(ProtocolError::InvalidNamespaceId);
        }
        if self.revision == 0 {
            return Err(ProtocolError::InvalidRevision);
        }
        let mut payload =
            Vec::with_capacity(NAMESPACE_ID_BYTES + NAMESPACE_REVISION_BYTES + MAX_POLICY_BYTES);
        payload.extend_from_slice(&self.namespace_id.to_be_bytes());
        payload.extend_from_slice(&self.revision.to_be_bytes());
        payload.extend_from_slice(&self.policy.encode()?);
        Ok(payload)
    }

    /// Decodes one complete namespace descriptor payload.
    pub fn decode(input: &[u8]) -> Result<Self> {
        let fixed = NAMESPACE_ID_BYTES + NAMESPACE_REVISION_BYTES;
        if input.len() < fixed {
            return Err(ProtocolError::FrameTooShort {
                expected: fixed,
                actual: input.len(),
            });
        }
        let namespace_id = read_u64_be(input)?;
        if namespace_id == 0 {
            return Err(ProtocolError::InvalidNamespaceId);
        }
        let revision = read_u64_be(&input[NAMESPACE_ID_BYTES..])?;
        if revision == 0 {
            return Err(ProtocolError::InvalidRevision);
        }
        let (policy, policy_len) = decode_namespace_policy(&input[fixed..])?
            .ok_or(ProtocolError::MissingNamespacePolicy)?;
        if fixed + policy_len != input.len() {
            return Err(ProtocolError::FrameLength {
                expected: fixed + policy_len,
                actual: input.len(),
            });
        }
        Ok(Self {
            namespace_id,
            revision,
            policy,
        })
    }
}

/// Optional behavior for one `SET` request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SetOptions {
    /// Atomic existence condition.
    pub condition: SetCondition,
    /// Item-level expiration selection.
    pub expiration_mode: ExpirationMode,
    /// Relative lifetime in milliseconds when `expiration_mode` is `ExplicitTtl`.
    pub ttl_ms: Option<u64>,
    /// Item-level eviction selection.
    pub eviction_mode: EvictionMode,
}

impl Default for SetOptions {
    fn default() -> Self {
        Self::NONE
    }
}

impl SetOptions {
    /// Creates unconditional `SET` behavior inheriting namespace defaults.
    pub const NONE: Self = Self {
        condition: SetCondition::Any,
        expiration_mode: ExpirationMode::Inherit,
        ttl_ms: None,
        eviction_mode: EvictionMode::Inherit,
    };

    /// Creates options from an existence condition and optional explicit TTL.
    pub const fn new(condition: SetCondition, ttl_ms: Option<u64>) -> Self {
        Self {
            condition,
            expiration_mode: match ttl_ms {
                Some(_) => ExpirationMode::ExplicitTtl,
                None => ExpirationMode::Inherit,
            },
            ttl_ms,
            eviction_mode: EvictionMode::Inherit,
        }
    }

    /// Creates options with all item-level policy selections.
    pub const fn with_policies(
        condition: SetCondition,
        expiration_mode: ExpirationMode,
        ttl_ms: Option<u64>,
        eviction_mode: EvictionMode,
    ) -> Self {
        Self {
            condition,
            expiration_mode,
            ttl_ms,
            eviction_mode,
        }
    }

    fn flags(self) -> Result<u8> {
        if self.ttl_ms == Some(0) {
            return Err(ProtocolError::InvalidSetTtl);
        }
        let condition = match self.condition {
            SetCondition::Any => SET_CONDITION_ANY_BITS,
            SetCondition::IfAbsent => SET_IF_ABSENT_BITS,
            SetCondition::IfPresent => SET_IF_PRESENT_BITS,
        };
        let expiration = match self.expiration_mode {
            ExpirationMode::Inherit => {
                if self.ttl_ms.is_some() {
                    return Err(ProtocolError::UnexpectedSetTtl);
                }
                SET_INHERIT_EXPIRATION_BITS
            }
            ExpirationMode::NoExpiry => {
                if self.ttl_ms.is_some() {
                    return Err(ProtocolError::UnexpectedSetTtl);
                }
                SET_NO_EXPIRY_BITS
            }
            ExpirationMode::ExplicitTtl => {
                if self.ttl_ms.is_none() {
                    return Err(ProtocolError::MissingSetTtl);
                }
                SET_EXPLICIT_TTL_BITS
            }
        };
        let eviction = match self.eviction_mode {
            EvictionMode::Inherit => SET_INHERIT_EVICTION_BITS,
            EvictionMode::Evictable => SET_EVICTABLE_BITS,
            EvictionMode::EvictionProtected => SET_EVICTION_PROTECTED_BITS,
        };
        Ok(condition | expiration | eviction)
    }

    /// Decodes the wire SET flags and optional TTL into validated options.
    ///
    /// `ttl_ms` is present only when the wire expiration mode carries a TTL.
    /// The method validates reserved bits, mutually exclusive conditions, and
    /// the relationship between the expiration mode and TTL field.
    ///
    /// # Arguments
    ///
    /// * `flags` - The complete one-octet SET flags field.
    /// * `ttl_ms` - The optional TTL carried after the item ID.
    ///
    /// # Returns
    ///
    /// The validated SET options.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when flags are reserved or contradictory, or
    /// when the TTL does not match the selected expiration mode.
    pub fn from_wire_parts(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
        if flags & SET_RESERVED_MASK != 0 {
            return Err(ProtocolError::UnknownRequestFlags(
                flags & SET_RESERVED_MASK,
            ));
        }
        let condition = match flags & SET_CONDITION_MASK {
            SET_CONDITION_ANY_BITS => SetCondition::Any,
            SET_IF_ABSENT_BITS => SetCondition::IfAbsent,
            SET_IF_PRESENT_BITS => SetCondition::IfPresent,
            _ => return Err(ProtocolError::ConflictingSetConditions),
        };
        let expiration_mode = match flags & SET_EXPIRATION_MASK {
            SET_INHERIT_EXPIRATION_BITS => {
                if ttl_ms.is_some() {
                    return Err(ProtocolError::UnexpectedSetTtl);
                }
                ExpirationMode::Inherit
            }
            SET_NO_EXPIRY_BITS => {
                if ttl_ms.is_some() {
                    return Err(ProtocolError::UnexpectedSetTtl);
                }
                ExpirationMode::NoExpiry
            }
            SET_EXPLICIT_TTL_BITS => {
                let ttl_ms = ttl_ms.ok_or(ProtocolError::MissingSetTtl)?;
                if ttl_ms == 0 {
                    return Err(ProtocolError::InvalidSetTtl);
                }
                ExpirationMode::ExplicitTtl
            }
            _ => {
                return Err(ProtocolError::InvalidSetOptions {
                    opcode: Opcode::Set,
                });
            }
        };
        if expiration_mode == ExpirationMode::ExplicitTtl && ttl_ms == Some(0) {
            return Err(ProtocolError::InvalidSetTtl);
        }
        let eviction_mode = match flags & SET_EVICTION_MASK {
            SET_INHERIT_EVICTION_BITS => EvictionMode::Inherit,
            SET_EVICTABLE_BITS => EvictionMode::Evictable,
            SET_EVICTION_PROTECTED_BITS => EvictionMode::EvictionProtected,
            _ => {
                return Err(ProtocolError::InvalidSetOptions {
                    opcode: Opcode::Set,
                });
            }
        };
        Ok(Self {
            condition,
            expiration_mode,
            ttl_ms,
            eviction_mode,
        })
    }
}

impl NamespacePolicy {
    /// Encodes the policy bytes used by namespace-management requests and responses.
    pub fn encode(self) -> Result<Vec<u8>> {
        let mut output = Vec::with_capacity(MAX_POLICY_BYTES);
        let mut flags = match self.default_expiration {
            ExpirationDefault::NoExpiry => POLICY_NO_EXPIRY,
            ExpirationDefault::FixedTtl { ttl_ms } => {
                if ttl_ms == 0 {
                    return Err(ProtocolError::InvalidNamespacePolicy(
                        "fixed namespace TTL must be positive",
                    ));
                }
                POLICY_FIXED_TTL
            }
        };
        if self.expiration_override == OverridePolicy::Allowed {
            flags |= POLICY_EXPIRATION_OVERRIDE;
        }
        if self.default_eviction == EvictionDefault::EvictionProtected {
            flags |= POLICY_EVICTION_PROTECTED;
        }
        if self.eviction_override == OverridePolicy::Allowed {
            flags |= POLICY_EVICTION_OVERRIDE;
        }
        output.push(flags);
        if let ExpirationDefault::FixedTtl { ttl_ms } = self.default_expiration {
            let (encoded, length) = encode_varuint(ttl_ms);
            output.extend_from_slice(&encoded[..length]);
        }
        Ok(output)
    }

    /// Decodes one complete policy from the beginning of `input`.
    pub fn decode(input: &[u8]) -> Result<Option<(Self, usize)>> {
        decode_namespace_policy(input)
    }

    /// Decodes the wire policy flags and optional default TTL.
    ///
    /// A TTL is required for `FixedTtl` and forbidden for `NoExpiry`.
    ///
    /// # Arguments
    ///
    /// * `flags` - The complete one-octet namespace policy flags field.
    /// * `ttl_ms` - The optional namespace default TTL.
    ///
    /// # Returns
    ///
    /// The validated namespace policy.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when flags are reserved or the TTL does not
    /// match the selected default-expiration mode.
    pub fn from_wire_parts(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
        decode_namespace_policy_parts(flags, ttl_ms)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestLayout {
    Empty,
    ApplicationValue,
    FieldSequence,
    Item,
    Set,
    Namespace,
    NamespaceOpen,
    NamespaceUpdatePolicy,
    NamespaceDelete,
}

/// Resolves a server request codec shape from generated byte-layout metadata.
///
/// This is deliberately kept in the server adapter. The protocol crate only
/// consumes the generated byte steps needed to find a frame boundary.
fn request_layout(opcode: Opcode) -> RequestLayout {
    match crate::contract::operation_contract(opcode).request_route {
        crate::contract::OperationRequestRoute::Empty => RequestLayout::Empty,
        crate::contract::OperationRequestRoute::ApplicationValue => RequestLayout::ApplicationValue,
        crate::contract::OperationRequestRoute::FieldSequence => RequestLayout::FieldSequence,
        crate::contract::OperationRequestRoute::Item => RequestLayout::Item,
        crate::contract::OperationRequestRoute::Set => RequestLayout::Set,
        crate::contract::OperationRequestRoute::Namespace => RequestLayout::Namespace,
        crate::contract::OperationRequestRoute::NamespaceOpen => RequestLayout::NamespaceOpen,
        crate::contract::OperationRequestRoute::NamespaceUpdatePolicy => {
            RequestLayout::NamespaceUpdatePolicy
        }
        crate::contract::OperationRequestRoute::NamespaceDelete => RequestLayout::NamespaceDelete,
    }
}

/// A validated variable-length request header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestHeader {
    opcode: Opcode,
    encoded_len: usize,
    value_len: usize,
    namespace_id: Option<u64>,
    item_id_start: Option<usize>,
    item_id_count: usize,
    set_options: SetOptions,
    has_ttl: bool,
}

impl RequestHeader {
    /// Returns the decoded operation.
    pub const fn opcode(self) -> Opcode {
        self.opcode
    }

    /// Returns the number of encoded bytes before a SET value.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the fixed item ID length for operations carrying an item ID.
    pub const fn item_id_len(self) -> usize {
        ITEM_ID_BYTES * self.item_id_count
    }

    /// Returns the number of item IDs carried by this request.
    pub const fn item_id_count(self) -> usize {
        self.item_id_count
    }

    /// Returns the opaque SET or application-value length, or zero for other operations.
    pub const fn value_len(self) -> usize {
        self.value_len
    }

    /// Returns the namespace ID carried by this request, when applicable.
    pub const fn namespace_id(self) -> Option<u64> {
        self.namespace_id
    }

    /// Returns whether a TTL varuint follows the SET item ID.
    pub const fn has_ttl(self) -> bool {
        self.has_ttl
    }

    /// Reports the complete frame length once all metadata is available.
    pub fn frame_len(self, _prefix: &[u8]) -> Result<Option<usize>> {
        self.encoded_len
            .checked_add(self.value_len)
            .map(Some)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }
}

/// Header metadata required to delimit one v1 request without decoding its
/// operation semantics.
///
/// The request body remains opaque. In particular, this type does not expose
/// namespace IDs, item IDs, SET policy, or an operation response contract.
/// Server-owned handlers and generated client codecs decide what those bytes
/// mean after the frame has been delimited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestFrameHeader {
    opcode: Opcode,
    encoded_len: usize,
    value_len: usize,
}

type WireResult<T> = openkache_protocol::Result<T>;
type WireError = openkache_protocol::ProtocolError;

impl RequestFrameHeader {
    /// Returns the operation discriminator.
    pub const fn opcode(self) -> Opcode {
        self.opcode
    }

    /// Returns the number of bytes before an application value body.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the value length carried by a value-bearing layout.
    ///
    /// This is framing metadata only. A server may apply a smaller
    /// operation-specific limit after it decodes the opaque body.
    pub const fn value_len(self) -> usize {
        self.value_len
    }

    /// Returns the complete frame length once the header is available.
    pub fn frame_len(self) -> WireResult<usize> {
        self.encoded_len
            .checked_add(self.value_len)
            .ok_or(WireError::FrameLengthOverflow)
    }
}

/// Reads one request header using only generated wire layout metadata.
///
/// This parser deliberately accepts operation-specific flag and field values
/// that are semantically invalid. It only performs checks needed to find the
/// next frame boundary (canonical varuints, representable lengths, and
/// complete fixed-width fields). The semantic `Request` decoder remains
/// available for the legacy built-in server/client adapters.
fn decode_wire_request_header(prefix: &[u8]) -> WireResult<Option<RequestFrameHeader>> {
    let Some(&opcode_byte) = prefix.first() else {
        return Ok(None);
    };
    let opcode = Opcode::try_from(opcode_byte)?;
    let layout = wire_request_layout(opcode);
    let mut cursor: usize = 0;
    let mut value_len = 0;
    for step in layout.steps {
        match *step {
            WireRequestStep::Fixed { bytes } => {
                let end = cursor
                    .checked_add(bytes)
                    .ok_or(WireError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(None);
                }
                cursor = end;
            }
            WireRequestStep::ValueLength => {
                let Some((length, encoded_len)) =
                    openkache_protocol::decode_varuint(&prefix[cursor..], "request value length")?
                else {
                    return Ok(None);
                };
                value_len = usize::try_from(length).map_err(|_| WireError::FrameLengthOverflow)?;
                validate_wire_value_length(value_len)?;
                cursor = cursor
                    .checked_add(encoded_len)
                    .ok_or(WireError::FrameLengthOverflow)?;
            }
            WireRequestStep::ConditionalVarUInt {
                selector_offset,
                mask,
                expected,
            } => {
                let Some(&selector) = prefix.get(selector_offset) else {
                    return Ok(None);
                };
                if selector & mask == expected {
                    let Some((_, encoded_len)) = openkache_protocol::decode_varuint(
                        &prefix[cursor..],
                        "request conditional integer",
                    )?
                    else {
                        return Ok(None);
                    };
                    cursor = cursor
                        .checked_add(encoded_len)
                        .ok_or(WireError::FrameLengthOverflow)?;
                }
            }
            WireRequestStep::ByteLength => {
                let Some(&length) = prefix.get(cursor) else {
                    return Ok(None);
                };
                let length = usize::from(length);
                let end = cursor
                    .checked_add(NAMESPACE_NAME_LENGTH_BYTES)
                    .and_then(|end| end.checked_add(length))
                    .ok_or(WireError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(None);
                }
                cursor = end;
            }
            WireRequestStep::ByteThenVarUInt {
                prefix_bytes,
                mask,
                expected,
            } => {
                let Some(encoded_len) =
                    decode_byte_then_varuint_len(&prefix[cursor..], prefix_bytes, mask, expected)?
                else {
                    return Ok(None);
                };
                cursor = cursor
                    .checked_add(encoded_len)
                    .ok_or(WireError::FrameLengthOverflow)?;
            }
            WireRequestStep::ConditionalByteThenVarUInt {
                selector_offset,
                mask,
                expected,
                prefix_bytes,
                value_mask,
                value_expected,
            } => {
                let Some(&selector) = prefix.get(selector_offset) else {
                    return Ok(None);
                };
                if selector & mask == expected {
                    let Some(encoded_len) = decode_byte_then_varuint_len(
                        &prefix[cursor..],
                        prefix_bytes,
                        value_mask,
                        value_expected,
                    )?
                    else {
                        return Ok(None);
                    };
                    cursor = cursor
                        .checked_add(encoded_len)
                        .ok_or(WireError::FrameLengthOverflow)?;
                }
            }
        }
    }
    Ok(Some(RequestFrameHeader {
        opcode,
        encoded_len: cursor,
        value_len,
    }))
}

/// Finds the length of one policy field without validating its meaning.
fn decode_byte_then_varuint_len(
    input: &[u8],
    prefix_bytes: usize,
    mask: u8,
    expected: u8,
) -> WireResult<Option<usize>> {
    let Some(&flags) = input.first() else {
        return Ok(None);
    };
    if flags & mask != expected {
        return Ok(Some(prefix_bytes));
    }
    let Some((_, ttl_len)) = openkache_protocol::decode_varuint(
        input.get(prefix_bytes..).unwrap_or_default(),
        "namespace default TTL",
    )?
    else {
        return Ok(None);
    };
    Ok(Some(
        prefix_bytes
            .checked_add(ttl_len)
            .ok_or(WireError::FrameLengthOverflow)?,
    ))
}

fn validate_wire_value_length(value_len: usize) -> WireResult<()> {
    if value_len > MAX_VALUE_BYTES {
        return Err(WireError::ValueTooLarge {
            size: value_len,
            maximum: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

/// A complete v1 request viewed as an opaque operation call.
///
/// The protocol runtime owns only frame delimiting here. It deliberately does
/// not decode namespace IDs, item IDs, values, or operation semantics. A
/// generated client codec or a server-owned handler may inspect [`body`] when
/// it knows the operation's modeled shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestFrame<'a> {
    opcode: Opcode,
    frame: &'a [u8],
    body_offset: usize,
}

impl<'a> RequestFrame<'a> {
    /// Decodes only the frame metadata needed to delimit one request.
    pub fn decode_header(prefix: &[u8]) -> WireResult<Option<RequestFrameHeader>> {
        decode_wire_request_header(prefix)
    }

    /// Reports the complete frame length once enough metadata is available.
    pub fn frame_len(prefix: &[u8]) -> WireResult<Option<usize>> {
        Self::decode_header(prefix)?
            .map(RequestFrameHeader::frame_len)
            .transpose()
    }

    /// Decodes one complete request without interpreting its operation body.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when the opcode, generated wire layout, or
    /// complete frame length is invalid.
    pub fn decode(frame: &'a [u8]) -> WireResult<Self> {
        let header = Self::decode_header(frame)?.ok_or(WireError::FrameTooShort {
            expected: REQUEST_FIXED_BYTES,
            actual: frame.len(),
        })?;
        let expected = header.frame_len()?;
        if frame.len() != expected {
            return Err(WireError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        Ok(Self {
            opcode: header.opcode,
            frame,
            body_offset: OPCODE_BYTES,
        })
    }

    /// Returns the operation discriminator.
    pub const fn opcode(self) -> Opcode {
        self.opcode
    }

    /// Returns the opaque operation body after the opcode.
    pub fn body(self) -> &'a [u8] {
        &self.frame[self.body_offset..]
    }

    /// Returns the original complete encoded frame.
    pub const fn encoded(self) -> &'a [u8] {
        self.frame
    }
}

/// A decoded OpenKache request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub opcode: Opcode,
    pub namespace_id: Option<u64>,
    pub item_ids: Vec<ItemId>,
    pub set_options: SetOptions,
    pub value: Vec<u8>,
    pub namespace_name: Option<Vec<u8>>,
    pub namespace_policy: Option<NamespacePolicy>,
    pub expected_revision: Option<u64>,
    pub create_if_missing: bool,
}

impl Request {
    /// Creates a request for an operation that has no namespace or item fields.
    pub fn new(opcode: Opcode, item_id: Option<ItemId>, value: Vec<u8>) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: None,
            item_ids: item_id.into_iter().collect(),
            set_options: SetOptions::NONE,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a data-plane request with a namespace ID.
    pub fn new_scoped(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_with_options(opcode, namespace_id, item_id, SetOptions::NONE, value)
    }

    /// Creates a data-plane request with explicit SET options.
    pub fn new_scoped_with_options(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        set_options: SetOptions,
        value: Vec<u8>,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: Some(namespace_id),
            item_ids: item_id.into_iter().collect(),
            set_options,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a data-plane request carrying one or more exact item IDs.
    pub fn new_scoped_items(
        opcode: Opcode,
        namespace_id: u64,
        item_ids: Vec<ItemId>,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: Some(namespace_id),
            item_ids,
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a SET request with explicit options.
    pub fn new_set(
        namespace_id: u64,
        item_id: ItemId,
        set_options: SetOptions,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_with_options(Opcode::Set, namespace_id, Some(item_id), set_options, value)
    }

    /// Creates a namespace-open request. An empty name is a valid name.
    pub fn namespace_open(
        name: impl AsRef<[u8]>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceOpen,
            namespace_id: None,
            item_ids: Vec::new(),
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: Some(name.as_ref().to_vec()),
            namespace_policy: policy,
            expected_revision: None,
            create_if_missing,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a namespace-policy update request.
    pub fn namespace_update_policy(
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceUpdatePolicy,
            namespace_id: Some(namespace_id),
            item_ids: Vec::new(),
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: Some(policy),
            expected_revision: Some(expected_revision),
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates an empty-only namespace-delete request.
    pub fn namespace_delete(namespace_id: u64, expected_revision: u64) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceDelete,
            namespace_id: Some(namespace_id),
            item_ids: Vec::new(),
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: None,
            expected_revision: Some(expected_revision),
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Encodes this request into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut frame = self.encode_prefix()?;
        frame.extend_from_slice(&self.value);
        Ok(frame)
    }

    /// Encodes this request while reusing its value allocation when practical.
    pub fn into_encoded(mut self) -> Result<Vec<u8>> {
        let prefix = self.encode_prefix()?;
        let value_len = self.value.len();
        self.value.reserve(prefix.len());
        self.value.resize(prefix.len() + value_len, 0);
        self.value.copy_within(0..value_len, prefix.len());
        self.value[..prefix.len()].copy_from_slice(&prefix);
        Ok(self.value)
    }

    fn encode_prefix(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut output = Vec::new();
        output.push(self.opcode as u8);
        let layout = request_layout(self.opcode);
        match layout {
            RequestLayout::Empty => {}
            RequestLayout::ApplicationValue | RequestLayout::FieldSequence => {
                let (encoded, length) = encode_varuint(self.value.len() as u64);
                output.extend_from_slice(&encoded[..length]);
            }
            RequestLayout::Item | RequestLayout::Namespace => {
                put_namespace_id(&mut output, self.namespace_id)?;
                if layout == RequestLayout::Item {
                    for item_id in &self.item_ids {
                        output.extend_from_slice(item_id.as_bytes());
                    }
                }
            }
            RequestLayout::Set => {
                put_namespace_id(&mut output, self.namespace_id)?;
                output.push(self.set_options.flags()?);
                output.extend_from_slice(
                    self.item_ids
                        .first()
                        .ok_or(ProtocolError::InvalidRequestShape {
                            opcode: self.opcode,
                            expected_item_id: ITEM_ID_BYTES,
                            expected_value: "any",
                        })?
                        .as_bytes(),
                );
                if let Some(ttl_ms) = self.set_options.ttl_ms {
                    let (encoded, length) = encode_varuint(ttl_ms);
                    output.extend_from_slice(&encoded[..length]);
                }
                let (encoded, length) = encode_varuint(self.value.len() as u64);
                output.extend_from_slice(&encoded[..length]);
            }
            RequestLayout::NamespaceOpen => {
                output.push(if self.create_if_missing {
                    OPEN_CREATE_IF_MISSING
                } else {
                    0
                });
                let name =
                    self.namespace_name
                        .as_deref()
                        .ok_or(ProtocolError::InvalidNamespaceName(
                            "namespace-open name is missing",
                        ))?;
                output.push(u8::try_from(name.len()).map_err(|_| {
                    ProtocolError::InvalidNamespaceName("namespace name exceeds 255 octets")
                })?);
                output.extend_from_slice(name);
                if self.create_if_missing {
                    output.extend_from_slice(
                        &self
                            .namespace_policy
                            .ok_or(ProtocolError::MissingNamespacePolicy)?
                            .encode()?,
                    );
                }
            }
            RequestLayout::NamespaceUpdatePolicy => {
                put_namespace_id(&mut output, self.namespace_id)?;
                put_revision(&mut output, self.expected_revision)?;
                output.extend_from_slice(
                    &self
                        .namespace_policy
                        .ok_or(ProtocolError::MissingNamespacePolicy)?
                        .encode()?,
                );
            }
            RequestLayout::NamespaceDelete => {
                output.push(DELETE_IF_EMPTY);
                put_namespace_id(&mut output, self.namespace_id)?;
                put_revision(&mut output, self.expected_revision)?;
            }
        }
        Ok(output)
    }

    /// Decodes and validates one complete request frame.
    pub fn decode(frame: &[u8]) -> Result<Self> {
        let header = Self::decode_header(frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: REQUEST_FIXED_BYTES,
            actual: frame.len(),
        })?;
        let expected = header
            .frame_len(frame)?
            .ok_or(ProtocolError::FrameTooShort {
                expected: header.encoded_len,
                actual: frame.len(),
            })?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        let item_ids = header
            .item_id_start
            .map(|start| {
                (0..header.item_id_count)
                    .map(|index| {
                        let item_start = start + index * ITEM_ID_BYTES;
                        ItemId::new(
                            frame[item_start..item_start + ITEM_ID_BYTES]
                                .try_into()
                                .expect("validated item ID range"),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let namespace_name = if request_layout(header.opcode) == RequestLayout::NamespaceOpen {
            let name_start = OPCODE_BYTES + OPEN_FLAGS_BYTES + NAMESPACE_NAME_LENGTH_BYTES;
            let name_len_offset = OPCODE_BYTES + OPEN_FLAGS_BYTES;
            Some(frame[name_start..name_start + usize::from(frame[name_len_offset])].to_vec())
        } else {
            None
        };
        let (namespace_policy, expected_revision, create_if_missing) =
            match request_layout(header.opcode) {
                RequestLayout::NamespaceOpen => {
                    let flags_offset = OPCODE_BYTES;
                    let name_len_offset = flags_offset + OPEN_FLAGS_BYTES;
                    let name_start = name_len_offset + NAMESPACE_NAME_LENGTH_BYTES;
                    let create = frame[flags_offset] & OPEN_CREATE_IF_MISSING != 0;
                    let policy = if create {
                        let start = name_start + usize::from(frame[name_len_offset]);
                        Some(
                            decode_namespace_policy(&frame[start..])?
                                .ok_or(ProtocolError::MissingNamespacePolicy)?
                                .0,
                        )
                    } else {
                        None
                    };
                    (policy, None, create)
                }
                RequestLayout::NamespaceUpdatePolicy => {
                    let revision_start = OPCODE_BYTES + NAMESPACE_ID_BYTES;
                    let revision = read_u64_be(&frame[revision_start..])?;
                    let start = revision_start + NAMESPACE_REVISION_BYTES;
                    let policy = Some(
                        decode_namespace_policy(&frame[start..])?
                            .ok_or(ProtocolError::MissingNamespacePolicy)?
                            .0,
                    );
                    (policy, Some(revision), false)
                }
                RequestLayout::NamespaceDelete => (
                    None,
                    Some(read_u64_be(
                        &frame[OPCODE_BYTES + DELETE_FLAGS_BYTES + NAMESPACE_ID_BYTES..],
                    )?),
                    false,
                ),
                _ => (None, None, false),
            };
        let ttl_ms = if header.has_ttl {
            let start = header.item_id_start.expect("SET has an item ID") + ITEM_ID_BYTES;
            Some(
                decode_varuint(&frame[start..], "SET TTL")?
                    .ok_or(ProtocolError::MissingSetTtl)?
                    .0,
            )
        } else {
            None
        };
        let request = Self {
            opcode: header.opcode,
            namespace_id: header.namespace_id,
            item_ids,
            set_options: if request_layout(header.opcode) == RequestLayout::Set {
                SetOptions::from_wire_parts(
                    frame[OPCODE_BYTES + NAMESPACE_ID_BYTES..]
                        .first()
                        .copied()
                        .unwrap(),
                    ttl_ms,
                )?
            } else {
                SetOptions::NONE
            },
            value: frame[header.encoded_len..].to_vec(),
            namespace_name,
            namespace_policy,
            expected_revision,
            create_if_missing,
        };
        request.validate()?;
        Ok(request)
    }

    /// Decodes a request while reusing the frame allocation for its value.
    pub fn decode_owned(mut frame: Vec<u8>) -> Result<Self> {
        let header = Self::decode_header(&frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: REQUEST_FIXED_BYTES,
            actual: frame.len(),
        })?;
        let expected = header
            .frame_len(&frame)?
            .ok_or(ProtocolError::FrameTooShort {
                expected: header.encoded_len,
                actual: frame.len(),
            })?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        let mut request = Self::decode(&frame)?;
        if matches!(
            request_layout(header.opcode),
            RequestLayout::Set | RequestLayout::ApplicationValue | RequestLayout::FieldSequence
        ) {
            frame.copy_within(header.encoded_len.., 0);
            frame.truncate(header.value_len);
            request.value = frame;
        }
        Ok(request)
    }

    /// Decodes a request header when enough metadata bytes are available.
    pub fn decode_header(prefix: &[u8]) -> Result<Option<RequestHeader>> {
        let Some(&opcode_byte) = prefix.first() else {
            return Ok(None);
        };
        let opcode = Opcode::try_from(opcode_byte)?;
        match request_layout(opcode) {
            RequestLayout::Empty => Ok(Some(RequestHeader {
                opcode,
                encoded_len: OPCODE_BYTES,
                value_len: 0,
                namespace_id: None,
                item_id_start: None,
                item_id_count: 0,
                set_options: SetOptions::NONE,
                has_ttl: false,
            })),
            RequestLayout::ApplicationValue => decode_application_value_header(prefix, opcode),
            RequestLayout::FieldSequence => decode_field_sequence_header(prefix, opcode),
            RequestLayout::Item => {
                let item_id_count = crate::contract::operation_contract(opcode).request_item_count;
                let required = OPCODE_BYTES
                    + NAMESPACE_ID_BYTES
                    + ITEM_ID_BYTES
                        .checked_mul(item_id_count)
                        .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < required {
                    return Ok(None);
                }
                let namespace_id = read_namespace_id(&prefix[OPCODE_BYTES..])?;
                Ok(Some(RequestHeader {
                    opcode,
                    encoded_len: required,
                    value_len: 0,
                    namespace_id: Some(namespace_id),
                    item_id_start: Some(OPCODE_BYTES + NAMESPACE_ID_BYTES),
                    item_id_count,
                    set_options: SetOptions::NONE,
                    has_ttl: false,
                }))
            }
            RequestLayout::Namespace => {
                let required = OPCODE_BYTES + NAMESPACE_ID_BYTES;
                if prefix.len() < required {
                    return Ok(None);
                }
                let namespace_id = read_namespace_id(&prefix[OPCODE_BYTES..])?;
                Ok(Some(RequestHeader {
                    opcode,
                    encoded_len: required,
                    value_len: 0,
                    namespace_id: Some(namespace_id),
                    item_id_start: None,
                    item_id_count: 0,
                    set_options: SetOptions::NONE,
                    has_ttl: false,
                }))
            }
            RequestLayout::Set => decode_set_header(prefix),
            RequestLayout::NamespaceOpen => decode_namespace_open_header(prefix),
            RequestLayout::NamespaceUpdatePolicy => decode_namespace_update_header(prefix),
            RequestLayout::NamespaceDelete => decode_namespace_delete_header(prefix),
        }
    }

    /// Reports the complete request frame length once metadata is available.
    pub fn frame_len(prefix: &[u8]) -> Result<Option<usize>> {
        Self::decode_header(prefix)?
            .map(|header| header.frame_len(prefix))
            .transpose()
            .map(|value| value.flatten())
    }

    fn validate(&self) -> Result<()> {
        validate_value_length(self.value.len())?;
        match request_layout(self.opcode) {
            RequestLayout::Empty => {
                if self.namespace_id.is_some()
                    || !self.item_ids.is_empty()
                    || self.set_options != SetOptions::NONE
                    || !self.value.is_empty()
                    || self.namespace_name.is_some()
                    || self.namespace_policy.is_some()
                    || self.expected_revision.is_some()
                    || self.create_if_missing
                {
                    return Err(ProtocolError::InvalidRequestShape {
                        opcode: self.opcode,
                        expected_item_id: 0,
                        expected_value: "0",
                    });
                }
            }
            RequestLayout::ApplicationValue => {
                if self.namespace_id.is_some()
                    || !self.item_ids.is_empty()
                    || self.set_options != SetOptions::NONE
                    || self.namespace_name.is_some()
                    || self.namespace_policy.is_some()
                    || self.expected_revision.is_some()
                    || self.create_if_missing
                {
                    return Err(ProtocolError::InvalidRequestShape {
                        opcode: self.opcode,
                        expected_item_id: 0,
                        expected_value: "any",
                    });
                }
            }
            RequestLayout::FieldSequence => {
                if self.namespace_id.is_some()
                    || !self.item_ids.is_empty()
                    || self.set_options != SetOptions::NONE
                    || self.namespace_name.is_some()
                    || self.namespace_policy.is_some()
                    || self.expected_revision.is_some()
                    || self.create_if_missing
                {
                    return Err(ProtocolError::InvalidRequestShape {
                        opcode: self.opcode,
                        expected_item_id: 0,
                        expected_value: "field sequence",
                    });
                }
            }
            RequestLayout::Item => {
                validate_namespace_id(self.namespace_id)?;
                let expected_item_count =
                    crate::contract::operation_contract(self.opcode).request_item_count;
                if self.item_ids.len() != expected_item_count
                    || self.set_options != SetOptions::NONE
                    || !self.value.is_empty()
                    || self.namespace_name.is_some()
                    || self.namespace_policy.is_some()
                    || self.expected_revision.is_some()
                    || self.create_if_missing
                {
                    return Err(ProtocolError::InvalidRequestShape {
                        opcode: self.opcode,
                        expected_item_id: ITEM_ID_BYTES * expected_item_count,
                        expected_value: "0",
                    });
                }
            }
            RequestLayout::Namespace => {
                validate_namespace_id(self.namespace_id)?;
                if !self.item_ids.is_empty()
                    || self.set_options != SetOptions::NONE
                    || !self.value.is_empty()
                    || self.namespace_name.is_some()
                    || self.namespace_policy.is_some()
                    || self.expected_revision.is_some()
                    || self.create_if_missing
                {
                    return Err(ProtocolError::InvalidRequestShape {
                        opcode: self.opcode,
                        expected_item_id: 0,
                        expected_value: "0",
                    });
                }
            }
            RequestLayout::Set => {
                validate_namespace_id(self.namespace_id)?;
                if self.item_ids.len() != 1 {
                    return Err(ProtocolError::InvalidItemIdLength {
                        opcode: self.opcode,
                        expected: ITEM_ID_BYTES,
                        actual: 0,
                    });
                }
                self.set_options.flags()?;
                if self.namespace_name.is_some()
                    || self.namespace_policy.is_some()
                    || self.expected_revision.is_some()
                    || self.create_if_missing
                {
                    return Err(ProtocolError::InvalidSetOptions {
                        opcode: self.opcode,
                    });
                }
            }
            RequestLayout::NamespaceOpen => {
                let name =
                    self.namespace_name
                        .as_deref()
                        .ok_or(ProtocolError::InvalidNamespaceName(
                            "namespace name missing",
                        ))?;
                validate_namespace_name(name)?;
                if self.create_if_missing != self.namespace_policy.is_some() {
                    return Err(if self.create_if_missing {
                        ProtocolError::MissingNamespacePolicy
                    } else {
                        ProtocolError::UnexpectedNamespacePolicy
                    });
                }
                if self.expected_revision.is_some()
                    || self.namespace_id.is_some()
                    || !self.item_ids.is_empty()
                    || self.set_options != SetOptions::NONE
                    || !self.value.is_empty()
                {
                    return Err(ProtocolError::InvalidRequestShape {
                        opcode: self.opcode,
                        expected_item_id: 0,
                        expected_value: "0",
                    });
                }
                if let Some(policy) = self.namespace_policy {
                    policy.encode()?;
                }
            }
            RequestLayout::NamespaceUpdatePolicy => {
                validate_namespace_id(self.namespace_id)?;
                validate_revision(self.expected_revision)?;
                self.namespace_policy
                    .ok_or(ProtocolError::MissingNamespacePolicy)?
                    .encode()?;
                if !self.item_ids.is_empty()
                    || self.set_options != SetOptions::NONE
                    || !self.value.is_empty()
                    || self.namespace_name.is_some()
                    || self.create_if_missing
                {
                    return Err(ProtocolError::InvalidRequestShape {
                        opcode: self.opcode,
                        expected_item_id: 0,
                        expected_value: "0",
                    });
                }
            }
            RequestLayout::NamespaceDelete => {
                validate_namespace_id(self.namespace_id)?;
                validate_revision(self.expected_revision)?;
                if !self.item_ids.is_empty()
                    || self.set_options != SetOptions::NONE
                    || !self.value.is_empty()
                    || self.namespace_name.is_some()
                    || self.namespace_policy.is_some()
                    || self.create_if_missing
                {
                    return Err(ProtocolError::InvalidRequestShape {
                        opcode: self.opcode,
                        expected_item_id: 0,
                        expected_value: "0",
                    });
                }
            }
        }
        Ok(())
    }
}

fn decode_set_header(prefix: &[u8]) -> Result<Option<RequestHeader>> {
    let fixed = OPCODE_BYTES + NAMESPACE_ID_BYTES + SET_FLAGS_BYTES + ITEM_ID_BYTES;
    if prefix.len() < fixed {
        return Ok(None);
    }
    let namespace_id = read_namespace_id(&prefix[OPCODE_BYTES..])?;
    let flags_offset = OPCODE_BYTES + NAMESPACE_ID_BYTES;
    let flags = prefix[flags_offset];
    if flags & SET_RESERVED_MASK != 0 {
        return Err(ProtocolError::UnknownRequestFlags(
            flags & SET_RESERVED_MASK,
        ));
    }
    if flags & SET_CONDITION_MASK == SET_CONDITION_RESERVED_BITS {
        return Err(ProtocolError::ConflictingSetConditions);
    }
    let has_ttl = matches!(flags & SET_EXPIRATION_MASK, SET_EXPLICIT_TTL_BITS);
    if !matches!(
        flags & SET_EXPIRATION_MASK,
        SET_INHERIT_EXPIRATION_BITS | SET_NO_EXPIRY_BITS | SET_EXPLICIT_TTL_BITS
    ) {
        return Err(ProtocolError::InvalidSetOptions {
            opcode: Opcode::Set,
        });
    }
    if !matches!(
        flags & SET_EVICTION_MASK,
        SET_INHERIT_EVICTION_BITS | SET_EVICTABLE_BITS | SET_EVICTION_PROTECTED_BITS
    ) {
        return Err(ProtocolError::InvalidSetOptions {
            opcode: Opcode::Set,
        });
    }
    let item_id_start = flags_offset + SET_FLAGS_BYTES;
    let mut cursor = fixed;
    let ttl_ms = if has_ttl {
        let Some((ttl, length)) = decode_varuint(&prefix[cursor..], "SET TTL")? else {
            return Ok(None);
        };
        if ttl == 0 {
            return Err(ProtocolError::InvalidSetTtl);
        }
        cursor += length;
        Some(ttl)
    } else {
        None
    };
    let Some((value_len, value_len_bytes)) = decode_varuint(&prefix[cursor..], "SET value length")?
    else {
        return Ok(None);
    };
    let value_len = usize::try_from(value_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
    validate_value_length(value_len)?;
    let set_options = SetOptions::from_wire_parts(flags, ttl_ms)?;
    Ok(Some(RequestHeader {
        opcode: Opcode::Set,
        encoded_len: cursor + value_len_bytes,
        value_len,
        namespace_id: Some(namespace_id),
        item_id_start: Some(item_id_start),
        item_id_count: 1,
        set_options,
        has_ttl,
    }))
}

fn decode_application_value_header(prefix: &[u8], opcode: Opcode) -> Result<Option<RequestHeader>> {
    let Some((value_len, value_len_bytes)) =
        decode_varuint(&prefix[OPCODE_BYTES..], "application value length")?
    else {
        return Ok(None);
    };
    let value_len = usize::try_from(value_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
    validate_value_length(value_len)?;
    Ok(Some(RequestHeader {
        opcode,
        encoded_len: OPCODE_BYTES + value_len_bytes,
        value_len,
        namespace_id: None,
        item_id_start: None,
        item_id_count: 0,
        set_options: SetOptions::NONE,
        has_ttl: false,
    }))
}

fn decode_field_sequence_header(prefix: &[u8], opcode: Opcode) -> Result<Option<RequestHeader>> {
    let Some((value_len, value_len_bytes)) =
        decode_varuint(&prefix[OPCODE_BYTES..], "field sequence length")?
    else {
        return Ok(None);
    };
    let value_len = usize::try_from(value_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
    validate_value_length(value_len)?;
    Ok(Some(RequestHeader {
        opcode,
        encoded_len: OPCODE_BYTES + value_len_bytes,
        value_len,
        namespace_id: None,
        item_id_start: None,
        item_id_count: 0,
        set_options: SetOptions::NONE,
        has_ttl: false,
    }))
}

fn decode_namespace_open_header(prefix: &[u8]) -> Result<Option<RequestHeader>> {
    let fixed = OPCODE_BYTES + OPEN_FLAGS_BYTES + NAMESPACE_NAME_LENGTH_BYTES;
    if prefix.len() < fixed {
        return Ok(None);
    }
    let flags = prefix[OPCODE_BYTES];
    if flags & OPEN_RESERVED_MASK != 0 {
        return Err(ProtocolError::UnknownRequestFlags(
            flags & OPEN_RESERVED_MASK,
        ));
    }
    let name_len_offset = OPCODE_BYTES + OPEN_FLAGS_BYTES;
    let name_start = fixed;
    let name_len = usize::from(prefix[name_len_offset]);
    let name_end = name_start + name_len;
    if prefix.len() < name_end {
        return Ok(None);
    }
    validate_namespace_name(&prefix[name_start..name_end])?;
    let create = flags & OPEN_CREATE_IF_MISSING != 0;
    let encoded_len = if create {
        let Some((_, policy_len)) = decode_namespace_policy(&prefix[name_end..])? else {
            return Ok(None);
        };
        name_end + policy_len
    } else {
        name_end
    };
    Ok(Some(RequestHeader {
        opcode: Opcode::NamespaceOpen,
        encoded_len,
        value_len: 0,
        namespace_id: None,
        item_id_start: None,
        item_id_count: 0,
        set_options: SetOptions::NONE,
        has_ttl: false,
    }))
}

fn decode_namespace_update_header(prefix: &[u8]) -> Result<Option<RequestHeader>> {
    let fixed = OPCODE_BYTES + NAMESPACE_ID_BYTES + NAMESPACE_REVISION_BYTES;
    if prefix.len() < fixed {
        return Ok(None);
    }
    let namespace_id = read_namespace_id(&prefix[OPCODE_BYTES..])?;
    let expected_revision = read_u64_be(&prefix[OPCODE_BYTES + NAMESPACE_ID_BYTES..])?;
    if expected_revision == 0 {
        return Err(ProtocolError::InvalidRevision);
    }
    let Some((_, policy_len)) = decode_namespace_policy(&prefix[fixed..])? else {
        return Ok(None);
    };
    Ok(Some(RequestHeader {
        opcode: Opcode::NamespaceUpdatePolicy,
        encoded_len: fixed + policy_len,
        value_len: 0,
        namespace_id: Some(namespace_id),
        item_id_start: None,
        item_id_count: 0,
        set_options: SetOptions::NONE,
        has_ttl: false,
    }))
}

fn decode_namespace_delete_header(prefix: &[u8]) -> Result<Option<RequestHeader>> {
    let fixed = OPCODE_BYTES + DELETE_FLAGS_BYTES + NAMESPACE_ID_BYTES + NAMESPACE_REVISION_BYTES;
    if prefix.len() < fixed {
        return Ok(None);
    }
    let flags_offset = OPCODE_BYTES;
    if prefix[flags_offset] & DELETE_MODE_MASK != DELETE_IF_EMPTY {
        return Err(ProtocolError::UnknownRequestFlags(prefix[flags_offset]));
    }
    if prefix[flags_offset] & DELETE_RESERVED_MASK != 0 {
        return Err(ProtocolError::UnknownRequestFlags(
            prefix[flags_offset] & DELETE_RESERVED_MASK,
        ));
    }
    let namespace_id = read_namespace_id(&prefix[flags_offset + DELETE_FLAGS_BYTES..])?;
    let expected_revision =
        read_u64_be(&prefix[flags_offset + DELETE_FLAGS_BYTES + NAMESPACE_ID_BYTES..])?;
    if expected_revision == 0 {
        return Err(ProtocolError::InvalidRevision);
    }
    Ok(Some(RequestHeader {
        opcode: Opcode::NamespaceDelete,
        encoded_len: fixed,
        value_len: 0,
        namespace_id: Some(namespace_id),
        item_id_start: None,
        item_id_count: 0,
        set_options: SetOptions::NONE,
        has_ttl: false,
    }))
}

fn decode_namespace_policy(input: &[u8]) -> Result<Option<(NamespacePolicy, usize)>> {
    let Some(&flags) = input.first() else {
        return Ok(None);
    };
    let (ttl_ms, encoded_len) = match flags & POLICY_DEFAULT_EXPIRATION_MASK {
        POLICY_NO_EXPIRY => (None, POLICY_FLAGS_BYTES),
        POLICY_FIXED_TTL => {
            let Some((ttl_ms, length)) =
                decode_varuint(&input[POLICY_FLAGS_BYTES..], "namespace default TTL")?
            else {
                return Ok(None);
            };
            (Some(ttl_ms), POLICY_FLAGS_BYTES + length)
        }
        _ => {
            return Err(ProtocolError::InvalidNamespacePolicy(
                "namespace default expiration is reserved",
            ));
        }
    };
    let policy = decode_namespace_policy_parts(flags, ttl_ms)?;
    Ok(Some((policy, encoded_len)))
}

fn decode_namespace_policy_parts(flags: u8, ttl_ms: Option<u64>) -> Result<NamespacePolicy> {
    if flags & POLICY_RESERVED_MASK != 0 {
        return Err(ProtocolError::InvalidNamespacePolicy(
            "namespace policy contains reserved bits",
        ));
    }
    let default_expiration = match flags & POLICY_DEFAULT_EXPIRATION_MASK {
        POLICY_NO_EXPIRY => {
            if ttl_ms.is_some() {
                return Err(ProtocolError::InvalidNamespacePolicy(
                    "namespace default TTL is only valid with fixed TTL mode",
                ));
            }
            ExpirationDefault::NoExpiry
        }
        POLICY_FIXED_TTL => {
            let ttl_ms = ttl_ms.ok_or(ProtocolError::InvalidNamespacePolicy(
                "fixed namespace TTL is missing",
            ))?;
            if ttl_ms == 0 {
                return Err(ProtocolError::InvalidNamespacePolicy(
                    "fixed namespace TTL must be positive",
                ));
            }
            ExpirationDefault::FixedTtl { ttl_ms }
        }
        _ => {
            return Err(ProtocolError::InvalidNamespacePolicy(
                "namespace default expiration is reserved",
            ));
        }
    };
    Ok(NamespacePolicy {
        default_expiration,
        expiration_override: if flags & POLICY_EXPIRATION_OVERRIDE != 0 {
            OverridePolicy::Allowed
        } else {
            OverridePolicy::Disallowed
        },
        default_eviction: if flags & POLICY_EVICTION_PROTECTED != 0 {
            EvictionDefault::EvictionProtected
        } else {
            EvictionDefault::Evictable
        },
        eviction_override: if flags & POLICY_EVICTION_OVERRIDE != 0 {
            OverridePolicy::Allowed
        } else {
            OverridePolicy::Disallowed
        },
    })
}

fn validate_namespace_name(name: &[u8]) -> Result<()> {
    if name.len() > NAMESPACE_NAME_MAX_BYTES {
        return Err(ProtocolError::InvalidNamespaceName(
            "namespace name exceeds 255 octets",
        ));
    }
    std::str::from_utf8(name)
        .map_err(|_| ProtocolError::InvalidNamespaceName("namespace name is not UTF-8"))?;
    Ok(())
}

fn validate_namespace_id(namespace_id: Option<u64>) -> Result<u64> {
    match namespace_id {
        Some(namespace_id @ 1..) => Ok(namespace_id),
        Some(0) => Err(ProtocolError::InvalidNamespaceId),
        None => Err(ProtocolError::MissingNamespaceId),
    }
}

fn validate_revision(revision: Option<u64>) -> Result<u64> {
    match revision {
        Some(revision @ 1..) => Ok(revision),
        _ => Err(ProtocolError::InvalidRevision),
    }
}

fn put_namespace_id(output: &mut Vec<u8>, namespace_id: Option<u64>) -> Result<()> {
    output.extend_from_slice(&validate_namespace_id(namespace_id)?.to_be_bytes());
    Ok(())
}

fn put_revision(output: &mut Vec<u8>, revision: Option<u64>) -> Result<()> {
    output.extend_from_slice(&validate_revision(revision)?.to_be_bytes());
    Ok(())
}

fn read_u64_be(input: &[u8]) -> Result<u64> {
    let bytes: [u8; NAMESPACE_ID_BYTES] = input
        .get(..NAMESPACE_ID_BYTES)
        .ok_or(ProtocolError::FrameTooShort {
            expected: NAMESPACE_ID_BYTES,
            actual: input.len(),
        })?
        .try_into()
        .expect("slice length checked");
    Ok(u64::from_be_bytes(bytes))
}

fn read_namespace_id(input: &[u8]) -> Result<u64> {
    let value = read_u64_be(input)?;
    if value == 0 {
        return Err(ProtocolError::InvalidNamespaceId);
    }
    Ok(value)
}

fn encode_varuint(value: u64) -> ([u8; MAX_VARUINT_BYTES], usize) {
    openkache_protocol::encode_varuint(value)
}

fn decode_varuint(input: &[u8], context: &'static str) -> Result<Option<(u64, usize)>> {
    openkache_protocol::decode_varuint(input, context).map_err(Into::into)
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
    #[error("value is too large: {size} bytes exceeds {maximum}")]
    ValueTooLarge { size: usize, maximum: usize },
    #[error("optional-value payload is invalid: {0}")]
    InvalidOptionalValues(&'static str),
    #[error("operation field sequence is invalid: {0}")]
    InvalidFieldSequence(&'static str),
    #[error("{opcode:?} requires a fixed item/value shape ({expected_item_id}, {expected_value})")]
    InvalidRequestShape {
        opcode: Opcode,
        expected_item_id: usize,
        expected_value: &'static str,
    },
    #[error("if-absent and if-present conditions cannot be combined")]
    ConflictingSetConditions,
    #[error("SET TTL must be greater than zero milliseconds")]
    InvalidSetTtl,
    #[error("SET TTL is required by ExplicitTtl")]
    MissingSetTtl,
    #[error("SET TTL is not allowed by this expiration mode")]
    UnexpectedSetTtl,
    #[error("SET options are not valid for {opcode:?}")]
    InvalidSetOptions { opcode: Opcode },
    #[error("namespace ID is missing")]
    MissingNamespaceId,
    #[error("namespace ID must be a positive non-zero u64")]
    InvalidNamespaceId,
    #[error("namespace name is invalid: {0}")]
    InvalidNamespaceName(&'static str),
    #[error("namespace policy is missing")]
    MissingNamespacePolicy,
    #[error("namespace policy is not allowed")]
    UnexpectedNamespacePolicy,
    #[error("namespace policy is invalid: {0}")]
    InvalidNamespacePolicy(&'static str),
    #[error("namespace revision must be positive")]
    InvalidRevision,
}

impl From<openkache_protocol::ProtocolError> for ProtocolError {
    fn from(error: openkache_protocol::ProtocolError) -> Self {
        match error {
            openkache_protocol::ProtocolError::UnknownOpcode(value) => Self::UnknownOpcode(value),
            openkache_protocol::ProtocolError::UnknownStatus(value) => Self::UnknownStatus(value),
            openkache_protocol::ProtocolError::FrameTooShort { expected, actual } => {
                Self::FrameTooShort { expected, actual }
            }
            openkache_protocol::ProtocolError::FrameLength { expected, actual } => {
                Self::FrameLength { expected, actual }
            }
            openkache_protocol::ProtocolError::FrameLengthOverflow => Self::FrameLengthOverflow,
            openkache_protocol::ProtocolError::NonCanonicalVaruint { context } => {
                Self::NonCanonicalVaruint { context }
            }
            openkache_protocol::ProtocolError::VaruintOverflow { context } => {
                Self::VaruintOverflow { context }
            }
            openkache_protocol::ProtocolError::ValueTooLarge { size, maximum } => {
                Self::ValueTooLarge { size, maximum }
            }
            openkache_protocol::ProtocolError::InvalidOptionalValues(message) => {
                Self::InvalidOptionalValues(message)
            }
            openkache_protocol::ProtocolError::InvalidFieldSequence(message) => {
                Self::InvalidFieldSequence(message)
            }
        }
    }
}

/// Convenience result type for protocol operations.
pub type Result<T> = std::result::Result<T, ProtocolError>;
