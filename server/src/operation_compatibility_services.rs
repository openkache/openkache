//! Composition-root services for compatibility-mode modeled operations.
//!
//! Generic operation parsing and execution do not depend on the concrete cache
//! or namespace registry. This module is the composition adapter that supplies
//! the capabilities used by the currently modeled operations while allowing
//! future operations to provide a different service bundle.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex,
    atomic::AtomicBool,
};

use futures_util::lock::Mutex as AsyncMutex;

use super::super::types::StoredItemValue;
use super::super::{KvError, SetOutcome};
use super::operation_compatibility_status as status;
use super::operation_api::{CapabilityKey, PrepareError, ResourceLock};
use super::operation_capabilities::CapabilityRegistry;
use super::{
    NamespaceDescriptor, NamespaceError, NamespaceOpenResult, NamespacePolicy, NamespaceRegistry,
    NetworkWorkerCache, ObservabilityState, SetReservation,
};

type NamespaceLock = (Arc<AsyncMutex<()>>, Arc<AtomicBool>);

/// Resource resolver for the compatibility namespace/storage adapter.
///
/// This adapter deliberately owns the only interpretation of the compatibility
/// eight-byte namespace identity. Generic operations do not pass their opaque
/// resource keys through this type; they obtain an API-owned resolver from the
/// capability catalog instead.
pub(super) struct CompatibilityResourceResolver {
    namespaces: Arc<Mutex<NamespaceRegistry>>,
}

impl CompatibilityResourceResolver {
    pub(super) fn new(namespaces: Arc<Mutex<NamespaceRegistry>>) -> Self {
        Self { namespaces }
    }

    pub(super) fn resolve_namespace(&self, identity: &[u8]) -> Result<ResourceLock, PrepareError> {
        let identity: [u8; 8] = identity
            .try_into()
            .map_err(|_| PrepareError::invalid_request(b"resource identity is malformed"))?;
        let identity = u64::from_be_bytes(identity);
        let (lock, active) = self.namespaces.operation_lock(identity).ok_or_else(|| {
            PrepareError::resource_unavailable(
                status::NAMESPACE_NOT_FOUND,
                b"namespace does not exist",
            )
        })?;
        Ok(ResourceLock::new(
            lock,
            active,
            PrepareError::resource_unavailable(
                status::NAMESPACE_NOT_FOUND,
                b"namespace was deleted while the operation was waiting",
            ),
        ))
    }

    pub(super) fn resolve_global(&self) -> Result<ResourceLock, PrepareError> {
        let shared = self.namespaces.lifecycle_lock().map_err(|_| {
            PrepareError::resource_unavailable(
                status::INTERNAL_ERROR,
                b"namespace metadata is unavailable",
            )
        })?;
        Ok(ResourceLock::unconditional(shared))
    }
}

/// Catalog key reserved by the compatibility adapter for its resource resolver.
///
/// The key is an adapter detail, not part of the generic dispatcher contract.
pub(super) const COMPATIBILITY_RESOURCE_RESOLVER: CapabilityKey<CompatibilityResourceResolver> =
    CapabilityKey::new("openkache.compatibility.resource_resolver");

/// Runtime services made available to this adapter by the composition root.
///
/// The bundle is an API-owned capability value rather than a shared
/// composition-context type. The compatibility module can therefore be
/// registered in another server composition without depending on the
/// operation registry's concrete runtime struct.
pub(super) struct CompatibilityRuntime {
    pub(super) cache: Arc<NetworkWorkerCache>,
    pub(super) namespaces: Arc<Mutex<NamespaceRegistry>>,
    pub(super) observability: Arc<ObservabilityState>,
}

impl CompatibilityRuntime {
    pub(super) const fn new(
        cache: Arc<NetworkWorkerCache>,
        namespaces: Arc<Mutex<NamespaceRegistry>>,
        observability: Arc<ObservabilityState>,
    ) -> Self {
        Self {
            cache,
            namespaces,
            observability,
        }
    }
}

pub(super) const COMPATIBILITY_RUNTIME: CapabilityKey<CompatibilityRuntime> =
    CapabilityKey::new("openkache.compatibility.runtime");

/// Adds the compatibility adapter's worker-scoped capabilities.
///
/// Generic runtime capabilities are installed by the server composition root
/// before this function is called. This adapter only adds compatibility resolver and
/// service values.
pub(super) fn install_compatibility_services(
    registry: &mut CapabilityRegistry,
    runtime: &CompatibilityRuntime,
) {
    let services: Arc<dyn CompatibilityServices + Send + Sync> = Arc::new(ServerContext::new(
        Arc::clone(&runtime.cache),
        Arc::clone(&runtime.namespaces),
        Arc::clone(&runtime.observability),
    ));
    registry.insert(
        COMPATIBILITY_RESOURCE_RESOLVER,
        CompatibilityResourceResolver::new(Arc::clone(&runtime.namespaces)),
    );
    registry.insert(COMPATIBILITY_SERVICES, services);
}

pub(super) type CacheFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, KvError>> + 'a>>;

/// Storage capability exposed to the compatibility API behavior.
///
/// This is an adapter contract over the current key/value worker. Generic API
/// modules should register their own capability types when they need richer
/// commands; the dispatcher and wire layers do not grow a cache-command enum.
pub(super) trait StorageCapability {
    fn namespace_item_worker(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
    ) -> usize;
    fn get_in_namespace<'a>(
        &'a self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
    ) -> CacheFuture<'a, Option<StoredItemValue>>;
    fn set_in_namespace<'a>(
        &'a self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
        value: StoredItemValue,
        options: super::super::SetOptions,
    ) -> CacheFuture<'a, SetOutcome>;
    fn delete_in_namespace<'a>(
        &'a self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
    ) -> CacheFuture<'a, bool>;
    fn stats<'a>(&'a self) -> CacheFuture<'a, Vec<String>>;
    fn sync_workers<'a>(&'a self, workers: &'a [usize]) -> CacheFuture<'a, ()>;
}

/// Namespace metadata capability exposed to the current storage behavior.
pub(super) trait NamespaceCapability {
    fn operation_lock(&self, namespace_id: u64) -> Option<NamespaceLock>;
    fn lifecycle_lock(&self) -> Result<Arc<AsyncMutex<()>>, NamespaceError>;
    fn exists(&self, namespace_id: u64) -> bool;
    fn policy(&self, namespace_id: u64) -> Option<NamespacePolicy>;
    fn open(
        &self,
        name: Vec<u8>,
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
    fn dirty_workers(&self, namespace_id: u64) -> Option<Vec<usize>>;
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
        worker: usize,
    ) -> Result<SetReservation, NamespaceError>;
    fn rollback_set_reservation(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
        worker: usize,
        reservation: SetReservation,
    ) -> Result<(), NamespaceError>;
    fn reserve_worker(&self, namespace_id: u64, worker: usize) -> Result<(), NamespaceError>;
    fn mark_delete(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
        deleted: bool,
    ) -> Result<(), NamespaceError>;
}

pub(super) trait ObservabilityCapability {
    fn stats_json_fields(&self) -> String;
}

/// Compatibility capability set supplied by the composition root.
///
/// This is deliberately outside the generic executor. The current compatibility
/// operations use the cache/namespace implementation through this adapter,
/// while a new API can use the opaque catalog without adding another method
/// here. Keeping the compatibility name explicit prevents this compatibility surface
/// from becoming the default extension point for future APIs.
pub(super) trait CompatibilityServices: Any + Send + Sync {
    fn storage(&self) -> &dyn StorageCapability;
    fn namespaces(&self) -> &dyn NamespaceCapability;
    fn observability(&self) -> &dyn ObservabilityCapability;
}

/// Capability key owned by the compatibility adapter.
///
/// The generic operation context only exposes the type-erased capability
/// catalog. Compatibility bindings opt into this key locally; no compatibility
/// service type crosses the generic handler or transport boundary.
pub(super) const COMPATIBILITY_SERVICES: CapabilityKey<
    Arc<dyn CompatibilityServices + Send + Sync>,
> = CapabilityKey::new("openkache.compatibility.services");

/// Server-owned dependencies for the current composition.
pub(super) struct ServerContext {
    pub(super) cache: Arc<NetworkWorkerCache>,
    pub(super) namespaces: Arc<Mutex<NamespaceRegistry>>,
    pub(super) observability: Arc<ObservabilityState>,
}

impl ServerContext {
    pub(super) const fn new(
        cache: Arc<NetworkWorkerCache>,
        namespaces: Arc<Mutex<NamespaceRegistry>>,
        observability: Arc<ObservabilityState>,
    ) -> Self {
        Self {
            cache,
            namespaces,
            observability,
        }
    }
}

impl StorageCapability for NetworkWorkerCache {
    fn namespace_item_worker(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
    ) -> usize {
        NetworkWorkerCache::namespace_item_worker(self, namespace_id, item_id)
    }

    fn get_in_namespace<'a>(
        &'a self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
    ) -> CacheFuture<'a, Option<StoredItemValue>> {
        Box::pin(NetworkWorkerCache::get_in_namespace(
            self,
            namespace_id,
            item_id,
        ))
    }

    fn set_in_namespace<'a>(
        &'a self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
        value: StoredItemValue,
        options: super::super::SetOptions,
    ) -> CacheFuture<'a, SetOutcome> {
        Box::pin(NetworkWorkerCache::set_in_namespace(
            self,
            namespace_id,
            item_id,
            value,
            options,
        ))
    }

    fn delete_in_namespace<'a>(
        &'a self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
    ) -> CacheFuture<'a, bool> {
        Box::pin(NetworkWorkerCache::delete_in_namespace(
            self,
            namespace_id,
            item_id,
        ))
    }

    fn stats<'a>(&'a self) -> CacheFuture<'a, Vec<String>> {
        Box::pin(NetworkWorkerCache::stats(self))
    }

    fn sync_workers<'a>(&'a self, workers: &'a [usize]) -> CacheFuture<'a, ()> {
        Box::pin(NetworkWorkerCache::sync_workers(self, workers))
    }
}

impl NamespaceCapability for Mutex<NamespaceRegistry> {
    fn operation_lock(&self, namespace_id: u64) -> Option<NamespaceLock> {
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
        name: Vec<u8>,
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

    fn dirty_workers(&self, namespace_id: u64) -> Option<Vec<usize>> {
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
        worker: usize,
    ) -> Result<SetReservation, NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .reserve_item(namespace_id, item_id, worker)
    }

    fn rollback_set_reservation(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
        worker: usize,
        reservation: SetReservation,
    ) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .rollback_set_reservation(namespace_id, item_id, worker, reservation)
    }

    fn reserve_worker(&self, namespace_id: u64, worker: usize) -> Result<(), NamespaceError> {
        self.lock()
            .map_err(|_| NamespaceError::Internal)?
            .reserve_worker(namespace_id, worker)
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

impl CompatibilityServices for ServerContext {
    fn storage(&self) -> &dyn StorageCapability {
        self.cache.as_ref()
    }

    fn namespaces(&self) -> &dyn NamespaceCapability {
        self.namespaces.as_ref()
    }

    fn observability(&self) -> &dyn ObservabilityCapability {
        self.observability.as_ref()
    }
}
