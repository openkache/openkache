//! Client-side protocol-v1 request projection.
//!
//! Generic request construction lives in `protocol.rs`. This module owns the
//! historical namespace/item/SET route vocabulary and the small semantic
//! helpers needed by typed compatibility methods.

use std::borrow::Cow;

use openkache_protocol::compat_v1::{
    NAMESPACE_NAME_MAX_BYTES, POLICY_DEFAULT_EXPIRATION_MASK, POLICY_EVICTION_OVERRIDE,
    POLICY_EVICTION_PROTECTED, POLICY_EXPIRATION_OVERRIDE, POLICY_FIXED_TTL, POLICY_FLAGS_BYTES,
    POLICY_NO_EXPIRY, POLICY_RESERVED_MASK, SET_CONDITION_ANY_BITS, SET_CONDITION_MASK,
    SET_EVICTABLE_BITS, SET_EVICTION_MASK, SET_EVICTION_PROTECTED_BITS, SET_EXPIRATION_MASK,
    SET_EXPLICIT_TTL_BITS, SET_IF_ABSENT_BITS, SET_IF_PRESENT_BITS, SET_INHERIT_EVICTION_BITS,
    SET_INHERIT_EXPIRATION_BITS, SET_NO_EXPIRY_BITS, SET_RESERVED_MASK,
};
use openkache_protocol::{
    ITEM_ID_BYTES, NAMESPACE_ID_BYTES, NAMESPACE_REVISION_BYTES, ItemId, decode_varuint,
    encode_varuint,
};
use smallvec::SmallVec;
use super::{
    Opcode, ProtocolError, Request, RequestRetryPolicy, Result, generic, invalid_shape,
    validate_operation_field, validate_value_length,
};

/// Condition applied atomically by a protocol-v1 `SET` request.
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

/// Item-level expiration selection in the protocol-v1 compatibility API.
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

/// Item-level capacity-eviction selection in the protocol-v1 API.
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

/// Whether a protocol-v1 namespace permits an item to override its default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverridePolicy {
    Allowed,
    Disallowed,
}

/// Namespace expiration default used by the protocol-v1 management adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpirationDefault {
    NoExpiry,
    FixedTtl { ttl_ms: u64 },
}

/// Namespace capacity-eviction default used by the protocol-v1 adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvictionDefault {
    Evictable,
    EvictionProtected,
}

/// Policy applied to newly written items in the protocol-v1 namespace API.
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

/// Namespace identity and policy returned by protocol-v1 management calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NamespaceDescriptor {
    pub namespace_id: u64,
    pub revision: u64,
    pub policy: NamespacePolicy,
}

/// Decodes the historical protocol-v1 optional-value response projection into
/// the same borrowed operation-field view used by generic clients.
///
/// The sentinel table is selected by the generated compatibility artifact.
/// Generic response decoding never sees this branch.
pub(crate) fn decode_response_fields<'a>(
    operation: Opcode,
    payload: &'a [u8],
) -> Result<generic::OperationFields<'a>> {
    if openkache_protocol::compat_v1::response_framing(operation)
        != Some(openkache_protocol::compat_v1::CompatibilityResponseFraming::OptionalValues)
    {
        return Err(ProtocolError::InvalidFieldSequence(
            "operation does not use the protocol-v1 optional-value response projection",
        ));
    }
    let plan = crate::contract::operation_wire_spec(operation).response;
    let mut offsets =
        SmallVec::<[(usize, usize); generic::INLINE_OPERATION_FIELDS]>::with_capacity(
            plan.fields.len(),
        );
    offsets.resize(plan.fields.len(), (usize::MAX, usize::MAX));
    openkache_protocol::compat_v1::OptionalValues::decode(
        payload,
        plan.fields.len(),
        &mut offsets,
    )
    .map_err(ProtocolError::from)?;
    for (index, field) in plan.fields.iter().enumerate() {
        let Some(value) = (offsets[index].0 != usize::MAX)
            .then(|| &payload[offsets[index].0..offsets[index].1])
        else {
            if field.required {
                return Err(ProtocolError::InvalidFieldSequence(
                    "required response field is missing",
                ));
            }
            continue;
        };
        validate_operation_field(field, value)?;
    }
    Ok(generic::OperationFields::from_parts(
        payload,
        offsets,
        plan.fields.len(),
    ))
}

/// Client-side representation of protocol-v1 SET policy bits.
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

    /// Creates options with all protocol-v1 policy selections.
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
}

impl SetWireOptions {
    /// Decodes the historical SET flags and optional TTL.
    ///
    /// This semantic codec belongs to the protocol-v1 adapter; generic
    /// request construction never needs to interpret these bits.
    #[allow(dead_code)]
    pub(crate) fn from_wire_parts(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
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
}

impl NamespacePolicy {
    /// Decodes the historical namespace policy flags and optional TTL.
    #[allow(dead_code)]
    pub(crate) fn from_wire_parts(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
        decode_namespace_policy_parts(flags, ttl_ms)
    }
}

const MAX_POLICY_BYTES: usize =
    POLICY_FLAGS_BYTES + openkache_protocol::MAX_VARUINT_BYTES;

impl NamespaceDescriptor {
    /// Encodes the historical descriptor payload owned by the v1 adapter.
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

    /// Decodes one complete historical descriptor payload.
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
    /// Encodes the historical namespace policy payload owned by the v1 adapter.
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

impl SetWireOptions {
    /// Encodes the historical SET policy bits owned by the v1 adapter.
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

pub(crate) fn decode_namespace_policy(
    input: &[u8],
) -> Result<Option<(NamespacePolicy, usize)>> {
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

/// Builds a typed protocol-v1 request. Callers must first confirm that the
/// opcode belongs to a published compatibility route.
pub(crate) fn request_from_contract(
    operation: Opcode,
    namespace_id: Option<u64>,
    item_id: &[u8],
    value: Vec<u8>,
    set_options: super::super::SetOptions,
) -> crate::Result<Request> {
    compact_request(operation, namespace_id, item_id, value, set_options)
}

/// Builds a scoped protocol-v1 request for the typed convenience client.
pub(crate) fn new_scoped(
    operation: Opcode,
    namespace_id: u64,
    item_id: Option<ItemId>,
    value: Vec<u8>,
) -> Result<Request> {
    scoped_request(
        operation,
        namespace_id,
        item_id.into_iter().collect(),
        SetWireOptions::NONE,
        value,
        super::generated_retry_policy(operation, false),
    )
}

/// Builds a scoped protocol-v1 request with SET policy options.
pub(crate) fn new_scoped_with_options(
    operation: Opcode,
    namespace_id: u64,
    item_id: Option<ItemId>,
    set_options: SetWireOptions,
    value: Vec<u8>,
) -> Result<Request> {
    scoped_request(
        operation,
        namespace_id,
        item_id.into_iter().collect(),
        set_options,
        value,
        super::generated_retry_policy(operation, false),
    )
}

/// Builds a retryable scoped protocol-v1 request.
pub(crate) fn new_scoped_retryable(
    operation: Opcode,
    namespace_id: u64,
    item_id: Option<ItemId>,
    value: Vec<u8>,
) -> Result<Request> {
    new_scoped(operation, namespace_id, item_id, value)
}

fn scoped_request(
    operation: Opcode,
    namespace_id: u64,
    item_ids: Vec<ItemId>,
    set_options: SetWireOptions,
    value: Vec<u8>,
    retry_policy: RequestRetryPolicy,
) -> Result<Request> {
    CompatibilityRequest {
        opcode: operation,
        namespace_id: Some(namespace_id),
        item_ids,
        set_options,
        value,
        namespace_name: None,
        namespace_policy: None,
        expected_revision: None,
        create_if_missing: false,
        retry_policy,
    }
    .into_request()
}

/// Builds the protocol-v1 namespace-open request.
pub(crate) fn namespace_open(
    name: impl AsRef<[u8]>,
    create_if_missing: bool,
    policy: Option<NamespacePolicy>,
) -> Result<Request> {
    CompatibilityRequest {
        opcode: Opcode::NamespaceOpen,
        namespace_id: None,
        item_ids: Vec::new(),
        set_options: SetWireOptions::NONE,
        value: Vec::new(),
        namespace_name: Some(name.as_ref().to_vec()),
        namespace_policy: policy,
        expected_revision: None,
        create_if_missing,
        retry_policy: super::generated_retry_policy(Opcode::NamespaceOpen, create_if_missing),
    }
    .into_request()
}

/// Builds the protocol-v1 namespace policy update request.
pub(crate) fn namespace_update_policy(
    namespace_id: u64,
    expected_revision: u64,
    policy: NamespacePolicy,
) -> Result<Request> {
    CompatibilityRequest {
        opcode: Opcode::NamespaceUpdatePolicy,
        namespace_id: Some(namespace_id),
        item_ids: Vec::new(),
        set_options: SetWireOptions::NONE,
        value: Vec::new(),
        namespace_name: None,
        namespace_policy: Some(policy),
        expected_revision: Some(expected_revision),
        create_if_missing: false,
        retry_policy: super::generated_retry_policy(Opcode::NamespaceUpdatePolicy, false),
    }
    .into_request()
}

/// Builds the protocol-v1 namespace deletion request.
pub(crate) fn namespace_delete(
    namespace_id: u64,
    expected_revision: u64,
) -> Result<Request> {
    CompatibilityRequest {
        opcode: Opcode::NamespaceDelete,
        namespace_id: Some(namespace_id),
        item_ids: Vec::new(),
        set_options: SetWireOptions::NONE,
        value: Vec::new(),
        namespace_name: None,
        namespace_policy: None,
        expected_revision: Some(expected_revision),
        create_if_missing: false,
        retry_policy: super::generated_retry_policy(Opcode::NamespaceDelete, false),
    }
    .into_request()
}

/// Returns whether generic ABI entry points must reject this opcode.
pub(crate) fn is_compatibility_operation(operation: Opcode) -> bool {
    openkache_protocol::compat_v1::request_projection(operation).is_some()
}

/// Semantic request state owned by the protocol-v1 adapter.
///
/// This is deliberately not part of the generic client request envelope. It
/// exists only long enough to validate the historical route and turn its
/// fields into an adapter-owned wire prefix.
#[derive(Debug)]
struct CompatibilityRequest {
    opcode: Opcode,
    namespace_id: Option<u64>,
    item_ids: Vec<ItemId>,
    set_options: SetWireOptions,
    value: Vec<u8>,
    namespace_name: Option<Vec<u8>>,
    namespace_policy: Option<NamespacePolicy>,
    expected_revision: Option<u64>,
    create_if_missing: bool,
    retry_policy: RequestRetryPolicy,
}

impl CompatibilityRequest {
    fn into_request(self) -> Result<Request> {
        validate_compact_request(&self)?;
        let prefix = encode_prefix(&self)?;
        Ok(Request::new_wire(
            self.opcode,
            prefix,
            self.value,
            self.retry_policy,
        ))
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

/// Encodes the historical prefix for a compatibility request.
fn encode_prefix(request: &CompatibilityRequest) -> Result<Vec<u8>> {
    let plan = openkache_protocol::operation::request_wire_plan(request.opcode).ok_or(
        ProtocolError::InvalidFieldSequence("compatibility operation has no request wire plan"),
    )?;
    let values = request_wire_values(request);
    let borrowed: Vec<Option<&[u8]>> = values.iter().map(|field| field.as_deref()).collect();
    openkache_protocol::encode_request_wire_prefix(request.opcode, &borrowed, plan)
        .map_err(Into::into)
}

fn request_wire_values(request: &CompatibilityRequest) -> Vec<Option<Cow<'_, [u8]>>> {
    let plan = crate::contract::operation_wire_spec(request.opcode).request.fields;
    let mut item_ids = request.item_ids.iter();
    plan
        .iter()
        .map(|field| match field.role {
            "namespace_id" => request
                .namespace_id
                .map(|value| Cow::Owned(value.to_be_bytes().to_vec())),
            "item_id" => item_ids
                .next()
                .map(|item_id| Cow::Borrowed(item_id.as_ref())),
            "value" => Some(Cow::Borrowed(request.value.as_slice())),
            "name" => request.namespace_name.as_deref().map(Cow::Borrowed),
            "expected_revision" => request
                .expected_revision
                .map(|value| Cow::Owned(value.to_be_bytes().to_vec())),
            "condition" => Some(Cow::Borrowed(set_condition_value(
                request.set_options.condition,
            ))),
            "expiration_mode" => Some(Cow::Borrowed(expiration_mode_value(
                request.set_options.expiration_mode,
            ))),
            "eviction_mode" => Some(Cow::Borrowed(eviction_mode_value(
                request.set_options.eviction_mode,
            ))),
            "ttl_milliseconds" => request
                .set_options
                .ttl_ms
                .map(|value| Cow::Owned(value.to_be_bytes().to_vec())),
            "create_if_missing" => Some(Cow::Borrowed(boolean_value(
                request.create_if_missing,
            ))),
            "policy" => None,
            "default_expiration" => request.namespace_policy.map(|policy| {
                Cow::Borrowed(default_expiration_value(policy.default_expiration))
            }),
            "default_ttl_milliseconds" => {
                request
                    .namespace_policy
                    .and_then(|policy| match policy.default_expiration {
                        ExpirationDefault::FixedTtl { ttl_ms } => {
                            Some(Cow::Owned(ttl_ms.to_be_bytes().to_vec()))
                        }
                        ExpirationDefault::NoExpiry => None,
                    })
            }
            "expiration_override" => request.namespace_policy.map(|policy| {
                Cow::Borrowed(override_policy_value(policy.expiration_override))
            }),
            "default_eviction" => request.namespace_policy.map(|policy| {
                Cow::Borrowed(default_eviction_value(policy.default_eviction))
            }),
            "eviction_override" => request.namespace_policy.map(|policy| {
                Cow::Borrowed(override_policy_value(policy.eviction_override))
            }),
            _ => None,
        })
        .collect()
}

const fn set_condition_value(value: SetCondition) -> &'static [u8] {
    match value {
        SetCondition::Any => b"any",
        SetCondition::IfAbsent => b"if_absent",
        SetCondition::IfPresent => b"if_present",
    }
}

const fn boolean_value(value: bool) -> &'static [u8] {
    if value { b"\x01" } else { b"\x00" }
}

const fn expiration_mode_value(value: ExpirationMode) -> &'static [u8] {
    match value {
        ExpirationMode::Inherit => b"inherit",
        ExpirationMode::NoExpiry => b"no_expiry",
        ExpirationMode::ExplicitTtl => b"explicit_ttl",
    }
}

const fn eviction_mode_value(value: EvictionMode) -> &'static [u8] {
    match value {
        EvictionMode::Inherit => b"inherit",
        EvictionMode::Evictable => b"evictable",
        EvictionMode::EvictionProtected => b"eviction_protected",
    }
}

const fn override_policy_value(value: OverridePolicy) -> &'static [u8] {
    match value {
        OverridePolicy::Allowed => b"allowed",
        OverridePolicy::Disallowed => b"disallowed",
    }
}

const fn default_expiration_value(value: ExpirationDefault) -> &'static [u8] {
    match value {
        ExpirationDefault::NoExpiry => b"no_expiry",
        ExpirationDefault::FixedTtl { .. } => b"fixed_ttl",
    }
}

const fn default_eviction_value(value: EvictionDefault) -> &'static [u8] {
    match value {
        EvictionDefault::Evictable => b"evictable",
        EvictionDefault::EvictionProtected => b"eviction_protected",
    }
}

fn compact_request(
    operation: Opcode,
    namespace_id: Option<u64>,
    item_id: &[u8],
    value: Vec<u8>,
    set_options: super::super::SetOptions,
) -> crate::Result<Request> {
    if !is_compatibility_operation(operation) {
        return Err(crate::Error::configuration(
            "operation",
            "operation has no exact generated request plan",
        ));
    }
    if compact_request_field_count(operation, "name") > 0
        || compact_request_field_count(operation, "expected_revision") > 0
    {
        return Err(crate::Error::configuration(
            "operation",
            "namespace-management operations use their typed request builders",
        ));
    }
    let items = parse_compact_item_ids(
        item_id,
        compact_request_field_count(operation, "item_id"),
    )
    .map_err(|message| crate::Error::configuration("item_id", message))?;
    let namespace_id = namespace_id.ok_or_else(|| {
        crate::Error::configuration(
            "namespace_id",
            "scoped operation requires a namespace ID",
        )
    })?;
    if !value.is_empty() {
        validate_compact_value(operation, &value).map_err(crate::Error::protocol)?;
    }
    let request = CompatibilityRequest {
        opcode: operation,
        namespace_id: Some(namespace_id),
        item_ids: items,
        set_options: set_options.into_protocol()?,
        value,
        namespace_name: None,
        namespace_policy: None,
        expected_revision: None,
        create_if_missing: false,
        retry_policy: super::generated_retry_policy(operation, false),
    };
    request.into_request().map_err(crate::Error::protocol)
}

fn validate_compact_request(request: &CompatibilityRequest) -> Result<()> {
    validate_value_length(request.value.len())?;
    let item_count = compact_request_field_count(request.opcode, "item_id");
    let value_count = compact_request_field_count(request.opcode, "value");
    let has_name = compact_request_field_count(request.opcode, "name") > 0;
    let has_revision = compact_request_field_count(request.opcode, "expected_revision") > 0;
    let has_policy = compact_request_field_count(request.opcode, "policy") > 0;
    if item_count > 0 {
        validate_namespace_id(request.namespace_id)?;
        if request.item_ids.len() != item_count
            || request.namespace_name.is_some()
            || request.namespace_policy.is_some()
            || request.expected_revision.is_some()
            || request.create_if_missing
        {
            return Err(invalid_shape(
                request.opcode,
                ITEM_ID_BYTES * item_count,
                if value_count == 0 { "0" } else { "any" },
            ));
        }
        if value_count == 0 {
            if request.set_options != SetWireOptions::NONE || !request.value.is_empty() {
                return Err(invalid_shape(request.opcode, ITEM_ID_BYTES, "0"));
            }
        } else {
            request.set_options.flags()?;
        }
        return Ok(());
    }
    if has_name {
        let name = request
            .namespace_name
            .as_deref()
            .ok_or(ProtocolError::InvalidNamespaceName("namespace name missing"))?;
        validate_namespace_name(name)?;
        if request.create_if_missing != request.namespace_policy.is_some() {
            return Err(if request.create_if_missing {
                ProtocolError::MissingNamespacePolicy
            } else {
                ProtocolError::UnexpectedNamespacePolicy
            });
        }
        if request.namespace_id.is_some()
            || !request.item_ids.is_empty()
            || request.set_options != SetWireOptions::NONE
            || !request.value.is_empty()
            || request.expected_revision.is_some()
        {
            return Err(invalid_shape(request.opcode, 0, "0"));
        }
        if let Some(policy) = request.namespace_policy {
            policy.encode()?;
        }
        return Ok(());
    }
    if has_revision {
        validate_namespace_id(request.namespace_id)?;
        validate_revision(request.expected_revision)?;
        if has_policy {
            request
                .namespace_policy
                .ok_or(ProtocolError::MissingNamespacePolicy)?
                .encode()?;
            if !request.item_ids.is_empty()
                || request.set_options != SetWireOptions::NONE
                || !request.value.is_empty()
                || request.namespace_name.is_some()
                || request.create_if_missing
            {
                return Err(invalid_shape(request.opcode, 0, "0"));
            }
        } else if request.has_non_empty_fields_except_namespace_revision() {
            return Err(invalid_shape(request.opcode, 0, "0"));
        }
        return Ok(());
    }
    validate_namespace_id(request.namespace_id)?;
    if request.has_non_empty_fields_except_namespace() {
        return Err(invalid_shape(request.opcode, 0, "0"));
    }
    Ok(())
}

fn validate_namespace_id(namespace_id: Option<u64>) -> Result<u64> {
    match namespace_id {
        Some(0) => Err(ProtocolError::InvalidNamespaceId),
        Some(namespace_id) => Ok(namespace_id),
        None => Err(ProtocolError::MissingNamespaceId),
    }
}

fn validate_revision(revision: Option<u64>) -> Result<u64> {
    match revision {
        Some(0) => Err(ProtocolError::InvalidRevision),
        Some(revision) => Ok(revision),
        None => Err(ProtocolError::InvalidRevision),
    }
}

fn validate_namespace_name(name: &[u8]) -> Result<()> {
    if name.len() > NAMESPACE_NAME_MAX_BYTES {
        return Err(ProtocolError::InvalidNamespaceName(
            "namespace name exceeds 255 octets",
        ));
    }
    if std::str::from_utf8(name).is_err() {
        return Err(ProtocolError::InvalidNamespaceName(
            "namespace name is not UTF-8",
        ));
    }
    Ok(())
}

/// Looks up a compact protocol-v1 field cardinality at the adapter boundary.
pub(super) fn compact_request_field_count(opcode: Opcode, role: &str) -> usize {
    crate::contract::operation_wire_spec(opcode)
        .request
        .fields
        .iter()
        .filter(|field| field.role == role)
        .count()
}

/// Returns the number of item identities carried by a compact route.
///
/// This keeps the generated role enum inside the protocol-v1 adapter. Client
/// convenience layers only need the adapter's cardinality decision.
pub(crate) fn compact_item_count(opcode: Opcode) -> usize {
    compact_request_field_count(opcode, "item_id")
}

pub(crate) fn uses_compact_item_route(operation: Opcode) -> bool {
    is_compatibility_operation(operation) && compact_item_count(operation) > 0
}

/// Returns whether the protocol-v1 adapter must supply a namespace prefix.
pub(crate) fn uses_compact_namespace_route(operation: Opcode) -> bool {
    is_compatibility_operation(operation)
        && compact_request_field_count(operation, "namespace_id") > 0
}

pub(crate) fn parse_compact_item_ids(
    bytes: &[u8],
    item_count: usize,
) -> std::result::Result<Vec<ItemId>, String> {
    let expected = item_count
        .checked_mul(ITEM_ID_BYTES)
        .ok_or_else(|| "item ID count overflows the ABI".to_owned())?;
    if bytes.len() != expected {
        return Err(format!(
            "expected {expected} bytes for {item_count} item IDs, got {}",
            bytes.len()
        ));
    }
    (0..item_count)
        .map(|index| {
            let bytes = bytes
                .get(index * ITEM_ID_BYTES..(index + 1) * ITEM_ID_BYTES)
                .expect("item ID range was validated");
            Ok(ItemId::new(
                bytes.try_into().expect("item ID width was validated"),
            ))
        })
        .collect()
}

/// Validates the payload field carried by a compact protocol-v1 request.
pub(crate) fn validate_compact_value(operation: Opcode, payload: &[u8]) -> Result<()> {
    if let Some(field) = crate::contract::operation_wire_spec(operation)
        .request
        .fields
        .iter()
        .find(|field| field.role == "value" || field.role == "payload")
    {
        super::validate_operation_field(field, payload)?;
    }
    Ok(())
}
