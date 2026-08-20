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
use openkache_protocol::{MAX_REQUEST_FRAME_BYTES, MAX_RESPONSE_FRAME_BYTES};
use rustls::crypto::SupportedKxGroup;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use super::{enforce_client_profile, response_frame_len, validate_response_frame};
use crate::config::PQ_GROUP;
use crate::request_engine::{
    RequestBytes, TransportConnection, TransportError, TransportKind, TransportLane,
};

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
