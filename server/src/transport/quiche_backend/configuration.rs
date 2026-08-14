use boring::pkey::PKey;
use boring::ssl::{SslContextBuilder, SslMethod, SslVerifyMode};
use boring::x509::X509;
use boring::x509::store::X509StoreBuilder;

use super::{NAME, ServerTlsConfig, TransportError};

const MAX_DATAGRAM_BYTES: usize = 65_535;
const MAX_BUFFERED_REQUEST_BYTES: usize = crate::protocol::max_request_frame_bytes() + 1;

pub(super) fn config(
    material: &ServerTlsConfig,
    max_concurrent_streams: usize,
) -> Result<quiche::Config, TransportError> {
    let certificate = X509::from_der(
        material
            .certificate_chain
            .first()
            .expect("validated TLS certificate chain"),
    )
    .map_err(|error| TransportError::backend(NAME, "certificate parsing", error))?;
    let private_key = PKey::private_key_from_der(material.private_key.secret_der())
        .map_err(|error| TransportError::backend(NAME, "private key parsing", error))?;
    let mut tls = SslContextBuilder::new(SslMethod::tls())
        .map_err(|error| TransportError::backend(NAME, "TLS configuration", error))?;
    tls.set_certificate(&certificate)
        .map_err(|error| TransportError::backend(NAME, "TLS certificate", error))?;
    for certificate in material.certificate_chain.iter().skip(1) {
        let certificate = X509::from_der(certificate)
            .map_err(|error| TransportError::backend(NAME, "certificate parsing", error))?;
        tls.add_extra_chain_cert(certificate)
            .map_err(|error| TransportError::backend(NAME, "TLS certificate chain", error))?;
    }
    tls.set_private_key(&private_key)
        .map_err(|error| TransportError::backend(NAME, "TLS private key", error))?;
    if !material.client_ca.is_empty() {
        let mut roots = X509StoreBuilder::new()
            .map_err(|error| TransportError::backend(NAME, "client CA store", error))?;
        for certificate in &material.client_ca {
            let certificate = X509::from_der(certificate).map_err(|error| {
                TransportError::backend(NAME, "client CA certificate parsing", error)
            })?;
            tls.add_client_ca(&certificate)
                .map_err(|error| TransportError::backend(NAME, "client CA names", error))?;
            roots
                .add_cert(certificate)
                .map_err(|error| TransportError::backend(NAME, "client CA store", error))?;
        }
        tls.set_verify_cert_store(roots.build())
            .map_err(|error| TransportError::backend(NAME, "client CA store", error))?;
        tls.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
        tls.set_session_id_context(b"openkache-mtls")
            .map_err(|error| TransportError::backend(NAME, "TLS session identity", error))?;
    }
    let mut config = quiche::Config::with_boring_ssl_ctx_builder(quiche::PROTOCOL_VERSION, tls)
        .map_err(|error| TransportError::backend(NAME, "configuration", error))?;
    config
        .set_application_protos(&[openkache_protocol::ALPN])
        .map_err(|error| TransportError::backend(NAME, "ALPN", error))?;
    config.set_max_idle_timeout(30_000);
    config.set_max_recv_udp_payload_size(MAX_DATAGRAM_BYTES);
    config.set_max_send_udp_payload_size(1_350);
    config.set_initial_max_data(64 * 1024 * 1024);
    config.set_initial_max_stream_data_bidi_remote(MAX_BUFFERED_REQUEST_BYTES as u64);
    config.set_initial_max_stream_data_bidi_local(64 * 1024);
    config.set_initial_max_streams_bidi(max_concurrent_streams as u64);
    config.set_initial_max_streams_uni(0);
    config.set_disable_active_migration(true);
    Ok(config)
}
