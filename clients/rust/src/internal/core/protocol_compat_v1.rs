//! Compact draft-v1 response and public policy projection owned by the Rust client.

use crate::internal_protocol::compat_v1::{
    POLICY_DEFAULT_EXPIRATION_MASK, POLICY_EVICTION_OVERRIDE, POLICY_EVICTION_PROTECTED,
    POLICY_EXPIRATION_OVERRIDE, POLICY_FIXED_TTL, POLICY_FLAGS_BYTES, POLICY_NO_EXPIRY,
    POLICY_RESERVED_MASK,
};
#[cfg(feature = "ffi")]
use crate::internal_protocol::compat_v1::{
    SET_CONDITION_ANY_BITS, SET_CONDITION_MASK, SET_EVICTABLE_BITS, SET_EVICTION_MASK,
    SET_EVICTION_PROTECTED_BITS, SET_EXPIRATION_MASK, SET_EXPLICIT_TTL_BITS, SET_IF_ABSENT_BITS,
    SET_IF_PRESENT_BITS, SET_INHERIT_EVICTION_BITS, SET_INHERIT_EXPIRATION_BITS,
    SET_NO_EXPIRY_BITS, SET_RESERVED_MASK,
};

use super::{
    EvictionDefault, ExpirationDefault, NamespacePolicy, OverridePolicy, ProtocolError, Result,
};
#[cfg(feature = "ffi")]
use super::{EvictionMode, ExpirationMode, SetCondition, SetWireOptions};

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
    let mut output =
        Vec::with_capacity(POLICY_FLAGS_BYTES + crate::internal_protocol::MAX_VARUINT_BYTES);
    output.push(flags);
    if let ExpirationDefault::FixedTtl { ttl_ms } = policy.default_expiration {
        let (encoded, encoded_len) = crate::internal_protocol::encode_varuint(ttl_ms);
        output.extend_from_slice(&encoded[..encoded_len]);
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
            let Some((ttl_ms, length)) = crate::internal_protocol::decode_varuint(
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
