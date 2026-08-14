//! Runtime storage-key derivation.
//!
//! Keeping the fixed-width engine identity and namespace scoping here leaves
//! worker lifecycle and request routing focused on scheduling. API-facing
//! storage addresses are normalized by a separate backend adapter.

use crate::StorageKey;
use crate::protocol::ItemId;

pub(crate) const DOMAIN_V2_CONTEXT: &str = "OpenKache StorageKey DomainV2 root";
pub(crate) const INTERNAL_NAMESPACE_ID: u64 = 0;
const STORAGE_KEY_DIGEST_START: usize = 8;

pub(crate) fn derive_domain_key(server_secret: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key(DOMAIN_V2_CONTEXT, server_secret)
}

pub(crate) fn derive_storage_key(domain_key: &[u8; 32], item_id: ItemId) -> StorageKey {
    derive_scoped_storage_key(domain_key, INTERNAL_NAMESPACE_ID, item_id)
}

/// Derives a storage key for the `(namespace_id, item_id)` wire identity.
///
/// The namespace prefix remains recoverable while the keyed digest binds the
/// complete wire identity without allocating a concatenated preimage.
pub(crate) fn derive_scoped_storage_key(
    domain_key: &[u8; 32],
    namespace_id: u64,
    item_id: ItemId,
) -> StorageKey {
    let namespace_bytes = namespace_id.to_be_bytes();
    let mut hasher = blake3::Hasher::new_keyed(domain_key);
    hasher.update(&namespace_bytes);
    hasher.update(item_id.as_ref());
    let digest = hasher.finalize();

    let mut bytes = [0; crate::types::STORAGE_KEY_BYTES];
    bytes[..STORAGE_KEY_DIGEST_START].copy_from_slice(&namespace_bytes);
    bytes[STORAGE_KEY_DIGEST_START..]
        .copy_from_slice(&digest.as_bytes()[..crate::types::STORAGE_KEY_BYTES - 8]);
    StorageKey::new(bytes)
}
