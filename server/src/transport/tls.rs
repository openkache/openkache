//! Provider-neutral TLS conformance boundary.
//!
//! OpenKache transport profiles are intentionally narrower than the defaults
//! of any one TLS implementation.  The boundary in this module is the only
//! place where the provider is selected: callers receive a TLS 1.3
//! configuration whose key-exchange list contains exactly the approved
//! `X25519MLKEM768` hybrid group.  Keeping that list singleton is what makes a
//! classical-only retry impossible.

use std::sync::Arc;

use super::ServerTlsConfig;
use rustls::crypto::{CryptoProvider, SupportedKxGroup};

/// The one ALPN identifier implemented by the v1 wire profile.
pub(crate) const ALPN: &[u8] = openkache_protocol::ALPN;

/// The required TLS 1.3 hybrid key-exchange group.
pub(crate) const PQ_GROUP: rustls::NamedGroup = rustls::NamedGroup::X25519MLKEM768;

/// A stable diagnostic for a provider/backend that cannot enforce the profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Conformance {
    /// The provider can enforce TLS 1.3, ALPN v1, and the approved hybrid group.
    Conforming,
    /// The provider is available but cannot meet the profile's PQ requirement.
    Unsupported(&'static str),
}

impl Conformance {
    pub(crate) const fn reason(self) -> Option<&'static str> {
        match self {
            Self::Conforming => None,
            Self::Unsupported(reason) => Some(reason),
        }
    }
}

/// Returns the provider used by every Rustls-backed profile.
///
/// Rustls enables the hybrid group by default with `prefer-post-quantum`, but
/// still advertises classical groups for interoperability.  OpenKache's
/// security contract is stricter: retain only the approved group so a peer
/// that offers classical key exchange cannot silently downgrade.
pub(crate) fn strict_pq_provider() -> Arc<CryptoProvider> {
    let mut provider = rustls::crypto::aws_lc_rs::default_provider();
    provider.kx_groups.retain(|group| group.name() == PQ_GROUP);
    debug_assert_eq!(provider.kx_groups.len(), 1);
    Arc::new(provider)
}

/// Builds a TLS 1.3-only server configuration for a QUIC or TCP profile.
pub(crate) fn strict_server_config(
    material: &ServerTlsConfig,
) -> Result<rustls::ServerConfig, rustls::Error> {
    let provider = strict_pq_provider();
    let builder = rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&[&rustls::version::TLS13])?;
    let mut config = if material.client_ca.is_empty() {
        builder.with_no_client_auth()
    } else {
        let mut roots = rustls::RootCertStore::empty();
        for certificate in &material.client_ca {
            roots.add(certificate.clone())?;
        }
        let verifier =
            rustls::server::WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider)
                .build()
                .map_err(|error| rustls::Error::General(error.to_string()))?;
        builder.with_client_cert_verifier(verifier)
    }
    .with_single_cert(
        material.certificate_chain.clone(),
        material.private_key.clone_key(),
    )?;
    // A resumed TLS 1.3 session has no fresh key-exchange group to validate.
    // Disable tickets and server-side session storage so every accepted lane
    // performs the mandatory hybrid exchange.
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(config)
}

/// Confirms the negotiated TLS properties after a stream handshake.
///
/// A Rustls-backed backend should call this before exposing application bytes.
/// QUIC adapters cannot access the inner Rustls connection after handing it to
/// the QUIC state machine, so they enforce the same requirement by supplying
/// the singleton provider above.
pub(crate) fn validate_negotiated(
    connection: &rustls::ServerConnection,
) -> Result<(), ConformanceError> {
    if connection.protocol_version() != Some(rustls::ProtocolVersion::TLSv1_3) {
        return Err(ConformanceError::ProtocolVersion);
    }
    if connection.alpn_protocol() != Some(ALPN) {
        return Err(ConformanceError::Alpn);
    }
    let group = connection
        .negotiated_key_exchange_group()
        .map(SupportedKxGroup::name)
        .ok_or(ConformanceError::KeyExchange)?;
    if group != PQ_GROUP {
        return Err(ConformanceError::KeyExchange);
    }
    Ok(())
}

/// TLS profile mismatch discovered after the handshake.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ConformanceError {
    #[error("TLS peer negotiated a protocol other than TLS 1.3")]
    ProtocolVersion,
    #[error("TLS peer did not negotiate ALPN openkache/1")]
    Alpn,
    #[error("TLS peer did not negotiate X25519MLKEM768")]
    KeyExchange,
}
