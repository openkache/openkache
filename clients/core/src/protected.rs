//! Shared application-key and plaintext-value clients for language bindings.

use std::sync::Arc;
use std::time::Duration;

use crate::value::Compression;
use crate::{
    Certificate, ClientIdentity, ClientTimeouts, ConnectionState, DataProtection,
    DataProtectionKey, DeleteOutcome, Endpoint, GetOutcome, Result, RetryPolicy, ServerTrust,
    SetOptions, SetOutcome,
};
#[cfg(feature = "quic-compio")]
use crate::{LocalRawClient, LocalRawClientBuilder};
#[cfg(feature = "quic-quinn")]
use crate::{RawClient, RawClientBuilder};

struct ProtectionSettings {
    compression: Compression,
    key: DataProtectionKey,
}

impl ProtectionSettings {
    fn new(key: DataProtectionKey) -> Self {
        Self {
            compression: Compression::Disabled,
            key,
        }
    }

    fn finish(self) -> Result<Arc<DataProtection>> {
        DataProtection::new(self.key, self.compression).map(Arc::new)
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
        }
    };
}

macro_rules! protected_client_methods {
    ($raw:ty) => {
        /// Borrows the exact-key protocol client owned by this protected client.
        pub fn raw(&self) -> &$raw {
            &self.raw
        }

        /// Verifies the connection and returns the complete request round-trip time.
        pub async fn ping(&self) -> Result<Duration> {
            self.raw.ping().await
        }

        /// Retrieves, authenticates, and decodes a value for arbitrary application key bytes.
        pub async fn get(&self, application_key: impl AsRef<[u8]>) -> Result<GetOutcome<Vec<u8>>> {
            let key = self.protection.item_key(application_key);
            match self.raw.get(key).await? {
                GetOutcome::Found(value) => self.protection.open(key, value).map(GetOutcome::Found),
                GetOutcome::NotFound => Ok(GetOutcome::NotFound),
            }
        }

        /// Protects and stores plaintext bytes for arbitrary application key bytes.
        pub async fn set(
            &self,
            application_key: impl AsRef<[u8]>,
            plaintext: Vec<u8>,
            options: SetOptions,
        ) -> Result<SetOutcome> {
            let key = self.protection.item_key(application_key);
            let value = self.protection.seal_owned(key, plaintext)?;
            self.raw.set(key, value, options).await
        }

        /// Deletes a value for arbitrary application key bytes.
        pub async fn delete(&self, application_key: impl AsRef<[u8]>) -> Result<DeleteOutcome> {
            self.raw
                .delete(self.protection.item_key(application_key))
                .await
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

    protected_client_methods!(LocalRawClient);
}
