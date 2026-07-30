//! QUIC backend boundary and persistent stream-lane pool.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::{RESPONSE_HEADER_BYTES, Response};

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
    ) -> impl Future<Output = Result<Self>>;

    fn acquire_lane(&self, deadline: Deadline) -> impl Future<Output = Result<Self::Lane<'_>>>;

    fn timeout<F: Future>(
        duration: Duration,
        future: F,
    ) -> impl Future<Output = Result<Option<F::Output>>>;

    fn close(&self);
}

pub(crate) trait ClientLane {
    fn write_request(
        &mut self,
        frame: Vec<u8>,
        timeout: Duration,
    ) -> impl Future<Output = Result<()>>;

    fn read_response(
        &mut self,
        maximum: usize,
        deadline: Deadline,
    ) -> impl Future<Output = Result<Vec<u8>>>;

    fn release(self);
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
            ) -> Result<Self> {
                $module::connect(address, server_name, tls, timeout)
                    .await
                    .map(|connection| Self(PooledConnection::new(connection, max_stream_lanes)))
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
            async fn write_request(&mut self, frame: Vec<u8>, timeout: Duration) -> Result<()> {
                self.0.write_request(frame, timeout).await
            }

            async fn read_response(
                &mut self,
                maximum: usize,
                deadline: Deadline,
            ) -> Result<Vec<u8>> {
                self.0.read_response(maximum, deadline).await
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
    type Stream: BackendStream;

    fn open_bi(
        &self,
        timeout: Duration,
    ) -> impl Future<Output = std::result::Result<Self::Stream, TransportError>>;

    fn close(&self);
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
    max_stream_lanes: usize,
}

struct PooledLane<'a, B: BackendConnection> {
    connection: &'a PooledConnection<B>,
    stream: Option<B::Stream>,
}

impl<B: BackendConnection> PooledConnection<B> {
    fn new(inner: B, max_stream_lanes: usize) -> Self {
        let (idle_lanes_tx, idle_lanes_rx) = flume::bounded(max_stream_lanes);
        let (lane_capacity_tx, lane_capacity_rx) = flume::bounded(max_stream_lanes);
        Self {
            inner,
            idle_lanes_tx,
            idle_lanes_rx,
            lane_capacity_tx,
            lane_capacity_rx,
            open_lanes: AtomicUsize::new(0),
            max_stream_lanes,
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
                .open_bi(deadline.remaining(Operation::StreamAcquisition)?)
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

    async fn write_request(&mut self, frame: Vec<u8>, timeout: Duration) -> Result<()> {
        self.stream
            .as_mut()
            .expect("a checked-out lane must own its stream")
            .write_all(frame, timeout)
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
                deadline.remaining(Operation::ResponseHeaderRead)?,
            )
            .await?;
        let frame_len = Response::frame_len_from_header(&frame).map_err(Error::protocol)?;
        if frame_len > maximum {
            return Err(Error::ResponseTooLarge { maximum });
        }
        let body_len = frame_len - RESPONSE_HEADER_BYTES;
        if body_len > 0 {
            let body = stream
                .read_exact(body_len, deadline.remaining(Operation::ResponseBodyRead)?)
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
