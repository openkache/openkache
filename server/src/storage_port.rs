//! API-facing storage capability bridge.
//!
//! Operation registration and capability lookup depend on this narrow module,
//! not on the runtime implementation module. The runtime-backed handle is
//! re-exported here only at the composition boundary; replacing the worker
//! implementation therefore does not change the generic operation catalog.

use std::sync::Arc;

use super::operation_capabilities::CapabilityKey;

#[allow(unused_imports)]
pub(crate) use super::super::types::{
    StorageWriteCondition, StorageWriteEviction, StorageWriteExpiration,
};
#[allow(unused_imports)]
pub(crate) use super::super::runtime::{
    StorageAddress, StorageError, StorageMutation, StorageMutationFuture, StoragePort,
    StorageReadFuture, StorageReadOwner, StorageReadValue, StorageResult, StorageValue,
    StorageWriteFuture, StorageWriteOptions, StorageWriteOutcome,
};

/// The generic capability identity used by API modules that need storage.
pub(super) const STORAGE_PORT: CapabilityKey<StoragePortHandle> =
    CapabilityKey::new("openkache.storage.port");

/// Keeps the handle type-erased at the capability boundary.
pub(crate) type StoragePortHandle = Arc<dyn StoragePort>;
