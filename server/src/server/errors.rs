//! Server lifecycle and configuration errors.

use std::net::SocketAddr;

use crate::KvError;
use crate::transport::TransportError;

/// Errors produced while configuring or running the QUIC server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("cache failed: {0}")]
    Cache(#[from] KvError),
    #[error(
        "production TLS and client authentication are required to bind {0}; configure [tls] or explicitly select insecure development mode"
    )]
    ProductionTlsRequired(SocketAddr),
    #[error("production TLS cannot be combined with insecure development mode")]
    ConflictingSecurityModes,
    #[error("plaintext RESP is restricted to a loopback address, not {0}")]
    PlaintextRespRequiresLoopback(SocketAddr),
    #[error("certificate generation failed: {0}")]
    Certificate(#[from] rcgen::Error),
    #[error("TLS identity file {path} is invalid: {message}")]
    TlsIdentity {
        path: std::path::PathBuf,
        message: String,
    },
    #[error("namespace metadata is invalid or unavailable: {0}")]
    NamespaceMetadata(String),
    #[error("TLS configuration failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("QUIC transport failed: {0}")]
    Transport(#[from] TransportError),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("network worker failed: {0}")]
    NetworkWorker(String),
}

/// Convenience result type for server lifecycle operations.
pub type Result<T> = std::result::Result<T, ServerError>;
