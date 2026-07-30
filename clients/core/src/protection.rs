//! Shared application-key hiding and value-protection composition.

use crate::value::{Compression, Encryption, ItemValue, Value, ValueCodec};
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

    /// Creates protection with an explicit authenticated-encryption profile.
    ///
    /// # Arguments
    ///
    /// * `key` - Application-managed data protection key.
    /// * `compression` - Compression policy applied before encryption.
    /// * `encryption` - Compact or Robust authenticated-encryption profile.
    ///
    /// # Returns
    ///
    /// A reusable application-key and value transformation.
    ///
    /// # Errors
    ///
    /// Returns an error for an unprotected profile or invalid compression settings.
    pub fn with_profile(
        key: DataProtectionKey,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        let codec = ValueCodec::protected_with_profile(&key, compression, encryption)?;
        Ok(Self { key, codec })
    }

    /// Derives the deterministic BLAKE3 item key for application key bytes.
    pub fn item_key(&self, application_key: impl AsRef<[u8]>) -> ItemKey {
        self.key.derive_item_key(application_key)
    }

    /// Serializes and protects one core logical value.
    ///
    /// # Arguments
    ///
    /// * `key` - Exact item key bound into authenticated encryption.
    /// * `value` - Raw or logical JSON value to encode.
    ///
    /// # Returns
    ///
    /// A complete value-format container for opaque storage.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid logical values, size-limit violations, compression failures,
    /// entropy failures, or encryption failures.
    pub fn encode(&self, key: ItemKey, value: Value) -> Result<ItemValue> {
        self.codec.encode(key, value).map_err(Into::into)
    }

    /// Authenticates and decodes one stored value into the core logical model.
    ///
    /// # Arguments
    ///
    /// * `key` - Exact item key expected by authenticated encryption.
    /// * `encoded` - Complete value-format container returned by the raw client.
    ///
    /// # Returns
    ///
    /// The decoded Raw or logical JSON value.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is malformed, unsupported, oversized, unauthenticated, or
    /// cannot be decompressed or deserialized.
    pub fn decode(&self, key: ItemKey, encoded: ItemValue) -> Result<Value> {
        self.codec.decode(key, encoded).map_err(Into::into)
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
        self.encode(key, Value::Raw(plaintext.to_vec()))
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
        self.encode(key, Value::Raw(plaintext))
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
        match self.decode(key, encoded)? {
            Value::Raw(bytes) => Ok(bytes),
            Value::Json(_) => Err(crate::value::Error::ExpectedRawValue.into()),
        }
    }
}
