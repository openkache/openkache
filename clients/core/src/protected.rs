//! Shared application-key and plaintext-value clients for language bindings.

use std::sync::Arc;
use std::time::Duration;

use crate::ValueKeyring;
use crate::value::{Compression, Encryption, JsonValue, Value};
use crate::{
    AlpnPolicy, Certificate, ClientIdentity, ClientRootKey, ClientTimeouts, ConnectionState,
    DataProtection, DataProtectionKey, DeleteOutcome, Endpoint, GetOutcome, KeyFormat, KeyType,
    NamespaceDescriptor, NamespacePolicy, Result, RetryPolicy, ServerTrust, SetOptions, SetOutcome,
    TypedKey,
};
#[cfg(feature = "quic-compio")]
use crate::{LocalRawClient, LocalRawClientBuilder};
#[cfg(feature = "quic-quinn")]
use crate::{RawClient, RawClientBuilder};
#[cfg(feature = "tls-tcp")]
use crate::{TlsTcpRawClient, TlsTcpRawClientBuilder};
use openkache_value::Value as StructuredValue;

struct ProtectionSettings {
    compression: Compression,
    encryption: Encryption,
    encryption_explicit: bool,
    key: Option<DataProtectionKey>,
    keyring: Option<ValueKeyring>,
    key_spec: KeyType,
    key_format: KeyFormat,
}

impl ProtectionSettings {
    fn new(key: DataProtectionKey) -> Self {
        Self::with_optional_key(Some(key))
    }

    fn unprotected() -> Self {
        Self::with_optional_key(None)
    }

    fn with_optional_key(key: Option<DataProtectionKey>) -> Self {
        Self {
            compression: Compression::default(),
            encryption: Encryption::Robust,
            encryption_explicit: false,
            key,
            keyring: None,
            key_spec: KeyType::Bytes,
            key_format: KeyFormat::NamespaceHash,
        }
    }

    fn finish_with_budget(self, budget: crate::RequestBudget) -> Result<Arc<DataProtection>> {
        let protection = match self.key {
            Some(key) => match self.keyring {
                Some(keyring) => DataProtection::with_keyring_and_key_spec(
                    key,
                    keyring,
                    self.key_spec,
                    self.compression,
                    self.encryption,
                ),
                None => DataProtection::with_profile_and_key_type_and_format(
                    key,
                    self.key_spec,
                    self.key_format,
                    self.compression,
                    self.encryption,
                ),
            },
            None => {
                if self.encryption_explicit && self.encryption != Encryption::Unprotected {
                    return Err(crate::Error::configuration(
                        "encryption",
                        "an encryption profile requires client_root_key",
                    ));
                }
                DataProtection::with_profile_and_key_type_and_format(
                    ClientRootKey::zero(),
                    self.key_spec,
                    self.key_format,
                    self.compression,
                    Encryption::Unprotected,
                )
            }
        }?;
        Ok(Arc::new(protection.with_budget(budget)))
    }
}

macro_rules! protected_builder_methods {
    ($builder:ident) => {
        impl $builder {
            /// Uses only the supplied server trust roots.
            pub fn server_trust(mut self, trust: ServerTrust) -> Self {
                self.raw = self.raw.server_trust(trust);
                self
            }

            /// Trusts one explicit CA or self-signed server certificate.
            pub fn trust_certificate(mut self, certificate: Certificate) -> Self {
                self.raw = self.raw.trust_certificate(certificate);
                self
            }

            /// Presents a mutual TLS client identity.
            pub fn client_identity(mut self, identity: ClientIdentity) -> Self {
                self.raw = self.raw.client_identity(identity);
                self
            }

            /// Configures the supported `openkache/1` protocol.
            pub fn alpn_policy(mut self, policy: AlpnPolicy) -> Self {
                self.raw = self.raw.alpn_policy(policy);
                self
            }

            /// Sets connection and complete-request deadlines.
            pub fn timeouts(mut self, timeouts: ClientTimeouts) -> Self {
                self.raw = self.raw.timeouts(timeouts);
                self
            }

            /// Sets retry attempts for response-safe operations.
            pub fn retry_policy(mut self, retry: RetryPolicy) -> Self {
                self.raw = self.raw.retry_policy(retry);
                self
            }

            /// Bounds simultaneous request lanes on one connection.
            ///
            /// TLS-over-TCP retains one ordered lane regardless of this
            /// value; the setting remains shared for API compatibility.
            pub fn max_in_flight(mut self, maximum: usize) -> Self {
                self.raw = self.raw.max_in_flight(maximum);
                self
            }

            /// Sets the aggregate bytes retained across transport and value work.
            pub fn max_in_flight_bytes(mut self, maximum: usize) -> Self {
                self.raw = self.raw.max_in_flight_bytes(maximum);
                self
            }

            /// Selects a previously server-assigned namespace ID without resolving a name.
            pub fn namespace_id(mut self, namespace_id: u64) -> Self {
                self.raw = self.raw.namespace_id(namespace_id);
                self
            }

            /// Resolves this namespace name with `CreateIfMissing` during connection setup.
            pub fn namespace_name(mut self, namespace_name: impl AsRef<[u8]>) -> Self {
                self.raw = self.raw.namespace_name(namespace_name);
                self
            }

            /// Supplies the policy used if the configured namespace name is missing.
            pub fn namespace_policy(mut self, policy: NamespacePolicy) -> Self {
                self.raw = self.raw.namespace_policy(policy);
                self
            }

            /// Applies optional client-side compression before encryption.
            pub fn compression(mut self, compression: Compression) -> Self {
                self.protection.compression = compression;
                self
            }

            /// Selects the authenticated-encryption profile for stored values.
            ///
            /// # Arguments
            ///
            /// * `encryption` - Unprotected, Compact, or Robust value profile.
            ///
            /// # Returns
            ///
            /// This builder with the selected value protection profile.
            pub fn encryption(mut self, encryption: Encryption) -> Self {
                self.protection.encryption = encryption;
                self.protection.encryption_explicit = true;
                self
            }

            /// Retains the pre-contract key-type selector for source
            /// compatibility.
            ///
            /// v1 infers `TypedKey` from each operation; a namespace does not
            /// enforce one key type. New callers should omit this setting.
            #[deprecated(note = "TypedKey is inferred per operation in v1")]
            pub fn key_spec(mut self, key_spec: KeyType) -> Self {
                self.protection.key_spec = key_spec;
                self
            }

            /// Configures immutable value-key IDs for read-old/write-new rotation.
            pub fn value_keyring(mut self, keyring: ValueKeyring) -> Self {
                self.protection.keyring = Some(keyring);
                self
            }

            /// Selects the client-local Item ID mapping profile.
            ///
            /// `NamespaceHash` is the default and binds mapped keys to the
            /// configured root and namespace. `PublicKeyOrHash` must be
            /// selected explicitly when public, cross-namespace key identity
            /// is intended.
            pub fn key_format(mut self, key_format: KeyFormat) -> Self {
                self.protection.key_format = key_format;
                self
            }
        }
    };
}

macro_rules! protected_client_methods {
    ($raw:ty) => {
        /// Borrows the exact-item-ID protocol client owned by this protected client.
        pub fn raw(&self) -> &$raw {
            &self.raw
        }

        /// Verifies the connection and returns the complete request round-trip time.
        pub async fn ping(&self) -> Result<Duration> {
            self.raw.ping().await
        }

        /// Returns the currently selected server-assigned namespace ID.
        pub fn namespace_id(&self) -> Option<u64> {
            self.raw.namespace_id()
        }

        /// Resolves a namespace name and optionally creates it.
        pub async fn namespace_open(
            &self,
            name: impl AsRef<[u8]>,
            create_if_missing: bool,
            policy: Option<NamespacePolicy>,
        ) -> Result<NamespaceDescriptor> {
            self.raw
                .namespace_open(name, create_if_missing, policy)
                .await
        }

        /// Replaces a namespace policy using its current revision.
        pub async fn namespace_update_policy(
            &self,
            namespace_id: u64,
            expected_revision: u64,
            policy: NamespacePolicy,
        ) -> Result<NamespaceDescriptor> {
            self.raw
                .namespace_update_policy(namespace_id, expected_revision, policy)
                .await
        }

        /// Deletes an empty namespace using its current revision.
        pub async fn namespace_delete(
            &self,
            namespace_id: u64,
            expected_revision: u64,
        ) -> Result<()> {
            self.raw
                .namespace_delete(namespace_id, expected_revision)
                .await
        }

        /// Retrieves, authenticates, and decodes a value for a portable key.
        pub async fn get(&self, key: impl Into<TypedKey>) -> Result<GetOutcome<Vec<u8>>> {
            match self.get_value(key).await? {
                GetOutcome::Found(Value::Raw(value)) => Ok(GetOutcome::Found(value)),
                GetOutcome::Found(Value::Json(_)) => {
                    Err(crate::value::Error::ExpectedRawValue.into())
                }
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        /// Retrieves a formatted value in the core logical model.
        ///
        /// # Arguments
        ///
        /// * `application_key` - Exact application key bytes used for item ID derivation.
        ///
        /// # Returns
        ///
        /// The decoded value or a not-found outcome.
        ///
        /// # Errors
        ///
        /// Returns an error when transport, authentication, decompression, or deserialization
        /// fails.
        pub async fn get_value(&self, key: impl Into<TypedKey>) -> Result<GetOutcome<Value>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            self.get_value_at_item_id(namespace_id, item_id).await
        }

        /// Retrieves and validates a canonical JSON helper value for a
        /// portable key. JSON is stored as selector-0 `OpaqueBytes`.
        pub async fn get_json(&self, key: impl Into<TypedKey>) -> Result<GetOutcome<JsonValue>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            self.get_json_at_item_id(namespace_id, item_id).await
        }

        /// Retrieves and decodes a StructuredValue-CBOR-v1 value for a portable key.
        pub async fn get_structured(
            &self,
            key: impl Into<TypedKey>,
        ) -> Result<GetOutcome<StructuredValue>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            self.get_structured_at_item_id(namespace_id, item_id).await
        }

        /// Retrieves a value when the adapter already owns canonical key bytes.
        pub async fn get_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<Value>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key(namespace_id, canonical_key.as_ref())?;
            self.get_value_at_item_id(namespace_id, item_id).await
        }

        /// Retrieves a canonical JSON helper value for canonical key bytes.
        pub async fn get_json_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<JsonValue>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key(namespace_id, canonical_key.as_ref())?;
            self.get_json_at_item_id(namespace_id, item_id).await
        }

        /// Retrieves a StructuredValue-CBOR-v1 value for canonical key bytes.
        pub async fn get_structured_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<StructuredValue>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key(namespace_id, canonical_key.as_ref())?;
            self.get_structured_at_item_id(namespace_id, item_id).await
        }

        /// Retrieves StructuredValue-CBOR-v1 bytes for byte-oriented native
        /// adapters without routing through JSON or Raw conversion.
        pub async fn get_structured_canonical_key_cbor(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<Vec<u8>>> {
            match self.get_structured_canonical_key(canonical_key).await? {
                GetOutcome::Found(value) => self
                    .protection
                    .encode_structured_cbor(&value)
                    .map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        /// Retrieves a value for canonical key bytes supplied by a low-level
        /// language adapter.
        ///
        /// The bytes must be exactly one canonical v1 key item. This method
        /// intentionally does not apply the configured `KeyType`; the CBOR
        /// major type is the low-level ABI's explicit type discriminator.
        #[cfg(feature = "ffi")]
        pub(crate) async fn get_canonical_key_unchecked(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<Value>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key_unchecked(namespace_id, canonical_key.as_ref())?;
            self.get_value_at_item_id(namespace_id, item_id).await
        }

        /// Retrieves a canonical JSON helper value for native ABI key bytes.
        #[cfg(feature = "ffi")]
        pub(crate) async fn get_json_canonical_key_unchecked(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<JsonValue>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key_unchecked(namespace_id, canonical_key.as_ref())?;
            self.get_json_at_item_id(namespace_id, item_id).await
        }

        /// Retrieves a StructuredValue-CBOR-v1 value for canonical key bytes supplied by a
        /// low-level native adapter. The canonical key's explicit type is trusted at this
        /// boundary, while the value envelope and structured payload remain fully validated by
        /// the shared core.
        #[cfg(feature = "ffi")]
        pub(crate) async fn get_structured_canonical_key_unchecked(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<StructuredValue>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key_unchecked(namespace_id, canonical_key.as_ref())?;
            self.get_structured_at_item_id(namespace_id, item_id).await
        }

        /// Retrieves StructuredValue-CBOR-v1 bytes for canonical key bytes
        /// supplied by a low-level native adapter.
        #[cfg(feature = "ffi")]
        pub(crate) async fn get_structured_canonical_key_cbor_unchecked(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<Vec<u8>>> {
            match self
                .get_structured_canonical_key_unchecked(canonical_key)
                .await?
            {
                GetOutcome::Found(value) => self
                    .protection
                    .encode_structured_cbor(&value)
                    .map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        async fn get_value_at_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
        ) -> Result<GetOutcome<Value>> {
            match self
                .raw
                .get_in_namespace_with_permit(namespace_id, item_id)
                .await?
            {
                GetOutcome::Found(value) => self
                    .protection
                    .decode_in_namespace(namespace_id, item_id, value)
                    .map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        async fn get_json_at_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
        ) -> Result<GetOutcome<JsonValue>> {
            match self
                .raw
                .get_in_namespace_with_permit(namespace_id, item_id)
                .await?
            {
                GetOutcome::Found(value) => self
                    .protection
                    .decode_json_in_namespace(namespace_id, item_id, value)
                    .map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        async fn get_structured_at_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
        ) -> Result<GetOutcome<StructuredValue>> {
            match self
                .raw
                .get_in_namespace_with_permit(namespace_id, item_id)
                .await?
            {
                GetOutcome::Found(value) => self
                    .protection
                    .open_structured_in_namespace(namespace_id, item_id, value)
                    .map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        /// Retrieves a formatted JSON helper value for an exact Item ID.
        pub async fn get_json_exact_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
        ) -> Result<GetOutcome<JsonValue>> {
            self.get_json_at_item_id(namespace_id, item_id).await
        }

        /// Retrieves a StructuredValue-CBOR-v1 value for an exact Item ID.
        pub async fn get_structured_exact_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
        ) -> Result<GetOutcome<StructuredValue>> {
            self.get_structured_at_item_id(namespace_id, item_id).await
        }

        /// Retrieves StructuredValue-CBOR-v1 bytes for an exact Item ID.
        pub async fn get_structured_exact_item_id_cbor(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
        ) -> Result<GetOutcome<Vec<u8>>> {
            match self
                .get_structured_exact_item_id(namespace_id, item_id)
                .await?
            {
                GetOutcome::Found(value) => self
                    .protection
                    .encode_structured_cbor(&value)
                    .map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        /// Protects and stores plaintext bytes for a portable key.
        pub async fn set(
            &self,
            key: impl Into<TypedKey>,
            plaintext: Vec<u8>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            self.set_value(key, Value::Raw(plaintext), options).await
        }

        /// Serializes, protects, and stores a core logical value.
        ///
        /// # Arguments
        ///
        /// * `key` - Portable key value used for Item ID derivation.
        /// * `value` - Raw or logical JSON value to encode.
        /// * `options` - Existence condition and optional expiration.
        ///
        /// # Returns
        ///
        /// The server's set outcome.
        ///
        /// # Errors
        ///
        /// Returns an error when serialization, protection, transport, or the server operation
        /// fails.
        pub async fn set_value(
            &self,
            key: impl Into<TypedKey>,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            let value = self
                .protection
                .encode_in_namespace(namespace_id, item_id, value)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Canonicalizes and stores a JSON helper value as selector-0
        /// `OpaqueBytes`.
        pub async fn set_json(
            &self,
            key: impl Into<TypedKey>,
            value: JsonValue,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            let value = self
                .protection
                .encode_json_in_namespace(namespace_id, item_id, value)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Serializes, protects, and stores a StructuredValue-CBOR-v1 value for a portable key.
        pub async fn set_structured(
            &self,
            key: impl Into<TypedKey>,
            value: StructuredValue,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            let value =
                self.protection
                    .seal_structured_in_namespace(namespace_id, item_id, &value)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Stores a JSON helper value at an exact Item ID.
        pub async fn set_json_exact_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
            value: JsonValue,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let value = self
                .protection
                .encode_json_in_namespace(namespace_id, item_id, value)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Stores a StructuredValue-CBOR-v1 value at an exact Item ID.
        pub async fn set_structured_exact_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
            value: StructuredValue,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let value =
                self.protection
                    .seal_structured_in_namespace(namespace_id, item_id, &value)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Decodes, protects, and stores one StructuredValue-CBOR-v1 payload
        /// at an exact Item ID.
        pub async fn set_structured_exact_item_id_cbor(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
            value: impl AsRef<[u8]>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let value = self.protection.seal_structured_cbor_in_namespace(
                namespace_id,
                item_id,
                value.as_ref(),
            )?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Stores a caller-owned version-0 envelope at a portable key.
        pub async fn set_v0(
            &self,
            key: impl Into<TypedKey>,
            bytes: Vec<u8>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            let value = self.protection.pass_through_v0(bytes)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Retrieves a caller-owned version-0 envelope at a portable key.
        pub async fn get_v0(&self, key: impl Into<TypedKey>) -> Result<GetOutcome<Vec<u8>>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            match self
                .raw
                .get_in_namespace_with_permit(namespace_id, item_id)
                .await?
            {
                GetOutcome::Found(value) => self.protection.open_v0(value).map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        /// Stores a caller-owned version-0 envelope at an exact Item ID.
        pub async fn set_v0_exact_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
            bytes: Vec<u8>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let value = self.protection.pass_through_v0(bytes)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Retrieves a caller-owned version-0 envelope at an exact Item ID.
        pub async fn get_v0_exact_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
        ) -> Result<GetOutcome<Vec<u8>>> {
            match self
                .raw
                .get_in_namespace_with_permit(namespace_id, item_id)
                .await?
            {
                GetOutcome::Found(value) => self.protection.open_v0(value).map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        /// Retrieves a caller-owned version-0 envelope for canonical key bytes.
        pub async fn get_v0_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<Vec<u8>>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key(namespace_id, canonical_key.as_ref())?;
            self.get_v0_exact_item_id(namespace_id, item_id).await
        }

        /// Stores a value when the adapter already owns canonical key bytes.
        pub async fn set_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key(namespace_id, canonical_key.as_ref())?;
            let value = self
                .protection
                .encode_in_namespace(namespace_id, item_id, value)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Stores a caller-owned version-0 envelope for canonical key bytes.
        pub async fn set_v0_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
            bytes: Vec<u8>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key(namespace_id, canonical_key.as_ref())?;
            self.set_v0_exact_item_id(namespace_id, item_id, bytes, options)
                .await
        }

        /// Stores a canonical JSON helper value for canonical key bytes.
        pub async fn set_json_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
            value: JsonValue,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key(namespace_id, canonical_key.as_ref())?;
            let value = self
                .protection
                .encode_json_in_namespace(namespace_id, item_id, value)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Stores a StructuredValue-CBOR-v1 value for canonical key bytes.
        pub async fn set_structured_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
            value: StructuredValue,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key(namespace_id, canonical_key.as_ref())?;
            let value =
                self.protection
                    .seal_structured_in_namespace(namespace_id, item_id, &value)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Stores one complete StructuredValue-CBOR-v1 payload for byte
        /// oriented native adapters.
        pub async fn set_structured_canonical_key_cbor(
            &self,
            canonical_key: impl AsRef<[u8]>,
            value: impl AsRef<[u8]>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key(namespace_id, canonical_key.as_ref())?;
            let value = self.protection.seal_structured_cbor_in_namespace(
                namespace_id,
                item_id,
                value.as_ref(),
            )?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Stores a value for canonical key bytes supplied by a low-level
        /// language adapter.
        #[cfg(feature = "ffi")]
        pub(crate) async fn set_canonical_key_unchecked(
            &self,
            canonical_key: impl AsRef<[u8]>,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key_unchecked(namespace_id, canonical_key.as_ref())?;
            let value = self
                .protection
                .encode_in_namespace(namespace_id, item_id, value)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Stores a canonical JSON helper value for native ABI key bytes.
        #[cfg(feature = "ffi")]
        pub(crate) async fn set_json_canonical_key_unchecked(
            &self,
            canonical_key: impl AsRef<[u8]>,
            value: JsonValue,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key_unchecked(namespace_id, canonical_key.as_ref())?;
            let value = self
                .protection
                .encode_json_in_namespace(namespace_id, item_id, value)?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Retrieves a caller-owned version-0 envelope for native ABI key bytes.
        #[cfg(feature = "ffi")]
        pub(crate) async fn get_v0_canonical_key_unchecked(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<Vec<u8>>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key_unchecked(namespace_id, canonical_key.as_ref())?;
            self.get_v0_exact_item_id(namespace_id, item_id).await
        }

        /// Stores a caller-owned version-0 envelope for native ABI key bytes.
        #[cfg(feature = "ffi")]
        pub(crate) async fn set_v0_canonical_key_unchecked(
            &self,
            canonical_key: impl AsRef<[u8]>,
            bytes: Vec<u8>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key_unchecked(namespace_id, canonical_key.as_ref())?;
            self.set_v0_exact_item_id(namespace_id, item_id, bytes, options)
                .await
        }

        /// Decodes, protects, and stores one StructuredValue-CBOR-v1 payload
        /// for canonical key bytes supplied by a low-level native adapter.
        #[cfg(feature = "ffi")]
        pub(crate) async fn set_structured_canonical_key_cbor_unchecked(
            &self,
            canonical_key: impl AsRef<[u8]>,
            value: impl AsRef<[u8]>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key_unchecked(namespace_id, canonical_key.as_ref())?;
            let value = self.protection.seal_structured_cbor_in_namespace(
                namespace_id,
                item_id,
                value.as_ref(),
            )?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        /// Deletes a value for a portable key.
        pub async fn delete(&self, key: impl Into<TypedKey>) -> Result<DeleteOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            self.raw.delete_in_namespace(namespace_id, item_id).await
        }

        /// Deletes a value when the adapter already owns canonical key bytes.
        pub async fn delete_canonical_key(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<DeleteOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key(namespace_id, canonical_key.as_ref())?;
            self.raw.delete_in_namespace(namespace_id, item_id).await
        }

        /// Deletes a value for canonical key bytes supplied by a low-level
        /// language adapter.
        #[cfg(feature = "ffi")]
        pub(crate) async fn delete_canonical_key_unchecked(
            &self,
            canonical_key: impl AsRef<[u8]>,
        ) -> Result<DeleteOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self
                .protection
                .item_id_from_canonical_key_unchecked(namespace_id, canonical_key.as_ref())?;
            self.raw.delete_in_namespace(namespace_id, item_id).await
        }

        /// Returns server statistics as their JSON text.
        pub async fn experimental_stats(&self) -> Result<String> {
            self.raw.experimental_stats().await
        }

        /// Waits until prior mutations satisfy the server durability barrier.
        pub async fn experimental_sync(&self) -> Result<()> {
            self.raw.experimental_sync().await
        }

        /// Returns a best-effort state snapshot that does not guarantee the next request succeeds.
        pub fn connection_state(&self) -> ConnectionState {
            self.raw.connection_state()
        }

        /// Explicitly replaces the current connection without replaying an operation.
        pub async fn reconnect(&self) -> Result<()> {
            self.raw.reconnect().await
        }

        /// Permanently and idempotently closes this client instance.
        pub async fn close(&self) -> Result<()> {
            self.raw.close().await
        }

        /// Returns the aggregate byte budget shared by transport and value work.
        pub fn request_budget(&self) -> crate::RequestBudget {
            self.protection.request_budget()
        }

        /// Returns the configured value resource limits.
        pub fn value_limits(&self) -> crate::value::ValueLimits {
            self.protection.value_limits()
        }
    };
}

#[cfg(feature = "quic-quinn")]
/// Shared protected client running on Tokio and Quinn.
#[derive(Clone)]
pub struct ProtectedClient {
    raw: RawClient,
    protection: Arc<DataProtection>,
}

#[cfg(feature = "quic-quinn")]
/// Connection and data-protection builder for a shared Tokio client.
pub struct ProtectedClientBuilder {
    raw: RawClientBuilder,
    protection: ProtectionSettings,
}

#[cfg(feature = "quic-quinn")]
protected_builder_methods!(ProtectedClientBuilder);

#[cfg(feature = "quic-quinn")]
impl ProtectedClientBuilder {
    /// Connects a client with mandatory application-key and value protection.
    pub async fn connect(self) -> Result<ProtectedClient> {
        let raw = self.raw.connect().await?;
        let protection = self.protection.finish_with_budget(raw.request_budget())?;
        Ok(ProtectedClient { raw, protection })
    }
}

#[cfg(feature = "quic-quinn")]
impl ProtectedClient {
    /// Connects with mandatory data protection, system trust, and default client behavior.
    pub async fn connect(endpoint: &str, key: DataProtectionKey) -> Result<Self> {
        Self::builder(endpoint.parse()?, key).connect().await
    }

    /// Starts explicit shared client configuration.
    pub fn builder(endpoint: Endpoint, key: DataProtectionKey) -> ProtectedClientBuilder {
        ProtectedClientBuilder {
            raw: RawClient::builder(endpoint),
            protection: ProtectionSettings::new(key),
        }
    }

    /// Starts a client with an explicit Item-ID root and independent value
    /// keyring.
    ///
    /// [`ClientRootKey::public`] (or [`ClientRootKey::zero`]) intentionally
    /// selects publicly derivable Item IDs. Value confidentiality comes only
    /// from the supplied keyring.
    pub fn builder_with_keyring(
        endpoint: Endpoint,
        item_id_root: ClientRootKey,
        keyring: ValueKeyring,
    ) -> ProtectedClientBuilder {
        ProtectedClientBuilder {
            raw: RawClient::builder(endpoint),
            protection: ProtectionSettings {
                key: Some(item_id_root),
                keyring: Some(keyring),
                ..ProtectionSettings::unprotected()
            },
        }
    }

    /// Starts an explicitly unprotected client with namespace-bound Item IDs.
    pub fn builder_unprotected(endpoint: Endpoint) -> ProtectedClientBuilder {
        ProtectedClientBuilder {
            raw: RawClient::builder(endpoint),
            protection: ProtectionSettings::unprotected(),
        }
    }

    protected_client_methods!(RawClient);
}

#[cfg(feature = "tls-tcp")]
/// Shared protected client running on Tokio and TLS-over-TCP.
#[derive(Clone)]
pub struct TlsTcpProtectedClient {
    raw: TlsTcpRawClient,
    protection: Arc<DataProtection>,
}

#[cfg(feature = "tls-tcp")]
/// Connection and data-protection builder for the TLS-over-TCP client.
pub struct TlsTcpProtectedClientBuilder {
    raw: TlsTcpRawClientBuilder,
    protection: ProtectionSettings,
}

#[cfg(feature = "tls-tcp")]
protected_builder_methods!(TlsTcpProtectedClientBuilder);

#[cfg(feature = "tls-tcp")]
impl TlsTcpProtectedClientBuilder {
    /// Connects a TLS-over-TCP client with mandatory application-key and value protection.
    pub async fn connect(self) -> Result<TlsTcpProtectedClient> {
        let raw = self.raw.connect().await?;
        let protection = self.protection.finish_with_budget(raw.request_budget())?;
        Ok(TlsTcpProtectedClient { raw, protection })
    }
}

#[cfg(feature = "tls-tcp")]
impl TlsTcpProtectedClient {
    /// Connects with mandatory data protection, system trust, and default TLS-over-TCP behavior.
    pub async fn connect(endpoint: &str, key: DataProtectionKey) -> Result<Self> {
        Self::builder(endpoint.parse()?, key).connect().await
    }

    /// Starts explicit TLS-over-TCP client configuration.
    pub fn builder(endpoint: Endpoint, key: DataProtectionKey) -> TlsTcpProtectedClientBuilder {
        TlsTcpProtectedClientBuilder {
            raw: TlsTcpRawClient::builder(endpoint),
            protection: ProtectionSettings::new(key),
        }
    }

    /// Starts an explicitly unprotected TLS-over-TCP client.
    pub fn builder_unprotected(endpoint: Endpoint) -> TlsTcpProtectedClientBuilder {
        TlsTcpProtectedClientBuilder {
            raw: TlsTcpRawClient::builder(endpoint),
            protection: ProtectionSettings::unprotected(),
        }
    }

    protected_client_methods!(TlsTcpRawClient);
}

#[cfg(feature = "quic-compio")]
/// Shared protected client confined to a Compio runtime.
#[derive(Clone)]
pub struct LocalProtectedClient {
    raw: LocalRawClient,
    protection: Arc<DataProtection>,
}

#[cfg(feature = "quic-compio")]
/// Connection and data-protection builder for a shared Compio client.
pub struct LocalProtectedClientBuilder {
    raw: LocalRawClientBuilder,
    protection: ProtectionSettings,
}

#[cfg(feature = "quic-compio")]
protected_builder_methods!(LocalProtectedClientBuilder);

#[cfg(feature = "quic-compio")]
impl LocalProtectedClientBuilder {
    /// Connects a Compio client with mandatory application-key and value protection.
    pub async fn connect(self) -> Result<LocalProtectedClient> {
        let raw = self.raw.connect().await?;
        let protection = self.protection.finish_with_budget(raw.request_budget())?;
        Ok(LocalProtectedClient { raw, protection })
    }
}

#[cfg(feature = "quic-compio")]
impl LocalProtectedClient {
    /// Connects with mandatory data protection, system trust, and default Compio behavior.
    pub async fn connect(endpoint: &str, key: DataProtectionKey) -> Result<Self> {
        Self::builder(endpoint.parse()?, key).connect().await
    }

    /// Starts explicit shared Compio client configuration.
    pub fn builder(endpoint: Endpoint, key: DataProtectionKey) -> LocalProtectedClientBuilder {
        LocalProtectedClientBuilder {
            raw: LocalRawClient::builder(endpoint),
            protection: ProtectionSettings::new(key),
        }
    }

    /// Starts a Compio client with an explicit Item-ID root and independent
    /// value keyring.
    ///
    /// [`ClientRootKey::public`] (or [`ClientRootKey::zero`]) intentionally
    /// selects publicly derivable Item IDs. Value confidentiality comes only
    /// from the supplied keyring.
    pub fn builder_with_keyring(
        endpoint: Endpoint,
        item_id_root: ClientRootKey,
        keyring: ValueKeyring,
    ) -> LocalProtectedClientBuilder {
        LocalProtectedClientBuilder {
            raw: LocalRawClient::builder(endpoint),
            protection: ProtectionSettings {
                key: Some(item_id_root),
                keyring: Some(keyring),
                ..ProtectionSettings::unprotected()
            },
        }
    }

    /// Starts an explicitly unprotected client with namespace-bound Item IDs.
    pub fn builder_unprotected(endpoint: Endpoint) -> LocalProtectedClientBuilder {
        LocalProtectedClientBuilder {
            raw: LocalRawClient::builder(endpoint),
            protection: ProtectionSettings::unprotected(),
        }
    }

    protected_client_methods!(LocalRawClient);
}
