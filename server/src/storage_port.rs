//! API-facing storage capability.
//!
//! Operation registration and capability lookup depend on this narrow module,
//! not on the runtime implementation module. The concrete facade keeps worker
//! details private while inherent async methods avoid per-call erased futures.

use std::sync::Arc;

use super::operation_capabilities::CapabilityKey;
use super::NetworkWorkerCache;
use super::super::observability::Operation;

#[allow(unused_imports)]
pub(crate) use super::super::types::{
    StorageWriteCondition, StorageWriteEviction, StorageWriteExpiration,
};
#[allow(unused_imports)]
pub(crate) use super::super::runtime::{
    StorageAddress, StorageError, StorageMutation, StorageReadOwner, StorageReadValue,
    StorageResult, StorageValue, StorageWriteOptions, StorageWriteOutcome,
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
        address: StorageAddress,
    ) -> StorageResult<Option<StorageReadValue>> {
        self.backend.storage_get(operation, address).await
    }

    /// Stores a value using the caller's operation attribution.
    #[allow(dead_code)]
    pub(crate) async fn set(
        &self,
        operation: Operation,
        address: StorageAddress,
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
        address: StorageAddress,
    ) -> StorageResult<StorageMutation> {
        self.backend.storage_delete(operation, address).await
    }
}
