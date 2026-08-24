// Generated from the OpenKache Smithy contract. Do not edit.

/// Values defined by the Smithy EvictionDefault shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvictionDefault {
    /// Smithy evictable value.
    Evictable,
    /// Smithy eviction_protected value.
    EvictionProtected,
}

impl EvictionDefault {
    pub fn smithy_value(self) -> &'static str {
        match self {
            Self::Evictable => "evictable",
            Self::EvictionProtected => "eviction_protected",
        }
    }

    pub fn from_smithy_value(value: &str) -> Option<Self> {
        match value {
            "evictable" => Some(Self::Evictable),
            "eviction_protected" => Some(Self::EvictionProtected),
            _ => None,
        }
    }
}

/// Values defined by the Smithy EvictionMode shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvictionMode {
    /// Smithy inherit value.
    Inherit,
    /// Smithy evictable value.
    Evictable,
    /// Smithy eviction_protected value.
    EvictionProtected,
}

impl EvictionMode {
    pub fn smithy_value(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Evictable => "evictable",
            Self::EvictionProtected => "eviction_protected",
        }
    }

    pub fn from_smithy_value(value: &str) -> Option<Self> {
        match value {
            "inherit" => Some(Self::Inherit),
            "evictable" => Some(Self::Evictable),
            "eviction_protected" => Some(Self::EvictionProtected),
            _ => None,
        }
    }
}

/// Values defined by the Smithy ExpirationDefault shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpirationDefault {
    /// Smithy no_expiry value.
    NoExpiry,
    /// Smithy fixed_ttl value.
    FixedTtl,
}

impl ExpirationDefault {
    pub fn smithy_value(self) -> &'static str {
        match self {
            Self::NoExpiry => "no_expiry",
            Self::FixedTtl => "fixed_ttl",
        }
    }

    pub fn from_smithy_value(value: &str) -> Option<Self> {
        match value {
            "no_expiry" => Some(Self::NoExpiry),
            "fixed_ttl" => Some(Self::FixedTtl),
            _ => None,
        }
    }
}

/// Values defined by the Smithy ExpirationMode shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpirationMode {
    /// Smithy inherit value.
    Inherit,
    /// Smithy no_expiry value.
    NoExpiry,
    /// Smithy explicit_ttl value.
    ExplicitTtl,
}

impl ExpirationMode {
    pub fn smithy_value(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::NoExpiry => "no_expiry",
            Self::ExplicitTtl => "explicit_ttl",
        }
    }

    pub fn from_smithy_value(value: &str) -> Option<Self> {
        match value {
            "inherit" => Some(Self::Inherit),
            "no_expiry" => Some(Self::NoExpiry),
            "explicit_ttl" => Some(Self::ExplicitTtl),
            _ => None,
        }
    }
}

/// Values defined by the Smithy OverridePolicy shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverridePolicy {
    /// Smithy allowed value.
    Allowed,
    /// Smithy disallowed value.
    Disallowed,
}

impl OverridePolicy {
    pub fn smithy_value(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Disallowed => "disallowed",
        }
    }

    pub fn from_smithy_value(value: &str) -> Option<Self> {
        match value {
            "allowed" => Some(Self::Allowed),
            "disallowed" => Some(Self::Disallowed),
            _ => None,
        }
    }
}

/// Values defined by the Smithy SetCondition shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetCondition {
    /// Smithy any value.
    Any,
    /// Smithy if_absent value.
    IfAbsent,
    /// Smithy if_present value.
    IfPresent,
}

impl SetCondition {
    pub fn smithy_value(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::IfAbsent => "if_absent",
            Self::IfPresent => "if_present",
        }
    }

    pub fn from_smithy_value(value: &str) -> Option<Self> {
        match value {
            "any" => Some(Self::Any),
            "if_absent" => Some(Self::IfAbsent),
            "if_present" => Some(Self::IfPresent),
            _ => None,
        }
    }
}

/// Values defined by the Smithy SetOutcome shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetOutcome {
    /// Smithy created value.
    Created,
    /// Smithy replaced value.
    Replaced,
    /// Smithy not_stored value.
    NotStored,
}

impl SetOutcome {
    pub fn smithy_value(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Replaced => "replaced",
            Self::NotStored => "not_stored",
        }
    }

    pub fn from_smithy_value(value: &str) -> Option<Self> {
        match value {
            "created" => Some(Self::Created),
            "replaced" => Some(Self::Replaced),
            "not_stored" => Some(Self::NotStored),
            _ => None,
        }
    }
}

/// Smithy DeleteInput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteInput {
    /// Smithy namespaceId member.
    pub namespace_id: u64,
    /// Smithy itemId member.
    pub item_id: Vec<u8>,
}

/// Smithy DeleteOutput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteOutput {
    /// Smithy deleted member.
    pub deleted: bool,
}

/// Smithy ExperimentalStatsInput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExperimentalStatsInput {
    /// Smithy namespaceId member.
    pub namespace_id: u64,
}

/// Smithy ExperimentalStatsOutput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExperimentalStatsOutput {
    /// Smithy json member.
    pub json: String,
}

/// Smithy ExperimentalSyncInput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExperimentalSyncInput {
    /// Smithy namespaceId member.
    pub namespace_id: u64,
}

/// Smithy ExperimentalSyncOutput structure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExperimentalSyncOutput;

/// Smithy GetInput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetInput {
    /// Smithy namespaceId member.
    pub namespace_id: u64,
    /// Smithy itemId member.
    pub item_id: Vec<u8>,
}

/// Smithy GetOutput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetOutput {
    /// Smithy value member.
    pub value: Option<Vec<u8>>,
}

/// Smithy NamespaceDeleteInput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamespaceDeleteInput {
    /// Smithy namespaceId member.
    pub namespace_id: u64,
    /// Smithy expectedRevision member.
    pub expected_revision: u64,
}

/// Smithy NamespaceDeleteOutput structure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NamespaceDeleteOutput;

/// Smithy NamespaceDescriptor structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamespaceDescriptor {
    /// Smithy namespaceId member.
    pub namespace_id: u64,
    /// Smithy revision member.
    pub revision: u64,
    /// Smithy policy member.
    pub policy: NamespacePolicy,
}

/// Smithy NamespaceOpenInput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamespaceOpenInput {
    /// Smithy name member.
    pub name: String,
    /// Smithy createIfMissing member.
    pub create_if_missing: bool,
    /// Smithy policy member.
    pub policy: Option<NamespacePolicy>,
}

/// Smithy NamespaceOpenOutput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamespaceOpenOutput {
    /// Smithy descriptor member.
    pub descriptor: NamespaceDescriptor,
    /// Smithy created member.
    pub created: bool,
}

/// Smithy NamespacePolicy structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamespacePolicy {
    /// Smithy defaultExpiration member.
    pub default_expiration: ExpirationDefault,
    /// Smithy defaultTtlMilliseconds member.
    pub default_ttl_milliseconds: Option<u64>,
    /// Smithy expirationOverride member.
    pub expiration_override: OverridePolicy,
    /// Smithy defaultEviction member.
    pub default_eviction: EvictionDefault,
    /// Smithy evictionOverride member.
    pub eviction_override: OverridePolicy,
}

/// Smithy NamespaceUpdatePolicyInput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamespaceUpdatePolicyInput {
    /// Smithy namespaceId member.
    pub namespace_id: u64,
    /// Smithy expectedRevision member.
    pub expected_revision: u64,
    /// Smithy policy member.
    pub policy: NamespacePolicy,
}

/// Smithy NamespaceUpdatePolicyOutput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamespaceUpdatePolicyOutput {
    /// Smithy descriptor member.
    pub descriptor: NamespaceDescriptor,
}

/// Smithy PingInput structure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PingInput;

/// Smithy PingOutput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PingOutput {
    /// Smithy payload member.
    pub payload: Vec<u8>,
}

/// Smithy SetInput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetInput {
    /// Smithy namespaceId member.
    pub namespace_id: u64,
    /// Smithy itemId member.
    pub item_id: Vec<u8>,
    /// Smithy value member.
    pub value: Vec<u8>,
    /// Smithy condition member.
    pub condition: Option<SetCondition>,
    /// Smithy expirationMode member.
    pub expiration_mode: Option<ExpirationMode>,
    /// Smithy evictionMode member.
    pub eviction_mode: Option<EvictionMode>,
    /// Smithy ttlMilliseconds member.
    pub ttl_milliseconds: Option<u64>,
}

/// Smithy SetOutput structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetOutput {
    /// Smithy outcome member.
    pub outcome: SetOutcome,
}

/// Operations defined by the OpenKache Smithy service.
///
/// The trait does not require Send futures because the Rust client exposes
/// both Tokio/Quinn and runtime-local Compio implementations. Callers that
/// need cross-thread scheduling can add the bound to the concrete client.
pub trait OpenKacheApi {
    /// Error returned by an operation.
    type Error;

    /// Invokes the Smithy Ping operation.
    fn ping(
        &self,
        input: PingInput,
    ) -> impl core::future::Future<
        Output = core::result::Result<PingOutput, Self::Error>,
    >;

    /// Invokes the Smithy Get operation.
    fn get(
        &self,
        input: GetInput,
    ) -> impl core::future::Future<
        Output = core::result::Result<GetOutput, Self::Error>,
    >;

    /// Invokes the Smithy Set operation.
    fn set(
        &self,
        input: SetInput,
    ) -> impl core::future::Future<
        Output = core::result::Result<SetOutput, Self::Error>,
    >;

    /// Invokes the Smithy Delete operation.
    fn delete(
        &self,
        input: DeleteInput,
    ) -> impl core::future::Future<
        Output = core::result::Result<DeleteOutput, Self::Error>,
    >;

    /// Invokes the Smithy ExperimentalStats operation.
    fn experimental_stats(
        &self,
        input: ExperimentalStatsInput,
    ) -> impl core::future::Future<
        Output = core::result::Result<ExperimentalStatsOutput, Self::Error>,
    >;

    /// Invokes the Smithy ExperimentalSync operation.
    fn experimental_sync(
        &self,
        input: ExperimentalSyncInput,
    ) -> impl core::future::Future<
        Output = core::result::Result<ExperimentalSyncOutput, Self::Error>,
    >;

    /// Invokes the Smithy NamespaceOpen operation.
    fn namespace_open(
        &self,
        input: NamespaceOpenInput,
    ) -> impl core::future::Future<
        Output = core::result::Result<NamespaceOpenOutput, Self::Error>,
    >;

    /// Invokes the Smithy NamespaceUpdatePolicy operation.
    fn namespace_update_policy(
        &self,
        input: NamespaceUpdatePolicyInput,
    ) -> impl core::future::Future<
        Output = core::result::Result<NamespaceUpdatePolicyOutput, Self::Error>,
    >;

    /// Invokes the Smithy NamespaceDelete operation.
    fn namespace_delete(
        &self,
        input: NamespaceDeleteInput,
    ) -> impl core::future::Future<
        Output = core::result::Result<NamespaceDeleteOutput, Self::Error>,
    >;
}
