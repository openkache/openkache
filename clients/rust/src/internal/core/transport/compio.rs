//! Compio-QUIC implementation of the client transport boundary.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use compio::BufResult;
use compio::buf::IoVectoredBuf;
use compio::io::{AsyncReadExt, AsyncWriteExt};

use super::{
    BackendConnection, BackendStream, TransportError as LegacyTransportError,
    enforce_client_profile,
};
use crate::internal_core::request::RequestAttempt;
use crate::internal_core::{Backend, Operation};

const BACKEND: Backend = Backend::Compio;
const QUIC_STREAM_CANCELLATION_CODE: compio_quic::VarInt = compio_quic::VarInt::from_u32(1);

pub(super) struct Connection {
    _endpoint: compio_quic::Endpoint,
    inner: compio_quic::Connection,
    negotiated_alpn: Vec<u8>,
    _incoming_streams: IncomingStreamRejection,
}

pub(super) struct Stream {
    send: compio_quic::SendStream,
    receive: compio_quic::RecvStream,
}

/// Rejects every server-initiated stream, which has no protocol meaning for a
/// client. Both stream directions are accepted so an unsolicited stream
/// cannot remain queued behind the client-initiated request lanes.
struct IncomingStreamRejection {
    _task: compio::runtime::JoinHandle<()>,
}

impl IncomingStreamRejection {
    fn spawn(connection: compio_quic::Connection) -> Self {
        let task = compio::runtime::spawn(async move {
            futures_util::future::join(
                reject_server_unidirectional(connection.clone()),
                reject_server_bidirectional(connection),
            )
            .await;
        });
        Self { _task: task }
    }
}

async fn reject_server_unidirectional(connection: compio_quic::Connection) {
    while let Ok(mut stream) = connection.accept_uni().await {
        let _ = stream.stop(QUIC_STREAM_CANCELLATION_CODE);
    }
}

async fn reject_server_bidirectional(connection: compio_quic::Connection) {
    while let Ok((mut send, mut receive)) = connection.accept_bi().await {
        let _ = send.reset(QUIC_STREAM_CANCELLATION_CODE);
        let _ = receive.stop(QUIC_STREAM_CANCELLATION_CODE);
    }
}

impl IoVectoredBuf for RequestAttempt {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        self.segments()
    }
}

pub(super) async fn connect(
    address: SocketAddr,
    server_name: &str,
    mut tls: rustls::ClientConfig,
    timeout: Duration,
) -> Result<Connection, LegacyTransportError> {
    if compio::runtime::Runtime::try_current().is_none() {
        return Err(LegacyTransportError::runtime(
            BACKEND,
            "an active Compio runtime is required",
        ));
    }
    enforce_client_profile(&mut tls).map_err(|error| {
        LegacyTransportError::backend(BACKEND, Operation::TlsInitialization, error)
    })?;
    let crypto = compio_quic::crypto::rustls::QuicClientConfig::try_from(tls).map_err(|error| {
        LegacyTransportError::backend(BACKEND, Operation::TlsInitialization, error)
    })?;
    let config = compio_quic::ClientConfig::new(Arc::new(crypto));
    let local_address = if address.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    compio::runtime::time::timeout(timeout, async {
        let endpoint = compio_quic::Endpoint::client(local_address)
            .await
            .map_err(|error| {
                LegacyTransportError::backend(BACKEND, Operation::EndpointInitialization, error)
            })?;
        let mut inner = endpoint
            .connect(address, server_name, Some(config))
            .map_err(|error| {
                LegacyTransportError::backend(BACKEND, Operation::ConnectionInitialization, error)
            })?
            .await
            .map_err(|error| LegacyTransportError::backend(BACKEND, Operation::Handshake, error))?;
        let negotiated_alpn = inner
            .handshake_data()
            .map_err(|error| LegacyTransportError::backend(BACKEND, Operation::Handshake, error))?
            .protocol
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
    })
    .await
    .map_err(|_| LegacyTransportError::timeout(BACKEND, Operation::ConnectionSetup, timeout))?
}

impl BackendConnection for Connection {
    type Stream = Stream;

    fn negotiated_alpn(&self) -> Option<&[u8]> {
        Some(&self.negotiated_alpn)
    }

    async fn open_bi(&self, timeout: Duration) -> Result<Self::Stream, LegacyTransportError> {
        let (send, receive) = compio::runtime::time::timeout(timeout, self.inner.open_bi_wait())
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
        let result =
            compio::runtime::time::timeout(timeout, self.send.write_vectored_all(request)).await;
        match result {
            Ok(BufResult(Ok(()), _)) => Ok(()),
            Ok(BufResult(Err(error), _)) => {
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
        match compio::runtime::time::timeout(timeout, self.receive.read_exact([0])).await {
            Ok(BufResult(Ok(()), bytes)) => Ok(bytes[0]),
            Ok(BufResult(Err(error), _)) => {
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
        match compio::runtime::time::timeout(
            timeout,
            self.receive.read_exact(Vec::with_capacity(length)),
        )
        .await
        {
            Ok(BufResult(Ok(()), bytes)) => Ok(bytes),
            Ok(BufResult(Err(error), _)) => {
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
