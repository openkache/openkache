//! Runtime storage-key derivation.
//!
//! Keeping fixed-width engine identity and storage-domain scoping here leaves
//! worker lifecycle and request routing focused on scheduling. API adapters
//! normalize their own identity models before this boundary.

use crate::StorageKey;

pub(crate) const DOMAIN_V2_CONTEXT: &str = "OpenKache StorageKey DomainV2 root";
pub(crate) const INTERNAL_STORAGE_DOMAIN_ID: u64 = 0;
const STORAGE_KEY_DIGEST_START: usize = 8;

pub(crate) fn derive_domain_key(server_secret: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key(DOMAIN_V2_CONTEXT, server_secret)
}

pub(crate) fn derive_storage_key(
    domain_key: &[u8; 32],
    identity: &[u8; crate::types::STORAGE_KEY_BYTES],
) -> StorageKey {
    derive_storage_key_for_domain(domain_key, INTERNAL_STORAGE_DOMAIN_ID, identity)
}

/// Derives a storage key for one opaque identity within a storage domain.
///
/// The domain prefix remains recoverable while the keyed digest binds the
/// complete identity without allocating a concatenated preimage.
pub(crate) fn derive_storage_key_for_domain(
    domain_key: &[u8; 32],
    storage_domain_id: u64,
    identity: &[u8; crate::types::STORAGE_KEY_BYTES],
) -> StorageKey {
    let domain_bytes = storage_domain_id.to_be_bytes();
    let mut hasher = blake3::Hasher::new_keyed(domain_key);
    hasher.update(&domain_bytes);
    hasher.update(identity);
    let digest = hasher.finalize();

    let mut bytes = [0; crate::types::STORAGE_KEY_BYTES];
    bytes[..STORAGE_KEY_DIGEST_START].copy_from_slice(&domain_bytes);
    bytes[STORAGE_KEY_DIGEST_START..]
        .copy_from_slice(&digest.as_bytes()[..crate::types::STORAGE_KEY_BYTES - 8]);
    StorageKey::new(bytes)
}
