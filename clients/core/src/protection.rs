//! Shared application-key hiding and value-protection composition.

use crate::value::{Compression, Encryption, ItemValue, Value, ValueCodec};
use crate::{
    ClientRootKey, DataProtectionKey, ItemId, KeyFormat, KeySpace, KeyType, Result, TypedKey,
};

/// Reusable keyed transformation shared by language-specific client layers.
pub struct DataProtection {
    key: ClientRootKey,
    key_type: KeyType,
    key_format: KeyFormat,
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
        Self::with_key_type(key, KeyType::Bytes, compression)
    }

    /// Creates mandatory protection with an explicit typed-key type.
    pub fn with_key_type(
        key: ClientRootKey,
        key_type: KeyType,
        compression: Compression,
    ) -> Result<Self> {
        if key.is_zero() {
            return Err(crate::Error::configuration(
                "client_root_key",
                "must not be all zero when value protection is enabled",
            ));
        }
        let codec = ValueCodec::protected(&key, compression)?;
        Ok(Self {
            key,
            key_type,
            key_format: KeyFormat::Hash,
            codec,
        })
    }

    /// Compatibility spelling for [`Self::with_key_type`].
    pub fn with_key_spec(
        key: ClientRootKey,
        key_spec: KeyType,
        compression: Compression,
    ) -> Result<Self> {
        Self::with_key_type(key, key_spec, compression)
    }

    /// Creates the default unprotected formatted client.
    ///
    /// The all-zero root still participates in namespace-bound Item ID
    /// derivation. Only value protection is disabled.
    pub fn unprotected(key_type: KeyType, compression: Compression) -> Result<Self> {
        Self::unprotected_with_root(
            ClientRootKey::zero(),
            key_type,
            KeyFormat::Hash,
            compression,
        )
    }

    /// Creates an unprotected value codec while retaining a supplied root for
    /// client-side Item ID derivation.
    ///
    /// This is used when a caller deliberately disables value encryption but
    /// still supplies a root-bound Item ID profile. The root is not used to
    /// encrypt values, but it remains the source of the Item ID derivation key.
    pub(crate) fn unprotected_with_root(
        key: ClientRootKey,
        key_type: KeyType,
        key_format: KeyFormat,
        compression: Compression,
    ) -> Result<Self> {
        KeySpace::with_format(key_type, key_format)
            .validate()
            .map_err(crate::Error::from)?;
        let codec = ValueCodec::compressed_with_key(&key, compression)?;
        Ok(Self {
            key,
            key_type,
            key_format,
            codec,
        })
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
        Self::with_profile_and_key_type(key, KeyType::Bytes, compression, encryption)
    }

    /// Creates protection with an explicit key type and encryption profile.
    pub fn with_profile_and_key_type(
        key: ClientRootKey,
        key_type: KeyType,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        if key.is_zero() && encryption != Encryption::Unprotected {
            return Err(crate::Error::configuration(
                "client_root_key",
                "must not be all zero when value protection is enabled",
            ));
        }
        let codec = ValueCodec::protected_with_profile(&key, compression, encryption)?;
        Ok(Self {
            key,
            key_type,
            key_format: KeyFormat::Hash,
            codec,
        })
    }

    /// Compatibility spelling for [`Self::with_profile_and_key_type`].
    pub fn with_profile_and_key_spec(
        key: ClientRootKey,
        key_spec: KeyType,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        Self::with_profile_and_key_type(key, key_spec, compression, encryption)
    }

    /// Returns the configured typed-key type.
    pub const fn key_type(&self) -> KeyType {
        self.key_type
    }

    /// Compatibility spelling for [`Self::key_type`].
    pub const fn key_spec(&self) -> KeyType {
        self.key_type()
    }

    /// Returns the client-owned key mapping profile.
    pub const fn key_format(&self) -> KeyFormat {
        self.key_format
    }

    /// Creates mandatory protection with an explicit key type and mapping profile.
    pub fn with_key_type_and_format(
        key: ClientRootKey,
        key_type: KeyType,
        key_format: KeyFormat,
        compression: Compression,
    ) -> Result<Self> {
        if key.is_zero() {
            return Err(crate::Error::configuration(
                "client_root_key",
                "must not be all zero when value protection is enabled",
            ));
        }
        KeySpace::with_format(key_type, key_format)
            .validate()
            .map_err(crate::Error::from)?;
        let codec = ValueCodec::protected(&key, compression)?;
        Ok(Self {
            key,
            key_type,
            key_format,
            codec,
        })
    }

    /// Compatibility spelling for [`Self::with_key_type_and_format`].
    pub fn with_key_spec_and_format(
        key: ClientRootKey,
        key_spec: KeyType,
        key_format: KeyFormat,
        compression: Compression,
    ) -> Result<Self> {
        Self::with_key_type_and_format(key, key_spec, key_format, compression)
    }

    /// Creates protection with explicit key mapping and value profile.
    pub fn with_key_type_and_format_profile(
        key: ClientRootKey,
        key_type: KeyType,
        key_format: KeyFormat,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        if key.is_zero() && encryption != Encryption::Unprotected {
            return Err(crate::Error::configuration(
                "client_root_key",
                "must not be all zero when value protection is enabled",
            ));
        }
        KeySpace::with_format(key_type, key_format)
            .validate()
            .map_err(crate::Error::from)?;
        let codec = ValueCodec::protected_with_profile(&key, compression, encryption)?;
        Ok(Self {
            key,
            key_type,
            key_format,
            codec,
        })
    }

    /// Compatibility spelling for [`Self::with_key_type_and_format_profile`].
    pub fn with_key_spec_and_format_profile(
        key: ClientRootKey,
        key_spec: KeyType,
        key_format: KeyFormat,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        Self::with_key_type_and_format_profile(
            key,
            key_spec,
            key_format,
            compression,
            encryption,
        )
    }

    /// Creates an unprotected client with an explicit key mapping profile.
    pub fn unprotected_with_format(
        key_type: KeyType,
        key_format: KeyFormat,
        compression: Compression,
    ) -> Result<Self> {
        KeySpace::with_format(key_type, key_format)
            .validate()
            .map_err(crate::Error::from)?;
        Ok(Self {
            key: ClientRootKey::zero(),
            key_type,
            key_format,
            codec: ValueCodec::compressed(compression)?,
        })
    }

    /// Derives a namespace-bound Item ID for a typed key.
    pub fn item_id_in_namespace(
        &self,
        namespace_id: u64,
        key: impl Into<TypedKey>,
    ) -> Result<ItemId> {
        let key = key.into();
        if key.key_type() != self.key_type {
            return Err(crate::Error::Key(crate::KeyError::KeyTypeMismatch {
                expected: self.key_type,
                actual: key.key_type(),
            }));
        }
        let resolved = KeySpace::with_format(self.key_type, self.key_format)
            .resolve(key)
            .map_err(crate::Error::from)?;
        self.key
            .derive_item_id_for_resolved_key(namespace_id, &resolved)
            .map_err(Into::into)
    }

    /// Derives an Item ID from canonical key bytes after validating the spec.
    pub fn item_id_from_canonical_key(
        &self,
        namespace_id: u64,
        canonical_key: &[u8],
    ) -> Result<ItemId> {
        let key = TypedKey::decode_canonical(canonical_key)?;
        if key.key_type() != self.key_type {
            return Err(crate::Error::Key(crate::KeyError::KeyTypeMismatch {
                expected: self.key_type,
                actual: key.key_type(),
            }));
        }
        let resolved = KeySpace::with_format(self.key_type, self.key_format)
            .resolve_canonical(canonical_key)
            .map_err(crate::Error::from)?;
        self.key
            .derive_item_id_for_resolved_key(namespace_id, &resolved)
            .map_err(Into::into)
    }

    /// Derives an Item ID from canonical key bytes without applying a configured
    /// [`KeyType`].
    ///
    /// This is the boundary for the low-level native ABI. The canonical CBOR
    /// item carries its own `Integer`, `Text`, or `Bytes` discriminator; typed
    /// high-level clients should use [`Self::item_id_from_canonical_key`]
    /// instead so one keyspace cannot accidentally mix types.
    #[cfg(feature = "ffi")]
    pub(crate) fn item_id_from_canonical_key_unchecked(
        &self,
        namespace_id: u64,
        canonical_key: &[u8],
    ) -> Result<ItemId> {
        self.key
            .derive_item_id_from_canonical_key(namespace_id, canonical_key)
            .map_err(Into::into)
    }

    /// Legacy byte-key convenience using namespace `1`.
    ///
    /// This helper still applies the configured [`KeyFormat`]. In particular,
    /// `ByteKeyOrHash` preserves direct byte keys up to the wire Item ID limit
    /// and hashes only longer keys. Use [`Self::try_item_id`] when the caller
    /// needs a validation error instead of the compatibility panic behavior.
    pub fn item_id(&self, application_key: impl AsRef<[u8]>) -> ItemId {
        self.try_item_id(application_key)
            .expect("legacy application key does not match the configured key space")
    }

    /// Fallible legacy byte-key convenience using namespace `1`.
    ///
    /// The bytes are interpreted by the configured [`KeyType`] and
    /// [`KeyFormat`]. Empty keys are valid; malformed text or integer bytes and
    /// oversized inputs are rejected.
    pub fn try_item_id(
        &self,
        application_key: impl AsRef<[u8]>,
    ) -> std::result::Result<ItemId, crate::KeyError> {
        if self.key_type == KeyType::Bytes && self.key_format == KeyFormat::ByteKeyOrHash {
            self.key
                .derive_byte_key_or_hash_in_namespace(1, application_key)
        } else {
            self.key.try_derive_item_id(application_key)
        }
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
        self.encode_in_namespace(1, item_id, value)
    }

    /// Serializes and protects a value while binding its namespace into AAD.
    pub fn encode_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: Value,
    ) -> Result<ItemValue> {
        self.codec
            .encode_in_namespace(namespace_id, item_id, value)
            .map_err(Into::into)
    }

    /// Serializes a logical JSON value as canonical UTF-8 `OpaqueBytes`.
    pub fn encode_json(
        &self,
        item_id: ItemId,
        value: crate::value::JsonValue,
    ) -> Result<ItemValue> {
        self.codec
            .encode_json(item_id, value)
            .map_err(Into::into)
    }

    /// Serializes a logical JSON value and binds its namespace into AAD.
    pub fn encode_json_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: crate::value::JsonValue,
    ) -> Result<ItemValue> {
        self.codec
            .encode_json_in_namespace(namespace_id, item_id, value)
            .map_err(Into::into)
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
        self.decode_in_namespace(1, item_id, encoded)
    }

    /// Authenticates and decodes a value while binding its namespace into AAD.
    pub fn decode_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<Value> {
        self.codec
            .decode_in_namespace(namespace_id, item_id, encoded)
            .map_err(Into::into)
    }

    /// Authenticates and parses canonical JSON stored as `OpaqueBytes`.
    pub fn decode_json(
        &self,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<crate::value::JsonValue> {
        self.codec
            .decode_json(item_id, encoded)
            .map_err(Into::into)
    }

    /// Authenticates and parses canonical JSON while binding its namespace.
    pub fn decode_json_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<crate::value::JsonValue> {
        self.codec
            .decode_json_in_namespace(namespace_id, item_id, encoded)
            .map_err(Into::into)
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
        self.seal_in_namespace(1, item_id, plaintext)
    }

    /// Encrypts bytes while binding their namespace and Item ID into AAD.
    pub fn seal_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        plaintext: &[u8],
    ) -> Result<ItemValue> {
        self.encode_in_namespace(namespace_id, item_id, Value::Raw(plaintext.to_vec()))
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
        self.seal_owned_in_namespace(1, item_id, plaintext)
    }

    /// Encrypts owned bytes while binding their namespace and Item ID into AAD.
    pub fn seal_owned_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        plaintext: Vec<u8>,
    ) -> Result<ItemValue> {
        self.encode_in_namespace(namespace_id, item_id, Value::Raw(plaintext))
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
        self.open_in_namespace(1, item_id, encoded)
    }

    /// Authenticates and opens bytes while binding their namespace and Item ID into AAD.
    pub fn open_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<Vec<u8>> {
        match self.decode_in_namespace(namespace_id, item_id, encoded)? {
            Value::Raw(bytes) => Ok(bytes),
            Value::Cbor(_) | Value::Json(_) => Err(crate::value::Error::ExpectedRawValue.into()),
        }
    }
}
