//! Compio-QUIC implementation of the client transport boundary.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use compio::BufResult;
use compio::io::{AsyncReadExt, AsyncWriteExt};

use super::{BackendConnection, BackendStream, TransportError};

const NAME: &str = "compio";

pub(super) struct Connection {
    _endpoint: compio_quic::Endpoint,
    inner: compio_quic::Connection,
}

pub(super) struct Stream {
    send: compio_quic::SendStream,
    receive: compio_quic::RecvStream,
}

pub(super) async fn connect(
    address: SocketAddr,
    server_name: &str,
    tls: rustls::ClientConfig,
    timeout: Duration,
) -> Result<Connection, TransportError> {
    if compio::runtime::Runtime::try_current().is_none() {
        return Err(TransportError::runtime(
            NAME,
            "an active Compio runtime is required",
        ));
    }
    let crypto = compio_quic::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|error| TransportError::backend(NAME, "TLS initialization", error))?;
    let config = compio_quic::ClientConfig::new(Arc::new(crypto));
    let local_address = if address.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    compio::runtime::time::timeout(timeout, async {
        let endpoint = compio_quic::Endpoint::client(local_address)
            .await
            .map_err(|error| TransportError::backend(NAME, "endpoint initialization", error))?;
        let inner = endpoint
            .connect(address, server_name, Some(config))
            .map_err(|error| TransportError::backend(NAME, "connection initialization", error))?
            .await
            .map_err(|error| TransportError::backend(NAME, "handshake", error))?;
        Ok(Connection {
            _endpoint: endpoint,
            inner,
        })
    })
    .await
    .map_err(|_| TransportError::timeout(NAME, "connection", timeout))?
}

impl BackendConnection for Connection {
    type Stream = Stream;

    async fn open_bi(&self, timeout: Duration) -> Result<Self::Stream, TransportError> {
        let (send, receive) = compio::runtime::time::timeout(timeout, self.inner.open_bi_wait())
            .await
            .map_err(|_| TransportError::timeout(NAME, "stream open", timeout))?
            .map_err(|error| TransportError::backend(NAME, "stream open", error))?;
        Ok(Stream { send, receive })
    }

    fn close(&self) {
        self.inner.close(0_u32.into(), b"client closed");
    }
}

impl BackendStream for Stream {
    async fn write_all(&mut self, bytes: Vec<u8>, timeout: Duration) -> Result<(), TransportError> {
        let BufResult(result, _) =
            compio::runtime::time::timeout(timeout, self.send.write_all(bytes))
                .await
                .map_err(|_| TransportError::timeout(NAME, "stream write", timeout))?;
        result.map_err(|error| TransportError::backend(NAME, "stream write", error))
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
        .map_err(|_| TransportError::timeout(NAME, "stream read", timeout))?;
        result.map_err(|error| TransportError::backend(NAME, "stream read", error))?;
        Ok(bytes)
    }
}
