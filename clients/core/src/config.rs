//! Stable core configuration types independent of the TLS and QUIC implementations.

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::protocol::{EvictionMode, ExpirationMode, SetCondition, SetWireOptions};
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
    /// Returns an error when `host` is not a valid TLS DNS name or IP address, or `port` is zero.
    pub fn new(host: impl Into<String>, port: u16) -> Result<Self> {
        let host = host.into();
        validate_server_name("endpoint.host", &host)?;
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
    /// Returns an error when the address port is zero or `server_name` is not a valid TLS DNS name
    /// or IP address.
    pub fn from_socket_addr(address: SocketAddr, server_name: impl Into<String>) -> Result<Self> {
        let server_name = server_name.into();
        if address.port() == 0 {
            return Err(Error::configuration(
                "endpoint.port",
                "must be greater than zero",
            ));
        }
        validate_server_name("endpoint.server_name", &server_name)?;
        Ok(Self {
            host: address.ip().to_string(),
            port: address.port(),
            address: Some(address),
            server_name,
        })
    }

    /// Replaces the TLS server name while preserving this endpoint's network destination.
    ///
    /// Native adapters receive a destination and a separate certificate identity. Keeping this
    /// operation on the shared endpoint type avoids duplicating validation in each binding.
    #[cfg(feature = "ffi")]
    pub(crate) fn with_server_name(mut self, server_name: impl Into<String>) -> Result<Self> {
        let server_name = server_name.into();
        validate_server_name("endpoint.server_name", &server_name)?;
        self.server_name = server_name;
        Ok(self)
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

fn validate_server_name(field: &'static str, value: &str) -> Result<()> {
    rustls::pki_types::ServerName::try_from(value.to_owned())
        .map(|_| ())
        .map_err(|error| Error::configuration(field, error.to_string()))
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
    /// Wraps one non-empty DER-encoded X.509 certificate.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Complete DER certificate bytes.
    ///
    /// # Returns
    ///
    /// Owned certificate bytes. Certificate validation occurs when the TLS
    /// configuration is built.
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
        let certificates = decode_pem_certificates(pem_bytes(bytes).ok_or_else(|| {
            Error::configuration(
                "certificate",
                "PEM input must start with a certificate block",
            )
        })?)?;
        let [certificate] =
            <[_; 1]>::try_from(certificates).map_err(|certificates: Vec<Self>| {
                Error::configuration(
                    "certificate",
                    format!(
                        "PEM input must contain exactly one certificate, got {}",
                        certificates.len()
                    ),
                )
            })?;
        Ok(certificate)
    }

    /// Wraps one DER certificate or parses a non-empty PEM certificate chain.
    ///
    /// # Arguments
    ///
    /// * `bytes` - One DER certificate or one or more PEM certificates.
    ///
    /// # Returns
    ///
    /// A certificate chain in input order. DER certificate validation occurs
    /// when the TLS configuration is built.
    ///
    /// # Errors
    ///
    /// Returns an error when the input is empty, PEM decoding fails, or a PEM input contains no
    /// certificates.
    pub fn from_der_or_pem_chain(bytes: &[u8]) -> Result<Vec<Self>> {
        if bytes.is_empty() {
            return Err(Error::configuration(
                "certificate",
                "input must not be empty",
            ));
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Err(Error::configuration(
                "certificate",
                "input must contain DER bytes or a PEM certificate block",
            ));
        }
        if let Some(pem) = pem_bytes(bytes) {
            decode_pem_certificates(pem)
        } else {
            Self::from_der(bytes.to_vec()).map(|certificate| vec![certificate])
        }
    }

    pub(crate) fn into_der(self) -> CertificateDer<'static> {
        self.0
    }
}

fn decode_pem_certificates(bytes: &[u8]) -> Result<Vec<Certificate>> {
    let certificates = CertificateDer::pem_slice_iter(bytes)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| Error::configuration("certificate", error.to_string()))?
        .into_iter()
        .map(Certificate)
        .collect::<Vec<_>>();
    if certificates.is_empty() {
        return Err(Error::configuration(
            "certificate",
            "PEM input contains no certificates",
        ));
    }
    Ok(certificates)
}

fn pem_bytes(bytes: &[u8]) -> Option<&[u8]> {
    let start = bytes.iter().position(|byte| !byte.is_ascii_whitespace())?;
    let bytes = &bytes[start..];
    bytes.starts_with(b"-----BEGIN ").then_some(bytes)
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
        let bytes = pem_bytes(bytes).ok_or_else(|| {
            Error::configuration("private_key", "PEM input must start with a key block")
        })?;
        PrivateKeyDer::from_pem_slice(bytes)
            .map(Self)
            .map_err(|error| Error::configuration("private_key", error.to_string()))
    }

    /// Parses one DER- or PEM-encoded private key.
    ///
    /// # Arguments
    ///
    /// * `bytes` - DER or PEM PKCS#1, PKCS#8, or SEC1 private-key bytes.
    ///
    /// # Returns
    ///
    /// An owned private key.
    ///
    /// # Errors
    ///
    /// Returns an error when the input is empty, PEM decoding fails, or the key format is
    /// unsupported.
    pub fn from_der_or_pem(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::configuration(
                "private_key",
                "input must not be empty",
            ));
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Err(Error::configuration(
                "private_key",
                "input must contain DER bytes or a PEM key block",
            ));
        }
        if pem_bytes(bytes).is_some() {
            Self::from_pem(bytes)
        } else {
            Self::from_der(bytes.to_vec())
        }
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
    /// An owned client identity. Remaining certificate and private-key
    /// validation occurs while building TLS configuration or during the
    /// handshake.
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

/// ALPN identifiers offered during QUIC/TLS negotiation.
///
/// The identifiers use the protocol's `openkache/<positive-decimal-version>`
/// grammar. Entries are offered in strict descending version order, as
/// required by the wire contract. A negotiated version below `minimum_version`
/// is rejected after the handshake, which lets a client advertise a fallback
/// while still enforcing its deployment minimum.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlpnPolicy {
    protocols: Vec<Vec<u8>>,
    minimum_version: u32,
}

impl AlpnPolicy {
    /// Creates an ALPN policy from a strict descending list of protocol names.
    ///
    /// # Arguments
    ///
    /// * `protocols` - Non-empty ALPN names such as `openkache/2` and
    ///   `openkache/1`, in descending version order.
    /// * `minimum_version` - Lowest protocol version this client will accept
    ///   after negotiation.
    ///
    /// # Returns
    ///
    /// A validated policy retained by the client builder.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Configuration`] when an identifier is malformed, the
    /// order is not strictly descending, a duplicate is present, the minimum
    /// is zero, or no offered version can satisfy the minimum.
    pub fn new(protocols: Vec<Vec<u8>>, minimum_version: u32) -> Result<Self> {
        if protocols.is_empty() {
            return Err(Error::configuration(
                "alpn.protocols",
                "must contain at least one protocol",
            ));
        }
        if minimum_version == 0 {
            return Err(Error::configuration(
                "alpn.minimum_version",
                "must be greater than zero",
            ));
        }

        let mut previous_version = None;
        let mut has_acceptable_version = false;
        for (index, protocol) in protocols.iter().enumerate() {
            let version = parse_alpn_version(protocol).map_err(|message| {
                Error::configuration(
                    "alpn.protocols",
                    format!("entry {index} is invalid: {message}"),
                )
            })?;
            if version >= minimum_version {
                has_acceptable_version = true;
            }
            if let Some(previous_version) = previous_version {
                if version >= previous_version {
                    return Err(Error::configuration(
                        "alpn.protocols",
                        "versions must be strictly descending with no duplicates",
                    ));
                }
            }
            previous_version = Some(version);
        }
        if !has_acceptable_version {
            return Err(Error::configuration(
                "alpn.minimum_version",
                "no offered protocol reaches the minimum version",
            ));
        }
        Ok(Self {
            protocols,
            minimum_version,
        })
    }

    /// Creates a policy from positive protocol version numbers.
    ///
    /// # Arguments
    ///
    /// * `versions` - Non-empty versions in strict descending order.
    /// * `minimum_version` - Lowest version accepted after negotiation.
    ///
    /// # Returns
    ///
    /// A validated policy with canonical `openkache/<version>` identifiers.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Configuration`] when the version list violates the
    /// same constraints as [`Self::new`].
    pub fn from_versions(versions: Vec<u32>, minimum_version: u32) -> Result<Self> {
        let protocols = versions
            .into_iter()
            .map(|version| format!("openkache/{version}").into_bytes())
            .collect();
        Self::new(protocols, minimum_version)
    }

    /// Returns the offered ALPN identifiers in negotiation order.
    pub fn protocols(&self) -> &[Vec<u8>] {
        &self.protocols
    }

    /// Returns the minimum negotiated protocol version accepted by the client.
    pub const fn minimum_version(&self) -> u32 {
        self.minimum_version
    }

    pub(crate) fn validate_negotiated(&self, protocol: &[u8]) -> Result<()> {
        if !self
            .protocols
            .iter()
            .any(|offered| offered.as_slice() == protocol)
        {
            return Err(Error::Connection(
                "server negotiated an ALPN protocol that was not offered".into(),
            ));
        }
        let version = parse_alpn_version(protocol).map_err(|message| {
            Error::Connection(format!(
                "server negotiated an invalid ALPN protocol: {message}"
            ))
        })?;
        if version < self.minimum_version {
            return Err(Error::Connection(format!(
                "server negotiated ALPN version {version}, below client minimum {}",
                self.minimum_version
            )));
        }
        if protocol != openkache_protocol::ALPN {
            return Err(Error::Connection(format!(
                "client does not implement negotiated ALPN {}",
                String::from_utf8_lossy(protocol)
            )));
        }
        Ok(())
    }
}

impl Default for AlpnPolicy {
    fn default() -> Self {
        let minimum_version = parse_alpn_version(openkache_protocol::ALPN)
            .expect("the generated OpenKache ALPN must contain a positive version");
        Self {
            protocols: vec![openkache_protocol::ALPN.to_vec()],
            minimum_version,
        }
    }
}

fn parse_alpn_version(protocol: &[u8]) -> std::result::Result<u32, &'static str> {
    const PREFIX: &[u8] = b"openkache/";
    let suffix = protocol
        .strip_prefix(PREFIX)
        .ok_or("must start with openkache/")?;
    if suffix.is_empty() {
        return Err("version is missing");
    }
    if suffix.len() > 1 && suffix[0] == b'0' {
        return Err("version must not contain leading zeroes");
    }
    if !suffix.iter().all(u8::is_ascii_digit) {
        return Err("version must contain only decimal digits");
    }
    let version = std::str::from_utf8(suffix)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or("version is outside the positive u32 range")?;
    (version > 0)
        .then_some(version)
        .ok_or("version must be greater than zero")
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
            connect: Duration::from_millis(crate::contract::DEFAULT_CONNECT_TIMEOUT_MILLISECONDS),
            request: Duration::from_millis(crate::contract::DEFAULT_REQUEST_TIMEOUT_MILLISECONDS),
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
        Self {
            max_attempts: crate::contract::DEFAULT_RETRY_MAX_ATTEMPTS,
        }
    }
}

/// Optional behavior for one set operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SetOptions {
    condition: SetCondition,
    expiration_mode: ExpirationMode,
    time_to_live_ms: Option<u64>,
    eviction_mode: EvictionMode,
}

impl SetOptions {
    /// Creates unconditional set behavior inheriting namespace expiration and eviction defaults.
    pub const fn new() -> Self {
        Self {
            condition: SetCondition::Any,
            expiration_mode: ExpirationMode::Inherit,
            time_to_live_ms: None,
            eviction_mode: EvictionMode::Inherit,
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
        self.expiration_mode = ExpirationMode::ExplicitTtl;
        self.time_to_live_ms = Some(milliseconds);
        self
    }

    /// Stores the item without a TTL-based expiration.
    pub const fn no_expiry(mut self) -> Self {
        self.expiration_mode = ExpirationMode::NoExpiry;
        self.time_to_live_ms = None;
        self
    }

    /// Resolves expiration from the selected namespace's current default.
    pub const fn inherit_expiration(mut self) -> Self {
        self.expiration_mode = ExpirationMode::Inherit;
        self.time_to_live_ms = None;
        self
    }

    /// Allows the namespace's capacity eviction algorithm to select this item.
    pub const fn evictable(mut self) -> Self {
        self.eviction_mode = EvictionMode::Evictable;
        self
    }

    /// Protects this item from capacity eviction.
    pub const fn eviction_protected(mut self) -> Self {
        self.eviction_mode = EvictionMode::EvictionProtected;
        self
    }

    /// Resolves eviction eligibility from the selected namespace's default.
    pub const fn inherit_eviction(mut self) -> Self {
        self.eviction_mode = EvictionMode::Inherit;
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

    pub(crate) fn into_wire_options(self) -> Result<SetWireOptions> {
        if self.time_to_live_ms == Some(0) {
            return Err(Error::configuration(
                "set.time_to_live_ms",
                "must be greater than zero",
            ));
        }
        Ok(SetWireOptions::with_policies(
            self.condition,
            self.expiration_mode,
            self.time_to_live_ms,
            self.eviction_mode,
        ))
    }

    #[cfg(feature = "ffi")]
    pub(crate) fn from_wire_options(options: SetWireOptions) -> Result<Self> {
        let mut converted = match options.condition {
            SetCondition::Any => Self::new(),
            SetCondition::IfAbsent => Self::new().if_absent(),
            SetCondition::IfPresent => Self::new().if_present(),
        };
        converted = match options.expiration_mode {
            ExpirationMode::Inherit => converted.inherit_expiration(),
            ExpirationMode::NoExpiry => converted.no_expiry(),
            ExpirationMode::ExplicitTtl => {
                converted.expires_after_millis(options.ttl_ms.ok_or_else(|| {
                    Error::configuration(
                        "set.time_to_live_ms",
                        "must be present with explicit TTL mode",
                    )
                })?)
            }
        };
        Ok(match options.eviction_mode {
            EvictionMode::Inherit => converted.inherit_eviction(),
            EvictionMode::Evictable => converted.evictable(),
            EvictionMode::EvictionProtected => converted.eviction_protected(),
        })
    }
}
