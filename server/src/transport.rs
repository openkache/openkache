//! QUIC backend boundary used by the OpenKache protocol server.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::QuicBackend;
use crate::network_runtime;
use crate::protocol::RequestFrame as ProtocolRequestFrame;
#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
use openkache_protocol::ResponseSegment;
use openkache_protocol::{RequestFrameHeader, ResponseParts};

#[path = "transport/tls.rs"]
mod tls;
#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
use tls::strict_server_config;
#[path = "transport/tcp.rs"]
mod tcp;

/// QUIC application error for connection-fatal malformed framing.
pub(super) const QUIC_MALFORMED_FRAME_ERROR_CODE: u64 = 0x01;

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
    /// Reports whether a compiled provider can enforce the transport profile.
    pub(super) fn conformance(backend: QuicBackend) -> tls::Conformance {
        match backend {
            QuicBackend::Quinn | QuicBackend::Noq | QuicBackend::Quiche => {
                tls::Conformance::Conforming
            }
            QuicBackend::Neqo => tls::Conformance::Unsupported(
                "neqo's NSS adapter cannot yet require X25519MLKEM768",
            ),
        }
    }

    /// Rejects backend selections that this binary cannot initialize.
    pub(super) fn validate_backend(backend: QuicBackend) -> Result<(), TransportError> {
        if let Some(reason) = Self::conformance(backend).reason() {
            return Err(TransportError::unavailable(backend, reason));
        }
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
            QuicBackend::Neqo => unreachable!("non-conforming backends return above"),
        }
    }

    /// Binds the selected implementation to the caller's UDP socket.
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
            QuicBackend::Neqo => unreachable!("non-conforming backends return above"),
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

    /// Takes the authenticated peer's leaf certificate, when client authentication is enabled.
    fn take_peer_certificate(&mut self) -> Option<CertificateDer<'static>>;

    /// Closes the QUIC connection with an application error code.
    fn close(&self, error_code: u64, reason: &[u8]);

    fn accept_bi(
        &self,
    ) -> impl Future<Output = Result<(Self::SendStream, Self::ReceiveStream), TransportError>>;
}

/// Receive half of one request stream.
pub(super) trait ReceiveStream {
    fn read_request<T>(
        &mut self,
        maximum: usize,
        timeout: Duration,
        budget: &RequestBudget,
        admit: impl FnOnce(RequestFrameHeader, &[u8]) -> Result<(), T>,
    ) -> impl Future<Output = Result<Result<RequestFrame, T>, StreamReadError>>;
}

/// Send half of one response stream.
pub(super) trait SendStream {
    fn write_response(
        &mut self,
        parts: ResponseParts,
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

/// Request bytes paired with the server-wide memory-budget reservation they consume.
pub(super) struct RequestFrame {
    pub(super) bytes: Vec<u8>,
    /// Whether the transport had already delivered bytes beyond this frame.
    ///
    /// QUIC stream reads may coalesce multiple client writes. If the backend
    /// exposes those trailing bytes, the lane must be retired after the
    /// current response because the peer violated request/response lockstep.
    pub(super) has_trailing_bytes: bool,
    _permit: RequestBudgetPermit,
}

impl RequestFrame {
    fn with_trailing_bytes(
        bytes: Vec<u8>,
        permit: RequestBudgetPermit,
        has_trailing_bytes: bool,
    ) -> Self {
        Self {
            bytes,
            has_trailing_bytes,
            _permit: permit,
        }
    }
}

/// Byte-weighted memory budget shared by every connection and network worker.
#[derive(Clone)]
pub(super) struct RequestBudget {
    inner: Arc<Mutex<RequestBudgetState>>,
}

struct RequestBudgetState {
    capacity: usize,
    used: usize,
    next_waiter_id: u64,
    waiters: HashMap<u64, Waker>,
}

pub(super) struct RequestBudgetPermit {
    inner: Arc<Mutex<RequestBudgetState>>,
    bytes: usize,
}

struct RequestBudgetAcquire {
    inner: Arc<Mutex<RequestBudgetState>>,
    bytes: usize,
    waiter_id: Option<u64>,
}

impl RequestBudget {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RequestBudgetState {
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
                inner: Arc::clone(&self.inner),
                bytes: 0,
            });
        }
        if bytes
            > self
                .inner
                .lock()
                .expect("request budget lock poisoned")
                .capacity
        {
            return Err(StreamReadError::TooLarge);
        }
        network_runtime::timeout(
            timeout,
            RequestBudgetAcquire {
                inner: Arc::clone(&self.inner),
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
        let inner = Arc::clone(&self.inner);
        let bytes = self.bytes;
        let mut state = inner.lock().expect("request budget lock poisoned");
        if state.used <= state.capacity - bytes {
            if let Some(waiter_id) = self.waiter_id.take() {
                state.waiters.remove(&waiter_id);
            }
            state.used += bytes;
            return Poll::Ready(RequestBudgetPermit {
                inner: Arc::clone(&inner),
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
            self.inner
                .lock()
                .expect("request budget lock poisoned")
                .waiters
                .remove(&waiter_id);
        }
    }
}

impl Drop for RequestBudgetPermit {
    fn drop(&mut self) {
        if self.bytes == 0 {
            return;
        }
        let mut waiters = {
            let mut state = self.inner.lock().expect("request budget lock poisoned");
            state.used = state
                .used
                .checked_sub(self.bytes)
                .expect("released request bytes must be reserved");
            std::mem::take(&mut state.waiters)
        };
        for (_, waiter) in waiters.drain() {
            waiter.wake();
        }
        let mut state = self.inner.lock().expect("request budget lock poisoned");
        if state.waiters.is_empty() {
            state.waiters = waiters;
        }
    }
}

#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
trait RequestByteStream {
    fn append_chunk(
        &mut self,
        frame: Vec<u8>,
        capacity: usize,
        backend: &'static str,
    ) -> impl Future<Output = Result<Vec<u8>, TransportError>>;
}

#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
async fn read_buffered_request<S: RequestByteStream, T>(
    stream: &mut S,
    backend: &'static str,
    maximum: usize,
    timeout: Duration,
    budget: &RequestBudget,
    admit: impl FnOnce(RequestFrameHeader, &[u8]) -> Result<(), T>,
) -> Result<Result<RequestFrame, T>, StreamReadError> {
    let first = network_runtime::timeout(timeout, stream.append_chunk(Vec::new(), 1, backend))
        .await
        .map_err(|_| StreamReadError::Timeout)?
        .map_err(StreamReadError::Transport)?;
    if first.is_empty() {
        return Err(StreamReadError::Transport(TransportError::backend(
            backend,
            "stream header read",
            "stream ended before a request frame",
        )));
    }
    let (frame, header) = network_runtime::timeout(timeout, async {
        let mut frame = first;
        loop {
            let additional = ProtocolRequestFrame::header_bytes_needed(&frame)?;
            if additional == 0 {
                let header = ProtocolRequestFrame::decode_header(&frame)?.ok_or(
                    openkache_protocol::ProtocolError::InvalidFieldSequence(
                        "header sizing completed before header decode",
                    ),
                )?;
                break Ok::<_, StreamReadError>((frame, header));
            }
            if additional > maximum.saturating_sub(frame.len()) {
                return Err(StreamReadError::TooLarge);
            }
            let previous_len = frame.len();
            frame = stream
                .append_chunk(frame, additional, backend)
                .await
                .map_err(StreamReadError::Transport)?;
            if frame.len() == previous_len {
                return Err(StreamReadError::Transport(TransportError::backend(
                    backend,
                    "stream header read",
                    "stream ended before request header completed",
                )));
            }
        }
    })
    .await
    .map_err(|_| StreamReadError::Timeout)??;
    let (mut frame, frame_len) = network_runtime::timeout(timeout, async {
        let frame = frame;
        let frame_len = header.frame_len()?;
        Ok::<_, StreamReadError>((frame, frame_len))
    })
    .await
    .map_err(|_| StreamReadError::Timeout)??;
    if frame_len > maximum {
        return Err(StreamReadError::TooLarge);
    }
    // Admission may reject on bounded header metadata, but the lane must
    // still consume the declared body before sending that correlated error.
    // This preserves the next frame boundary and allows the lane to continue.
    let rejection = admit(header, &frame[..header.encoded_len()]).err();
    let permit = budget.acquire(header.body_len(), timeout).await?;
    let body = network_runtime::timeout(timeout, async {
        while frame.len() < frame_len {
            let previous_len = frame.len();
            frame = stream
                .append_chunk(frame, frame_len - previous_len, backend)
                .await
                .map_err(StreamReadError::Transport)?;
            if frame.len() == previous_len {
                return Err(StreamReadError::Transport(TransportError::backend(
                    backend,
                    "stream body read",
                    "stream ended before request body completed",
                )));
            }
        }
        Ok::<_, StreamReadError>(frame)
    })
    .await
    .map_err(|_| StreamReadError::Timeout)??;
    if let Some(rejection) = rejection {
        drop(permit);
        return Ok(Err(rejection));
    }
    // Probe the backend's already-readable bytes once so a client that pipelined a second
    // request cannot make us interpret that request after the first response.
    // The zero-duration timeout is non-blocking: when no byte is buffered the
    // receive future is cancelled and the lane remains reusable.
    let has_trailing_bytes =
        match network_runtime::timeout(Duration::ZERO, stream.has_readable_byte(backend)).await {
            Err(_) => false,
            Ok(result) => result.map_err(StreamReadError::Transport)?,
        };
    Ok(Ok(RequestFrame::with_trailing_bytes(
        body,
        permit,
        has_trailing_bytes,
    )))
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
struct ResponseWriteSegments(smallvec::SmallVec<[ResponseSegment; 8]>);

#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
impl compio::buf::IoVectoredBuf for ResponseWriteSegments {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        self.0.iter().map(ResponseSegment::as_slice)
    }
}

#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
fn response_write_segments(parts: ResponseParts) -> ResponseWriteSegments {
    ResponseWriteSegments(parts.into_segments())
}

#[cfg(feature = "quic-quinn")]
#[path = "transport/quinn_backend.rs"]
mod quinn_backend;
#[cfg(feature = "quic-noq")]
#[path = "transport/noq_backend.rs"]
mod noq_backend;
#[cfg(feature = "quic-quiche")]
#[path = "transport/quiche_backend.rs"]
mod quiche_backend;
