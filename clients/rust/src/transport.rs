//! Pluggable QUIC transport layer.
//!
//! Re-exports the currently selected backend (quinn / noq) as a uniform
//! [`Connection`] / [`BidiStream`] pair so that the rest of the client
//! crate does not depend on any particular QUIC implementation.

use std::net::SocketAddr;

#[cfg(feature = "backend-quinn")]
mod quinn_impl;
#[cfg(feature = "backend-quinn")]
use quinn_impl as backend;

#[cfg(all(feature = "backend-noq", not(feature = "backend-quinn")))]
mod noq_impl;
#[cfg(all(feature = "backend-noq", not(feature = "backend-quinn")))]
use noq_impl as backend;

use crate::Result;

/// Wraps a backend-specific QUIC connection.
pub(crate) struct Connection {
    inner: backend::BackendConnection,
}

/// Wraps a backend-specific bidirectional QUIC stream.
pub(crate) struct BidiStream {
    inner: backend::BackendStream,
}

/// Open a QUIC connection to `addr`, authenticating as `server_name` with the
/// given TLS configuration.
pub(crate) async fn connect(
    addr: SocketAddr,
    server_name: &str,
    tls: rustls::ClientConfig,
) -> Result<Connection> {
    let inner = backend::connect(addr, server_name, tls).await?;
    Ok(Connection { inner })
}

impl Connection {
    /// Open a new bidirectional stream over the QUIC connection.
    pub(crate) async fn open_bi(&self) -> Result<BidiStream> {
        let inner = self.inner.open_bi().await?;
        Ok(BidiStream { inner })
    }
}

impl BidiStream {
    /// Writes one complete request and closes the sending half.
    pub(crate) async fn write_request(&mut self, frame: &[u8]) -> Result<()> {
        self.inner.write_all(frame).await?;
        self.inner.finish()
    }

    /// Reads one complete response up to `maximum` bytes.
    pub(crate) async fn read_response(&mut self, maximum: usize) -> Result<Vec<u8>> {
        self.inner.read_to_end(maximum).await
    }
}
