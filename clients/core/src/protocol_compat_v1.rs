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
use openkache_protocol::{ITEM_ID_BYTES, ItemId};

use super::{
    Opcode, ProtocolError, Request, Result, SetWireOptions, invalid_shape, validate_value_length,
};

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
            SET_CONDITION_ANY_BITS => super::SetCondition::Any,
            SET_IF_ABSENT_BITS => super::SetCondition::IfAbsent,
            SET_IF_PRESENT_BITS => super::SetCondition::IfPresent,
            _ => return Err(ProtocolError::ConflictingSetConditions),
        };
        let expiration_mode = match flags & SET_EXPIRATION_MASK {
            SET_INHERIT_EXPIRATION_BITS => {
                if ttl_ms.is_some() {
                    return Err(ProtocolError::UnexpectedSetTtl);
                }
                super::ExpirationMode::Inherit
            }
            SET_NO_EXPIRY_BITS => {
                if ttl_ms.is_some() {
                    return Err(ProtocolError::UnexpectedSetTtl);
                }
                super::ExpirationMode::NoExpiry
            }
            SET_EXPLICIT_TTL_BITS => {
                let ttl_ms = ttl_ms.ok_or(ProtocolError::MissingSetTtl)?;
                if ttl_ms == 0 {
                    return Err(ProtocolError::InvalidSetTtl);
                }
                super::ExpirationMode::ExplicitTtl
            }
            _ => {
                return Err(ProtocolError::InvalidSetOptions {
                    opcode: Opcode::Set,
                });
            }
        };
        let eviction_mode = match flags & SET_EVICTION_MASK {
            SET_INHERIT_EVICTION_BITS => super::EvictionMode::Inherit,
            SET_EVICTABLE_BITS => super::EvictionMode::Evictable,
            SET_EVICTION_PROTECTED_BITS => super::EvictionMode::EvictionProtected,
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

impl super::NamespacePolicy {
    /// Decodes the historical namespace policy flags and optional TTL.
    #[allow(dead_code)]
    pub(crate) fn from_wire_parts(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
        decode_namespace_policy_parts(flags, ttl_ms)
    }
}

pub(crate) fn decode_namespace_policy(
    input: &[u8],
) -> Result<Option<(super::NamespacePolicy, usize)>> {
    let Some(&flags) = input.first() else {
        return Ok(None);
    };
    let (ttl_ms, encoded_len) = match flags & POLICY_DEFAULT_EXPIRATION_MASK {
        POLICY_NO_EXPIRY => (None, POLICY_FLAGS_BYTES),
        POLICY_FIXED_TTL => {
            let Some((ttl_ms, length)) =
                super::decode_varuint(&input[POLICY_FLAGS_BYTES..], "namespace default TTL")?
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

fn decode_namespace_policy_parts(flags: u8, ttl_ms: Option<u64>) -> Result<super::NamespacePolicy> {
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
            super::ExpirationDefault::NoExpiry
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
            super::ExpirationDefault::FixedTtl { ttl_ms }
        }
        _ => {
            return Err(ProtocolError::InvalidNamespacePolicy(
                "namespace default expiration is reserved",
            ));
        }
    };
    Ok(super::NamespacePolicy {
        default_expiration,
        expiration_override: if flags & POLICY_EXPIRATION_OVERRIDE != 0 {
            super::OverridePolicy::Allowed
        } else {
            super::OverridePolicy::Disallowed
        },
        default_eviction: if flags & POLICY_EVICTION_PROTECTED != 0 {
            super::EvictionDefault::EvictionProtected
        } else {
            super::EvictionDefault::Evictable
        },
        eviction_override: if flags & POLICY_EVICTION_OVERRIDE != 0 {
            super::OverridePolicy::Allowed
        } else {
            super::OverridePolicy::Disallowed
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

/// Returns whether generic ABI entry points must reject this opcode.
pub(crate) fn is_compatibility_operation(operation: Opcode) -> bool {
    openkache_protocol::compat_v1::request_projection(operation).is_some()
}

/// Encodes the historical prefix for a compatibility request.
///
/// `None` means the request is generic and should use the plan-driven prefix
/// in `protocol.rs`.
pub(crate) fn encode_prefix(request: &Request) -> Result<Option<Vec<u8>>> {
    let Some(plan) = openkache_protocol::operation::request_wire_plan(request.opcode) else {
        return Ok(None);
    };
    let values = request_wire_values(request);
    let borrowed: Vec<Option<&[u8]>> = values.iter().map(|field| field.as_deref()).collect();
    openkache_protocol::encode_request_wire_prefix(request.opcode, &borrowed, plan)
        .map(Some)
        .map_err(Into::into)
}

fn request_wire_values(request: &Request) -> Vec<Option<Cow<'_, [u8]>>> {
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
                        super::ExpirationDefault::FixedTtl { ttl_ms } => {
                            Some(Cow::Owned(ttl_ms.to_be_bytes().to_vec()))
                        }
                        super::ExpirationDefault::NoExpiry => None,
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

const fn set_condition_value(value: super::SetCondition) -> &'static [u8] {
    match value {
        super::SetCondition::Any => b"any",
        super::SetCondition::IfAbsent => b"if_absent",
        super::SetCondition::IfPresent => b"if_present",
    }
}

const fn boolean_value(value: bool) -> &'static [u8] {
    if value { b"\x01" } else { b"\x00" }
}

const fn expiration_mode_value(value: super::ExpirationMode) -> &'static [u8] {
    match value {
        super::ExpirationMode::Inherit => b"inherit",
        super::ExpirationMode::NoExpiry => b"no_expiry",
        super::ExpirationMode::ExplicitTtl => b"explicit_ttl",
    }
}

const fn eviction_mode_value(value: super::EvictionMode) -> &'static [u8] {
    match value {
        super::EvictionMode::Inherit => b"inherit",
        super::EvictionMode::Evictable => b"evictable",
        super::EvictionMode::EvictionProtected => b"eviction_protected",
    }
}

const fn override_policy_value(value: super::OverridePolicy) -> &'static [u8] {
    match value {
        super::OverridePolicy::Allowed => b"allowed",
        super::OverridePolicy::Disallowed => b"disallowed",
    }
}

const fn default_expiration_value(value: super::ExpirationDefault) -> &'static [u8] {
    match value {
        super::ExpirationDefault::NoExpiry => b"no_expiry",
        super::ExpirationDefault::FixedTtl { .. } => b"fixed_ttl",
    }
}

const fn default_eviction_value(value: super::EvictionDefault) -> &'static [u8] {
    match value {
        super::EvictionDefault::Evictable => b"evictable",
        super::EvictionDefault::EvictionProtected => b"eviction_protected",
    }
}

/// Validates a compatibility request. `false` means generic validation should
/// run in the plan-driven request core.
pub(crate) fn validate_request(request: &Request) -> Result<bool> {
    if !is_compatibility_operation(request.opcode) {
        return Ok(false);
    }
    validate_compact_request(request)?;
    Ok(true)
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
    Request::new_scoped_items_with_options(
        operation,
        namespace_id,
        items,
        set_options.into_protocol()?,
        value,
    )
    .map_err(crate::Error::protocol)
}

fn validate_compact_request(request: &Request) -> Result<()> {
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
