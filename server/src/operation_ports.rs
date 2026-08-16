//! Neutral capability ports supplied by the server composition root.
//!
//! API modules depend on these narrow capability contracts rather than on
//! API-family-specific service names. The concrete providers stay in the server
//! composition layer, while each API decides which operation state it needs at
//! startup.

use std::sync::{Arc, Mutex};

use futures_util::lock::Mutex as AsyncMutex;
use openkache_protocol::OwnedRange;

use super::operation_capabilities::CapabilityKey;
use super::operation_preparation::ResourceLock;
use super::storage_port::StorageRoute;
use super::{
    NamespaceDescriptor, NamespaceError, NamespaceOpenResult, NamespacePolicy, NamespaceRegistry,
    ObservabilityState, ObservabilityStats, SetReservation,
};

/// Namespace metadata capability exposed to API behavior.
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

/// Observability capability needed by APIs that expose server statistics.
pub(super) trait ObservabilityCapability: Send + Sync {
    fn stats_snapshot(&self) -> ObservabilityStats;
}

pub(super) type NamespaceCapabilityHandle = Arc<dyn NamespaceCapability>;
pub(super) type ObservabilityCapabilityHandle = Arc<dyn ObservabilityCapability>;

/// Neutral capability identities. API modules may request these at startup
/// without depending on an API family or wire version.
pub(super) const NAMESPACE_PORT: CapabilityKey<NamespaceCapabilityHandle> =
    CapabilityKey::new("openkache.namespace.port");
pub(super) const OBSERVABILITY_PORT: CapabilityKey<ObservabilityCapabilityHandle> =
    CapabilityKey::new("openkache.observability.port");

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
    fn stats_snapshot(&self) -> ObservabilityStats {
        ObservabilityState::stats_summary(self)
    }
}
