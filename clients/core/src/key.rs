//! Fixed-size item IDs and reusable keyed derivation helpers.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use std::collections::VecDeque;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{Error, ITEM_ID_BYTES, MutationId, Result};

pub(crate) const PROTECTION_KEY_BYTES: usize =
    crate::contract::VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES;

/// Bytes in an application-managed data protection key.
pub const DATA_PROTECTION_KEY_BYTES: usize = PROTECTION_KEY_BYTES;
/// Maximum number of retired keys retained for a rotation window.
pub const MAX_PREVIOUS_DATA_PROTECTION_KEYS: usize = 8;

pub(crate) fn random_mutation_id() -> Result<MutationId> {
    let mut bytes = [0; openkache_protocol::MUTATION_ID_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| Error::Io(error.to_string()))?;
    Ok(MutationId::new(bytes))
}

/// Derives a stable physical-request token from one logical mutation token and item ID.
///
/// Protected key rotation can probe more than one derived item ID for a single logical
/// delete. Each probe is a distinct wire request, so it needs a distinct token to avoid
/// being rejected as a mutation conflict while still being replayable on a caller retry.
pub(crate) fn scoped_mutation_id(mutation_id: MutationId, item_id: ItemId) -> MutationId {
    let mut input = [0_u8; openkache_protocol::MUTATION_ID_BYTES + ITEM_ID_BYTES];
    input[..openkache_protocol::MUTATION_ID_BYTES].copy_from_slice(mutation_id.as_bytes());
    input[openkache_protocol::MUTATION_ID_BYTES..].copy_from_slice(item_id.as_bytes());
    let digest = blake3::hash(&input);
    let mut bytes = [0_u8; openkache_protocol::MUTATION_ID_BYTES];
    bytes.copy_from_slice(&digest.as_bytes()[..openkache_protocol::MUTATION_ID_BYTES]);
    MutationId::new(bytes)
}

/// Exact fixed-size item ID sent through the OpenKache protocol.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ItemId([u8; ITEM_ID_BYTES]);

impl ItemId {
    /// Wraps an exact item ID without hashing it again.
    pub const fn from_bytes(bytes: [u8; ITEM_ID_BYTES]) -> Self {
        Self(bytes)
    }

    /// Copies an exact item ID from a language binding or dynamic buffer.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Exactly 32 opaque item ID bytes.
    ///
    /// # Returns
    ///
    /// An item ID that preserves the supplied bytes without hashing.
    ///
    /// # Errors
    ///
    /// Returns an error when `bytes` does not contain exactly 32 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let exact: &[u8; ITEM_ID_BYTES] = bytes.try_into().map_err(|_| {
            Error::configuration(
                "item_id",
                format!(
                    "must contain exactly {ITEM_ID_BYTES} bytes, got {}",
                    bytes.len()
                ),
            )
        })?;
        Ok(Self::from_bytes(*exact))
    }

    /// Returns the exact wire bytes.
    pub const fn as_bytes(&self) -> &[u8; ITEM_ID_BYTES] {
        &self.0
    }

    /// Consumes the item ID and returns its exact wire bytes.
    pub const fn into_bytes(self) -> [u8; ITEM_ID_BYTES] {
        self.0
    }

    pub(crate) const fn into_protocol(self) -> openkache_protocol::ItemId {
        openkache_protocol::ItemId::new(self.0)
    }
}

impl AsRef<[u8]> for ItemId {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<[u8; ITEM_ID_BYTES]> for ItemId {
    fn from(bytes: [u8; ITEM_ID_BYTES]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl TryFrom<&[u8]> for ItemId {
    type Error = Error;

    fn try_from(bytes: &[u8]) -> Result<Self> {
        Self::from_slice(bytes)
    }
}

/// Application-managed master secret used to hide keys and encrypt values.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DataProtectionKey {
    master_key: [u8; DATA_PROTECTION_KEY_BYTES],
    item_id_root: [u8; DATA_PROTECTION_KEY_BYTES],
    value_root_key: [u8; DATA_PROTECTION_KEY_BYTES],
}

/// Active data-protection key plus a bounded read/delete rotation window.
pub struct DataProtectionKeyRing {
    active: DataProtectionKey,
    previous: VecDeque<DataProtectionKey>,
}

impl DataProtectionKeyRing {
    /// Creates a ring with one active key and no retired keys.
    pub fn new(active: DataProtectionKey) -> Self {
        Self {
            active,
            previous: VecDeque::new(),
        }
    }

    /// Creates a ring from an active key and up to eight previous keys.
    pub fn with_previous(
        active: DataProtectionKey,
        previous: impl IntoIterator<Item = DataProtectionKey>,
    ) -> Result<Self> {
        let previous = previous.into_iter().collect::<VecDeque<_>>();
        if previous.len() > MAX_PREVIOUS_DATA_PROTECTION_KEYS {
            return Err(Error::configuration(
                "data_protection_key_ring",
                format!("retains at most {MAX_PREVIOUS_DATA_PROTECTION_KEYS} previous keys"),
            ));
        }
        Ok(Self { active, previous })
    }

    /// Returns the active key without exposing its bytes.
    pub fn active(&self) -> &DataProtectionKey {
        &self.active
    }

    /// Returns retired keys in newest-to-oldest order.
    pub fn previous(&self) -> impl Iterator<Item = &DataProtectionKey> {
        self.previous.iter()
    }

    /// Returns the number of retired keys retained for reads and deletes.
    pub fn previous_len(&self) -> usize {
        self.previous.len()
    }

    /// Promotes a new key and retains the former active key at the front of the window.
    pub fn rotate(&mut self, next_active: DataProtectionKey) {
        let old_active = std::mem::replace(&mut self.active, next_active);
        self.previous.push_front(old_active);
        while self.previous.len() > MAX_PREVIOUS_DATA_PROTECTION_KEYS {
            self.previous.pop_back();
        }
    }

    pub(crate) fn into_keys(self) -> Vec<DataProtectionKey> {
        std::iter::once(self.active).chain(self.previous).collect()
    }
}

impl DataProtectionKey {
    /// Creates a data protection key from exact random bytes.
    pub fn from_bytes(bytes: [u8; DATA_PROTECTION_KEY_BYTES]) -> Self {
        let item_id_root =
            blake3::derive_key(crate::contract::VALUE_FORMAT_ITEM_ID_ROOT_CONTEXT, &bytes);
        let value_root_key =
            blake3::derive_key(crate::contract::VALUE_FORMAT_VALUE_ROOT_CONTEXT, &bytes);
        Self {
            master_key: bytes,
            item_id_root,
            value_root_key,
        }
    }

    /// Copies an exact data protection key from a language binding or configuration buffer.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Exactly 32 random secret bytes.
    ///
    /// # Returns
    ///
    /// An owned data protection key.
    ///
    /// # Errors
    ///
    /// Returns an error when `bytes` does not contain exactly 32 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let exact: &[u8; DATA_PROTECTION_KEY_BYTES] = bytes.try_into().map_err(|_| {
            Error::configuration(
                "data_protection_key",
                format!(
                    "must contain exactly {DATA_PROTECTION_KEY_BYTES} bytes, got {}",
                    bytes.len()
                ),
            )
        })?;
        Ok(Self::from_bytes(*exact))
    }

    /// Decodes a Base64-encoded 32-byte random secret.
    ///
    /// # Arguments
    ///
    /// * `encoded` - Standard padded or unpadded Base64 text.
    ///
    /// # Returns
    ///
    /// An owned data protection key.
    ///
    /// # Errors
    ///
    /// Returns an error when Base64 decoding fails or does not produce exactly 32 bytes.
    pub fn from_base64(encoded: &str) -> Result<Self> {
        let engine = if encoded.ends_with('=') {
            &STANDARD
        } else {
            &STANDARD_NO_PAD
        };
        let decoded = Zeroizing::new(
            engine
                .decode(encoded)
                .map_err(|error| Error::configuration("data_protection_key", error.to_string()))?,
        );
        if decoded.len() != DATA_PROTECTION_KEY_BYTES {
            return Err(Error::configuration(
                "data_protection_key",
                format!(
                    "must decode to exactly {DATA_PROTECTION_KEY_BYTES} bytes, got {}",
                    decoded.len()
                ),
            ));
        }
        let mut bytes = [0; DATA_PROTECTION_KEY_BYTES];
        bytes.copy_from_slice(&decoded);
        Ok(Self::from_bytes(bytes))
    }

    /// Returns the canonical padded Base64 representation for secret storage.
    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.master_key)
    }

    /// Derives the deterministic BLAKE3 item ID for application key bytes.
    ///
    /// # Arguments
    ///
    /// * `application_key` - Exact application key bytes without normalization or framing.
    ///
    /// # Returns
    ///
    /// The deterministic item ID scoped to this data protection key.
    pub fn derive_item_id(&self, application_key: impl AsRef<[u8]>) -> ItemId {
        ItemId::from_bytes(
            *blake3::keyed_hash(&self.item_id_root, application_key.as_ref()).as_bytes(),
        )
    }

    pub(crate) fn value_root_key(&self) -> Zeroizing<[u8; DATA_PROTECTION_KEY_BYTES]> {
        Zeroizing::new(self.value_root_key)
    }
}

impl TryFrom<&[u8]> for DataProtectionKey {
    type Error = Error;

    fn try_from(bytes: &[u8]) -> Result<Self> {
        Self::from_slice(bytes)
    }
}
