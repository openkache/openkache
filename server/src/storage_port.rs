//! API-facing storage capability.
//!
//! Operation registration and capability lookup depend on this narrow module,
//! not on the runtime implementation module. The concrete facade keeps worker
//! details private while inherent async methods avoid per-call erased futures.

use std::future::Future;
use std::sync::Arc;

use super::NetworkWorkerCache;
use super::operation_capabilities::CapabilityKey;

use super::super::observability::Operation;
#[allow(unused_imports)]
pub(crate) use super::super::runtime::{
    PreparedStorageAddress, StorageError, StorageMutation, StorageReadBytes, StorageReadOwner,
    StorageReadValue, StorageResult, StorageRoute, StorageScope, StorageValue, StorageWriteOptions,
    StorageWriteOutcome,
};
#[allow(unused_imports)]
pub(crate) use super::super::types::{
    StorageWriteCondition, StorageWriteEviction, StorageWriteExpiration,
};

/// The generic capability identity used by API modules that need storage.
pub(super) const STORAGE_PORT: CapabilityKey<StoragePort> =
    CapabilityKey::new("openkache.storage.port");

/// Concrete API-facing facade over one network worker's storage requester.
#[derive(Clone)]
pub(crate) struct StoragePort {
    backend: Arc<NetworkWorkerCache>,
}

impl StoragePort {
    pub(super) fn new(backend: Arc<NetworkWorkerCache>) -> Self {
        Self { backend }
    }

    /// Submits a read and returns its allocation-free completion future.
    #[allow(dead_code)]
    pub(crate) fn get(
        &self,
        operation: Operation,
        address: PreparedStorageAddress,
    ) -> impl Future<Output = StorageResult<Option<StorageReadValue>>> + '_ {
        self.backend.storage_get(operation, address)
    }

    /// Moves a value into a write and returns its allocation-free completion future.
    #[allow(dead_code)]
    pub(crate) fn set(
        &self,
        operation: Operation,
        address: PreparedStorageAddress,
        value: StorageValue,
        options: StorageWriteOptions,
    ) -> impl Future<Output = StorageResult<StorageWriteOutcome>> + '_ {
        self.backend.storage_set(operation, address, value, options)
    }

    /// Submits a deletion and returns its allocation-free completion future.
    #[allow(dead_code)]
    pub(crate) fn delete(
        &self,
        operation: Operation,
        address: PreparedStorageAddress,
    ) -> impl Future<Output = StorageResult<StorageMutation>> + '_ {
        self.backend.storage_delete(operation, address)
    }

    /// Compares the current value and atomically exchanges it when it matches.
    ///
    /// `None` is the absence sentinel for either side. Values retain their
    /// caller-owned ranges until the keyed worker consumes them.
    #[allow(dead_code)]
    pub(crate) fn compare_exchange(
        &self,
        operation: Operation,
        address: PreparedStorageAddress,
        expected: Option<StorageValue>,
        replacement: Option<StorageValue>,
        options: StorageWriteOptions,
    ) -> impl Future<Output = StorageResult<StorageMutation>> + '_ {
        self.backend
            .storage_compare_exchange(operation, address, expected, replacement, options)
    }
}

/// Statically dispatched storage data plane.
///
/// Production uses [`StoragePort`] directly. Alternate backends retain exact
/// future types, avoiding request-path type erasure and heap allocation.
/// Calls submit work before returning; their futures only observe completion.
#[allow(dead_code)]
pub(crate) trait StorageDataPort: Send + Sync {
    fn max_item_bytes(&self) -> usize;

    fn prepare_address(&self, scope: StorageScope<'_>, identity: &[u8]) -> PreparedStorageAddress;

    fn route_for(&self, address: &PreparedStorageAddress) -> StorageRoute;

    fn get(
        &self,
        operation: Operation,
        storage_address: PreparedStorageAddress,
    ) -> impl Future<Output = StorageResult<Option<StorageReadValue>>> + '_;

    fn set(
        &self,
        operation: Operation,
        storage_address: PreparedStorageAddress,
        value: StorageValue,
        options: StorageWriteOptions,
    ) -> impl Future<Output = StorageResult<StorageWriteOutcome>> + '_;

    fn delete(
        &self,
        operation: Operation,
        storage_address: PreparedStorageAddress,
    ) -> impl Future<Output = StorageResult<StorageMutation>> + '_;

    fn compare_exchange(
        &self,
        operation: Operation,
        storage_address: PreparedStorageAddress,
        expected: Option<StorageValue>,
        replacement: Option<StorageValue>,
        options: StorageWriteOptions,
    ) -> impl Future<Output = StorageResult<StorageMutation>> + '_;
}

/// Statically dispatched storage administration plane.
#[allow(dead_code)]
pub(crate) trait StorageAdministrationPort: Send + Sync {
    fn stats(&self, operation: Operation) -> impl Future<Output = StorageResult<Vec<String>>> + '_;

    fn sync_routes<'a>(
        &'a self,
        routes: &'a [StorageRoute],
        operation: Operation,
    ) -> impl Future<Output = StorageResult<()>> + 'a;
}

impl StorageDataPort for StoragePort {
    fn max_item_bytes(&self) -> usize {
        self.backend.max_item_bytes()
    }

    fn prepare_address(&self, scope: StorageScope<'_>, identity: &[u8]) -> PreparedStorageAddress {
        self.backend.prepare_address(scope, identity)
    }

    fn route_for(&self, address: &PreparedStorageAddress) -> StorageRoute {
        self.backend.storage_route_for(address)
    }

    fn get(
        &self,
        operation: Operation,
        storage_address: PreparedStorageAddress,
    ) -> impl Future<Output = StorageResult<Option<StorageReadValue>>> + '_ {
        StoragePort::get(self, operation, storage_address)
    }

    fn set(
        &self,
        operation: Operation,
        storage_address: PreparedStorageAddress,
        value: StorageValue,
        options: StorageWriteOptions,
    ) -> impl Future<Output = StorageResult<StorageWriteOutcome>> + '_ {
        StoragePort::set(self, operation, storage_address, value, options)
    }

    fn delete(
        &self,
        operation: Operation,
        storage_address: PreparedStorageAddress,
    ) -> impl Future<Output = StorageResult<StorageMutation>> + '_ {
        StoragePort::delete(self, operation, storage_address)
    }

    fn compare_exchange(
        &self,
        operation: Operation,
        storage_address: PreparedStorageAddress,
        expected: Option<StorageValue>,
        replacement: Option<StorageValue>,
        options: StorageWriteOptions,
    ) -> impl Future<Output = StorageResult<StorageMutation>> + '_ {
        StoragePort::compare_exchange(
            self,
            operation,
            storage_address,
            expected,
            replacement,
            options,
        )
    }
}

impl StorageAdministrationPort for StoragePort {
    fn stats(&self, operation: Operation) -> impl Future<Output = StorageResult<Vec<String>>> + '_ {
        async move {
            self.backend
                .stats(operation)
                .await
                .map_err(StorageError::from)
        }
    }

    fn sync_routes<'a>(
        &'a self,
        routes: &'a [StorageRoute],
        operation: Operation,
    ) -> impl Future<Output = StorageResult<()>> + 'a {
        async move {
            self.backend
                .sync_routes(routes, operation)
                .await
                .map_err(StorageError::from)
        }
    }
}
