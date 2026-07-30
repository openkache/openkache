//! Quinn implementation of the client transport boundary.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use super::{BackendConnection, BackendStream, TransportError};

const NAME: &str = "quinn";

pub(super) struct Connection {
    _endpoint: quinn::Endpoint,
    inner: quinn::Connection,
}

pub(super) struct Stream {
    send: quinn::SendStream,
    receive: quinn::RecvStream,
}

pub(super) async fn connect(
    address: SocketAddr,
    server_name: &str,
    tls: rustls::ClientConfig,
    timeout: Duration,
) -> Result<Connection, TransportError> {
    tokio::runtime::Handle::try_current()
        .map_err(|_| TransportError::runtime(NAME, "an active Tokio runtime is required"))?;
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|error| TransportError::backend(NAME, "TLS initialization", error))?;
    let config = quinn::ClientConfig::new(Arc::new(crypto));
    let local_address = if address.is_ipv4() {
        "0.0.0.0:0".parse().expect("valid IPv4 wildcard address")
    } else {
        "[::]:0".parse().expect("valid IPv6 wildcard address")
    };
    let endpoint = quinn::Endpoint::client(local_address)
        .map_err(|error| TransportError::backend(NAME, "endpoint initialization", error))?;
    let connecting = endpoint
        .connect_with(config, address, server_name)
        .map_err(|error| TransportError::backend(NAME, "connection initialization", error))?;
    let inner = tokio::time::timeout(timeout, connecting)
        .await
        .map_err(|_| TransportError::timeout(NAME, "connection", timeout))?
        .map_err(|error| TransportError::backend(NAME, "handshake", error))?;
    Ok(Connection {
        _endpoint: endpoint,
        inner,
    })
}

impl BackendConnection for Connection {
    type Stream = Stream;

    async fn open_bi(&self, timeout: Duration) -> Result<Self::Stream, TransportError> {
        let (send, receive) = tokio::time::timeout(timeout, self.inner.open_bi())
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
        tokio::time::timeout(timeout, self.send.write_all(&bytes))
            .await
            .map_err(|_| TransportError::timeout(NAME, "stream write", timeout))?
            .map_err(|error| TransportError::backend(NAME, "stream write", error))
    }

    async fn read_exact(
        &mut self,
        length: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        let mut bytes = vec![0; length];
        tokio::time::timeout(timeout, self.receive.read_exact(&mut bytes))
            .await
            .map_err(|_| TransportError::timeout(NAME, "stream read", timeout))?
            .map_err(|error| TransportError::backend(NAME, "stream read", error))?;
        Ok(bytes)
    }
}
