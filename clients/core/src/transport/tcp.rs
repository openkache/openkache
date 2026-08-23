//! Strict TLS-over-TCP binding for one transport-neutral request lane.
//!
//! The binding owns TLS 1.3 setup and opaque frame delimiting only. It never
//! interprets operation payloads, so value parsing remains in the protocol and
//! client API layers.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::lock::Mutex;
use openkache_protocol::{
    MAX_REQUEST_FRAME_BYTES, MAX_RESPONSE_FRAME_BYTES, ResponseHeaderBytes, ResponseParts,
};
use rustls::crypto::SupportedKxGroup;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use super::{
    ClientConnection, ClientLane, Deadline, RequestBudget, RequestBudgetPermit,
    enforce_client_profile, response_frame_len, validate_response_frame,
};
use crate::config::PQ_GROUP;
use crate::request::RequestAttempt;
use crate::request_engine::{
    RequestBytes, TransportConnection, TransportError, TransportKind, TransportLane,
};
use crate::{Error, Operation, Result};

type ClientTlsStream = TlsStream<TcpStream>;

/// One TLS-over-TCP connection containing its single ordered lane.
pub struct TlsTcpTransport {
    lane: Arc<TlsTcpLane>,
}

/// Compatibility spelling for the TLS-over-TCP transport binding.
pub type TcpTransport = TlsTcpTransport;

/// One TLS-over-TCP request lane.
///
/// TCP supplies one ordered byte stream, so a connection exposes exactly one
/// lane. The TLS stream is split into independently locked halves to permit
/// pipelined writes while a complete response frame is awaited.
pub struct TlsTcpLane {
    send: Arc<Mutex<WriteHalf<ClientTlsStream>>>,
    receive: Arc<Mutex<ReadHalf<ClientTlsStream>>>,
    closed: Arc<AtomicBool>,
    fatal: Arc<AtomicBool>,
}

/// Compatibility spelling for the TLS-over-TCP lane.
pub type TcpLane = TlsTcpLane;

impl TlsTcpTransport {
    /// Establishes the one-lane TLS-over-TCP OpenKache profile.
    ///
    /// The supplied configuration must offer only `openkache/1` and the
    /// mandatory `X25519MLKEM768` group. The handshake is revalidated before
    /// application bytes are exposed, so a caller cannot accidentally use a
    /// plaintext or classical-only fallback.
    pub async fn connect(
        address: SocketAddr,
        server_name: &str,
        tls: rustls::ClientConfig,
        timeout: Duration,
    ) -> crate::Result<Self> {
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(crate::Error::Connection(
                "an active Tokio runtime is required".into(),
            ));
        }
        let mut tls = tls;
        enforce_client_profile(&mut tls)?;
        let server_name = rustls::pki_types::ServerName::try_from(server_name.to_owned())
            .map_err(|error| crate::Error::configuration("server_name", error.to_string()))?;
        let socket = tokio::time::timeout(timeout, TcpStream::connect(address))
            .await
            .map_err(|_| crate::Error::Timeout {
                operation: crate::Operation::ConnectionSetup,
            })?
            .map_err(crate::Error::io)?;
        socket.set_nodelay(true).map_err(crate::Error::io)?;
        let connector = tokio_rustls::TlsConnector::from(Arc::new(tls));
        let stream = tokio::time::timeout(timeout, connector.connect(server_name, socket))
            .await
            .map_err(|_| crate::Error::Timeout {
                operation: crate::Operation::Handshake,
            })?
            .map_err(crate::Error::io)?;
        validate_handshake(&stream)?;
        let (receive, send) = tokio::io::split(stream);
        Ok(Self {
            lane: Arc::new(TlsTcpLane {
                send: Arc::new(Mutex::new(send)),
                receive: Arc::new(Mutex::new(receive)),
                closed: Arc::new(AtomicBool::new(false)),
                fatal: Arc::new(AtomicBool::new(false)),
            }),
        })
    }

    /// Returns the negotiated ALPN protocol.
    pub(crate) fn negotiated_alpn(&self) -> Option<&[u8]> {
        Some(openkache_protocol::ALPN)
    }
}

impl TlsTcpLane {
    /// Reads one response while reserving the payload budget before allocation.
    ///
    /// The low-level [`TransportLane`] API intentionally returns one complete
    /// frame for callers that own their own resource policy. The request
    /// engine uses this variant so a declared response body cannot be read
    /// into an unbudgeted frame before the shared aggregate byte permit is
    /// acquired.
    async fn read_response_with_budget(
        &self,
        maximum: usize,
        budget: &RequestBudget,
        timeout: Duration,
    ) -> Result<(ResponseParts, RequestBudgetPermit)> {
        if self.fatal.load(Ordering::Acquire) {
            return Err(Error::Connection("TLS-over-TCP lane has failed".into()));
        }
        let maximum = maximum.min(MAX_RESPONSE_FRAME_BYTES);
        if maximum == 0 {
            return Err(Error::configuration(
                "response maximum",
                "must be greater than zero",
            ));
        }
        let mut receive = self.receive.lock().await;
        let mut header = ResponseHeaderBytes::new();
        let frame_len = loop {
            let mut byte = [0_u8; 1];
            if let Err(error) = receive.read_exact(&mut byte).await {
                close_after_transport_error(&self.send, &self.closed, &self.fatal);
                return Err(Error::Connection(error.to_string()));
            }
            header.push(byte[0]).map_err(|error| {
                close_after_protocol_error(&self.send, &self.closed, &self.fatal);
                Error::protocol(error)
            })?;
            match response_frame_len(header.as_slice(), maximum) {
                Ok(Some(length)) => break length,
                Ok(None) => continue,
                Err(error) => {
                    close_after_protocol_error(&self.send, &self.closed, &self.fatal);
                    return Err(Error::Connection(error));
                }
            }
        };
        let header_len = header.len();
        if header_len > frame_len {
            close_after_protocol_error(&self.send, &self.closed, &self.fatal);
            return Err(Error::Connection(
                "response header exceeds complete frame".into(),
            ));
        }
        let payload_len = frame_len - header_len;
        let permit = match budget.acquire(payload_len, timeout).await {
            Ok(permit) => permit,
            Err(error) => {
                // The body is still unread, so this ordered TCP lane cannot
                // be returned to the pool after an admission failure.
                self.close();
                return Err(error);
            }
        };
        let mut payload = Vec::new();
        if payload.try_reserve_exact(payload_len).is_err() {
            drop(permit);
            close_after_protocol_error(&self.send, &self.closed, &self.fatal);
            return Err(Error::Connection("response allocation failed".into()));
        }
        payload.resize(payload_len, 0);
        if payload_len > 0
            && let Err(error) = receive.read_exact(&mut payload).await
        {
            drop(permit);
            close_after_transport_error(&self.send, &self.closed, &self.fatal);
            return Err(Error::Connection(error.to_string()));
        }
        let response = ResponseParts::decode(header, payload).map_err(|error| {
            close_after_protocol_error(&self.send, &self.closed, &self.fatal);
            Error::protocol(error)
        })?;
        Ok((response, permit))
    }
}

impl TransportConnection for TlsTcpTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::TlsTcp
    }

    fn lanes(&self) -> Vec<Arc<dyn TransportLane>> {
        vec![Arc::clone(&self.lane) as Arc<dyn TransportLane>]
    }

    fn close(&self) {
        self.lane.close();
    }
}

impl TransportLane for TlsTcpLane {
    fn write_request(
        &self,
        request: RequestBytes,
    ) -> BoxFuture<'static, std::result::Result<(), TransportError>> {
        let send = Arc::clone(&self.send);
        let closed = Arc::clone(&self.closed);
        let fatal = Arc::clone(&self.fatal);
        Box::pin(async move {
            if closed.load(Ordering::Acquire) {
                return Err(TransportError::before_send("TLS-over-TCP lane is closed"));
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
                            closed.store(true, Ordering::Release);
                            fatal.store(true, Ordering::Release);
                            let _ = send.shutdown().await;
                            return Err(if transmitted {
                                TransportError::after_send("TLS write made no progress")
                            } else {
                                TransportError::before_send("TLS write made no progress")
                            });
                        }
                        Err(error) => {
                            closed.store(true, Ordering::Release);
                            fatal.store(true, Ordering::Release);
                            let _ = send.shutdown().await;
                            return Err(if transmitted {
                                TransportError::after_send(error.to_string())
                            } else {
                                TransportError::before_send(error.to_string())
                            });
                        }
                    }
                }
            }
            if let Err(error) = send.flush().await {
                closed.store(true, Ordering::Release);
                fatal.store(true, Ordering::Release);
                let _ = send.shutdown().await;
                return Err(TransportError::after_send(error.to_string()));
            }
            if closed.load(Ordering::Acquire) {
                let _ = send.shutdown().await;
                return Err(TransportError::after_send("TLS-over-TCP lane is closing"));
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
        let closed = Arc::clone(&self.closed);
        let fatal = Arc::clone(&self.fatal);
        Box::pin(async move {
            if fatal.load(Ordering::Acquire) {
                return Err(TransportError::after_send("TLS-over-TCP lane has failed"));
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
                    close_after_transport_error(&send, &closed, &fatal);
                    return Err(TransportError::after_send(error.to_string()));
                }
                header.push(byte[0]);
                match response_frame_len(&header, maximum) {
                    Ok(Some(length)) => break length,
                    Ok(None) => continue,
                    Err(error) => {
                        close_after_protocol_error(&send, &closed, &fatal);
                        return Err(TransportError::after_send(error));
                    }
                }
            };
            let header_len = header.len();
            if header_len > frame_len {
                close_after_protocol_error(&send, &closed, &fatal);
                return Err(TransportError::after_send(
                    "response header exceeds complete frame",
                ));
            }
            let mut frame = header;
            frame
                .try_reserve_exact(frame_len.saturating_sub(header_len))
                .map_err(|_| {
                    close_after_protocol_error(&send, &closed, &fatal);
                    TransportError::after_send("response allocation failed")
                })?;
            frame.resize(frame_len, 0);
            if header_len < frame_len {
                if let Err(error) = receive.read_exact(&mut frame[header_len..]).await {
                    close_after_transport_error(&send, &closed, &fatal);
                    return Err(TransportError::after_send(error.to_string()));
                }
            }
            if let Err(error) = validate_response_frame(&frame, frame_len) {
                close_after_protocol_error(&send, &closed, &fatal);
                return Err(TransportError::after_send(error));
            }
            Ok(frame)
        })
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        // A TLS close-notify cleanly half-closes the request direction while
        // retaining the receive half so already admitted responses can drain.
        shutdown_send(&self.send);
    }
}

fn close_after_protocol_error(
    send: &Arc<Mutex<WriteHalf<ClientTlsStream>>>,
    closed: &Arc<AtomicBool>,
    fatal: &Arc<AtomicBool>,
) {
    fatal.store(true, Ordering::Release);
    if closed.swap(true, Ordering::AcqRel) {
        return;
    }
    shutdown_send(send);
}

fn close_after_transport_error(
    send: &Arc<Mutex<WriteHalf<ClientTlsStream>>>,
    closed: &Arc<AtomicBool>,
    fatal: &Arc<AtomicBool>,
) {
    close_after_protocol_error(send, closed, fatal);
}

fn shutdown_send(send: &Arc<Mutex<WriteHalf<ClientTlsStream>>>) {
    let send = Arc::clone(send);
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            let mut send = send.lock().await;
            let _ = send.shutdown().await;
        });
    }
}

fn validate_handshake(stream: &ClientTlsStream) -> crate::Result<()> {
    let (_, connection) = stream.get_ref();
    if connection.protocol_version() != Some(rustls::ProtocolVersion::TLSv1_3) {
        return Err(crate::Error::Connection(
            "server did not negotiate TLS 1.3".into(),
        ));
    }
    if connection.alpn_protocol() != Some(openkache_protocol::ALPN) {
        return Err(crate::Error::Connection(
            "server did not negotiate openkache/1".into(),
        ));
    }
    if connection
        .negotiated_key_exchange_group()
        .map(SupportedKxGroup::name)
        != Some(PQ_GROUP)
    {
        return Err(crate::Error::Connection(
            "server did not negotiate X25519MLKEM768".into(),
        ));
    }
    Ok(())
}

/// Internal request-engine connection wrapper for the one-lane TCP profile.
///
/// The public [`TransportConnection`] surface permits pipelined writes, while
/// the legacy high-level request API expects one request/response exchange per
/// acquired lane.  A single async gate preserves that API's correlation
/// invariant without inventing a second TCP lane.
pub(crate) struct Connection {
    inner: TlsTcpTransport,
    gate: Mutex<()>,
    budget: RequestBudget,
}

pub(crate) struct Lane<'a> {
    connection: &'a Connection,
    _gate: futures_util::lock::MutexGuard<'a, ()>,
    response_permit: Option<RequestBudgetPermit>,
}

impl ClientConnection for Connection {
    type Lane<'a> = Lane<'a>;

    async fn connect(
        address: SocketAddr,
        server_name: &str,
        tls: rustls::ClientConfig,
        timeout: Duration,
        _max_stream_lanes: usize,
        budget: RequestBudget,
    ) -> Result<Self> {
        Ok(Self {
            inner: TlsTcpTransport::connect(address, server_name, tls, timeout).await?,
            gate: Mutex::new(()),
            budget,
        })
    }

    async fn acquire_lane(&self, deadline: Deadline) -> Result<Self::Lane<'_>> {
        let remaining = deadline.remaining(Operation::StreamAcquisition)?;
        let gate = tokio::time::timeout(remaining, self.gate.lock())
            .await
            .map_err(|_| Error::Timeout {
                operation: Operation::StreamAcquisition,
            })?;
        Ok(Lane {
            connection: self,
            _gate: gate,
            response_permit: None,
        })
    }

    fn negotiated_alpn(&self) -> Option<&[u8]> {
        self.inner.negotiated_alpn()
    }

    async fn timeout<F: std::future::Future>(
        duration: Duration,
        future: F,
    ) -> Result<Option<F::Output>> {
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(Error::Connection(
                "an active Tokio runtime is required".into(),
            ));
        }
        Ok(tokio::time::timeout(duration, future).await.ok())
    }

    fn close(&self) {
        self.inner.close();
    }
}

impl ClientLane for Lane<'_> {
    async fn write_request(&mut self, request: RequestAttempt, timeout: Duration) -> Result<()> {
        let frame = match request {
            RequestAttempt::Once(frame) => frame,
            RequestAttempt::Replay(frame) => frame.clone_owned().map_err(Error::protocol)?,
        };
        let request = RequestBytes::new(frame);
        let _permit = self.connection.budget.try_reserve(request.len())?;
        match tokio::time::timeout(timeout, self.connection.inner.lane.write_request(request)).await
        {
            Ok(result) => result.map_err(|error| Error::Connection(error.to_string())),
            Err(_) => {
                self.connection.inner.close();
                Err(Error::Timeout {
                    operation: Operation::RequestWrite,
                })
            }
        }
    }

    async fn read_response(
        &mut self,
        maximum: usize,
        deadline: Deadline,
    ) -> Result<openkache_protocol::ResponseParts> {
        let remaining = deadline.remaining(Operation::ResponseBodyRead)?;
        let (decoded, permit) = match tokio::time::timeout(
            remaining,
            self.connection.inner.lane.read_response_with_budget(
                maximum,
                &self.connection.budget,
                remaining,
            ),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                self.connection.inner.close();
                return Err(Error::Timeout {
                    operation: Operation::ResponseBodyRead,
                });
            }
        };
        self.response_permit = Some(permit);
        Ok(decoded)
    }

    fn take_response_permit(&mut self) -> Option<RequestBudgetPermit> {
        self.response_permit.take()
    }

    fn release(self) {}
}
