//! Composition-root services for compatibility-mode modeled operations.
//!
//! Generic operation parsing and execution do not depend on the concrete cache
//! or namespace registry. This module is the composition adapter that supplies
//! the capabilities used by the currently modeled operations while allowing
//! future operations to provide a different service bundle.

use std::sync::{Arc, Mutex};

use futures_util::lock::Mutex as AsyncMutex;
use openkache_protocol::OwnedRange;

use super::super::types::{
    StorageWriteCondition, StorageWriteEviction, StorageWriteExpiration, StorageWriteOptions,
};
use super::operation_capabilities::CapabilityKey;
use super::operation_preparation::ResourceLock;
use super::storage_port::{StoragePort, StorageRoute};
use super::{
    NamespaceDescriptor, NamespaceError, NamespaceOpenResult, NamespacePolicy, NamespaceRegistry,
    ObservabilityState, SetReservation,
};

/// Namespace metadata capability exposed to the current storage behavior.
pub(super) trait NamespaceCapability: Send + Sync {
    fn operation_lock(&self, namespace_id: u64) -> Option<ResourceLock>;
    fn lifecycle_lock(&self) -> Result<Arc<AsyncMutex<()>>, NamespaceError>;
    fn exists(&self, namespace_id: u64) -> bool;
    fn policy(&self, namespace_id: u64) -> Option<NamespacePolicy>;
    fn open(
        &self,
        name: OwnedRange,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> Result<(NamespaceOpenResult, NamespaceDescriptor), NamespaceError>;
    fn update(
        &self,
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> Result<NamespaceDescriptor, NamespaceError>;
    fn delete(&self, namespace_id: u64, expected_revision: u64) -> Result<(), NamespaceError>;
    fn tracked_items(&self, namespace_id: u64) -> Option<Vec<openkache_protocol::ItemId>>;
    fn dirty_workers(&self, namespace_id: u64) -> Option<Vec<StorageRoute>>;
    fn mark_workers_clean(&self, namespace_id: u64) -> Result<(), NamespaceError>;
    fn prune_item(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
    ) -> Result<(), NamespaceError>;
    fn reserve_item(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
        route: StorageRoute,
    ) -> Result<SetReservation, NamespaceError>;
    fn rollback_set_reservation(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
        route: StorageRoute,
        reservation: SetReservation,
    ) -> Result<(), NamespaceError>;
    fn reserve_worker(&self, namespace_id: u64, route: StorageRoute) -> Result<(), NamespaceError>;
    fn mark_delete(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
        deleted: bool,
    ) -> Result<(), NamespaceError>;
}

pub(super) trait ObservabilityCapability: Send + Sync {
    fn stats_json_fields(&self) -> String;
}

pub(super) type NamespaceCapabilityHandle = Arc<dyn NamespaceCapability>;
pub(super) type ObservabilityCapabilityHandle = Arc<dyn ObservabilityCapability>;

pub(super) const COMPATIBILITY_NAMESPACE_PORT: CapabilityKey<NamespaceCapabilityHandle> =
    CapabilityKey::new("openkache.compatibility.namespace_port");
pub(super) const COMPATIBILITY_OBSERVABILITY_PORT: CapabilityKey<ObservabilityCapabilityHandle> =
    CapabilityKey::new("openkache.compatibility.observability_port");

pub(super) struct GetState {
    pub(super) storage: StoragePort,
    pub(super) namespaces: NamespaceCapabilityHandle,
}

pub(super) struct SetState {
    pub(super) storage: StoragePort,
    pub(super) namespaces: NamespaceCapabilityHandle,
    pub(super) max_item_bytes: usize,
}

pub(super) struct DeleteState {
    pub(super) storage: StoragePort,
    pub(super) namespaces: NamespaceCapabilityHandle,
}

pub(super) struct StatsState {
    pub(super) storage: StoragePort,
    pub(super) namespaces: NamespaceCapabilityHandle,
    pub(super) observability: ObservabilityCapabilityHandle,
}

pub(super) struct SyncState {
    pub(super) storage: StoragePort,
    pub(super) namespaces: NamespaceCapabilityHandle,
}

pub(super) struct NamespaceOpenState {
    pub(super) namespaces: NamespaceCapabilityHandle,
}

pub(super) struct NamespaceUpdateState {
    pub(super) namespaces: NamespaceCapabilityHandle,
}

pub(super) struct NamespaceDeleteState {
    pub(super) storage: StoragePort,
    pub(super) namespaces: NamespaceCapabilityHandle,
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

impl NamespaceCapability for Mutex<NamespaceRegistry> {
    fn operation_lock(&self, namespace_id: u64) -> Option<ResourceLock> {
        self.lock().ok()?.operation_lock(namespace_id)
    }

    fn lifecycle_lock(&self) -> Result<Arc<AsyncMutex<()>>, NamespaceError> {
        self.lock()
            .map(|registry| registry.lifecycle_lock())
            .map_err(|_| NamespaceError::Internal)
    }

    fn exists(&self, namespace_id: u64) -> bool {
        self.lock()
            .ok()
            .and_then(|registry| registry.descriptor(namespace_id))
            .is_some()
    }

    fn policy(&self, namespace_id: u64) -> Option<NamespacePolicy> {
        self.lock().ok()?.policy(namespace_id)
    }

    fn open(
        &self,
        name: OwnedRange,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> Result<(NamespaceOpenResult, NamespaceDescriptor), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .open(name, create_if_missing, policy)
    }

    fn update(
        &self,
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> Result<NamespaceDescriptor, NamespaceError> {
        self.lock().map_err(|_| NamespaceError::Internal)?.update(
            namespace_id,
            expected_revision,
            policy,
        )
    }

    fn delete(&self, namespace_id: u64, expected_revision: u64) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .delete(namespace_id, expected_revision)
    }

    fn tracked_items(&self, namespace_id: u64) -> Option<Vec<openkache_protocol::ItemId>> {
        self.lock().ok()?.tracked_items(namespace_id)
    }

    fn dirty_workers(&self, namespace_id: u64) -> Option<Vec<StorageRoute>> {
        self.lock().ok()?.dirty_workers(namespace_id)
    }

    fn mark_workers_clean(&self, namespace_id: u64) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .mark_workers_clean(namespace_id)
    }

    fn prune_item(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
    ) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .prune_item(namespace_id, item_id)
    }

    fn reserve_item(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
        route: StorageRoute,
    ) -> Result<SetReservation, NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .reserve_item(namespace_id, item_id, route)
    }

    fn rollback_set_reservation(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
        route: StorageRoute,
        reservation: SetReservation,
    ) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .rollback_set_reservation(namespace_id, item_id, route, reservation)
    }

    fn reserve_worker(&self, namespace_id: u64, route: StorageRoute) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .reserve_worker(namespace_id, route)
    }

    fn mark_delete(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
        deleted: bool,
    ) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .mark_delete(namespace_id, item_id, deleted)
    }
}

impl ObservabilityCapability for ObservabilityState {
    fn stats_json_fields(&self) -> String {
        ObservabilityState::stats_json_fields(self)
    }
}
