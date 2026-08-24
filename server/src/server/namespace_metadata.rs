//! Durable namespace snapshot schema.
//!
//! This codec is intentionally independent of every network wire profile.
//! The v1 snapshot owns an explicit compact policy representation and fixed
//! StorageKey membership so future wire-profile changes cannot alter recovery
//! semantics.

use std::io::{self, ErrorKind};

use crate::protocol::{EvictionDefault, ExpirationDefault, NamespacePolicy, OverridePolicy};

pub(crate) const MAGIC: &[u8; 8] = b"OKNSPACE";
pub(crate) const VERSION: u32 = 1;
pub(crate) const NAME_MAX_BYTES: usize = u8::MAX as usize;

const POLICY_FIXED_TTL: u8 = 0x01;
const POLICY_EXPIRATION_OVERRIDE: u8 = 0x02;
const POLICY_EVICTION_PROTECTED: u8 = 0x04;
const POLICY_EVICTION_OVERRIDE: u8 = 0x08;
const POLICY_RESERVED_MASK: u8 = 0xf0;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message.into())
}

pub(crate) fn encode_policy(policy: NamespacePolicy) -> io::Result<Vec<u8>> {
    let mut flags = 0;
    let ttl_ms = match policy.default_expiration {
        ExpirationDefault::NoExpiry => None,
        ExpirationDefault::FixedTtl { ttl_ms } if ttl_ms != 0 => {
            flags |= POLICY_FIXED_TTL;
            Some(ttl_ms)
        }
        ExpirationDefault::FixedTtl { .. } => {
            return Err(invalid("fixed namespace TTL must be positive"));
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

    let mut encoded = Vec::with_capacity(1 + openkache_protocol::MAX_VARUINT_BYTES);
    encoded.push(flags);
    if let Some(ttl_ms) = ttl_ms {
        let (encoded_ttl, encoded_ttl_len) = openkache_protocol::encode_vu128(ttl_ms);
        encoded.extend_from_slice(&encoded_ttl[..encoded_ttl_len]);
    }
    Ok(encoded)
}

pub(crate) fn decode_policy(
    metadata_version: u32,
    input: &[u8],
) -> io::Result<Option<(NamespacePolicy, usize)>> {
    if metadata_version != VERSION {
        return Err(invalid("namespace metadata version is unsupported"));
    }
    decode_current_policy(input)
}

fn decode_current_policy(input: &[u8]) -> io::Result<Option<(NamespacePolicy, usize)>> {
    let Some(&flags) = input.first() else {
        return Ok(None);
    };
    if flags & POLICY_RESERVED_MASK != 0 {
        return Err(invalid("namespace metadata policy contains reserved bits"));
    }
    let (default_expiration, used) = if flags & POLICY_FIXED_TTL == 0 {
        (ExpirationDefault::NoExpiry, 1)
    } else {
        let Some((ttl_ms, encoded_len)) =
            openkache_protocol::decode_vu128(&input[1..], "namespace metadata TTL")
                .map_err(|error| invalid(error.to_string()))?
        else {
            return Ok(None);
        };
        if ttl_ms == 0 {
            return Err(invalid("fixed namespace TTL must be positive"));
        }
        (ExpirationDefault::FixedTtl { ttl_ms }, 1 + encoded_len)
    };
    Ok(Some((policy_from_flags(flags, default_expiration), used)))
}

fn policy_from_flags(flags: u8, default_expiration: ExpirationDefault) -> NamespacePolicy {
    NamespacePolicy {
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
    }
}
