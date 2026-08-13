//! Client-side protocol-v1 request projection.
//!
//! Generic request construction lives in `protocol.rs`. This module owns the
//! historical namespace/item/SET route vocabulary and the small semantic
//! helpers needed by typed compatibility methods.

use openkache_protocol::compat_v1::OperationFieldDirection;
pub(crate) use openkache_protocol::compat_v1::route_for_opcode;
use openkache_protocol::compat_v1::{
    DELETE_IF_EMPTY, NAMESPACE_NAME_MAX_BYTES, OPEN_CREATE_IF_MISSING, OperationCompactV1Route,
    OperationFieldRole, POLICY_DEFAULT_EXPIRATION_MASK, POLICY_EVICTION_OVERRIDE,
    POLICY_EVICTION_PROTECTED, POLICY_EXPIRATION_OVERRIDE, POLICY_FIXED_TTL, POLICY_FLAGS_BYTES,
    POLICY_NO_EXPIRY, POLICY_RESERVED_MASK, SET_CONDITION_ANY_BITS, SET_CONDITION_MASK,
    SET_EVICTABLE_BITS, SET_EVICTION_MASK, SET_EVICTION_PROTECTED_BITS, SET_EXPIRATION_MASK,
    SET_EXPLICIT_TTL_BITS, SET_IF_ABSENT_BITS, SET_IF_PRESENT_BITS, SET_INHERIT_EVICTION_BITS,
    SET_INHERIT_EXPIRATION_BITS, SET_NO_EXPIRY_BITS, SET_RESERVED_MASK, operation_field_count,
};
use openkache_protocol::{ITEM_ID_BYTES, ItemId};

use super::{
    Opcode, ProtocolError, Request, Result, SetWireOptions, append_varuint, invalid_shape,
    validate_value_length,
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
    route_for_opcode(operation).is_some()
}

/// Encodes the historical prefix for a compatibility request.
///
/// `None` means the request is generic and should use the plan-driven prefix
/// in `protocol.rs`.
pub(crate) fn encode_prefix(request: &Request) -> Result<Option<Vec<u8>>> {
    if route_for_opcode(request.opcode).is_none() {
        return Ok(None);
    }
    let mut output = Vec::new();
    output.push(request.opcode as u8);
    match compact_request_route(request.opcode)? {
        CompactV1RequestRoute::Item | CompactV1RequestRoute::Set => {
            append_namespace_id(&mut output, request.namespace_id)?;
            if compact_request_field_count(request.opcode, OperationFieldRole::Value) > 0 {
                output.push(request.set_options.flags()?);
            }
            for item_id in &request.item_ids {
                output.extend_from_slice(item_id.as_ref());
            }
            if compact_request_field_count(request.opcode, OperationFieldRole::Value) > 0 {
                if let Some(ttl_ms) = request.set_options.ttl_ms {
                    append_varuint(&mut output, ttl_ms);
                }
                append_varuint(&mut output, request.value.len() as u64);
            }
        }
        CompactV1RequestRoute::Namespace => {
            append_namespace_id(&mut output, request.namespace_id)?;
        }
        CompactV1RequestRoute::NamespaceOpen => {
            output.push(if request.create_if_missing {
                OPEN_CREATE_IF_MISSING
            } else {
                0
            });
            let name =
                request
                    .namespace_name
                    .as_deref()
                    .ok_or(ProtocolError::InvalidNamespaceName(
                        "namespace-open name is missing",
                    ))?;
            output.push(u8::try_from(name.len()).map_err(|_| {
                ProtocolError::InvalidNamespaceName("namespace name exceeds 255 octets")
            })?);
            output.extend_from_slice(name);
            if request.create_if_missing {
                output.extend_from_slice(
                    &request
                        .namespace_policy
                        .ok_or(ProtocolError::MissingNamespacePolicy)?
                        .encode()?,
                );
            }
        }
        CompactV1RequestRoute::NamespaceUpdatePolicy => {
            append_namespace_id(&mut output, request.namespace_id)?;
            append_revision(&mut output, request.expected_revision)?;
            output.extend_from_slice(
                &request
                    .namespace_policy
                    .ok_or(ProtocolError::MissingNamespacePolicy)?
                    .encode()?,
            );
        }
        CompactV1RequestRoute::NamespaceDelete => {
            output.push(DELETE_IF_EMPTY);
            append_namespace_id(&mut output, request.namespace_id)?;
            append_revision(&mut output, request.expected_revision)?;
        }
    }
    Ok(Some(output))
}

/// Validates a compatibility request. `false` means generic validation should
/// run in the plan-driven request core.
pub(crate) fn validate_request(request: &Request) -> Result<bool> {
    if route_for_opcode(request.opcode).is_none() {
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
    match compact_request_route(operation).map_err(crate::Error::protocol)? {
        CompactV1RequestRoute::Item
        | CompactV1RequestRoute::Set
        | CompactV1RequestRoute::Namespace => {
            let items = parse_compact_item_ids(
                item_id,
                compact_request_field_count(operation, OperationFieldRole::ItemId),
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
                set_options.into_wire_options()?,
                value,
            )
            .map_err(crate::Error::protocol)
        }
        CompactV1RequestRoute::NamespaceOpen
        | CompactV1RequestRoute::NamespaceUpdatePolicy
        | CompactV1RequestRoute::NamespaceDelete => Err(crate::Error::configuration(
            "operation",
            "namespace-management operations use their typed request builders",
        )),
    }
}

fn validate_compact_request(request: &Request) -> Result<()> {
    validate_value_length(request.value.len())?;
    match compact_request_route(request.opcode)? {
        CompactV1RequestRoute::Item | CompactV1RequestRoute::Set => {
            validate_namespace_id(request.namespace_id)?;
            let item_count =
                compact_request_field_count(request.opcode, OperationFieldRole::ItemId);
            let value_count =
                compact_request_field_count(request.opcode, OperationFieldRole::Value);
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
        }
        CompactV1RequestRoute::Namespace => {
            validate_namespace_id(request.namespace_id)?;
            if request.has_non_empty_fields_except_namespace() {
                return Err(invalid_shape(request.opcode, 0, "0"));
            }
        }
        CompactV1RequestRoute::NamespaceOpen => {
            let name =
                request
                    .namespace_name
                    .as_deref()
                    .ok_or(ProtocolError::InvalidNamespaceName(
                        "namespace name missing",
                    ))?;
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
        }
        CompactV1RequestRoute::NamespaceUpdatePolicy => {
            validate_namespace_id(request.namespace_id)?;
            validate_revision(request.expected_revision)?;
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
        }
        CompactV1RequestRoute::NamespaceDelete => {
            validate_namespace_id(request.namespace_id)?;
            validate_revision(request.expected_revision)?;
            if request.has_non_empty_fields_except_namespace_revision() {
                return Err(invalid_shape(request.opcode, 0, "0"));
            }
        }
    }
    Ok(())
}

fn append_namespace_id(output: &mut Vec<u8>, namespace_id: Option<u64>) -> Result<()> {
    output.extend_from_slice(&validate_namespace_id(namespace_id)?.to_be_bytes());
    Ok(())
}

fn append_revision(output: &mut Vec<u8>, revision: Option<u64>) -> Result<()> {
    output.extend_from_slice(&validate_revision(revision)?.to_be_bytes());
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
pub(super) fn compact_request_field_count(opcode: Opcode, role: OperationFieldRole) -> usize {
    operation_field_count(opcode, OperationFieldDirection::Request, role)
}

/// Returns the number of item identities carried by a compact route.
///
/// This keeps the generated role enum inside the protocol-v1 adapter. Client
/// convenience layers only need the adapter's cardinality decision.
pub(crate) fn compact_item_count(opcode: Opcode) -> usize {
    compact_request_field_count(opcode, OperationFieldRole::ItemId)
}

/// Compact request layouts owned by the protocol-v1 compatibility adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompactV1RequestRoute {
    Item,
    Set,
    Namespace,
    NamespaceOpen,
    NamespaceUpdatePolicy,
    NamespaceDelete,
}

pub(crate) fn compact_request_route(operation: Opcode) -> Result<CompactV1RequestRoute> {
    route_for_opcode(operation)
        .map(|route| match route {
            OperationCompactV1Route::Item => CompactV1RequestRoute::Item,
            OperationCompactV1Route::Set => CompactV1RequestRoute::Set,
            OperationCompactV1Route::Namespace => CompactV1RequestRoute::Namespace,
            OperationCompactV1Route::NamespaceOpen => CompactV1RequestRoute::NamespaceOpen,
            OperationCompactV1Route::NamespaceUpdatePolicy => {
                CompactV1RequestRoute::NamespaceUpdatePolicy
            }
            OperationCompactV1Route::NamespaceDelete => CompactV1RequestRoute::NamespaceDelete,
        })
        .ok_or(ProtocolError::InvalidFieldSequence(
            "operation has no protocol-v1 compact route",
        ))
}

pub(crate) fn uses_compact_item_route(operation: Opcode) -> bool {
    matches!(
        route_for_opcode(operation),
        Some(OperationCompactV1Route::Item | OperationCompactV1Route::Set)
    )
}

/// Returns whether the protocol-v1 adapter must supply a namespace prefix.
pub(crate) fn uses_compact_namespace_route(operation: Opcode) -> bool {
    matches!(
        route_for_opcode(operation),
        Some(
            OperationCompactV1Route::Item
                | OperationCompactV1Route::Set
                | OperationCompactV1Route::Namespace
                | OperationCompactV1Route::NamespaceUpdatePolicy
                | OperationCompactV1Route::NamespaceDelete
        )
    )
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
    if let Some(field) = openkache_protocol::operation_wire_spec(operation)
        .request
        .fields
        .iter()
        .find(|field| field.role == "value" || field.role == "payload")
    {
        super::validate_operation_field(field, payload)?;
    }
    Ok(())
}
