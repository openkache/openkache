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
    /// Write all bytes from `buf` into the stream (like
    /// [`tokio::io::AsyncWriteExt::write_all`]).
    pub(crate) async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.inner.write_all(buf).await
    }

    /// Read the remainder of the stream into `buf`, returning the number of
    /// bytes read.
    pub(crate) async fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        self.inner.read_to_end(buf).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_and_stream_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Connection>();
        assert_send::<BidiStream>();
    }
}
