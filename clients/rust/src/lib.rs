//! Ergonomic Rust API layered over the reusable OpenKache client core.

#[cfg(feature = "ffi")]
pub mod ffi;

/// Smithy-generated service operation inputs, outputs, enums, and client trait.
pub mod smithy {
    include!(concat!(env!("OUT_DIR"), "/smithy_api.rs"));
}

#[cfg(any(feature = "quic-compio", feature = "quic-quinn"))]
use std::future::{Future, IntoFuture};
#[cfg(any(feature = "quic-compio", feature = "quic-quinn"))]
use std::pin::Pin;
use std::sync::Arc;
#[cfg(any(feature = "quic-compio", feature = "quic-quinn"))]
use std::time::Duration;

pub use openkache_client_core::{
    Backend, Certificate, ClientIdentity, ClientTimeouts, ConnectionState,
    DATA_PROTECTION_KEY_BYTES, DataProtection, DataProtectionKey, DeleteOutcome, Endpoint, Error,
    GetOutcome, ITEM_ID_BYTES, ItemId, ItemValue, Operation, PrivateKey, Result, RetryPolicy,
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

fn smithy_set_options(
    condition: Option<smithy::SetCondition>,
    ttl_milliseconds: Option<i64>,
) -> Result<SetOptions> {
    let mut options = match condition {
        None => SetOptions::new(),
        Some(smithy::SetCondition::IfAbsent) => SetOptions::new().if_absent(),
        Some(smithy::SetCondition::IfPresent) => SetOptions::new().if_present(),
    };
    if let Some(ttl_milliseconds) = ttl_milliseconds {
        if ttl_milliseconds <= 0 {
            return Err(Error::Configuration {
                field: "set.ttl_milliseconds",
                message: "must be greater than zero milliseconds".into(),
            });
        }
        let ttl_milliseconds =
            u64::try_from(ttl_milliseconds).map_err(|_| Error::Configuration {
                field: "set.ttl_milliseconds",
                message: "must fit in an unsigned 64-bit millisecond count".into(),
            })?;
        options = options.expires_after_millis(ttl_milliseconds);
    }
    Ok(options)
}

macro_rules! impl_smithy_api {
    ($client:ident) => {
        impl smithy::OpenKacheApi for $client {
            type Error = Error;

            async fn ping(
                &self,
                _input: smithy::PingInput,
            ) -> std::result::Result<smithy::PingOutput, Self::Error> {
                $client::ping(self).await?;
                Ok(smithy::PingOutput)
            }

            async fn get(
                &self,
                input: smithy::GetInput,
            ) -> std::result::Result<smithy::GetOutput, Self::Error> {
                let item_id = ItemId::from_slice(&input.item_id)?;
                let value = match $client::get(self, item_id).await? {
                    GetOutcome::Found(value) => Some(value.into_bytes()),
                    GetOutcome::NotFound => None,
                };
                Ok(smithy::GetOutput { value })
            }

            async fn set(
                &self,
                input: smithy::SetInput,
            ) -> std::result::Result<smithy::SetOutput, Self::Error> {
                let item_id = ItemId::from_slice(&input.item_id)?;
                let options = smithy_set_options(input.condition, input.ttl_milliseconds)?;
                let outcome =
                    $client::set(self, item_id, ItemValue::new(input.value), options).await?;
                let outcome = match outcome {
                    SetOutcome::Created => smithy::SetOutcome::Created,
                    SetOutcome::Replaced => smithy::SetOutcome::Replaced,
                    SetOutcome::NotStored => smithy::SetOutcome::NotStored,
                };
                Ok(smithy::SetOutput { outcome })
            }

            async fn delete(
                &self,
                input: smithy::DeleteInput,
            ) -> std::result::Result<smithy::DeleteOutput, Self::Error> {
                let item_id = ItemId::from_slice(&input.item_id)?;
                let deleted = $client::delete(self, item_id).await? == DeleteOutcome::Deleted;
                Ok(smithy::DeleteOutput { deleted })
            }

            async fn stats(
                &self,
                _input: smithy::StatsInput,
            ) -> std::result::Result<smithy::StatsOutput, Self::Error> {
                let json = $client::stats(self).await?;
                Ok(smithy::StatsOutput { json })
            }

            async fn sync(
                &self,
                _input: smithy::SyncInput,
            ) -> std::result::Result<smithy::SyncOutput, Self::Error> {
                $client::sync(self).await?;
                Ok(smithy::SyncOutput)
            }
        }
    };
}

#[cfg(feature = "quic-quinn")]
impl_smithy_api!(RawClient);

#[cfg(feature = "quic-compio")]
impl_smithy_api!(LocalRawClient);

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

            /// Selects the authenticated-encryption profile for stored values.
            ///
            /// # Arguments
            ///
            /// * `encryption` - Compact or Robust authenticated-encryption profile.
            ///
            /// # Returns
            ///
            /// This builder with the selected value protection profile.
            pub fn encryption(mut self, encryption: value::Encryption) -> Self {
                self.inner = self.inner.encryption(encryption);
                self
            }
        }
    };
}

#[cfg(any(feature = "quic-compio", feature = "quic-quinn"))]
macro_rules! client_methods {
    ($client:ident, $request:ident) => {
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

            /// Retrieves and decodes a value in the shared logical value model.
            pub async fn get_value(
                &self,
                application_key: impl AsRef<[u8]>,
            ) -> Result<GetOutcome<value::Value>> {
                self.inner.get_value(application_key).await
            }

            /// Deletes a value for arbitrary application key bytes.
            pub async fn delete(&self, application_key: impl AsRef<[u8]>) -> Result<DeleteOutcome> {
                self.inner.delete(application_key).await
            }

            /// Serializes, protects, and stores a value in the shared logical value model.
            pub async fn set_value(
                &self,
                application_key: impl AsRef<[u8]>,
                value: value::Value,
                options: SetOptions,
            ) -> Result<SetOutcome> {
                self.inner.set_value(application_key, value, options).await
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

            /// Starts an awaitable set request with persistent, unconditional defaults.
            pub fn set<'a>(
                &'a self,
                application_key: impl AsRef<[u8]>,
                value: impl IntoValue,
            ) -> $request<'a> {
                self.set_with_options(application_key, value, SetOptions::new())
            }

            /// Starts an awaitable set request with explicit wire-level options.
            pub fn set_with_options<'a>(
                &'a self,
                application_key: impl AsRef<[u8]>,
                value: impl IntoValue,
                options: SetOptions,
            ) -> $request<'a> {
                $request {
                    client: self,
                    application_key: application_key.as_ref().to_vec(),
                    value: value.into_value(),
                    options,
                }
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
}

#[cfg(feature = "quic-quinn")]
client_methods!(Client, SetRequest);

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
}

#[cfg(feature = "quic-compio")]
client_methods!(LocalClient, LocalSetRequest);

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

impl IntoValue for Box<[u8]> {
    fn into_value(self) -> Vec<u8> {
        self.into_vec()
    }
}

impl IntoValue for String {
    fn into_value(self) -> Vec<u8> {
        self.into_bytes()
    }
}

fn copy_borrowed_value<T: AsRef<[u8]> + ?Sized>(value: &T) -> Vec<u8> {
    value.as_ref().to_vec()
}

macro_rules! impl_borrowed_into_value {
    ($($type:ty),+ $(,)?) => {
        $(
            impl IntoValue for $type {
                fn into_value(self) -> Vec<u8> {
                    copy_borrowed_value(self)
                }
            }
        )+
    };
}

impl_borrowed_into_value!(&str, &[u8], &Vec<u8>);

impl<const N: usize> IntoValue for [u8; N] {
    fn into_value(self) -> Vec<u8> {
        self.to_vec()
    }
}

impl<const N: usize> IntoValue for &[u8; N] {
    fn into_value(self) -> Vec<u8> {
        copy_borrowed_value(self)
    }
}

impl IntoValue for &Arc<Vec<u8>> {
    fn into_value(self) -> Vec<u8> {
        self.as_ref().clone()
    }
}

impl IntoValue for Arc<Vec<u8>> {
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

#[cfg(feature = "quic-compio")]
/// Awaitable Compio set request with optional condition and TTL modifiers.
pub struct LocalSetRequest<'a> {
    client: &'a LocalClient,
    application_key: Vec<u8>,
    value: Vec<u8>,
    options: SetOptions,
}

macro_rules! set_request_methods {
    ($request:ident) => {
        impl $request<'_> {
            /// Replaces all set options with an explicit value.
            pub fn options(mut self, options: SetOptions) -> Self {
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
    };
}

macro_rules! set_request_future {
    ($request:ident $(+ $bound:ident)?) => {
        impl<'a> IntoFuture for $request<'a> {
            type Output = Result<SetOutcome>;
            type IntoFuture =
                Pin<Box<dyn Future<Output = Self::Output> $(+ $bound)? + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                Box::pin(async move {
                    self.client
                        .inner
                        .set(self.application_key, self.value, self.options)
                        .await
                })
            }
        }
    };
}

#[cfg(feature = "quic-quinn")]
set_request_methods!(SetRequest);

#[cfg(feature = "quic-quinn")]
set_request_future!(SetRequest + Send);

#[cfg(feature = "quic-compio")]
set_request_methods!(LocalSetRequest);

#[cfg(feature = "quic-compio")]
set_request_future!(LocalSetRequest);
