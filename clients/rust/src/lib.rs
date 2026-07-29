//! QUIC client for the OpenKache binary protocol.

#[cfg(feature = "ffi")]
pub mod ffi;
mod transport;
pub mod value;
pub mod value_envelope;

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use openkache_protocol::{
    ClientKeyDigest, MAX_RESPONSE_FRAME_BYTES, Opcode, Request, Response, Status,
};
pub use openkache_protocol::{SetCondition, SetOptions};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

#[cfg(not(any(feature = "quic-compio", feature = "quic-quinn")))]
compile_error!("enable at least one client QUIC backend feature");

/// All client-level errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid client configuration: {0}")]
    Configuration(String),
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("operation timed out during {operation}")]
    Timeout { operation: &'static str },
    #[error("{backend} QUIC requires {message}")]
    Runtime {
        backend: &'static str,
        message: String,
    },
    #[error("{backend} QUIC {operation} failed: {message}")]
    Transport {
        backend: &'static str,
        operation: &'static str,
        message: String,
    },
    #[error("server returned {status:?}: {message}")]
    Server { status: Status, message: String },
    #[error("unexpected {operation} response status: {status:?}")]
    UnexpectedStatus {
        operation: &'static str,
        status: Status,
    },
    #[error("unexpected PING response payload")]
    UnexpectedPingPayload,
    #[error("response exceeds protocol limit of {maximum} bytes")]
    ResponseTooLarge { maximum: usize },
    #[error("TLS configuration failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("protocol failed: {0}")]
    Protocol(#[from] openkache_protocol::ProtocolError),
    #[error("response is not valid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("value transformation failed: {0}")]
    Value(#[from] value::Error),
}

/// Convenience alias for client results.
pub type Result<T> = std::result::Result<T, Error>;

/// Result of storing a key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetOutcome {
    Created,
    Replaced,
    NotStored,
}

/// QUIC implementation used by the client connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuicBackend {
    /// Quinn with Tokio packet I/O.
    Quinn,
    /// Quinn protocol state managed through Compio packet I/O.
    Compio,
}

impl QuicBackend {
    /// Returns the stable configuration and diagnostics label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quinn => "quinn",
            Self::Compio => "compio",
        }
    }
}

const COMPILED_QUIC_BACKENDS: &[QuicBackend] = &[
    #[cfg(feature = "quic-quinn")]
    QuicBackend::Quinn,
    #[cfg(feature = "quic-compio")]
    QuicBackend::Compio,
];

/// QUIC backend selection for one client connection.
#[derive(Clone, Copy, Debug, Default)]
pub struct QuicOptions {
    /// Explicit backend selection. Single-backend builds select it automatically.
    pub backend: Option<QuicBackend>,
}

impl QuicOptions {
    fn selected_backend(self) -> Result<QuicBackend> {
        if let Some(backend) = self.backend {
            return Ok(backend);
        }
        if let [backend] = COMPILED_QUIC_BACKENDS {
            return Ok(*backend);
        }
        #[cfg(all(feature = "quic-compio", feature = "quic-quinn"))]
        {
            if compio::runtime::Runtime::try_current().is_some() {
                return Ok(QuicBackend::Compio);
            }
            if tokio::runtime::Handle::try_current().is_ok() {
                return Ok(QuicBackend::Quinn);
            }
        }
        Err(Error::Configuration(
            "quic.backend must be specified when multiple QUIC backends are compiled and no supported runtime is active"
                .into(),
        ))
    }
}

/// Deadlines applied to connection setup and complete request/response exchanges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientTimeouts {
    /// Maximum duration for endpoint initialization and the QUIC/TLS handshake.
    pub connect: Duration,
    /// Maximum duration for lane acquisition, request transmission, and response receipt.
    pub request: Duration,
}

impl Default for ClientTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(5),
            request: Duration::from_secs(2),
        }
    }
}

/// Whole-operation retry policy for response-safe cache operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    /// Maximum total attempts, including the initial request.
    pub max_attempts: usize,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 2 }
    }
}

/// Optional client behaviors layered over the OpenKache wire protocol.
#[derive(Default)]
pub struct ClientOptions {
    /// Compression and end-to-end encryption applied to stored values.
    pub value_codec: value::ValueCodec,
    /// Optional certificate identity presented to an mTLS server.
    pub identity: Option<ClientIdentity>,
    /// QUIC implementation selected for the connection.
    pub quic: QuicOptions,
    /// Bounded connection and request durations.
    pub timeouts: ClientTimeouts,
    /// Retry attempts for response-safe operations after connection failures.
    pub retry: RetryPolicy,
}

/// Certificate chain and private key presented during mutual TLS authentication.
pub struct ClientIdentity {
    certificate_chain: Vec<CertificateDer<'static>>,
    private_key: PrivateKeyDer<'static>,
}

impl ClientIdentity {
    /// Creates a client identity from DER-encoded certificate and private-key material.
    ///
    /// # Arguments
    ///
    /// * `certificate_chain` - Leaf certificate followed by any intermediate certificates.
    /// * `private_key` - Private key corresponding to the leaf certificate.
    ///
    /// # Returns
    ///
    /// An identity that can be placed in [`ClientOptions`].
    pub fn new(
        certificate_chain: Vec<CertificateDer<'static>>,
        private_key: PrivateKeyDer<'static>,
    ) -> Self {
        Self {
            certificate_chain,
            private_key,
        }
    }
}

/// A reusable QUIC connection to an OpenKache server.
pub struct Client {
    connection: RwLock<Arc<transport::Connection>>,
    reconnect: futures_util::lock::Mutex<()>,
    address: SocketAddr,
    server_name: String,
    tls: rustls::ClientConfig,
    backend: QuicBackend,
    connect_timeout: Duration,
    value_codec: value::ValueCodec,
    request_timeout: Duration,
    retry: RetryPolicy,
}

impl Client {
    /// Connects to a server and trusts the supplied DER certificate.
    pub async fn connect(
        address: SocketAddr,
        server_name: &str,
        trusted_certificate_der: &[u8],
    ) -> Result<Self> {
        Self::connect_with_options(
            address,
            server_name,
            trusted_certificate_der,
            ClientOptions::default(),
        )
        .await
    }

    /// Connects to a server with explicit value compression and encryption options.
    ///
    /// # Arguments
    ///
    /// * `address` - Server UDP socket address.
    /// * `server_name` - TLS certificate name expected from the server.
    /// * `trusted_certificate_der` - DER certificate trusted for this connection.
    /// * `options` - Client-side value transformation settings.
    ///
    /// # Returns
    ///
    /// A reusable, connected client.
    ///
    /// # Errors
    ///
    /// Returns an error when TLS configuration or the QUIC handshake fails.
    // Compio connections are thread-affine; Arc provides task-local shared ownership here and
    // preserves Send + Sync for Quinn-only builds.
    #[allow(clippy::arc_with_non_send_sync)]
    pub async fn connect_with_options(
        address: SocketAddr,
        server_name: &str,
        trusted_certificate_der: &[u8],
        options: ClientOptions,
    ) -> Result<Self> {
        let ClientOptions {
            value_codec,
            identity,
            quic,
            timeouts,
            retry,
        } = options;
        if timeouts.connect.is_zero() || timeouts.request.is_zero() {
            return Err(Error::Configuration(
                "client timeouts must be greater than zero".into(),
            ));
        }
        if Instant::now().checked_add(timeouts.connect).is_none()
            || Instant::now().checked_add(timeouts.request).is_none()
        {
            return Err(Error::Configuration(
                "client timeouts exceed the platform clock range".into(),
            ));
        }
        if retry.max_attempts == 0 {
            return Err(Error::Configuration(
                "retry.max_attempts must be greater than zero".into(),
            ));
        }
        let backend = quic.selected_backend()?;
        let tls = make_tls_config(trusted_certificate_der, identity)?;
        let connection =
            transport::connect(backend, address, server_name, tls.clone(), timeouts.connect)
                .await?;
        Ok(Self {
            connection: RwLock::new(Arc::new(connection)),
            reconnect: futures_util::lock::Mutex::new(()),
            address,
            server_name: server_name.to_string(),
            tls,
            backend,
            connect_timeout: timeouts.connect,
            value_codec,
            request_timeout: timeouts.request,
            retry,
        })
    }

    /// Verifies that the server is reachable and speaks protocol v2.
    pub async fn ping(&self) -> Result<()> {
        let response = self
            .request(Request::new(Opcode::Ping, None, Vec::new())?)
            .await?;
        expect_status("PING", response.status, &[Status::Ok])?;
        if response.payload != b"PONG" {
            return Err(Error::UnexpectedPingPayload);
        }
        Ok(())
    }

    /// Retrieves a value, returning `None` when the key does not exist.
    pub async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let client_key_digest = ClientKeyDigest::from_user_key(key);
        let response = self
            .request(Request::new(
                Opcode::Get,
                Some(client_key_digest),
                Vec::new(),
            )?)
            .await?;
        match response.status {
            Status::Ok => Ok(Some(self.value_codec.open(
                client_key_digest,
                response.value_flags,
                response.payload,
            )?)),
            Status::NotFound => Ok(None),
            status => Err(unexpected("GET", status)),
        }
    }

    /// Stores a value and reports whether it created or replaced the key.
    pub async fn set(&self, key: &[u8], value: &[u8]) -> Result<SetOutcome> {
        self.set_owned_with_options(key, value.to_vec(), SetOptions::NONE)
            .await
    }

    /// Stores a value with an optional TTL and atomic existence condition.
    pub async fn set_with_options(
        &self,
        key: &[u8],
        value: &[u8],
        options: SetOptions,
    ) -> Result<SetOutcome> {
        self.set_owned_with_options(key, value.to_vec(), options)
            .await
    }

    /// Stores an owned value while reusing its allocation when practical.
    ///
    /// # Arguments
    ///
    /// * `key` - Exact application key bytes.
    /// * `value` - Owned application value.
    ///
    /// # Returns
    ///
    /// Whether the operation created or replaced the key.
    ///
    /// # Errors
    ///
    /// Returns an error when value transformation, transport, protocol, or server execution fails.
    pub async fn set_owned(&self, key: &[u8], value: Vec<u8>) -> Result<SetOutcome> {
        self.set_owned_with_options(key, value, SetOptions::NONE)
            .await
    }

    /// Stores an owned value with an optional TTL and atomic existence condition.
    ///
    /// # Arguments
    ///
    /// * `key` - Exact application key bytes.
    /// * `value` - Owned application value.
    /// * `options` - Optional TTL and `NX` or `XX` condition.
    ///
    /// # Returns
    ///
    /// Whether the key was created, replaced, or not stored because its condition failed.
    ///
    /// # Errors
    ///
    /// Returns an error when value transformation, transport, protocol, or server execution fails.
    pub async fn set_owned_with_options(
        &self,
        key: &[u8],
        value: Vec<u8>,
        options: SetOptions,
    ) -> Result<SetOutcome> {
        let client_key_digest = ClientKeyDigest::from_user_key(key);
        let sealed = self.value_codec.seal_owned(client_key_digest, value)?;
        let response = self
            .request(Request::new_set(
                Opcode::Set,
                Some(client_key_digest),
                sealed.flags,
                options,
                sealed.bytes,
            )?)
            .await?;
        match response.status {
            Status::Created => Ok(SetOutcome::Created),
            Status::Replaced => Ok(SetOutcome::Replaced),
            Status::NotStored => Ok(SetOutcome::NotStored),
            status => Err(unexpected("SET", status)),
        }
    }

    /// Deletes a key and returns whether it existed.
    pub async fn delete(&self, key: &[u8]) -> Result<bool> {
        let client_key_digest = ClientKeyDigest::from_user_key(key);
        let response = self
            .request(Request::new(
                Opcode::Delete,
                Some(client_key_digest),
                Vec::new(),
            )?)
            .await?;
        match response.status {
            Status::Deleted => Ok(true),
            Status::NotFound => Ok(false),
            status => Err(unexpected("DELETE", status)),
        }
    }

    /// Returns the server's JSON statistics payload.
    pub async fn stats(&self) -> Result<String> {
        let response = self
            .request(Request::new(Opcode::Stats, None, Vec::new())?)
            .await?;
        expect_status("STATS", response.status, &[Status::Ok])?;
        Ok(String::from_utf8(response.payload)?)
    }

    /// Requests a durability barrier.
    pub async fn sync(&self) -> Result<()> {
        let response = self
            .request(Request::new(Opcode::Sync, None, Vec::new())?)
            .await?;
        expect_status("SYNC", response.status, &[Status::Ok])
    }

    async fn request(&self, request: Request) -> Result<Response> {
        let deadline = transport::Deadline::after(self.request_timeout)?;
        let max_attempts = if matches!(request.opcode, Opcode::Ping | Opcode::Get | Opcode::Stats) {
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
                Err(error) if attempt < max_attempts && error.is_connection_failure() => {
                    self.reconnect(&connection, deadline).await?;
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("every configured request has at least one attempt")
    }

    async fn request_once(
        &self,
        connection: &transport::Connection,
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
                        status: response.status,
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

    fn current_connection(&self) -> Result<Arc<transport::Connection>> {
        self.connection
            .read()
            .map(|connection| Arc::clone(&connection))
            .map_err(|_| Error::Connection("connection state lock is poisoned".into()))
    }

    #[allow(clippy::arc_with_non_send_sync)]
    async fn reconnect(
        &self,
        failed: &Arc<transport::Connection>,
        deadline: transport::Deadline,
    ) -> Result<()> {
        let remaining = deadline.remaining("connection retry")?;
        let Some(_guard) =
            transport::timeout(self.backend, remaining, self.reconnect.lock()).await?
        else {
            return Err(Error::Timeout {
                operation: "connection retry",
            });
        };
        let current = self.current_connection()?;
        if !Arc::ptr_eq(&current, failed) {
            return Ok(());
        }
        let timeout = deadline
            .remaining("connection retry")?
            .min(self.connect_timeout);
        let replacement = transport::connect(
            self.backend,
            self.address,
            &self.server_name,
            self.tls.clone(),
            timeout,
        )
        .await?;
        let mut connection = self
            .connection
            .write()
            .map_err(|_| Error::Connection("connection state lock is poisoned".into()))?;
        if Arc::ptr_eq(&connection, failed) {
            *connection = Arc::new(replacement);
        }
        Ok(())
    }
}

impl Error {
    fn is_connection_failure(&self) -> bool {
        matches!(
            self,
            Self::Connection(_) | Self::Timeout { .. } | Self::Transport { .. } | Self::Io(_)
        )
    }
}

fn make_tls_config(
    trusted_certificate_der: &[u8],
    identity: Option<ClientIdentity>,
) -> Result<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(trusted_certificate_der.to_vec()))?;
    let provider = rustls::crypto::ring::default_provider();
    let builder = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(roots);
    let mut config = match identity {
        Some(identity) => {
            builder.with_client_auth_cert(identity.certificate_chain, identity.private_key)?
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
        Err(unexpected(operation, status))
    }
}

fn unexpected(operation: &'static str, status: Status) -> Error {
    Error::UnexpectedStatus { operation, status }
}
