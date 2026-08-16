//! Network-worker view of the storage runtime.
//!
//! This adapter carries requester identity for telemetry and exposes both the
//! neutral keyed cache calls through one network-owned handle. Worker
//! lifecycle stays in sibling runtime modules; generic address hashing lives
//! here at the adapter boundary.

use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::observability::{NetworkWorkerId, Operation};
use crate::types::{StoredItemBytes, StoredItemValue};
use crate::{KvError, Result, SetOutcome, StorageKey};

use super::ThreadedKvkache;
use super::storage_port::{
    PreparedStorageAddress, StorageError, StorageMutation, StorageReadValue, StorageResult,
    StorageRoute, StorageScope, StorageValue, StorageWriteOptions, StorageWriteOutcome,
};

const GENERIC_STORAGE_ADDRESS_DOMAIN: &[u8] = b"openkache/generic-storage-address/v1\0";

fn opaque_storage_key_for_scope(scope: &StorageScope, identity: &[u8]) -> StorageKey {
    let mut hasher = Sha256::new();
    hasher.update(GENERIC_STORAGE_ADDRESS_DOMAIN);
    // Keep the generic tuple unambiguous without allocating or changing the
    // fixed-width compact storage key.  Length prefixes prevent `(ab, c)` from
    // colliding with `(a, bc)` while the digest remains exactly 32 bytes.
    hasher.update((scope.as_bytes().len() as u64).to_be_bytes());
    hasher.update(scope.as_bytes());
    hasher.update((identity.len() as u64).to_be_bytes());
    hasher.update(identity);
    let digest = hasher.finalize();
    let mut bytes = [0_u8; crate::types::STORAGE_KEY_BYTES];
    bytes.copy_from_slice(&digest[..crate::types::STORAGE_KEY_BYTES]);
    StorageKey::new(bytes)
}

pub(crate) fn into_storage_read_value(value: StoredItemValue) -> StorageReadValue {
    match value.bytes {
        StoredItemBytes::Owned(buffer) => match Arc::try_unwrap(buffer) {
            Ok(buffer) => StorageReadValue::from_owned(buffer),
            Err(buffer) => {
                let len = buffer.len();
                StorageReadValue::from_shared_owner(buffer, 0..len)
                    .expect("a stored item range remains within its buffer")
            }
        },
        StoredItemBytes::RangedOwned { buffer, range } => match Arc::try_unwrap(buffer) {
            Ok(buffer) => StorageReadValue::from_owned_range(buffer, range)
                .expect("a stored item range remains within its buffer"),
            Err(buffer) => StorageReadValue::from_shared_owner(buffer, range)
                .expect("a stored item range remains within its buffer"),
        },
        StoredItemBytes::Segment { segment, range } => {
            StorageReadValue::from_shared_owner(segment, range)
                .expect("a stored item range remains within its segment")
        }
        StoredItemBytes::DirectRead { buffer, range } => {
            StorageReadValue::from_owner(StoredItemBytes::DirectRead { buffer, range })
        }
        StoredItemBytes::SharedDirectRead { buffer, range } => {
            StorageReadValue::from_shared_owner(buffer, range)
                .expect("a stored item range remains within its direct-read buffer")
        }
    }
}

impl From<KvError> for StorageError {
    fn from(error: KvError) -> Self {
        let message = error.to_string();
        match error {
            KvError::InvalidRequest(_) => Self::InvalidRequest(message),
            KvError::Worker(_) => Self::Worker(message),
            KvError::NoCapacity => Self::NoCapacity(message),
            KvError::TableFull | KvError::CapacityExhausted { .. } => Self::Overloaded(message),
            KvError::ItemTooLarge { .. } | KvError::BlobSegmentFull { .. } => {
                Self::TooLarge(message)
            }
            KvError::Timeout(_) => Self::Timeout(message),
            KvError::Io(_) | KvError::InvalidConfig(_) | KvError::Usage(_) => {
                Self::Backend(message)
            }
        }
    }
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

    pub(crate) async fn sync_workers(&self, workers: &[usize], operation: Operation) -> Result<()> {
        self.cache
            .sync_workers_async_with_requester(workers, operation, Some(self.network_worker))
            .await
    }

    pub(crate) async fn sync_routes(
        &self,
        routes: &[StorageRoute],
        operation: Operation,
    ) -> Result<()> {
        let workers = routes
            .iter()
            .map(|route| route.worker())
            .collect::<Vec<_>>();
        self.sync_workers(&workers, operation).await
    }

    /// Retrieves a value through the neutral storage adapter.
    pub(crate) async fn storage_get(
        &self,
        operation: Operation,
        prepared: PreparedStorageAddress,
    ) -> StorageResult<Option<StorageReadValue>> {
        let storage_key = StorageKey::new(*prepared.as_bytes());
        self.cache
            .get_storage_key_with_requester(storage_key, operation, Some(self.network_worker))
            .await
            .map(|value| value.map(into_storage_read_value))
            .map_err(StorageError::from)
    }

    /// Stores a value through the neutral storage adapter.
    pub(crate) async fn storage_set(
        &self,
        operation: Operation,
        prepared: PreparedStorageAddress,
        value: StorageValue,
        options: StorageWriteOptions,
    ) -> StorageResult<StorageWriteOutcome> {
        let storage_key = StorageKey::new(*prepared.as_bytes());
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
        prepared: PreparedStorageAddress,
    ) -> StorageResult<StorageMutation> {
        let storage_key = StorageKey::new(*prepared.as_bytes());
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

    pub(crate) fn prepare_address(
        &self,
        scope: StorageScope,
        identity: &[u8],
    ) -> PreparedStorageAddress {
        let key = opaque_storage_key_for_scope(&scope, identity);
        let route = StorageRoute::from_worker(self.worker_for(&key));
        PreparedStorageAddress::new(key.into_bytes(), route)
    }

    pub(crate) fn prepare_compatibility_address(
        &self,
        storage_domain_id: u64,
        identity: &[u8; crate::types::STORAGE_KEY_BYTES],
    ) -> PreparedStorageAddress {
        let key = self.storage_key_for_domain_identity(storage_domain_id, identity);
        let route = StorageRoute::from_worker(self.worker_for(&key));
        PreparedStorageAddress::new(key.into_bytes(), route)
    }

    pub(crate) fn storage_route_for(&self, prepared: &PreparedStorageAddress) -> StorageRoute {
        prepared.route()
    }
}
