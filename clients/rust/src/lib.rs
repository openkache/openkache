//! Layered QUIC client for the OpenKache binary protocol.

mod config;
#[cfg(feature = "ffi")]
pub mod ffi;
mod key;
mod transport;
pub mod value;
pub mod value_envelope;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

#[cfg(feature = "quic-compio")]
use compio::net::ToSocketAddrsAsync;
use openkache_protocol::{MAX_RESPONSE_FRAME_BYTES, Opcode, Request, Response, Status};
#[cfg(any(feature = "quic-compio", feature = "quic-quinn"))]
use std::future::{Future, IntoFuture};
#[cfg(any(feature = "quic-compio", feature = "quic-quinn"))]
use std::pin::Pin;
use transport::{ClientConnection, ClientLane};

pub use config::{
    Certificate, ClientIdentity, ClientTimeouts, Endpoint, PrivateKey, RetryPolicy, ServerTrust,
    SetCondition, SetOptions,
};
pub use key::{DATA_PROTECTION_KEY_BYTES, DataProtectionKey, KEY_BYTES, Key};
pub use value::EncodedValue;

#[cfg(not(any(feature = "quic-compio", feature = "quic-quinn")))]
compile_error!("enable at least one client QUIC backend feature");

const DEFAULT_MAX_IN_FLIGHT: usize = 256;
const STATE_CONNECTED: u8 = 0;
const STATE_RECONNECTING: u8 = 1;
const STATE_DISCONNECTED: u8 = 2;
const STATE_CLOSED: u8 = 3;

/// Programmatic category for a server-side failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ServerErrorCode {
    /// The request was malformed or violated an operation constraint.
    InvalidRequest,
    /// The server does not implement the requested operation.
    UnsupportedOperation,
    /// The request exceeded a protocol or server size limit.
    TooLarge,
    /// The server could not accept more work.
    Overloaded,
    /// The server-side operation exceeded its deadline.
    Timeout,
    /// The authenticated identity is not authorized for the operation.
    Forbidden,
    /// The server encountered an internal failure.
    Internal,
}

/// All client-level failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A client setting was invalid.
    #[error("invalid {field}: {message}")]
    Configuration {
        /// Stable setting identifier.
        field: &'static str,
        /// Human-readable validation detail.
        message: String,
    },
    /// Connection setup or replacement failed.
    #[error("connection failed: {0}")]
    Connection(String),
    /// A complete client operation exceeded its deadline.
    #[error("operation timed out during {operation}")]
    Timeout {
        /// Operation phase that reached the deadline.
        operation: &'static str,
    },
    /// The selected asynchronous runtime is unavailable.
    #[error("{backend} runtime unavailable: {message}")]
    Runtime {
        /// Stable runtime backend name.
        backend: &'static str,
        /// Human-readable runtime requirement.
        message: String,
    },
    /// A typed QUIC transport operation failed.
    #[error("{backend} QUIC {operation} failed: {message}")]
    Transport {
        /// Stable QUIC backend name.
        backend: &'static str,
        /// Stable failed-operation name.
        operation: &'static str,
        /// Human-readable backend detail.
        message: String,
    },
    /// The server returned an error status.
    #[error("server returned {code:?}: {message}")]
    Server {
        /// Client-owned programmatic server error category.
        code: ServerErrorCode,
        /// Human-readable server payload.
        message: String,
    },
    /// A successful response had an invalid status or payload shape.
    #[error("unexpected {operation} response: {message}")]
    UnexpectedResponse {
        /// Operation whose response was invalid.
        operation: &'static str,
        /// Human-readable protocol detail.
        message: String,
    },
    /// A response exceeded the protocol frame limit.
    #[error("response exceeds protocol limit of {maximum} bytes")]
    ResponseTooLarge {
        /// Maximum accepted response bytes.
        maximum: usize,
    },
    /// TLS configuration or certificate validation failed.
    #[error("TLS configuration failed: {0}")]
    Tls(String),
    /// Binary protocol encoding or decoding failed.
    #[error("protocol failed: {0}")]
    Protocol(String),
    /// An operating-system I/O operation failed.
    #[error("I/O failed: {0}")]
    Io(String),
    /// Client-side value encoding or decoding failed.
    #[error("value transformation failed: {0}")]
    Value(#[from] value::Error),
    /// The client was explicitly and permanently closed.
    #[error("client is closed")]
    ClientClosed,
    /// A mutation may have reached the server before the connection failed.
    #[error("{operation} result is unknown after a connection failure: {message}")]
    AmbiguousOutcome {
        /// Mutation whose result is unknown.
        operation: &'static str,
        /// Human-readable underlying connection failure.
        message: String,
    },
}

impl Error {
    pub(crate) fn configuration(field: &'static str, message: impl Into<String>) -> Self {
        Self::Configuration {
            field,
            message: message.into(),
        }
    }

    fn is_connection_failure(&self) -> bool {
        matches!(
            self,
            Self::Connection(_) | Self::Timeout { .. } | Self::Transport { .. } | Self::Io(_)
        )
    }
}

impl From<rustls::Error> for Error {
    fn from(error: rustls::Error) -> Self {
        Self::Tls(error.to_string())
    }
}

impl From<openkache_protocol::ProtocolError> for Error {
    fn from(error: openkache_protocol::ProtocolError) -> Self {
        Self::Protocol(error.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Convenience alias for client results.
pub type Result<T> = std::result::Result<T, Error>;

/// Successful lookup result, separate from transport or protocol failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GetOutcome<T> {
    /// The key existed and returned its value.
    Found(T),
    /// The key did not exist.
    NotFound,
}

impl<T> GetOutcome<T> {
    /// Converts the outcome to Rust's conventional optional-value representation.
    pub fn into_option(self) -> Option<T> {
        match self {
            Self::Found(value) => Some(value),
            Self::NotFound => None,
        }
    }

    /// Returns whether the outcome contains a value matching the predicate.
    pub fn is_found_and(self, predicate: impl FnOnce(T) -> bool) -> bool {
        match self {
            Self::Found(value) => predicate(value),
            Self::NotFound => false,
        }
    }
}

/// Successful result of storing a key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetOutcome {
    /// A new key was stored.
    Created,
    /// An existing key was replaced.
    Replaced,
    /// A conditional set did not match and changed nothing.
    NotStored,
}

/// Successful result of deleting a key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteOutcome {
    /// The existing key was removed.
    Deleted,
    /// The key did not exist.
    NotFound,
}

/// Current best-effort connection state snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConnectionState {
    /// The latest connection is available.
    Connected,
    /// A request is replacing a failed connection.
    Reconnecting,
    /// The latest connection failed and a later operation will reconnect.
    Disconnected,
    /// The client was explicitly closed and cannot reconnect.
    Closed,
}

struct Core<C: ClientConnection> {
    connection: RwLock<Arc<C>>,
    reconnect: futures_util::lock::Mutex<()>,
    address: SocketAddr,
    server_name: String,
    tls: rustls::ClientConfig,
    connect_timeout: Duration,
    request_timeout: Duration,
    retry: RetryPolicy,
    max_in_flight: usize,
    state: AtomicU8,
}

impl<C: ClientConnection> Core<C> {
    async fn connect(
        address: SocketAddr,
        server_name: String,
        tls: rustls::ClientConfig,
        timeouts: ClientTimeouts,
        retry: RetryPolicy,
        max_in_flight: usize,
    ) -> Result<Self> {
        let connection = C::connect(
            address,
            &server_name,
            tls.clone(),
            timeouts.connect,
            max_in_flight,
        )
        .await?;
        Ok(Self {
            connection: RwLock::new(Arc::new(connection)),
            reconnect: futures_util::lock::Mutex::new(()),
            address,
            server_name,
            tls,
            connect_timeout: timeouts.connect,
            request_timeout: timeouts.request,
            retry,
            max_in_flight,
            state: AtomicU8::new(STATE_CONNECTED),
        })
    }

    fn connection_state(&self) -> ConnectionState {
        match self.state.load(Ordering::Acquire) {
            STATE_CONNECTED => ConnectionState::Connected,
            STATE_RECONNECTING => ConnectionState::Reconnecting,
            STATE_DISCONNECTED => ConnectionState::Disconnected,
            STATE_CLOSED => ConnectionState::Closed,
            _ => unreachable!("connection state is always a known discriminator"),
        }
    }

    async fn ping(&self) -> Result<Duration> {
        let started = Instant::now();
        let response = self
            .request(Request::new(Opcode::Ping, None, Vec::new())?)
            .await?;
        expect_status("PING", response.status, &[Status::Ok])?;
        if response.payload != b"PONG" {
            return Err(Error::UnexpectedResponse {
                operation: "PING",
                message: "payload is not PONG".into(),
            });
        }
        Ok(started.elapsed())
    }

    async fn get(&self, key: Key) -> Result<GetOutcome<EncodedValue>> {
        let response = self
            .request(Request::new(
                Opcode::Get,
                Some(key.into_protocol()),
                Vec::new(),
            )?)
            .await?;
        match response.status {
            Status::Ok => Ok(GetOutcome::Found(EncodedValue::from_protocol(
                response.payload,
                response.value_flags,
            ))),
            Status::NotFound => Ok(GetOutcome::NotFound),
            status => Err(unexpected_status("GET", status)),
        }
    }

    async fn set(&self, key: Key, value: EncodedValue, options: SetOptions) -> Result<SetOutcome> {
        let (flags, bytes) = value.into_protocol();
        let request = Request::new_set(
            Opcode::Set,
            Some(key.into_protocol()),
            flags,
            options.into_protocol()?,
            bytes,
        )?;
        match self.request(request).await?.status {
            Status::Created => Ok(SetOutcome::Created),
            Status::Replaced => Ok(SetOutcome::Replaced),
            Status::NotStored => Ok(SetOutcome::NotStored),
            status => Err(unexpected_status("SET", status)),
        }
    }

    async fn delete(&self, key: Key) -> Result<DeleteOutcome> {
        let response = self
            .request(Request::new(
                Opcode::Delete,
                Some(key.into_protocol()),
                Vec::new(),
            )?)
            .await?;
        match response.status {
            Status::Deleted => Ok(DeleteOutcome::Deleted),
            Status::NotFound => Ok(DeleteOutcome::NotFound),
            status => Err(unexpected_status("DELETE", status)),
        }
    }

    async fn stats(&self) -> Result<String> {
        let response = self
            .request(Request::new(Opcode::Stats, None, Vec::new())?)
            .await?;
        expect_status("STATS", response.status, &[Status::Ok])?;
        String::from_utf8(response.payload)
            .map_err(|error| Error::Protocol(format!("STATS response is not UTF-8: {error}")))
    }

    async fn sync(&self) -> Result<()> {
        let response = self
            .request(Request::new(Opcode::Sync, None, Vec::new())?)
            .await?;
        expect_status("SYNC", response.status, &[Status::Ok])
    }

    async fn request(&self, request: Request) -> Result<Response> {
        if self.connection_state() == ConnectionState::Closed {
            return Err(Error::ClientClosed);
        }
        let deadline = transport::Deadline::after(self.request_timeout)?;
        if self.connection_state() == ConnectionState::Disconnected {
            self.reconnect_before(deadline).await?;
        }
        let response_safe = matches!(request.opcode, Opcode::Ping | Opcode::Get | Opcode::Stats);
        let max_attempts = if response_safe {
            self.retry.max_attempts
        } else {
            1
        };
        for attempt in 1..=max_attempts {
            let connection = self.current_connection()?;
            match self
                .request_once(&connection, request.clone(), deadline)
                .await
            {
                Ok(response) => return Ok(response),
                Err(error) if error.is_connection_failure() => {
                    self.state.store(STATE_DISCONNECTED, Ordering::Release);
                    if response_safe && attempt < max_attempts {
                        self.reconnect_failed(&connection, deadline).await?;
                        continue;
                    }
                    if !response_safe {
                        return Err(Error::AmbiguousOutcome {
                            operation: opcode_name(request.opcode),
                            message: error.to_string(),
                        });
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("every configured request has at least one attempt")
    }

    async fn request_once(
        &self,
        connection: &C,
        request: Request,
        deadline: transport::Deadline,
    ) -> Result<Response> {
        let mut stream = connection.acquire_lane(deadline).await?;
        let result = async {
            stream
                .write_request(request.into_encoded()?, deadline)
                .await?;
            let frame = stream
                .read_response(MAX_RESPONSE_FRAME_BYTES, deadline)
                .await?;
            Response::decode_owned(frame).map_err(Error::from)
        }
        .await;
        let response = match result {
            Ok(response) => {
                if response.status.is_error() {
                    return Err(Error::Server {
                        code: server_error_code(response.status),
                        message: String::from_utf8_lossy(&response.payload).into_owned(),
                    });
                }
                stream.release();
                response
            }
            Err(error) => return Err(error),
        };
        Ok(response)
    }

    fn current_connection(&self) -> Result<Arc<C>> {
        if self.connection_state() == ConnectionState::Closed {
            return Err(Error::ClientClosed);
        }
        self.connection
            .read()
            .map(|connection| Arc::clone(&connection))
            .map_err(|_| Error::Connection("connection state lock is poisoned".into()))
    }

    async fn reconnect_before(&self, deadline: transport::Deadline) -> Result<()> {
        let current = self.current_connection()?;
        self.reconnect_failed(&current, deadline).await
    }

    async fn reconnect_failed(&self, failed: &Arc<C>, deadline: transport::Deadline) -> Result<()> {
        let remaining = deadline.remaining("connection retry")?;
        let Some(_guard) = C::timeout(remaining, self.reconnect.lock()).await? else {
            return Err(Error::Timeout {
                operation: "connection retry",
            });
        };
        if self.connection_state() == ConnectionState::Closed {
            return Err(Error::ClientClosed);
        }
        let current = self.current_connection()?;
        if !Arc::ptr_eq(&current, failed) {
            return Ok(());
        }
        self.state.store(STATE_RECONNECTING, Ordering::Release);
        let timeout = deadline
            .remaining("connection retry")?
            .min(self.connect_timeout);
        let replacement = match C::connect(
            self.address,
            &self.server_name,
            self.tls.clone(),
            timeout,
            self.max_in_flight,
        )
        .await
        {
            Ok(connection) => connection,
            Err(error) => {
                self.state.store(STATE_DISCONNECTED, Ordering::Release);
                return Err(error);
            }
        };
        let mut connection = self
            .connection
            .write()
            .map_err(|_| Error::Connection("connection state lock is poisoned".into()))?;
        if Arc::ptr_eq(&connection, failed) {
            failed.close();
            *connection = Arc::new(replacement);
        }
        self.state.store(STATE_CONNECTED, Ordering::Release);
        Ok(())
    }

    async fn reconnect(&self) -> Result<()> {
        let deadline = transport::Deadline::after(self.connect_timeout)?;
        self.state.store(STATE_DISCONNECTED, Ordering::Release);
        self.reconnect_before(deadline).await
    }

    async fn close(&self) -> Result<()> {
        let previous = self.state.swap(STATE_CLOSED, Ordering::AcqRel);
        if previous != STATE_CLOSED {
            let connection = self
                .connection
                .read()
                .map_err(|_| Error::Connection("connection state lock is poisoned".into()))?;
            connection.close();
        }
        Ok(())
    }
}

struct ValueLayer {
    data_protection_key: Option<DataProtectionKey>,
    codec: value::ValueCodec,
}

impl ValueLayer {
    fn key(&self, application_key: &[u8]) -> Key {
        self.data_protection_key.as_ref().map_or_else(
            || Key::derive(application_key),
            |key| key.derive_key(application_key),
        )
    }
}

struct BuilderSettings {
    endpoint: Endpoint,
    trust: ServerTrust,
    identity: Option<ClientIdentity>,
    timeouts: ClientTimeouts,
    retry: RetryPolicy,
    max_in_flight: usize,
    compression: value::Compression,
    data_protection_key: Option<DataProtectionKey>,
}

impl BuilderSettings {
    fn new(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            trust: ServerTrust::default(),
            identity: None,
            timeouts: ClientTimeouts::default(),
            retry: RetryPolicy::default(),
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
            compression: value::Compression::Disabled,
            data_protection_key: None,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.timeouts.connect.is_zero() || self.timeouts.request.is_zero() {
            return Err(Error::configuration(
                "timeouts",
                "must be greater than zero",
            ));
        }
        for (field, timeout) in [
            ("timeouts.connect", self.timeouts.connect),
            ("timeouts.request", self.timeouts.request),
        ] {
            if Instant::now().checked_add(timeout).is_none() {
                return Err(Error::configuration(
                    field,
                    "exceeds the platform clock range",
                ));
            }
        }
        if self.retry.max_attempts == 0 {
            return Err(Error::configuration(
                "retry.max_attempts",
                "must be greater than zero",
            ));
        }
        if self.max_in_flight == 0 {
            return Err(Error::configuration(
                "max_in_flight",
                "must be greater than zero",
            ));
        }
        Ok(())
    }

    fn finish(self) -> Result<(ConnectionSettings, Arc<ValueLayer>)> {
        self.validate()?;
        let codec = match &self.data_protection_key {
            Some(key) => value::ValueCodec::encrypted(key.derive_value_key(), self.compression)?,
            None => value::ValueCodec::compressed(self.compression)?,
        };
        let tls = make_tls_config(self.trust, self.identity)?;
        Ok((
            ConnectionSettings {
                endpoint: self.endpoint,
                tls,
                timeouts: self.timeouts,
                retry: self.retry,
                max_in_flight: self.max_in_flight,
            },
            Arc::new(ValueLayer {
                data_protection_key: self.data_protection_key,
                codec,
            }),
        ))
    }
}

struct ConnectionSettings {
    endpoint: Endpoint,
    tls: rustls::ClientConfig,
    timeouts: ClientTimeouts,
    retry: RetryPolicy,
    max_in_flight: usize,
}

macro_rules! builder_methods {
    ($builder:ident) => {
        impl $builder {
            /// Uses only the supplied server trust roots.
            pub fn server_trust(mut self, trust: ServerTrust) -> Self {
                self.settings.trust = trust;
                self
            }

            /// Trusts one explicit CA or self-signed server certificate.
            pub fn trust_certificate(mut self, certificate: Certificate) -> Self {
                self.settings.trust = ServerTrust::Custom(vec![certificate]);
                self
            }

            /// Presents a mutual TLS client identity.
            pub fn client_identity(mut self, identity: ClientIdentity) -> Self {
                self.settings.identity = Some(identity);
                self
            }

            /// Sets connection and complete-request deadlines.
            pub fn timeouts(mut self, timeouts: ClientTimeouts) -> Self {
                self.settings.timeouts = timeouts;
                self
            }

            /// Sets retry attempts for response-safe operations.
            pub fn retry_policy(mut self, retry: RetryPolicy) -> Self {
                self.settings.retry = retry;
                self
            }

            /// Bounds simultaneous request lanes on one QUIC connection.
            pub fn max_in_flight(mut self, maximum: usize) -> Self {
                self.settings.max_in_flight = maximum;
                self
            }

            /// Applies client-side compression before optional encryption.
            pub fn compression(mut self, compression: value::Compression) -> Self {
                self.settings.compression = compression;
                self
            }

            /// Hides application keys and encrypts values with one master key.
            pub fn data_protection_key(mut self, key: DataProtectionKey) -> Self {
                self.settings.data_protection_key = Some(key);
                self
            }
        }
    };
}

#[cfg(feature = "quic-quinn")]
/// Exact-key, exact-value protocol client running on Tokio and Quinn.
#[derive(Clone)]
pub struct RawClient(Arc<Core<transport::QuinnConnection>>);

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
    settings: BuilderSettings,
}

#[cfg(feature = "quic-quinn")]
builder_methods!(ClientBuilder);

#[cfg(feature = "quic-quinn")]
impl ClientBuilder {
    /// Connects a Tokio/Quinn client with the configured layers.
    pub async fn connect(self) -> Result<Client> {
        let (settings, values) = self.settings.finish()?;
        let raw = RawClient(connect_quinn(settings).await?);
        Ok(Client { raw, values })
    }

    /// Connects only the transport and protocol layer.
    pub async fn connect_raw(self) -> Result<RawClient> {
        let (settings, _) = self.settings.finish()?;
        Ok(RawClient(connect_quinn(settings).await?))
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
            settings: BuilderSettings::new(endpoint),
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

#[cfg(feature = "quic-quinn")]
impl RawClient {
    /// Connects an exact protocol client with system trust.
    pub async fn connect(endpoint: &str) -> Result<Self> {
        Client::builder(endpoint.parse()?).connect_raw().await
    }

    /// Starts explicit configuration for an exact protocol client.
    pub fn builder(endpoint: Endpoint) -> ClientBuilder {
        Client::builder(endpoint)
    }

    /// Verifies the connection and returns the complete request round-trip time.
    pub async fn ping(&self) -> Result<Duration> {
        self.0.ping().await
    }

    /// Retrieves exact encoded bytes for an exact 32-byte wire key.
    pub async fn get(&self, key: Key) -> Result<GetOutcome<EncodedValue>> {
        self.0.get(key).await
    }

    /// Stores exact encoded bytes with explicit wire-level set options.
    pub async fn set(
        &self,
        key: Key,
        value: EncodedValue,
        options: SetOptions,
    ) -> Result<SetOutcome> {
        self.0.set(key, value, options).await
    }

    /// Deletes an exact 32-byte wire key.
    pub async fn delete(&self, key: Key) -> Result<DeleteOutcome> {
        self.0.delete(key).await
    }

    /// Returns server statistics as their JSON text.
    pub async fn stats(&self) -> Result<String> {
        self.0.stats().await
    }

    /// Waits until prior mutations satisfy the server durability barrier.
    pub async fn sync(&self) -> Result<()> {
        self.0.sync().await
    }

    /// Returns a best-effort state snapshot that does not guarantee the next request succeeds.
    pub fn connection_state(&self) -> ConnectionState {
        self.0.connection_state()
    }

    /// Explicitly replaces the current connection without replaying an operation.
    pub async fn reconnect(&self) -> Result<()> {
        self.0.reconnect().await
    }

    /// Permanently and idempotently closes this client instance.
    pub async fn close(&self) -> Result<()> {
        self.0.close().await
    }
}

#[cfg(feature = "quic-compio")]
/// Exact-key, exact-value protocol client confined to a Compio runtime.
#[derive(Clone)]
pub struct LocalRawClient(Arc<Core<transport::CompioConnection>>);

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
    settings: BuilderSettings,
}

#[cfg(feature = "quic-compio")]
builder_methods!(LocalClientBuilder);

#[cfg(feature = "quic-compio")]
impl LocalClientBuilder {
    /// Connects a high-level Compio client with the configured layers.
    pub async fn connect(self) -> Result<LocalClient> {
        let (settings, values) = self.settings.finish()?;
        let raw = LocalRawClient(connect_compio(settings).await?);
        Ok(LocalClient { raw, values })
    }

    /// Connects only the exact transport and protocol layer on Compio.
    pub async fn connect_raw(self) -> Result<LocalRawClient> {
        let (settings, _) = self.settings.finish()?;
        Ok(LocalRawClient(connect_compio(settings).await?))
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
            settings: BuilderSettings::new(endpoint),
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

#[cfg(feature = "quic-compio")]
impl LocalRawClient {
    /// Connects an exact protocol client with system trust on an active Compio runtime.
    pub async fn connect(endpoint: &str) -> Result<Self> {
        LocalClient::builder(endpoint.parse()?).connect_raw().await
    }

    /// Starts explicit configuration for an exact Compio protocol client.
    pub fn builder(endpoint: Endpoint) -> LocalClientBuilder {
        LocalClient::builder(endpoint)
    }

    /// Verifies the connection and returns the complete request round-trip time.
    pub async fn ping(&self) -> Result<Duration> {
        self.0.ping().await
    }

    /// Retrieves exact encoded bytes for an exact 32-byte wire key.
    pub async fn get(&self, key: Key) -> Result<GetOutcome<EncodedValue>> {
        self.0.get(key).await
    }

    /// Stores exact encoded bytes with explicit wire-level set options.
    pub async fn set(
        &self,
        key: Key,
        value: EncodedValue,
        options: SetOptions,
    ) -> Result<SetOutcome> {
        self.0.set(key, value, options).await
    }

    /// Deletes an exact 32-byte wire key.
    pub async fn delete(&self, key: Key) -> Result<DeleteOutcome> {
        self.0.delete(key).await
    }

    /// Returns server statistics as their JSON text.
    pub async fn stats(&self) -> Result<String> {
        self.0.stats().await
    }

    /// Waits until prior mutations satisfy the server durability barrier.
    pub async fn sync(&self) -> Result<()> {
        self.0.sync().await
    }

    /// Returns a best-effort state snapshot that does not guarantee the next request succeeds.
    pub fn connection_state(&self) -> ConnectionState {
        self.0.connection_state()
    }

    /// Explicitly replaces the current connection without replaying an operation.
    pub async fn reconnect(&self) -> Result<()> {
        self.0.reconnect().await
    }

    /// Permanently and idempotently closes this client instance.
    pub async fn close(&self) -> Result<()> {
        self.0.close().await
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
    key: Key,
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
    key: Key,
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

#[cfg(feature = "quic-quinn")]
async fn connect_quinn(
    settings: ConnectionSettings,
) -> Result<Arc<Core<transport::QuinnConnection>>> {
    let address = resolve_quinn(&settings.endpoint, settings.timeouts.connect).await?;
    Core::connect(
        address,
        settings.endpoint.server_name().to_owned(),
        settings.tls,
        settings.timeouts,
        settings.retry,
        settings.max_in_flight,
    )
    .await
    .map(Arc::new)
}

#[cfg(feature = "quic-compio")]
async fn connect_compio(
    settings: ConnectionSettings,
) -> Result<Arc<Core<transport::CompioConnection>>> {
    let address = resolve_compio(&settings.endpoint, settings.timeouts.connect).await?;
    Core::connect(
        address,
        settings.endpoint.server_name().to_owned(),
        settings.tls,
        settings.timeouts,
        settings.retry,
        settings.max_in_flight,
    )
    .await
    .map(Arc::new)
}

#[cfg(feature = "quic-quinn")]
async fn resolve_quinn(endpoint: &Endpoint, timeout: Duration) -> Result<SocketAddr> {
    if let Some(address) = endpoint.resolved_address() {
        return Ok(address);
    }
    let addresses = tokio::time::timeout(
        timeout,
        tokio::net::lookup_host((endpoint.host(), endpoint.port())),
    )
    .await
    .map_err(|_| Error::Timeout {
        operation: "DNS resolution",
    })?
    .map_err(Error::from)?;
    addresses.into_iter().next().ok_or_else(|| {
        Error::Connection(format!(
            "{}:{} resolved to no addresses",
            endpoint.host(),
            endpoint.port()
        ))
    })
}

#[cfg(feature = "quic-compio")]
async fn resolve_compio(endpoint: &Endpoint, timeout: Duration) -> Result<SocketAddr> {
    if let Some(address) = endpoint.resolved_address() {
        return Ok(address);
    }
    let target = (endpoint.host(), endpoint.port());
    let addresses = compio::runtime::time::timeout(timeout, target.to_socket_addrs_async())
        .await
        .map_err(|_| Error::Timeout {
            operation: "DNS resolution",
        })?
        .map_err(Error::from)?;
    addresses.into_iter().next().ok_or_else(|| {
        Error::Connection(format!(
            "{}:{} resolved to no addresses",
            endpoint.host(),
            endpoint.port()
        ))
    })
}

fn make_tls_config(
    trust: ServerTrust,
    identity: Option<ClientIdentity>,
) -> Result<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    match trust {
        ServerTrust::System => {
            let result = rustls_native_certs::load_native_certs();
            if result.certs.is_empty() {
                return Err(Error::Tls(format!(
                    "system trust store contained no usable certificates: {:?}",
                    result.errors
                )));
            }
            for certificate in result.certs {
                roots
                    .add(certificate)
                    .map_err(|error| Error::Tls(error.to_string()))?;
            }
        }
        ServerTrust::Custom(certificates) => {
            if certificates.is_empty() {
                return Err(Error::configuration(
                    "server_trust",
                    "custom trust requires at least one certificate",
                ));
            }
            for certificate in certificates {
                roots
                    .add(certificate.into_der())
                    .map_err(|error| Error::Tls(error.to_string()))?;
            }
        }
    }
    let provider = rustls::crypto::ring::default_provider();
    let builder = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(roots);
    let mut config = match identity {
        Some(identity) => {
            let (chain, key) = identity.into_rustls();
            builder.with_client_auth_cert(chain, key)?
        }
        None => builder.with_no_client_auth(),
    };
    config.alpn_protocols = vec![openkache_protocol::ALPN.to_vec()];
    Ok(config)
}

fn expect_status(operation: &'static str, status: Status, expected: &[Status]) -> Result<()> {
    if expected.contains(&status) {
        Ok(())
    } else {
        Err(unexpected_status(operation, status))
    }
}

fn unexpected_status(operation: &'static str, status: Status) -> Error {
    Error::UnexpectedResponse {
        operation,
        message: format!("unexpected status {status:?}"),
    }
}

fn server_error_code(status: Status) -> ServerErrorCode {
    match status {
        Status::InvalidRequest => ServerErrorCode::InvalidRequest,
        Status::UnsupportedOpcode => ServerErrorCode::UnsupportedOperation,
        Status::TooLarge => ServerErrorCode::TooLarge,
        Status::Overloaded => ServerErrorCode::Overloaded,
        Status::Timeout => ServerErrorCode::Timeout,
        Status::Forbidden => ServerErrorCode::Forbidden,
        Status::InternalError => ServerErrorCode::Internal,
        _ => ServerErrorCode::Internal,
    }
}

fn opcode_name(opcode: Opcode) -> &'static str {
    match opcode {
        Opcode::Ping => "PING",
        Opcode::Get => "GET",
        Opcode::Set => "SET",
        Opcode::Delete => "DELETE",
        Opcode::Stats => "STATS",
        Opcode::Sync => "SYNC",
    }
}
