//! Compio QUIC transport for the OpenKache client.

use std::net::SocketAddr;
use std::sync::Arc;

use compio::BufResult;
use compio::io::AsyncWriteExt;
use compio_quic::{Endpoint, RecvStream, SendStream};

use crate::{Error, Result};

/// Keeps the Compio endpoint alive alongside its QUIC connection.
pub(crate) struct Connection {
    _endpoint: Endpoint,
    inner: compio_quic::Connection,
}

/// A Compio bidirectional QUIC stream.
pub(crate) struct BidiStream {
    send: SendStream,
    receive: RecvStream,
}

/// Open a QUIC connection to `addr`, authenticating as `server_name` with the
/// given TLS configuration.
pub(crate) async fn connect(
    addr: SocketAddr,
    server_name: &str,
    tls: rustls::ClientConfig,
) -> Result<Connection> {
    let crypto = compio_quic::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|error| Error::Connection(error.to_string()))?;
    let config = compio_quic::ClientConfig::new(Arc::new(crypto));
    let local_address = if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let endpoint = Endpoint::client(local_address).await?;
    let inner = endpoint
        .connect(addr, server_name, Some(config))
        .map_err(|error| Error::Connection(error.to_string()))?
        .await
        .map_err(|error| Error::Connection(error.to_string()))?;
    Ok(Connection {
        _endpoint: endpoint,
        inner,
    })
}

impl Connection {
    /// Open a new bidirectional stream over the QUIC connection.
    pub(crate) async fn open_bi(&self) -> Result<BidiStream> {
        let (send, receive) = self
            .inner
            .open_bi_wait()
            .await
            .map_err(|error| Error::Connection(error.to_string()))?;
        Ok(BidiStream { send, receive })
    }
}

impl BidiStream {
    /// Writes one complete request and closes the sending half.
    pub(crate) async fn write_request(&mut self, frame: Vec<u8>) -> Result<()> {
        let BufResult(result, _) = self.send.write_all(frame).await;
        result?;
        self.send
            .finish()
            .map_err(|error| Error::Connection(error.to_string()))
    }

    /// Reads one complete response up to `maximum` bytes.
    pub(crate) async fn read_response(&mut self, maximum: usize) -> Result<Vec<u8>> {
        let mut frame = Vec::new();
        while let Some(chunk) = self
            .receive
            .read_chunk(maximum.saturating_add(1), true)
            .await
            .map_err(|error| Error::Connection(error.to_string()))?
        {
            if frame.len().saturating_add(chunk.bytes.len()) > maximum {
                return Err(Error::ResponseTooLarge { maximum });
            }
            frame.extend_from_slice(&chunk.bytes);
        }
        Ok(frame)
    }
}
