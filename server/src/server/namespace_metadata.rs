//! Durable namespace snapshot schema.
//!
//! This codec is intentionally independent of every network wire profile.
//! Versions 1 and 2 used the then-current compact policy bytes; version 3
//! added the explicit compact policy representation but still stored fixed
//! width Item IDs. Their decoders are retained here as durable storage
//! contracts. Version 4 owns the explicit policy representation and the
//! variable-length Item ID list used by the current wire contract.

use std::io::{self, ErrorKind};

use crate::protocol::{EvictionDefault, ExpirationDefault, NamespacePolicy, OverridePolicy};

pub(crate) const MAGIC: &[u8; 8] = b"OKNSPACE";
pub(crate) const VERSION: u32 = 4;
pub(crate) const LEGACY_V1_VERSION: u32 = 1;
pub(crate) const LEGACY_V2_VERSION: u32 = 2;
pub(crate) const LEGACY_V3_VERSION: u32 = 3;
pub(crate) const NAME_MAX_BYTES: usize = u8::MAX as usize;

const POLICY_FIXED_TTL: u8 = 0x01;
const POLICY_EXPIRATION_OVERRIDE: u8 = 0x02;
const POLICY_EVICTION_PROTECTED: u8 = 0x04;
const POLICY_EVICTION_OVERRIDE: u8 = 0x08;
const POLICY_RESERVED_MASK: u8 = 0xf0;

const LEGACY_POLICY_DEFAULT_EXPIRATION_MASK: u8 = 0x03;
const LEGACY_POLICY_NO_EXPIRY: u8 = 0x00;
const LEGACY_POLICY_FIXED_TTL: u8 = 0x01;
const LEGACY_POLICY_EXPIRATION_OVERRIDE: u8 = 0x04;
const LEGACY_POLICY_EVICTION_PROTECTED: u8 = 0x08;
const LEGACY_POLICY_EVICTION_OVERRIDE: u8 = 0x10;
const LEGACY_POLICY_RESERVED_MASK: u8 = 0xe0;

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
    match metadata_version {
        VERSION | LEGACY_V3_VERSION => decode_policy_v3(input),
        LEGACY_V1_VERSION | LEGACY_V2_VERSION => decode_legacy_policy(input),
        _ => Err(invalid("namespace metadata version is unsupported")),
    }
}

fn decode_policy_v3(input: &[u8]) -> io::Result<Option<(NamespacePolicy, usize)>> {
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
    Ok(Some((
        policy_from_flags(flags, default_expiration, false),
        used,
    )))
}

fn decode_legacy_policy(input: &[u8]) -> io::Result<Option<(NamespacePolicy, usize)>> {
    let Some(&flags) = input.first() else {
        return Ok(None);
    };
    if flags & LEGACY_POLICY_RESERVED_MASK != 0 {
        return Err(invalid("namespace metadata policy contains reserved bits"));
    }
    let (default_expiration, used) = match flags & LEGACY_POLICY_DEFAULT_EXPIRATION_MASK {
        LEGACY_POLICY_NO_EXPIRY => (ExpirationDefault::NoExpiry, 1),
        LEGACY_POLICY_FIXED_TTL => {
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
        }
        _ => return Err(invalid("namespace metadata expiration mode is reserved")),
    };
    Ok(Some((
        policy_from_flags(flags, default_expiration, true),
        used,
    )))
}

fn policy_from_flags(
    flags: u8,
    default_expiration: ExpirationDefault,
    legacy: bool,
) -> NamespacePolicy {
    let (expiration_override, eviction_protected, eviction_override) = if legacy {
        (
            LEGACY_POLICY_EXPIRATION_OVERRIDE,
            LEGACY_POLICY_EVICTION_PROTECTED,
            LEGACY_POLICY_EVICTION_OVERRIDE,
        )
    } else {
        (
            POLICY_EXPIRATION_OVERRIDE,
            POLICY_EVICTION_PROTECTED,
            POLICY_EVICTION_OVERRIDE,
        )
    };
    NamespacePolicy {
        default_expiration,
        expiration_override: if flags & expiration_override != 0 {
            OverridePolicy::Allowed
        } else {
            OverridePolicy::Disallowed
        },
        default_eviction: if flags & eviction_protected != 0 {
            EvictionDefault::EvictionProtected
        } else {
            EvictionDefault::Evictable
        },
        eviction_override: if flags & eviction_override != 0 {
            OverridePolicy::Allowed
        } else {
            OverridePolicy::Disallowed
        },
    }
}
