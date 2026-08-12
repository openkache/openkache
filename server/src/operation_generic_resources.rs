//! API-owned resources used by the route-less example operations.
//!
//! The executor sees only opaque [`ResourceLock`] handles and capability
//! values. Resource identity, storage, and mutation semantics remain local to
//! this module; adding another API does not require a new executor branch.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::lock::Mutex as AsyncMutex;

use super::operation_api::{CapabilityKey, ResourceLock};
use super::operation_capabilities::{CapabilityCatalog, CapabilityRegistry};
use super::storage_port::{STORAGE_PORT, StoragePortHandle};

#[derive(Default)]
pub(crate) struct ExperimentalResourceStore {
    pub(crate) locks: Mutex<HashMap<Vec<u8>, Arc<AsyncMutex<()>>>>,
    pub(crate) values: Mutex<HashMap<Vec<u8>, Vec<u8>>>,
}

impl ExperimentalResourceStore {
    pub(crate) fn resource_lock(
        &self,
        identity: &[u8],
    ) -> Result<ResourceLock, &'static [u8]> {
        let mut locks = self
            .locks
            .lock()
            .map_err(|_| b"experimental resource lock registry is unavailable".as_slice())?;
        let lock = Arc::clone(
            locks
                .entry(identity.to_vec())
                .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
        );
        Ok(ResourceLock::unconditional(lock))
    }

    pub(crate) fn mutate(
        &self,
        source: &[u8],
        target: &[u8],
        payload: &[u8],
    ) -> Result<Vec<u8>, &'static [u8]> {
        let mut values = self
            .values
            .lock()
            .map_err(|_| b"experimental resource store is unavailable".as_slice())?;
        values.insert(source.to_vec(), payload.to_vec());
        values.insert(target.to_vec(), payload.to_vec());
        Ok(u64::try_from(values.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes()
            .to_vec())
    }
}

pub(crate) const EXPERIMENTAL_RESOURCE_STORE: CapabilityKey<ExperimentalResourceStore> =
    CapabilityKey::new("openkache.experimental.resource_store");

pub(crate) fn install_capabilities(
    registry: &mut CapabilityRegistry,
    source: &dyn CapabilityCatalog,
) {
    if let Some(storage) = super::operation_api::downcast_capability(source, STORAGE_PORT) {
        install_storage_port(registry, storage.clone());
    }
    install_resource_store(registry);
}

/// Installs the runtime-neutral storage port used by this API module.
///
/// The network loop only forwards the already-created port through the API
/// composition boundary. It does not need to know which generic operation
/// consumes storage, and a future API can expose a different capability
/// without changing the transport path.
pub(crate) fn install_storage_port(
    registry: &mut CapabilityRegistry,
    storage: StoragePortHandle,
) {
    registry.insert(STORAGE_PORT, storage);
}

pub(crate) fn install_resource_store(registry: &mut CapabilityRegistry) {
    registry.insert(
        EXPERIMENTAL_RESOURCE_STORE,
        ExperimentalResourceStore::default(),
    );
}
