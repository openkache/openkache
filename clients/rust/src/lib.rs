//! Ergonomic Rust API layered over the reusable OpenKache client core.

#[cfg(feature = "ffi")]
pub mod ffi;

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

pub use openkache_client_core::{
    Backend, Certificate, ClientIdentity, ClientTimeouts, ConnectionState,
    DATA_PROTECTION_KEY_BYTES, DataProtection, DataProtectionKey, DeleteOutcome, Endpoint, Error,
    GetOutcome, ITEM_KEY_BYTES, ItemKey, ItemValue, Operation, PrivateKey, Result, RetryPolicy,
    ServerErrorCode, ServerTrust, SetCondition, SetOptions, SetOutcome, value, value_envelope,
};
#[cfg(feature = "quic-compio")]
use openkache_client_core::{
    LocalProtectedClient as SharedLocalClient,
    LocalProtectedClientBuilder as SharedLocalClientBuilder,
};
#[cfg(feature = "quic-compio")]
pub use openkache_client_core::{LocalRawClient, LocalRawClientBuilder};
#[cfg(feature = "quic-quinn")]
use openkache_client_core::{
    ProtectedClient as SharedClient, ProtectedClientBuilder as SharedClientBuilder,
};
#[cfg(feature = "quic-quinn")]
pub use openkache_client_core::{RawClient, RawClientBuilder};

macro_rules! builder_methods {
    ($builder:ident) => {
        impl $builder {
            /// Uses only the supplied server trust roots.
            pub fn server_trust(mut self, trust: ServerTrust) -> Self {
                self.inner = self.inner.server_trust(trust);
                self
            }

            /// Trusts one explicit CA or self-signed server certificate.
            pub fn trust_certificate(mut self, certificate: Certificate) -> Self {
                self.inner = self.inner.trust_certificate(certificate);
                self
            }

            /// Presents a mutual TLS client identity.
            pub fn client_identity(mut self, identity: ClientIdentity) -> Self {
                self.inner = self.inner.client_identity(identity);
                self
            }

            /// Sets connection and complete-request deadlines.
            pub fn timeouts(mut self, timeouts: ClientTimeouts) -> Self {
                self.inner = self.inner.timeouts(timeouts);
                self
            }

            /// Sets retry attempts for response-safe operations.
            pub fn retry_policy(mut self, retry: RetryPolicy) -> Self {
                self.inner = self.inner.retry_policy(retry);
                self
            }

            /// Bounds simultaneous request lanes on one QUIC connection.
            pub fn max_in_flight(mut self, maximum: usize) -> Self {
                self.inner = self.inner.max_in_flight(maximum);
                self
            }

            /// Applies optional client-side compression before encryption.
            pub fn compression(mut self, compression: value::Compression) -> Self {
                self.inner = self.inner.compression(compression);
                self
            }
        }
    };
}

macro_rules! client_methods {
    ($client:ident) => {
        impl $client {
            /// Verifies the connection and returns the complete request round-trip time.
            pub async fn ping(&self) -> Result<Duration> {
                self.inner.ping().await
            }

            /// Retrieves and decodes a value for arbitrary application key bytes.
            pub async fn get(
                &self,
                application_key: impl AsRef<[u8]>,
            ) -> Result<GetOutcome<Vec<u8>>> {
                self.inner.get(application_key).await
            }

            /// Deletes a value for arbitrary application key bytes.
            pub async fn delete(&self, application_key: impl AsRef<[u8]>) -> Result<DeleteOutcome> {
                self.inner.delete(application_key).await
            }

            /// Returns server statistics as their JSON text.
            pub async fn stats(&self) -> Result<String> {
                self.inner.stats().await
            }

            /// Waits until prior mutations satisfy the server durability barrier.
            pub async fn sync(&self) -> Result<()> {
                self.inner.sync().await
            }

            /// Returns a best-effort state snapshot that does not guarantee the next request succeeds.
            pub fn connection_state(&self) -> ConnectionState {
                self.inner.connection_state()
            }

            /// Explicitly replaces the current connection without replaying an operation.
            pub async fn reconnect(&self) -> Result<()> {
                self.inner.reconnect().await
            }

            /// Permanently and idempotently closes this client instance.
            pub async fn close(&self) -> Result<()> {
                self.inner.close().await
            }
        }
    };
}

#[cfg(feature = "quic-quinn")]
/// High-level application-key and plaintext-value client running on Tokio and Quinn.
#[derive(Clone)]
pub struct Client {
    inner: SharedClient,
}

#[cfg(feature = "quic-quinn")]
/// Connection and value-layer builder for Tokio clients.
pub struct ClientBuilder {
    inner: SharedClientBuilder,
}

#[cfg(feature = "quic-quinn")]
builder_methods!(ClientBuilder);

#[cfg(feature = "quic-quinn")]
impl ClientBuilder {
    /// Connects a Tokio/Quinn client with the configured layers.
    pub async fn connect(self) -> Result<Client> {
        self.inner.connect().await.map(|inner| Client { inner })
    }
}

#[cfg(feature = "quic-quinn")]
impl Client {
    /// Connects with mandatory data protection, system trust, and default client behavior.
    pub async fn connect(endpoint: &str, data_protection_key: DataProtectionKey) -> Result<Self> {
        Self::builder(endpoint.parse()?, data_protection_key)
            .connect()
            .await
    }

    /// Starts explicit client configuration.
    pub fn builder(endpoint: Endpoint, data_protection_key: DataProtectionKey) -> ClientBuilder {
        ClientBuilder {
            inner: SharedClient::builder(endpoint, data_protection_key),
        }
    }

    /// Borrows the exact protocol layer owned by this client.
    pub fn raw(&self) -> &RawClient {
        self.inner.raw()
    }

    /// Starts an awaitable set request with persistent, unconditional defaults.
    pub fn set<'a>(
        &'a self,
        application_key: impl AsRef<[u8]>,
        value: impl IntoValue,
    ) -> SetRequest<'a> {
        SetRequest {
            client: self,
            application_key: application_key.as_ref().to_vec(),
            value: value.into_value(),
            options: SetOptions::new(),
        }
    }
}

#[cfg(feature = "quic-quinn")]
client_methods!(Client);

#[cfg(feature = "quic-compio")]
/// High-level application-key and plaintext-value client confined to a Compio runtime.
#[derive(Clone)]
pub struct LocalClient {
    inner: SharedLocalClient,
}

#[cfg(feature = "quic-compio")]
/// Connection and value-layer builder for Compio clients.
pub struct LocalClientBuilder {
    inner: SharedLocalClientBuilder,
}

#[cfg(feature = "quic-compio")]
builder_methods!(LocalClientBuilder);

#[cfg(feature = "quic-compio")]
impl LocalClientBuilder {
    /// Connects a high-level Compio client with the configured layers.
    pub async fn connect(self) -> Result<LocalClient> {
        self.inner
            .connect()
            .await
            .map(|inner| LocalClient { inner })
    }
}

#[cfg(feature = "quic-compio")]
impl LocalClient {
    /// Connects with mandatory data protection, system trust, and default Compio behavior.
    pub async fn connect(endpoint: &str, data_protection_key: DataProtectionKey) -> Result<Self> {
        Self::builder(endpoint.parse()?, data_protection_key)
            .connect()
            .await
    }

    /// Starts explicit Compio client configuration.
    pub fn builder(
        endpoint: Endpoint,
        data_protection_key: DataProtectionKey,
    ) -> LocalClientBuilder {
        LocalClientBuilder {
            inner: SharedLocalClient::builder(endpoint, data_protection_key),
        }
    }

    /// Borrows the exact protocol layer owned by this client.
    pub fn raw(&self) -> &LocalRawClient {
        self.inner.raw()
    }

    /// Starts an awaitable set request with persistent, unconditional defaults.
    pub fn set<'a>(
        &'a self,
        application_key: impl AsRef<[u8]>,
        value: impl IntoValue,
    ) -> LocalSetRequest<'a> {
        LocalSetRequest {
            client: self,
            application_key: application_key.as_ref().to_vec(),
            value: value.into_value(),
            options: SetOptions::new(),
        }
    }
}

#[cfg(feature = "quic-compio")]
client_methods!(LocalClient);

/// Conversion into an owned value buffer without separate borrowed and owned method names.
pub trait IntoValue {
    /// Converts the value to the owned buffer used by an asynchronous request.
    fn into_value(self) -> Vec<u8>;
}

impl IntoValue for Vec<u8> {
    fn into_value(self) -> Vec<u8> {
        self
    }
}

impl IntoValue for &[u8] {
    fn into_value(self) -> Vec<u8> {
        self.to_vec()
    }
}

impl<const N: usize> IntoValue for &[u8; N] {
    fn into_value(self) -> Vec<u8> {
        self.to_vec()
    }
}

impl IntoValue for &Vec<u8> {
    fn into_value(self) -> Vec<u8> {
        self.clone()
    }
}

impl IntoValue for &Arc<Vec<u8>> {
    fn into_value(self) -> Vec<u8> {
        self.as_ref().clone()
    }
}

#[cfg(feature = "quic-quinn")]
/// Awaitable Tokio set request with optional condition and TTL modifiers.
pub struct SetRequest<'a> {
    client: &'a Client,
    application_key: Vec<u8>,
    value: Vec<u8>,
    options: SetOptions,
}

#[cfg(feature = "quic-quinn")]
impl SetRequest<'_> {
    /// Stores only if the key does not exist.
    pub fn if_absent(mut self) -> Self {
        self.options = self.options.if_absent();
        self
    }

    /// Stores only if the key already exists.
    pub fn if_present(mut self) -> Self {
        self.options = self.options.if_present();
        self
    }

    /// Sets a positive relative expiration in exact milliseconds.
    pub fn expires_after_millis(mut self, milliseconds: u64) -> Self {
        self.options = self.options.expires_after_millis(milliseconds);
        self
    }
}

#[cfg(feature = "quic-quinn")]
impl<'a> IntoFuture for SetRequest<'a> {
    type Output = Result<SetOutcome>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            self.client
                .inner
                .set(self.application_key, self.value, self.options)
                .await
        })
    }
}

#[cfg(feature = "quic-compio")]
/// Awaitable Compio set request with optional condition and TTL modifiers.
pub struct LocalSetRequest<'a> {
    client: &'a LocalClient,
    application_key: Vec<u8>,
    value: Vec<u8>,
    options: SetOptions,
}

#[cfg(feature = "quic-compio")]
impl LocalSetRequest<'_> {
    #[cfg(feature = "ffi")]
    pub(crate) fn options(mut self, options: SetOptions) -> Self {
        self.options = options;
        self
    }

    /// Stores only if the key does not exist.
    pub fn if_absent(mut self) -> Self {
        self.options = self.options.if_absent();
        self
    }

    /// Stores only if the key already exists.
    pub fn if_present(mut self) -> Self {
        self.options = self.options.if_present();
        self
    }

    /// Sets a positive relative expiration in exact milliseconds.
    pub fn expires_after_millis(mut self, milliseconds: u64) -> Self {
        self.options = self.options.expires_after_millis(milliseconds);
        self
    }
}

#[cfg(feature = "quic-compio")]
impl<'a> IntoFuture for LocalSetRequest<'a> {
    type Output = Result<SetOutcome>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            self.client
                .inner
                .set(self.application_key, self.value, self.options)
                .await
        })
    }
}
