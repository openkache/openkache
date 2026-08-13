//! Shared application-key and plaintext-value clients for language bindings.

use std::sync::Arc;
use std::time::Duration;

use crate::value::{Compression, Encryption, Value};
use crate::{
    AlpnPolicy, Certificate, ClientIdentity, ClientTimeouts, ConnectionState, DataProtection,
    DataProtectionKey, DeleteOutcome, Endpoint, GetOutcome, KeyFormat, KeyType,
    NamespaceDescriptor, NamespacePolicy, Result, RetryPolicy, ServerTrust, SetOptions, SetOutcome,
    TypedKey,
};
#[cfg(feature = "quic-compio")]
use crate::{LocalRawClient, LocalRawClientBuilder};
#[cfg(feature = "quic-quinn")]
use crate::{RawClient, RawClientBuilder};

struct ProtectionSettings {
    compression: Compression,
    encryption: Encryption,
    encryption_explicit: bool,
    key: Option<DataProtectionKey>,
    key_type: KeyType,
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
            compression: Compression::Disabled,
            encryption: Encryption::Robust,
            encryption_explicit: false,
            key,
            key_type: KeyType::Bytes,
            key_format: KeyFormat::Hash,
        }
    }

    fn finish(self) -> Result<Arc<DataProtection>> {
        match self.key {
            Some(key) => if self.encryption == Encryption::Unprotected {
                DataProtection::unprotected_with_root(
                    key,
                    self.key_type,
                    self.key_format,
                    self.compression,
                )
            } else {
                DataProtection::with_key_type_and_format_profile(
                    key,
                    self.key_type,
                    self.key_format,
                    self.compression,
                    self.encryption,
                )
            }
            .map(Arc::new),
            None => {
                if self.encryption_explicit && self.encryption != Encryption::Unprotected {
                    return Err(crate::Error::configuration(
                        "encryption",
                        "an encryption profile requires client_root_key",
                    ));
                }
                DataProtection::unprotected_with_format(
                    self.key_type,
                    self.key_format,
                    self.compression,
                )
                .map(Arc::new)
            }
        }
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

            /// Offers protocol versions in descending order and enforces a minimum version.
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

            /// Bounds simultaneous request lanes on one QUIC connection.
            pub fn max_in_flight(mut self, maximum: usize) -> Self {
                self.raw = self.raw.max_in_flight(maximum);
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
            /// * `encryption` - Compact or Robust authenticated-encryption profile.
            ///
            /// # Returns
            ///
            /// This builder with the selected value protection profile.
            pub fn encryption(mut self, encryption: Encryption) -> Self {
                self.protection.encryption = encryption;
                self.protection.encryption_explicit = true;
                self
            }

            /// Selects the exact key type accepted by this formatted keyspace.
            pub fn key_type(mut self, key_type: KeyType) -> Self {
                self.protection.key_type = key_type;
                self
            }

            /// Compatibility spelling for [`Self::key_type`].
            pub fn key_spec(self, key_spec: KeyType) -> Self {
                self.key_type(key_spec)
            }

            /// Selects the client-only key mapping profile.
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

        /// Retrieves, authenticates, and decodes a value for a typed key.
        pub async fn get(&self, key: impl Into<TypedKey>) -> Result<GetOutcome<Vec<u8>>> {
            match self.get_value(key).await? {
                GetOutcome::Found(Value::Raw(value)) => Ok(GetOutcome::Found(value)),
                GetOutcome::Found(Value::Cbor(_)) | GetOutcome::Found(Value::Json(_)) => {
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
            self.get_value_with_profile(key, None).await
        }

        /// Retrieves and decodes a value with an operation-local protection profile.
        ///
        /// `None` uses the client instance default. An explicit profile must match the
        /// envelope and applies only to this operation.
        pub async fn get_value_with_profile(
            &self,
            key: impl Into<TypedKey>,
            encryption: Option<Encryption>,
        ) -> Result<GetOutcome<Value>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            self.get_value_at_item_id(namespace_id, item_id, encryption)
                .await
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
            self.get_value_at_item_id(namespace_id, item_id, None).await
        }

        /// Retrieves a value from logical key bytes using the configured key
        /// type and mapping profile.
        ///
        /// This is intended for language adapters whose native values are
        /// already represented as UTF-8, decimal integer, or direct byte
        /// sequences. Unlike `get_canonical_key`, it does not expect a CBOR
        /// wrapper around the logical key.
        pub async fn get_logical_key(
            &self,
            key_type: KeyType,
            logical_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<Value>> {
            self.get_logical_key_with_profile(key_type, logical_key, None)
                .await
        }

        /// Retrieves a logical key with an operation-local protection profile.
        pub async fn get_logical_key_with_profile(
            &self,
            key_type: KeyType,
            logical_key: impl AsRef<[u8]>,
            encryption: Option<Encryption>,
        ) -> Result<GetOutcome<Value>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_from_logical_bytes_for_type(
                namespace_id,
                key_type,
                logical_key.as_ref(),
            )?;
            self.get_value_at_item_id(namespace_id, item_id, encryption)
                .await
        }

        #[cfg(feature = "ffi")]
        pub(crate) async fn get_key_for_ffi(
            &self,
            key: &[u8],
            key_type: Option<KeyType>,
            encryption: Option<Encryption>,
        ) -> Result<GetOutcome<Value>> {
            match key_type {
                Some(key_type) => {
                    self.get_logical_key_with_profile(key_type, key, encryption)
                        .await
                }
                None => {
                    let namespace_id = self.raw.ensure_namespace_id().await?;
                    let item_id = self
                        .protection
                        .item_id_from_canonical_key_unchecked(namespace_id, key)?;
                    self.get_value_at_item_id(namespace_id, item_id, encryption)
                        .await
                }
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
            self.get_value_at_item_id(namespace_id, item_id, None).await
        }

        async fn get_value_at_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
            encryption: Option<Encryption>,
        ) -> Result<GetOutcome<Value>> {
            match self.raw.get_in_namespace(namespace_id, item_id).await? {
                GetOutcome::Found(value) => self
                    .protection
                    .decode_in_namespace_with_optional_profile(
                        namespace_id,
                        item_id,
                        value,
                        encryption,
                    )
                    .map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        /// Protects and stores plaintext bytes for a typed key.
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
        /// * `key` - Typed key value used for Item ID derivation.
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
            self.set_value_with_profile(key, value, options, None).await
        }

        /// Serializes and stores a value with an operation-local protection profile.
        ///
        /// `None` uses the client instance default. An explicit profile applies only to this
        /// operation and does not mutate the client.
        pub async fn set_value_with_profile(
            &self,
            key: impl Into<TypedKey>,
            value: Value,
            options: SetOptions,
            encryption: Option<Encryption>,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            let value = self.protection.encode_in_namespace_with_optional_profile(
                namespace_id,
                item_id,
                value,
                encryption,
            )?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
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

        /// Stores a value from logical key bytes using the configured profile.
        pub async fn set_logical_key(
            &self,
            key_type: KeyType,
            logical_key: impl AsRef<[u8]>,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            self.set_logical_key_with_profile(key_type, logical_key, value, options, None)
                .await
        }

        /// Stores a logical key with an operation-local protection profile.
        pub async fn set_logical_key_with_profile(
            &self,
            key_type: KeyType,
            logical_key: impl AsRef<[u8]>,
            value: Value,
            options: SetOptions,
            encryption: Option<Encryption>,
        ) -> Result<SetOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_from_logical_bytes_for_type(
                namespace_id,
                key_type,
                logical_key.as_ref(),
            )?;
            let value = self.protection.encode_in_namespace_with_optional_profile(
                namespace_id,
                item_id,
                value,
                encryption,
            )?;
            self.raw
                .set_in_namespace(namespace_id, item_id, value, options)
                .await
        }

        #[cfg(feature = "ffi")]
        pub(crate) async fn set_key_for_ffi(
            &self,
            key: &[u8],
            key_type: Option<KeyType>,
            value: Value,
            options: SetOptions,
            encryption: Option<Encryption>,
        ) -> Result<SetOutcome> {
            match key_type {
                Some(key_type) => {
                    self.set_logical_key_with_profile(key_type, key, value, options, encryption)
                        .await
                }
                None => {
                    let namespace_id = self.raw.ensure_namespace_id().await?;
                    let item_id = self
                        .protection
                        .item_id_from_canonical_key_unchecked(namespace_id, key)?;
                    let value = self.protection.encode_in_namespace_with_optional_profile(
                        namespace_id,
                        item_id,
                        value,
                        encryption,
                    )?;
                    self.raw
                        .set_in_namespace(namespace_id, item_id, value, options)
                        .await
                }
            }
        }

        /// Deletes a value for a typed key.
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

        /// Deletes a value from logical key bytes using the configured profile.
        pub async fn delete_logical_key(
            &self,
            key_type: KeyType,
            logical_key: impl AsRef<[u8]>,
        ) -> Result<DeleteOutcome> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_from_logical_bytes_for_type(
                namespace_id,
                key_type,
                logical_key.as_ref(),
            )?;
            self.raw.delete_in_namespace(namespace_id, item_id).await
        }

        #[cfg(feature = "ffi")]
        pub(crate) async fn delete_key_for_ffi(
            &self,
            key: &[u8],
            key_type: Option<KeyType>,
            _encryption: Option<Encryption>,
        ) -> Result<DeleteOutcome> {
            match key_type {
                Some(key_type) => self.delete_logical_key(key_type, key).await,
                None => self.delete_canonical_key_unchecked(key).await,
            }
        }

        /// Returns server statistics as their JSON text.
        pub async fn stats(&self) -> Result<String> {
            self.raw.stats().await
        }

        /// Waits until prior mutations satisfy the server durability barrier.
        pub async fn sync(&self) -> Result<()> {
            self.raw.sync().await
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
        let protection = self.protection.finish()?;
        let raw = self.raw.connect().await?;
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

    /// Starts an explicitly unprotected client with namespace-bound Item IDs.
    pub fn builder_unprotected(endpoint: Endpoint) -> ProtectedClientBuilder {
        ProtectedClientBuilder {
            raw: RawClient::builder(endpoint),
            protection: ProtectionSettings::unprotected(),
        }
    }

    protected_client_methods!(RawClient);
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
        let protection = self.protection.finish()?;
        let raw = self.raw.connect().await?;
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

    /// Starts an explicitly unprotected client with namespace-bound Item IDs.
    pub fn builder_unprotected(endpoint: Endpoint) -> LocalProtectedClientBuilder {
        LocalProtectedClientBuilder {
            raw: LocalRawClient::builder(endpoint),
            protection: ProtectionSettings::unprotected(),
        }
    }

    protected_client_methods!(LocalRawClient);
}
