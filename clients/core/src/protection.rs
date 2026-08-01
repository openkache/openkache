//! Shared application-key hiding and value-protection composition.

use crate::value::{Compression, Encryption, ItemValue, Value, ValueCodec};
use crate::{DataProtectionKey, DataProtectionKeyRing, ItemId, Result};

/// Reusable keyed transformation shared by language-specific client layers.
pub struct DataProtection {
    keys: Vec<DataProtectionKey>,
    codecs: Vec<ValueCodec>,
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
        Self::with_key_ring(
            DataProtectionKeyRing::new(key),
            compression,
            Encryption::Robust,
        )
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
        Self::with_key_ring(DataProtectionKeyRing::new(key), compression, encryption)
    }

    /// Creates protection that writes with the active ring key and reads/deletes with
    /// the active key followed by each retained previous key.
    pub fn with_key_ring(
        key_ring: DataProtectionKeyRing,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        let keys = key_ring.into_keys();
        let codecs = keys
            .iter()
            .map(|key| {
                ValueCodec::protected_with_profile(key, compression, encryption).map_err(Into::into)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { keys, codecs })
    }

    /// Derives the deterministic BLAKE3 item ID for application key bytes.
    pub fn item_id(&self, application_key: impl AsRef<[u8]>) -> ItemId {
        self.keys[0].derive_item_id(application_key)
    }

    /// Returns item IDs for the active key followed by all retained previous keys.
    pub fn item_ids(&self, application_key: impl AsRef<[u8]>) -> Vec<ItemId> {
        let application_key = application_key.as_ref();
        self.keys
            .iter()
            .map(|key| key.derive_item_id(application_key))
            .collect()
    }

    /// Serializes and protects one core logical value.
    ///
    /// # Arguments
    ///
    /// * `item_id` - Exact item ID bound into authenticated encryption.
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
    pub fn encode(&self, item_id: ItemId, value: Value) -> Result<ItemValue> {
        self.codecs[0].encode(item_id, value).map_err(Into::into)
    }

    /// Authenticates and decodes one stored value into the core logical model.
    ///
    /// # Arguments
    ///
    /// * `item_id` - Exact item ID expected by authenticated encryption.
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
    pub fn decode(&self, item_id: ItemId, encoded: ItemValue) -> Result<Value> {
        let mut last_error = None;
        for codec in &self.codecs {
            match codec.decode(item_id, encoded.clone()) {
                Ok(value) => return Ok(value),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error
            .expect("a key ring always contains an active codec")
            .into())
    }

    /// Encrypts a borrowed plaintext value and binds it to its item ID.
    ///
    /// # Arguments
    ///
    /// * `item_id` - Item ID derived for the value's application key.
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
    pub fn seal(&self, item_id: ItemId, plaintext: &[u8]) -> Result<ItemValue> {
        self.encode(item_id, Value::Raw(plaintext.to_vec()))
    }

    /// Encrypts an owned plaintext value while reusing its allocation when practical.
    ///
    /// # Arguments
    ///
    /// * `item_id` - Item ID derived for the value's application key.
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
    pub fn seal_owned(&self, item_id: ItemId, plaintext: Vec<u8>) -> Result<ItemValue> {
        self.encode(item_id, Value::Raw(plaintext))
    }

    /// Authenticates, decrypts, and optionally decompresses a stored value.
    ///
    /// # Arguments
    ///
    /// * `item_id` - Item ID used as authenticated data.
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
    pub fn open(&self, item_id: ItemId, encoded: ItemValue) -> Result<Vec<u8>> {
        match self.decode(item_id, encoded)? {
            Value::Raw(bytes) => Ok(bytes),
            Value::Json(_) => Err(crate::value::Error::ExpectedRawValue.into()),
        }
    }
}
