//! Low-level QUIC client core for the OpenKache binary protocol.

mod config;
mod key;
mod protected;
mod protection;
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
use transport::{ClientConnection, ClientLane};

pub use config::{
    Certificate, ClientIdentity, ClientTimeouts, Endpoint, PrivateKey, RetryPolicy, ServerTrust,
    SetCondition, SetOptions,
};
pub use key::{DATA_PROTECTION_KEY_BYTES, DataProtectionKey, ITEM_KEY_BYTES, ItemKey};
#[cfg(feature = "quic-compio")]
pub use protected::{LocalProtectedClient, LocalProtectedClientBuilder};
#[cfg(feature = "quic-quinn")]
pub use protected::{ProtectedClient, ProtectedClientBuilder};
pub use protection::DataProtection;
pub use value::ItemValue;

#[cfg(not(any(feature = "quic-compio", feature = "quic-quinn")))]
compile_error!("enable at least one client QUIC backend feature");

const DEFAULT_MAX_IN_FLIGHT: usize = 256;
const STATE_CONNECTED: u8 = 0;
const STATE_RECONNECTING: u8 = 1;
const STATE_DISCONNECTED: u8 = 2;
const STATE_CLOSED: u8 = 3;

/// Client-owned identifier for an asynchronous runtime and QUIC backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Backend {
    /// Tokio and Quinn.
    Quinn,
    /// Compio and Compio-QUIC.
    Compio,
}

impl std::fmt::Display for Backend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Quinn => "quinn",
            Self::Compio => "compio",
        })
    }
}

/// Stable client operation or operation phase used by structured errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Operation {
    /// `PING` request.
    Ping,
    /// `GET` request.
    Get,
    /// `SET` request.
    Set,
    /// `DELETE` request.
    Delete,
    /// `STATS` request.
    Stats,
    /// `SYNC` request.
    Sync,
    /// DNS lookup.
    DnsResolution,
    /// Initial QUIC and TLS connection establishment.
    ConnectionSetup,
    /// Replacement connection establishment.
    ConnectionRetry,
    /// Reusable stream-lane acquisition.
    StreamAcquisition,
    /// Request encoding and transmission.
    RequestWrite,
    /// Response-header receipt.
    ResponseHeaderRead,
    /// Response-body receipt.
    ResponseBodyRead,
    /// TLS-to-QUIC configuration conversion.
    TlsInitialization,
    /// Local QUIC endpoint initialization.
    EndpointInitialization,
    /// Remote connection initialization.
    ConnectionInitialization,
    /// QUIC and TLS handshake.
    Handshake,
    /// Bidirectional stream creation.
    StreamOpen,
    /// QUIC stream write.
    StreamWrite,
    /// QUIC stream read.
    StreamRead,
}

impl std::fmt::Display for Operation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Ping => "PING",
            Self::Get => "GET",
            Self::Set => "SET",
            Self::Delete => "DELETE",
            Self::Stats => "STATS",
            Self::Sync => "SYNC",
            Self::DnsResolution => "DNS resolution",
            Self::ConnectionSetup => "connection setup",
            Self::ConnectionRetry => "connection retry",
            Self::StreamAcquisition => "stream acquisition",
            Self::RequestWrite => "request write",
            Self::ResponseHeaderRead => "response header read",
            Self::ResponseBodyRead => "response body read",
            Self::TlsInitialization => "TLS initialization",
            Self::EndpointInitialization => "endpoint initialization",
            Self::ConnectionInitialization => "connection initialization",
            Self::Handshake => "handshake",
            Self::StreamOpen => "stream open",
            Self::StreamWrite => "stream write",
            Self::StreamRead => "stream read",
        })
    }
}

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
        operation: Operation,
    },
    /// The selected asynchronous runtime is unavailable.
    #[error("{backend} runtime unavailable: {message}")]
    Runtime {
        /// Stable runtime backend name.
        backend: Backend,
        /// Human-readable runtime requirement.
        message: String,
    },
    /// A typed QUIC transport operation failed.
    #[error("{backend} QUIC {operation} failed: {message}")]
    Transport {
        /// Stable QUIC backend name.
        backend: Backend,
        /// Stable failed-operation name.
        operation: Operation,
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
        operation: Operation,
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
    /// A mutation may have reached the server before its response could be confirmed.
    #[error("{operation} result is unknown after request transmission: {cause}")]
    AmbiguousOutcome {
        /// Mutation whose result is unknown.
        operation: Operation,
        /// Structured client-owned failure that prevented confirmation.
        #[source]
        cause: Box<Error>,
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

    fn invalidates_connection_before_send(&self) -> bool {
        matches!(
            self,
            Self::Connection(_) | Self::Transport { .. } | Self::Io(_)
        )
    }

    fn tls(error: rustls::Error) -> Self {
        Self::Tls(error.to_string())
    }

    fn protocol(error: openkache_protocol::ProtocolError) -> Self {
        Self::Protocol(error.to_string())
    }

    fn io(error: std::io::Error) -> Self {
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

struct RequestFailure {
    error: Error,
    may_have_reached_server: bool,
    invalidates_connection: bool,
}

impl RequestFailure {
    fn before_send(error: Error) -> Self {
        let invalidates_connection = error.invalidates_connection_before_send();
        Self {
            error,
            may_have_reached_server: false,
            invalidates_connection,
        }
    }

    fn after_send(error: Error) -> Self {
        let invalidates_connection = error.is_connection_failure();
        Self {
            error,
            may_have_reached_server: true,
            invalidates_connection,
        }
    }
}

impl<C: ClientConnection> Core<C> {
    async fn connect(
        address: SocketAddr,
        server_name: String,
        tls: rustls::ClientConfig,
        timeouts: ClientTimeouts,
        retry: RetryPolicy,
        max_in_flight: usize,
        deadline: transport::Deadline,
    ) -> Result<Self> {
        let connection = C::connect(
            address,
            &server_name,
            tls.clone(),
            deadline.remaining(Operation::ConnectionSetup)?,
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
            .request(Request::new(Opcode::Ping, None, Vec::new()).map_err(Error::protocol)?)
            .await?;
        expect_status(Operation::Ping, response.status, &[Status::Ok])?;
        if response.payload != b"PONG" {
            return Err(Error::UnexpectedResponse {
                operation: Operation::Ping,
                message: "payload is not PONG".into(),
            });
        }
        Ok(started.elapsed())
    }

    async fn get(&self, key: ItemKey) -> Result<GetOutcome<ItemValue>> {
        let response = self
            .request(
                Request::new(Opcode::Get, Some(key.into_protocol()), Vec::new())
                    .map_err(Error::protocol)?,
            )
            .await?;
        match response.status {
            Status::Ok => Ok(GetOutcome::Found(ItemValue::from_protocol(
                response.payload,
                response.value_flags,
            ))),
            Status::NotFound => Ok(GetOutcome::NotFound),
            status => Err(unexpected_status(Operation::Get, status)),
        }
    }

    async fn set(&self, key: ItemKey, value: ItemValue, options: SetOptions) -> Result<SetOutcome> {
        let (flags, bytes) = value.into_protocol();
        let request = Request::new_set(
            Opcode::Set,
            Some(key.into_protocol()),
            flags,
            options.into_protocol()?,
            bytes,
        )
        .map_err(Error::protocol)?;
        match self.request(request).await?.status {
            Status::Created => Ok(SetOutcome::Created),
            Status::Replaced => Ok(SetOutcome::Replaced),
            Status::NotStored => Ok(SetOutcome::NotStored),
            status => Err(unexpected_status(Operation::Set, status)),
        }
    }

    async fn delete(&self, key: ItemKey) -> Result<DeleteOutcome> {
        let response = self
            .request(
                Request::new(Opcode::Delete, Some(key.into_protocol()), Vec::new())
                    .map_err(Error::protocol)?,
            )
            .await?;
        match response.status {
            Status::Deleted => Ok(DeleteOutcome::Deleted),
            Status::NotFound => Ok(DeleteOutcome::NotFound),
            status => Err(unexpected_status(Operation::Delete, status)),
        }
    }

    async fn stats(&self) -> Result<String> {
        let response = self
            .request(Request::new(Opcode::Stats, None, Vec::new()).map_err(Error::protocol)?)
            .await?;
        expect_status(Operation::Stats, response.status, &[Status::Ok])?;
        String::from_utf8(response.payload)
            .map_err(|error| Error::Protocol(format!("STATS response is not UTF-8: {error}")))
    }

    async fn sync(&self) -> Result<()> {
        let response = self
            .request(Request::new(Opcode::Sync, None, Vec::new()).map_err(Error::protocol)?)
            .await?;
        expect_status(Operation::Sync, response.status, &[Status::Ok])
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
        let operation = operation(request.opcode);
        let mut request = Some(request);
        for attempt in 1..=max_attempts {
            let connection = self.current_connection()?;
            let attempt_request = if attempt == max_attempts {
                request
                    .take()
                    .expect("the final attempt owns the original request")
            } else {
                request
                    .as_ref()
                    .expect("a retryable request remains available")
                    .clone()
            };
            match self
                .request_once(&connection, attempt_request, deadline)
                .await
            {
                Ok(response) if response.status.is_error() => {
                    return Err(Error::Server {
                        code: server_error_code(response.status),
                        message: String::from_utf8_lossy(&response.payload).into_owned(),
                    });
                }
                Ok(response) => return Ok(response),
                Err(failure) => {
                    if failure.invalidates_connection {
                        self.mark_disconnected(&connection);
                    }
                    if response_safe && failure.invalidates_connection && attempt < max_attempts {
                        self.reconnect_failed(&connection, deadline).await?;
                        continue;
                    }
                    if !response_safe && failure.may_have_reached_server {
                        return Err(Error::AmbiguousOutcome {
                            operation,
                            cause: Box::new(failure.error),
                        });
                    }
                    return Err(failure.error);
                }
            }
        }
        unreachable!("every configured request has at least one attempt")
    }

    async fn request_once(
        &self,
        connection: &C,
        request: Request,
        deadline: transport::Deadline,
    ) -> std::result::Result<Response, RequestFailure> {
        let mut stream = connection
            .acquire_lane(deadline)
            .await
            .map_err(RequestFailure::before_send)?;
        let frame = request
            .into_encoded()
            .map_err(Error::protocol)
            .map_err(RequestFailure::before_send)?;
        let write_timeout = deadline
            .remaining(Operation::RequestWrite)
            .map_err(RequestFailure::before_send)?;
        stream
            .write_request(frame, write_timeout)
            .await
            .map_err(RequestFailure::after_send)?;
        let frame = stream
            .read_response(MAX_RESPONSE_FRAME_BYTES, deadline)
            .await
            .map_err(RequestFailure::after_send)?;
        let response = Response::decode_owned(frame)
            .map_err(Error::protocol)
            .map_err(RequestFailure::after_send)?;
        stream.release();
        Ok(response)
    }

    fn mark_disconnected(&self, failed: &Arc<C>) {
        let Ok(current) = self.connection.read() else {
            return;
        };
        if !Arc::ptr_eq(&current, failed) {
            return;
        }
        let _ = self
            .state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state != STATE_CLOSED).then_some(STATE_DISCONNECTED)
            });
    }

    fn set_state_unless_closed(&self, next: u8) -> Result<()> {
        self.state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state != STATE_CLOSED).then_some(next)
            })
            .map(|_| ())
            .map_err(|_| Error::ClientClosed)
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
        let remaining = deadline.remaining(Operation::ConnectionRetry)?;
        let Some(_guard) = C::timeout(remaining, self.reconnect.lock()).await? else {
            return Err(Error::Timeout {
                operation: Operation::ConnectionRetry,
            });
        };
        if self.connection_state() == ConnectionState::Closed {
            return Err(Error::ClientClosed);
        }
        if self.connection_state() == ConnectionState::Connected {
            return Ok(());
        }
        let current = self.current_connection()?;
        if !Arc::ptr_eq(&current, failed) {
            return Ok(());
        }
        self.set_state_unless_closed(STATE_RECONNECTING)?;
        let timeout = deadline
            .remaining(Operation::ConnectionRetry)?
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
                self.mark_disconnected(failed);
                return Err(error);
            }
        };
        let mut connection = self
            .connection
            .write()
            .map_err(|_| Error::Connection("connection state lock is poisoned".into()))?;
        if self.connection_state() == ConnectionState::Closed {
            replacement.close();
            return Err(Error::ClientClosed);
        }
        if Arc::ptr_eq(&connection, failed) {
            failed.close();
            *connection = Arc::new(replacement);
        }
        self.set_state_unless_closed(STATE_CONNECTED)?;
        Ok(())
    }

    async fn reconnect(&self) -> Result<()> {
        let deadline = transport::Deadline::after(self.connect_timeout)?;
        let current = self.current_connection()?;
        self.mark_disconnected(&current);
        self.reconnect_failed(&current, deadline).await
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

struct BuilderSettings {
    endpoint: Endpoint,
    trust: ServerTrust,
    identity: Option<ClientIdentity>,
    timeouts: ClientTimeouts,
    retry: RetryPolicy,
    max_in_flight: usize,
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

    fn finish(self) -> Result<ConnectionSettings> {
        self.validate()?;
        let tls = make_tls_config(self.trust, self.identity)?;
        Ok(ConnectionSettings {
            endpoint: self.endpoint,
            tls,
            timeouts: self.timeouts,
            retry: self.retry,
            max_in_flight: self.max_in_flight,
        })
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
        }
    };
}

#[cfg(feature = "quic-quinn")]
/// Exact-key, exact-value protocol client running on Tokio and Quinn.
#[derive(Clone)]
pub struct RawClient(Arc<Core<transport::QuinnConnection>>);

#[cfg(feature = "quic-quinn")]
/// Connection builder for the Tokio/Quinn low-level client.
pub struct RawClientBuilder {
    settings: BuilderSettings,
}

#[cfg(feature = "quic-quinn")]
builder_methods!(RawClientBuilder);

#[cfg(feature = "quic-quinn")]
impl RawClientBuilder {
    /// Connects the configured exact protocol client.
    pub async fn connect(self) -> Result<RawClient> {
        Ok(RawClient(connect_quinn(self.settings.finish()?).await?))
    }
}

#[cfg(feature = "quic-quinn")]
impl RawClient {
    /// Connects an exact protocol client with system trust.
    pub async fn connect(endpoint: &str) -> Result<Self> {
        Self::builder(endpoint.parse()?).connect().await
    }

    /// Starts explicit configuration for an exact protocol client.
    pub fn builder(endpoint: Endpoint) -> RawClientBuilder {
        RawClientBuilder {
            settings: BuilderSettings::new(endpoint),
        }
    }

    /// Verifies the connection and returns the complete request round-trip time.
    pub async fn ping(&self) -> Result<Duration> {
        self.0.ping().await
    }

    /// Retrieves exact encoded bytes for an exact 32-byte item key.
    pub async fn get(&self, key: ItemKey) -> Result<GetOutcome<ItemValue>> {
        self.0.get(key).await
    }

    /// Stores exact encoded bytes with explicit wire-level set options.
    pub async fn set(
        &self,
        key: ItemKey,
        value: ItemValue,
        options: SetOptions,
    ) -> Result<SetOutcome> {
        self.0.set(key, value, options).await
    }

    /// Deletes an exact 32-byte item key.
    pub async fn delete(&self, key: ItemKey) -> Result<DeleteOutcome> {
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
/// Connection builder for the Compio low-level client.
pub struct LocalRawClientBuilder {
    settings: BuilderSettings,
}

#[cfg(feature = "quic-compio")]
builder_methods!(LocalRawClientBuilder);

#[cfg(feature = "quic-compio")]
impl LocalRawClientBuilder {
    /// Connects the configured exact protocol client on Compio.
    pub async fn connect(self) -> Result<LocalRawClient> {
        Ok(LocalRawClient(
            connect_compio(self.settings.finish()?).await?,
        ))
    }
}

#[cfg(feature = "quic-compio")]
impl LocalRawClient {
    /// Connects an exact protocol client with system trust on an active Compio runtime.
    pub async fn connect(endpoint: &str) -> Result<Self> {
        Self::builder(endpoint.parse()?).connect().await
    }

    /// Starts explicit configuration for an exact Compio protocol client.
    pub fn builder(endpoint: Endpoint) -> LocalRawClientBuilder {
        LocalRawClientBuilder {
            settings: BuilderSettings::new(endpoint),
        }
    }

    /// Verifies the connection and returns the complete request round-trip time.
    pub async fn ping(&self) -> Result<Duration> {
        self.0.ping().await
    }

    /// Retrieves exact encoded bytes for an exact 32-byte item key.
    pub async fn get(&self, key: ItemKey) -> Result<GetOutcome<ItemValue>> {
        self.0.get(key).await
    }

    /// Stores exact encoded bytes with explicit wire-level set options.
    pub async fn set(
        &self,
        key: ItemKey,
        value: ItemValue,
        options: SetOptions,
    ) -> Result<SetOutcome> {
        self.0.set(key, value, options).await
    }

    /// Deletes an exact 32-byte item key.
    pub async fn delete(&self, key: ItemKey) -> Result<DeleteOutcome> {
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

#[cfg(feature = "quic-quinn")]
async fn connect_quinn(
    settings: ConnectionSettings,
) -> Result<Arc<Core<transport::QuinnConnection>>> {
    let deadline = transport::Deadline::after(settings.timeouts.connect)?;
    let address = resolve_quinn(
        &settings.endpoint,
        deadline.remaining(Operation::DnsResolution)?,
    )
    .await?;
    Core::connect(
        address,
        settings.endpoint.server_name().to_owned(),
        settings.tls,
        settings.timeouts,
        settings.retry,
        settings.max_in_flight,
        deadline,
    )
    .await
    .map(Arc::new)
}

#[cfg(feature = "quic-compio")]
async fn connect_compio(
    settings: ConnectionSettings,
) -> Result<Arc<Core<transport::CompioConnection>>> {
    let deadline = transport::Deadline::after(settings.timeouts.connect)?;
    let address = resolve_compio(
        &settings.endpoint,
        deadline.remaining(Operation::DnsResolution)?,
    )
    .await?;
    Core::connect(
        address,
        settings.endpoint.server_name().to_owned(),
        settings.tls,
        settings.timeouts,
        settings.retry,
        settings.max_in_flight,
        deadline,
    )
    .await
    .map(Arc::new)
}

#[cfg(feature = "quic-quinn")]
async fn resolve_quinn(endpoint: &Endpoint, timeout: Duration) -> Result<SocketAddr> {
    if tokio::runtime::Handle::try_current().is_err() {
        return Err(Error::Runtime {
            backend: Backend::Quinn,
            message: "an active Tokio runtime is required".into(),
        });
    }
    if let Some(address) = endpoint.resolved_address() {
        return Ok(address);
    }
    let addresses = tokio::time::timeout(
        timeout,
        tokio::net::lookup_host((endpoint.host(), endpoint.port())),
    )
    .await
    .map_err(|_| Error::Timeout {
        operation: Operation::DnsResolution,
    })?
    .map_err(Error::io)?;
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
    if compio::runtime::Runtime::try_current().is_none() {
        return Err(Error::Runtime {
            backend: Backend::Compio,
            message: "an active Compio runtime is required".into(),
        });
    }
    if let Some(address) = endpoint.resolved_address() {
        return Ok(address);
    }
    let target = (endpoint.host(), endpoint.port());
    let addresses = compio::runtime::time::timeout(timeout, target.to_socket_addrs_async())
        .await
        .map_err(|_| Error::Timeout {
            operation: Operation::DnsResolution,
        })?
        .map_err(Error::io)?;
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
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(Error::tls)?
        .with_root_certificates(roots);
    let mut config = match identity {
        Some(identity) => {
            let (chain, key) = identity.into_rustls();
            builder
                .with_client_auth_cert(chain, key)
                .map_err(Error::tls)?
        }
        None => builder.with_no_client_auth(),
    };
    config.alpn_protocols = vec![openkache_protocol::ALPN.to_vec()];
    Ok(config)
}

fn expect_status(operation: Operation, status: Status, expected: &[Status]) -> Result<()> {
    if expected.contains(&status) {
        Ok(())
    } else {
        Err(unexpected_status(operation, status))
    }
}

fn unexpected_status(operation: Operation, status: Status) -> Error {
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

fn operation(opcode: Opcode) -> Operation {
    match opcode {
        Opcode::Ping => Operation::Ping,
        Opcode::Get => Operation::Get,
        Opcode::Set => Operation::Set,
        Opcode::Delete => Operation::Delete,
        Opcode::Stats => Operation::Stats,
        Opcode::Sync => Operation::Sync,
    }
}
