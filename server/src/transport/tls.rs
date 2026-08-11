//! TLS material and shared Rustls configuration.

#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// Parsed TLS material shared by every reuse-port endpoint.
pub(crate) struct ServerTlsConfig {
    pub(crate) certificate_chain: Vec<CertificateDer<'static>>,
    pub(crate) private_key: PrivateKeyDer<'static>,
    pub(crate) client_ca: Vec<CertificateDer<'static>>,
}

#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
pub(super) fn rustls_config(
    material: &ServerTlsConfig,
) -> Result<rustls::ServerConfig, rustls::Error> {
    let builder = rustls::ServerConfig::builder();
    let mut tls = if material.client_ca.is_empty() {
        builder.with_no_client_auth()
    } else {
        let mut roots = rustls::RootCertStore::empty();
        for certificate in &material.client_ca {
            roots.add(certificate.clone())?;
        }
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|error| rustls::Error::General(error.to_string()))?;
        builder.with_client_cert_verifier(verifier)
    }
    .with_single_cert(
        material.certificate_chain.clone(),
        material.private_key.clone_key(),
    )?;
    tls.alpn_protocols = vec![openkache_protocol::ALPN.to_vec()];
    Ok(tls)
}
