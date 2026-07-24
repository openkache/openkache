//! Noq-backed QUIC transport implementation.
//!
//! This module is compiled when the `backend-noq` feature is active and
//! `backend-quinn` is not.

use std::net::SocketAddr;
use std::sync::Arc;

use crate::{Error, Result};

/// A QUIC connection using the noq crate.
pub(crate) struct BackendConnection {
    conn: noq::Connection,
}

/// A bidirectional QUIC stream using the noq crate.
pub(crate) struct BackendStream {
    send: noq::SendStream,
    recv: noq::RecvStream,
}

/// Open a QUIC connection via noq.
pub(crate) async fn connect(
    addr: SocketAddr,
    server_name: &str,
    tls: rustls::ClientConfig,
) -> Result<BackendConnection> {
    let endpoint = noq::Endpoint::client("[::]:0".parse().unwrap())
        .map_err(|e| Error::Connection(e.to_string()))?;

    let crypto = noq::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|e| Error::Connection(e.to_string()))?;
    let mut config = noq::ClientConfig::new(Arc::new(crypto));
    let mut transport = noq::TransportConfig::default();
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

    /// Closes the sending half after one request frame.
    pub(crate) fn finish(&mut self) -> Result<()> {
        self.send
            .finish()
            .map_err(|error| Error::Connection(error.to_string()))
    }

    /// Reads the complete response with a fixed upper bound.
    pub(crate) async fn read_to_end(&mut self, maximum: usize) -> Result<Vec<u8>> {
        self.recv
            .read_to_end(maximum)
            .await
            .map_err(|error| Error::Connection(error.to_string()))
    }
}
