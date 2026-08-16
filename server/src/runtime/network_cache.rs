//! Network-worker view of the storage runtime.
//!
//! This adapter carries requester identity for telemetry and exposes both the
//! neutral keyed cache calls through one network-owned handle. Worker
//! lifecycle stays in sibling runtime modules; generic address hashing lives
//! here at the adapter boundary.

use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::observability::{NetworkWorkerId, Operation};
use crate::types::StoredItemValue;
use crate::{KvError, Result, SetOutcome, StorageKey};

use super::ThreadedKvkache;
use super::storage_port::{
    StorageAddress, StorageError, StorageMutation, StorageReadOwner, StorageReadValue,
    StorageResult, StorageValue, StorageWriteOptions, StorageWriteOutcome,
};

const GENERIC_STORAGE_ADDRESS_DOMAIN: &[u8] = b"openkache/generic-storage-address/v1\0";

impl From<KvError> for StorageError {
    fn from(error: KvError) -> Self {
        match error {
            KvError::InvalidRequest(message) => Self::InvalidRequest(message),
            KvError::Worker(message) => Self::Worker(message),
            KvError::Timeout(message) => Self::Timeout(message.into()),
            KvError::CapacityExhausted { resource } => {
                Self::Unavailable(format!("{resource} capacity is exhausted"))
            }
            KvError::NoCapacity => Self::Unavailable("storage has no writable capacity".into()),
            error => Self::Backend(error.to_string()),
        }
    }
}

fn storage_key_for_address(address: &StorageAddress) -> StorageKey {
    let mut hasher = Sha256::new();
    hasher.update(GENERIC_STORAGE_ADDRESS_DOMAIN);
    hasher.update(address.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; crate::types::STORAGE_KEY_BYTES];
    bytes.copy_from_slice(&digest[..crate::types::STORAGE_KEY_BYTES]);
    StorageKey::new(bytes)
}

/// A network-worker-owned view of the storage runtime and its request shard.
#[derive(Clone)]
pub(crate) struct NetworkWorkerCache {
    cache: Arc<ThreadedKvkache>,
    network_worker: NetworkWorkerId,
}

impl NetworkWorkerCache {
    pub(crate) fn new(cache: Arc<ThreadedKvkache>, network_worker: NetworkWorkerId) -> Self {
        Self {
            cache,
            network_worker,
        }
    }

    pub(crate) fn max_item_bytes(&self) -> usize {
        self.cache.max_item_bytes()
    }

    /// Routes a storage key using the runtime routing hash.
    pub(crate) fn worker_for(&self, storage_key: &StorageKey) -> usize {
        self.cache.owner(storage_key)
    }

    pub(crate) fn storage_key_for_identity(
        &self,
        identity: &[u8; crate::types::STORAGE_KEY_BYTES],
    ) -> StorageKey {
        self.cache.storage_key_for_identity(identity)
    }

    pub(crate) fn storage_key_for_domain_identity(
        &self,
        storage_domain_id: u64,
        identity: &[u8; crate::types::STORAGE_KEY_BYTES],
    ) -> StorageKey {
        self.cache
            .storage_key_for_domain_identity(storage_domain_id, identity)
    }

    pub(crate) async fn get_storage_key(
        &self,
        storage_key: StorageKey,
        operation: Operation,
    ) -> Result<Option<StoredItemValue>> {
        self.cache
            .get_storage_key_with_requester(storage_key, operation, Some(self.network_worker))
            .await
    }

    pub(crate) async fn set_storage_key(
        &self,
        storage_key: StorageKey,
        value: StoredItemValue,
        options: StorageWriteOptions,
        operation: Operation,
    ) -> Result<SetOutcome> {
        self.cache
            .set_storage_key_with_requester(
                storage_key,
                value,
                options,
                operation,
                Some(self.network_worker),
            )
            .await
    }

    pub(crate) async fn delete_storage_key(
        &self,
        storage_key: StorageKey,
        operation: Operation,
    ) -> Result<bool> {
        self.cache
            .delete_storage_key_with_requester(storage_key, operation, Some(self.network_worker))
            .await
    }

    pub(crate) async fn stats(&self, operation: Operation) -> Result<Vec<String>> {
        self.cache
            .stats_async_with_requester(operation, Some(self.network_worker))
            .await
    }

    pub(crate) async fn sync(&self, operation: Operation) -> Result<()> {
        self.cache
            .sync_async_with_requester(operation, Some(self.network_worker))
            .await
    }

    pub(crate) async fn sync_workers(
        &self,
        workers: &[usize],
        operation: Operation,
    ) -> Result<()> {
        self.cache
            .sync_workers_async_with_requester(workers, operation, Some(self.network_worker))
            .await
    }

    /// Retrieves a value through the neutral storage adapter.
    pub(crate) async fn storage_get(
        &self,
        operation: Operation,
        storage_address: StorageAddress,
    ) -> StorageResult<Option<StorageReadValue>> {
        let storage_key = storage_key_for_address(&storage_address);
        self.cache
            .get_storage_key_with_requester(storage_key, operation, Some(self.network_worker))
            .await
            .map(|value| value.map(StorageReadValue::from_owner))
            .map_err(StorageError::from)
    }

    /// Stores a value through the neutral storage adapter.
    pub(crate) async fn storage_set(
        &self,
        operation: Operation,
        storage_address: StorageAddress,
        value: StorageValue,
        options: StorageWriteOptions,
    ) -> StorageResult<StorageWriteOutcome> {
        let storage_key = storage_key_for_address(&storage_address);
        let value = StoredItemValue::from_owned_range(value.into_owned_range());
        self.cache
            .set_storage_key_with_requester(
                storage_key,
                value,
                options,
                operation,
                Some(self.network_worker),
            )
            .await
            .map(|outcome| match outcome {
                SetOutcome::Created => StorageWriteOutcome::Created,
                SetOutcome::Replaced => StorageWriteOutcome::Replaced,
                SetOutcome::NotStored => StorageWriteOutcome::Unchanged,
            })
            .map_err(StorageError::from)
    }

    /// Deletes a value through the neutral storage adapter.
    pub(crate) async fn storage_delete(
        &self,
        operation: Operation,
        storage_address: StorageAddress,
    ) -> StorageResult<StorageMutation> {
        let storage_key = storage_key_for_address(&storage_address);
        self.cache
            .delete_storage_key_with_requester(storage_key, operation, Some(self.network_worker))
            .await
            .map(|deleted| {
                if deleted {
                    StorageMutation::Applied
                } else {
                    StorageMutation::Unchanged
                }
            })
            .map_err(StorageError::from)
    }
}

impl StorageReadOwner for StoredItemValue {
    fn as_bytes(&self) -> &[u8] {
        self.as_ref()
    }
}
