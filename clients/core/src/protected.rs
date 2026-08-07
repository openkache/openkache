//! Shared application-key and plaintext-value clients for language bindings.

use std::sync::Arc;
use std::time::Duration;

use crate::value::{Compression, Encryption, Value};
use crate::{
    AlpnPolicy, Certificate, ClientIdentity, ClientTimeouts, ConnectionState, DataProtection,
    DataProtectionKey, DeleteOutcome, Endpoint, GetOutcome, KeySpec, NamespaceDescriptor,
    NamespacePolicy, PortableKey, Result, RetryPolicy, ServerTrust, SetOptions, SetOutcome,
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
    key_spec: KeySpec,
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
            key_spec: KeySpec::Bytes,
        }
    }

    fn finish(self) -> Result<Arc<DataProtection>> {
        match self.key {
            Some(key) => if self.encryption == Encryption::Unprotected {
                DataProtection::unprotected(self.key_spec, self.compression)
            } else {
                DataProtection::with_profile_and_key_spec(
                    key,
                    self.key_spec,
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
                DataProtection::unprotected(self.key_spec, self.compression).map(Arc::new)
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
            pub fn key_spec(mut self, key_spec: KeySpec) -> Self {
                self.protection.key_spec = key_spec;
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
        pub async fn get(&self, key: impl Into<PortableKey>) -> Result<GetOutcome<Vec<u8>>> {
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
        pub async fn get_value(&self, key: impl Into<PortableKey>) -> Result<GetOutcome<Value>> {
            let namespace_id = self.raw.ensure_namespace_id().await?;
            let item_id = self.protection.item_id_in_namespace(namespace_id, key)?;
            self.get_value_at_item_id(namespace_id, item_id).await
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

        /// Retrieves a value for canonical key bytes supplied by a low-level
        /// language adapter.
        ///
        /// The bytes must be exactly one canonical v1 key item. This method
        /// intentionally does not apply the configured `KeySpec`; the CBOR
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

        async fn get_value_at_item_id(
            &self,
            namespace_id: u64,
            item_id: crate::ItemId,
        ) -> Result<GetOutcome<Value>> {
            match self.raw.get_in_namespace(namespace_id, item_id).await? {
                GetOutcome::Found(value) => self
                    .protection
                    .decode_in_namespace(namespace_id, item_id, value)
                    .map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        /// Protects and stores plaintext bytes for a portable key.
        pub async fn set(
            &self,
            key: impl Into<PortableKey>,
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
            key: impl Into<PortableKey>,
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

        /// Deletes a value for a portable key.
        pub async fn delete(&self, key: impl Into<PortableKey>) -> Result<DeleteOutcome> {
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
