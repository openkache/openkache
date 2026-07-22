//! QUIC-based async client for the OpenKache cache server.
//!
//! Supports interchangeable transport backends (quinn, noq) selected at
//! compile time via Cargo feature flags.  Provides simple `get` / `set` /
//! `delete` operations over a single bidirectional QUIC stream per request.

mod transport;

use std::net::SocketAddr;

/// All client-level errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Connection setup or stream operation failed.
    #[error("connection failed: {0}")]
    Connection(String),

    /// Server returned an error-line response.
    #[error("server error: {0}")]
    Server(String),

    /// TLS configuration or handshake error.
    #[error("TLS error: {0}")]
    Tls(#[from] rustls::Error),

    /// Wraps [`std::io::Error`].
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience alias for client results.
pub type Result<T> = std::result::Result<T, Error>;

/// A QUIC connection to an OpenKache server.
pub struct Client {
    conn: transport::Connection,
}

// ---------------------------------------------------------------------------
// TLS setup
// ---------------------------------------------------------------------------

fn make_tls_config() -> Result<rustls::ClientConfig> {
    let provider = rustls::crypto::ring::default_provider();
    let mut config = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(Error::Tls)?
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    config.alpn_protocols = vec![b"openkache".to_vec()];
    Ok(config)
}

// ---------------------------------------------------------------------------
// Client API
// ---------------------------------------------------------------------------

impl Client {
    /// Open a QUIC connection to `addr`, using `server_name` for TLS SNI.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Connection`] if the endpoint or handshake fails,
    /// [`Error::Tls`] if the TLS configuration cannot be built.
    pub async fn connect(addr: SocketAddr, server_name: &str) -> Result<Self> {
        let tls = make_tls_config()?;
        let conn = transport::connect(addr, server_name, tls).await?;
        Ok(Self { conn })
    }

    /// Retrieve the value for `key`.
    ///
    /// Returns `None` when the key does not exist.
    pub async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut stream = self.conn.open_bi().await?;
        stream
            .write_all(build_command(b"GET", key, None, None).as_slice())
            .await?;
        read_response(&mut stream).await
    }

    /// Store `value` for `key`, optionally with a TTL in seconds.
    pub async fn set(&self, key: &[u8], value: &[u8], ttl: Option<u32>) -> Result<()> {
        let mut stream = self.conn.open_bi().await?;
        stream
            .write_all(build_command(b"SET", key, Some(value), ttl).as_slice())
            .await?;
        read_response(&mut stream).await?;
        Ok(())
    }

    /// Delete `key`.  Returns `true` if the key existed before deletion.
    pub async fn delete(&self, key: &[u8]) -> Result<bool> {
        let mut stream = self.conn.open_bi().await?;
        stream
            .write_all(build_command(b"DEL", key, None, None).as_slice())
            .await?;
        let resp = read_response(&mut stream).await?;
        Ok(resp.is_some())
    }
}

// ---------------------------------------------------------------------------
// Wire-protocol helpers
// ---------------------------------------------------------------------------

fn build_command(cmd: &[u8], key: &[u8], value: Option<&[u8]>, ttl: Option<u32>) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(cmd);
    buf.push(b' ');
    if let Some(ttl) = ttl {
        buf.extend_from_slice(ttl.to_string().as_bytes());
        buf.push(b' ');
    }
    buf.extend_from_slice(key);
    buf.extend_from_slice(b"\r\n");
    if let Some(value) = value {
        let len = value.len().to_string();
        buf.extend_from_slice(len.as_bytes());
        buf.extend_from_slice(b"\r\n");
        buf.extend_from_slice(value);
        buf.extend_from_slice(b"\r\n");
    }
    buf
}

fn parse_response(raw: &[u8]) -> Result<Option<Vec<u8>>> {
    if raw.is_empty() {
        return Ok(None);
    }
    let status = raw[0];
    match status {
        b'+' => Ok(Some(raw[1..].to_vec())),
        b'-' => {
            let msg = String::from_utf8_lossy(&raw[1..]).to_string();
            Err(Error::Server(msg))
        }
        b'$' => {
            if raw.len() == 1 {
                return Ok(None);
            }
            Ok(Some(raw[1..].to_vec()))
        }
        _ => Ok(Some(raw.to_vec())),
    }
}

async fn read_response(stream: &mut transport::BidiStream) -> Result<Option<Vec<u8>>> {
    let mut response = Vec::new();
    let n = stream.read_to_end(&mut response).await?;
    if n == 0 {
        return Ok(None);
    }
    parse_response(&response)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- build_command ------------------------------------------------------

    #[test]
    fn build_get_command() {
        let cmd = build_command(b"GET", b"mykey", None, None);
        assert_eq!(cmd, b"GET mykey\r\n");
    }

    #[test]
    fn build_set_command_no_ttl() {
        let cmd = build_command(b"SET", b"k", Some(b"v"), None);
        assert_eq!(cmd, b"SET k\r\n1\r\nv\r\n");
    }

    #[test]
    fn build_set_command_with_ttl() {
        let cmd = build_command(b"SET", b"k", Some(b"v"), Some(300));
        assert_eq!(cmd, b"SET 300 k\r\n1\r\nv\r\n");
    }

    #[test]
    fn build_del_command() {
        let cmd = build_command(b"DEL", b"mykey", None, None);
        assert_eq!(cmd, b"DEL mykey\r\n");
    }

    // -- parse_response -----------------------------------------------------

    #[test]
    fn parse_empty_returns_none() {
        assert!(parse_response(b"").unwrap().is_none());
    }

    #[test]
    fn parse_plus_status_ok() {
        let r = parse_response(b"+OK").unwrap();
        assert_eq!(r, Some(b"OK".to_vec()));
    }

    #[test]
    fn parse_plus_status_trailing() {
        let r = parse_response(b"+value\r\n").unwrap();
        assert_eq!(r, Some(b"value\r\n".to_vec()));
    }

    #[test]
    fn parse_minus_error() {
        let err = parse_response(b"-not found").unwrap_err();
        assert!(matches!(err, Error::Server(_)));
        assert_eq!(err.to_string(), "server error: not found");
    }

    #[test]
    fn parse_dollar_bulk() {
        let r = parse_response(b"$hello").unwrap();
        assert_eq!(r, Some(b"hello".to_vec()));
    }

    #[test]
    fn parse_dollar_empty_returns_none() {
        assert!(parse_response(b"$").unwrap().is_none());
    }

    #[test]
    fn parse_unknown_status_passthrough() {
        let r = parse_response(b":42").unwrap();
        assert_eq!(r, Some(b":42".to_vec()));
    }

    // -- make_tls_config ----------------------------------------------------

    #[test]
    fn tls_config_uses_openkache_alpn() {
        let cfg = make_tls_config().unwrap();
        assert!(cfg.alpn_protocols.contains(&b"openkache".to_vec()));
    }

    // -- Error type traits --------------------------------------------------

    #[test]
    fn error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Error>();
    }

    #[test]
    fn error_display_connection() {
        let e = Error::Connection("timeout".into());
        assert_eq!(e.to_string(), "connection failed: timeout");
    }

    #[test]
    fn error_display_server() {
        let e = Error::Server("not found".into());
        assert_eq!(e.to_string(), "server error: not found");
    }

    #[test]
    fn error_display_tls() {
        let e = Error::Tls(rustls::Error::NoApplicationProtocol);
        assert!(
            e.to_string().starts_with("TLS error:"),
            "unexpected TLS message: {}",
            e.to_string()
        );
    }
}
