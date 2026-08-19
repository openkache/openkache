//! Runtime storage-key derivation.
//!
//! Keeping fixed-width engine identity and storage-domain scoping here leaves
//! worker lifecycle and request routing focused on scheduling. API adapters
//! normalize their own identity models before this boundary.

use crate::types::StorageKey;

pub(crate) const DOMAIN_V2_CONTEXT: &str = "OpenKache StorageKey DomainV2 root";
pub(crate) const INTERNAL_STORAGE_DOMAIN_ID: u64 = 0;
pub(crate) const SCOPED_STORAGE_ADDRESS_TAG: &[u8] = b"\xffSK\x01";
pub(crate) const ITEM_ID_STORAGE_SCOPE: &[u8] = b"OpenKache Item ID v1";
const STORAGE_KEY_DIGEST_START: usize = 8;

pub(crate) fn derive_domain_key(server_secret: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key(DOMAIN_V2_CONTEXT, server_secret)
}

pub(crate) fn derive_storage_key(
    domain_key: &[u8; 32],
    identity: &[u8; crate::types::STORAGE_KEY_BYTES],
) -> StorageKey {
    let domain_bytes = INTERNAL_STORAGE_DOMAIN_ID.to_be_bytes();
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

/// Derives one fixed-width key from an unambiguous opaque scope/identity tuple.
///
/// The inputs are streamed directly into the keyed hash so callers do not
/// need to allocate a joined identity. Length prefixes keep arbitrary byte
/// pairs distinct without exposing an API-specific identifier shape.
pub(crate) fn derive_scoped_storage_key(
    domain_key: &[u8; 32],
    scope: &[u8],
    identity: &[u8],
) -> StorageKey {
    let mut hasher = blake3::Hasher::new_keyed(domain_key);
    hasher.update(SCOPED_STORAGE_ADDRESS_TAG);
    hasher.update(&(scope.len() as u64).to_be_bytes());
    hasher.update(scope);
    hasher.update(&(identity.len() as u64).to_be_bytes());
    hasher.update(identity);
    StorageKey::new(*hasher.finalize().as_bytes())
}
