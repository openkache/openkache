//! QUIC backend boundary used by the OpenKache protocol server.

use std::future::Future;
#[cfg(feature = "quic-quiche")]
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::CertificateDer;

use crate::QuicBackend;
use crate::network_runtime;
use openkache_protocol::{OwnedRange, ResponseParts};
#[cfg(feature = "quic-quiche")]
use openkache_protocol::ResponseSegment;

#[path = "transport/request.rs"]
mod request;
pub(crate) use request::{RequestBudget, RequestFrame, StreamReadError};
pub(crate) use request::{RequestByteStream, read_buffered_request};

#[path = "transport/tls.rs"]
mod tls;
pub(crate) use tls::ServerTlsConfig;
#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
use tls::rustls_config;

#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
#[path = "transport/response.rs"]
mod response;
#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
use response::write_response_segments;

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
}

/// Receive half of one request stream.
pub(super) trait ReceiveStream {
    fn read_request(
        &mut self,
        maximum: usize,
        maximum_value: usize,
        timeout: Duration,
        budget: &RequestBudget,
        frame_layout_provider: &dyn crate::protocol::FrameLayoutProvider,
    ) -> impl Future<Output = Result<RequestFrame, StreamReadError>>;
}

/// Send half of one response stream.
pub(super) trait SendStream {
    fn write_response(
        &mut self,
        parts: ResponseParts,
        timeout: Duration,
    ) -> impl Future<Output = Result<(), TransportError>>;
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

#[cfg(feature = "quic-quinn")]
#[path = "transport/quinn.rs"]
mod quinn_backend;

#[cfg(feature = "quic-noq")]
#[path = "transport/noq.rs"]
mod noq_backend;

#[cfg(feature = "quic-quiche")]
#[path = "transport/quiche.rs"]
mod quiche_backend;
