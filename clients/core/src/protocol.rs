//! Client-owned request and domain codecs.
//!
//! The public protocol crate only finds opaque frame boundaries.  This module
//! is the Rust client's semantic adapter: it validates the modeled request
//! shapes and encodes namespace policy and SET options.

use openkache_protocol::{
    ITEM_ID_BYTES, ItemId, MAX_VALUE_BYTES, NAMESPACE_ID_BYTES, NAMESPACE_REVISION_BYTES, Opcode,
    decode_varuint, encode_varuint,
};

use crate::contract::{
    DELETE_IF_EMPTY, NAMESPACE_NAME_MAX_BYTES, OPEN_CREATE_IF_MISSING, OperationRequestKind,
    POLICY_DEFAULT_EXPIRATION_MASK, POLICY_EVICTION_OVERRIDE, POLICY_EVICTION_PROTECTED,
    POLICY_EXPIRATION_OVERRIDE, POLICY_FIXED_TTL, POLICY_FLAGS_BYTES, POLICY_NO_EXPIRY,
    POLICY_RESERVED_MASK, SET_CONDITION_ANY_BITS, SET_CONDITION_MASK, SET_EVICTABLE_BITS,
    SET_EVICTION_MASK, SET_EVICTION_PROTECTED_BITS, SET_EXPIRATION_MASK, SET_EXPLICIT_TTL_BITS,
    SET_IF_ABSENT_BITS, SET_IF_PRESENT_BITS, SET_INHERIT_EVICTION_BITS,
    SET_INHERIT_EXPIRATION_BITS, SET_NO_EXPIRY_BITS, SET_RESERVED_MASK, operation_contract,
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
    #[error("optional-value payload is invalid: {0}")]
    InvalidOptionalValues(&'static str),
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
        }
    }
}

type Result<T> = std::result::Result<T, ProtocolError>;

const MAX_POLICY_BYTES: usize = POLICY_FLAGS_BYTES + openkache_protocol::MAX_VARUINT_BYTES;

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

    /// Decodes policy flags and an optional fixed default TTL.
    pub fn from_wire_parts(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
        decode_namespace_policy_parts(flags, ttl_ms)
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

    /// Decodes the wire SET flags and optional TTL into validated options.
    ///
    /// This semantic codec belongs to the client adapter because the shared
    /// protocol crate only finds the request frame boundary.
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
#[derive(Clone, Debug, Eq, PartialEq)]
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
}

impl Request {
    pub(crate) fn new(opcode: Opcode, item_id: Option<ItemId>, value: Vec<u8>) -> Result<Self> {
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
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn new_scoped(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_with_options(opcode, namespace_id, item_id, SetWireOptions::NONE, value)
    }

    pub(crate) fn new_scoped_with_options(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        set_options: SetWireOptions,
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

    pub(crate) fn new_scoped_items(
        opcode: Opcode,
        namespace_id: u64,
        item_ids: Vec<ItemId>,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: Some(namespace_id),
            item_ids,
            set_options: SetWireOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
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
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn into_encoded(mut self) -> Result<Vec<u8>> {
        self.validate()?;
        let prefix = self.encode_prefix()?;
        let value_len = self.value.len();
        self.value.reserve(prefix.len());
        self.value.resize(prefix.len() + value_len, 0);
        self.value.copy_within(0..value_len, prefix.len());
        self.value[..prefix.len()].copy_from_slice(&prefix);
        Ok(self.value)
    }

    fn encode_prefix(&self) -> Result<Vec<u8>> {
        let contract = operation_contract(self.opcode);
        let mut output = Vec::new();
        output.push(self.opcode as u8);
        match contract.request_kind {
            OperationRequestKind::Empty => {}
            OperationRequestKind::ApplicationValue => {
                append_varuint(&mut output, self.value.len() as u64);
            }
            OperationRequestKind::ScopedItem => {
                append_namespace_id(&mut output, self.namespace_id)?;
                if contract.request_value_count > 0 {
                    output.push(self.set_options.flags()?);
                }
                for item_id in &self.item_ids {
                    output.extend_from_slice(item_id.as_ref());
                }
                if contract.request_value_count > 0 {
                    if let Some(ttl_ms) = self.set_options.ttl_ms {
                        append_varuint(&mut output, ttl_ms);
                    }
                    append_varuint(&mut output, self.value.len() as u64);
                }
            }
            OperationRequestKind::ScopedNamespace => {
                append_namespace_id(&mut output, self.namespace_id)?;
            }
            OperationRequestKind::NamespaceOpen => {
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
            OperationRequestKind::NamespaceUpdatePolicy => {
                append_namespace_id(&mut output, self.namespace_id)?;
                append_revision(&mut output, self.expected_revision)?;
                output.extend_from_slice(
                    &self
                        .namespace_policy
                        .ok_or(ProtocolError::MissingNamespacePolicy)?
                        .encode()?,
                );
            }
            OperationRequestKind::NamespaceDelete => {
                output.push(DELETE_IF_EMPTY);
                append_namespace_id(&mut output, self.namespace_id)?;
                append_revision(&mut output, self.expected_revision)?;
            }
        }
        Ok(output)
    }

    fn validate(&self) -> Result<()> {
        validate_value_length(self.value.len())?;
        let contract = operation_contract(self.opcode);
        match contract.request_kind {
            OperationRequestKind::Empty => {
                if self.has_non_empty_fields() {
                    return Err(invalid_shape(self.opcode, 0, "0"));
                }
            }
            OperationRequestKind::ApplicationValue => {
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
            }
            OperationRequestKind::ScopedItem => {
                validate_namespace_id(self.namespace_id)?;
                if self.item_ids.len() != contract.request_item_count
                    || self.namespace_name.is_some()
                    || self.namespace_policy.is_some()
                    || self.expected_revision.is_some()
                    || self.create_if_missing
                {
                    return Err(invalid_shape(
                        self.opcode,
                        ITEM_ID_BYTES * contract.request_item_count,
                        if contract.request_value_count == 0 {
                            "0"
                        } else {
                            "any"
                        },
                    ));
                }
                if contract.request_value_count == 0 {
                    if self.set_options != SetWireOptions::NONE || !self.value.is_empty() {
                        return Err(invalid_shape(self.opcode, ITEM_ID_BYTES, "0"));
                    }
                } else {
                    self.set_options.flags()?;
                }
            }
            OperationRequestKind::ScopedNamespace => {
                validate_namespace_id(self.namespace_id)?;
                if self.has_non_empty_fields_except_namespace() {
                    return Err(invalid_shape(self.opcode, 0, "0"));
                }
            }
            OperationRequestKind::NamespaceOpen => {
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
                if self.namespace_id.is_some()
                    || !self.item_ids.is_empty()
                    || self.set_options != SetWireOptions::NONE
                    || !self.value.is_empty()
                    || self.expected_revision.is_some()
                {
                    return Err(invalid_shape(self.opcode, 0, "0"));
                }
                if let Some(policy) = self.namespace_policy {
                    policy.encode()?;
                }
            }
            OperationRequestKind::NamespaceUpdatePolicy => {
                validate_namespace_id(self.namespace_id)?;
                validate_revision(self.expected_revision)?;
                self.namespace_policy
                    .ok_or(ProtocolError::MissingNamespacePolicy)?
                    .encode()?;
                if !self.item_ids.is_empty()
                    || self.set_options != SetWireOptions::NONE
                    || !self.value.is_empty()
                    || self.namespace_name.is_some()
                    || self.create_if_missing
                {
                    return Err(invalid_shape(self.opcode, 0, "0"));
                }
            }
            OperationRequestKind::NamespaceDelete => {
                validate_namespace_id(self.namespace_id)?;
                validate_revision(self.expected_revision)?;
                if self.has_non_empty_fields_except_namespace_revision() {
                    return Err(invalid_shape(self.opcode, 0, "0"));
                }
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
    Ok(Some((
        decode_namespace_policy_parts(flags, ttl_ms)?,
        encoded_len,
    )))
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

fn append_varuint(output: &mut Vec<u8>, value: u64) {
    let (encoded, length) = encode_varuint(value);
    output.extend_from_slice(&encoded[..length]);
}

fn append_namespace_id(output: &mut Vec<u8>, namespace_id: Option<u64>) -> Result<()> {
    output.extend_from_slice(&validate_namespace_id(namespace_id)?.to_be_bytes());
    Ok(())
}

fn append_revision(output: &mut Vec<u8>, revision: Option<u64>) -> Result<()> {
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

fn validate_value_length(value_len: usize) -> Result<()> {
    if value_len > MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: value_len,
            maximum: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

/// Decodes an ordered sequence of independently optional opaque values.
///
/// This codec belongs to the client semantic adapter because it is used by
/// the modeled GET2 response, not by the shared wire-framing crate.
pub(crate) fn decode_optional_values(
    payload: &[u8],
    value_count: usize,
) -> Result<Vec<Option<Vec<u8>>>> {
    const LENGTH_BYTES: usize = std::mem::size_of::<u32>();
    const MISSING: u32 = u32::MAX;
    validate_value_length(payload.len())?;
    let mut cursor = 0usize;
    let mut values = Vec::with_capacity(value_count);
    for _ in 0..value_count {
        let end = cursor
            .checked_add(LENGTH_BYTES)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        let length_bytes: [u8; LENGTH_BYTES] = payload
            .get(cursor..end)
            .ok_or(ProtocolError::InvalidOptionalValues(
                "optional-value payload is missing an entry length",
            ))?
            .try_into()
            .expect("optional-value length has a fixed width");
        cursor = end;
        let length = u32::from_be_bytes(length_bytes);
        if length == MISSING {
            values.push(None);
            continue;
        }
        let length = usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
        let end = cursor
            .checked_add(length)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        let bytes = payload
            .get(cursor..end)
            .ok_or(ProtocolError::InvalidOptionalValues(
                "optional-value payload entry is truncated",
            ))?;
        values.push(Some(bytes.to_vec()));
        cursor = end;
    }
    if cursor != payload.len() {
        return Err(ProtocolError::InvalidOptionalValues(
            "optional-value payload contains trailing bytes",
        ));
    }
    Ok(values)
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
