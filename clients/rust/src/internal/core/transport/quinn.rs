//! Quinn implementation of the client transport boundary.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::internal_protocol::{MAX_REQUEST_FRAME_BYTES, MAX_RESPONSE_FRAME_BYTES};
use futures_util::future::BoxFuture;
use futures_util::lock::Mutex;

use super::{
    BackendConnection, BackendStream, TransportError as LegacyTransportError,
    enforce_client_profile, response_frame_len, validate_response_frame,
};
use crate::internal_core::request::RequestAttempt;
use crate::internal_core::request_engine::{
    RequestBytes, TransportConnection, TransportError, TransportKind, TransportLane,
};
use crate::internal_core::{Backend, Operation};

const BACKEND: Backend = Backend::Quinn;
const QUIC_STREAM_CANCELLATION_CODE: quinn::VarInt = quinn::VarInt::from_u32(1);

pub(super) struct Connection {
    _endpoint: quinn::Endpoint,
    inner: quinn::Connection,
    negotiated_alpn: Vec<u8>,
    _incoming_streams: IncomingStreamRejection,
}

pub(super) struct Stream {
    send: quinn::SendStream,
    receive: quinn::RecvStream,
}

/// Rejects every server-initiated stream, which has no protocol meaning for a
/// client.  Keeping both accept loops active prevents an unsolicited stream
/// from remaining queued behind the client-initiated request lanes.
struct IncomingStreamRejection {
    task: tokio::task::JoinHandle<()>,
}

impl IncomingStreamRejection {
    fn spawn(connection: quinn::Connection) -> Self {
        let task = tokio::spawn(async move {
            futures_util::future::join(
                reject_server_unidirectional(connection.clone()),
                reject_server_bidirectional(connection),
            )
            .await;
        });
        Self { task }
    }
}

impl Drop for IncomingStreamRejection {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn reject_server_unidirectional(connection: quinn::Connection) {
    while let Ok(mut stream) = connection.accept_uni().await {
        let _ = stream.stop(QUIC_STREAM_CANCELLATION_CODE);
    }
}

async fn reject_server_bidirectional(connection: quinn::Connection) {
    while let Ok((mut send, mut receive)) = connection.accept_bi().await {
        let _ = send.reset(QUIC_STREAM_CANCELLATION_CODE);
        let _ = receive.stop(QUIC_STREAM_CANCELLATION_CODE);
    }
}

pub(super) async fn connect(
    address: SocketAddr,
    server_name: &str,
    mut tls: rustls::ClientConfig,
    timeout: Duration,
) -> Result<Connection, LegacyTransportError> {
    tokio::runtime::Handle::try_current().map_err(|_| {
        LegacyTransportError::runtime(BACKEND, "an active Tokio runtime is required")
    })?;
    enforce_client_profile(&mut tls).map_err(|error| {
        LegacyTransportError::backend(BACKEND, Operation::TlsInitialization, error)
    })?;
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls).map_err(|error| {
        LegacyTransportError::backend(BACKEND, Operation::TlsInitialization, error)
    })?;
    let config = quinn::ClientConfig::new(Arc::new(crypto));
    let local_address = SocketAddr::new(
        if address.is_ipv4() {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V6(Ipv6Addr::UNSPECIFIED)
        },
        0,
    );
    let endpoint = quinn::Endpoint::client(local_address).map_err(|error| {
        LegacyTransportError::backend(BACKEND, Operation::EndpointInitialization, error)
    })?;
    let connecting = endpoint
        .connect_with(config, address, server_name)
        .map_err(|error| {
            LegacyTransportError::backend(BACKEND, Operation::ConnectionInitialization, error)
        })?;
    let inner = tokio::time::timeout(timeout, connecting)
        .await
        .map_err(|_| LegacyTransportError::timeout(BACKEND, Operation::ConnectionSetup, timeout))?
        .map_err(|error| LegacyTransportError::backend(BACKEND, Operation::Handshake, error))?;
    let negotiated_alpn = inner
        .handshake_data()
        .and_then(|data| data.downcast::<quinn::crypto::rustls::HandshakeData>().ok())
        .and_then(|data| data.protocol)
        .ok_or_else(|| {
            LegacyTransportError::backend(
                BACKEND,
                Operation::Handshake,
                "server did not negotiate an ALPN protocol",
            )
        })?;
    let _incoming_streams = IncomingStreamRejection::spawn(inner.clone());
    Ok(Connection {
        _endpoint: endpoint,
        inner,
        negotiated_alpn,
        _incoming_streams,
    })
}

/// A QUIC connection that exposes transport-neutral request lanes.
///
/// Each lane owns one client-initiated bidirectional QUIC stream. Send and
/// receive halves are locked independently so pipelined writes continue while
/// a response is being read.
pub struct Transport {
    _endpoint: quinn::Endpoint,
    inner: quinn::Connection,
    lanes: Vec<Arc<Lane>>,
    _incoming_streams: IncomingStreamRejection,
}

/// A transport-neutral QUIC lane.
pub struct Lane {
    send: Arc<Mutex<quinn::SendStream>>,
    receive: Arc<Mutex<quinn::RecvStream>>,
    connection: quinn::Connection,
    closed: Arc<AtomicBool>,
}

impl Transport {
    /// Establishes a strict OpenKache QUIC connection with `lane_count` lanes.
    pub async fn connect(
        address: SocketAddr,
        server_name: &str,
        mut tls: rustls::ClientConfig,
        timeout: Duration,
        lane_count: usize,
    ) -> crate::internal_core::Result<Self> {
        if lane_count == 0 {
            return Err(crate::internal_core::Error::configuration(
                "lane_count",
                "must be greater than zero",
            ));
        }
        enforce_client_profile(&mut tls)?;
        let connection = connect(address, server_name, tls, timeout)
            .await
            .map_err(crate::internal_core::Error::from)?;
        if connection.negotiated_alpn.as_slice() != crate::internal_protocol::ALPN {
            connection.inner.close(0_u32.into(), b"invalid ALPN");
            return Err(crate::internal_core::Error::Connection(
                "server did not negotiate openkache/1".into(),
            ));
        }
        let Connection {
            _endpoint,
            inner,
            _incoming_streams,
            ..
        } = connection;
        let mut lanes = Vec::with_capacity(lane_count);
        for _ in 0..lane_count {
            let (send, receive) = tokio::time::timeout(timeout, inner.open_bi())
                .await
                .map_err(|_| crate::internal_core::Error::Timeout {
                    operation: Operation::StreamOpen,
                })?
                .map_err(|error| crate::internal_core::Error::Connection(error.to_string()))?;
            lanes.push(Arc::new(Lane {
                send: Arc::new(Mutex::new(send)),
                receive: Arc::new(Mutex::new(receive)),
                connection: inner.clone(),
                closed: Arc::new(AtomicBool::new(false)),
            }));
        }
        Ok(Self {
            _endpoint,
            inner,
            lanes,
            _incoming_streams,
        })
    }
}

impl TransportConnection for Transport {
    fn kind(&self) -> TransportKind {
        TransportKind::Quic
    }

    fn lanes(&self) -> Vec<Arc<dyn TransportLane>> {
        self.lanes
            .iter()
            .cloned()
            .map(|lane| lane as Arc<dyn TransportLane>)
            .collect()
    }

    fn close(&self) {
        for lane in &self.lanes {
            lane.close();
        }
        self.inner.close(0_u32.into(), b"client closed");
    }
}

impl TransportLane for Lane {
    fn write_request(
        &self,
        request: RequestBytes,
    ) -> BoxFuture<'static, std::result::Result<(), TransportError>> {
        let send = Arc::clone(&self.send);
        let receive = Arc::clone(&self.receive);
        let closed = Arc::clone(&self.closed);
        Box::pin(async move {
            if closed.load(Ordering::Acquire) {
                return Err(TransportError::before_send("QUIC lane is closed"));
            }
            if request.is_empty() {
                return Err(TransportError::before_send(
                    "request frame must not be empty",
                ));
            }
            if request.len() > MAX_REQUEST_FRAME_BYTES {
                return Err(TransportError::before_send(format!(
                    "request frame exceeds {MAX_REQUEST_FRAME_BYTES} bytes"
                )));
            }
            let mut send = send.lock().await;
            let mut transmitted = false;
            for segment in request.segments() {
                let mut offset = 0;
                while offset < segment.len() {
                    match send.write(&segment[offset..]).await {
                        Ok(written) if written > 0 => {
                            transmitted = true;
                            offset += written;
                        }
                        Ok(_) => {
                            retire_after_write_error(&mut send, &receive, &closed);
                            return Err(if transmitted {
                                TransportError::after_send("QUIC write made no progress")
                            } else {
                                TransportError::before_send("QUIC write made no progress")
                            });
                        }
                        Err(error) => {
                            retire_after_write_error(&mut send, &receive, &closed);
                            return Err(if transmitted {
                                TransportError::after_send(error.to_string())
                            } else {
                                TransportError::before_send(error.to_string())
                            });
                        }
                    }
                }
            }
            Ok(())
        })
    }

    fn read_response(
        &self,
        maximum: usize,
    ) -> BoxFuture<'static, std::result::Result<Vec<u8>, TransportError>> {
        let receive = Arc::clone(&self.receive);
        let send = Arc::clone(&self.send);
        let connection = self.connection.clone();
        let closed = Arc::clone(&self.closed);
        Box::pin(async move {
            if closed.load(Ordering::Acquire) {
                return Err(TransportError::after_send("QUIC lane is closed"));
            }
            let maximum = maximum.min(MAX_RESPONSE_FRAME_BYTES);
            if maximum == 0 {
                return Err(TransportError::before_send(
                    "response maximum must be greater than zero",
                ));
            }
            let mut receive = receive.lock().await;
            let mut header = Vec::with_capacity(32);
            let frame_len = loop {
                let mut byte = [0_u8; 1];
                if let Err(error) = receive.read_exact(&mut byte).await {
                    retire_lane(&mut receive, &send, &closed);
                    return Err(TransportError::after_send(error.to_string()));
                }
                header.push(byte[0]);
                match response_frame_len(&header, maximum) {
                    Ok(Some(length)) => break length,
                    Ok(None) => continue,
                    Err(error) => {
                        fatal_protocol(
                            &connection,
                            &mut receive,
                            &send,
                            &closed,
                            b"malformed response frame",
                        );
                        return Err(TransportError::after_send(error));
                    }
                }
            };
            let header_len = header.len();
            if header_len > frame_len {
                fatal_protocol(
                    &connection,
                    &mut receive,
                    &send,
                    &closed,
                    b"malformed response frame",
                );
                return Err(TransportError::after_send(
                    "response header exceeds complete frame",
                ));
            }
            let mut frame = header;
            frame
                .try_reserve_exact(frame_len.saturating_sub(header_len))
                .map_err(|_| {
                    retire_lane(&mut receive, &send, &closed);
                    TransportError::after_send("response allocation failed")
                })?;
            frame.resize(frame_len, 0);
            if header_len < frame_len {
                if let Err(error) = receive.read_exact(&mut frame[header_len..]).await {
                    retire_lane(&mut receive, &send, &closed);
                    return Err(TransportError::after_send(error.to_string()));
                }
            }
            if let Err(error) = validate_response_frame(&frame, frame_len) {
                fatal_protocol(
                    &connection,
                    &mut receive,
                    &send,
                    &closed,
                    b"malformed response frame",
                );
                return Err(TransportError::after_send(error));
            }
            Ok(frame)
        })
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        // QUIC cancellation is directional: RESET the request direction and
        // STOP the response direction without tearing down the connection.
        reset_send(&self.send, 0_u32.into());
        stop_receive(&self.receive, 0_u32.into());
    }
}

fn retire_lane(
    receive: &mut quinn::RecvStream,
    send: &Arc<Mutex<quinn::SendStream>>,
    closed: &Arc<AtomicBool>,
) {
    if closed.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = receive.stop(QUIC_STREAM_CANCELLATION_CODE);
    reset_send(send, QUIC_STREAM_CANCELLATION_CODE);
}

fn retire_after_write_error(
    send: &mut quinn::SendStream,
    receive: &Arc<Mutex<quinn::RecvStream>>,
    closed: &Arc<AtomicBool>,
) {
    // The write future owns the send lock, so reset it directly before marking
    // the lane closed. STOP_SENDING may use the receive lock asynchronously,
    // but it is scheduled before the caller can retire this lane.
    let _ = send.reset(QUIC_STREAM_CANCELLATION_CODE);
    stop_receive(receive, QUIC_STREAM_CANCELLATION_CODE);
    closed.store(true, Ordering::Release);
}

fn fatal_protocol(
    connection: &quinn::Connection,
    receive: &mut quinn::RecvStream,
    send: &Arc<Mutex<quinn::SendStream>>,
    closed: &Arc<AtomicBool>,
    reason: &[u8],
) {
    retire_lane(receive, send, closed);
    connection.close(QUIC_STREAM_CANCELLATION_CODE, reason);
}

fn reset_send(send: &Arc<Mutex<quinn::SendStream>>, code: quinn::VarInt) {
    if let Some(mut send) = send.try_lock() {
        let _ = send.reset(code);
        return;
    }
    let Some(handle) = tokio::runtime::Handle::try_current().ok() else {
        return;
    };
    let send = Arc::clone(send);
    handle.spawn(async move {
        let mut send = send.lock().await;
        let _ = send.reset(code);
    });
}

fn stop_receive(receive: &Arc<Mutex<quinn::RecvStream>>, code: quinn::VarInt) {
    if let Some(mut receive) = receive.try_lock() {
        let _ = receive.stop(code);
        return;
    }
    let Some(handle) = tokio::runtime::Handle::try_current().ok() else {
        return;
    };
    let receive = Arc::clone(receive);
    handle.spawn(async move {
        let mut receive = receive.lock().await;
        let _ = receive.stop(code);
    });
}

impl BackendConnection for Connection {
    type Stream = Stream;

    fn negotiated_alpn(&self) -> Option<&[u8]> {
        Some(&self.negotiated_alpn)
    }

    async fn open_bi(&self, timeout: Duration) -> Result<Self::Stream, LegacyTransportError> {
        let (send, receive) = tokio::time::timeout(timeout, self.inner.open_bi())
            .await
            .map_err(|_| LegacyTransportError::timeout(BACKEND, Operation::StreamOpen, timeout))?
            .map_err(|error| {
                LegacyTransportError::backend(BACKEND, Operation::StreamOpen, error)
            })?;
        Ok(Stream { send, receive })
    }

    fn close(&self) {
        self.inner.close(0_u32.into(), b"client closed");
    }
}

impl BackendStream for Stream {
    async fn write_request(
        &mut self,
        request: RequestAttempt,
        timeout: Duration,
    ) -> Result<(), LegacyTransportError> {
        let result = tokio::time::timeout(timeout, async {
            for segment in request.segments() {
                self.send.write_all(segment).await?;
            }
            Ok::<(), quinn::WriteError>(())
        })
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.retire();
                Err(LegacyTransportError::backend(
                    BACKEND,
                    Operation::StreamWrite,
                    error,
                ))
            }
            Err(_) => {
                self.retire();
                Err(LegacyTransportError::timeout(
                    BACKEND,
                    Operation::StreamWrite,
                    timeout,
                ))
            }
        }
    }

    async fn read_byte(&mut self, timeout: Duration) -> Result<u8, LegacyTransportError> {
        let mut byte = [0];
        match tokio::time::timeout(timeout, self.receive.read_exact(&mut byte)).await {
            Ok(Ok(())) => Ok(byte[0]),
            Ok(Err(error)) => {
                self.retire();
                Err(LegacyTransportError::backend(
                    BACKEND,
                    Operation::StreamRead,
                    error,
                ))
            }
            Err(_) => {
                self.retire();
                Err(LegacyTransportError::timeout(
                    BACKEND,
                    Operation::StreamRead,
                    timeout,
                ))
            }
        }
    }

    async fn read_exact(
        &mut self,
        length: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, LegacyTransportError> {
        let mut bytes = vec![0; length];
        match tokio::time::timeout(timeout, self.receive.read_exact(&mut bytes)).await {
            Ok(Ok(())) => Ok(bytes),
            Ok(Err(error)) => {
                self.retire();
                Err(LegacyTransportError::backend(
                    BACKEND,
                    Operation::StreamRead,
                    error,
                ))
            }
            Err(_) => {
                self.retire();
                Err(LegacyTransportError::timeout(
                    BACKEND,
                    Operation::StreamRead,
                    timeout,
                ))
            }
        }
    }
}

impl Stream {
    fn retire(&mut self) {
        let _ = self.send.reset(QUIC_STREAM_CANCELLATION_CODE);
        let _ = self.receive.stop(QUIC_STREAM_CANCELLATION_CODE);
    }
}
