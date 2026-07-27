//! QUIC client for the OpenKache binary protocol.

mod transport;

use std::net::SocketAddr;

use openkache_protocol::{
    ClientKeyDigest, MAX_RESPONSE_FRAME_BYTES, Opcode, Request, Response, Status,
};
use rustls::pki_types::CertificateDer;

/// All client-level errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("server returned {status:?}: {message}")]
    Server { status: Status, message: String },
    #[error("unexpected {operation} response status: {status:?}")]
    UnexpectedStatus {
        operation: &'static str,
        status: Status,
    },
    #[error("unexpected PING response payload")]
    UnexpectedPingPayload,
    #[error("response exceeds protocol limit of {maximum} bytes")]
    ResponseTooLarge { maximum: usize },
    #[error("TLS configuration failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("protocol failed: {0}")]
    Protocol(#[from] openkache_protocol::ProtocolError),
    #[error("response is not valid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience alias for client results.
pub type Result<T> = std::result::Result<T, Error>;

/// Result of storing a key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetOutcome {
    Created,
    Replaced,
}

/// A reusable QUIC connection to an OpenKache server.
pub struct Client {
    connection: transport::Connection,
}

impl Client {
    /// Connects to a server and trusts the supplied DER certificate.
    pub async fn connect(
        address: SocketAddr,
        server_name: &str,
        trusted_certificate_der: &[u8],
    ) -> Result<Self> {
        let tls = make_tls_config(trusted_certificate_der)?;
        let connection = transport::connect(address, server_name, tls).await?;
        Ok(Self { connection })
    }

    /// Verifies that the server is reachable and speaks protocol v1.
    pub async fn ping(&self) -> Result<()> {
        let response = self
            .request(Request::new(Opcode::Ping, None, Vec::new())?)
            .await?;
        expect_status("PING", response.status, &[Status::Ok])?;
        if response.payload != b"PONG" {
            return Err(Error::UnexpectedPingPayload);
        }
        Ok(())
    }

    /// Retrieves a value, returning `None` when the key does not exist.
    pub async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let client_key_digest = ClientKeyDigest::from_user_key(key);
        let response = self
            .request(Request::new(
                Opcode::Get,
                Some(client_key_digest),
                Vec::new(),
            )?)
            .await?;
        match response.status {
            Status::Ok => Ok(Some(response.payload)),
            Status::NotFound => Ok(None),
            status => Err(unexpected("GET", status)),
        }
    }

    /// Stores a value and reports whether it created or replaced the key.
    pub async fn set(&self, key: &[u8], value: &[u8]) -> Result<SetOutcome> {
        let client_key_digest = ClientKeyDigest::from_user_key(key);
        let response = self
            .request(Request::new(
                Opcode::Set,
                Some(client_key_digest),
                value.to_vec(),
            )?)
            .await?;
        match response.status {
            Status::Created => Ok(SetOutcome::Created),
            Status::Replaced => Ok(SetOutcome::Replaced),
            status => Err(unexpected("SET", status)),
        }
    }

    /// Deletes a key and returns whether it existed.
    pub async fn delete(&self, key: &[u8]) -> Result<bool> {
        let client_key_digest = ClientKeyDigest::from_user_key(key);
        let response = self
            .request(Request::new(
                Opcode::Delete,
                Some(client_key_digest),
                Vec::new(),
            )?)
            .await?;
        match response.status {
            Status::Deleted => Ok(true),
            Status::NotFound => Ok(false),
            status => Err(unexpected("DELETE", status)),
        }
    }

    /// Returns the server's JSON statistics payload.
    pub async fn stats(&self) -> Result<String> {
        let response = self
            .request(Request::new(Opcode::Stats, None, Vec::new())?)
            .await?;
        expect_status("STATS", response.status, &[Status::Ok])?;
        Ok(String::from_utf8(response.payload)?)
    }

    /// Requests a durability barrier.
    pub async fn sync(&self) -> Result<()> {
        let response = self
            .request(Request::new(Opcode::Sync, None, Vec::new())?)
            .await?;
        expect_status("SYNC", response.status, &[Status::Ok])
    }

    async fn request(&self, request: Request) -> Result<Response> {
        let mut stream = self.connection.open_bi().await?;
        stream.write_request(request.encode()?).await?;
        let frame = stream.read_response(MAX_RESPONSE_FRAME_BYTES).await?;
        let response = Response::decode(&frame)?;
        if response.status.is_error() {
            return Err(Error::Server {
                status: response.status,
                message: String::from_utf8_lossy(&response.payload).into_owned(),
            });
        }
        Ok(response)
    }
}

fn make_tls_config(trusted_certificate_der: &[u8]) -> Result<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(trusted_certificate_der.to_vec()))?;
    let provider = rustls::crypto::ring::default_provider();
    let mut config = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![openkache_protocol::ALPN.to_vec()];
    Ok(config)
}

fn expect_status(operation: &'static str, status: Status, expected: &[Status]) -> Result<()> {
    if expected.contains(&status) {
        Ok(())
    } else {
        Err(unexpected(operation, status))
    }
}

fn unexpected(operation: &'static str, status: Status) -> Error {
    Error::UnexpectedStatus { operation, status }
}
