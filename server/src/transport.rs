//! QUIC backend boundary used by the OpenKache protocol server.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
#[cfg(feature = "quic-quiche")]
use std::net::SocketAddr;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use compio::BufResult;
#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
use compio::io::{AsyncReadExt, AsyncWriteExt};
use openkache_protocol::{REQUEST_HEADER_BYTES, Request};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::QuicBackend;

/// Parsed TLS material shared by every reuse-port endpoint.
pub(super) struct ServerTlsConfig {
    pub(super) certificate_chain: Vec<CertificateDer<'static>>,
    pub(super) private_key: PrivateKeyDer<'static>,
    pub(super) client_ca: Vec<CertificateDer<'static>>,
}

/// Backend-independent endpoint selected when the server binds.
pub(super) enum ServerEndpoint {
    #[cfg(feature = "quic-quinn")]
    Quinn(quinn_backend::Endpoint),
    #[cfg(feature = "quic-noq")]
    Noq(noq_backend::Endpoint),
    #[cfg(feature = "quic-quiche")]
    Quiche(quiche_backend::Endpoint),
}

impl ServerEndpoint {
    /// Rejects backend selections that this binary cannot initialize.
    pub(super) fn validate_backend(backend: QuicBackend) -> Result<(), TransportError> {
        match backend {
            QuicBackend::Quinn => {
                #[cfg(feature = "quic-quinn")]
                {
                    Ok(())
                }
                #[cfg(not(feature = "quic-quinn"))]
                {
                    Err(TransportError::not_compiled(backend, "quic-quinn"))
                }
            }
            QuicBackend::Noq => {
                #[cfg(feature = "quic-noq")]
                {
                    Ok(())
                }
                #[cfg(not(feature = "quic-noq"))]
                {
                    Err(TransportError::not_compiled(backend, "quic-noq"))
                }
            }
            QuicBackend::Quiche => {
                #[cfg(feature = "quic-quiche")]
                {
                    Ok(())
                }
                #[cfg(not(feature = "quic-quiche"))]
                {
                    Err(TransportError::not_compiled(backend, "quic-quiche"))
                }
            }
            QuicBackend::Neqo => Err(TransportError::unavailable(
                backend,
                "the official neqo transport is not published as a standalone crate and requires NSS certificate-database integration",
            )),
        }
    }

    /// Binds the selected implementation to a Compio UDP socket.
    pub(super) async fn bind(
        backend: QuicBackend,
        socket: std::net::UdpSocket,
        tls: Arc<ServerTlsConfig>,
        max_concurrent_streams: usize,
    ) -> Result<Self, TransportError> {
        Self::validate_backend(backend)?;
        match backend {
            QuicBackend::Quinn => {
                #[cfg(feature = "quic-quinn")]
                {
                    Ok(Self::Quinn(
                        quinn_backend::Endpoint::bind(socket, tls, max_concurrent_streams).await?,
                    ))
                }
                #[cfg(not(feature = "quic-quinn"))]
                {
                    Err(TransportError::not_compiled(backend, "quic-quinn"))
                }
            }
            QuicBackend::Noq => {
                #[cfg(feature = "quic-noq")]
                {
                    Ok(Self::Noq(
                        noq_backend::Endpoint::bind(socket, tls, max_concurrent_streams).await?,
                    ))
                }
                #[cfg(not(feature = "quic-noq"))]
                {
                    Err(TransportError::not_compiled(backend, "quic-noq"))
                }
            }
            QuicBackend::Quiche => {
                #[cfg(feature = "quic-quiche")]
                {
                    Ok(Self::Quiche(
                        quiche_backend::Endpoint::bind(socket, tls, max_concurrent_streams).await?,
                    ))
                }
                #[cfg(not(feature = "quic-quiche"))]
                {
                    Err(TransportError::not_compiled(backend, "quic-quiche"))
                }
            }
            QuicBackend::Neqo => Err(TransportError::unavailable(
                backend,
                "the official neqo transport is not published as a standalone crate and requires NSS certificate-database integration",
            )),
        }
    }
}

/// Endpoint behavior needed by the generic connection serving loop.
pub(super) trait Endpoint {
    type Incoming: Incoming;

    fn wait_incoming(&self) -> impl Future<Output = Option<Self::Incoming>>;

    fn close(&self, reason: &[u8]);

    fn shutdown(self) -> impl Future<Output = Result<(), TransportError>>;
}

/// A QUIC handshake accepted by an endpoint.
pub(super) trait Incoming {
    type Connection: Connection;

    fn connect(self) -> impl Future<Output = Result<Self::Connection, TransportError>>;
}

/// A connected peer capable of accepting bidirectional streams.
pub(super) trait Connection {
    type SendStream: SendStream;
    type ReceiveStream: ReceiveStream;

    /// Returns the authenticated peer's leaf certificate, when client authentication is enabled.
    fn peer_certificate(&self) -> Option<CertificateDer<'static>>;

    fn accept_bi(
        &self,
    ) -> impl Future<Output = Result<(Self::SendStream, Self::ReceiveStream), TransportError>>;
}

/// Receive half of one request stream.
pub(super) trait ReceiveStream {
    fn read_request(
        &mut self,
        maximum: usize,
        timeout: Duration,
        budget: &RequestBudget,
    ) -> impl Future<Output = Result<RequestFrame, StreamReadError>>;
}

/// Send half of one response stream.
pub(super) trait SendStream {
    fn write_response(
        &mut self,
        frame: Vec<u8>,
        timeout: Duration,
    ) -> impl Future<Output = Result<(), TransportError>>;
}

/// Failure while receiving a request frame.
#[derive(Debug, thiserror::Error)]
pub(super) enum StreamReadError {
    #[error("request read timed out")]
    Timeout,
    #[error("request exceeds the protocol limit")]
    TooLarge,
    #[error(transparent)]
    Protocol(#[from] openkache_protocol::ProtocolError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

/// Request bytes paired with the worker-local memory-budget reservation they consume.
pub(super) struct RequestFrame {
    pub(super) bytes: Vec<u8>,
    _permit: RequestBudgetPermit,
}

impl RequestFrame {
    fn new(bytes: Vec<u8>, permit: RequestBudgetPermit) -> Self {
        Self {
            bytes,
            _permit: permit,
        }
    }
}

/// Byte-weighted memory budget shared by every connection on one network worker.
#[derive(Clone)]
pub(super) struct RequestBudget {
    inner: Rc<RefCell<RequestBudgetState>>,
}

struct RequestBudgetState {
    capacity: usize,
    used: usize,
    next_waiter_id: u64,
    waiters: HashMap<u64, Waker>,
}

pub(super) struct RequestBudgetPermit {
    inner: Rc<RefCell<RequestBudgetState>>,
    bytes: usize,
}

struct RequestBudgetAcquire {
    inner: Rc<RefCell<RequestBudgetState>>,
    bytes: usize,
    waiter_id: Option<u64>,
}

impl RequestBudget {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            inner: Rc::new(RefCell::new(RequestBudgetState {
                capacity,
                used: 0,
                next_waiter_id: 0,
                waiters: HashMap::new(),
            })),
        }
    }

    pub(super) async fn acquire(
        &self,
        bytes: usize,
        timeout: Duration,
    ) -> Result<RequestBudgetPermit, StreamReadError> {
        if bytes == 0 {
            return Ok(RequestBudgetPermit {
                inner: Rc::clone(&self.inner),
                bytes: 0,
            });
        }
        if bytes > self.inner.borrow().capacity {
            return Err(StreamReadError::TooLarge);
        }
        compio::runtime::time::timeout(
            timeout,
            RequestBudgetAcquire {
                inner: Rc::clone(&self.inner),
                bytes,
                waiter_id: None,
            },
        )
        .await
        .map_err(|_| StreamReadError::Timeout)
    }
}

impl Future for RequestBudgetAcquire {
    type Output = RequestBudgetPermit;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = Rc::clone(&self.inner);
        let bytes = self.bytes;
        let mut state = inner.borrow_mut();
        if state.used <= state.capacity - bytes {
            if let Some(waiter_id) = self.waiter_id.take() {
                state.waiters.remove(&waiter_id);
            }
            state.used += bytes;
            return Poll::Ready(RequestBudgetPermit {
                inner: Rc::clone(&inner),
                bytes,
            });
        }

        if let Some(waiter_id) = self.waiter_id
            && let Some(waiter) = state.waiters.get_mut(&waiter_id)
        {
            if !waiter.will_wake(context.waker()) {
                waiter.clone_from(context.waker());
            }
            return Poll::Pending;
        }

        let waiter_id = state.next_waiter_id;
        state.next_waiter_id = state
            .next_waiter_id
            .checked_add(1)
            .expect("request budget waiter identifier overflowed");
        state.waiters.insert(waiter_id, context.waker().clone());
        drop(state);
        self.waiter_id = Some(waiter_id);
        Poll::Pending
    }
}

impl Drop for RequestBudgetAcquire {
    fn drop(&mut self) {
        if let Some(waiter_id) = self.waiter_id {
            self.inner.borrow_mut().waiters.remove(&waiter_id);
        }
    }
}

impl Drop for RequestBudgetPermit {
    fn drop(&mut self) {
        if self.bytes == 0 {
            return;
        }
        let waiters = {
            let mut state = self.inner.borrow_mut();
            state.used = state
                .used
                .checked_sub(self.bytes)
                .expect("released request bytes must be reserved");
            std::mem::take(&mut state.waiters)
        };
        for waiter in waiters.into_values() {
            waiter.wake();
        }
    }
}

/// Stable transport failure with backend and operation context.
#[derive(Debug, thiserror::Error)]
#[error("{backend} QUIC {operation} failed: {message}")]
pub struct TransportError {
    backend: &'static str,
    operation: &'static str,
    message: String,
}

impl TransportError {
    fn backend(
        backend: &'static str,
        operation: &'static str,
        error: impl std::fmt::Display,
    ) -> Self {
        Self {
            backend,
            operation,
            message: error.to_string(),
        }
    }

    #[cfg(any(
        not(feature = "quic-quinn"),
        not(feature = "quic-noq"),
        not(feature = "quic-quiche")
    ))]
    fn not_compiled(backend: QuicBackend, feature: &'static str) -> Self {
        Self {
            backend: backend.as_str(),
            operation: "selection",
            message: format!("backend was not compiled; enable Cargo feature `{feature}`"),
        }
    }

    fn unavailable(backend: QuicBackend, reason: &'static str) -> Self {
        Self {
            backend: backend.as_str(),
            operation: "selection",
            message: format!("backend is unavailable: {reason}"),
        }
    }
}

#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
fn tls_config(material: &ServerTlsConfig) -> Result<rustls::ServerConfig, rustls::Error> {
    let builder = rustls::ServerConfig::builder();
    let mut tls = if material.client_ca.is_empty() {
        builder.with_no_client_auth()
    } else {
        let mut roots = rustls::RootCertStore::empty();
        for certificate in &material.client_ca {
            roots.add(certificate.clone())?;
        }
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|error| rustls::Error::General(error.to_string()))?;
        builder.with_client_cert_verifier(verifier)
    }
    .with_single_cert(
        material.certificate_chain.clone(),
        material.private_key.clone_key(),
    )?;
    tls.alpn_protocols = vec![openkache_protocol::ALPN.to_vec()];
    Ok(tls)
}

#[cfg(feature = "quic-quinn")]
mod quinn_backend {
    use super::*;

    const NAME: &str = "quinn";

    pub(crate) struct Endpoint(compio_quic::Endpoint);

    impl Endpoint {
        pub(super) async fn bind(
            socket: std::net::UdpSocket,
            material: Arc<ServerTlsConfig>,
            max_concurrent_streams: usize,
        ) -> Result<Self, TransportError> {
            let tls = tls_config(&material)
                .map_err(|error| TransportError::backend(NAME, "TLS configuration", error))?;
            let crypto = compio_quic::crypto::rustls::QuicServerConfig::try_from(tls)
                .map_err(|error| TransportError::backend(NAME, "TLS initialization", error))?;
            let socket = compio::net::UdpSocket::from_std(socket)
                .map_err(|error| TransportError::backend(NAME, "socket initialization", error))?;
            let max_concurrent_streams =
                u32::try_from(max_concurrent_streams).map_err(|error| {
                    TransportError::backend(NAME, "stream limit configuration", error)
                })?;
            let mut transport = compio_quic::TransportConfig::default();
            transport
                .max_concurrent_bidi_streams(compio_quic::VarInt::from_u32(max_concurrent_streams))
                .max_concurrent_uni_streams(compio_quic::VarInt::from_u32(0));
            let mut server_config = compio_quic::ServerConfig::with_crypto(Arc::new(crypto));
            server_config.transport_config(Arc::new(transport));
            let endpoint = compio_quic::Endpoint::new(
                socket,
                compio_quic::EndpointConfig::default(),
                Some(server_config),
                None,
            )
            .map_err(|error| TransportError::backend(NAME, "endpoint initialization", error))?;
            Ok(Self(endpoint))
        }
    }

    impl super::Endpoint for Endpoint {
        type Incoming = Incoming;

        async fn wait_incoming(&self) -> Option<Self::Incoming> {
            self.0.wait_incoming().await.map(Incoming)
        }

        fn close(&self, reason: &[u8]) {
            self.0.close(compio_quic::VarInt::from_u32(0), reason);
        }

        async fn shutdown(self) -> Result<(), TransportError> {
            self.0
                .shutdown()
                .await
                .map_err(|error| TransportError::backend(NAME, "shutdown", error))
        }
    }

    pub(crate) struct Incoming(compio_quic::Incoming);

    impl super::Incoming for Incoming {
        type Connection = Connection;

        async fn connect(self) -> Result<Self::Connection, TransportError> {
            self.0
                .await
                .map(Connection)
                .map_err(|error| TransportError::backend(NAME, "handshake", error))
        }
    }

    pub(crate) struct Connection(compio_quic::Connection);

    impl super::Connection for Connection {
        type SendStream = SendStream;
        type ReceiveStream = ReceiveStream;

        fn peer_certificate(&self) -> Option<CertificateDer<'static>> {
            self.0
                .peer_identity()
                .and_then(|certificates| certificates.first().cloned())
        }

        async fn accept_bi(
            &self,
        ) -> Result<(Self::SendStream, Self::ReceiveStream), TransportError> {
            self.0
                .accept_bi()
                .await
                .map(|(send, receive)| (SendStream(send), ReceiveStream(receive)))
                .map_err(|error| TransportError::backend(NAME, "stream accept", error))
        }
    }

    pub(crate) struct ReceiveStream(compio_quic::RecvStream);

    impl super::ReceiveStream for ReceiveStream {
        async fn read_request(
            &mut self,
            maximum: usize,
            timeout: Duration,
            budget: &RequestBudget,
        ) -> Result<RequestFrame, StreamReadError> {
            let BufResult(result, mut frame) = self.0.read_exact(Vec::with_capacity(1)).await;
            result.map_err(|error| TransportError::backend(NAME, "stream header read", error))?;
            let BufResult(result, header) = compio::runtime::time::timeout(
                timeout,
                self.0
                    .read_exact(Vec::with_capacity(REQUEST_HEADER_BYTES - 1)),
            )
            .await
            .map_err(|_| StreamReadError::Timeout)?;
            result.map_err(|error| TransportError::backend(NAME, "stream header read", error))?;
            frame.extend_from_slice(&header);
            let frame_len = Request::frame_len_from_header(&frame)?;
            let value_len = Request::value_len_from_header(&frame)?;
            if frame_len > maximum {
                return Err(StreamReadError::TooLarge);
            }
            let body_len = frame_len - REQUEST_HEADER_BYTES;
            let permit = budget.acquire(value_len, timeout).await?;
            if body_len == 0 {
                return Ok(RequestFrame::new(frame, permit));
            }
            let BufResult(result, body) = compio::runtime::time::timeout(
                timeout,
                self.0.read_exact(Vec::with_capacity(body_len)),
            )
            .await
            .map_err(|_| StreamReadError::Timeout)?;
            result.map_err(|error| TransportError::backend(NAME, "stream body read", error))?;
            frame.extend_from_slice(&body);
            Ok(RequestFrame::new(frame, permit))
        }
    }

    pub(crate) struct SendStream(compio_quic::SendStream);

    impl super::SendStream for SendStream {
        async fn write_response(
            &mut self,
            frame: Vec<u8>,
            timeout: Duration,
        ) -> Result<(), TransportError> {
            let BufResult(result, _) =
                compio::runtime::time::timeout(timeout, self.0.write_all(frame))
                    .await
                    .map_err(|error| {
                        TransportError::backend(NAME, "stream write timeout", error)
                    })?;
            result.map_err(|error| TransportError::backend(NAME, "stream write", error))?;
            Ok(())
        }
    }
}

#[cfg(feature = "quic-noq")]
mod noq_backend {
    use super::*;

    const NAME: &str = "noq";

    pub(crate) struct Endpoint(comnoq::Endpoint);

    impl Endpoint {
        pub(super) async fn bind(
            socket: std::net::UdpSocket,
            material: Arc<ServerTlsConfig>,
            max_concurrent_streams: usize,
        ) -> Result<Self, TransportError> {
            let tls = tls_config(&material)
                .map_err(|error| TransportError::backend(NAME, "TLS configuration", error))?;
            let crypto = comnoq::crypto::rustls::QuicServerConfig::try_from(tls)
                .map_err(|error| TransportError::backend(NAME, "TLS initialization", error))?;
            let socket = compio::net::UdpSocket::from_std(socket)
                .map_err(|error| TransportError::backend(NAME, "socket initialization", error))?;
            let max_concurrent_streams =
                u32::try_from(max_concurrent_streams).map_err(|error| {
                    TransportError::backend(NAME, "stream limit configuration", error)
                })?;
            let mut transport = comnoq::TransportConfig::default();
            transport
                .max_concurrent_bidi_streams(comnoq::VarInt::from_u32(max_concurrent_streams))
                .max_concurrent_uni_streams(comnoq::VarInt::from_u32(0));
            let mut server_config = comnoq::ServerConfig::with_crypto(Arc::new(crypto));
            server_config.transport_config(Arc::new(transport));
            let endpoint = comnoq::Endpoint::new(
                socket,
                comnoq::EndpointConfig::default(),
                Some(server_config),
                None,
            )
            .map_err(|error| TransportError::backend(NAME, "endpoint initialization", error))?;
            Ok(Self(endpoint))
        }
    }

    impl super::Endpoint for Endpoint {
        type Incoming = Incoming;

        async fn wait_incoming(&self) -> Option<Self::Incoming> {
            self.0.accept().await.map(Incoming)
        }

        fn close(&self, reason: &[u8]) {
            self.0.close(comnoq::VarInt::from_u32(0), reason);
        }

        async fn shutdown(self) -> Result<(), TransportError> {
            self.0
                .shutdown()
                .await
                .map_err(|error| TransportError::backend(NAME, "shutdown", error))
        }
    }

    pub(crate) struct Incoming(comnoq::Incoming);

    impl super::Incoming for Incoming {
        type Connection = Connection;

        async fn connect(self) -> Result<Self::Connection, TransportError> {
            self.0
                .await
                .map(Connection)
                .map_err(|error| TransportError::backend(NAME, "handshake", error))
        }
    }

    pub(crate) struct Connection(comnoq::Connection);

    impl super::Connection for Connection {
        type SendStream = SendStream;
        type ReceiveStream = ReceiveStream;

        fn peer_certificate(&self) -> Option<CertificateDer<'static>> {
            self.0
                .peer_identity()
                .and_then(|certificates| certificates.first().cloned())
        }

        async fn accept_bi(
            &self,
        ) -> Result<(Self::SendStream, Self::ReceiveStream), TransportError> {
            self.0
                .accept_bi()
                .await
                .map(|(send, receive)| (SendStream(send), ReceiveStream(receive)))
                .map_err(|error| TransportError::backend(NAME, "stream accept", error))
        }
    }

    pub(crate) struct ReceiveStream(comnoq::RecvStream);

    impl super::ReceiveStream for ReceiveStream {
        async fn read_request(
            &mut self,
            maximum: usize,
            timeout: Duration,
            budget: &RequestBudget,
        ) -> Result<RequestFrame, StreamReadError> {
            let BufResult(result, mut frame) = self.0.read_exact(Vec::with_capacity(1)).await;
            result.map_err(|error| TransportError::backend(NAME, "stream header read", error))?;
            let BufResult(result, header) = compio::runtime::time::timeout(
                timeout,
                self.0
                    .read_exact(Vec::with_capacity(REQUEST_HEADER_BYTES - 1)),
            )
            .await
            .map_err(|_| StreamReadError::Timeout)?;
            result.map_err(|error| TransportError::backend(NAME, "stream header read", error))?;
            frame.extend_from_slice(&header);
            let frame_len = Request::frame_len_from_header(&frame)?;
            let value_len = Request::value_len_from_header(&frame)?;
            if frame_len > maximum {
                return Err(StreamReadError::TooLarge);
            }
            let body_len = frame_len - REQUEST_HEADER_BYTES;
            let permit = budget.acquire(value_len, timeout).await?;
            if body_len == 0 {
                return Ok(RequestFrame::new(frame, permit));
            }
            let BufResult(result, body) = compio::runtime::time::timeout(
                timeout,
                self.0.read_exact(Vec::with_capacity(body_len)),
            )
            .await
            .map_err(|_| StreamReadError::Timeout)?;
            result.map_err(|error| TransportError::backend(NAME, "stream body read", error))?;
            frame.extend_from_slice(&body);
            Ok(RequestFrame::new(frame, permit))
        }
    }

    pub(crate) struct SendStream(comnoq::SendStream);

    impl super::SendStream for SendStream {
        async fn write_response(
            &mut self,
            frame: Vec<u8>,
            timeout: Duration,
        ) -> Result<(), TransportError> {
            let BufResult(result, _) =
                compio::runtime::time::timeout(timeout, self.0.write_all(frame))
                    .await
                    .map_err(|error| {
                        TransportError::backend(NAME, "stream write timeout", error)
                    })?;
            result.map_err(|error| TransportError::backend(NAME, "stream write", error))?;
            Ok(())
        }
    }
}

#[cfg(feature = "quic-quiche")]
mod quiche_backend {
    use std::collections::HashMap;

    use boring::pkey::PKey;
    use boring::ssl::{SslContextBuilder, SslMethod, SslVerifyMode};
    use boring::x509::X509;
    use boring::x509::store::X509StoreBuilder;
    use compio::runtime::JoinHandle;
    use futures_util::{FutureExt, StreamExt, pin_mut, select};

    use super::*;
    use crate::channel::{self, AsyncReceiver, Sender, TrySendError};

    const NAME: &str = "quiche";
    const MAX_DATAGRAM_BYTES: usize = 65_535;
    const MAX_BUFFERED_REQUEST_BYTES: usize = openkache_protocol::MAX_REQUEST_FRAME_BYTES + 1;
    const REQUEST_CANCELLED_ERROR_CODE: u64 = 0;
    const STREAM_CHUNK_BYTES: usize = 16 * 1024;
    const STREAM_CHUNK_BACKLOG: usize = 1;

    pub(crate) struct Endpoint {
        incoming: AsyncReceiver<Incoming>,
        commands: Sender<Command>,
        driver: JoinHandle<Result<(), TransportError>>,
    }

    impl Endpoint {
        pub(super) async fn bind(
            socket: std::net::UdpSocket,
            material: Arc<ServerTlsConfig>,
            max_concurrent_streams: usize,
        ) -> Result<Self, TransportError> {
            let socket = compio::net::UdpSocket::from_std(socket)
                .map_err(|error| TransportError::backend(NAME, "socket initialization", error))?;
            let local_address = socket
                .local_addr()
                .map_err(|error| TransportError::backend(NAME, "local address", error))?;
            let config = config(&material, max_concurrent_streams)?;
            let (incoming_sender, incoming) = channel::unbounded_async();
            let (commands, command_receiver) = channel::unbounded_async();
            let driver_commands = commands.clone();
            let driver = compio::runtime::spawn(async move {
                Driver {
                    socket,
                    local_address,
                    config,
                    incoming: incoming_sender,
                    command_sender: driver_commands,
                    commands: Some(command_receiver),
                    routes: HashMap::new(),
                    clients: HashMap::new(),
                }
                .run()
                .await
            });
            Ok(Self {
                incoming,
                commands,
                driver,
            })
        }
    }

    impl super::Endpoint for Endpoint {
        type Incoming = Incoming;

        async fn wait_incoming(&self) -> Option<Self::Incoming> {
            self.incoming.recv_async().await.ok()
        }

        fn close(&self, reason: &[u8]) {
            let _ = self.commands.try_send(Command::Close(reason.to_vec()));
        }

        async fn shutdown(self) -> Result<(), TransportError> {
            let _ = self.commands.send_async(Command::Shutdown).await;
            match self.driver.await {
                Ok(result) => result,
                Err(error) => Err(TransportError::backend(
                    NAME,
                    "driver task",
                    format!("{error:?}"),
                )),
            }
        }
    }

    pub(crate) struct Incoming(Connection);

    impl super::Incoming for Incoming {
        type Connection = Connection;

        async fn connect(self) -> Result<Self::Connection, TransportError> {
            Ok(self.0)
        }
    }

    pub(crate) struct Connection {
        connection_id: Vec<u8>,
        peer_certificate: Option<CertificateDer<'static>>,
        streams: AsyncReceiver<Stream>,
        commands: Sender<Command>,
    }

    impl super::Connection for Connection {
        type SendStream = SendStream;
        type ReceiveStream = ReceiveStream;

        fn peer_certificate(&self) -> Option<CertificateDer<'static>> {
            self.peer_certificate.clone()
        }

        async fn accept_bi(
            &self,
        ) -> Result<(Self::SendStream, Self::ReceiveStream), TransportError> {
            let stream = self
                .streams
                .recv_async()
                .await
                .map_err(|error| TransportError::backend(NAME, "stream accept", error))?;
            Ok((
                SendStream {
                    connection_id: self.connection_id.clone(),
                    stream_id: stream.stream_id,
                    commands: self.commands.clone(),
                },
                ReceiveStream {
                    connection_id: self.connection_id.clone(),
                    stream_id: stream.stream_id,
                    commands: self.commands.clone(),
                    chunks: stream.chunks,
                    buffered: Vec::new(),
                },
            ))
        }
    }

    struct Stream {
        stream_id: u64,
        chunks: AsyncReceiver<Vec<u8>>,
    }

    pub(crate) struct ReceiveStream {
        connection_id: Vec<u8>,
        stream_id: u64,
        commands: Sender<Command>,
        chunks: AsyncReceiver<Vec<u8>>,
        buffered: Vec<u8>,
    }

    impl ReceiveStream {
        async fn next_chunk(&self, operation: &'static str) -> Result<Vec<u8>, TransportError> {
            let chunk = self
                .chunks
                .recv_async()
                .await
                .map_err(|error| TransportError::backend(NAME, operation, error))?;
            let _ = self.commands.try_send(Command::ResumeRequest {
                connection_id: self.connection_id.clone(),
                stream_id: self.stream_id,
            });
            Ok(chunk)
        }
    }

    impl super::ReceiveStream for ReceiveStream {
        async fn read_request(
            &mut self,
            maximum: usize,
            timeout: Duration,
            budget: &RequestBudget,
        ) -> Result<RequestFrame, StreamReadError> {
            while self.buffered.is_empty() {
                let chunk = self.next_chunk("stream header read").await?;
                self.buffered.extend_from_slice(&chunk);
            }
            compio::runtime::time::timeout(timeout, async {
                while self.buffered.len() < REQUEST_HEADER_BYTES {
                    let chunk = self.next_chunk("stream header read").await?;
                    self.buffered.extend_from_slice(&chunk);
                }
                Ok::<(), TransportError>(())
            })
            .await
            .map_err(|_| StreamReadError::Timeout)?
            .map_err(StreamReadError::Transport)?;
            let frame_len = Request::frame_len_from_header(&self.buffered[..REQUEST_HEADER_BYTES])?;
            let value_len = Request::value_len_from_header(&self.buffered[..REQUEST_HEADER_BYTES])?;
            if frame_len > maximum {
                return Err(StreamReadError::TooLarge);
            }
            let permit = budget.acquire(value_len, timeout).await?;
            compio::runtime::time::timeout(timeout, async {
                while self.buffered.len() < frame_len {
                    let chunk = self.next_chunk("stream body read").await?;
                    self.buffered.extend_from_slice(&chunk);
                }
                Ok::<(), TransportError>(())
            })
            .await
            .map_err(|_| StreamReadError::Timeout)?
            .map_err(StreamReadError::Transport)?;
            Ok(RequestFrame::new(
                self.buffered.drain(..frame_len).collect(),
                permit,
            ))
        }
    }

    impl Drop for ReceiveStream {
        fn drop(&mut self) {
            let _ = self.commands.try_send(Command::CancelRequest {
                connection_id: self.connection_id.clone(),
                stream_id: self.stream_id,
            });
        }
    }

    pub(crate) struct SendStream {
        connection_id: Vec<u8>,
        stream_id: u64,
        commands: Sender<Command>,
    }

    impl super::SendStream for SendStream {
        async fn write_response(
            &mut self,
            frame: Vec<u8>,
            timeout: Duration,
        ) -> Result<(), TransportError> {
            let (reply, response) = channel::bounded_sync_async(1);
            self.commands
                .send_async(Command::SendResponse {
                    connection_id: self.connection_id.clone(),
                    stream_id: self.stream_id,
                    frame,
                    reply,
                })
                .await
                .map_err(|error| TransportError::backend(NAME, "stream write", error))?;
            let result = compio::runtime::time::timeout(timeout, response.recv_async())
                .await
                .map_err(|error| TransportError::backend(NAME, "stream write timeout", error))?
                .map_err(|error| TransportError::backend(NAME, "stream write", error))?;
            result.map_err(|message| TransportError::backend(NAME, "stream write", message))
        }
    }

    enum Command {
        Close(Vec<u8>),
        CancelRequest {
            connection_id: Vec<u8>,
            stream_id: u64,
        },
        ResumeRequest {
            connection_id: Vec<u8>,
            stream_id: u64,
        },
        SendResponse {
            connection_id: Vec<u8>,
            stream_id: u64,
            frame: Vec<u8>,
            reply: Sender<Result<(), String>>,
        },
        Shutdown,
    }

    struct PendingResponse {
        frame: Vec<u8>,
        written: usize,
        reply: Sender<Result<(), String>>,
    }

    struct RequestChunks {
        chunks: Sender<Vec<u8>>,
        pending: Option<Vec<u8>>,
        finished: bool,
    }

    struct Client {
        connection: quiche::Connection,
        announced: bool,
        streams: Option<Sender<Stream>>,
        requests: HashMap<u64, RequestChunks>,
        responses: HashMap<u64, PendingResponse>,
    }

    struct Driver {
        socket: compio::net::UdpSocket,
        local_address: SocketAddr,
        config: quiche::Config,
        incoming: Sender<Incoming>,
        command_sender: Sender<Command>,
        commands: Option<AsyncReceiver<Command>>,
        routes: HashMap<Vec<u8>, Vec<u8>>,
        clients: HashMap<Vec<u8>, Client>,
    }

    impl Driver {
        async fn run(mut self) -> Result<(), TransportError> {
            let receive_socket = self.socket.clone();
            let mut packets = Box::pin(receive_socket.recv_from_multi());
            let commands = self
                .commands
                .take()
                .expect("QUIC driver command receiver is present");
            loop {
                let packet = packets.next().fuse();
                let command = commands.recv_async().fuse();
                let timeout = compio::runtime::time::sleep(
                    self.next_timeout()
                        .unwrap_or_else(|| Duration::from_secs(86_400)),
                )
                .fuse();
                pin_mut!(packet, command, timeout);

                let mut shutting_down = false;
                select! {
                    packet = packet => self.receive_packet(packet)?,
                    command = command => {
                        let Ok(command) = command else {
                            break;
                        };
                        shutting_down = self.handle_command(command);
                    }
                    () = timeout => {
                        for client in self.clients.values_mut() {
                            client.connection.on_timeout();
                        }
                    }
                }

                self.process_connections();
                self.flush_packets().await?;
                self.collect_closed();
                if shutting_down {
                    break;
                }
            }
            Ok(())
        }

        fn next_timeout(&self) -> Option<Duration> {
            self.clients
                .values()
                .filter_map(|client| client.connection.timeout())
                .min()
        }

        fn receive_packet(
            &mut self,
            packet: Option<std::io::Result<compio::driver::op::RecvFromMultiResult>>,
        ) -> Result<(), TransportError> {
            let packet = packet
                .ok_or_else(|| {
                    TransportError::backend(NAME, "receive", "UDP receive stream ended")
                })?
                .map_err(|error| TransportError::backend(NAME, "receive", error))?;
            let Some(peer_address) = packet.addr().and_then(|address| address.as_socket()) else {
                return Ok(());
            };
            let mut datagram = packet.data().to_vec();
            let header = match quiche::Header::from_slice(&mut datagram, quiche::MAX_CONN_ID_LEN) {
                Ok(header) => header,
                Err(_) => return Ok(()),
            };
            let destination_id = header.dcid.as_ref().to_vec();
            let connection_id = match self.routes.get(&destination_id) {
                Some(connection_id) => connection_id.clone(),
                None => {
                    if header.ty != quiche::Type::Initial
                        || !quiche::version_is_supported(header.version)
                    {
                        return Ok(());
                    }
                    self.accept_connection(destination_id, peer_address)?
                }
            };
            let Some(client) = self.clients.get_mut(&connection_id) else {
                return Ok(());
            };
            let receive_info = quiche::RecvInfo {
                from: peer_address,
                to: self.local_address,
            };
            if client.connection.recv(&mut datagram, receive_info).is_err() {
                return Ok(());
            }
            Ok(())
        }

        fn accept_connection(
            &mut self,
            original_destination_id: Vec<u8>,
            peer_address: SocketAddr,
        ) -> Result<Vec<u8>, TransportError> {
            let connection_id = rand::random::<[u8; quiche::MAX_CONN_ID_LEN]>().to_vec();
            let source_id = quiche::ConnectionId::from_ref(&connection_id);
            let connection = quiche::accept(
                &source_id,
                None,
                self.local_address,
                peer_address,
                &mut self.config,
            )
            .map_err(|error| TransportError::backend(NAME, "connection accept", error))?;
            self.routes
                .insert(original_destination_id, connection_id.clone());
            self.routes
                .insert(connection_id.clone(), connection_id.clone());
            self.clients.insert(
                connection_id.clone(),
                Client {
                    connection,
                    announced: false,
                    streams: None,
                    requests: HashMap::new(),
                    responses: HashMap::new(),
                },
            );
            Ok(connection_id)
        }

        fn handle_command(&mut self, command: Command) -> bool {
            match command {
                Command::Close(reason) => {
                    for client in self.clients.values_mut() {
                        let _ = client.connection.close(true, 0, &reason);
                    }
                    false
                }
                Command::CancelRequest {
                    connection_id,
                    stream_id,
                } => {
                    if let Some(client) = self.clients.get_mut(&connection_id) {
                        client.requests.remove(&stream_id);
                        let _ = client.connection.stream_shutdown(
                            stream_id,
                            quiche::Shutdown::Read,
                            REQUEST_CANCELLED_ERROR_CODE,
                        );
                    }
                    false
                }
                Command::ResumeRequest {
                    connection_id,
                    stream_id,
                } => {
                    if let Some(client) = self.clients.get_mut(&connection_id) {
                        receive_stream(client, stream_id);
                    }
                    false
                }
                Command::SendResponse {
                    connection_id,
                    stream_id,
                    frame,
                    reply,
                } => {
                    let Some(client) = self.clients.get_mut(&connection_id) else {
                        let _ = reply.try_send(Err("connection is closed".to_string()));
                        return false;
                    };
                    client.responses.insert(
                        stream_id,
                        PendingResponse {
                            frame,
                            written: 0,
                            reply,
                        },
                    );
                    progress_response(client, stream_id);
                    false
                }
                Command::Shutdown => {
                    for client in self.clients.values_mut() {
                        let _ = client.connection.close(true, 0, b"server shutting down");
                    }
                    true
                }
            }
        }

        fn process_connections(&mut self) {
            let connection_ids: Vec<_> = self.clients.keys().cloned().collect();
            for connection_id in connection_ids {
                let Some(client) = self.clients.get_mut(&connection_id) else {
                    continue;
                };
                if !client.announced && client.connection.is_established() {
                    let (streams, stream_receiver) = channel::unbounded_async();
                    client.streams = Some(streams);
                    client.announced = true;
                    let _ = self.incoming.try_send(Incoming(Connection {
                        connection_id: connection_id.clone(),
                        peer_certificate: client
                            .connection
                            .peer_cert()
                            .map(|certificate| CertificateDer::from(certificate.to_vec())),
                        streams: stream_receiver,
                        commands: self.command_sender.clone(),
                    }));
                }

                let writable: Vec<_> = client.connection.writable().collect();
                for stream_id in writable {
                    progress_response(client, stream_id);
                }

                let readable: Vec<_> = client.connection.readable().collect();
                for stream_id in readable {
                    if stream_id & 0x3 != 0 {
                        continue;
                    }
                    receive_stream(client, stream_id);
                }
            }
        }

        async fn flush_packets(&mut self) -> Result<(), TransportError> {
            let mut datagrams = Vec::new();
            let mut output = vec![0_u8; MAX_DATAGRAM_BYTES];
            for client in self.clients.values_mut() {
                loop {
                    match client.connection.send(&mut output) {
                        Ok((written, information)) => {
                            datagrams.push((output[..written].to_vec(), information.to));
                        }
                        Err(quiche::Error::Done) => break,
                        Err(_) => {
                            let _ = client
                                .connection
                                .close(false, 1, b"packet generation failed");
                            break;
                        }
                    }
                }
            }
            for (datagram, address) in datagrams {
                let BufResult(result, _) = self.socket.send_to(datagram, address).await;
                result.map_err(|error| TransportError::backend(NAME, "packet send", error))?;
            }
            Ok(())
        }

        fn collect_closed(&mut self) {
            self.clients
                .retain(|_, client| !client.connection.is_closed());
            self.routes
                .retain(|_, connection_id| self.clients.contains_key(connection_id));
        }
    }

    fn receive_stream(client: &mut Client, stream_id: u64) {
        let Some(stream_sender) = client.streams.clone() else {
            return;
        };
        let request = client.requests.entry(stream_id).or_insert_with(|| {
            let (chunks, chunk_receiver) = channel::bounded_sync_async(STREAM_CHUNK_BACKLOG);
            let _ = stream_sender.try_send(Stream {
                stream_id,
                chunks: chunk_receiver,
            });
            RequestChunks {
                chunks,
                pending: None,
                finished: false,
            }
        });
        if let Some(chunk) = request.pending.take() {
            match request.chunks.try_send(chunk) {
                Ok(()) if request.finished => {
                    client.requests.remove(&stream_id);
                    return;
                }
                Ok(()) => {}
                Err(TrySendError::Full(chunk)) => {
                    request.pending = Some(chunk);
                    return;
                }
                Err(TrySendError::Disconnected(_)) => {
                    client.requests.remove(&stream_id);
                    return;
                }
            }
        }
        let mut buffer = [0_u8; STREAM_CHUNK_BYTES];
        loop {
            if request.chunks.is_full() {
                break;
            }
            match client.connection.stream_recv(stream_id, &mut buffer) {
                Ok((read, finished)) => {
                    request.finished = finished;
                    match request.chunks.try_send(buffer[..read].to_vec()) {
                        Ok(()) => {}
                        Err(TrySendError::Full(chunk)) => {
                            request.pending = Some(chunk);
                            break;
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            client.requests.remove(&stream_id);
                            break;
                        }
                    }
                    if finished {
                        client.requests.remove(&stream_id);
                        break;
                    }
                }
                Err(quiche::Error::Done) => break,
                Err(_) => {
                    client.requests.remove(&stream_id);
                    break;
                }
            }
        }
    }

    fn progress_response(client: &mut Client, stream_id: u64) {
        let Some(mut response) = client.responses.remove(&stream_id) else {
            return;
        };
        let remaining = &response.frame[response.written..];
        match client.connection.stream_send(stream_id, remaining, false) {
            Ok(written) => {
                response.written += written;
                if response.written == response.frame.len() {
                    let _ = response.reply.try_send(Ok(()));
                } else {
                    client.responses.insert(stream_id, response);
                }
            }
            Err(quiche::Error::Done) => {
                client.responses.insert(stream_id, response);
            }
            Err(error) => {
                let _ = response.reply.try_send(Err(error.to_string()));
            }
        }
    }

    fn config(
        material: &ServerTlsConfig,
        max_concurrent_streams: usize,
    ) -> Result<quiche::Config, TransportError> {
        let certificate = X509::from_der(
            material
                .certificate_chain
                .first()
                .expect("validated TLS certificate chain"),
        )
        .map_err(|error| TransportError::backend(NAME, "certificate parsing", error))?;
        let private_key = PKey::private_key_from_der(material.private_key.secret_der())
            .map_err(|error| TransportError::backend(NAME, "private key parsing", error))?;
        let mut tls = SslContextBuilder::new(SslMethod::tls())
            .map_err(|error| TransportError::backend(NAME, "TLS configuration", error))?;
        tls.set_certificate(&certificate)
            .map_err(|error| TransportError::backend(NAME, "TLS certificate", error))?;
        for certificate in material.certificate_chain.iter().skip(1) {
            let certificate = X509::from_der(certificate)
                .map_err(|error| TransportError::backend(NAME, "certificate parsing", error))?;
            tls.add_extra_chain_cert(certificate)
                .map_err(|error| TransportError::backend(NAME, "TLS certificate chain", error))?;
        }
        tls.set_private_key(&private_key)
            .map_err(|error| TransportError::backend(NAME, "TLS private key", error))?;
        if !material.client_ca.is_empty() {
            let mut roots = X509StoreBuilder::new()
                .map_err(|error| TransportError::backend(NAME, "client CA store", error))?;
            for certificate in &material.client_ca {
                let certificate = X509::from_der(certificate).map_err(|error| {
                    TransportError::backend(NAME, "client CA certificate parsing", error)
                })?;
                tls.add_client_ca(&certificate)
                    .map_err(|error| TransportError::backend(NAME, "client CA names", error))?;
                roots
                    .add_cert(certificate)
                    .map_err(|error| TransportError::backend(NAME, "client CA store", error))?;
            }
            tls.set_verify_cert_store(roots.build())
                .map_err(|error| TransportError::backend(NAME, "client CA store", error))?;
            tls.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
            tls.set_session_id_context(b"openkache-mtls")
                .map_err(|error| TransportError::backend(NAME, "TLS session identity", error))?;
        }
        let mut config = quiche::Config::with_boring_ssl_ctx_builder(quiche::PROTOCOL_VERSION, tls)
            .map_err(|error| TransportError::backend(NAME, "configuration", error))?;
        config
            .set_application_protos(&[openkache_protocol::ALPN])
            .map_err(|error| TransportError::backend(NAME, "ALPN", error))?;
        config.set_max_idle_timeout(30_000);
        config.set_max_recv_udp_payload_size(MAX_DATAGRAM_BYTES);
        config.set_max_send_udp_payload_size(1_350);
        config.set_initial_max_data(64 * 1024 * 1024);
        config.set_initial_max_stream_data_bidi_remote(MAX_BUFFERED_REQUEST_BYTES as u64);
        config.set_initial_max_stream_data_bidi_local(64 * 1024);
        config.set_initial_max_streams_bidi(max_concurrent_streams as u64);
        config.set_initial_max_streams_uni(0);
        config.set_disable_active_migration(true);
        Ok(config)
    }
}
