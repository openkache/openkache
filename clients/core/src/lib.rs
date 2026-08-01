//! Low-level QUIC client core for the OpenKache binary protocol.

/// Client-only defaults, ABI discriminators, and value-format constants generated from
/// `clients/model/openkache.smithy`.
pub mod contract {
    include!(concat!(env!("OUT_DIR"), "/client_contract.rs"));
}

mod config;
#[cfg(feature = "ffi")]
pub mod ffi;
mod key;
mod protected;
mod protection;
mod transport;
pub mod value;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

#[cfg(feature = "quic-compio")]
use compio::net::ToSocketAddrsAsync;
use openkache_protocol::{MAX_RESPONSE_FRAME_BYTES, Opcode, Request, Response, Status};
use transport::{ClientConnection, ClientLane};

pub use config::{
    Certificate, ClientIdentity, ClientTimeouts, Endpoint, PrivateKey, RetryPolicy, ServerTrust,
    SetOptions,
};
pub use contract::{ConnectionState, DEFAULT_MAX_IN_FLIGHT};
pub use key::{
    DATA_PROTECTION_KEY_BYTES, DataProtectionKey, DataProtectionKeyRing, ItemId,
    MAX_PREVIOUS_DATA_PROTECTION_KEYS, random_mutation_id,
};
pub use openkache_protocol::MutationId;
pub use openkache_protocol::{ITEM_ID_BYTES, SetCondition};
#[cfg(feature = "quic-compio")]
pub use protected::{LocalProtectedClient, LocalProtectedClientBuilder};
#[cfg(feature = "quic-quinn")]
pub use protected::{ProtectedClient, ProtectedClientBuilder};
pub use protection::DataProtection;
pub use value::ItemValue;

#[cfg(not(any(feature = "quic-compio", feature = "quic-quinn")))]
compile_error!("enable at least one client QUIC backend feature");

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

impl Backend {
    /// Returns the Smithy-defined native backend discriminator.
    pub const fn ffi_code(self) -> u32 {
        match self {
            Self::Quinn => contract::FFI_BACKEND_QUINN,
            Self::Compio => contract::FFI_BACKEND_COMPIO,
        }
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

impl Operation {
    /// Returns the caller-facing FFI operation discriminator when this value
    /// identifies a protocol operation.
    pub const fn ffi_operation_code(self) -> u32 {
        match self {
            Self::Ping => contract::FfiOperation::Ping.code(),
            Self::Get => contract::FfiOperation::Get.code(),
            Self::Set => contract::FfiOperation::Set.code(),
            Self::Delete => contract::FfiOperation::Delete.code(),
            Self::Stats => contract::FfiOperation::Stats.code(),
            Self::Sync => contract::FfiOperation::Sync.code(),
            Self::DnsResolution
            | Self::ConnectionSetup
            | Self::ConnectionRetry
            | Self::StreamAcquisition
            | Self::RequestWrite
            | Self::ResponseHeaderRead
            | Self::ResponseBodyRead
            | Self::TlsInitialization
            | Self::EndpointInitialization
            | Self::ConnectionInitialization
            | Self::Handshake
            | Self::StreamOpen
            | Self::StreamWrite
            | Self::StreamRead => 0,
        }
    }

    /// Returns the Smithy-defined phase discriminator for this operation.
    pub const fn ffi_phase_code(self) -> u32 {
        match self {
            Self::DnsResolution => contract::FFI_PHASE_DNS_RESOLUTION,
            Self::ConnectionSetup => contract::FFI_PHASE_CONNECTION_SETUP,
            Self::ConnectionRetry => contract::FFI_PHASE_CONNECTION_RETRY,
            Self::StreamAcquisition => contract::FFI_PHASE_STREAM_ACQUISITION,
            Self::RequestWrite => contract::FFI_PHASE_REQUEST_WRITE,
            Self::ResponseHeaderRead => contract::FFI_PHASE_RESPONSE_HEADER_READ,
            Self::ResponseBodyRead => contract::FFI_PHASE_RESPONSE_BODY_READ,
            Self::TlsInitialization => contract::FFI_PHASE_TLS_INITIALIZATION,
            Self::EndpointInitialization => contract::FFI_PHASE_ENDPOINT_INITIALIZATION,
            Self::ConnectionInitialization => contract::FFI_PHASE_CONNECTION_INITIALIZATION,
            Self::Handshake => contract::FFI_PHASE_HANDSHAKE,
            Self::StreamOpen => contract::FFI_PHASE_STREAM_OPEN,
            Self::StreamWrite => contract::FFI_PHASE_STREAM_WRITE,
            Self::StreamRead => contract::FFI_PHASE_STREAM_READ,
            Self::Ping | Self::Get | Self::Set | Self::Delete | Self::Stats | Self::Sync => {
                contract::FFI_PHASE_UNKNOWN
            }
        }
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
    /// The mutation token was already used for a different request.
    MutationConflict,
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
        /// Token that can be reused to safely retry the mutation.
        ///
        /// Administrative `SYNC` requests retain the legacy ambiguous outcome
        /// behavior and therefore have no mutation token.
        mutation_id: Option<MutationId>,
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

/// Transport and protocol counters collected by one core client connection.
///
/// The snapshot is intentionally independent of the native FFI counters so
/// language adapters can expose the same retry/reconnect/error observations
/// without reimplementing transport behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CoreMetricsSnapshot {
    /// Number of retry attempts after an initial request attempt.
    pub retries: u64,
    /// Number of replacement connections established.
    pub reconnects: u64,
    /// Number of transport failures observed while executing requests.
    pub transport_errors: u64,
    /// Number of protocol encoding/decoding failures observed while executing requests.
    pub protocol_errors: u64,
}

#[derive(Default)]
struct CoreMetrics {
    retries: AtomicU64,
    reconnects: AtomicU64,
    transport_errors: AtomicU64,
    protocol_errors: AtomicU64,
}

struct Core<C: ClientConnection> {
    connection: RwLock<Arc<C>>,
    reconnect: futures_util::lock::Mutex<()>,
    addresses: Vec<SocketAddr>,
    next_address: AtomicUsize,
    reconnect_attempts: AtomicUsize,
    server_name: String,
    tls: rustls::ClientConfig,
    connect_timeout: Duration,
    request_timeout: Duration,
    retry: RetryPolicy,
    max_in_flight: usize,
    state: AtomicU32,
    metrics: CoreMetrics,
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
        addresses: Vec<SocketAddr>,
        server_name: String,
        tls: rustls::ClientConfig,
        timeouts: ClientTimeouts,
        retry: RetryPolicy,
        max_in_flight: usize,
        deadline: transport::Deadline,
    ) -> Result<Self> {
        let mut last_error = None;
        let mut connection = None;
        let mut connected_address_index = None;
        for (index, address) in addresses.iter().enumerate() {
            match C::connect(
                *address,
                &server_name,
                tls.clone(),
                deadline.remaining(Operation::ConnectionSetup)?,
                max_in_flight,
            )
            .await
            {
                Ok(value) => {
                    connection = Some(value);
                    connected_address_index = Some(index);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let connection = connection.ok_or_else(|| {
            last_error.unwrap_or_else(|| Error::Connection("DNS returned no addresses".into()))
        })?;
        let next_address = connected_address_index
            .map(|index| (index + 1) % addresses.len())
            .unwrap_or(0);
        Ok(Self {
            connection: RwLock::new(Arc::new(connection)),
            reconnect: futures_util::lock::Mutex::new(()),
            addresses,
            next_address: AtomicUsize::new(next_address),
            reconnect_attempts: AtomicUsize::new(0),
            server_name,
            tls,
            connect_timeout: timeouts.connect,
            request_timeout: timeouts.request,
            retry,
            max_in_flight,
            state: AtomicU32::new(ConnectionState::Connected.code()),
            metrics: CoreMetrics::default(),
        })
    }

    fn metrics_snapshot(&self) -> CoreMetricsSnapshot {
        CoreMetricsSnapshot {
            retries: self.metrics.retries.load(Ordering::Relaxed),
            reconnects: self.metrics.reconnects.load(Ordering::Relaxed),
            transport_errors: self.metrics.transport_errors.load(Ordering::Relaxed),
            protocol_errors: self.metrics.protocol_errors.load(Ordering::Relaxed),
        }
    }

    fn connection_state(&self) -> ConnectionState {
        ConnectionState::try_from(self.state.load(Ordering::Acquire))
            .unwrap_or(ConnectionState::Unknown)
    }

    async fn ping(&self) -> Result<Duration> {
        self.ping_with_transmission(None).await
    }

    async fn ping_with_transmission(&self, transmission: Option<&AtomicBool>) -> Result<Duration> {
        let started = Instant::now();
        let response = self
            .request_with_transmission(
                Request::new(Opcode::Ping, None, Vec::new()).map_err(Error::protocol)?,
                transmission,
            )
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

    async fn get(&self, item_id: ItemId) -> Result<GetOutcome<ItemValue>> {
        self.get_with_transmission(item_id, None).await
    }

    async fn get_with_transmission(
        &self,
        item_id: ItemId,
        transmission: Option<&AtomicBool>,
    ) -> Result<GetOutcome<ItemValue>> {
        let response = self
            .request_with_transmission(
                Request::new(Opcode::Get, Some(item_id.into_protocol()), Vec::new())
                    .map_err(Error::protocol)?,
                transmission,
            )
            .await?;
        match response.status {
            Status::Ok => Ok(GetOutcome::Found(ItemValue::new(response.payload))),
            Status::NotFound => Ok(GetOutcome::NotFound),
            status => Err(unexpected_status(Operation::Get, status)),
        }
    }

    async fn set(
        &self,
        item_id: ItemId,
        value: ItemValue,
        options: SetOptions,
    ) -> Result<SetOutcome> {
        self.set_with_transmission(item_id, value, options, None)
            .await
    }

    async fn set_with_transmission(
        &self,
        item_id: ItemId,
        value: ItemValue,
        options: SetOptions,
        transmission: Option<&AtomicBool>,
    ) -> Result<SetOutcome> {
        let mutation_id = options.mutation_id().unwrap_or(random_mutation_id()?);
        let request = Request::new_set(
            Opcode::Set,
            Some(item_id.into_protocol()),
            options.into_protocol(mutation_id)?,
            value.into_bytes(),
        )
        .map_err(Error::protocol)?;
        match self
            .request_with_transmission(request, transmission)
            .await?
            .status
        {
            Status::Created => Ok(SetOutcome::Created),
            Status::Replaced => Ok(SetOutcome::Replaced),
            Status::NotStored => Ok(SetOutcome::NotStored),
            status => Err(unexpected_status(Operation::Set, status)),
        }
    }

    async fn delete(&self, item_id: ItemId) -> Result<DeleteOutcome> {
        self.delete_with_transmission(item_id, random_mutation_id()?, None)
            .await
    }

    async fn delete_with_transmission(
        &self,
        item_id: ItemId,
        mutation_id: MutationId,
        transmission: Option<&AtomicBool>,
    ) -> Result<DeleteOutcome> {
        self.delete_with_mutation_id(item_id, mutation_id, transmission)
            .await
    }

    async fn delete_with_mutation_id(
        &self,
        item_id: ItemId,
        mutation_id: MutationId,
        transmission: Option<&AtomicBool>,
    ) -> Result<DeleteOutcome> {
        let response = self
            .request_with_transmission(
                Request::new_with_mutation(
                    Opcode::Delete,
                    Some(item_id.into_protocol()),
                    Some(mutation_id),
                    Vec::new(),
                )
                .map_err(Error::protocol)?,
                transmission,
            )
            .await?;
        match response.status {
            Status::Deleted => Ok(DeleteOutcome::Deleted),
            Status::NotFound => Ok(DeleteOutcome::NotFound),
            status => Err(unexpected_status(Operation::Delete, status)),
        }
    }

    async fn stats(&self) -> Result<String> {
        self.stats_with_transmission(None).await
    }

    async fn stats_with_transmission(&self, transmission: Option<&AtomicBool>) -> Result<String> {
        let response = self
            .request_with_transmission(
                Request::new(Opcode::Stats, None, Vec::new()).map_err(Error::protocol)?,
                transmission,
            )
            .await?;
        expect_status(Operation::Stats, response.status, &[Status::Ok])?;
        String::from_utf8(response.payload)
            .map_err(|error| Error::Protocol(format!("STATS response is not UTF-8: {error}")))
    }

    async fn sync(&self) -> Result<()> {
        self.sync_with_transmission(None).await
    }

    async fn sync_with_transmission(&self, transmission: Option<&AtomicBool>) -> Result<()> {
        let response = self
            .request_with_transmission(
                Request::new(Opcode::Sync, None, Vec::new()).map_err(Error::protocol)?,
                transmission,
            )
            .await?;
        expect_status(Operation::Sync, response.status, &[Status::Ok])
    }

    async fn request_with_transmission(
        &self,
        request: Request,
        transmission: Option<&AtomicBool>,
    ) -> Result<Response> {
        if self.connection_state() == ConnectionState::Closed {
            return Err(Error::ClientClosed);
        }
        let deadline = transport::Deadline::after(self.request_timeout)?;
        if self.connection_state() == ConnectionState::Disconnected {
            self.reconnect_before(deadline).await?;
        }
        let response_safe = matches!(request.opcode, Opcode::Ping | Opcode::Get | Opcode::Stats);
        let mutation_id = request.mutation_id;
        let mutation = mutation_id.is_some();
        let max_attempts = if response_safe || mutation {
            self.retry.max_attempts
        } else {
            1
        };
        let operation = operation(request.opcode);
        let mut request = Some(request);
        for attempt in 1..=max_attempts {
            if attempt > 1 {
                self.metrics.retries.fetch_add(1, Ordering::Relaxed);
                let delay = self.retry.delay_before(attempt);
                if delay > deadline.remaining(operation)? {
                    return Err(Error::Timeout { operation });
                }
                C::sleep(delay).await;
            }
            let connection = self.current_connection()?;
            let attempt_request = if attempt == max_attempts {
                request.take()
            } else {
                request.as_ref().cloned()
            };
            let Some(attempt_request) = attempt_request else {
                return Err(Error::Connection(
                    "request retry state was exhausted before the final attempt".into(),
                ));
            };
            match self
                .request_once(
                    &connection,
                    attempt_request,
                    deadline,
                    transmission,
                    // A retried request may be racing another retry on a
                    // replacement connection. Discarding its stream after
                    // the response prevents a remotely finished stream from
                    // being returned to the idle pool and reused by the
                    // other attempt.
                    attempt == 1,
                )
                .await
            {
                Ok(response) if response.status.is_error() => {
                    let error = Error::Server {
                        code: server_error_code(response.status),
                        message: String::from_utf8_lossy(&response.payload).into_owned(),
                    };
                    // A server-side timeout means execution may have
                    // started before the response deadline. Preserve the
                    // mutation token (and SYNC's legacy ambiguity) so the
                    // caller can safely retry or reconcile the outcome.
                    if (mutation || operation == Operation::Sync)
                        && response.status == Status::Timeout
                    {
                        return Err(Error::AmbiguousOutcome {
                            operation,
                            mutation_id,
                            cause: Box::new(error),
                        });
                    }
                    return Err(error);
                }
                Ok(response) => return Ok(response),
                Err(failure) => {
                    match &failure.error {
                        Error::Transport { .. } | Error::Connection(_) | Error::Io(_) => {
                            self.metrics
                                .transport_errors
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        Error::Protocol(_) => {
                            self.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                    if failure.invalidates_connection {
                        self.mark_disconnected(&connection);
                    }
                    if (response_safe || mutation)
                        && failure.invalidates_connection
                        && attempt < max_attempts
                    {
                        // A caller may close the client while a mutation is
                        // waiting for its response. The request was already
                        // transmitted, so reconnecting would both violate
                        // the terminal close contract and lose the token's
                        // useful ambiguity information.
                        if self.connection_state() == ConnectionState::Closed {
                            if (mutation || operation == Operation::Sync)
                                && failure.may_have_reached_server
                            {
                                return Err(Error::AmbiguousOutcome {
                                    operation,
                                    mutation_id,
                                    cause: Box::new(failure.error),
                                });
                            }
                            return Err(failure.error);
                        }
                        if let Err(reconnect_error) =
                            self.reconnect_failed(&connection, deadline).await
                        {
                            if (mutation || operation == Operation::Sync)
                                && failure.may_have_reached_server
                            {
                                return Err(Error::AmbiguousOutcome {
                                    operation,
                                    mutation_id,
                                    cause: Box::new(reconnect_error),
                                });
                            }
                            return Err(reconnect_error);
                        }
                        continue;
                    }
                    if (mutation || operation == Operation::Sync) && failure.may_have_reached_server
                    {
                        return Err(Error::AmbiguousOutcome {
                            operation,
                            mutation_id,
                            cause: Box::new(failure.error),
                        });
                    }
                    return Err(failure.error);
                }
            }
        }
        Err(Error::Connection(
            "request retry policy did not permit an attempt".into(),
        ))
    }

    async fn request_once(
        &self,
        connection: &C,
        request: Request,
        deadline: transport::Deadline,
        transmission: Option<&AtomicBool>,
        reuse_lane: bool,
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
        // A write can be interrupted after only part of the frame reached
        // the transport. Mark the request as potentially transmitted before
        // starting it so cancellation and write-timeout errors retain the
        // mutation token needed for a safe retry.
        if let Some(transmission) = transmission {
            transmission.store(true, Ordering::Release);
        }
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
        if reuse_lane {
            stream.release();
        }
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
                (state != ConnectionState::Closed.code())
                    .then_some(ConnectionState::Disconnected.code())
            });
    }

    fn set_state_unless_closed(&self, next: ConnectionState) -> Result<()> {
        self.state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state != ConnectionState::Closed.code()).then_some(next.code())
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
        self.set_state_unless_closed(ConnectionState::Reconnecting)?;
        let reconnect_attempt = self.reconnect_attempts.fetch_add(1, Ordering::Relaxed) + 1;
        let backoff = self.retry.delay_before(reconnect_attempt.saturating_add(1));
        let remaining = deadline
            .remaining(Operation::ConnectionRetry)
            .inspect_err(|_| self.mark_disconnected(failed))?;
        if backoff > remaining {
            self.mark_disconnected(failed);
            return Err(Error::Timeout {
                operation: Operation::ConnectionRetry,
            });
        }
        if !backoff.is_zero() {
            C::sleep(backoff).await;
        }
        let start = self.next_address.fetch_add(1, Ordering::Relaxed);
        let mut replacement = None;
        let mut last_error = None;
        for offset in 0..self.addresses.len() {
            let address = self.addresses[(start + offset) % self.addresses.len()];
            let timeout = deadline
                .remaining(Operation::ConnectionRetry)
                .inspect_err(|_| self.mark_disconnected(failed))?
                .min(self.connect_timeout);
            match C::connect(
                address,
                &self.server_name,
                self.tls.clone(),
                timeout,
                self.max_in_flight,
            )
            .await
            {
                Ok(connection) => {
                    replacement = Some(connection);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let replacement = match replacement {
            Some(connection) => connection,
            None => {
                self.mark_disconnected(failed);
                return Err(last_error
                    .unwrap_or_else(|| Error::Connection("all resolved addresses failed".into())));
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
        if !Arc::ptr_eq(&connection, failed) {
            replacement.close();
            return Ok(());
        }
        failed.close();
        *connection = Arc::new(replacement);
        self.metrics.reconnects.fetch_add(1, Ordering::Relaxed);
        self.reconnect_attempts.store(0, Ordering::Relaxed);
        self.set_state_unless_closed(ConnectionState::Connected)?;
        Ok(())
    }

    async fn reconnect(&self) -> Result<()> {
        let deadline = transport::Deadline::after(self.connect_timeout)?;
        let current = self.current_connection()?;
        self.mark_disconnected(&current);
        self.reconnect_failed(&current, deadline).await
    }

    async fn close(&self) -> Result<()> {
        let previous = self
            .state
            .swap(ConnectionState::Closed.code(), Ordering::AcqRel);
        if previous != ConnectionState::Closed.code() {
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
        if self.max_in_flight > u32::MAX as usize {
            return Err(Error::configuration(
                "max_in_flight",
                "must not exceed u32::MAX",
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

macro_rules! raw_client_methods {
    ($client:ident) => {
        impl $client {
            /// Verifies the connection and returns the complete request round-trip time.
            pub async fn ping(&self) -> Result<Duration> {
                self.0.ping().await
            }

            /// Retrieves exact encoded bytes for a fixed-size item ID.
            pub async fn get(&self, item_id: ItemId) -> Result<GetOutcome<ItemValue>> {
                self.0.get(item_id).await
            }

            /// Stores exact encoded bytes with explicit wire-level set options.
            pub async fn set(
                &self,
                item_id: ItemId,
                value: ItemValue,
                options: SetOptions,
            ) -> Result<SetOutcome> {
                self.0.set(item_id, value, options).await
            }

            #[allow(dead_code)]
            pub(crate) async fn set_with_transmission(
                &self,
                item_id: ItemId,
                value: ItemValue,
                options: SetOptions,
                transmission: &AtomicBool,
            ) -> Result<SetOutcome> {
                self.0
                    .set_with_transmission(item_id, value, options, Some(transmission))
                    .await
            }

            /// Deletes a fixed-size item ID.
            pub async fn delete(&self, item_id: ItemId) -> Result<DeleteOutcome> {
                self.0.delete(item_id).await
            }

            /// Deletes an item while reusing the supplied idempotency token.
            pub async fn delete_with_mutation_id(
                &self,
                item_id: ItemId,
                mutation_id: MutationId,
            ) -> Result<DeleteOutcome> {
                self.0
                    .delete_with_mutation_id(item_id, mutation_id, None)
                    .await
            }

            #[allow(dead_code)]
            pub(crate) async fn delete_with_mutation_id_with_transmission(
                &self,
                item_id: ItemId,
                mutation_id: MutationId,
                transmission: &AtomicBool,
            ) -> Result<DeleteOutcome> {
                self.0
                    .delete_with_mutation_id(item_id, mutation_id, Some(transmission))
                    .await
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

            /// Returns a point-in-time snapshot of retry, reconnect, and
            /// transport/protocol error counters.
            pub fn metrics_snapshot(&self) -> CoreMetricsSnapshot {
                self.0.metrics_snapshot()
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
    };
}

#[cfg(feature = "quic-quinn")]
/// Exact-item-ID, exact-value protocol client running on Tokio and Quinn.
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
}

#[cfg(feature = "quic-quinn")]
raw_client_methods!(RawClient);

#[cfg(feature = "quic-compio")]
/// Exact-item-ID, exact-value protocol client confined to a Compio runtime.
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
}

#[cfg(feature = "quic-compio")]
raw_client_methods!(LocalRawClient);

#[cfg(feature = "quic-quinn")]
async fn connect_quinn(
    settings: ConnectionSettings,
) -> Result<Arc<Core<transport::QuinnConnection>>> {
    let deadline = transport::Deadline::after(settings.timeouts.connect)?;
    let addresses = resolve_quinn(
        &settings.endpoint,
        deadline.remaining(Operation::DnsResolution)?,
    )
    .await?;
    Core::connect(
        addresses,
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
    let addresses = resolve_compio(
        &settings.endpoint,
        deadline.remaining(Operation::DnsResolution)?,
    )
    .await?;
    Core::connect(
        addresses,
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
async fn resolve_quinn(endpoint: &Endpoint, timeout: Duration) -> Result<Vec<SocketAddr>> {
    if tokio::runtime::Handle::try_current().is_err() {
        return Err(Error::Runtime {
            backend: Backend::Quinn,
            message: "an active Tokio runtime is required".into(),
        });
    }
    if let Some(address) = endpoint.resolved_address() {
        return Ok(vec![address]);
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
    let addresses = addresses.collect::<Vec<_>>();
    (!addresses.is_empty()).then_some(addresses).ok_or_else(|| {
        Error::Connection(format!(
            "{}:{} resolved to no addresses",
            endpoint.host(),
            endpoint.port()
        ))
    })
}

#[cfg(feature = "quic-compio")]
async fn resolve_compio(endpoint: &Endpoint, timeout: Duration) -> Result<Vec<SocketAddr>> {
    if compio::runtime::Runtime::try_current().is_none() {
        return Err(Error::Runtime {
            backend: Backend::Compio,
            message: "an active Compio runtime is required".into(),
        });
    }
    if let Some(address) = endpoint.resolved_address() {
        return Ok(vec![address]);
    }
    let target = (endpoint.host(), endpoint.port());
    let addresses = compio::runtime::time::timeout(timeout, target.to_socket_addrs_async())
        .await
        .map_err(|_| Error::Timeout {
            operation: Operation::DnsResolution,
        })?
        .map_err(Error::io)?;
    let addresses = addresses.into_iter().collect::<Vec<_>>();
    (!addresses.is_empty()).then_some(addresses).ok_or_else(|| {
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
        Status::MutationConflict => ServerErrorCode::MutationConflict,
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
