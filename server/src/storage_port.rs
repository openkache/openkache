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
    PreparedStorageAddress, StorageReadBytes, StorageReadOwner, StorageReadValue, StorageResult,
    StorageRoute, StorageScope, StorageValue, StorageWriteOptions, StorageWriteOutcome,
    StorageError, StorageMutation,
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

    /// Retrieves a value using the caller's operation attribution.
    #[allow(dead_code)]
    pub(crate) async fn get(
        &self,
        operation: Operation,
        address: PreparedStorageAddress,
    ) -> StorageResult<Option<StorageReadValue>> {
        self.backend.storage_get(operation, address).await
    }

    /// Stores a value using the caller's operation attribution.
    #[allow(dead_code)]
    pub(crate) async fn set(
        &self,
        operation: Operation,
        address: PreparedStorageAddress,
        value: StorageValue,
        options: StorageWriteOptions,
    ) -> StorageResult<StorageWriteOutcome> {
        self.backend
            .storage_set(operation, address, value, options)
            .await
    }

    /// Deletes a value using the caller's operation attribution.
    #[allow(dead_code)]
    pub(crate) async fn delete(
        &self,
        operation: Operation,
        address: PreparedStorageAddress,
    ) -> StorageResult<StorageMutation> {
        self.backend.storage_delete(operation, address).await
    }
}

/// Statically dispatched storage data plane.
///
/// Production uses [`StoragePort`] directly. Alternate backends retain exact
/// future types, avoiding request-path type erasure and heap allocation.
#[allow(dead_code)]
pub(crate) trait StorageDataPort: Send + Sync {
    fn max_item_bytes(&self) -> usize;

    fn prepare_address(
        &self,
        scope: StorageScope,
        identity: &[u8],
    ) -> PreparedStorageAddress;

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
}

/// Compatibility-only address preparation preserving the published v2 key.
///
/// Generic APIs use [`StorageDataPort::prepare_address`]. This adapter keeps
/// the legacy namespace key derivation outside the common port contract.
#[allow(dead_code)]
pub(crate) trait CompatibilityStorageAddressPort: StorageDataPort {
    fn prepare_compatibility_address(
        &self,
        storage_domain_id: u64,
        identity: &[u8; super::super::types::STORAGE_KEY_BYTES],
    ) -> PreparedStorageAddress {
        self.prepare_address(
            StorageScope::from_owned(storage_domain_id.to_be_bytes().to_vec()),
            identity,
        )
    }
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

    fn prepare_address(
        &self,
        scope: StorageScope,
        identity: &[u8],
    ) -> PreparedStorageAddress {
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
}

impl CompatibilityStorageAddressPort for StoragePort {
    fn prepare_compatibility_address(
        &self,
        storage_domain_id: u64,
        identity: &[u8; super::super::types::STORAGE_KEY_BYTES],
    ) -> PreparedStorageAddress {
        self.backend
            .prepare_compatibility_address(storage_domain_id, identity)
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
