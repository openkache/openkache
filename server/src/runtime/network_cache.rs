//! Network-worker view of the storage runtime.
//!
//! This adapter carries requester identity for telemetry and exposes both the
//! neutral keyed cache calls through one network-owned handle. Worker
//! lifecycle stays in sibling runtime modules; generic address hashing lives
//! here at the adapter boundary.

use std::future::Future;
use std::sync::Arc;

use crate::observability::{NetworkWorkerId, Operation};
use crate::types::{StoredItemBytes, StoredItemValue};
use crate::{KvError, Result, SetOutcome, StorageKey};

use super::RequestAdmissionError;
use super::keyed_storage;
use super::storage_keys;
use super::storage_port::{
    PreparedStorageAddress, StorageError, StorageMutation, StorageReadOwnerPool, StorageReadValue,
    StorageResult, StorageRoute, StorageScope, StorageValue, StorageWriteOptions,
    StorageWriteOutcome,
};
use super::{ThreadedKvkache, WorkerControlRequest, WorkerRequest, WorkerResponse};

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

pub(crate) fn into_storage_read_value_with_pool(
    value: StoredItemValue,
    owner_pool: &StorageReadOwnerPool,
) -> StorageResult<StorageReadValue> {
    match value.bytes {
        StoredItemBytes::DirectRead { buffer, range } => owner_pool
            .try_retain(StoredItemBytes::DirectRead { buffer, range })
            .map_err(|_| {
                StorageError::Overloaded("stable storage-read owner pool is exhausted".into())
            }),
        value => Ok(into_storage_read_value(StoredItemValue { bytes: value })),
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

impl From<RequestAdmissionError> for StorageError {
    fn from(error: RequestAdmissionError) -> Self {
        match error {
            RequestAdmissionError::QueueFull | RequestAdmissionError::CompletionFull => {
                Self::Overloaded(error.to_string())
            }
            RequestAdmissionError::Disconnected => Self::Worker(error.to_string()),
        }
    }
}

/// A network-worker-owned view of the storage runtime and its request shard.
#[derive(Clone)]
pub(crate) struct NetworkWorkerCache {
    cache: Arc<ThreadedKvkache>,
    network_worker: NetworkWorkerId,
    stable_owner_pools: Arc<[StorageReadOwnerPool]>,
}

impl NetworkWorkerCache {
    pub(crate) fn new(cache: Arc<ThreadedKvkache>, network_worker: NetworkWorkerId) -> Self {
        Self {
            stable_owner_pools: Arc::clone(&cache.stable_owner_pools),
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

    pub(crate) fn storage_key_for_item_id(
        &self,
        item_id: crate::openkache_protocol::ItemId,
    ) -> StorageKey {
        self.cache.storage_key_for_item_id(item_id)
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

    pub(crate) async fn experimental_stats(&self, operation: Operation) -> Result<Vec<String>> {
        let mut stats = Vec::with_capacity(self.cache.worker_count());
        for worker in 0..self.cache.worker_count() {
            let response = self
                .cache
                .try_network_request(worker, operation, self.network_worker, |response| {
                    WorkerRequest::Control(WorkerControlRequest::ExperimentalStats { response })
                })?
                .await?;
            match response {
                WorkerResponse::Control(
                    super::WorkerControlResponse::ExperimentalStats(worker_stats),
                ) => {
                    stats.push(format!("thread={worker} {worker_stats}"));
                }
                response => {
                    return Err(KvError::Worker(format!(
                        "unexpected experimental_stats response: {response:?}"
                    )));
                }
            }
        }
        Ok(stats)
    }

    pub(crate) async fn experimental_sync(&self, operation: Operation) -> Result<()> {
        let workers = 0..self.cache.worker_count();
        for worker in workers {
            self.sync_worker(worker, operation).await?;
        }
        Ok(())
    }

    async fn sync_worker(&self, worker: usize, operation: Operation) -> Result<()> {
        if worker >= self.cache.worker_count() {
            return Err(KvError::Worker(format!("unknown storage worker {worker}")));
        }
        match self
            .cache
            .try_network_request(worker, operation, self.network_worker, move |response| {
                WorkerRequest::Control(WorkerControlRequest::ExperimentalSync { response })
            })?
            .await?
        {
            WorkerResponse::Control(super::WorkerControlResponse::ExperimentalSynced) => Ok(()),
            response => Err(KvError::Worker(format!(
                "unexpected experimental_sync response: {response:?}"
            ))),
        }
    }

    pub(crate) async fn sync_routes(
        &self,
        routes: &[StorageRoute],
        operation: Operation,
    ) -> Result<()> {
        for route in routes {
            self.sync_worker(route.worker(), operation).await?;
        }
        Ok(())
    }

    /// Retrieves a value through the neutral storage adapter.
    pub(crate) fn storage_get(
        &self,
        operation: Operation,
        prepared: PreparedStorageAddress,
    ) -> impl Future<Output = StorageResult<Option<StorageReadValue>>> + '_ {
        let worker = prepared.route().worker();
        let storage_key = StorageKey::new(*prepared.as_bytes());
        let pending = self.cache.try_network_request(
            worker,
            operation,
            self.network_worker,
            move |response| WorkerRequest::Keyed {
                storage_key,
                command: keyed_storage::get(operation, response),
            },
        );
        async move {
            let value = pending
                .map_err(StorageError::from)?
                .await
                .and_then(|response| keyed_storage::value_response(response, "storage get"))
                .map_err(StorageError::from)?;
            value
                .map(|value| {
                    into_storage_read_value_with_pool(value, &self.stable_owner_pools[worker])
                })
                .transpose()
        }
    }

    /// Stores a value through the neutral storage adapter.
    pub(crate) fn storage_set(
        &self,
        operation: Operation,
        prepared: PreparedStorageAddress,
        value: StorageValue,
        options: StorageWriteOptions,
    ) -> impl Future<Output = StorageResult<StorageWriteOutcome>> + '_ {
        let worker = prepared.route().worker();
        let storage_key = StorageKey::new(*prepared.as_bytes());
        let value = StoredItemValue::from_owned_range(value.into_owned_range());
        let pending = self.cache.try_network_request(
            worker,
            operation,
            self.network_worker,
            move |response| WorkerRequest::Keyed {
                storage_key,
                command: keyed_storage::set(operation, value, options, response),
            },
        );
        async move {
            pending
                .map_err(StorageError::from)?
                .await
                .and_then(|response| keyed_storage::set_response(response, "storage set"))
                .map(|outcome| match outcome {
                    SetOutcome::Created => StorageWriteOutcome::Created,
                    SetOutcome::Replaced => StorageWriteOutcome::Replaced,
                    SetOutcome::NotStored => StorageWriteOutcome::Unchanged,
                })
                .map_err(StorageError::from)
        }
    }

    /// Deletes a value through the neutral storage adapter.
    pub(crate) fn storage_delete(
        &self,
        operation: Operation,
        prepared: PreparedStorageAddress,
    ) -> impl Future<Output = StorageResult<StorageMutation>> + '_ {
        let worker = prepared.route().worker();
        let storage_key = StorageKey::new(*prepared.as_bytes());
        let pending = self.cache.try_network_request(
            worker,
            operation,
            self.network_worker,
            move |response| WorkerRequest::Keyed {
                storage_key,
                command: keyed_storage::delete(operation, response),
            },
        );
        async move {
            pending
                .map_err(StorageError::from)?
                .await
                .and_then(|response| keyed_storage::delete_response(response, "storage delete"))
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

    /// Compares and conditionally exchanges one value through the keyed
    /// worker. The command is deliberately non-collapsible so its comparison
    /// and mutation remain one serialized storage action.
    pub(crate) fn storage_compare_exchange(
        &self,
        operation: Operation,
        prepared: PreparedStorageAddress,
        expected: Option<StorageValue>,
        replacement: Option<StorageValue>,
        options: StorageWriteOptions,
    ) -> impl Future<Output = StorageResult<StorageMutation>> + '_ {
        let worker = prepared.route().worker();
        let storage_key = StorageKey::new(*prepared.as_bytes());
        let expected =
            expected.map(|value| StoredItemValue::from_owned_range(value.into_owned_range()));
        let replacement =
            replacement.map(|value| StoredItemValue::from_owned_range(value.into_owned_range()));
        let pending = self.cache.try_network_request(
            worker,
            operation,
            self.network_worker,
            move |response| WorkerRequest::Keyed {
                storage_key,
                command: keyed_storage::compare_exchange(
                    operation,
                    expected,
                    replacement,
                    options,
                    response,
                ),
            },
        );
        async move {
            pending
                .map_err(StorageError::from)?
                .await
                .and_then(|response| {
                    keyed_storage::compare_exchange_response(response, "storage compare exchange")
                })
                .map(|changed| {
                    if changed {
                        StorageMutation::Applied
                    } else {
                        StorageMutation::Unchanged
                    }
                })
                .map_err(StorageError::from)
        }
    }

    pub(crate) fn prepare_address(
        &self,
        scope: StorageScope<'_>,
        identity: &[u8],
    ) -> PreparedStorageAddress {
        let key = storage_keys::derive_scoped_storage_key(
            &self.cache.storage_domain_key,
            scope.as_bytes(),
            identity,
        );
        let route = StorageRoute::from_worker(self.worker_for(&key));
        PreparedStorageAddress::new(key.into_bytes(), route)
    }

    pub(crate) fn prepare_storage_key(&self, storage_key: StorageKey) -> PreparedStorageAddress {
        let route = StorageRoute::from_worker(self.worker_for(&storage_key));
        PreparedStorageAddress::new(storage_key.into_bytes(), route)
    }

    pub(crate) fn storage_route_for(&self, prepared: &PreparedStorageAddress) -> StorageRoute {
        prepared.route()
    }
}
