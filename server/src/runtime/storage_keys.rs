//! Runtime storage-key derivation.
//!
//! Keeping the fixed-width engine identity and namespace scoping here leaves
//! worker lifecycle and request routing focused on scheduling. API-facing
//! storage addresses are normalized by a separate backend adapter.

use aes::{
    Aes256,
    cipher::{Block, BlockCipherEncrypt, KeyInit},
};
use openkache_protocol::NAMESPACE_ID_BYTES;

use crate::StorageKey;
use crate::protocol::ItemId;

pub(crate) fn derive_storage_key(server_cipher: &Aes256, item_id: ItemId) -> StorageKey {
    // Compact v1 currently supplies one storage-width identity. Keep that
    // compatibility requirement explicit here instead of deriving the
    // persisted key width from the wire model.
    let mut bytes: [u8; crate::types::STORAGE_KEY_BYTES] = item_id.into_bytes();

    // SAFETY: `Block<Aes256>` is layout-identical to `[u8; 16]`, so two blocks
    // exactly cover the 32-byte digest buffer while preserving its alignment
    // and exclusive borrow.
    let blocks = unsafe { &mut *(bytes.as_mut_ptr() as *mut [Block<Aes256>; 2]) };

    // AES-MDS-AES keeps this fixed-size derivation in place and on the AES
    // hardware path: two parallel AES passes surround an invertible MDS layer
    // so each digest half influences both output blocks.
    server_cipher.encrypt_blocks(blocks);

    let [first_block, second_block] = blocks;
    for (first, second) in first_block.iter_mut().zip(second_block.iter_mut()) {
        let first_byte = *first;
        let second_byte = *second;
        *first = first_byte ^ second_byte;
        *second = first_byte ^ gf_double(second_byte);
    }

    server_cipher.encrypt_blocks(blocks);
    StorageKey::new(bytes)
}

/// Derives a storage key for the `(namespace_id, item_id)` wire identity.
///
/// A namespace-specific AES key keeps equal item IDs in two namespaces
/// distinct without exposing namespace semantics to the worker scheduler.
pub(crate) fn derive_scoped_storage_key(
    server_cipher: &Aes256,
    namespace_id: u64,
    item_id: ItemId,
) -> StorageKey {
    let mut scope_material = [0u8; 32];
    scope_material[..NAMESPACE_ID_BYTES].copy_from_slice(&namespace_id.to_be_bytes());
    scope_material[NAMESPACE_ID_BYTES..].copy_from_slice(b"OpenKache namespace v1!!");
    let namespace_key = derive_storage_key(server_cipher, ItemId::new(scope_material)).into_bytes();
    let namespace_cipher = Aes256::new_from_slice(&namespace_key)
        .expect("AES-256 namespace derivation always produces a 32-byte key");
    derive_storage_key(&namespace_cipher, item_id)
}

fn gf_double(byte: u8) -> u8 {
    (byte << 1) ^ (0x1b & 0u8.wrapping_sub(byte >> 7))
}
