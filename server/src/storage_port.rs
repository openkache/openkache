//! API-facing storage capability bridge.
//!
//! Operation registration and capability lookup depend on this narrow module,
//! not on the runtime implementation module. The runtime-backed handle is
//! re-exported here only at the composition boundary; replacing the worker
//! implementation therefore does not change the generic operation catalog.

use std::sync::Arc;

use super::operation_api::CapabilityKey;
use super::operation_capabilities::CapabilityRegistry;

#[allow(unused_imports)]
pub(crate) use super::super::runtime::{
    StorageAddress, StorageBatchOperation, StorageBatchResult, StorageContext,
    StorageContextFuture, StorageError, StorageMutation, StorageMutationFuture, StoragePort,
    StoragePortExt, StorageReadFuture, StorageResult, StorageTask, StorageTaskCancellation,
    StorageTaskFuture, StorageTaskIsolation, StorageTaskMetadata, StorageTaskOutput,
    StorageTaskScheduling, StorageTaskScope, StorageTypedTaskFuture, StorageValue,
    StorageWriteCondition, StorageWriteEviction, StorageWriteExpiration, StorageWriteFuture,
    StorageWriteOptions, StorageWriteOutcome, downcast_storage_output,
};

/// The generic capability identity used by API modules that need storage.
pub(super) const STORAGE_PORT: CapabilityKey<StoragePortHandle> =
    CapabilityKey::new("openkache.storage.port");

/// Keeps the handle type-erased at the capability boundary.
pub(crate) type StoragePortHandle = Arc<dyn StoragePort>;

/// Installs the neutral storage capability shared by API modules.
pub(super) fn install(registry: &mut CapabilityRegistry, storage: StoragePortHandle) {
    registry.insert(STORAGE_PORT, storage);
}
