//! Client-owned domain values and request projection.
//!
//! The neutral protocol crate owns operation identifiers, frame plans, and
//! reusable codecs. This module owns Rust client semantics and selects the
//! compact-v1 adapter only for operations that explicitly join that profile.

use openkache_protocol::{
    ItemId, MAX_OPERATION_REQUEST_FIELDS, MAX_VALUE_BYTES, NAMESPACE_ID_BYTES,
    NAMESPACE_REVISION_BYTES, Opcode, OperationFramePolicy, OperationLayoutFraming,
    decode_planned_fields, encode_varuint, operation_wire_spec,
};

#[path = "protocol_compat_v1.rs"]
mod compat_v1;

/// Client-side protocol validation failures.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// A neutral wire primitive rejected the frame or payload.
    #[error(transparent)]
    Wire(#[from] openkache_protocol::ProtocolError),
    /// A request contains reserved compatibility bits.
    #[error("request flags contain unknown bits 0x{0:02x}")]
    UnknownRequestFlags(u8),
    /// SET combines mutually exclusive existence conditions.
    #[error("if-absent and if-present conditions cannot be combined")]
    ConflictingSetConditions,
    /// A generated field codec rejected its value.
    #[error("invalid operation field codec: {0}")]
    InvalidFieldCodec(String),
    /// A request does not match its generated or compatibility shape.
    #[error("{opcode:?} requires request shape ({expected_item_ids} item IDs, {expected_value})")]
    InvalidRequestShape {
        /// Operation whose request was invalid.
        opcode: Opcode,
        /// Required number of item identifiers.
        expected_item_ids: usize,
        /// Required value shape.
        expected_value: &'static str,
    },
    /// SET carries a zero TTL.
    #[error("SET TTL must be greater than zero milliseconds")]
    InvalidSetTtl,
    /// Explicit TTL mode omitted its TTL.
    #[error("SET TTL is required by ExplicitTtl")]
    MissingSetTtl,
    /// A non-TTL expiration mode carried a TTL.
    #[error("SET TTL is not allowed by this expiration mode")]
    UnexpectedSetTtl,
    /// SET flags do not represent one valid option tuple.
    #[error("SET options are invalid")]
    InvalidSetOptions,
    /// A scoped request omitted its namespace ID.
    #[error("namespace ID is missing")]
    MissingNamespaceId,
    /// Namespace IDs must be non-zero.
    #[error("namespace ID must be a positive non-zero u64")]
    InvalidNamespaceId,
    /// A namespace name violates compact-v1 requirements.
    #[error("namespace name is invalid: {0}")]
    InvalidNamespaceName(&'static str),
    /// Namespace creation or update omitted its policy.
    #[error("namespace policy is missing")]
    MissingNamespacePolicy,
    /// A non-creating namespace open carried a policy.
    #[error("namespace policy is not allowed")]
    UnexpectedNamespacePolicy,
    /// Namespace policy bits or TTL are invalid.
    #[error("namespace policy is invalid: {0}")]
    InvalidNamespacePolicy(&'static str),
    /// Namespace revisions must be non-zero.
    #[error("namespace revision must be positive")]
    InvalidRevision,
    /// A generic request uses a response-only or otherwise invalid layout.
    #[error("invalid operation field sequence: {0}")]
    InvalidFieldSequence(&'static str),
}

type Result<T> = std::result::Result<T, ProtocolError>;

/// Condition applied atomically by a SET request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SetCondition {
    /// Store regardless of whether the item exists.
    #[default]
    Any,
    /// Store only when the item does not exist.
    IfAbsent,
    /// Store only when the item already exists.
    IfPresent,
}

/// Item-level expiration selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExpirationMode {
    /// Resolve the namespace default at the SET linearization point.
    #[default]
    Inherit,
    /// Store without a TTL deadline.
    NoExpiry,
    /// Carry a positive TTL in the SET request.
    ExplicitTtl,
}

/// Item-level capacity-eviction selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EvictionMode {
    /// Resolve the namespace default at the SET linearization point.
    #[default]
    Inherit,
    /// Permit selection by the namespace eviction algorithm.
    Evictable,
    /// Exclude the item from capacity eviction.
    EvictionProtected,
}

/// Whether a namespace permits an item-level override.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverridePolicy {
    /// Item-level overrides are accepted.
    Allowed,
    /// Item-level overrides are rejected.
    Disallowed,
}

/// Namespace expiration default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpirationDefault {
    /// Items do not expire by default.
    NoExpiry,
    /// Items inherit one positive fixed TTL.
    FixedTtl {
        /// Default relative TTL in milliseconds.
        ttl_ms: u64,
    },
}

/// Namespace capacity-eviction default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvictionDefault {
    /// Items may be selected for capacity eviction.
    Evictable,
    /// Items are protected from capacity eviction.
    EvictionProtected,
}

/// Policy applied to newly written items in one namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NamespacePolicy {
    /// Default expiration applied at SET time.
    pub default_expiration: ExpirationDefault,
    /// Whether an item may override expiration.
    pub expiration_override: OverridePolicy,
    /// Default capacity-eviction behavior.
    pub default_eviction: EvictionDefault,
    /// Whether an item may override eviction behavior.
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

/// Namespace identity and policy returned by namespace operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NamespaceDescriptor {
    /// Server-assigned namespace identity.
    pub namespace_id: u64,
    /// Monotonic policy revision.
    pub revision: u64,
    /// Current namespace policy.
    pub policy: NamespacePolicy,
}

impl NamespaceDescriptor {
    /// Encodes one compact-v1 namespace descriptor.
    ///
    /// # Returns
    ///
    /// The exact compatibility payload.
    ///
    /// # Errors
    ///
    /// Returns an error for zero identifiers or revisions and invalid policy
    /// values.
    pub fn encode(self) -> Result<Vec<u8>> {
        if self.namespace_id == 0 {
            return Err(ProtocolError::InvalidNamespaceId);
        }
        if self.revision == 0 {
            return Err(ProtocolError::InvalidRevision);
        }
        let policy = self.policy.encode()?;
        let mut payload =
            Vec::with_capacity(NAMESPACE_ID_BYTES + NAMESPACE_REVISION_BYTES + policy.len());
        payload.extend_from_slice(&self.namespace_id.to_be_bytes());
        payload.extend_from_slice(&self.revision.to_be_bytes());
        payload.extend_from_slice(&policy);
        Ok(payload)
    }

    /// Decodes one complete compact-v1 namespace descriptor.
    ///
    /// # Arguments
    ///
    /// * `input` - Exact response payload bytes.
    ///
    /// # Returns
    ///
    /// The validated descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error for truncation, trailing bytes, zero identifiers or
    /// revisions, and invalid policy values.
    pub fn decode(input: &[u8]) -> Result<Self> {
        let fixed = NAMESPACE_ID_BYTES + NAMESPACE_REVISION_BYTES;
        if input.len() < fixed {
            return Err(openkache_protocol::ProtocolError::FrameTooShort {
                expected: fixed,
                actual: input.len(),
            }
            .into());
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
            return Err(openkache_protocol::ProtocolError::FrameLength {
                expected: fixed + policy_len,
                actual: input.len(),
            }
            .into());
        }
        Ok(Self {
            namespace_id,
            revision,
            policy,
        })
    }
}

impl NamespacePolicy {
    /// Encodes the compact-v1 namespace policy.
    ///
    /// # Returns
    ///
    /// The exact compatibility policy bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when a fixed TTL is zero.
    pub fn encode(self) -> Result<Vec<u8>> {
        compat_v1::encode_namespace_policy(self)
    }

    #[cfg(feature = "ffi")]
    pub(crate) fn from_wire_parts(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
        compat_v1::decode_namespace_policy_parts(flags, ttl_ms)
    }
}

/// Client-side representation of compact-v1 SET policy bits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SetWireOptions {
    pub(crate) condition: SetCondition,
    pub(crate) expiration_mode: ExpirationMode,
    pub(crate) ttl_ms: Option<u64>,
    pub(crate) eviction_mode: EvictionMode,
}

impl SetWireOptions {
    pub(crate) const NONE: Self = Self {
        condition: SetCondition::Any,
        expiration_mode: ExpirationMode::Inherit,
        ttl_ms: None,
        eviction_mode: EvictionMode::Inherit,
    };

    pub(crate) const fn with_policies(
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

    #[cfg(feature = "ffi")]
    pub(crate) fn from_wire_parts(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
        compat_v1::decode_set_options(flags, ttl_ms)
    }
}

/// Closed replay decision attached by the client adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestRetryPolicy {
    /// The operation is safe to replay.
    Always,
    /// The operation must not be replayed automatically.
    Never,
}

impl RequestRetryPolicy {
    pub(crate) const fn is_safe(self) -> bool {
        matches!(self, Self::Always)
    }
}

fn generated_retry_policy(opcode: Opcode, create_if_missing: bool) -> RequestRetryPolicy {
    use crate::contract::OperationRetryMode;

    match crate::contract::operation_client_projection(opcode).map(|value| value.retry_mode) {
        Some(OperationRetryMode::Always) => RequestRetryPolicy::Always,
        Some(OperationRetryMode::WhenNotCreating) if !create_if_missing => {
            RequestRetryPolicy::Always
        }
        Some(OperationRetryMode::Never | OperationRetryMode::WhenNotCreating) | None => {
            RequestRetryPolicy::Never
        }
    }
}

/// A validated request owned by the Rust client adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Request {
    pub(crate) opcode: Opcode,
    namespace_id: Option<u64>,
    item_ids: Vec<ItemId>,
    set_options: SetWireOptions,
    value: Vec<u8>,
    namespace_name: Option<Vec<u8>>,
    namespace_policy: Option<NamespacePolicy>,
    expected_revision: Option<u64>,
    pub(crate) create_if_missing: bool,
    pub(crate) retry_policy: RequestRetryPolicy,
}

/// Owned request pieces ready for a transport write.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RequestParts {
    pub(crate) prefix: Vec<u8>,
    pub(crate) payload: Vec<u8>,
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
            retry_policy: generated_retry_policy(opcode, false),
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
            retry_policy: generated_retry_policy(opcode, false),
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
        let prefix = if let Some(prefix) = compat_v1::encode_prefix(&self)? {
            prefix
        } else {
            let mut prefix = Vec::with_capacity(1 + openkache_protocol::MAX_VARUINT_BYTES);
            prefix.push(self.opcode as u8);
            let request = operation_wire_spec(self.opcode).request;
            match request.framing {
                OperationLayoutFraming::Empty => {}
                OperationLayoutFraming::Opaque => {
                    append_varuint(&mut prefix, self.value.len() as u64);
                }
                OperationLayoutFraming::OrderedFields | OperationLayoutFraming::FieldSequence => {
                    if request.frame == OperationFramePolicy::LengthDelimited {
                        append_varuint(&mut prefix, self.value.len() as u64);
                    }
                }
                OperationLayoutFraming::OptionalValues => {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "optional-value framing is response-only",
                    ));
                }
            }
            prefix
        };
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
        if self.namespace_id.is_some()
            || !self.item_ids.is_empty()
            || self.set_options != SetWireOptions::NONE
            || self.namespace_name.is_some()
            || self.namespace_policy.is_some()
            || self.expected_revision.is_some()
            || self.create_if_missing
        {
            return Err(invalid_shape(self.opcode, 0, "generic body"));
        }
        let request = operation_wire_spec(self.opcode).request;
        match request.framing {
            OperationLayoutFraming::Empty if self.value.is_empty() => Ok(()),
            OperationLayoutFraming::Empty => Err(invalid_shape(self.opcode, 0, "empty body")),
            OperationLayoutFraming::Opaque => {
                if let Some(field) = request.fields.first() {
                    validate_operation_field(field, &self.value)?;
                }
                Ok(())
            }
            OperationLayoutFraming::OrderedFields | OperationLayoutFraming::FieldSequence => {
                if request.fields.len() > MAX_OPERATION_REQUEST_FIELDS {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "generated request field bound is stale",
                    ));
                }
                let mut offsets = [(usize::MAX, usize::MAX); MAX_OPERATION_REQUEST_FIELDS];
                decode_planned_fields(
                    &self.value,
                    request.fields,
                    request.layout,
                    &mut offsets[..request.fields.len()],
                )?;
                for (index, field) in request.fields.iter().enumerate() {
                    let (start, end) = offsets[index];
                    if start == usize::MAX {
                        if field.required {
                            return Err(ProtocolError::InvalidFieldSequence(
                                "required request field is missing",
                            ));
                        }
                    } else {
                        validate_operation_field(field, &self.value[start..end])?;
                    }
                }
                Ok(())
            }
            OperationLayoutFraming::OptionalValues => Err(ProtocolError::InvalidFieldSequence(
                "optional-value framing is response-only",
            )),
        }
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

fn validate_operation_field(
    field: &openkache_protocol::OperationFieldPlan,
    payload: &[u8],
) -> Result<()> {
    if field.encoded_width != 0 && payload.len() != field.encoded_width {
        return Err(ProtocolError::InvalidFieldSequence(
            "operation field does not match its declared fixed width",
        ));
    }
    openkache_protocol::codec::validate_field_codecs_with_nested_widths(
        payload,
        field.codecs,
        field.nested_codecs,
        field.nested_widths,
        field.nested_enum_values,
        field.nested_union_tags,
        field.enum_values,
        field.union_tags,
        openkache_protocol::wire_codec_kind,
    )
    .map_err(|error| {
        ProtocolError::InvalidFieldCodec(String::from_utf8_lossy(error.message()).into_owned())
    })
}

fn append_varuint(output: &mut Vec<u8>, value: u64) {
    let (encoded, length) = encode_varuint(value);
    output.extend_from_slice(&encoded[..length]);
}

fn read_u64_be(input: &[u8]) -> Result<u64> {
    let bytes: [u8; NAMESPACE_ID_BYTES] = input
        .get(..NAMESPACE_ID_BYTES)
        .ok_or(openkache_protocol::ProtocolError::FrameTooShort {
            expected: NAMESPACE_ID_BYTES,
            actual: input.len(),
        })?
        .try_into()
        .expect("slice length checked");
    Ok(u64::from_be_bytes(bytes))
}

fn validate_value_length(value_len: usize) -> Result<()> {
    if value_len > MAX_VALUE_BYTES {
        return Err(openkache_protocol::ProtocolError::ValueTooLarge {
            size: value_len,
            maximum: MAX_VALUE_BYTES,
        }
        .into());
    }
    Ok(())
}

fn invalid_shape(
    opcode: Opcode,
    expected_item_ids: usize,
    expected_value: &'static str,
) -> ProtocolError {
    ProtocolError::InvalidRequestShape {
        opcode,
        expected_item_ids,
        expected_value,
    }
}
