//! Quinn-backed QUIC transport implementation.
//!
//! This module is compiled when the `backend-quinn` feature is active.

use std::net::SocketAddr;
use std::sync::Arc;

use crate::{Error, Result};

/// A QUIC connection using the quinn crate.
pub(crate) struct BackendConnection {
    conn: quinn::Connection,
}

/// A bidirectional QUIC stream using the quinn crate.
pub(crate) struct BackendStream {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

/// Open a QUIC connection via quinn.
pub(crate) async fn connect(
    addr: SocketAddr,
    server_name: &str,
    tls: rustls::ClientConfig,
) -> Result<BackendConnection> {
    let endpoint = quinn::Endpoint::client("[::]:0".parse().unwrap())
        .map_err(|e| Error::Connection(e.to_string()))?;

    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|e| Error::Connection(e.to_string()))?;
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(100u32.into());
    config.transport_config(Arc::new(transport));

    let conn = endpoint
        .connect_with(config, addr, server_name)
        .map_err(|e| Error::Connection(e.to_string()))?
        .await
        .map_err(|e| Error::Connection(e.to_string()))?;

    Ok(BackendConnection { conn })
}

impl BackendConnection {
    /// Open a bidirectional stream.
    pub(crate) async fn open_bi(&self) -> Result<BackendStream> {
        let (send, recv) = self
            .conn
            .open_bi()
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;
        Ok(BackendStream { send, recv })
    }
}

impl BackendStream {
    /// Write all bytes to the send stream.
    pub(crate) async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.send
            .write_all(buf)
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;
        Ok(())
    }

    /// Read all remaining data from the receive stream.
    pub(crate) async fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let data = self
            .recv
            .read_to_end(usize::MAX)
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;
        buf.extend_from_slice(&data);
        Ok(data.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_connection_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<BackendConnection>();
        assert_send::<BackendStream>();
    }

    #[test]
    fn error_type_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<crate::Error>();
    }
}
