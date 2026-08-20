//! Neutral capability ports supplied by the server composition root.
//!
//! API modules depend on these narrow capability contracts rather than on
//! API-family-specific service names. The concrete providers stay in the server
//! composition layer, while each API decides which operation state it needs at
//! startup.

use std::sync::{Arc, Mutex};

use super::operation_capabilities::CapabilityKey;
use super::storage_port::StorageRoute;
use super::{
    NamespaceError, NamespaceOperationLock, NamespacePolicy, NamespaceRegistry, ObservabilityState,
    ObservabilityStats, SetReservation,
};
use super::super::types::StorageKey;

/// Namespace locking capability used during operation preparation.
pub(super) trait NamespaceCoordinationCapability: Send + Sync {
    fn operation_lock(&self, namespace_id: u64) -> Option<NamespaceOperationLock>;
}

/// Namespace descriptor and policy capability exposed to API behavior.
pub(super) trait NamespaceCatalogCapability: Send + Sync {
    fn exists(&self, namespace_id: u64) -> bool;
    fn policy(&self, namespace_id: u64) -> Option<NamespacePolicy>;
}

/// Namespace item and worker membership capability exposed to API behavior.
pub(super) trait NamespaceMembershipCapability: Send + Sync {
    fn dirty_workers(&self, namespace_id: u64) -> Option<Vec<StorageRoute>>;
    fn mark_workers_clean(&self, namespace_id: u64) -> Result<(), NamespaceError>;
    fn prune_item(&self, namespace_id: u64, storage_key: StorageKey) -> Result<(), NamespaceError>;
    fn reserve_item(
        &self,
        namespace_id: u64,
        storage_key: StorageKey,
        route: StorageRoute,
    ) -> Result<SetReservation, NamespaceError>;
    fn rollback_set_reservation(
        &self,
        namespace_id: u64,
        storage_key: StorageKey,
        route: StorageRoute,
        reservation: SetReservation,
    ) -> Result<(), NamespaceError>;
    fn reserve_worker(&self, namespace_id: u64, route: StorageRoute) -> Result<(), NamespaceError>;
    fn mark_delete(
        &self,
        namespace_id: u64,
        storage_key: StorageKey,
        deleted: bool,
    ) -> Result<(), NamespaceError>;
}

/// Observability capability needed by APIs that expose server statistics.
pub(super) trait ObservabilityCapability: Send + Sync {
    fn stats_snapshot(&self) -> ObservabilityStats;
}

pub(super) type NamespaceCoordinationCapabilityHandle = Arc<dyn NamespaceCoordinationCapability>;
pub(super) type NamespaceCatalogCapabilityHandle = Arc<dyn NamespaceCatalogCapability>;
pub(super) type NamespaceMembershipCapabilityHandle = Arc<dyn NamespaceMembershipCapability>;
pub(super) type ObservabilityCapabilityHandle = Arc<dyn ObservabilityCapability>;

/// Neutral capability identities. API modules may request these at startup
/// without depending on an API family or wire version.
pub(super) const NAMESPACE_COORDINATION_PORT: CapabilityKey<NamespaceCoordinationCapabilityHandle> =
    CapabilityKey::new("openkache.namespace.coordination.port");
pub(super) const NAMESPACE_CATALOG_PORT: CapabilityKey<NamespaceCatalogCapabilityHandle> =
    CapabilityKey::new("openkache.namespace.catalog.port");
pub(super) const NAMESPACE_MEMBERSHIP_PORT: CapabilityKey<NamespaceMembershipCapabilityHandle> =
    CapabilityKey::new("openkache.namespace.membership.port");
pub(super) const OBSERVABILITY_PORT: CapabilityKey<ObservabilityCapabilityHandle> =
    CapabilityKey::new("openkache.observability.port");

impl NamespaceCoordinationCapability for Mutex<NamespaceRegistry> {
    fn operation_lock(&self, namespace_id: u64) -> Option<NamespaceOperationLock> {
        self.lock().ok()?.operation_lock(namespace_id)
    }
}

impl NamespaceCatalogCapability for Mutex<NamespaceRegistry> {
    fn exists(&self, namespace_id: u64) -> bool {
        self.lock()
            .ok()
            .and_then(|registry| registry.descriptor(namespace_id))
            .is_some()
    }

    fn policy(&self, namespace_id: u64) -> Option<NamespacePolicy> {
        self.lock().ok()?.policy(namespace_id)
    }
}

impl NamespaceMembershipCapability for Mutex<NamespaceRegistry> {
    fn dirty_workers(&self, namespace_id: u64) -> Option<Vec<StorageRoute>> {
        self.lock().ok()?.dirty_workers(namespace_id)
    }

    fn mark_workers_clean(&self, namespace_id: u64) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .mark_workers_clean(namespace_id)
    }

    fn prune_item(&self, namespace_id: u64, storage_key: StorageKey) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .prune_item(namespace_id, storage_key)
    }

    fn reserve_item(
        &self,
        namespace_id: u64,
        storage_key: StorageKey,
        route: StorageRoute,
    ) -> Result<SetReservation, NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .reserve_item(namespace_id, storage_key, route)
    }

    fn rollback_set_reservation(
        &self,
        namespace_id: u64,
        storage_key: StorageKey,
        route: StorageRoute,
        reservation: SetReservation,
    ) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .rollback_set_reservation(namespace_id, storage_key, route, reservation)
    }

    fn reserve_worker(&self, namespace_id: u64, route: StorageRoute) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .reserve_worker(namespace_id, route)
    }

    fn mark_delete(
        &self,
        namespace_id: u64,
        storage_key: StorageKey,
        deleted: bool,
    ) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .mark_delete(namespace_id, storage_key, deleted)
    }
}

impl ObservabilityCapability for ObservabilityState {
    fn stats_snapshot(&self) -> ObservabilityStats {
        ObservabilityState::stats_summary(self)
    }
}
