//! Quinn implementation of the client transport boundary.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use super::{BackendConnection, BackendStream, TransportError};
use crate::{Backend, Operation};

const BACKEND: Backend = Backend::Quinn;

pub(super) struct Connection {
    _endpoint: quinn::Endpoint,
    inner: quinn::Connection,
    negotiated_alpn: Vec<u8>,
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
        .map_err(|_| TransportError::runtime(BACKEND, "an active Tokio runtime is required"))?;
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|error| TransportError::backend(BACKEND, Operation::TlsInitialization, error))?;
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
        TransportError::backend(BACKEND, Operation::EndpointInitialization, error)
    })?;
    let connecting = endpoint
        .connect_with(config, address, server_name)
        .map_err(|error| {
            TransportError::backend(BACKEND, Operation::ConnectionInitialization, error)
        })?;
    let inner = tokio::time::timeout(timeout, connecting)
        .await
        .map_err(|_| TransportError::timeout(BACKEND, Operation::ConnectionSetup, timeout))?
        .map_err(|error| TransportError::backend(BACKEND, Operation::Handshake, error))?;
    let negotiated_alpn = inner
        .handshake_data()
        .and_then(|data| data.downcast::<quinn::crypto::rustls::HandshakeData>().ok())
        .and_then(|data| data.protocol)
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
}

impl BackendConnection for Connection {
    type Stream = Stream;

    fn negotiated_alpn(&self) -> Option<&[u8]> {
        Some(&self.negotiated_alpn)
    }

    async fn open_bi(&self, timeout: Duration) -> Result<Self::Stream, TransportError> {
        let (send, receive) = tokio::time::timeout(timeout, self.inner.open_bi())
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
    async fn write_all(&mut self, bytes: Vec<u8>, timeout: Duration) -> Result<(), TransportError> {
        tokio::time::timeout(timeout, self.send.write_all(&bytes))
            .await
            .map_err(|_| TransportError::timeout(BACKEND, Operation::StreamWrite, timeout))?
            .map_err(|error| TransportError::backend(BACKEND, Operation::StreamWrite, error))
    }

    async fn read_exact(
        &mut self,
        length: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        let mut bytes = vec![0; length];
        tokio::time::timeout(timeout, self.receive.read_exact(&mut bytes))
            .await
            .map_err(|_| TransportError::timeout(BACKEND, Operation::StreamRead, timeout))?
            .map_err(|error| TransportError::backend(BACKEND, Operation::StreamRead, error))?;
        Ok(bytes)
    }
}
