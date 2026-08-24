//! QUIC backend boundary used by the OpenKache protocol server.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::QuicBackend;
use crate::network_runtime;
use crate::protocol::RequestFrame as ProtocolRequestFrame;
#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
use openkache_protocol::ResponseSegment;
use openkache_protocol::{RequestFrameHeader, ResponseParts};

#[path = "transport/tls.rs"]
mod tls;
pub(super) use tls::strict_server_config;
#[path = "transport/tcp.rs"]
pub(super) mod tcp;

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
            QuicBackend::Neqo => Err(TransportError::unavailable(
                backend,
                "the official neqo transport is not published as a standalone crate and requires NSS certificate-database integration",
            )),
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

    /// Takes the authenticated peer's leaf certificate, when client authentication is enabled.
    fn take_peer_certificate(&mut self) -> Option<CertificateDer<'static>>;

    fn accept_bi(
        &self,
    ) -> impl Future<Output = Result<(Self::SendStream, Self::ReceiveStream), TransportError>>;

    /// Accepts an incoming unidirectional stream so the protocol server can
    /// reject it without reading application bytes.
    fn accept_uni(&self) -> impl Future<Output = Result<Self::ReceiveStream, TransportError>>;

    /// Closes this connection with an application error.
    fn close(&self, error_code: u64, reason: &[u8]);
}

/// Receive half of one request stream.
pub(super) trait ReceiveStream {
    fn read_request<T>(
        &mut self,
        maximum: usize,
        timeout: Duration,
        budget: &RequestBudget,
        progress: &AtomicBool,
        admit: impl FnOnce(RequestFrameHeader, &[u8]) -> Result<(), T>,
    ) -> impl Future<Output = Result<RequestRead<T>, StreamReadError>>;

    /// Rejects further request-direction bytes without consuming them.
    fn stop(&mut self);
}

/// Send half of one response stream.
pub(super) trait SendStream {
    fn write_response(
        &mut self,
        parts: ResponseParts,
        timeout: Duration,
    ) -> impl Future<Output = Result<(), TransportError>>;

    /// Sends a clean response-direction half-close after queued responses.
    fn finish(&mut self) -> Result<(), TransportError>;

    /// Cancels the response direction after the peer has stopped reading it.
    fn reset(&mut self);
}

/// Failure while receiving a request frame.
#[derive(Debug, thiserror::Error)]
pub(super) enum StreamReadError {
    #[error("request read timed out")]
    Timeout,
    #[error("request exceeds the protocol limit")]
    TooLarge,
    #[error(transparent)]
    Malformed(#[from] openkache_protocol::ProtocolError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

/// Result of advancing a request lane by one explicit frame boundary.
///
/// A clean finish is observable only before another frame begins. A peer reset
/// is directional cancellation rather than malformed framing, even when it
/// interrupts an incomplete frame.
pub(super) enum RequestRead<T> {
    /// One complete admitted request and its reserved body bytes.
    Frame(RequestFrame),
    /// A complete, delimited request rejected before operation execution.
    Rejected {
        header: RequestFrameHeader,
        rejection: T,
    },
    /// A complete request that could not reserve the shared body budget.
    ///
    /// The body has been discarded exactly to its declared boundary, so this
    /// remains a correlated, recoverable lane event rather than malformed
    /// framing.
    Overloaded {
        header: RequestFrameHeader,
        timed_out: bool,
    },
    /// The lane has complete buffered bytes but its response window is full.
    ///
    /// This is a recoverable receive-side pause, not malformed framing. The
    /// caller must finish an outstanding response before asking the lane for
    /// another request event.
    Backpressured,
    /// The peer FINed the request direction at a frame boundary.
    Finished,
    /// The peer reset the request direction.
    Cancelled,
}

/// Request bytes paired with the server-wide memory-budget reservation they consume.
pub(super) struct RequestFrame {
    pub(super) header: RequestFrameHeader,
    pub(super) bytes: Vec<u8>,
    pub(super) _permit: RequestBudgetPermit,
}

impl RequestFrame {
    fn new(header: RequestFrameHeader, bytes: Vec<u8>, permit: RequestBudgetPermit) -> Self {
        Self {
            header,
            bytes,
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

    pub(super) fn capacity(&self) -> usize {
        self.inner
            .lock()
            .expect("request budget lock poisoned")
            .capacity
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

trait RequestByteStream {
    fn append_chunk(
        &mut self,
        frame: Vec<u8>,
        capacity: usize,
        backend: &'static str,
    ) -> impl Future<Output = Result<ChunkRead, TransportError>>;
}

enum ChunkRead {
    Bytes(Vec<u8>),
    Finished,
    Cancelled,
}

async fn read_buffered_request<S: RequestByteStream, T>(
    stream: &mut S,
    backend: &'static str,
    maximum: usize,
    timeout: Duration,
    budget: &RequestBudget,
    progress: &AtomicBool,
    admit: impl FnOnce(RequestFrameHeader, &[u8]) -> Result<(), T>,
) -> Result<RequestRead<T>, StreamReadError> {
    // A request deadline covers the complete frame, including admission,
    // memory-budget waiting, and every partial transport read. Passing the
    // configured timeout to each chunk would let a slow peer reset the budget
    // indefinitely by dribbling one byte per interval.
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    let header_read = read_request_header(stream, backend, maximum, deadline, progress).await?;
    let (mut frame, header) = match header_read {
        HeaderRead::Header { frame, header } => (frame, header),
        HeaderRead::Finished => return Ok(RequestRead::Finished),
        HeaderRead::Cancelled => return Ok(RequestRead::Cancelled),
    };
    let frame_len = header.frame_len()?;
    if frame_len > maximum {
        return Err(StreamReadError::TooLarge);
    }

    if let Err(rejection) = admit(header, &frame[..header.encoded_len()]) {
        match discard_body(
            stream,
            backend,
            frame_len.saturating_sub(frame.len()),
            deadline,
            progress,
        )
        .await?
        {
            BodyRead::Complete => {
                return Ok(RequestRead::Rejected { header, rejection });
            }
            BodyRead::Cancelled => return Ok(RequestRead::Cancelled),
        }
    }

    let permit = match budget
        .acquire(header.body_len(), remaining_request_timeout(deadline)?)
        .await
    {
        Ok(permit) => permit,
        Err(error @ (StreamReadError::Timeout | StreamReadError::TooLarge)) => {
            let timed_out = matches!(error, StreamReadError::Timeout);
            // A request that cannot reserve memory was never admitted. Drain
            // exactly its known body so later pipelined frame boundaries stay
            // intact; the caller turns the rejection into a correlated
            // overload response.
            match discard_body(
                stream,
                backend,
                frame_len.saturating_sub(frame.len()),
                deadline,
                progress,
            )
            .await?
            {
                BodyRead::Complete => {
                    return Ok(RequestRead::Overloaded { header, timed_out });
                }
                BodyRead::Cancelled => return Ok(RequestRead::Cancelled),
            }
        }
        Err(error) => return Err(error),
    };
    loop {
        let remaining = frame_len.saturating_sub(frame.len());
        if remaining == 0 {
            return Ok(RequestRead::Frame(RequestFrame::new(header, frame, permit)));
        }
        let actual = frame.len();
        let previous_len = frame.len();
        match append_chunk(stream, frame, remaining, backend, deadline).await? {
            ChunkRead::Bytes(next) => {
                // The caller may race this read against operation execution.
                // Mark delivery after extending the local frame so it knows
                // whether dropping the future would lose buffered bytes.
                if next.len() > previous_len {
                    progress.store(true, Ordering::Relaxed);
                }
                frame = next;
            }
            ChunkRead::Finished => {
                return Err(truncated_frame_error(actual, frame_len));
            }
            ChunkRead::Cancelled => return Ok(RequestRead::Cancelled),
        }
    }
}

async fn read_request_header<S: RequestByteStream>(
    stream: &mut S,
    backend: &'static str,
    maximum: usize,
    deadline: Instant,
    progress: &AtomicBool,
) -> Result<HeaderRead, StreamReadError> {
    let mut frame = Vec::new();
    loop {
        let additional = ProtocolRequestFrame::header_bytes_needed(&frame)?;
        if additional == 0 {
            let header = ProtocolRequestFrame::decode_header(&frame)?.ok_or(
                openkache_protocol::ProtocolError::InvalidFieldSequence(
                    "header sizing completed before header decode",
                ),
            )?;
            return Ok(HeaderRead::Header { frame, header });
        }
        if additional > maximum.saturating_sub(frame.len()) {
            return Err(StreamReadError::TooLarge);
        }
        let expected = frame
            .len()
            .checked_add(additional)
            .ok_or(openkache_protocol::ProtocolError::FrameLengthOverflow)?;
        let was_empty = frame.is_empty();
        let actual = frame.len();
        let previous_len = frame.len();
        match append_chunk(stream, frame, additional, backend, deadline).await? {
            ChunkRead::Bytes(next) => {
                if next.len() > previous_len {
                    progress.store(true, Ordering::Relaxed);
                }
                frame = next;
            }
            ChunkRead::Finished if was_empty => return Ok(HeaderRead::Finished),
            ChunkRead::Finished => return Err(truncated_frame_error(actual, expected)),
            ChunkRead::Cancelled => return Ok(HeaderRead::Cancelled),
        }
    }
}

enum HeaderRead {
    Header {
        frame: Vec<u8>,
        header: RequestFrameHeader,
    },
    Finished,
    Cancelled,
}

async fn append_chunk<S: RequestByteStream>(
    stream: &mut S,
    frame: Vec<u8>,
    capacity: usize,
    backend: &'static str,
    deadline: Instant,
) -> Result<ChunkRead, StreamReadError> {
    network_runtime::timeout(
        remaining_request_timeout(deadline)?,
        stream.append_chunk(frame, capacity, backend),
    )
        .await
        .map_err(|_| StreamReadError::Timeout)?
        .map_err(StreamReadError::Transport)
}

enum BodyRead {
    Complete,
    Cancelled,
}

async fn discard_body<S: RequestByteStream>(
    stream: &mut S,
    backend: &'static str,
    mut remaining: usize,
    deadline: Instant,
    progress: &AtomicBool,
) -> Result<BodyRead, StreamReadError> {
    const DISCARD_CHUNK_BYTES: usize = 16 * 1024;
    let mut scratch = Vec::new();
    while remaining > 0 {
        let capacity = remaining.min(DISCARD_CHUNK_BYTES);
        let previous_len = scratch.len();
        match append_chunk(stream, scratch, capacity, backend, deadline).await? {
            ChunkRead::Bytes(next) => {
                let read = next
                    .len()
                    .checked_sub(previous_len)
                    .ok_or(openkache_protocol::ProtocolError::FrameLengthOverflow)?;
                if read > 0 {
                    progress.store(true, Ordering::Relaxed);
                }
                remaining = remaining
                    .checked_sub(read)
                    .ok_or(openkache_protocol::ProtocolError::FrameLengthOverflow)?;
                scratch = next;
                scratch.clear();
            }
            ChunkRead::Finished => {
                return Err(truncated_frame_error(0, remaining));
            }
            ChunkRead::Cancelled => return Ok(BodyRead::Cancelled),
        }
    }
    Ok(BodyRead::Complete)
}

fn remaining_request_timeout(deadline: Instant) -> Result<Duration, StreamReadError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(StreamReadError::Timeout)
    } else {
        Ok(remaining)
    }
}

fn truncated_frame_error(actual: usize, expected: usize) -> StreamReadError {
    StreamReadError::Malformed(openkache_protocol::ProtocolError::FrameTooShort {
        expected,
        actual,
    })
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
    pub(crate) fn backend(
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

#[cfg(feature = "quic-noq")]
#[path = "transport/noq_backend.rs"]
mod noq_backend;
#[cfg(feature = "quic-quiche")]
#[path = "transport/quiche_backend.rs"]
mod quiche_backend;
#[cfg(feature = "quic-quinn")]
#[path = "transport/quinn_backend.rs"]
mod quinn_backend;
