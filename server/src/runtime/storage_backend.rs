//! Concrete cache adapter for the runtime-neutral storage task context.
//!
//! API-owned tasks use the byte-oriented [`super::storage_context::StorageBackend`]
//! contract. This module is the composition boundary that translates that
//! contract into the cache engine's fixed-width key and protocol-facing write
//! policy. Keeping the translation here prevents generic storage code from
//! depending on wire types.

use crate::protocol::{EvictionMode, ExpirationMode, SetCondition, SetOptions};
use crate::types::StoredItemValue;
use crate::{KvError, Kvkache, SetOutcome, StorageKey};
use sha2::{Digest, Sha256};

use super::storage_context;
use super::storage_port::{
    StorageAddress, StorageContextFuture, StorageError, StorageMutation, StorageWriteCondition,
    StorageWriteEviction, StorageWriteExpiration, StorageWriteOptions,
};

/// Domain separator for addresses submitted through the generic storage port.
///
/// Generic API addresses intentionally occupy a cryptographically separated
/// namespace from protocol-v1 item identities. The storage engine still uses
/// one fixed-width [`StorageKey`], so the adapter hashes every address rather
/// than treating a 32-byte application key as an engine key by accident.
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

/// Converts an API-owned opaque address at the runtime boundary.
///
/// Routing and worker-local storage still use the fixed-width engine key. The
/// conversion stays in this adapter so generic API code does not need to know
/// how addresses are normalized or which namespace the engine uses.
pub(crate) fn storage_key_for_address(address: &StorageAddress) -> StorageKey {
    let mut hasher = Sha256::new();
    hasher.update(GENERIC_STORAGE_ADDRESS_DOMAIN);
    hasher.update(address.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; crate::types::STORAGE_KEY_BYTES];
    bytes.copy_from_slice(&digest[..crate::types::STORAGE_KEY_BYTES]);
    StorageKey::new(bytes)
}

/// Concrete worker-local backend for the runtime-neutral storage task context.
pub(super) struct RuntimeStorageBackend<'a> {
    cache: &'a mut Kvkache,
}

impl<'a> RuntimeStorageBackend<'a> {
    pub(super) const fn new(cache: &'a mut Kvkache) -> Self {
        Self { cache }
    }
}

#[allow(dead_code)]
pub(super) fn protocol_storage_options(options: StorageWriteOptions) -> SetOptions {
    let condition = match options.condition {
        StorageWriteCondition::Any => SetCondition::Any,
        StorageWriteCondition::IfAbsent => SetCondition::IfAbsent,
        StorageWriteCondition::IfPresent => SetCondition::IfPresent,
    };
    let (expiration_mode, ttl_ms) = match options.expiration {
        StorageWriteExpiration::Inherit => (ExpirationMode::Inherit, None),
        StorageWriteExpiration::NoExpiry => (ExpirationMode::NoExpiry, None),
        StorageWriteExpiration::Ttl(ttl_ms) => (ExpirationMode::ExplicitTtl, Some(ttl_ms)),
    };
    let eviction_mode = match options.eviction {
        StorageWriteEviction::Inherit => EvictionMode::Inherit,
        StorageWriteEviction::Evictable => EvictionMode::Evictable,
        StorageWriteEviction::Protected => EvictionMode::EvictionProtected,
    };
    SetOptions::with_policies(condition, expiration_mode, ttl_ms, eviction_mode)
}

impl storage_context::StorageBackend for RuntimeStorageBackend<'_> {
    fn get<'a>(
        &'a mut self,
        storage_address: StorageAddress,
    ) -> StorageContextFuture<'a, Option<Vec<u8>>> {
        let storage_key = storage_key_for_address(&storage_address);
        Box::pin(async move {
            self.cache
                .get_encoded(&storage_key)
                .await
                .map(|value| value.map(StoredItemValue::into_bytes))
                .map_err(StorageError::from)
        })
    }

    fn set<'a>(
        &'a mut self,
        storage_address: StorageAddress,
        value: Vec<u8>,
        options: StorageWriteOptions,
    ) -> StorageContextFuture<'a, StorageMutation> {
        let storage_key = storage_key_for_address(&storage_address);
        Box::pin(async move {
            self.cache
                .set_encoded_with_options(
                    storage_key,
                    StoredItemValue::new(value),
                    protocol_storage_options(options),
                )
                .await
                .map(|outcome| match outcome {
                    SetOutcome::Created | SetOutcome::Replaced => StorageMutation::Applied,
                    SetOutcome::NotStored => StorageMutation::Unchanged,
                })
                .map_err(StorageError::from)
        })
    }

    fn delete<'a>(
        &'a mut self,
        storage_address: StorageAddress,
    ) -> StorageContextFuture<'a, StorageMutation> {
        let storage_key = storage_key_for_address(&storage_address);
        Box::pin(async move {
            self.cache
                .delete(&storage_key)
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

    fn compare_and_set<'a>(
        &'a mut self,
        storage_address: StorageAddress,
        expected: Option<&'a [u8]>,
        replacement: Option<Vec<u8>>,
        options: StorageWriteOptions,
    ) -> StorageContextFuture<'a, bool> {
        let storage_key = storage_key_for_address(&storage_address);
        Box::pin(async move {
            self.cache
                .compare_and_set_encoded_with_options(
                    storage_key,
                    expected,
                    replacement.map(StoredItemValue::new),
                    protocol_storage_options(options),
                )
                .await
                .map_err(StorageError::from)
        })
    }
}
