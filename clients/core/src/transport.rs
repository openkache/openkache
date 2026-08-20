//! QUIC backend boundary and persistent stream-lane pool.

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use crossfire::{MAsyncRx, MAsyncTx};
use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::{Response, ResponseHeaderBytes, ResponseParts};

use crate::request::RequestAttempt;
use crate::{Backend, Error, Operation, Result};

#[cfg(feature = "quic-compio")]
mod compio;
#[cfg(feature = "quic-quinn")]
mod quinn;

pub(crate) trait ClientConnection: Sized {
    type Lane<'a>: ClientLane
    where
        Self: 'a;

    fn connect(
        address: SocketAddr,
        server_name: &str,
        tls: rustls::ClientConfig,
        timeout: Duration,
        max_stream_lanes: usize,
        budget: RequestBudget,
    ) -> impl Future<Output = Result<Self>>;

    fn acquire_lane(&self, deadline: Deadline) -> impl Future<Output = Result<Self::Lane<'_>>>;

    fn negotiated_alpn(&self) -> Option<&[u8]>;

    fn timeout<F: Future>(
        duration: Duration,
        future: F,
    ) -> impl Future<Output = Result<Option<F::Output>>>;

    fn close(&self);
}

pub(crate) trait ClientLane {
    fn write_request(
        &mut self,
        request: RequestAttempt,
        timeout: Duration,
    ) -> impl Future<Output = Result<()>>;

    fn read_response(
        &mut self,
        maximum: usize,
        deadline: Deadline,
    ) -> impl Future<Output = Result<ResponseParts>>;

    /// Transfers the response-body lease to a caller-owned value.
    ///
    /// The lease is present only after a successful response read. Protected
    /// clients use it to keep network bytes accounted while value decoding
    /// performs authentication, decompression, and structured parsing.
    fn take_response_permit(&mut self) -> Option<RequestBudgetPermit>;

    fn release(self);
}

/// One weighted byte budget shared by transport and value work.
///
/// The permit is acquired before a response body allocation/read and is held
/// by the lane until the response callback completes. Value codecs use the
/// same budget for decrypted, decompressed, and structured-value allocations.
#[derive(Clone)]
pub struct InFlightByteBudget {
    inner: Arc<RequestBudgetInner>,
}

pub type RequestBudget = InFlightByteBudget;

struct RequestBudgetInner {
    capacity: usize,
    state: Mutex<RequestBudgetState>,
}

struct RequestBudgetState {
    used: usize,
    next_waiter_id: u64,
    waiters: HashMap<u64, Waker>,
}

pub struct BytePermit {
    inner: Arc<RequestBudgetInner>,
    bytes: usize,
}

pub(crate) type RequestBudgetPermit = BytePermit;

struct RequestBudgetAcquire {
    inner: Arc<RequestBudgetInner>,
    bytes: usize,
    waiter_id: Option<u64>,
}

impl InFlightByteBudget {
    /// Creates one aggregate byte budget shared by concurrent operations.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(RequestBudgetInner {
                capacity,
                state: Mutex::new(RequestBudgetState {
                    used: 0,
                    next_waiter_id: 0,
                    waiters: HashMap::new(),
                }),
            }),
        }
    }

    /// Returns the configured aggregate capacity.
    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }

    /// Reserves bytes synchronously for a bounded codec operation.
    /// Reserves bytes immediately or returns a local resource-limit error.
    pub fn try_reserve(&self, bytes: usize) -> Result<BytePermit> {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("request budget lock poisoned");
        if bytes > self.inner.capacity.saturating_sub(state.used) {
            return Err(Error::ResourceLimit {
                requested: bytes,
                maximum: self.inner.capacity,
            });
        }
        state.used += bytes;
        Ok(BytePermit {
            inner: Arc::clone(&self.inner),
            bytes,
        })
    }

    /// Waits for bytes without allocating until a permit is available.
    ///
    /// The timeout is applied by the backend wrapper around the response read;
    /// it is retained in this API so callers cannot accidentally omit an
    /// operation deadline when acquiring network capacity.
    pub async fn reserve(&self, bytes: usize, _timeout: Duration) -> Result<BytePermit> {
        if bytes > self.inner.capacity {
            return Err(Error::ResourceLimit {
                requested: bytes,
                maximum: self.inner.capacity,
            });
        }
        Ok(RequestBudgetAcquire {
            inner: Arc::clone(&self.inner),
            bytes,
            waiter_id: None,
        }
        .await)
    }

    pub(crate) async fn acquire(&self, bytes: usize, timeout: Duration) -> Result<BytePermit> {
        self.reserve(bytes, timeout).await
    }
}

impl Future for RequestBudgetAcquire {
    type Output = RequestBudgetPermit;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = Arc::clone(&self.inner);
        let mut state = inner.state.lock().expect("request budget lock poisoned");
        if self.bytes <= inner.capacity.saturating_sub(state.used) {
            if let Some(waiter_id) = self.waiter_id.take() {
                state.waiters.remove(&waiter_id);
            }
            state.used += self.bytes;
            drop(state);
            return Poll::Ready(RequestBudgetPermit {
                inner,
                bytes: self.bytes,
            });
        }

        if let Some(waiter_id) = self.waiter_id {
            if let Some(waiter) = state.waiters.get_mut(&waiter_id) {
                if !waiter.will_wake(context.waker()) {
                    waiter.clone_from(context.waker());
                }
            }
            return Poll::Pending;
        }

        let waiter_id = state.next_waiter_id;
        state.next_waiter_id = state
            .next_waiter_id
            .checked_add(1)
            .expect("request budget waiter identifier overflowed");
        state.waiters.insert(waiter_id, context.waker().clone());
        self.waiter_id = Some(waiter_id);
        Poll::Pending
    }
}

impl Drop for RequestBudgetAcquire {
    fn drop(&mut self) {
        if let Some(waiter_id) = self.waiter_id {
            self.inner
                .state
                .lock()
                .expect("request budget lock poisoned")
                .waiters
                .remove(&waiter_id);
        }
    }
}

impl BytePermit {
    /// Returns the number of bytes retained by this permit.
    pub const fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for BytePermit {
    fn drop(&mut self) {
        if self.bytes == 0 {
            return;
        }
        let waiters = {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("request budget lock poisoned");
            state.used = state
                .used
                .checked_sub(self.bytes)
                .expect("released request bytes must be reserved");
            // Keep registrations in place while waking. A wake-up is only a
            // hint: if one waiter consumes the newly available bytes, every
            // other waiter must remain registered for the next release.
            state.waiters.values().cloned().collect::<Vec<_>>()
        };
        for waiter in waiters {
            waiter.wake();
        }
    }
}

macro_rules! connection_backend {
    ($connection:ident, $lane:ident, $module:ident, $timeout:ident) => {
        pub(crate) struct $connection(PooledConnection<$module::Connection>);

        pub(crate) struct $lane<'a>(PooledLane<'a, $module::Connection>);

        impl ClientConnection for $connection {
            type Lane<'a> = $lane<'a>;

            async fn connect(
                address: SocketAddr,
                server_name: &str,
                tls: rustls::ClientConfig,
                timeout: Duration,
                max_stream_lanes: usize,
                budget: RequestBudget,
            ) -> Result<Self> {
                $module::connect(address, server_name, tls, timeout)
                    .await
                    .map(|connection| {
                        Self(PooledConnection::new(connection, max_stream_lanes, budget))
                    })
                    .map_err(Error::from)
            }

            async fn acquire_lane(&self, deadline: Deadline) -> Result<Self::Lane<'_>> {
                let remaining = deadline.remaining(Operation::StreamAcquisition)?;
                match $timeout(remaining, self.0.acquire_lane(deadline)).await? {
                    Some(result) => result.map($lane),
                    None => Err(Error::Timeout {
                        operation: Operation::StreamAcquisition,
                    }),
                }
            }

            fn negotiated_alpn(&self) -> Option<&[u8]> {
                self.0.inner.negotiated_alpn()
            }

            async fn timeout<F: Future>(
                duration: Duration,
                future: F,
            ) -> Result<Option<F::Output>> {
                $timeout(duration, future).await
            }

            fn close(&self) {
                self.0.inner.close();
            }
        }

        impl ClientLane for $lane<'_> {
            async fn write_request(
                &mut self,
                request: RequestAttempt,
                timeout: Duration,
            ) -> Result<()> {
                self.0.write_request(request, timeout).await
            }

            async fn read_response(
                &mut self,
                maximum: usize,
                deadline: Deadline,
            ) -> Result<ResponseParts> {
                let remaining = deadline.remaining(Operation::ResponseBodyRead)?;
                match $timeout(remaining, self.0.read_response(maximum, deadline)).await? {
                    Some(result) => result,
                    None => Err(Error::Timeout {
                        operation: Operation::ResponseBodyRead,
                    }),
                }
            }

            fn take_response_permit(&mut self) -> Option<RequestBudgetPermit> {
                self.0.take_response_permit()
            }

            fn release(self) {
                self.0.release();
            }
        }
    };
}

#[cfg(feature = "quic-quinn")]
connection_backend!(QuinnConnection, QuinnLane, quinn, timeout_quinn);
#[cfg(feature = "quic-compio")]
connection_backend!(CompioConnection, CompioLane, compio, timeout_compio);

#[cfg(feature = "quic-quinn")]
async fn timeout_quinn<F: Future>(duration: Duration, future: F) -> Result<Option<F::Output>> {
    if tokio::runtime::Handle::try_current().is_err() {
        return Err(
            TransportError::runtime(Backend::Quinn, "an active Tokio runtime is required").into(),
        );
    }
    Ok(tokio::time::timeout(duration, future).await.ok())
}

#[cfg(feature = "quic-compio")]
async fn timeout_compio<F: Future>(duration: Duration, future: F) -> Result<Option<F::Output>> {
    if ::compio::runtime::Runtime::try_current().is_none() {
        return Err(TransportError::runtime(
            Backend::Compio,
            "an active Compio runtime is required",
        )
        .into());
    }
    Ok(::compio::runtime::time::timeout(duration, future)
        .await
        .ok())
}

trait BackendConnection {
    type Stream: BackendStream + 'static;

    fn negotiated_alpn(&self) -> Option<&[u8]>;

    fn open_bi(
        &self,
        timeout: Duration,
    ) -> impl Future<Output = std::result::Result<Self::Stream, TransportError>>;

    fn close(&self);
}

trait BackendStream {
    fn write_request(
        &mut self,
        request: RequestAttempt,
        timeout: Duration,
    ) -> impl Future<Output = std::result::Result<(), TransportError>>;

    fn read_byte(
        &mut self,
        timeout: Duration,
    ) -> impl Future<Output = std::result::Result<u8, TransportError>>;

    fn read_exact(
        &mut self,
        length: usize,
        timeout: Duration,
    ) -> impl Future<Output = std::result::Result<Vec<u8>, TransportError>>;
}

type LaneSender<T> = MAsyncTx<crossfire::mpmc::Array<T>>;
type LaneReceiver<T> = MAsyncRx<crossfire::mpmc::Array<T>>;

struct PooledConnection<B: BackendConnection> {
    inner: B,
    idle_lanes_tx: LaneSender<B::Stream>,
    idle_lanes_rx: LaneReceiver<B::Stream>,
    lane_capacity_tx: LaneSender<()>,
    lane_capacity_rx: LaneReceiver<()>,
    open_lanes: AtomicUsize,
    max_stream_lanes: usize,
    budget: RequestBudget,
}

struct PooledLane<'a, B: BackendConnection> {
    connection: &'a PooledConnection<B>,
    stream: Option<B::Stream>,
    response_permit: Option<RequestBudgetPermit>,
}

impl<B: BackendConnection> PooledConnection<B> {
    fn new(inner: B, max_stream_lanes: usize, budget: RequestBudget) -> Self {
        let (idle_lanes_tx, idle_lanes_rx) = crossfire::mpmc::bounded_async(max_stream_lanes);
        let (lane_capacity_tx, lane_capacity_rx) = crossfire::mpmc::bounded_async(max_stream_lanes);
        Self {
            inner,
            idle_lanes_tx,
            idle_lanes_rx,
            lane_capacity_tx,
            lane_capacity_rx,
            open_lanes: AtomicUsize::new(0),
            max_stream_lanes,
            budget,
        }
    }

    async fn acquire_lane(&self, deadline: Deadline) -> Result<PooledLane<'_, B>> {
        loop {
            if let Ok(lane) = self.idle_lanes_rx.try_recv() {
                return Ok(PooledLane::new(self, lane));
            }
            if !self.reserve_lane() {
                let idle = self.idle_lanes_rx.recv().fuse();
                let capacity = self.lane_capacity_rx.recv().fuse();
                pin_mut!(idle, capacity);
                select! {
                    lane = idle => {
                        return lane
                            .map(|lane| PooledLane::new(self, lane))
                            .map_err(|_| Error::Connection("stream lane pool closed".into()));
                    }
                    _ = capacity => continue,
                }
            }

            let reservation = LaneReservation::new(self);
            let opening = self
                .inner
                .open_bi(deadline.remaining(Operation::StreamAcquisition)?)
                .fuse();
            let idle = self.idle_lanes_rx.recv().fuse();
            pin_mut!(opening, idle);
            select! {
                opened = opening => {
                    let stream = opened?;
                    reservation.commit();
                    return Ok(PooledLane::new(self, stream));
                }
                lane = idle => {
                    return lane
                        .map(|lane| PooledLane::new(self, lane))
                        .map_err(|_| Error::Connection("stream lane pool closed".into()));
                }
            }
        }
    }

    fn reserve_lane(&self) -> bool {
        self.open_lanes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |open| {
                (open < self.max_stream_lanes).then_some(open + 1)
            })
            .is_ok()
    }

    fn release_lane(&self, lane: B::Stream) {
        if self.idle_lanes_tx.try_send(lane).is_err() {
            self.remove_lane();
        }
    }

    fn discard_lane(&self, lane: B::Stream) {
        drop(lane);
        self.remove_lane();
    }

    fn remove_lane(&self) {
        if self
            .open_lanes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |open| {
                open.checked_sub(1)
            })
            .is_ok()
        {
            let _ = self.lane_capacity_tx.try_send(());
        }
    }
}

impl<'a, B: BackendConnection> PooledLane<'a, B> {
    fn new(connection: &'a PooledConnection<B>, stream: B::Stream) -> Self {
        Self {
            connection,
            stream: Some(stream),
            response_permit: None,
        }
    }

    async fn write_request(&mut self, request: RequestAttempt, timeout: Duration) -> Result<()> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| Error::Connection("stream lane has already been released".into()))?;
        // Retain request bytes only for the duration of the network write.
        // The permit is dropped before response admission, allowing one
        // full-sized request and response to make progress under the same
        // aggregate budget.
        let _request_permit = self.connection.budget.try_reserve(request.len())?;
        stream.write_request(request, timeout).await?;
        Ok(())
    }

    async fn read_response(&mut self, maximum: usize, deadline: Deadline) -> Result<ResponseParts> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| Error::Connection("stream lane has already been released".into()))?;
        let mut header_bytes = ResponseHeaderBytes::new();
        let header = loop {
            header_bytes
                .push(
                    stream
                        .read_byte(deadline.remaining(Operation::ResponseHeaderRead)?)
                        .await?,
                )
                .map_err(Error::protocol)?;
            if let Some(header) =
                Response::decode_header(header_bytes.as_slice()).map_err(Error::protocol)?
            {
                break header;
            }
        };
        let frame_len = header.frame_len().map_err(Error::protocol)?;
        if frame_len > maximum {
            return Err(Error::ResponseTooLarge { maximum });
        }
        let body_len = header.payload_len();
        let permit = self
            .connection
            .budget
            .acquire(body_len, deadline.remaining(Operation::ResponseBodyRead)?)
            .await?;
        let payload = if body_len == 0 {
            Vec::new()
        } else {
            stream
                .read_exact(body_len, deadline.remaining(Operation::ResponseBodyRead)?)
                .await?
        };
        let response = ResponseParts::decode(header_bytes, payload).map_err(Error::protocol)?;
        self.response_permit = Some(permit);
        Ok(response)
    }

    fn take_response_permit(&mut self) -> Option<RequestBudgetPermit> {
        self.response_permit.take()
    }

    fn release(mut self) {
        if let Some(stream) = self.stream.take() {
            self.connection.release_lane(stream);
        }
    }
}

impl<B: BackendConnection> Drop for PooledLane<'_, B> {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            self.connection.discard_lane(stream);
        }
    }
}

struct LaneReservation<'a, B: BackendConnection> {
    connection: &'a PooledConnection<B>,
    active: bool,
}

impl<'a, B: BackendConnection> LaneReservation<'a, B> {
    fn new(connection: &'a PooledConnection<B>) -> Self {
        Self {
            connection,
            active: true,
        }
    }

    fn commit(mut self) {
        self.active = false;
    }
}

impl<B: BackendConnection> Drop for LaneReservation<'_, B> {
    fn drop(&mut self) {
        if self.active {
            self.connection.remove_lane();
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Deadline(Instant);

impl Deadline {
    pub(crate) fn after(timeout: Duration) -> Result<Self> {
        Instant::now()
            .checked_add(timeout)
            .map(Self)
            .ok_or_else(|| {
                Error::configuration("request_timeout", "exceeds the platform clock range")
            })
    }

    pub(crate) fn remaining(self, operation: Operation) -> Result<Duration> {
        self.0
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(Error::Timeout { operation })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{backend} QUIC {operation} failed: {message}")]
pub(crate) struct TransportError {
    backend: Backend,
    operation: Operation,
    message: String,
    kind: TransportErrorKind,
}

#[derive(Clone, Copy, Debug)]
enum TransportErrorKind {
    Runtime,
    Timeout,
    Transport,
}

impl TransportError {
    fn backend(backend: Backend, operation: Operation, error: impl std::fmt::Display) -> Self {
        Self {
            backend,
            operation,
            message: error.to_string(),
            kind: TransportErrorKind::Transport,
        }
    }

    fn runtime(backend: Backend, message: &'static str) -> Self {
        Self {
            backend,
            operation: Operation::ConnectionSetup,
            message: message.into(),
            kind: TransportErrorKind::Runtime,
        }
    }

    fn timeout(backend: Backend, operation: Operation, timeout: Duration) -> Self {
        Self {
            backend,
            operation,
            message: format!("timed out after {timeout:?}"),
            kind: TransportErrorKind::Timeout,
        }
    }
}

impl From<TransportError> for Error {
    fn from(error: TransportError) -> Self {
        match error.kind {
            TransportErrorKind::Runtime => Self::Runtime {
                backend: error.backend,
                message: error.message,
            },
            TransportErrorKind::Timeout => Self::Timeout {
                operation: error.operation,
            },
            TransportErrorKind::Transport => Self::Transport {
                backend: error.backend,
                operation: error.operation,
                message: error.message,
            },
        }
    }
}
