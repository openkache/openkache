//! Shared application-key and plaintext-value clients for language bindings.

use std::sync::Arc;
use std::time::Duration;

use crate::value::{Compression, Encryption, Value};
use crate::{
    Certificate, ClientIdentity, ClientTimeouts, ConnectionState, CoreMetricsSnapshot,
    DataProtection, DataProtectionKey, DataProtectionKeyRing, DeleteOutcome, Endpoint, GetOutcome,
    MutationId, Result, RetryPolicy, ServerTrust, SetOptions, SetOutcome,
};
#[cfg(feature = "quic-compio")]
use crate::{LocalRawClient, LocalRawClientBuilder};
#[cfg(feature = "quic-quinn")]
use crate::{RawClient, RawClientBuilder};
use crate::{key::random_mutation_id, key::scoped_mutation_id};

struct ProtectionSettings {
    compression: Compression,
    encryption: Encryption,
    key_ring: DataProtectionKeyRing,
}

impl ProtectionSettings {
    fn new(key: DataProtectionKey) -> Self {
        Self {
            compression: Compression::Disabled,
            encryption: Encryption::Robust,
            key_ring: DataProtectionKeyRing::new(key),
        }
    }

    fn finish(self) -> Result<Arc<DataProtection>> {
        DataProtection::with_key_ring(self.key_ring, self.compression, self.encryption)
            .map(Arc::new)
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
                self
            }

            /// Uses an active key plus a bounded set of previous keys for rotation.
            pub fn key_ring(mut self, key_ring: DataProtectionKeyRing) -> Self {
                self.protection.key_ring = key_ring;
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

        /// Retrieves, authenticates, and decodes a value for arbitrary application key bytes.
        pub async fn get(&self, application_key: impl AsRef<[u8]>) -> Result<GetOutcome<Vec<u8>>> {
            match self.get_value(application_key).await? {
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
        pub async fn get_value(
            &self,
            application_key: impl AsRef<[u8]>,
        ) -> Result<GetOutcome<Value>> {
            for item_id in self.protection.item_ids(application_key.as_ref()) {
                match self.raw.get(item_id).await? {
                    GetOutcome::Found(value) => {
                        return self
                            .protection
                            .decode(item_id, value)
                            .map(GetOutcome::Found);
                    }
                    GetOutcome::NotFound => {}
                }
            }
            Ok(GetOutcome::NotFound)
        }

        /// Protects and stores plaintext bytes for arbitrary application key bytes.
        pub async fn set(
            &self,
            application_key: impl AsRef<[u8]>,
            plaintext: Vec<u8>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            self.set_value(application_key, Value::Raw(plaintext), options)
                .await
        }

        /// Serializes, protects, and stores a core logical value.
        ///
        /// # Arguments
        ///
        /// * `application_key` - Exact application key bytes used for item ID derivation.
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
            application_key: impl AsRef<[u8]>,
            value: Value,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let item_id = self.protection.item_id(application_key);
            let value = self.protection.encode(item_id, value)?;
            self.raw.set(item_id, value, options).await
        }

        /// Deletes a value for arbitrary application key bytes.
        pub async fn delete(&self, application_key: impl AsRef<[u8]>) -> Result<DeleteOutcome> {
            self.delete_with_mutation_id(application_key, random_mutation_id()?)
                .await
        }

        /// Deletes an application key while reusing an idempotency token.
        pub async fn delete_with_mutation_id(
            &self,
            application_key: impl AsRef<[u8]>,
            mutation_id: MutationId,
        ) -> Result<DeleteOutcome> {
            for item_id in self.protection.item_ids(application_key.as_ref()) {
                match self
                    .raw
                    .delete_with_mutation_id(item_id, scoped_mutation_id(mutation_id, item_id))
                    .await?
                {
                    DeleteOutcome::Deleted => return Ok(DeleteOutcome::Deleted),
                    DeleteOutcome::NotFound => {}
                }
            }
            Ok(DeleteOutcome::NotFound)
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

        /// Returns retry, reconnect, and transport/protocol error counters
        /// collected by the shared core.
        pub fn metrics_snapshot(&self) -> CoreMetricsSnapshot {
            self.raw.metrics_snapshot()
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

    /// Starts a builder using an active key and a bounded read/delete rotation window.
    pub fn builder_with_key_ring(
        endpoint: Endpoint,
        key_ring: DataProtectionKeyRing,
    ) -> ProtectedClientBuilder {
        ProtectedClientBuilder {
            raw: RawClient::builder(endpoint),
            protection: ProtectionSettings {
                compression: Compression::Disabled,
                encryption: Encryption::Robust,
                key_ring,
            },
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

    /// Starts a builder using an active key and a bounded read/delete rotation window.
    pub fn builder_with_key_ring(
        endpoint: Endpoint,
        key_ring: DataProtectionKeyRing,
    ) -> LocalProtectedClientBuilder {
        LocalProtectedClientBuilder {
            raw: LocalRawClient::builder(endpoint),
            protection: ProtectionSettings {
                compression: Compression::Disabled,
                encryption: Encryption::Robust,
                key_ring,
            },
        }
    }

    protected_client_methods!(LocalRawClient);
}
