//! Client-owned request and domain codecs.
//!
//! The public protocol crate only finds opaque frame boundaries.  This module
//! is the Rust client's semantic adapter: it validates the modeled request
//! shapes and encodes namespace policy and SET options.

use crate::contract::{
    MAX_OPERATION_REQUEST_FIELDS, POLICY_EVICTION_OVERRIDE, POLICY_EVICTION_PROTECTED,
    POLICY_EXPIRATION_OVERRIDE, POLICY_FIXED_TTL, POLICY_FLAGS_BYTES, POLICY_NO_EXPIRY,
    SET_CONDITION_ANY_BITS, SET_EVICTABLE_BITS, SET_EVICTION_PROTECTED_BITS, SET_EXPLICIT_TTL_BITS,
    SET_IF_ABSENT_BITS, SET_IF_PRESENT_BITS, SET_INHERIT_EVICTION_BITS,
    SET_INHERIT_EXPIRATION_BITS, SET_NO_EXPIRY_BITS, operation_wire_spec,
};
use openkache_protocol::{
    ItemId, MAX_VALUE_BYTES, NAMESPACE_ID_BYTES, NAMESPACE_REVISION_BYTES, Opcode,
    decode_layout_fields, decode_varuint, encode_varuint,
};

/// Client-adapter validation errors.
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
    #[error("request flags contain unknown bits 0x{0:02x}")]
    UnknownRequestFlags(u8),
    #[error("if-absent and if-present conditions cannot be combined")]
    ConflictingSetConditions,
    #[error("invalid optional-values payload: {0}")]
    InvalidOptionalValues(&'static str),
    #[error("invalid operation field sequence: {0}")]
    InvalidFieldSequence(&'static str),
    #[error("invalid operation field codec: {0}")]
    InvalidFieldCodec(&'static str),
    #[error("{opcode:?} requires a fixed item/value shape ({expected_item_id}, {expected_value})")]
    InvalidRequestShape {
        opcode: Opcode,
        expected_item_id: usize,
        expected_value: &'static str,
    },
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

type Result<T> = std::result::Result<T, ProtocolError>;

const MAX_POLICY_BYTES: usize = POLICY_FLAGS_BYTES + openkache_protocol::MAX_VARUINT_BYTES;
#[path = "protocol_adapters.rs"]
mod adapters;
#[path = "protocol_compat_v1.rs"]
pub(crate) mod compat_v1;
#[path = "protocol_generic.rs"]
pub(crate) mod generic;
pub(crate) use self::compat_v1::{
    compact_item_count, uses_compact_item_route, uses_compact_namespace_route,
};
pub use self::generic::OperationFields;
pub(crate) use self::generic::{decode_response_fields_view, validate_operation_field};

/// Builds one protocol-v1 request from the generated operation contract.
///
/// The caller supplies only the generic request inputs. Framing selection and
/// the compact namespace/item/SET projection stay in this adapter so the
/// client execution surface does not grow a route family for each API.
pub(crate) fn request_from_contract(
    operation: Opcode,
    namespace_id: Option<u64>,
    item_id: &[u8],
    value: Vec<u8>,
    set_options: crate::SetOptions,
) -> crate::Result<Request> {
    adapters::request_from_contract(operation, namespace_id, item_id, value, set_options)
}

/// Builds a request for a generic unary operation from its already encoded
/// body.
///
/// This is the neutral client boundary: callers provide only the opcode and
/// the operation body. Namespace IDs, item IDs, and SET options belong to the
/// protocol-v1 compatibility adapter and are intentionally unavailable here;
/// a compatibility opcode reports that generic unary requests cannot use the
/// compact protocol-v1 projection.
pub(crate) fn request_from_unary(operation: Opcode, body: Vec<u8>) -> crate::Result<Request> {
    adapters::request_from_unary(operation, body)
}

/// Builds an ordered-field request from values already validated by the
/// generated descriptor.
pub(crate) fn request_from_fields(
    operation: Opcode,
    fields: Vec<Option<Vec<u8>>>,
) -> crate::Result<Request> {
    adapters::request_from_fields(operation, fields)
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
        let (policy, policy_len) = compat_v1::decode_namespace_policy(&input[fixed..])?
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

impl NamespacePolicy {
    /// Encodes the namespace policy payload.
    pub fn encode(self) -> Result<Vec<u8>> {
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
        let mut output = Vec::with_capacity(MAX_POLICY_BYTES);
        output.push(flags);
        if let ExpirationDefault::FixedTtl { ttl_ms } = self.default_expiration {
            let (encoded, length) = encode_varuint(ttl_ms);
            output.extend_from_slice(&encoded[..length]);
        }
        Ok(output)
    }
}

/// Client-side representation of SET policy bits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SetWireOptions {
    pub condition: SetCondition,
    pub expiration_mode: ExpirationMode,
    pub ttl_ms: Option<u64>,
    pub eviction_mode: EvictionMode,
}

impl SetWireOptions {
    /// The ordinary unconditional SET behavior.
    pub const NONE: Self = Self {
        condition: SetCondition::Any,
        expiration_mode: ExpirationMode::Inherit,
        ttl_ms: None,
        eviction_mode: EvictionMode::Inherit,
    };

    /// Creates options with all wire policy selections.
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

    pub(crate) fn flags(self) -> Result<u8> {
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
}

/// A validated request owned by the Rust client adapter.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RequestRetryPolicy {
    /// The request is safe to replay after a connection failure.
    Always,
    /// The request must not be replayed automatically.
    Never,
}

/// Applies the Smithy-generated replay declaration to an owned request.
///
/// Retry safety is attached when the operation adapter constructs the request;
/// the transport only consumes this closed policy and never branches on an
/// operation name or domain-specific mutation. The declaration lives in the
/// client metadata envelope, outside the normative wire layout.
pub(crate) fn generated_retry_policy(
    opcode: Opcode,
    create_if_missing: bool,
) -> RequestRetryPolicy {
    match crate::contract::operation_client_projection(opcode).retry_mode {
        crate::contract::OperationRetryMode::Always => RequestRetryPolicy::Always,
        crate::contract::OperationRetryMode::Never => RequestRetryPolicy::Never,
        crate::contract::OperationRetryMode::WhenNotCreating => {
            if create_if_missing {
                RequestRetryPolicy::Never
            } else {
                RequestRetryPolicy::Always
            }
        }
    }
}

impl RequestRetryPolicy {
    pub(crate) const fn is_safe(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) opcode: Opcode,
    pub(crate) namespace_id: Option<u64>,
    pub(crate) item_ids: Vec<ItemId>,
    pub(crate) set_options: SetWireOptions,
    pub(crate) value: Vec<u8>,
    pub(crate) namespace_name: Option<Vec<u8>>,
    pub(crate) namespace_policy: Option<NamespacePolicy>,
    pub(crate) expected_revision: Option<u64>,
    pub(crate) create_if_missing: bool,
    pub(crate) retry_policy: RequestRetryPolicy,
}

/// Owned request pieces ready for a transport write.
///
/// Keeping the protocol-v1 prefix separate from the already-owned payload
/// avoids shifting or copying the payload merely to prepend framing.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RequestParts {
    pub(crate) prefix: Vec<u8>,
    pub(crate) payload: Vec<u8>,
}

impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        self.opcode == other.opcode
            && self.namespace_id == other.namespace_id
            && self.item_ids == other.item_ids
            && self.set_options == other.set_options
            && self.value == other.value
            && self.namespace_name == other.namespace_name
            && self.namespace_policy == other.namespace_policy
            && self.expected_revision == other.expected_revision
            && self.create_if_missing == other.create_if_missing
    }
}

impl Eq for Request {}

impl Request {
    #[allow(dead_code)]
    pub(crate) fn new(opcode: Opcode, item_id: Option<ItemId>, value: Vec<u8>) -> Result<Self> {
        Self::new_with_retry_policy(
            opcode,
            item_id,
            value,
            generated_retry_policy(opcode, false),
        )
    }

    /// Builds a route-less request from an already encoded generic body.
    ///
    /// Namespace, item, and SET fields are compatibility-only concerns; new
    /// APIs use this constructor or the generated field helper instead of
    /// populating the historical request facade.
    pub(crate) fn new_generic(opcode: Opcode, value: Vec<u8>) -> Result<Self> {
        Self::new_generic_with_retry_policy(opcode, value, generated_retry_policy(opcode, false))
    }

    pub(crate) fn new_generic_with_retry_policy(
        opcode: Opcode,
        value: Vec<u8>,
        retry_policy: RequestRetryPolicy,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: None,
            item_ids: Vec::new(),
            set_options: SetWireOptions::NONE,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
            retry_policy,
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn new_with_retry_policy(
        opcode: Opcode,
        item_id: Option<ItemId>,
        value: Vec<u8>,
        retry_policy: RequestRetryPolicy,
    ) -> Result<Self> {
        if item_id.is_none() {
            return Self::new_generic_with_retry_policy(opcode, value, retry_policy);
        }
        let request = Self {
            opcode,
            namespace_id: None,
            item_ids: item_id.into_iter().collect(),
            set_options: SetWireOptions::NONE,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
            retry_policy,
        };
        request.validate()?;
        Ok(request)
    }

    /// Constructs an ordered request after its generated field plan has
    /// already validated the payload. Keeping this private avoids validating
    /// and allocating field offsets a second time in the client adapter.
    fn new_ordered_unchecked(opcode: Opcode, value: Vec<u8>) -> Self {
        Self {
            opcode,
            namespace_id: None,
            item_ids: Vec::new(),
            set_options: SetWireOptions::NONE,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
            retry_policy: generated_retry_policy(opcode, false),
        }
    }

    pub(crate) fn new_scoped(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_with_retry_policy(
            opcode,
            namespace_id,
            item_id,
            value,
            generated_retry_policy(opcode, false),
        )
    }

    pub(crate) fn new_scoped_with_options(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        set_options: SetWireOptions,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_items_with_retry_policy(
            opcode,
            namespace_id,
            item_id.into_iter().collect(),
            set_options,
            value,
            generated_retry_policy(opcode, false),
        )
    }

    pub(crate) fn new_scoped_retryable(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_with_retry_policy(
            opcode,
            namespace_id,
            item_id,
            value,
            generated_retry_policy(opcode, false),
        )
    }

    pub(crate) fn new_scoped_with_retry_policy(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        value: Vec<u8>,
        retry_policy: RequestRetryPolicy,
    ) -> Result<Self> {
        Self::new_scoped_items_with_retry_policy(
            opcode,
            namespace_id,
            item_id.into_iter().collect(),
            SetWireOptions::NONE,
            value,
            retry_policy,
        )
    }

    pub(crate) fn new_scoped_items_with_options(
        opcode: Opcode,
        namespace_id: u64,
        item_ids: Vec<ItemId>,
        set_options: SetWireOptions,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_items_with_retry_policy(
            opcode,
            namespace_id,
            item_ids,
            set_options,
            value,
            generated_retry_policy(opcode, false),
        )
    }

    pub(crate) fn new_scoped_items_with_retry_policy(
        opcode: Opcode,
        namespace_id: u64,
        item_ids: Vec<ItemId>,
        set_options: SetWireOptions,
        value: Vec<u8>,
        retry_policy: RequestRetryPolicy,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: Some(namespace_id),
            item_ids,
            set_options,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
            retry_policy,
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn namespace_open(
        name: impl AsRef<[u8]>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceOpen,
            namespace_id: None,
            item_ids: Vec::new(),
            set_options: SetWireOptions::NONE,
            value: Vec::new(),
            namespace_name: Some(name.as_ref().to_vec()),
            namespace_policy: policy,
            expected_revision: None,
            create_if_missing,
            // Replay safety is decided by this typed namespace adapter while
            // constructing the request. The generic request core does not
            // inspect `create_if_missing` or any operation-specific field.
            retry_policy: generated_retry_policy(Opcode::NamespaceOpen, create_if_missing),
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn namespace_update_policy(
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceUpdatePolicy,
            namespace_id: Some(namespace_id),
            item_ids: Vec::new(),
            set_options: SetWireOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: Some(policy),
            expected_revision: Some(expected_revision),
            create_if_missing: false,
            retry_policy: generated_retry_policy(Opcode::NamespaceUpdatePolicy, false),
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn namespace_delete(namespace_id: u64, expected_revision: u64) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceDelete,
            namespace_id: Some(namespace_id),
            item_ids: Vec::new(),
            set_options: SetWireOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: None,
            expected_revision: Some(expected_revision),
            create_if_missing: false,
            retry_policy: generated_retry_policy(Opcode::NamespaceDelete, false),
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn into_parts(self) -> Result<RequestParts> {
        // Every constructor validates its generated framing before returning.
        // Avoid replaying the ordered-field decoder at the transport boundary.
        let prefix = if let Some(prefix) = compat_v1::encode_prefix(&self)? {
            prefix
        } else {
            let mut prefix = vec![self.opcode as u8];
            match operation_wire_spec(self.opcode).request.framing {
                crate::contract::OperationLayoutFraming::Empty => {}
                crate::contract::OperationLayoutFraming::Opaque => {
                    append_varuint(&mut prefix, self.value.len() as u64);
                }
                crate::contract::OperationLayoutFraming::OrderedFields
                | crate::contract::OperationLayoutFraming::FieldSequence => {
                    if operation_wire_spec(self.opcode).request.frame
                        == crate::contract::OperationFramePolicy::LengthDelimited
                    {
                        append_varuint(&mut prefix, self.value.len() as u64);
                    }
                }
                crate::contract::OperationLayoutFraming::OptionalValues => {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "optional-value framing is response-only",
                    ));
                }
            }
            prefix
        };
        validate_value_length(self.value.len())?;
        Ok(RequestParts {
            prefix,
            payload: self.value,
        })
    }

    fn validate(&self) -> Result<()> {
        validate_value_length(self.value.len())?;
        if compat_v1::validate_request(self)? {
            return Ok(());
        }
        match operation_wire_spec(self.opcode).request.framing {
            crate::contract::OperationLayoutFraming::Empty => {
                if self.has_non_empty_fields() {
                    return Err(invalid_shape(self.opcode, 0, "0"));
                }
            }
            crate::contract::OperationLayoutFraming::Opaque => {
                if self.namespace_id.is_some()
                    || !self.item_ids.is_empty()
                    || self.set_options != SetWireOptions::NONE
                    || self.namespace_name.is_some()
                    || self.namespace_policy.is_some()
                    || self.expected_revision.is_some()
                    || self.create_if_missing
                {
                    return Err(invalid_shape(self.opcode, 0, "any"));
                }
                if let Some(field) = operation_wire_spec(self.opcode).request.fields.first() {
                    validate_operation_field(field, &self.value)?;
                }
            }
            crate::contract::OperationLayoutFraming::OrderedFields
            | crate::contract::OperationLayoutFraming::FieldSequence => {
                if self.set_options != SetWireOptions::NONE
                    || self.namespace_name.is_some()
                    || self.namespace_policy.is_some()
                    || self.expected_revision.is_some()
                    || self.create_if_missing
                {
                    return Err(invalid_shape(self.opcode, 0, "field sequence"));
                }
                let contract = operation_wire_spec(self.opcode);
                let plan = contract.request.fields;
                if plan.len() > MAX_OPERATION_REQUEST_FIELDS {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "generated operation request field bound is stale",
                    ));
                }
                let mut offsets = [(usize::MAX, usize::MAX); MAX_OPERATION_REQUEST_FIELDS];
                let required: [bool; MAX_OPERATION_REQUEST_FIELDS] = std::array::from_fn(|index| {
                    contract
                        .request
                        .fields
                        .get(index)
                        .is_some_and(|field| field.required)
                });
                let widths: [usize; MAX_OPERATION_REQUEST_FIELDS] = std::array::from_fn(|index| {
                    contract
                        .request
                        .fields
                        .get(index)
                        .map_or(0, |field| field.encoded_width)
                });
                decode_layout_fields(
                    &self.value,
                    contract.request.layout,
                    &required[..plan.len()],
                    &widths[..plan.len()],
                    &mut offsets[..plan.len()],
                )
                .map_err(ProtocolError::from)?;
                for (field, value) in plan.iter().zip((0..plan.len()).map(|index| {
                    let (start, end) = offsets[index];
                    (start != usize::MAX).then(|| &self.value[start..end])
                })) {
                    if field.required && value.is_none() {
                        return Err(ProtocolError::InvalidFieldSequence(
                            "required request field is missing",
                        ));
                    }
                    if let Some(value) = value {
                        validate_operation_field(field, value)?;
                    }
                }
            }
            crate::contract::OperationLayoutFraming::OptionalValues => {
                return Err(ProtocolError::InvalidFieldSequence(
                    "optional-value framing is response-only",
                ));
            }
        }
        Ok(())
    }

    fn has_non_empty_fields(&self) -> bool {
        self.namespace_id.is_some()
            || !self.item_ids.is_empty()
            || self.set_options != SetWireOptions::NONE
            || !self.value.is_empty()
            || self.namespace_name.is_some()
            || self.namespace_policy.is_some()
            || self.expected_revision.is_some()
            || self.create_if_missing
    }

    fn has_non_empty_fields_except_namespace(&self) -> bool {
        !self.item_ids.is_empty()
            || self.set_options != SetWireOptions::NONE
            || !self.value.is_empty()
            || self.namespace_name.is_some()
            || self.namespace_policy.is_some()
            || self.expected_revision.is_some()
            || self.create_if_missing
    }

    fn has_non_empty_fields_except_namespace_revision(&self) -> bool {
        !self.item_ids.is_empty()
            || self.set_options != SetWireOptions::NONE
            || !self.value.is_empty()
            || self.namespace_name.is_some()
            || self.namespace_policy.is_some()
            || self.create_if_missing
    }
}

fn append_varuint(output: &mut Vec<u8>, value: u64) {
    let (encoded, length) = encode_varuint(value);
    output.extend_from_slice(&encoded[..length]);
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

fn validate_value_length(value_len: usize) -> Result<()> {
    if value_len > MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: value_len,
            maximum: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

fn invalid_shape(
    opcode: Opcode,
    expected_item_id: usize,
    expected_value: &'static str,
) -> ProtocolError {
    ProtocolError::InvalidRequestShape {
        opcode,
        expected_item_id,
        expected_value,
    }
}
