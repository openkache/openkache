//! Network-worker view of the storage runtime.
//!
//! This adapter carries requester identity for telemetry and exposes both the
//! compatibility cache calls and the neutral [`StoragePort`] through one
//! network-owned handle. Worker lifecycle and key derivation stay in sibling
//! runtime modules.

use std::sync::Arc;

use crate::observability::{NetworkWorkerId, Operation};
use crate::protocol::ItemId;
use crate::types::StoredItemValue;
use crate::{Result, SetOutcome, StorageKey};

use super::ThreadedKvkache;
use super::storage_backend;
use super::storage_port::{
    StorageAddress, StorageError, StorageMutation, StorageMutationFuture, StoragePort,
    StorageReadFuture, StorageReadOwner, StorageReadValue, StorageResult, StorageTaskFuture,
    StorageTaskOutput, StorageTaskScope, StorageValue, StorageWriteFuture, StorageWriteOptions,
    StorageWriteOutcome,
};
use super::storage_task::StorageTask;

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

    /// Runs API-owned storage work on one worker without adding an
    /// operation-specific worker command or response variant.
    #[allow(dead_code)]
    async fn execute_storage_task(
        &self,
        worker: usize,
        task: StorageTask,
    ) -> StorageResult<StorageTaskOutput> {
        self.cache
            .execute_storage_task_with_requester(worker, Some(self.network_worker), task)
            .await
    }

    /// Runs an API-owned task on the worker selected by its storage key.
    #[allow(dead_code)]
    async fn execute_storage_task_for_key(
        &self,
        storage_key: StorageKey,
        task: StorageTask,
    ) -> StorageResult<StorageTaskOutput> {
        let worker = self.worker_for(&storage_key);
        self.cache
            .execute_storage_task_for_key_with_requester(
                worker,
                storage_key,
                Some(self.network_worker),
                task,
            )
            .await
    }

    /// Runs an API-owned task on a deterministic worker when it has no key.
    #[allow(dead_code)]
    async fn execute_storage_task_unbound(
        &self,
        task: StorageTask,
    ) -> StorageResult<StorageTaskOutput> {
        let worker = self.network_worker.index() % self.cache.workers.len();
        self.cache
            .execute_storage_task_with_requester(worker, Some(self.network_worker), task)
            .await
    }

    /// Routes an API-owned opaque storage key using the same hash as keyed
    /// compatibility operations.
    #[allow(dead_code)]
    pub(crate) fn worker_for(&self, storage_key: &StorageKey) -> usize {
        self.cache.owner(storage_key)
    }

    pub(crate) fn namespace_item_storage_key(
        &self,
        namespace_id: u64,
        item_id: ItemId,
    ) -> StorageKey {
        self.cache
            .namespace_item_storage_key(namespace_id, item_id)
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

    pub(crate) async fn get_stored(
        &self,
        item_id: ItemId,
        operation: Operation,
    ) -> Result<Option<StoredItemValue>> {
        self.cache
            .get_async_with_requester(item_id, operation, Some(self.network_worker))
            .await
    }

    pub(crate) async fn set_with_options(
        &self,
        item_id: ItemId,
        value: StoredItemValue,
        options: StorageWriteOptions,
        operation: Operation,
    ) -> Result<SetOutcome> {
        self.cache
            .set_async_with_options_requester(
                item_id,
                value,
                options,
                operation,
                Some(self.network_worker),
            )
            .await
    }

    pub(crate) async fn delete(&self, item_id: ItemId, operation: Operation) -> Result<bool> {
        self.cache
            .delete_async_with_requester(item_id, operation, Some(self.network_worker))
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
}

impl StoragePort for NetworkWorkerCache {
    fn get<'a>(&'a self, storage_address: StorageAddress) -> StorageReadFuture<'a> {
        let storage_key = storage_backend::storage_key_for_address(&storage_address);
        Box::pin(async move {
            self.cache
                .get_storage_key_with_requester(
                    storage_key,
                    Operation::unknown(),
                    Some(self.network_worker),
                )
                .await
                .map(|value| value.map(StorageReadValue::from_owner))
                .map_err(StorageError::from)
        })
    }

    fn set<'a>(
        &'a self,
        storage_address: StorageAddress,
        value: StorageValue,
        options: StorageWriteOptions,
    ) -> StorageWriteFuture<'a> {
        let storage_key = storage_backend::storage_key_for_address(&storage_address);
        let value = StoredItemValue::from_owned_range(value.into_owned_range());
        Box::pin(async move {
            self.cache
                .set_storage_key_with_requester(
                    storage_key,
                    value,
                    options,
                    Operation::unknown(),
                    Some(self.network_worker),
                )
                .await
                .map(|outcome| match outcome {
                    SetOutcome::Created => StorageWriteOutcome::Created,
                    SetOutcome::Replaced => StorageWriteOutcome::Replaced,
                    SetOutcome::NotStored => StorageWriteOutcome::Unchanged,
                })
                .map_err(StorageError::from)
        })
    }

    fn delete<'a>(&'a self, storage_address: StorageAddress) -> StorageMutationFuture<'a> {
        let storage_key = storage_backend::storage_key_for_address(&storage_address);
        Box::pin(async move {
            self.cache
                .delete_storage_key_with_requester(
                    storage_key,
                    Operation::unknown(),
                    Some(self.network_worker),
                )
                .await
                .map(|deleted| {
                    if deleted {
                        StorageMutation::Applied
                    } else {
                        StorageMutation::Unchanged
                    }
                })
                .map_err(StorageError::from)
        })
    }

    fn execute_for_key<'a>(
        &'a self,
        storage_address: StorageAddress,
        task: StorageTask,
    ) -> StorageTaskFuture<'a> {
        if let Err(message) = task
            .metadata()
            .validate_submission(StorageTaskScope::SingleKey)
        {
            return Box::pin(async move { Err(StorageError::InvalidRequest(message.into())) });
        }
        Box::pin(async move {
            self.execute_storage_task_for_key(
                storage_backend::storage_key_for_address(&storage_address),
                task,
            )
            .await
        })
    }

    fn execute_for_keys<'a>(
        &'a self,
        storage_addresses: &'a [StorageAddress],
        task: StorageTask,
    ) -> StorageTaskFuture<'a> {
        if let Err(message) = task
            .metadata()
            .validate_submission(StorageTaskScope::KeySet)
        {
            return Box::pin(async move { Err(StorageError::InvalidRequest(message.into())) });
        }
        let Some(first) = storage_addresses.first() else {
            return Box::pin(async {
                Err(StorageError::InvalidRequest(
                    "storage task requires at least one key".into(),
                ))
            });
        };
        let worker = self.worker_for(&storage_backend::storage_key_for_address(first));
        if storage_addresses.iter().skip(1).any(|storage_address| {
            self.worker_for(&storage_backend::storage_key_for_address(storage_address)) != worker
        }) {
            return Box::pin(async {
                Err(StorageError::Worker(
                    "multi-key storage task crosses worker boundaries".into(),
                ))
            });
        }
        Box::pin(async move {
            self.cache
                .execute_storage_task_with_requester(worker, Some(self.network_worker), task)
                .await
        })
    }

    fn execute_unbound<'a>(&'a self, task: StorageTask) -> StorageTaskFuture<'a> {
        if let Err(message) = task
            .metadata()
            .validate_submission(StorageTaskScope::Unbound)
        {
            return Box::pin(async move { Err(StorageError::InvalidRequest(message.into())) });
        }
        Box::pin(async move { self.execute_storage_task_unbound(task).await })
    }
}

impl StorageReadOwner for StoredItemValue {
    fn as_bytes(&self) -> &[u8] {
        self.as_ref()
    }
}
