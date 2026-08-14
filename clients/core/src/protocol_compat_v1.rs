//! Compact draft-v1 projection owned by the Rust client.

use openkache_protocol::ITEM_ID_BYTES;
use openkache_protocol::compat_v1::{
    DELETE_IF_EMPTY, NAMESPACE_NAME_MAX_BYTES, OPEN_CREATE_IF_MISSING, OperationCompactV1Route,
    OperationFieldDirection, OperationFieldRole, POLICY_DEFAULT_EXPIRATION_MASK,
    POLICY_EVICTION_OVERRIDE, POLICY_EVICTION_PROTECTED, POLICY_EXPIRATION_OVERRIDE,
    POLICY_FIXED_TTL, POLICY_FLAGS_BYTES, POLICY_NO_EXPIRY, POLICY_RESERVED_MASK,
    SET_CONDITION_ANY_BITS, SET_EVICTABLE_BITS, SET_EVICTION_PROTECTED_BITS, SET_EXPLICIT_TTL_BITS,
    SET_IF_ABSENT_BITS, SET_IF_PRESENT_BITS, SET_INHERIT_EVICTION_BITS,
    SET_INHERIT_EXPIRATION_BITS, SET_NO_EXPIRY_BITS, operation_field_count, route_for_opcode,
};
#[cfg(feature = "ffi")]
use openkache_protocol::compat_v1::{
    SET_CONDITION_MASK, SET_EVICTION_MASK, SET_EXPIRATION_MASK, SET_RESERVED_MASK,
};

use super::{
    DraftV1Request, EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode,
    NamespacePolicy, OverridePolicy, ProtocolError, Result, SetCondition, SetWireOptions,
    append_varuint, invalid_shape, validate_operation_field, validate_value_length,
};

#[cfg(feature = "ffi")]
pub(super) fn decode_set_options(flags: u8, ttl_ms: Option<u64>) -> Result<SetWireOptions> {
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
        SET_INHERIT_EXPIRATION_BITS if ttl_ms.is_none() => ExpirationMode::Inherit,
        SET_NO_EXPIRY_BITS if ttl_ms.is_none() => ExpirationMode::NoExpiry,
        SET_EXPLICIT_TTL_BITS => {
            if ttl_ms.ok_or(ProtocolError::MissingSetTtl)? == 0 {
                return Err(ProtocolError::InvalidSetTtl);
            }
            ExpirationMode::ExplicitTtl
        }
        SET_INHERIT_EXPIRATION_BITS | SET_NO_EXPIRY_BITS => {
            return Err(ProtocolError::UnexpectedSetTtl);
        }
        _ => return Err(ProtocolError::InvalidSetOptions),
    };
    let eviction_mode = match flags & SET_EVICTION_MASK {
        SET_INHERIT_EVICTION_BITS => EvictionMode::Inherit,
        SET_EVICTABLE_BITS => EvictionMode::Evictable,
        SET_EVICTION_PROTECTED_BITS => EvictionMode::EvictionProtected,
        _ => return Err(ProtocolError::InvalidSetOptions),
    };
    Ok(SetWireOptions {
        condition,
        expiration_mode,
        ttl_ms,
        eviction_mode,
    })
}

pub(super) fn encode_namespace_policy(policy: NamespacePolicy) -> Result<Vec<u8>> {
    let mut flags = match policy.default_expiration {
        ExpirationDefault::NoExpiry => POLICY_NO_EXPIRY,
        ExpirationDefault::FixedTtl { ttl_ms } if ttl_ms > 0 => POLICY_FIXED_TTL,
        ExpirationDefault::FixedTtl { .. } => {
            return Err(ProtocolError::InvalidNamespacePolicy(
                "fixed namespace TTL must be positive",
            ));
        }
    };
    if policy.expiration_override == OverridePolicy::Allowed {
        flags |= POLICY_EXPIRATION_OVERRIDE;
    }
    if policy.default_eviction == EvictionDefault::EvictionProtected {
        flags |= POLICY_EVICTION_PROTECTED;
    }
    if policy.eviction_override == OverridePolicy::Allowed {
        flags |= POLICY_EVICTION_OVERRIDE;
    }
    let mut output = Vec::with_capacity(POLICY_FLAGS_BYTES + openkache_protocol::MAX_VARUINT_BYTES);
    output.push(flags);
    if let ExpirationDefault::FixedTtl { ttl_ms } = policy.default_expiration {
        append_varuint(&mut output, ttl_ms);
    }
    Ok(output)
}

pub(super) fn decode_namespace_policy(input: &[u8]) -> Result<Option<(NamespacePolicy, usize)>> {
    let Some(&flags) = input.first() else {
        return Ok(None);
    };
    let (ttl_ms, encoded_len) = match flags & POLICY_DEFAULT_EXPIRATION_MASK {
        POLICY_NO_EXPIRY => (None, POLICY_FLAGS_BYTES),
        POLICY_FIXED_TTL => {
            let Some((ttl_ms, length)) = openkache_protocol::decode_varuint(
                &input[POLICY_FLAGS_BYTES..],
                "namespace default TTL",
            )?
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

pub(super) fn decode_namespace_policy_parts(
    flags: u8,
    ttl_ms: Option<u64>,
) -> Result<NamespacePolicy> {
    if flags & POLICY_RESERVED_MASK != 0 {
        return Err(ProtocolError::InvalidNamespacePolicy(
            "namespace policy contains reserved bits",
        ));
    }
    let default_expiration = match flags & POLICY_DEFAULT_EXPIRATION_MASK {
        POLICY_NO_EXPIRY if ttl_ms.is_none() => ExpirationDefault::NoExpiry,
        POLICY_NO_EXPIRY => {
            return Err(ProtocolError::InvalidNamespacePolicy(
                "namespace default TTL requires fixed TTL mode",
            ));
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

pub(super) fn encode_prefix(request: &DraftV1Request) -> Result<Option<Vec<u8>>> {
    let Some(route) = route_for_opcode(request.opcode) else {
        return Ok(None);
    };
    let mut output = Vec::with_capacity(
        1 + openkache_protocol::NAMESPACE_ID_BYTES
            + ITEM_ID_BYTES
            + 2 * openkache_protocol::MAX_VARUINT_BYTES,
    );
    output.push(request.opcode as u8);
    match route {
        OperationCompactV1Route::Item => {
            append_namespace_id(&mut output, request.namespace_id)?;
            for item_id in &request.item_ids {
                output.extend_from_slice(item_id.as_bytes());
            }
        }
        OperationCompactV1Route::Set => {
            append_namespace_id(&mut output, request.namespace_id)?;
            output.push(set_flags(request.set_options)?);
            let item_id = request
                .item_ids
                .first()
                .ok_or_else(|| invalid_shape(request.opcode, 1, "value"))?;
            output.extend_from_slice(item_id.as_bytes());
            if let Some(ttl_ms) = request.set_options.ttl_ms {
                append_varuint(&mut output, ttl_ms);
            }
            append_varuint(&mut output, request.value.len() as u64);
        }
        OperationCompactV1Route::Namespace => {
            append_namespace_id(&mut output, request.namespace_id)?;
        }
        OperationCompactV1Route::NamespaceOpen => {
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
        OperationCompactV1Route::NamespaceUpdatePolicy => {
            append_namespace_id(&mut output, request.namespace_id)?;
            append_revision(&mut output, request.expected_revision)?;
            output.extend_from_slice(
                &request
                    .namespace_policy
                    .ok_or(ProtocolError::MissingNamespacePolicy)?
                    .encode()?,
            );
        }
        OperationCompactV1Route::NamespaceDelete => {
            output.push(DELETE_IF_EMPTY);
            append_namespace_id(&mut output, request.namespace_id)?;
            append_revision(&mut output, request.expected_revision)?;
        }
    }
    Ok(Some(output))
}

pub(super) fn validate_request(request: &DraftV1Request) -> Result<bool> {
    let Some(route) = route_for_opcode(request.opcode) else {
        return Ok(false);
    };
    validate_value_length(request.value.len())?;
    match route {
        OperationCompactV1Route::Item | OperationCompactV1Route::Set => {
            validate_namespace_id(request.namespace_id)?;
            let item_count = operation_field_count(
                request.opcode,
                OperationFieldDirection::Request,
                OperationFieldRole::ItemId,
            );
            let value_count = operation_field_count(
                request.opcode,
                OperationFieldDirection::Request,
                OperationFieldRole::Value,
            );
            if request.item_ids.len() != item_count
                || request.namespace_name.is_some()
                || request.namespace_policy.is_some()
                || request.expected_revision.is_some()
                || request.create_if_missing
            {
                return Err(invalid_shape(
                    request.opcode,
                    item_count,
                    if value_count == 0 { "empty" } else { "value" },
                ));
            }
            if value_count == 0 {
                if request.set_options != SetWireOptions::NONE || !request.value.is_empty() {
                    return Err(invalid_shape(request.opcode, item_count, "empty"));
                }
            } else {
                set_flags(request.set_options)?;
                if let Some(field) = openkache_protocol::operation_wire_spec(request.opcode)
                    .request
                    .fields
                    .iter()
                    .find(|field| field.role == "value" || field.role == "payload")
                {
                    validate_operation_field(field, &request.value)?;
                }
            }
        }
        OperationCompactV1Route::Namespace => {
            validate_namespace_id(request.namespace_id)?;
            if request.has_non_empty_fields_except_namespace() {
                return Err(invalid_shape(request.opcode, 0, "empty"));
            }
        }
        OperationCompactV1Route::NamespaceOpen => {
            let name =
                request
                    .namespace_name
                    .as_deref()
                    .ok_or(ProtocolError::InvalidNamespaceName(
                        "namespace name is missing",
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
                return Err(invalid_shape(request.opcode, 0, "empty"));
            }
            if let Some(policy) = request.namespace_policy {
                policy.encode()?;
            }
        }
        OperationCompactV1Route::NamespaceUpdatePolicy => {
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
                return Err(invalid_shape(request.opcode, 0, "empty"));
            }
        }
        OperationCompactV1Route::NamespaceDelete => {
            validate_namespace_id(request.namespace_id)?;
            validate_revision(request.expected_revision)?;
            if request.has_non_empty_fields_except_namespace_revision() {
                return Err(invalid_shape(request.opcode, 0, "empty"));
            }
        }
    }
    Ok(true)
}

fn set_flags(options: SetWireOptions) -> Result<u8> {
    if options.ttl_ms == Some(0) {
        return Err(ProtocolError::InvalidSetTtl);
    }
    let condition = match options.condition {
        SetCondition::Any => SET_CONDITION_ANY_BITS,
        SetCondition::IfAbsent => SET_IF_ABSENT_BITS,
        SetCondition::IfPresent => SET_IF_PRESENT_BITS,
    };
    let expiration = match options.expiration_mode {
        ExpirationMode::Inherit if options.ttl_ms.is_none() => SET_INHERIT_EXPIRATION_BITS,
        ExpirationMode::NoExpiry if options.ttl_ms.is_none() => SET_NO_EXPIRY_BITS,
        ExpirationMode::ExplicitTtl if options.ttl_ms.is_some() => SET_EXPLICIT_TTL_BITS,
        ExpirationMode::ExplicitTtl => return Err(ProtocolError::MissingSetTtl),
        ExpirationMode::Inherit | ExpirationMode::NoExpiry => {
            return Err(ProtocolError::UnexpectedSetTtl);
        }
    };
    let eviction = match options.eviction_mode {
        EvictionMode::Inherit => SET_INHERIT_EVICTION_BITS,
        EvictionMode::Evictable => SET_EVICTABLE_BITS,
        EvictionMode::EvictionProtected => SET_EVICTION_PROTECTED_BITS,
    };
    Ok(condition | expiration | eviction)
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
        Some(0) | None => Err(ProtocolError::InvalidRevision),
        Some(revision) => Ok(revision),
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
