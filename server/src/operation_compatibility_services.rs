//! State owned by the compatibility API and its SET policy adapter.
//!
//! Each modeled operation keeps only the exact neutral ports it needs.
//! Composition and concrete server providers remain outside this API-owned
//! state module.

use super::super::types::{
    StorageWriteCondition, StorageWriteEviction, StorageWriteExpiration, StorageWriteOptions,
};
use super::operation_ports::{
    NamespaceCatalogCapabilityHandle, NamespaceCoordinationCapabilityHandle,
    NamespaceMembershipCapabilityHandle, ObservabilityCapabilityHandle,
};
use super::storage_port::StoragePort;

pub(super) struct GetState<S = StoragePort> {
    pub(super) storage: S,
    pub(super) coordination: NamespaceCoordinationCapabilityHandle,
    pub(super) catalog: NamespaceCatalogCapabilityHandle,
    pub(super) membership: NamespaceMembershipCapabilityHandle,
}

pub(super) struct SetState<S = StoragePort> {
    pub(super) storage: S,
    pub(super) coordination: NamespaceCoordinationCapabilityHandle,
    pub(super) catalog: NamespaceCatalogCapabilityHandle,
    pub(super) membership: NamespaceMembershipCapabilityHandle,
    pub(super) max_item_bytes: usize,
}

pub(super) struct DeleteState<S = StoragePort> {
    pub(super) storage: S,
    pub(super) coordination: NamespaceCoordinationCapabilityHandle,
    pub(super) catalog: NamespaceCatalogCapabilityHandle,
    pub(super) membership: NamespaceMembershipCapabilityHandle,
}

pub(super) struct ExperimentalStatsState<S = StoragePort> {
    pub(super) storage: S,
    pub(super) coordination: NamespaceCoordinationCapabilityHandle,
    pub(super) catalog: NamespaceCatalogCapabilityHandle,
    pub(super) observability: ObservabilityCapabilityHandle,
}

pub(super) struct ExperimentalSyncState<S = StoragePort> {
    pub(super) storage: S,
    pub(super) coordination: NamespaceCoordinationCapabilityHandle,
    pub(super) catalog: NamespaceCatalogCapabilityHandle,
    pub(super) membership: NamespaceMembershipCapabilityHandle,
}

pub(super) struct NamespaceOpenState {
    pub(super) coordination: NamespaceCoordinationCapabilityHandle,
    pub(super) catalog: NamespaceCatalogCapabilityHandle,
}

pub(super) struct NamespaceUpdateState {
    pub(super) coordination: NamespaceCoordinationCapabilityHandle,
    pub(super) catalog: NamespaceCatalogCapabilityHandle,
}

pub(super) struct NamespaceDeleteState<S = StoragePort> {
    pub(super) storage: S,
    pub(super) coordination: NamespaceCoordinationCapabilityHandle,
    pub(super) catalog: NamespaceCatalogCapabilityHandle,
    pub(super) membership: NamespaceMembershipCapabilityHandle,
}

pub(crate) const fn storage_write_options(
    options: super::super::SetOptions,
) -> Result<StorageWriteOptions, &'static [u8]> {
    let expiration = match (options.expiration_mode, options.ttl_ms) {
        (super::super::ExpirationMode::Inherit, None) => StorageWriteExpiration::Inherit,
        (super::super::ExpirationMode::NoExpiry, None) => StorageWriteExpiration::NoExpiry,
        (super::super::ExpirationMode::ExplicitTtl, Some(ttl_ms)) => {
            StorageWriteExpiration::Ttl(ttl_ms)
        }
        _ => return Err(b"resolved SET expiration policy is inconsistent"),
    };
    Ok(StorageWriteOptions {
        condition: match options.condition {
            super::super::SetCondition::Any => StorageWriteCondition::Any,
            super::super::SetCondition::IfAbsent => StorageWriteCondition::IfAbsent,
            super::super::SetCondition::IfPresent => StorageWriteCondition::IfPresent,
        },
        expiration,
        eviction: match options.eviction_mode {
            super::super::EvictionMode::Inherit => StorageWriteEviction::Inherit,
            super::super::EvictionMode::Evictable => StorageWriteEviction::Evictable,
            super::super::EvictionMode::EvictionProtected => StorageWriteEviction::Protected,
        },
    })
}
