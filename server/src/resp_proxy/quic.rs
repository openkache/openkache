use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;

use super::resp_backend::RespBackend;
use openkache_protocol::{
    ALPN, MAX_REQUEST_FRAME_BYTES, OpaqueRequestFrame, Opcode, wire_request_layout,
};
use quinn::crypto::rustls::QuicServerConfig;
use quinn::{Connection, Endpoint, EndpointConfig, RecvStream, SendStream, TokioRuntime};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

const READ_CHUNK_BYTES: usize = 64 * 1024;
const MAX_CONCURRENT_STREAMS: u32 = 256;

pub(super) fn server_config() -> io::Result<quinn::ServerConfig> {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).map_err(io::Error::other)?;
    let certificate = CertificateDer::from(cert);
    let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(signing_key.serialize_der()));

    let mut provider = rustls::crypto::aws_lc_rs::default_provider();
    provider
        .kx_groups
        .retain(|group| group.name() == rustls::NamedGroup::X25519MLKEM768);
    if provider.kx_groups.len() != 1 {
        return Err(io::Error::other(
            "AWS-LC did not provide the required X25519MLKEM768 group",
        ));
    }

    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::<CryptoProvider>::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(vec![certificate], private_key)
        .map_err(io::Error::other)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    tls.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    tls.send_tls13_tickets = 0;

    let crypto = QuicServerConfig::try_from(tls).map_err(io::Error::other)?;
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(MAX_CONCURRENT_STREAMS.into());
    transport.max_concurrent_uni_streams(0_u8.into());

    let mut server = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    server.transport_config(Arc::new(transport));
    Ok(server)
}

pub(super) async fn serve(
    socket: UdpSocket,
    server_config: quinn::ServerConfig,
    resp_backend: SocketAddr,
) -> io::Result<()> {
    let endpoint = Endpoint::new(
        EndpointConfig::default(),
        Some(server_config),
        socket,
        Arc::new(TokioRuntime),
    )?;
    while let Some(incoming) = endpoint.accept().await {
        tokio::spawn(async move {
            let connection = match incoming.await {
                Ok(connection) => connection,
                Err(error) => {
                    eprintln!("native RESP proxy QUIC handshake failed: {error}");
                    return;
                }
            };

            serve_connection(connection, resp_backend).await;
        });
    }

    Ok(())
}

async fn serve_connection(connection: Connection, resp_backend: SocketAddr) {
    while let Ok((send, receive)) = connection.accept_bi().await {
        tokio::spawn(async move {
            if let Err(error) = serve_lane(send, receive, resp_backend).await {
                eprintln!("native RESP proxy lane failed: {error}");
            }
        });
    }
}

async fn serve_lane(
    mut send: SendStream,
    mut receive: RecvStream,
    resp_backend: SocketAddr,
) -> io::Result<()> {
    let mut reader = RequestReader::default();
    let mut backend = RespBackend::new(resp_backend);

    while let Some(frame) = reader.next(&mut receive).await? {
        let response = super::mapping::dispatch(&frame, &mut backend).await?;
        let encoded = response.into_encoded().map_err(io::Error::other)?;
        send.write_all(&encoded).await.map_err(io::Error::other)?;
    }

    send.finish().map_err(io::Error::other)
}

#[derive(Default)]
struct RequestReader {
    buffered: Vec<u8>,
}

impl RequestReader {
    async fn next(&mut self, receive: &mut RecvStream) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(frame) = self.try_take_frame()? {
                return Ok(Some(frame));
            }

            let chunk = receive
                .read_chunk(READ_CHUNK_BYTES, true)
                .await
                .map_err(io::Error::other)?;
            let Some(chunk) = chunk else {
                if self.buffered.is_empty() {
                    return Ok(None);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "native request stream ended in a partial frame",
                ));
            };

            if self.buffered.len().saturating_add(chunk.bytes.len()) > MAX_REQUEST_FRAME_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "native request exceeds the protocol frame limit",
                ));
            }
            self.buffered.extend_from_slice(&chunk.bytes);
        }
    }

    fn try_take_frame(&mut self) -> io::Result<Option<Vec<u8>>> {
        let Some(&opcode_byte) = self.buffered.first() else {
            return Ok(None);
        };
        let opcode = Opcode::try_from(opcode_byte).map_err(io::Error::other)?;
        let layout = wire_request_layout(opcode);
        let Some(header) =
            OpaqueRequestFrame::decode_header(&self.buffered, layout).map_err(io::Error::other)?
        else {
            return Ok(None);
        };
        let frame_len = header.frame_len().map_err(io::Error::other)?;
        if frame_len > MAX_REQUEST_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "native request exceeds the protocol frame limit",
            ));
        }
        if self.buffered.len() < frame_len {
            return Ok(None);
        }

        let remaining = self.buffered.split_off(frame_len);
        let frame = std::mem::replace(&mut self.buffered, remaining);
        Ok(Some(frame))
    }
}
