//! Public namespace and item policy values used by the v1 compatibility API.
//!
//! The policy types are semantic server values. Their historical flag-byte
//! encoding remains in [`super::compat_v1`], while request framing and generic
//! operation parsing stay in [`super`]'s other protocol modules.

use super::super::operation_compatibility_contract::POLICY_FLAGS_BYTES;

use super::{ProtocolError, Result, compat_v1};
use openkache_protocol::{NAMESPACE_ID_BYTES, NAMESPACE_REVISION_BYTES};

pub(super) const MAX_POLICY_BYTES: usize =
    POLICY_FLAGS_BYTES + openkache_protocol::MAX_VARUINT_BYTES;

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

impl NamespacePolicy {
    /// Decodes protocol-v1 policy flags and an optional fixed default TTL.
    ///
    /// The compatibility adapter owns the historical bit layout; this facade
    /// keeps the public helper small and prevents generic planners from
    /// depending on that layout.
    pub fn from_wire_parts(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
        Self::decode_wire_parts(flags, ttl_ms)
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
    /// Encodes the descriptor payload returned by namespace-management
    /// requests.
    pub fn encode(self) -> Result<Vec<u8>> {
        let mut payload =
            Vec::with_capacity(NAMESPACE_ID_BYTES + NAMESPACE_REVISION_BYTES + MAX_POLICY_BYTES);
        self.encode_into(|bytes| {
            payload.extend_from_slice(bytes);
            Ok(())
        })?;
        Ok(payload)
    }

    pub(crate) fn encode_inline(self) -> Result<openkache_protocol::InlineBytes> {
        let mut payload = openkache_protocol::InlineBytes::new();
        self.encode_into(|bytes| {
            payload.try_extend_from_slice(bytes)?;
            Ok(())
        })?;
        Ok(payload)
    }

    fn encode_into(self, mut append: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
        if self.namespace_id == 0 {
            return Err(ProtocolError::InvalidNamespaceId);
        }
        if self.revision == 0 {
            return Err(ProtocolError::InvalidRevision);
        }
        append(&self.namespace_id.to_be_bytes())?;
        append(&self.revision.to_be_bytes())?;
        self.policy.encode_into(append)?;
        Ok(())
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
        let namespace_id = super::read_u64_be(input)?;
        if namespace_id == 0 {
            return Err(ProtocolError::InvalidNamespaceId);
        }
        let revision = super::read_u64_be(&input[NAMESPACE_ID_BYTES..])?;
        if revision == 0 {
            return Err(ProtocolError::InvalidRevision);
        }
        let (policy, policy_len) = compat_v1::decode_namespace_policy(&input[fixed..])?
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

/// Optional behavior for one `SET` request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SetOptions {
    /// Atomic existence condition.
    pub condition: SetCondition,
    /// Item-level expiration selection.
    pub expiration_mode: ExpirationMode,
    /// Relative lifetime in milliseconds when `expiration_mode` is
    /// `ExplicitTtl`.
    pub ttl_ms: Option<u64>,
    /// Item-level eviction selection.
    pub eviction_mode: EvictionMode,
}

impl Default for SetOptions {
    fn default() -> Self {
        Self::NONE
    }
}

impl SetOptions {
    /// Creates unconditional `SET` behavior inheriting namespace defaults.
    pub const NONE: Self = Self {
        condition: SetCondition::Any,
        expiration_mode: ExpirationMode::Inherit,
        ttl_ms: None,
        eviction_mode: EvictionMode::Inherit,
    };

    /// Creates options from an existence condition and optional explicit TTL.
    pub const fn new(condition: SetCondition, ttl_ms: Option<u64>) -> Self {
        Self {
            condition,
            expiration_mode: match ttl_ms {
                Some(_) => ExpirationMode::ExplicitTtl,
                None => ExpirationMode::Inherit,
            },
            ttl_ms,
            eviction_mode: EvictionMode::Inherit,
        }
    }

    /// Creates options with all item-level policy selections.
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

    /// Decodes the protocol-v1 SET flags and optional TTL.
    ///
    /// The wire interpretation remains implemented by the v1 compatibility
    /// adapter; this facade preserves the public protocol helper without
    /// exposing the adapter module or coupling generic planning to SET
    /// semantics.
    pub fn from_wire_parts(flags: u8, ttl_ms: Option<u64>) -> Result<Self> {
        Self::decode_set_options(flags, ttl_ms)
    }
}
