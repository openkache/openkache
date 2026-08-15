//! Client-owned domain values and request projection.
//!
//! The neutral protocol crate owns operation identifiers, frame plans, and
//! reusable codecs. This module owns Rust client semantics and the remaining
//! compact-v1 response projections.

use openkache_protocol::{NAMESPACE_ID_BYTES, NAMESPACE_REVISION_BYTES, Opcode};

use crate::request::RequestRetryPolicy;

#[path = "protocol_compat_v1.rs"]
mod compat_v1;
#[path = "operation_request.rs"]
mod operation_request;

pub(crate) use operation_request::OperationRequest;

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

/// Client-side SET policy selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SetWireOptions {
    pub(crate) condition: SetCondition,
    pub(crate) expiration_mode: ExpirationMode,
    pub(crate) ttl_ms: Option<u64>,
    pub(crate) eviction_mode: EvictionMode,
}

impl SetWireOptions {
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
