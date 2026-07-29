//! QUIC backend boundary and persistent stream-lane pool.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::{RESPONSE_HEADER_BYTES, Response};

use crate::{Error, QuicBackend, Result};

#[cfg(feature = "quic-compio")]
mod compio;
#[cfg(feature = "quic-quinn")]
mod quinn;

const MAX_STREAM_LANES: usize = 256;

pub(crate) struct Connection(ConnectionInner);

enum ConnectionInner {
    #[cfg(feature = "quic-compio")]
    Compio(PooledConnection<compio::Connection>),
    #[cfg(feature = "quic-quinn")]
    Quinn(PooledConnection<quinn::Connection>),
}

pub(crate) struct Lane<'a>(LaneInner<'a>);

enum LaneInner<'a> {
    #[cfg(feature = "quic-compio")]
    Compio(PooledLane<'a, compio::Connection>),
    #[cfg(feature = "quic-quinn")]
    Quinn(PooledLane<'a, quinn::Connection>),
}

pub(crate) async fn connect(
    backend: QuicBackend,
    address: SocketAddr,
    server_name: &str,
    tls: rustls::ClientConfig,
    timeout: Duration,
) -> Result<Connection> {
    match backend {
        QuicBackend::Compio => {
            #[cfg(feature = "quic-compio")]
            {
                compio::connect(address, server_name, tls, timeout)
                    .await
                    .map(PooledConnection::new)
                    .map(ConnectionInner::Compio)
                    .map(Connection)
                    .map_err(Error::from)
            }
            #[cfg(not(feature = "quic-compio"))]
            {
                Err(TransportError::not_compiled(backend, "quic-compio").into())
            }
        }
        QuicBackend::Quinn => {
            #[cfg(feature = "quic-quinn")]
            {
                quinn::connect(address, server_name, tls, timeout)
                    .await
                    .map(PooledConnection::new)
                    .map(ConnectionInner::Quinn)
                    .map(Connection)
                    .map_err(Error::from)
            }
            #[cfg(not(feature = "quic-quinn"))]
            {
                Err(TransportError::not_compiled(backend, "quic-quinn").into())
            }
        }
    }
}

pub(crate) async fn timeout<F: Future>(
    backend: QuicBackend,
    duration: Duration,
    future: F,
) -> Result<Option<F::Output>> {
    match backend {
        QuicBackend::Compio => {
            #[cfg(feature = "quic-compio")]
            {
                if ::compio::runtime::Runtime::try_current().is_none() {
                    return Err(TransportError::runtime(
                        "compio",
                        "an active Compio runtime is required",
                    )
                    .into());
                }
                Ok(::compio::runtime::time::timeout(duration, future)
                    .await
                    .ok())
            }
            #[cfg(not(feature = "quic-compio"))]
            {
                drop(future);
                Err(TransportError::not_compiled(backend, "quic-compio").into())
            }
        }
        QuicBackend::Quinn => {
            #[cfg(feature = "quic-quinn")]
            {
                if tokio::runtime::Handle::try_current().is_err() {
                    return Err(TransportError::runtime(
                        "quinn",
                        "an active Tokio runtime is required",
                    )
                    .into());
                }
                Ok(tokio::time::timeout(duration, future).await.ok())
            }
            #[cfg(not(feature = "quic-quinn"))]
            {
                drop(future);
                Err(TransportError::not_compiled(backend, "quic-quinn").into())
            }
        }
    }
}

impl Connection {
    pub(crate) async fn acquire_lane(&self, deadline: Deadline) -> Result<Lane<'_>> {
        match &self.0 {
            #[cfg(feature = "quic-compio")]
            ConnectionInner::Compio(connection) => {
                acquire_lane_with_deadline(QuicBackend::Compio, connection, deadline)
                    .await
                    .map(LaneInner::Compio)
                    .map(Lane)
            }
            #[cfg(feature = "quic-quinn")]
            ConnectionInner::Quinn(connection) => {
                acquire_lane_with_deadline(QuicBackend::Quinn, connection, deadline)
                    .await
                    .map(LaneInner::Quinn)
                    .map(Lane)
            }
        }
    }
}

async fn acquire_lane_with_deadline<B: BackendConnection>(
    backend: QuicBackend,
    connection: &PooledConnection<B>,
    deadline: Deadline,
) -> Result<PooledLane<'_, B>> {
    let remaining = deadline.remaining("stream acquisition")?;
    match timeout(backend, remaining, connection.acquire_lane(deadline)).await? {
        Some(result) => result,
        None => Err(Error::Timeout {
            operation: "stream acquisition",
        }),
    }
}

impl Lane<'_> {
    pub(crate) async fn write_request(&mut self, frame: Vec<u8>, deadline: Deadline) -> Result<()> {
        match &mut self.0 {
            #[cfg(feature = "quic-compio")]
            LaneInner::Compio(lane) => lane.write_request(frame, deadline).await,
            #[cfg(feature = "quic-quinn")]
            LaneInner::Quinn(lane) => lane.write_request(frame, deadline).await,
        }
    }

    pub(crate) async fn read_response(
        &mut self,
        maximum: usize,
        deadline: Deadline,
    ) -> Result<Vec<u8>> {
        match &mut self.0 {
            #[cfg(feature = "quic-compio")]
            LaneInner::Compio(lane) => lane.read_response(maximum, deadline).await,
            #[cfg(feature = "quic-quinn")]
            LaneInner::Quinn(lane) => lane.read_response(maximum, deadline).await,
        }
    }

    pub(crate) fn release(self) {
        match self.0 {
            #[cfg(feature = "quic-compio")]
            LaneInner::Compio(lane) => lane.release(),
            #[cfg(feature = "quic-quinn")]
            LaneInner::Quinn(lane) => lane.release(),
        }
    }
}

trait BackendConnection {
    type Stream: BackendStream;

    fn open_bi(
        &self,
        timeout: Duration,
    ) -> impl Future<Output = std::result::Result<Self::Stream, TransportError>>;
}

trait BackendStream {
    fn write_all(
        &mut self,
        bytes: Vec<u8>,
        timeout: Duration,
    ) -> impl Future<Output = std::result::Result<(), TransportError>>;

    fn read_exact(
        &mut self,
        length: usize,
        timeout: Duration,
    ) -> impl Future<Output = std::result::Result<Vec<u8>, TransportError>>;
}

struct PooledConnection<B: BackendConnection> {
    inner: B,
    idle_lanes_tx: flume::Sender<B::Stream>,
    idle_lanes_rx: flume::Receiver<B::Stream>,
    lane_capacity_tx: flume::Sender<()>,
    lane_capacity_rx: flume::Receiver<()>,
    open_lanes: AtomicUsize,
}

struct PooledLane<'a, B: BackendConnection> {
    connection: &'a PooledConnection<B>,
    stream: Option<B::Stream>,
}

impl<B: BackendConnection> PooledConnection<B> {
    fn new(inner: B) -> Self {
        let (idle_lanes_tx, idle_lanes_rx) = flume::bounded(MAX_STREAM_LANES);
        let (lane_capacity_tx, lane_capacity_rx) = flume::bounded(MAX_STREAM_LANES);
        Self {
            inner,
            idle_lanes_tx,
            idle_lanes_rx,
            lane_capacity_tx,
            lane_capacity_rx,
            open_lanes: AtomicUsize::new(0),
        }
    }

    async fn acquire_lane(&self, deadline: Deadline) -> Result<PooledLane<'_, B>> {
        loop {
            if let Ok(lane) = self.idle_lanes_rx.try_recv() {
                return Ok(PooledLane::new(self, lane));
            }
            if !self.reserve_lane() {
                let idle = self.idle_lanes_rx.recv_async().fuse();
                let capacity = self.lane_capacity_rx.recv_async().fuse();
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
                .open_bi(deadline.remaining("stream acquisition")?)
                .fuse();
            let idle = self.idle_lanes_rx.recv_async().fuse();
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
                (open < MAX_STREAM_LANES).then_some(open + 1)
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
        let previous = self.open_lanes.fetch_sub(1, Ordering::AcqRel);
        assert!(previous > 0, "a removed stream lane must be open");
        let _ = self.lane_capacity_tx.try_send(());
    }
}

impl<'a, B: BackendConnection> PooledLane<'a, B> {
    fn new(connection: &'a PooledConnection<B>, stream: B::Stream) -> Self {
        Self {
            connection,
            stream: Some(stream),
        }
    }

    async fn write_request(&mut self, frame: Vec<u8>, deadline: Deadline) -> Result<()> {
        self.stream
            .as_mut()
            .expect("a checked-out lane must own its stream")
            .write_all(frame, deadline.remaining("request write")?)
            .await?;
        Ok(())
    }

    async fn read_response(&mut self, maximum: usize, deadline: Deadline) -> Result<Vec<u8>> {
        let stream = self
            .stream
            .as_mut()
            .expect("a checked-out lane must own its stream");
        let mut frame = stream
            .read_exact(
                RESPONSE_HEADER_BYTES,
                deadline.remaining("response header read")?,
            )
            .await?;
        let frame_len = Response::frame_len_from_header(&frame)?;
        if frame_len > maximum {
            return Err(Error::ResponseTooLarge { maximum });
        }
        let body_len = frame_len - RESPONSE_HEADER_BYTES;
        if body_len > 0 {
            let body = stream
                .read_exact(body_len, deadline.remaining("response body read")?)
                .await?;
            frame.extend_from_slice(&body);
        }
        Ok(frame)
    }

    fn release(mut self) {
        let stream = self
            .stream
            .take()
            .expect("a released lane must own its stream");
        self.connection.release_lane(stream);
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
                Error::Configuration("request timeout exceeds the platform clock range".into())
            })
    }

    pub(crate) fn remaining(self, operation: &'static str) -> Result<Duration> {
        self.0
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(Error::Timeout { operation })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{backend} QUIC {operation} failed: {message}")]
pub(crate) struct TransportError {
    backend: &'static str,
    operation: &'static str,
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
    fn backend(
        backend: &'static str,
        operation: &'static str,
        error: impl std::fmt::Display,
    ) -> Self {
        Self {
            backend,
            operation,
            message: error.to_string(),
            kind: TransportErrorKind::Transport,
        }
    }

    fn runtime(backend: &'static str, message: &'static str) -> Self {
        Self {
            backend,
            operation: "runtime selection",
            message: message.into(),
            kind: TransportErrorKind::Runtime,
        }
    }

    fn timeout(backend: &'static str, operation: &'static str, timeout: Duration) -> Self {
        Self {
            backend,
            operation,
            message: format!("timed out after {timeout:?}"),
            kind: TransportErrorKind::Timeout,
        }
    }

    #[cfg(any(not(feature = "quic-compio"), not(feature = "quic-quinn")))]
    fn not_compiled(backend: QuicBackend, feature: &'static str) -> Self {
        Self {
            backend: backend.as_str(),
            operation: "selection",
            message: format!("backend was not compiled; enable Cargo feature `{feature}`"),
            kind: TransportErrorKind::Transport,
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
