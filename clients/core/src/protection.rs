//! Shared application-key hiding and value-protection composition.

use crate::value::{Compression, ItemValue, ValueCodec};
use crate::{DataProtectionKey, ItemKey, Result};

/// Reusable keyed transformation shared by language-specific client layers.
pub struct DataProtection {
    key: DataProtectionKey,
    codec: ValueCodec,
}

impl DataProtection {
    /// Creates mandatory application-key hiding and value encryption.
    ///
    /// # Arguments
    ///
    /// * `key` - Application-managed master secret.
    /// * `compression` - Optional compression applied before encryption.
    ///
    /// # Returns
    ///
    /// A reusable transformation with domain-separated key and value subkeys.
    ///
    /// # Errors
    ///
    /// Returns an error when the compression settings are invalid.
    pub fn new(key: DataProtectionKey, compression: Compression) -> Result<Self> {
        let codec = ValueCodec::protected(&key, compression)?;
        Ok(Self { key, codec })
    }

    /// Derives the deterministic HMAC-SHA-256 item key for application key bytes.
    pub fn item_key(&self, application_key: impl AsRef<[u8]>) -> ItemKey {
        self.key.derive_item_key(application_key)
    }

    /// Encrypts a borrowed plaintext value and binds it to its item key.
    ///
    /// # Arguments
    ///
    /// * `key` - Item key derived for the value's application key.
    /// * `plaintext` - Exact application value bytes.
    ///
    /// # Returns
    ///
    /// Opaque encrypted bytes containing the client-owned transformation envelope.
    ///
    /// # Errors
    ///
    /// Returns an error for oversized values, entropy failures, compression failures, or
    /// encryption failures.
    pub fn seal(&self, key: ItemKey, plaintext: &[u8]) -> Result<ItemValue> {
        self.codec.seal(key, plaintext).map_err(Into::into)
    }

    /// Encrypts an owned plaintext value while reusing its allocation when practical.
    ///
    /// # Arguments
    ///
    /// * `key` - Item key derived for the value's application key.
    /// * `plaintext` - Owned application value bytes.
    ///
    /// # Returns
    ///
    /// Opaque encrypted bytes containing the client-owned transformation envelope.
    ///
    /// # Errors
    ///
    /// Returns an error for oversized values, entropy failures, compression failures, or
    /// encryption failures.
    pub fn seal_owned(&self, key: ItemKey, plaintext: Vec<u8>) -> Result<ItemValue> {
        self.codec.seal_owned(key, plaintext).map_err(Into::into)
    }

    /// Authenticates, decrypts, and optionally decompresses a stored value.
    ///
    /// # Arguments
    ///
    /// * `key` - Item key used as authenticated data.
    /// * `encoded` - Exact encrypted value returned by the raw client.
    ///
    /// # Returns
    ///
    /// The original plaintext application value.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is malformed, oversized, cannot be authenticated, or
    /// cannot be decompressed.
    pub fn open(&self, key: ItemKey, encoded: ItemValue) -> Result<Vec<u8>> {
        self.codec.open(key, encoded).map_err(Into::into)
    }
}
