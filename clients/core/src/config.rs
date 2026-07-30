//! Stable core configuration types independent of the TLS and QUIC implementations.

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::{Error, Result};

/// Network destination and TLS identity of one OpenKache server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Endpoint {
    host: String,
    port: u16,
    address: Option<SocketAddr>,
    server_name: String,
}

impl Endpoint {
    /// Creates an endpoint whose hostname is used for both DNS and TLS verification.
    ///
    /// # Arguments
    ///
    /// * `host` - DNS hostname or IP address without a port.
    /// * `port` - Server UDP port.
    ///
    /// # Returns
    ///
    /// A validated endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when `host` is empty or `port` is zero.
    pub fn new(host: impl Into<String>, port: u16) -> Result<Self> {
        let host = host.into();
        if host.is_empty() {
            return Err(Error::configuration("endpoint.host", "must not be empty"));
        }
        if port == 0 {
            return Err(Error::configuration(
                "endpoint.port",
                "must be greater than zero",
            ));
        }
        let address = host
            .parse::<IpAddr>()
            .ok()
            .map(|address| SocketAddr::new(address, port));
        Ok(Self {
            server_name: host.clone(),
            host,
            port,
            address,
        })
    }

    /// Creates an endpoint with a resolved address and independent TLS server name.
    ///
    /// # Arguments
    ///
    /// * `address` - Resolved UDP destination.
    /// * `server_name` - DNS name or IP address expected in the server certificate.
    ///
    /// # Returns
    ///
    /// A validated endpoint that performs no DNS lookup.
    ///
    /// # Errors
    ///
    /// Returns an error when the address port is zero or the server name is empty.
    pub fn from_socket_addr(address: SocketAddr, server_name: impl Into<String>) -> Result<Self> {
        let server_name = server_name.into();
        if address.port() == 0 {
            return Err(Error::configuration(
                "endpoint.port",
                "must be greater than zero",
            ));
        }
        if server_name.is_empty() {
            return Err(Error::configuration(
                "endpoint.server_name",
                "must not be empty",
            ));
        }
        Ok(Self {
            host: address.ip().to_string(),
            port: address.port(),
            address: Some(address),
            server_name,
        })
    }

    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    pub(crate) const fn port(&self) -> u16 {
        self.port
    }

    pub(crate) const fn resolved_address(&self) -> Option<SocketAddr> {
        self.address
    }

    pub(crate) fn server_name(&self) -> &str {
        &self.server_name
    }
}

impl FromStr for Endpoint {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let authority = value
            .parse::<http::uri::Authority>()
            .map_err(|error| Error::configuration("endpoint", error.to_string()))?;
        if authority.as_str().contains('@') {
            return Err(Error::configuration(
                "endpoint",
                "must not contain user information",
            ));
        }
        let port = authority
            .port_u16()
            .ok_or_else(|| Error::configuration("endpoint.port", "is required"))?;
        let host = authority
            .host()
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or_else(|| authority.host());
        Self::new(host, port)
    }
}

/// X.509 certificate accepted by the stable client API.
#[derive(Clone, Debug)]
pub struct Certificate(CertificateDer<'static>);

impl Certificate {
    /// Parses one DER-encoded X.509 certificate.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Complete DER certificate bytes.
    ///
    /// # Returns
    ///
    /// An owned certificate.
    ///
    /// # Errors
    ///
    /// Returns an error when the input is empty.
    pub fn from_der(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(Error::configuration(
                "certificate",
                "DER input must not be empty",
            ));
        }
        Ok(Self(CertificateDer::from(bytes)))
    }

    /// Parses exactly one PEM-encoded X.509 certificate.
    ///
    /// # Arguments
    ///
    /// * `bytes` - PEM certificate bytes.
    ///
    /// # Returns
    ///
    /// An owned certificate.
    ///
    /// # Errors
    ///
    /// Returns an error when PEM decoding fails or contains other than one certificate.
    pub fn from_pem(bytes: &[u8]) -> Result<Self> {
        let certificates = CertificateDer::pem_slice_iter(bytes)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| Error::configuration("certificate", error.to_string()))?;
        let [certificate] = <[_; 1]>::try_from(certificates).map_err(|certificates: Vec<_>| {
            Error::configuration(
                "certificate",
                format!(
                    "PEM input must contain exactly one certificate, got {}",
                    certificates.len()
                ),
            )
        })?;
        Ok(Self(certificate))
    }

    pub(crate) fn into_der(self) -> CertificateDer<'static> {
        self.0
    }
}

/// Private key accepted by the stable client API.
#[derive(Debug)]
pub struct PrivateKey(PrivateKeyDer<'static>);

impl PrivateKey {
    /// Parses one DER-encoded private key.
    ///
    /// # Arguments
    ///
    /// * `bytes` - DER PKCS#1, PKCS#8, or SEC1 private-key bytes.
    ///
    /// # Returns
    ///
    /// An owned private key.
    ///
    /// # Errors
    ///
    /// Returns an error when the DER key format is unsupported.
    pub fn from_der(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        PrivateKeyDer::try_from(bytes.into())
            .map(Self)
            .map_err(|_| Error::configuration("private_key", "unsupported DER key format"))
    }

    /// Parses one PEM-encoded private key.
    ///
    /// # Arguments
    ///
    /// * `bytes` - PEM private-key bytes.
    ///
    /// # Returns
    ///
    /// An owned private key.
    ///
    /// # Errors
    ///
    /// Returns an error when PEM decoding fails or the key format is unsupported.
    pub fn from_pem(bytes: &[u8]) -> Result<Self> {
        PrivateKeyDer::from_pem_slice(bytes)
            .map(Self)
            .map_err(|error| Error::configuration("private_key", error.to_string()))
    }

    pub(crate) fn into_der(self) -> PrivateKeyDer<'static> {
        self.0
    }
}

/// Certificate chain and private key presented during mutual TLS authentication.
#[derive(Debug)]
pub struct ClientIdentity {
    certificate_chain: Vec<Certificate>,
    private_key: PrivateKey,
}

impl ClientIdentity {
    /// Creates a mutual TLS identity.
    ///
    /// # Arguments
    ///
    /// * `certificate_chain` - Leaf certificate followed by intermediate certificates.
    /// * `private_key` - Private key corresponding to the leaf certificate.
    ///
    /// # Returns
    ///
    /// A validated client identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the certificate chain is empty.
    pub fn new(certificate_chain: Vec<Certificate>, private_key: PrivateKey) -> Result<Self> {
        if certificate_chain.is_empty() {
            return Err(Error::configuration(
                "client_identity.certificate_chain",
                "must not be empty",
            ));
        }
        Ok(Self {
            certificate_chain,
            private_key,
        })
    }

    pub(crate) fn into_rustls(self) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        (
            self.certificate_chain
                .into_iter()
                .map(Certificate::into_der)
                .collect(),
            self.private_key.into_der(),
        )
    }
}

/// Trust roots used to authenticate the server.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub enum ServerTrust {
    /// Load the operating system's native root certificates.
    #[default]
    System,
    /// Trust only the supplied certificates, including an explicitly trusted self-signed leaf.
    Custom(Vec<Certificate>),
}

/// Deadlines applied to connection setup and complete request exchanges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientTimeouts {
    /// DNS resolution, endpoint initialization, and QUIC/TLS handshake deadline.
    pub connect: Duration,
    /// Lane acquisition, request transmission, and response receipt deadline.
    pub request: Duration,
}

impl Default for ClientTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(5),
            request: Duration::from_secs(2),
        }
    }
}

/// Retry policy for operations whose responses can be requested again safely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    /// Maximum total attempts, including the initial request.
    pub max_attempts: usize,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 2 }
    }
}

/// Atomic existence condition for one set operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum SetCondition {
    /// Store regardless of whether the key exists.
    #[default]
    None,
    /// Store only when the key does not exist.
    IfAbsent,
    /// Store only when the key already exists.
    IfPresent,
}

/// Optional behavior for one set operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SetOptions {
    condition: SetCondition,
    time_to_live_ms: Option<u64>,
}

impl SetOptions {
    /// Creates persistent, unconditional set behavior.
    pub const fn new() -> Self {
        Self {
            condition: SetCondition::None,
            time_to_live_ms: None,
        }
    }

    /// Stores only when the key does not exist.
    pub const fn if_absent(mut self) -> Self {
        self.condition = SetCondition::IfAbsent;
        self
    }

    /// Stores only when the key already exists.
    pub const fn if_present(mut self) -> Self {
        self.condition = SetCondition::IfPresent;
        self
    }

    /// Applies a relative expiration in milliseconds.
    pub const fn expires_after_millis(mut self, milliseconds: u64) -> Self {
        self.time_to_live_ms = Some(milliseconds);
        self
    }

    /// Returns the configured existence condition.
    pub const fn condition(self) -> SetCondition {
        self.condition
    }

    /// Returns the configured relative expiration in milliseconds.
    pub const fn time_to_live_millis(self) -> Option<u64> {
        self.time_to_live_ms
    }

    pub(crate) fn into_protocol(self) -> Result<openkache_protocol::SetOptions> {
        let condition = match self.condition {
            SetCondition::None => openkache_protocol::SetCondition::None,
            SetCondition::IfAbsent => openkache_protocol::SetCondition::IfAbsent,
            SetCondition::IfPresent => openkache_protocol::SetCondition::IfPresent,
        };
        if self.time_to_live_ms == Some(0) {
            return Err(Error::configuration(
                "set.time_to_live_ms",
                "must be greater than zero",
            ));
        }
        Ok(openkache_protocol::SetOptions::new(
            condition,
            self.time_to_live_ms,
        ))
    }
}
