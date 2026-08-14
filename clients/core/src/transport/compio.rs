//! Compio-QUIC implementation of the client transport boundary.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use compio::BufResult;
use compio::buf::IoVectoredBuf;
use compio::io::{AsyncReadExt, AsyncWriteExt};

use super::{BackendConnection, BackendStream, TransportError};
use crate::protocol::RequestAttempt;
use crate::{Backend, Operation};

const BACKEND: Backend = Backend::Compio;

pub(super) struct Connection {
    _endpoint: compio_quic::Endpoint,
    inner: compio_quic::Connection,
    negotiated_alpn: Vec<u8>,
}

pub(super) struct Stream {
    send: compio_quic::SendStream,
    receive: compio_quic::RecvStream,
}

impl IoVectoredBuf for RequestAttempt {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        self.segments()
    }
}

pub(super) async fn connect(
    address: SocketAddr,
    server_name: &str,
    tls: rustls::ClientConfig,
    timeout: Duration,
) -> Result<Connection, TransportError> {
    if compio::runtime::Runtime::try_current().is_none() {
        return Err(TransportError::runtime(
            BACKEND,
            "an active Compio runtime is required",
        ));
    }
    let crypto = compio_quic::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|error| TransportError::backend(BACKEND, Operation::TlsInitialization, error))?;
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
                TransportError::backend(BACKEND, Operation::EndpointInitialization, error)
            })?;
        let mut inner = endpoint
            .connect(address, server_name, Some(config))
            .map_err(|error| {
                TransportError::backend(BACKEND, Operation::ConnectionInitialization, error)
            })?
            .await
            .map_err(|error| TransportError::backend(BACKEND, Operation::Handshake, error))?;
        let negotiated_alpn = inner
            .handshake_data()
            .map_err(|error| TransportError::backend(BACKEND, Operation::Handshake, error))?
            .protocol
            .ok_or_else(|| {
                TransportError::backend(
                    BACKEND,
                    Operation::Handshake,
                    "server did not negotiate an ALPN protocol",
                )
            })?;
        Ok(Connection {
            _endpoint: endpoint,
            inner,
            negotiated_alpn,
        })
    })
    .await
    .map_err(|_| TransportError::timeout(BACKEND, Operation::ConnectionSetup, timeout))?
}

impl BackendConnection for Connection {
    type Stream = Stream;

    fn negotiated_alpn(&self) -> Option<&[u8]> {
        Some(&self.negotiated_alpn)
    }

    async fn open_bi(&self, timeout: Duration) -> Result<Self::Stream, TransportError> {
        let (send, receive) = compio::runtime::time::timeout(timeout, self.inner.open_bi_wait())
            .await
            .map_err(|_| TransportError::timeout(BACKEND, Operation::StreamOpen, timeout))?
            .map_err(|error| TransportError::backend(BACKEND, Operation::StreamOpen, error))?;
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
    ) -> Result<(), TransportError> {
        let BufResult(result, _) = compio::runtime::time::timeout(
            timeout,
            self.send.write_vectored_all(request),
        )
        .await
        .map_err(|_| TransportError::timeout(BACKEND, Operation::StreamWrite, timeout))?;
        result
            .map_err(|error| TransportError::backend(BACKEND, Operation::StreamWrite, error))
    }

    async fn read_byte(&mut self, timeout: Duration) -> Result<u8, TransportError> {
        let BufResult(result, bytes) =
            compio::runtime::time::timeout(timeout, self.receive.read_exact([0]))
                .await
                .map_err(|_| TransportError::timeout(BACKEND, Operation::StreamRead, timeout))?;
        result.map_err(|error| TransportError::backend(BACKEND, Operation::StreamRead, error))?;
        Ok(bytes[0])
    }

    async fn read_exact(
        &mut self,
        length: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        let BufResult(result, bytes) = compio::runtime::time::timeout(
            timeout,
            self.receive.read_exact(Vec::with_capacity(length)),
        )
        .await
        .map_err(|_| TransportError::timeout(BACKEND, Operation::StreamRead, timeout))?;
        result.map_err(|error| TransportError::backend(BACKEND, Operation::StreamRead, error))?;
        Ok(bytes)
    }
}
