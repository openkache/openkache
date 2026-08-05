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
pub mod value_envelope;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

#[cfg(feature = "quic-compio")]
use compio::net::ToSocketAddrsAsync;
use openkache_protocol::{MAX_RESPONSE_FRAME_BYTES, Opcode, Request, Response, Status};
use transport::{ClientConnection, ClientLane};

pub use config::{
    AlpnPolicy, Certificate, ClientIdentity, ClientTimeouts, Endpoint, PrivateKey, RetryPolicy,
    ServerTrust, SetOptions,
};
pub use contract::{ConnectionState, DEFAULT_MAX_IN_FLIGHT};
pub use key::{DATA_PROTECTION_KEY_BYTES, DataProtectionKey, ItemId};
pub use openkache_protocol::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, ITEM_ID_BYTES,
    NamespaceDescriptor, NamespacePolicy, OverridePolicy, SetCondition,
};
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
    /// `NAMESPACE_OPEN` request.
    NamespaceOpen,
    /// `NAMESPACE_UPDATE_POLICY` request.
    NamespaceUpdatePolicy,
    /// `NAMESPACE_DELETE` request.
    NamespaceDelete,
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
            Self::NamespaceOpen => "NAMESPACE_OPEN",
            Self::NamespaceUpdatePolicy => "NAMESPACE_UPDATE_POLICY",
            Self::NamespaceDelete => "NAMESPACE_DELETE",
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
    /// The request could not be admitted because protected items consume available capacity.
    NoCapacity,
    /// The request selected an item policy disallowed by its namespace.
    PolicyConflict,
    /// An optimistic namespace revision did not match.
    Conflict,
    /// The requested namespace does not exist.
    NamespaceNotFound,
    /// Namespace deletion requires an empty namespace.
    NamespaceNotEmpty,
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

struct Core<C: ClientConnection> {
    connection: RwLock<Arc<C>>,
    reconnect: futures_util::lock::Mutex<()>,
    namespace_open: futures_util::lock::Mutex<()>,
    address: SocketAddr,
    server_name: String,
    tls: rustls::ClientConfig,
    alpn: AlpnPolicy,
    connect_timeout: Duration,
    request_timeout: Duration,
    retry: RetryPolicy,
    max_in_flight: usize,
    namespace_id: AtomicU64,
    namespace_name: Vec<u8>,
    namespace_policy: NamespacePolicy,
    state: AtomicU32,
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

    fn after_response(error: Error) -> Self {
        Self {
            error,
            may_have_reached_server: true,
            invalidates_connection: false,
        }
    }
}

impl<C: ClientConnection> Core<C> {
    async fn connect(
        address: SocketAddr,
        server_name: String,
        tls: rustls::ClientConfig,
        alpn: AlpnPolicy,
        timeouts: ClientTimeouts,
        retry: RetryPolicy,
        max_in_flight: usize,
        namespace_id: Option<u64>,
        namespace_name: Vec<u8>,
        namespace_policy: NamespacePolicy,
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
        if let Some(protocol) = connection.negotiated_alpn() {
            if let Err(error) = alpn.validate_negotiated(protocol) {
                connection.close();
                return Err(error);
            }
        } else {
            connection.close();
            return Err(Error::Connection(
                "server did not negotiate an ALPN protocol".into(),
            ));
        }
        let core = Self {
            connection: RwLock::new(Arc::new(connection)),
            reconnect: futures_util::lock::Mutex::new(()),
            namespace_open: futures_util::lock::Mutex::new(()),
            address,
            server_name,
            tls,
            alpn,
            connect_timeout: timeouts.connect,
            request_timeout: timeouts.request,
            retry,
            max_in_flight,
            namespace_id: AtomicU64::new(namespace_id.unwrap_or(0)),
            namespace_name,
            namespace_policy,
            state: AtomicU32::new(ConnectionState::Connected.code()),
        };
        Ok(core)
    }

    fn connection_state(&self) -> ConnectionState {
        ConnectionState::try_from(self.state.load(Ordering::Acquire))
            .unwrap_or(ConnectionState::Unknown)
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

    async fn get(&self, item_id: ItemId) -> Result<GetOutcome<ItemValue>> {
        let namespace_id = self.ensure_namespace().await?;
        self.get_in_namespace(namespace_id, item_id).await
    }

    async fn get_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
    ) -> Result<GetOutcome<ItemValue>> {
        validate_client_namespace_id(namespace_id)?;
        let response = self
            .request(
                Request::new_scoped(
                    Opcode::Get,
                    namespace_id,
                    Some(item_id.into_protocol()),
                    Vec::new(),
                )
                .map_err(Error::protocol)?,
            )
            .await?;
        match response.status {
            Status::Ok => Ok(GetOutcome::Found(ItemValue::new(response.payload))),
            Status::NotFound if response.payload.is_empty() => Ok(GetOutcome::NotFound),
            Status::NotFound => Err(Error::UnexpectedResponse {
                operation: Operation::Get,
                message: "NotFound response must have an empty payload".into(),
            }),
            status => Err(unexpected_status(Operation::Get, status)),
        }
    }

    async fn set(
        &self,
        item_id: ItemId,
        value: ItemValue,
        options: SetOptions,
    ) -> Result<SetOutcome> {
        let namespace_id = self.ensure_namespace().await?;
        self.set_in_namespace(namespace_id, item_id, value, options)
            .await
    }

    async fn set_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: ItemValue,
        options: SetOptions,
    ) -> Result<SetOutcome> {
        validate_client_namespace_id(namespace_id)?;
        let request = Request::new_scoped_with_options(
            Opcode::Set,
            namespace_id,
            Some(item_id.into_protocol()),
            options.into_protocol()?,
            value.into_bytes(),
        )
        .map_err(Error::protocol)?;
        let response = self.request(request).await?;
        match response.status {
            Status::Created if response.payload.is_empty() => Ok(SetOutcome::Created),
            Status::Replaced if response.payload.is_empty() => Ok(SetOutcome::Replaced),
            Status::NotStored if response.payload.is_empty() => Ok(SetOutcome::NotStored),
            Status::Created | Status::Replaced | Status::NotStored => {
                Err(Error::UnexpectedResponse {
                    operation: Operation::Set,
                    message: "SET success responses must have an empty payload".into(),
                })
            }
            status => Err(unexpected_status(Operation::Set, status)),
        }
    }

    async fn delete(&self, item_id: ItemId) -> Result<DeleteOutcome> {
        let namespace_id = self.ensure_namespace().await?;
        self.delete_in_namespace(namespace_id, item_id).await
    }

    async fn delete_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
    ) -> Result<DeleteOutcome> {
        validate_client_namespace_id(namespace_id)?;
        let response = self
            .request(
                Request::new_scoped(
                    Opcode::Delete,
                    namespace_id,
                    Some(item_id.into_protocol()),
                    Vec::new(),
                )
                .map_err(Error::protocol)?,
            )
            .await?;
        match response.status {
            Status::Deleted if response.payload.is_empty() => Ok(DeleteOutcome::Deleted),
            Status::NotFound if response.payload.is_empty() => Ok(DeleteOutcome::NotFound),
            Status::Deleted | Status::NotFound => Err(Error::UnexpectedResponse {
                operation: Operation::Delete,
                message: "DELETE domain responses must have an empty payload".into(),
            }),
            status => Err(unexpected_status(Operation::Delete, status)),
        }
    }

    async fn stats(&self) -> Result<String> {
        let namespace_id = self.ensure_namespace().await?;
        self.stats_in_namespace(namespace_id).await
    }

    async fn stats_in_namespace(&self, namespace_id: u64) -> Result<String> {
        validate_client_namespace_id(namespace_id)?;
        let response = self
            .request(
                Request::new_scoped(Opcode::Stats, namespace_id, None, Vec::new())
                    .map_err(Error::protocol)?,
            )
            .await?;
        expect_status(Operation::Stats, response.status, &[Status::Ok])?;
        validate_stats_payload(&response.payload)?;
        String::from_utf8(response.payload)
            .map_err(|error| Error::Protocol(format!("STATS response is not UTF-8: {error}")))
    }

    async fn sync(&self) -> Result<()> {
        let namespace_id = self.ensure_namespace().await?;
        self.sync_in_namespace(namespace_id).await
    }

    async fn sync_in_namespace(&self, namespace_id: u64) -> Result<()> {
        validate_client_namespace_id(namespace_id)?;
        let response = self
            .request(
                Request::new_scoped(Opcode::Sync, namespace_id, None, Vec::new())
                    .map_err(Error::protocol)?,
            )
            .await?;
        expect_status(Operation::Sync, response.status, &[Status::Ok])?;
        if response.payload.is_empty() {
            Ok(())
        } else {
            Err(Error::UnexpectedResponse {
                operation: Operation::Sync,
                message: "SYNC success responses must have an empty payload".into(),
            })
        }
    }

    async fn request(&self, request: Request) -> Result<Response> {
        if self.connection_state() == ConnectionState::Closed {
            return Err(Error::ClientClosed);
        }
        let deadline = transport::Deadline::after(self.request_timeout)?;
        if self.connection_state() == ConnectionState::Disconnected {
            self.reconnect_before(deadline).await?;
        }
        let response_safe = matches!(request.opcode, Opcode::Ping | Opcode::Get | Opcode::Stats)
            || (request.opcode == Opcode::NamespaceOpen && !request.create_if_missing);
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
        Err(Error::Connection(
            "request retry policy did not permit an attempt".into(),
        ))
    }

    fn selected_namespace_id(&self) -> Result<u64> {
        match self.namespace_id.load(Ordering::Acquire) {
            namespace_id @ 1.. => Ok(namespace_id),
            0 => Err(Error::configuration(
                "namespace",
                "no namespace is selected; call namespace_open first",
            )),
        }
    }

    async fn ensure_namespace(&self) -> Result<u64> {
        if let Ok(namespace_id) = self.selected_namespace_id() {
            return Ok(namespace_id);
        }
        let _guard = self.namespace_open.lock().await;
        if let Ok(namespace_id) = self.selected_namespace_id() {
            return Ok(namespace_id);
        }
        self.open_namespace(
            self.namespace_name.clone(),
            true,
            Some(self.namespace_policy),
        )
        .await
        .map(|descriptor| descriptor.namespace_id)
    }

    /// Returns the currently selected server-assigned namespace ID.
    fn namespace_id(&self) -> Option<u64> {
        match self.namespace_id.load(Ordering::Acquire) {
            namespace_id @ 1.. => Some(namespace_id),
            0 => None,
        }
    }

    async fn open_namespace(
        &self,
        name: Vec<u8>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> Result<NamespaceDescriptor> {
        self.open_namespace_with_outcome(name, create_if_missing, policy)
            .await
            .map(|(descriptor, _created)| descriptor)
    }

    async fn open_namespace_with_outcome(
        &self,
        name: Vec<u8>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> Result<(NamespaceDescriptor, bool)> {
        let response = self
            .request(
                Request::namespace_open(name, create_if_missing, policy)
                    .map_err(Error::protocol)?,
            )
            .await?;
        let operation = Operation::NamespaceOpen;
        let status = response.status;
        if !matches!(status, Status::Ok | Status::Created) {
            return Err(unexpected_status(operation, status));
        }
        let descriptor = NamespaceDescriptor::decode(&response.payload).map_err(Error::protocol)?;
        self.namespace_id
            .store(descriptor.namespace_id, Ordering::Release);
        Ok((descriptor, status == Status::Created))
    }

    async fn update_namespace_policy(
        &self,
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> Result<NamespaceDescriptor> {
        let response = self
            .request(
                Request::namespace_update_policy(namespace_id, expected_revision, policy)
                    .map_err(Error::protocol)?,
            )
            .await?;
        expect_status(
            Operation::NamespaceUpdatePolicy,
            response.status,
            &[Status::Ok],
        )?;
        let descriptor = NamespaceDescriptor::decode(&response.payload).map_err(Error::protocol)?;
        if descriptor.namespace_id != namespace_id {
            return Err(Error::UnexpectedResponse {
                operation: Operation::NamespaceUpdatePolicy,
                message: format!(
                    "descriptor namespace ID {} does not match requested namespace ID {namespace_id}",
                    descriptor.namespace_id
                ),
            });
        }
        let expected_next_revision = expected_revision.checked_add(1).ok_or_else(|| {
            Error::UnexpectedResponse {
                operation: Operation::NamespaceUpdatePolicy,
                message: "successful policy update cannot follow the maximum revision".into(),
            }
        })?;
        if descriptor.revision != expected_next_revision {
            return Err(Error::UnexpectedResponse {
                operation: Operation::NamespaceUpdatePolicy,
                message: format!(
                    "descriptor revision {} does not follow expected revision {expected_revision}",
                    descriptor.revision
                ),
            });
        }
        if self.namespace_id.load(Ordering::Acquire) == namespace_id {
            self.namespace_id
                .store(descriptor.namespace_id, Ordering::Release);
        }
        Ok(descriptor)
    }

    async fn delete_namespace(&self, namespace_id: u64, expected_revision: u64) -> Result<()> {
        let response = self
            .request(
                Request::namespace_delete(namespace_id, expected_revision)
                    .map_err(Error::protocol)?,
            )
            .await?;
        expect_status(
            Operation::NamespaceDelete,
            response.status,
            &[Status::Deleted],
        )?;
        if !response.payload.is_empty() {
            return Err(Error::UnexpectedResponse {
                operation: Operation::NamespaceDelete,
                message: "NAMESPACE_DELETE success responses must have an empty payload".into(),
            });
        }
        let _ = self.namespace_id.compare_exchange(
            namespace_id,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        Ok(())
    }

    async fn request_once(
        &self,
        connection: &C,
        request: Request,
        deadline: transport::Deadline,
    ) -> std::result::Result<Response, RequestFailure> {
        let opcode = request.opcode;
        let create_if_missing = request.create_if_missing;
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
        // Validate the operation/status/payload contract before returning the lane to
        // the pool. A status that is not meaningful for this request is a protocol
        // violation; the lane must be discarded even when the QUIC connection remains
        // usable. This also prevents a malformed success from being mistaken for a
        // definitive mutation result.
        if let Err(error) =
            validate_response_contract(opcode, create_if_missing, &response)
        {
            return Err(RequestFailure::after_response(error));
        }
        // Error responses may be emitted while the server is still parsing a request,
        // in which case the server terminates the lane. Retiring every error lane is
        // conservative and remains valid for errors that the server could have
        // returned on a reusable lane.
        if !response.status.is_error() {
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
        if let Some(protocol) = replacement.negotiated_alpn() {
            if let Err(error) = self.alpn.validate_negotiated(protocol) {
                replacement.close();
                self.mark_disconnected(failed);
                return Err(error);
            }
        } else {
            replacement.close();
            self.mark_disconnected(failed);
            return Err(Error::Connection(
                "server did not negotiate an ALPN protocol".into(),
            ));
        }
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

fn validate_client_namespace_id(namespace_id: u64) -> Result<()> {
    if namespace_id == 0 {
        return Err(Error::configuration(
            "namespace_id",
            "must be a positive server-assigned namespace ID",
        ));
    }
    Ok(())
}

struct BuilderSettings {
    endpoint: Endpoint,
    trust: ServerTrust,
    identity: Option<ClientIdentity>,
    alpn: AlpnPolicy,
    timeouts: ClientTimeouts,
    retry: RetryPolicy,
    max_in_flight: usize,
    namespace_id: Option<u64>,
    namespace_name: Vec<u8>,
    namespace_policy: NamespacePolicy,
}

impl BuilderSettings {
    fn new(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            trust: ServerTrust::default(),
            identity: None,
            alpn: AlpnPolicy::default(),
            timeouts: ClientTimeouts::default(),
            retry: RetryPolicy::default(),
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
            namespace_id: None,
            namespace_name: Vec::new(),
            namespace_policy: NamespacePolicy::default(),
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
        if self.namespace_id == Some(0) {
            return Err(Error::configuration(
                "namespace_id",
                "must be a positive server-assigned namespace ID",
            ));
        }
        Ok(())
    }

    fn finish(self) -> Result<ConnectionSettings> {
        self.validate()?;
        let tls = make_tls_config(self.trust, self.identity, &self.alpn)?;
        Ok(ConnectionSettings {
            endpoint: self.endpoint,
            tls,
            alpn: self.alpn,
            timeouts: self.timeouts,
            retry: self.retry,
            max_in_flight: self.max_in_flight,
            namespace_id: self.namespace_id,
            namespace_name: self.namespace_name,
            namespace_policy: self.namespace_policy,
        })
    }
}

struct ConnectionSettings {
    endpoint: Endpoint,
    tls: rustls::ClientConfig,
    alpn: AlpnPolicy,
    timeouts: ClientTimeouts,
    retry: RetryPolicy,
    max_in_flight: usize,
    namespace_id: Option<u64>,
    namespace_name: Vec<u8>,
    namespace_policy: NamespacePolicy,
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

            /// Offers protocol versions in descending order and enforces a
            /// minimum negotiated version.
            pub fn alpn_policy(mut self, policy: AlpnPolicy) -> Self {
                self.settings.alpn = policy;
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

            /// Selects the server-assigned namespace ID used by data-plane requests.
            ///
            /// Namespace IDs are opaque positive 64-bit values returned by
            /// `NAMESPACE_OPEN`; this setting does not create or resolve a namespace.
            pub fn namespace_id(mut self, namespace_id: u64) -> Self {
                self.settings.namespace_id = Some(namespace_id);
                self
            }

            /// Selects the namespace name resolved during connection setup.
            ///
            /// The default is the empty namespace name. The wire protocol has no default
            /// namespace; this is only an SDK convenience that performs `NAMESPACE_OPEN` with
            /// `CreateIfMissing` before the first data-plane request.
            pub fn namespace_name(mut self, namespace_name: impl AsRef<[u8]>) -> Self {
                self.settings.namespace_name = namespace_name.as_ref().to_vec();
                self
            }

            /// Supplies the policy used if the configured namespace name is missing.
            pub fn namespace_policy(mut self, policy: NamespacePolicy) -> Self {
                self.settings.namespace_policy = policy;
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

            /// Returns the currently selected server-assigned namespace ID.
            pub fn namespace_id(&self) -> Option<u64> {
                self.0.namespace_id()
            }

            /// Resolves a namespace name and optionally creates it.
            ///
            /// A zero-length name is an ordinary valid namespace name. This method changes the
            /// namespace selected by subsequent data-plane calls on this client.
            pub async fn namespace_open(
                &self,
                name: impl AsRef<[u8]>,
                create_if_missing: bool,
                policy: Option<NamespacePolicy>,
            ) -> Result<NamespaceDescriptor> {
                self.0
                    .open_namespace(name.as_ref().to_vec(), create_if_missing, policy)
                    .await
            }

            /// Resolves a namespace and reports whether the request created it.
            pub async fn namespace_open_with_outcome(
                &self,
                name: impl AsRef<[u8]>,
                create_if_missing: bool,
                policy: Option<NamespacePolicy>,
            ) -> Result<(NamespaceDescriptor, bool)> {
                self.0
                    .open_namespace_with_outcome(name.as_ref().to_vec(), create_if_missing, policy)
                    .await
            }

            /// Replaces a namespace policy using its current revision.
            pub async fn namespace_update_policy(
                &self,
                namespace_id: u64,
                expected_revision: u64,
                policy: NamespacePolicy,
            ) -> Result<NamespaceDescriptor> {
                self.0
                    .update_namespace_policy(namespace_id, expected_revision, policy)
                    .await
            }

            /// Deletes an empty namespace using its current revision.
            pub async fn namespace_delete(
                &self,
                namespace_id: u64,
                expected_revision: u64,
            ) -> Result<()> {
                self.0
                    .delete_namespace(namespace_id, expected_revision)
                    .await
            }

            /// Retrieves exact encoded bytes for a fixed-size item ID.
            pub async fn get(&self, item_id: ItemId) -> Result<GetOutcome<ItemValue>> {
                self.0.get(item_id).await
            }

            /// Retrieves exact encoded bytes in an explicitly supplied namespace.
            pub async fn get_in_namespace(
                &self,
                namespace_id: u64,
                item_id: ItemId,
            ) -> Result<GetOutcome<ItemValue>> {
                self.0.get_in_namespace(namespace_id, item_id).await
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

            /// Stores exact encoded bytes in an explicitly supplied namespace.
            pub async fn set_in_namespace(
                &self,
                namespace_id: u64,
                item_id: ItemId,
                value: ItemValue,
                options: SetOptions,
            ) -> Result<SetOutcome> {
                self.0
                    .set_in_namespace(namespace_id, item_id, value, options)
                    .await
            }

            /// Deletes a fixed-size item ID.
            pub async fn delete(&self, item_id: ItemId) -> Result<DeleteOutcome> {
                self.0.delete(item_id).await
            }

            /// Deletes a fixed-size item ID in an explicitly supplied namespace.
            pub async fn delete_in_namespace(
                &self,
                namespace_id: u64,
                item_id: ItemId,
            ) -> Result<DeleteOutcome> {
                self.0.delete_in_namespace(namespace_id, item_id).await
            }

            /// Returns server statistics as their JSON text.
            pub async fn stats(&self) -> Result<String> {
                self.0.stats().await
            }

            /// Returns statistics for an explicitly supplied namespace.
            pub async fn stats_in_namespace(&self, namespace_id: u64) -> Result<String> {
                self.0.stats_in_namespace(namespace_id).await
            }

            /// Waits until prior mutations satisfy the server durability barrier.
            pub async fn sync(&self) -> Result<()> {
                self.0.sync().await
            }

            /// Waits for the durability barrier for an explicitly supplied namespace.
            pub async fn sync_in_namespace(&self, namespace_id: u64) -> Result<()> {
                self.0.sync_in_namespace(namespace_id).await
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
    let address = resolve_quinn(
        &settings.endpoint,
        deadline.remaining(Operation::DnsResolution)?,
    )
    .await?;
    Core::connect(
        address,
        settings.endpoint.server_name().to_owned(),
        settings.tls,
        settings.alpn,
        settings.timeouts,
        settings.retry,
        settings.max_in_flight,
        settings.namespace_id,
        settings.namespace_name,
        settings.namespace_policy,
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
        settings.alpn,
        settings.timeouts,
        settings.retry,
        settings.max_in_flight,
        settings.namespace_id,
        settings.namespace_name,
        settings.namespace_policy,
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
    alpn: &AlpnPolicy,
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
    config.alpn_protocols = alpn.protocols().to_vec();
    Ok(config)
}

fn expect_status(operation: Operation, status: Status, expected: &[Status]) -> Result<()> {
    if expected.contains(&status) {
        Ok(())
    } else {
        Err(unexpected_status(operation, status))
    }
}

fn validate_stats_payload(payload: &[u8]) -> Result<()> {
    let value: serde_json::Value =
        serde_json::from_slice(payload).map_err(|error| Error::UnexpectedResponse {
            operation: Operation::Stats,
            message: format!("STATS response is not valid JSON: {error}"),
        })?;
    let object = value.as_object().ok_or_else(|| Error::UnexpectedResponse {
        operation: Operation::Stats,
        message: "STATS response must be a JSON object".into(),
    })?;
    if object
        .get("storage")
        .and_then(serde_json::Value::as_str)
        .is_none()
    {
        return Err(Error::UnexpectedResponse {
            operation: Operation::Stats,
            message: "STATS response must contain a string storage member".into(),
        });
    }
    let workers = object
        .get("workers")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Error::UnexpectedResponse {
            operation: Operation::Stats,
            message: "STATS response must contain a workers array".into(),
        })?;
    if workers
        .iter()
        .any(|worker| serde_json::Value::as_str(worker).is_none())
    {
        return Err(Error::UnexpectedResponse {
            operation: Operation::Stats,
            message: "STATS workers entries must be strings".into(),
        });
    }
    Ok(())
}

fn validate_response_contract(
    opcode: Opcode,
    create_if_missing: bool,
    response: &Response,
) -> Result<()> {
    let operation = operation(opcode);
    if response.status.is_error() {
        let applicable = match response.status {
            Status::InvalidRequest
            | Status::TooLarge
            | Status::Overloaded
            | Status::Timeout
            | Status::Forbidden
            | Status::InternalError => true,
            Status::NoCapacity | Status::PolicyConflict => opcode == Opcode::Set,
            Status::Conflict => {
                matches!(opcode, Opcode::NamespaceUpdatePolicy | Opcode::NamespaceDelete)
            }
            Status::NamespaceNotFound => matches!(
                opcode,
                Opcode::Get
                    | Opcode::Set
                    | Opcode::Delete
                    | Opcode::Stats
                    | Opcode::Sync
                    | Opcode::NamespaceOpen
                    | Opcode::NamespaceUpdatePolicy
                    | Opcode::NamespaceDelete
            ),
            Status::NamespaceNotEmpty => opcode == Opcode::NamespaceDelete,
            Status::UnsupportedOpcode => false,
            // `Status::try_from` rejects unassigned values before this helper runs.
            Status::Ok
            | Status::NotFound
            | Status::Created
            | Status::Replaced
            | Status::Deleted
            | Status::NotStored => false,
        };
        if !applicable {
            return Err(Error::UnexpectedResponse {
                operation,
                message: format!(
                    "status {:?} is not applicable to {opcode:?}",
                    response.status
                ),
            });
        }
        return Ok(());
    }

    let invalid_payload = |message: &'static str| {
        Err(Error::UnexpectedResponse {
            operation,
            message: message.into(),
        })
    };
    let descriptor_payload = || {
        NamespaceDescriptor::decode(&response.payload).map(|_| ()).map_err(|error| {
            Error::UnexpectedResponse {
                operation,
                message: format!("namespace descriptor is invalid: {error}"),
            }
        })
    };
    match (opcode, response.status) {
        (Opcode::Ping, Status::Ok) if response.payload == b"PONG" => Ok(()),
        (Opcode::Ping, Status::Ok) => invalid_payload("PING success payload must be PONG"),

        (Opcode::Get, Status::Ok) => Ok(()),
        (Opcode::Get, Status::NotFound) if response.payload.is_empty() => Ok(()),
        (Opcode::Get, Status::NotFound) => {
            invalid_payload("GET NotFound responses must have an empty payload")
        }

        (Opcode::Set, Status::Created | Status::Replaced | Status::NotStored)
            if response.payload.is_empty() =>
        {
            Ok(())
        }
        (Opcode::Set, Status::Created | Status::Replaced | Status::NotStored) => {
            invalid_payload("SET success responses must have an empty payload")
        }

        (Opcode::Delete, Status::Deleted | Status::NotFound)
            if response.payload.is_empty() =>
        {
            Ok(())
        }
        (Opcode::Delete, Status::Deleted | Status::NotFound) => {
            invalid_payload("DELETE domain responses must have an empty payload")
        }

        (Opcode::Stats, Status::Ok) => validate_stats_payload(&response.payload),
        (Opcode::Sync, Status::Ok) if response.payload.is_empty() => Ok(()),
        (Opcode::Sync, Status::Ok) => {
            invalid_payload("SYNC success responses must have an empty payload")
        }

        (Opcode::NamespaceOpen, Status::Ok) => descriptor_payload(),
        (Opcode::NamespaceOpen, Status::Created) if create_if_missing => descriptor_payload(),
        (Opcode::NamespaceUpdatePolicy, Status::Ok) => descriptor_payload(),
        (Opcode::NamespaceDelete, Status::Deleted) if response.payload.is_empty() => Ok(()),
        (Opcode::NamespaceDelete, Status::Deleted) => {
            invalid_payload("NAMESPACE_DELETE success responses must have an empty payload")
        }
        (_, status) => Err(unexpected_status(operation, status)),
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
        Status::NoCapacity => ServerErrorCode::NoCapacity,
        Status::PolicyConflict => ServerErrorCode::PolicyConflict,
        Status::Conflict => ServerErrorCode::Conflict,
        Status::NamespaceNotFound => ServerErrorCode::NamespaceNotFound,
        Status::NamespaceNotEmpty => ServerErrorCode::NamespaceNotEmpty,
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
        Opcode::NamespaceOpen => Operation::NamespaceOpen,
        Opcode::NamespaceUpdatePolicy => Operation::NamespaceUpdatePolicy,
        Opcode::NamespaceDelete => Operation::NamespaceDelete,
    }
}
