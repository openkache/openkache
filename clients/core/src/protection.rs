//! Shared application-key hiding and value-protection composition.

use crate::key::{KeyBinding, KeyInput, KeyResolver};
use crate::value::{Compression, Encryption, ItemValue, Value, ValueCodec};
use crate::{
    ClientRootKey, DataProtectionKey, ItemId, KeyFormat, KeyType, ResolvedKey, Result, TypedKey,
};

/// Reusable keyed transformation shared by language-specific client layers.
pub struct DataProtection {
    resolver: KeyResolver,
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

    /// Creates mandatory protection with an explicit key type.
    pub fn with_key_type(
        key: ClientRootKey,
        key_type: KeyType,
        compression: Compression,
    ) -> Result<Self> {
        Self::with_key_type_and_format(key, key_type, KeyFormat::Hash, compression)
    }

    /// Creates mandatory protection with an explicit key type and mapping profile.
    pub fn with_key_type_and_format(
        key: ClientRootKey,
        key_type: KeyType,
        format: KeyFormat,
        compression: Compression,
    ) -> Result<Self> {
        crate::KeySpace::with_format(key_type, format).validate()?;
        if key.is_zero() {
            return Err(crate::Error::configuration(
                "client_root_key",
                "must not be all zero when value protection is enabled",
            ));
        }
        let resolver = KeyResolver::with_format(key, key_type, format);
        let codec = ValueCodec::protected(resolver.root(), compression)?;
        Ok(Self { resolver, codec })
    }

    /// Creates the default unprotected formatted client.
    ///
    /// The all-zero root still participates in namespace-bound Item ID
    /// derivation. Only value protection is disabled.
    pub fn unprotected(key_type: KeyType, compression: Compression) -> Result<Self> {
        Self::unprotected_with_format(key_type, KeyFormat::Hash, compression)
    }

    /// Creates an unprotected client with an explicit mapping profile.
    pub fn unprotected_with_format(
        key_type: KeyType,
        format: KeyFormat,
        compression: Compression,
    ) -> Result<Self> {
        Self::unprotected_with_root(ClientRootKey::zero(), key_type, format, compression)
    }

    /// Creates an unprotected value codec while retaining the supplied root
    /// for Item ID derivation.
    pub(crate) fn unprotected_with_root(
        key: ClientRootKey,
        key_type: KeyType,
        format: KeyFormat,
        compression: Compression,
    ) -> Result<Self> {
        crate::KeySpace::with_format(key_type, format).validate()?;
        let resolver = KeyResolver::with_format(key, key_type, format);
        let codec = ValueCodec::compressed(compression)?;
        Ok(Self { resolver, codec })
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
        Self::with_profile_and_key_type_and_format(
            key,
            key_type,
            KeyFormat::Hash,
            compression,
            encryption,
        )
    }

    /// Creates protection with an explicit key type, mapping, and encryption profile.
    pub fn with_profile_and_key_type_and_format(
        key: ClientRootKey,
        key_type: KeyType,
        format: KeyFormat,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        crate::KeySpace::with_format(key_type, format).validate()?;
        if key.is_zero() && encryption != Encryption::Unprotected {
            return Err(crate::Error::configuration(
                "client_root_key",
                "must not be all zero when value protection is enabled",
            ));
        }
        let resolver = KeyResolver::with_format(key, key_type, format);
        let codec = ValueCodec::protected_with_profile(resolver.root(), compression, encryption)?;
        Ok(Self { resolver, codec })
    }

    /// Returns the configured key type.
    pub const fn key_type(&self) -> KeyType {
        self.resolver.key_type()
    }

    /// Returns the configured client-owned key mapping profile.
    pub const fn key_format(&self) -> KeyFormat {
        self.resolver.format()
    }

    /// Derives a namespace-bound Item ID for a typed key.
    pub fn item_id_in_namespace(
        &self,
        namespace_id: u64,
        key: impl Into<TypedKey>,
    ) -> Result<ItemId> {
        Ok(self
            .resolver
            .bind_input(namespace_id, KeyInput::typed(key))?
            .item_id)
    }

    /// Derives a direct byte Item ID under `ByteKeyOrHash`.
    pub fn byte_key_or_hash_in_namespace(
        &self,
        namespace_id: u64,
        key: impl AsRef<[u8]>,
    ) -> Result<ItemId> {
        if self.key_type() != KeyType::Bytes || self.key_format() != KeyFormat::ByteKeyOrHash {
            return Err(crate::Error::configuration(
                "key_format",
                "byte_key_or_hash_in_namespace requires KeyType::Bytes with KeyFormat::ByteKeyOrHash",
            ));
        }
        self.resolver
            .root()
            .derive_byte_key_or_hash_in_namespace(namespace_id, key)
            .map_err(Into::into)
    }

    /// Resolves one internal key input at the shared core boundary.
    pub(crate) fn resolve_key_input(&self, input: KeyInput) -> Result<ResolvedKey> {
        self.resolver.resolve_input(input).map_err(Into::into)
    }

    /// Resolves and binds one internal key input without exposing the
    /// canonical representation to client layers.
    pub(crate) fn bind_key_input(&self, namespace_id: u64, input: KeyInput) -> Result<KeyBinding> {
        self.resolver
            .bind_input(namespace_id, input)
            .map_err(Into::into)
    }

    /// Resolves an already validated key and binds it to one namespace.
    pub(crate) fn bind_resolved_key(
        &self,
        namespace_id: u64,
        key: &ResolvedKey,
    ) -> Result<KeyBinding> {
        self.resolver.bind(namespace_id, key).map_err(Into::into)
    }

    /// Derives an Item ID from canonical key bytes after validating the key type.
    pub fn item_id_from_canonical_key(
        &self,
        namespace_id: u64,
        canonical_key: &[u8],
    ) -> Result<ItemId> {
        Ok(self
            .bind_key_input(
                namespace_id,
                KeyInput::canonical_in_space(canonical_key.to_owned()),
            )?
            .item_id)
    }

    /// Legacy byte-key convenience using namespace `1`.
    pub fn item_id(&self, application_key: impl AsRef<[u8]>) -> ItemId {
        self.resolver.legacy_item_id(application_key)
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
            Value::Json(_) => Err(crate::value::Error::ExpectedRawValue.into()),
        }
    }
}
