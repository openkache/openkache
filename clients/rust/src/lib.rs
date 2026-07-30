//! Ergonomic Rust API layered over the reusable OpenKache client core.

#[cfg(feature = "ffi")]
pub mod ffi;

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

pub use openkache_client_core::{
    Backend, Certificate, ClientIdentity, ClientTimeouts, ConnectionState,
    DATA_PROTECTION_KEY_BYTES, DataProtectionKey, DeleteOutcome, Endpoint, Error, GetOutcome,
    ITEM_KEY_BYTES, ItemKey, ItemValue, Operation, PrivateKey, Result, RetryPolicy,
    ServerErrorCode, ServerTrust, SetCondition, SetOptions, SetOutcome, value, value_envelope,
};
#[cfg(feature = "quic-compio")]
pub use openkache_client_core::{LocalRawClient, LocalRawClientBuilder};
#[cfg(feature = "quic-quinn")]
pub use openkache_client_core::{RawClient, RawClientBuilder};

struct ValueLayer {
    data_protection_key: Option<DataProtectionKey>,
    codec: value::ValueCodec,
}

impl ValueLayer {
    fn new(
        data_protection_key: Option<DataProtectionKey>,
        compression: value::Compression,
    ) -> Result<Self> {
        let codec = match &data_protection_key {
            Some(key) => value::ValueCodec::protected(key, compression)?,
            None => value::ValueCodec::compressed(compression)?,
        };
        Ok(Self {
            data_protection_key,
            codec,
        })
    }

    fn key(&self, application_key: &[u8]) -> ItemKey {
        self.data_protection_key.as_ref().map_or_else(
            || ItemKey::derive(application_key),
            |key| key.derive_item_key(application_key),
        )
    }
}

struct ValueSettings {
    compression: value::Compression,
    data_protection_key: Option<DataProtectionKey>,
}

impl Default for ValueSettings {
    fn default() -> Self {
        Self {
            compression: value::Compression::Disabled,
            data_protection_key: None,
        }
    }
}

impl ValueSettings {
    fn finish(self) -> Result<Arc<ValueLayer>> {
        ValueLayer::new(self.data_protection_key, self.compression).map(Arc::new)
    }
}

macro_rules! builder_methods {
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

            /// Applies client-side compression before optional encryption.
            pub fn compression(mut self, compression: value::Compression) -> Self {
                self.values.compression = compression;
                self
            }

            /// Hides application keys and encrypts values with one master key.
            pub fn data_protection_key(mut self, key: DataProtectionKey) -> Self {
                self.values.data_protection_key = Some(key);
                self
            }
        }
    };
}

#[cfg(feature = "quic-quinn")]
/// High-level application-key and plaintext-value client running on Tokio and Quinn.
#[derive(Clone)]
pub struct Client {
    raw: RawClient,
    values: Arc<ValueLayer>,
}

#[cfg(feature = "quic-quinn")]
/// Connection and value-layer builder for Tokio clients.
pub struct ClientBuilder {
    raw: RawClientBuilder,
    values: ValueSettings,
}

#[cfg(feature = "quic-quinn")]
builder_methods!(ClientBuilder);

#[cfg(feature = "quic-quinn")]
impl ClientBuilder {
    /// Connects a Tokio/Quinn client with the configured layers.
    pub async fn connect(self) -> Result<Client> {
        let values = self.values.finish()?;
        let raw = self.raw.connect().await?;
        Ok(Client { raw, values })
    }

    /// Connects only the low-level transport and protocol layer.
    pub async fn connect_raw(self) -> Result<RawClient> {
        self.raw.connect().await
    }
}

#[cfg(feature = "quic-quinn")]
impl Client {
    /// Connects with system trust and default client behavior.
    pub async fn connect(endpoint: &str) -> Result<Self> {
        Self::builder(endpoint.parse()?).connect().await
    }

    /// Starts explicit client configuration.
    pub fn builder(endpoint: Endpoint) -> ClientBuilder {
        ClientBuilder {
            raw: RawClient::builder(endpoint),
            values: ValueSettings::default(),
        }
    }

    /// Borrows the exact protocol layer owned by this client.
    pub fn raw(&self) -> &RawClient {
        &self.raw
    }

    /// Verifies the connection and returns the complete request round-trip time.
    pub async fn ping(&self) -> Result<Duration> {
        self.raw.ping().await
    }

    /// Retrieves and decodes a value for arbitrary application key bytes.
    pub async fn get(&self, application_key: impl AsRef<[u8]>) -> Result<GetOutcome<Vec<u8>>> {
        let key = self.values.key(application_key.as_ref());
        match self.raw.get(key).await? {
            GetOutcome::Found(value) => self
                .values
                .codec
                .open(key, value)
                .map(GetOutcome::Found)
                .map_err(Error::from),
            GetOutcome::NotFound => Ok(GetOutcome::NotFound),
        }
    }

    /// Starts an awaitable set request with persistent, unconditional defaults.
    pub fn set<'a>(
        &'a self,
        application_key: impl AsRef<[u8]>,
        value: impl IntoValue,
    ) -> SetRequest<'a> {
        SetRequest {
            client: self,
            key: self.values.key(application_key.as_ref()),
            value: value.into_value(),
            options: SetOptions::new(),
        }
    }

    /// Deletes a value for arbitrary application key bytes.
    pub async fn delete(&self, application_key: impl AsRef<[u8]>) -> Result<DeleteOutcome> {
        self.raw
            .delete(self.values.key(application_key.as_ref()))
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
}

#[cfg(feature = "quic-compio")]
/// High-level application-key and plaintext-value client confined to a Compio runtime.
#[derive(Clone)]
pub struct LocalClient {
    raw: LocalRawClient,
    values: Arc<ValueLayer>,
}

#[cfg(feature = "quic-compio")]
/// Connection and value-layer builder for Compio clients.
pub struct LocalClientBuilder {
    raw: LocalRawClientBuilder,
    values: ValueSettings,
}

#[cfg(feature = "quic-compio")]
builder_methods!(LocalClientBuilder);

#[cfg(feature = "quic-compio")]
impl LocalClientBuilder {
    /// Connects a high-level Compio client with the configured layers.
    pub async fn connect(self) -> Result<LocalClient> {
        let values = self.values.finish()?;
        let raw = self.raw.connect().await?;
        Ok(LocalClient { raw, values })
    }

    /// Connects only the low-level transport and protocol layer on Compio.
    pub async fn connect_raw(self) -> Result<LocalRawClient> {
        self.raw.connect().await
    }
}

#[cfg(feature = "quic-compio")]
impl LocalClient {
    /// Connects with system trust and default behavior on an active Compio runtime.
    pub async fn connect(endpoint: &str) -> Result<Self> {
        Self::builder(endpoint.parse()?).connect().await
    }

    /// Starts explicit Compio client configuration.
    pub fn builder(endpoint: Endpoint) -> LocalClientBuilder {
        LocalClientBuilder {
            raw: LocalRawClient::builder(endpoint),
            values: ValueSettings::default(),
        }
    }

    /// Borrows the exact protocol layer owned by this client.
    pub fn raw(&self) -> &LocalRawClient {
        &self.raw
    }

    /// Verifies the connection and returns the complete request round-trip time.
    pub async fn ping(&self) -> Result<Duration> {
        self.raw.ping().await
    }

    /// Retrieves and decodes a value for arbitrary application key bytes.
    pub async fn get(&self, application_key: impl AsRef<[u8]>) -> Result<GetOutcome<Vec<u8>>> {
        let key = self.values.key(application_key.as_ref());
        match self.raw.get(key).await? {
            GetOutcome::Found(value) => self
                .values
                .codec
                .open(key, value)
                .map(GetOutcome::Found)
                .map_err(Error::from),
            GetOutcome::NotFound => Ok(GetOutcome::NotFound),
        }
    }

    /// Starts an awaitable set request with persistent, unconditional defaults.
    pub fn set<'a>(
        &'a self,
        application_key: impl AsRef<[u8]>,
        value: impl IntoValue,
    ) -> LocalSetRequest<'a> {
        LocalSetRequest {
            client: self,
            key: self.values.key(application_key.as_ref()),
            value: value.into_value(),
            options: SetOptions::new(),
        }
    }

    /// Deletes a value for arbitrary application key bytes.
    pub async fn delete(&self, application_key: impl AsRef<[u8]>) -> Result<DeleteOutcome> {
        self.raw
            .delete(self.values.key(application_key.as_ref()))
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
}

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
    key: ItemKey,
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
            let value = self.client.values.codec.seal_owned(self.key, self.value)?;
            self.client.raw.set(self.key, value, self.options).await
        })
    }
}

#[cfg(feature = "quic-compio")]
/// Awaitable Compio set request with optional condition and TTL modifiers.
pub struct LocalSetRequest<'a> {
    client: &'a LocalClient,
    key: ItemKey,
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
            let value = self.client.values.codec.seal_owned(self.key, self.value)?;
            self.client.raw.set(self.key, value, self.options).await
        })
    }
}
