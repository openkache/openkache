//! Draft-v1 compact policy flag codecs.
//!
//! Namespace and SET policies are semantic values exposed by the public
//! protocol facade, but their compact bit layout belongs to the v1 adapter.
//! Keeping these codecs in their own module prevents route framing and policy
//! semantics from growing into one compatibility file while preserving one
//! adapter-owned implementation.

use super::super::operation_compatibility_contract as contract;
use super::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, NamespacePolicy, Opcode,
    OverridePolicy, ProtocolError, Result, SetCondition, SetOptions, decode_varuint,
    encode_varuint,
};

use contract::{
    POLICY_DEFAULT_EXPIRATION_MASK, POLICY_EVICTION_OVERRIDE, POLICY_EVICTION_PROTECTED,
    POLICY_EXPIRATION_OVERRIDE, POLICY_FIXED_TTL, POLICY_FLAGS_BYTES, POLICY_NO_EXPIRY,
    POLICY_RESERVED_MASK, SET_CONDITION_ANY_BITS, SET_CONDITION_MASK, SET_EVICTABLE_BITS,
    SET_EVICTION_MASK, SET_EVICTION_PROTECTED_BITS, SET_EXPIRATION_MASK, SET_EXPLICIT_TTL_BITS,
    SET_IF_ABSENT_BITS, SET_IF_PRESENT_BITS, SET_INHERIT_EVICTION_BITS,
    SET_INHERIT_EXPIRATION_BITS, SET_NO_EXPIRY_BITS, SET_RESERVED_MASK,
};

impl SetOptions {
    /// Decodes the historical SET flags and optional TTL.
    pub(crate) fn decode_set_options(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
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
    /// Encodes the historical namespace policy payload.
    pub fn encode(self) -> Result<Vec<u8>> {
        let mut output = Vec::with_capacity(super::policy::MAX_POLICY_BYTES);
        self.encode_into(|bytes| {
            output.extend_from_slice(bytes);
            Ok(())
        })?;
        Ok(output)
    }

    pub(crate) fn encode_into(self, mut append: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
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
        append(&[flags])?;
        if let ExpirationDefault::FixedTtl { ttl_ms } = self.default_expiration {
            let (encoded, length) = encode_varuint(ttl_ms);
            append(&encoded[..length])?;
        }
        Ok(())
    }

    /// Decodes one complete policy from the beginning of `input`.
    pub fn decode(input: &[u8]) -> Result<Option<(Self, usize)>> {
        decode_namespace_policy(input)
    }

    /// Decodes policy flags and an optional fixed default TTL.
    pub(crate) fn decode_wire_parts(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
        decode_namespace_policy_parts(flags, ttl_ms)
    }
}

pub(crate) fn decode_namespace_policy(input: &[u8]) -> Result<Option<(NamespacePolicy, usize)>> {
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
