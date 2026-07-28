//! Compio QUIC transport for the OpenKache client.

use std::cell::Cell;
use std::net::SocketAddr;
use std::sync::Arc;

use compio::BufResult;
use compio::io::{AsyncReadExt, AsyncWriteExt};
use compio_quic::{Endpoint, RecvStream, SendStream};
use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::{RESPONSE_HEADER_BYTES, Response};

use crate::{Error, Result};

const MAX_STREAM_LANES: usize = 256;

/// Keeps the Compio endpoint alive alongside its QUIC connection.
pub(crate) struct Connection {
    _endpoint: Endpoint,
    inner: compio_quic::Connection,
    idle_lanes_tx: flume::Sender<BidiStream>,
    idle_lanes_rx: flume::Receiver<BidiStream>,
    lane_capacity_tx: flume::Sender<()>,
    lane_capacity_rx: flume::Receiver<()>,
    open_lanes: Cell<usize>,
}

/// A Compio bidirectional QUIC stream.
pub(crate) struct BidiStream {
    send: SendStream,
    receive: RecvStream,
}

/// Open a QUIC connection to `addr`, authenticating as `server_name` with the
/// given TLS configuration.
pub(crate) async fn connect(
    addr: SocketAddr,
    server_name: &str,
    tls: rustls::ClientConfig,
) -> Result<Connection> {
    let crypto = compio_quic::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|error| Error::Connection(error.to_string()))?;
    let config = compio_quic::ClientConfig::new(Arc::new(crypto));
    let local_address = if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let endpoint = Endpoint::client(local_address).await?;
    let inner = endpoint
        .connect(addr, server_name, Some(config))
        .map_err(|error| Error::Connection(error.to_string()))?
        .await
        .map_err(|error| Error::Connection(error.to_string()))?;
    let (idle_lanes_tx, idle_lanes_rx) = flume::bounded(MAX_STREAM_LANES);
    let (lane_capacity_tx, lane_capacity_rx) = flume::bounded(MAX_STREAM_LANES);
    Ok(Connection {
        _endpoint: endpoint,
        inner,
        idle_lanes_tx,
        idle_lanes_rx,
        lane_capacity_tx,
        lane_capacity_rx,
        open_lanes: Cell::new(0),
    })
}

impl Connection {
    /// Acquires an idle lane, growing the connection-local pool when needed.
    pub(crate) async fn acquire_lane(&self) -> Result<BidiStream> {
        loop {
            if let Ok(lane) = self.idle_lanes_rx.try_recv() {
                return Ok(lane);
            }
            if self.open_lanes.get() >= MAX_STREAM_LANES {
                let idle = self.idle_lanes_rx.recv_async().fuse();
                let capacity = self.lane_capacity_rx.recv_async().fuse();
                pin_mut!(idle, capacity);
                select! {
                    lane = idle => {
                        return lane
                            .map_err(|_| Error::Connection("stream lane pool closed".into()));
                    }
                    _ = capacity => continue,
                }
            }

            let reservation = LaneReservation::new(self);
            let opening = self.inner.open_bi_wait().fuse();
            let idle = self.idle_lanes_rx.recv_async().fuse();
            pin_mut!(opening, idle);
            select! {
                opened = opening => {
                    let (send, receive) =
                        opened.map_err(|error| Error::Connection(error.to_string()))?;
                    reservation.commit();
                    return Ok(BidiStream { send, receive });
                }
                lane = idle => {
                    return lane
                        .map_err(|_| Error::Connection("stream lane pool closed".into()));
                }
            }
        }
    }

    /// Returns a healthy lane to the connection-local pool.
    pub(crate) fn release_lane(&self, lane: BidiStream) {
        if self.idle_lanes_tx.try_send(lane).is_err() {
            self.remove_lane();
        }
    }

    /// Removes a failed lane so a later request can replace it.
    pub(crate) fn discard_lane(&self, lane: BidiStream) {
        drop(lane);
        self.remove_lane();
    }

    fn remove_lane(&self) {
        self.open_lanes.set(
            self.open_lanes
                .get()
                .checked_sub(1)
                .expect("a removed stream lane must be open"),
        );
        let _ = self.lane_capacity_tx.try_send(());
    }
}

struct LaneReservation<'a> {
    connection: &'a Connection,
    active: bool,
}

impl<'a> LaneReservation<'a> {
    fn new(connection: &'a Connection) -> Self {
        connection.open_lanes.set(connection.open_lanes.get() + 1);
        Self {
            connection,
            active: true,
        }
    }

    fn commit(mut self) {
        self.active = false;
    }
}

impl Drop for LaneReservation<'_> {
    fn drop(&mut self) {
        if self.active {
            self.connection.remove_lane();
        }
    }
}

impl BidiStream {
    /// Writes one complete request without closing this reusable lane.
    pub(crate) async fn write_request(&mut self, frame: Vec<u8>) -> Result<()> {
        let BufResult(result, _) = self.send.write_all(frame).await;
        Ok(result?)
    }

    /// Reads exactly one length-delimited response up to `maximum` bytes.
    pub(crate) async fn read_response(&mut self, maximum: usize) -> Result<Vec<u8>> {
        let BufResult(result, mut frame) = self
            .receive
            .read_exact(Vec::with_capacity(RESPONSE_HEADER_BYTES))
            .await;
        result?;
        let frame_len = Response::frame_len_from_header(&frame)?;
        if frame_len > maximum {
            return Err(Error::ResponseTooLarge { maximum });
        }
        let body_len = frame_len - RESPONSE_HEADER_BYTES;
        if body_len > 0 {
            let BufResult(result, body) =
                self.receive.read_exact(Vec::with_capacity(body_len)).await;
            result?;
            frame.extend_from_slice(&body);
        }
        Ok(frame)
    }
}
