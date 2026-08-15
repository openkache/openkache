//! Composition-root services for compatibility-mode modeled operations.
//!
//! Generic operation parsing and execution do not depend on the concrete cache
//! or namespace registry. This module is the composition adapter that supplies
//! the capabilities used by the currently modeled operations while allowing
//! future operations to provide a different service bundle.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures_util::lock::Mutex as AsyncMutex;
use openkache_protocol::{Opcode, OwnedRange};

use super::super::types::{
    StorageKey, StorageWriteCondition, StorageWriteEviction, StorageWriteExpiration,
    StorageWriteOptions, StoredItemValue,
};
use super::super::{KvError, SetOutcome};
use super::operation_api::{CapabilityKey, ResourceLock};
use super::operation_contract::telemetry_operation;
use super::{
    NamespaceDescriptor, NamespaceError, NamespaceOpenResult, NamespacePolicy, NamespaceRegistry,
    NetworkWorkerCache, ObservabilityState, SetReservation,
};

pub(super) type CacheFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, KvError>> + 'a>>;

/// Storage capability exposed to the compatibility API behavior.
///
/// This is an adapter contract over the current key/value worker. Generic API
/// modules should register their own capability types when they need richer
/// commands; the dispatcher and wire layers do not grow a cache-command enum.
pub(super) trait StorageCapability: Send + Sync {
    fn max_item_bytes(&self) -> usize;
    fn namespace_item_storage_key(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
    ) -> StorageKey;
    fn worker_for(&self, storage_key: &StorageKey) -> usize;
    fn get_storage_key<'a>(
        &'a self,
        storage_key: StorageKey,
    ) -> CacheFuture<'a, Option<StoredItemValue>>;
    fn set_storage_key<'a>(
        &'a self,
        storage_key: StorageKey,
        value: StoredItemValue,
        options: StorageWriteOptions,
    ) -> CacheFuture<'a, SetOutcome>;
    fn delete_storage_key<'a>(&'a self, storage_key: StorageKey) -> CacheFuture<'a, bool>;
    fn stats<'a>(&'a self) -> CacheFuture<'a, Vec<String>>;
    fn sync_workers<'a>(&'a self, workers: &'a [usize]) -> CacheFuture<'a, ()>;
}

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

pub(super) trait ObservabilityCapability: Send + Sync {
    fn stats_json_fields(&self) -> String;
}

pub(super) type StorageCapabilityHandle = Arc<dyn StorageCapability>;
pub(super) type NamespaceCapabilityHandle = Arc<dyn NamespaceCapability>;
pub(super) type ObservabilityCapabilityHandle = Arc<dyn ObservabilityCapability>;

pub(super) const COMPATIBILITY_STORAGE_PORT: CapabilityKey<StorageCapabilityHandle> =
    CapabilityKey::new("openkache.compatibility.storage_port");
pub(super) const COMPATIBILITY_NAMESPACE_PORT: CapabilityKey<NamespaceCapabilityHandle> =
    CapabilityKey::new("openkache.compatibility.namespace_port");
pub(super) const COMPATIBILITY_OBSERVABILITY_PORT: CapabilityKey<ObservabilityCapabilityHandle> =
    CapabilityKey::new("openkache.compatibility.observability_port");

pub(super) struct GetState {
    pub(super) storage: StorageCapabilityHandle,
    pub(super) namespaces: NamespaceCapabilityHandle,
}

pub(super) struct SetState {
    pub(super) storage: StorageCapabilityHandle,
    pub(super) namespaces: NamespaceCapabilityHandle,
    pub(super) max_item_bytes: usize,
}

pub(super) struct DeleteState {
    pub(super) storage: StorageCapabilityHandle,
    pub(super) namespaces: NamespaceCapabilityHandle,
}

pub(super) struct StatsState {
    pub(super) storage: StorageCapabilityHandle,
    pub(super) namespaces: NamespaceCapabilityHandle,
    pub(super) observability: ObservabilityCapabilityHandle,
}

pub(super) struct SyncState {
    pub(super) storage: StorageCapabilityHandle,
    pub(super) namespaces: NamespaceCapabilityHandle,
}

pub(super) struct NamespaceOpenState {
    pub(super) namespaces: NamespaceCapabilityHandle,
}

pub(super) struct NamespaceUpdateState {
    pub(super) namespaces: NamespaceCapabilityHandle,
}

pub(super) struct NamespaceDeleteState {
    pub(super) storage: StorageCapabilityHandle,
    pub(super) namespaces: NamespaceCapabilityHandle,
}

impl StorageCapability for NetworkWorkerCache {
    fn max_item_bytes(&self) -> usize {
        NetworkWorkerCache::max_item_bytes(self)
    }

    fn namespace_item_storage_key(
        &self,
        namespace_id: u64,
        item_id: openkache_protocol::ItemId,
    ) -> StorageKey {
        NetworkWorkerCache::storage_key_for_domain_identity(self, namespace_id, item_id.as_bytes())
    }

    fn worker_for(&self, storage_key: &StorageKey) -> usize {
        NetworkWorkerCache::worker_for(self, storage_key)
    }

    fn get_storage_key<'a>(
        &'a self,
        storage_key: StorageKey,
    ) -> CacheFuture<'a, Option<StoredItemValue>> {
        Box::pin(NetworkWorkerCache::get_storage_key(
            self,
            storage_key,
            telemetry_operation(Opcode::Get),
        ))
    }

    fn set_storage_key<'a>(
        &'a self,
        storage_key: StorageKey,
        value: StoredItemValue,
        options: StorageWriteOptions,
    ) -> CacheFuture<'a, SetOutcome> {
        Box::pin(NetworkWorkerCache::set_storage_key(
            self,
            storage_key,
            value,
            options,
            telemetry_operation(Opcode::Set),
        ))
    }

    fn delete_storage_key<'a>(&'a self, storage_key: StorageKey) -> CacheFuture<'a, bool> {
        Box::pin(NetworkWorkerCache::delete_storage_key(
            self,
            storage_key,
            telemetry_operation(Opcode::Delete),
        ))
    }

    fn stats<'a>(&'a self) -> CacheFuture<'a, Vec<String>> {
        Box::pin(NetworkWorkerCache::stats(
            self,
            telemetry_operation(Opcode::Stats),
        ))
    }

    fn sync_workers<'a>(&'a self, workers: &'a [usize]) -> CacheFuture<'a, ()> {
        Box::pin(NetworkWorkerCache::sync_workers(
            self,
            workers,
            telemetry_operation(Opcode::Sync),
        ))
    }
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
