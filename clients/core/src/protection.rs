//! Shared application-key hiding and value-protection composition.

use crate::key::validate_canonical_key;
use crate::value::{Compression, Encryption, ItemValue, Value, ValueCodec, ValueKeyring};
use crate::{
    ClientRootKey, DataProtectionKey, ItemId, KeyFormat, KeyType, Result, TypedKey,
    transport::RequestBudget,
};
use openkache_value::Value as StructuredValue;

/// Reusable keyed transformation shared by language-specific client layers.
pub struct DataProtection {
    key: ClientRootKey,
    key_spec: KeyType,
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
        Self::with_key_spec(key, KeyType::Bytes, compression)
    }

    /// Creates mandatory protection with an explicit formatted key spec.
    pub fn with_key_spec(
        key: ClientRootKey,
        key_spec: KeyType,
        compression: Compression,
    ) -> Result<Self> {
        if key.is_zero() {
            return Err(crate::Error::configuration(
                "client_root_key",
                "must not be all zero when value protection is enabled",
            ));
        }
        let codec =
            ValueCodec::protected(&key, compression)?.allow_read_profile(Encryption::Compact);
        Ok(Self {
            key,
            key_spec,
            key_format: KeyFormat::NamespaceHash,
            codec,
        })
    }

    /// Creates protection with a separate Item ID root and value-key keyring.
    ///
    /// The root key remains solely responsible for deterministic application
    /// key/Item ID derivation. The supplied keyring owns value-key rotation:
    /// readers accept every inserted ID while writes use its active ID.
    pub fn with_keyring(
        key: ClientRootKey,
        keyring: ValueKeyring,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        Self::with_keyring_and_key_spec(key, keyring, KeyType::Bytes, compression, encryption)
    }

    /// Creates rotating value protection with an explicit formatted key spec.
    pub fn with_keyring_and_key_spec(
        key: ClientRootKey,
        keyring: ValueKeyring,
        key_spec: KeyType,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        if key.is_zero() && encryption != Encryption::Unprotected {
            return Err(crate::Error::configuration(
                "client_root_key",
                "must not be all zero when value protection is enabled",
            ));
        }
        let codec = match encryption {
            Encryption::Unprotected => ValueCodec::compressed(compression)?,
            Encryption::Compact | Encryption::Robust => {
                let alternate = match encryption {
                    Encryption::Compact => Encryption::Robust,
                    Encryption::Robust => Encryption::Compact,
                    Encryption::Unprotected => unreachable!(),
                };
                ValueCodec::with_keyring(keyring, compression, encryption)?
                    .allow_read_profile(alternate)
            }
        };
        Ok(Self {
            key,
            key_spec,
            key_format: KeyFormat::NamespaceHash,
            codec,
        })
    }

    /// Applies one aggregate byte budget to value protection work.
    ///
    /// The budget is retained by this transformation and shared by envelope,
    /// decrypted, decompressed, and structured-codec allocations. Protected
    /// network clients install their own connection budget automatically;
    /// standalone callers can use this method to apply the same bound.
    pub fn with_budget(mut self, budget: RequestBudget) -> Self {
        self.codec = self.codec.with_budget(budget);
        self
    }

    /// Creates the default unprotected formatted client.
    ///
    /// The all-zero root still participates in namespace-bound Item ID
    /// derivation. Only value protection is disabled.
    pub fn unprotected(key_spec: KeyType, compression: Compression) -> Result<Self> {
        let codec = ValueCodec::compressed(compression)?;
        Ok(Self {
            key: ClientRootKey::zero(),
            key_spec,
            key_format: KeyFormat::NamespaceHash,
            codec,
        })
    }

    /// Creates protection with an explicit authenticated-encryption profile.
    ///
    /// # Arguments
    ///
    /// * `key` - Application-managed data protection key.
    /// * `compression` - Compression policy applied before encryption.
    /// * `encryption` - Unprotected, Compact, or Robust profile.
    ///
    /// # Returns
    ///
    /// A reusable application-key and value transformation.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid compression setting or protected profile
    /// configuration.
    pub fn with_profile(
        key: DataProtectionKey,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        Self::with_profile_and_key_spec(key, KeyType::Bytes, compression, encryption)
    }

    /// Creates protection with an explicit key spec and encryption profile.
    pub fn with_profile_and_key_spec(
        key: ClientRootKey,
        key_spec: KeyType,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        if key.is_zero() && encryption != Encryption::Unprotected {
            return Err(crate::Error::configuration(
                "client_root_key",
                "must not be all zero when value protection is enabled",
            ));
        }
        let codec = match encryption {
            Encryption::Unprotected => ValueCodec::compressed(compression)?,
            Encryption::Compact | Encryption::Robust => {
                let alternate = match encryption {
                    Encryption::Compact => Encryption::Robust,
                    Encryption::Robust => Encryption::Compact,
                    Encryption::Unprotected => unreachable!(),
                };
                ValueCodec::protected_with_profile(&key, compression, encryption)?
                    .allow_read_profile(alternate)
            }
        };
        Ok(Self {
            key,
            key_spec,
            key_format: KeyFormat::NamespaceHash,
            codec,
        })
    }

    /// Creates a protection facade with an explicit key type and mapping
    /// profile. Mapping and value protection remain independent.
    pub fn with_profile_and_key_type_and_format(
        key: ClientRootKey,
        key_type: crate::KeyType,
        key_format: KeyFormat,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        crate::KeySpace::with_format(key_type, key_format)
            .validate()
            .map_err(crate::Error::from)?;
        if key.is_zero() && encryption != Encryption::Unprotected {
            return Err(crate::Error::configuration(
                "client_root_key",
                "must not be all zero when value protection is enabled",
            ));
        }
        let codec = ValueCodec::protected_with_profile(&key, compression, encryption)?;
        Ok(Self {
            key,
            key_spec: key_type,
            key_format,
            codec,
        })
    }

    /// Returns the compatibility key-type selector.
    ///
    /// `TypedKey` operations infer their type per call under the v1 contract;
    /// this accessor remains for callers migrating from the pre-contract
    /// `key_spec` setting.
    pub const fn key_spec(&self) -> KeyType {
        self.key_spec
    }

    /// Returns the configured compatibility key-type selector.
    ///
    /// The v1 contract infers `TypedKey` from each operation. This accessor is
    /// retained for callers migrating from the pre-contract `key_spec` name;
    /// it does not change the canonical-key ABI boundary.
    pub const fn key_type(&self) -> KeyType {
        self.key_spec
    }

    /// Returns the configured Item ID mapping profile.
    pub const fn key_format(&self) -> KeyFormat {
        self.key_format
    }

    /// Derives a namespace-bound Item ID for one typed key.
    ///
    /// The `TypedKey` variant is inferred by the operation. The compatibility
    /// `key_spec` setting is not a namespace policy and is not applied here.
    /// Returns the configured value resource limits.
    pub fn value_limits(&self) -> crate::value::ValueLimits {
        self.codec.limits()
    }

    /// Returns the aggregate byte budget shared by transport and value work.
    pub fn request_budget(&self) -> RequestBudget {
        self.codec.budget().clone()
    }

    /// Derives a namespace-bound Item ID for a typed portable key.
    pub fn item_id_in_namespace(
        &self,
        namespace_id: u64,
        key: impl Into<TypedKey>,
    ) -> Result<ItemId> {
        let key = key.into();
        self.key
            .derive_item_id_in_namespace_with_format(namespace_id, self.key_format, key)
            .map_err(Into::into)
    }

    /// Derives an Item ID from canonical key bytes after validating one item.
    pub fn item_id_from_canonical_key(
        &self,
        namespace_id: u64,
        canonical_key: &[u8],
    ) -> Result<ItemId> {
        let canonical_key = validate_canonical_key(canonical_key)?;
        self.key
            .derive_item_id_from_validated_canonical_key(
                namespace_id,
                self.key_format,
                canonical_key,
            )
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
        let canonical_key = validate_canonical_key(canonical_key)?;
        self.key
            .derive_item_id_from_validated_canonical_key(
                namespace_id,
                self.key_format,
                canonical_key,
            )
            .map_err(Into::into)
    }

    /// Explicit compatibility path for the pre-contract byte-key mapping.
    pub fn legacy_item_id(&self, application_key: impl AsRef<[u8]>) -> ItemId {
        self.key.derive_item_id(application_key)
    }

    /// Legacy byte-key convenience retained for source compatibility.
    ///
    /// New mapped operations should use [`Self::item_id_in_namespace`].
    #[deprecated(note = "use item_id_in_namespace or legacy_item_id explicitly")]
    pub fn item_id(&self, application_key: impl AsRef<[u8]>) -> ItemId {
        self.legacy_item_id(application_key)
    }

    /// Resolves a typed key through the explicit public preserve-or-hash
    /// profile. The method name keeps this escape hatch distinct from mapped
    /// NamespaceHash operations.
    pub fn public_key_or_hash_in_namespace(
        &self,
        namespace_id: u64,
        key: impl Into<TypedKey>,
    ) -> Result<ItemId> {
        let key = key.into();
        self.key
            .derive_item_id_in_namespace_with_format(namespace_id, KeyFormat::PublicKeyOrHash, key)
            .map_err(Into::into)
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

    /// Serializes and protects one canonical StructuredValue-CBOR-v1 value.
    ///
    /// The structured ABI accepts the value-model algebra directly; opaque
    /// bytes and JSON convenience views are intentionally separate APIs.
    pub fn encode_structured_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: &StructuredValue,
    ) -> Result<ItemValue> {
        self.codec
            .seal_structured_in_namespace(namespace_id, item_id, value)
            .map_err(Into::into)
    }

    /// Serializes and protects a StructuredValue-CBOR-v1 value while binding its namespace.
    pub fn seal_structured_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: &StructuredValue,
    ) -> Result<ItemValue> {
        self.encode_structured_in_namespace(namespace_id, item_id, value)
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

    /// Authenticates and decodes one canonical StructuredValue-CBOR-v1 value.
    pub fn decode_structured_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<StructuredValue> {
        self.codec
            .open_structured_in_namespace(namespace_id, item_id, encoded)
            .map_err(Into::into)
    }

    /// Authenticates and decodes one StructuredValue-CBOR-v1 value.
    pub fn open_structured_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<StructuredValue> {
        self.decode_structured_in_namespace(namespace_id, item_id, encoded)
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
