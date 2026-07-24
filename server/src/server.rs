//! Minimal QUIC server backed by an in-memory hash map.

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use openkache_protocol::{
    ALPN, MAX_REQUEST_FRAME_BYTES, Opcode, ProtocolError, Request, Response, Status,
};
use quinn::{Endpoint, VarInt};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::sync::RwLock;

/// In-memory storage used by the first runnable OpenKache server.
#[derive(Default)]
pub struct MemoryStore {
    values: RwLock<HashMap<Vec<u8>, Vec<u8>>>,
    syncs: AtomicU64,
}

impl MemoryStore {
    async fn execute(&self, request: Request) -> Response {
        match request.opcode {
            Opcode::Ping => response(Status::Ok, b"PONG".to_vec()),
            Opcode::Get => {
                let values = self.values.read().await;
                match values.get(&request.key) {
                    Some(value) => response(Status::Ok, value.clone()),
                    None => response(Status::NotFound, Vec::new()),
                }
            }
            Opcode::Set => {
                let mut values = self.values.write().await;
                let status = if values.insert(request.key, request.value).is_some() {
                    Status::Replaced
                } else {
                    Status::Created
                };
                response(status, Vec::new())
            }
            Opcode::Delete => {
                let mut values = self.values.write().await;
                let status = if values.remove(&request.key).is_some() {
                    Status::Deleted
                } else {
                    Status::NotFound
                };
                response(status, Vec::new())
            }
            Opcode::Stats => {
                let keys = self.values.read().await.len();
                let syncs = self.syncs.load(Ordering::Relaxed);
                response(
                    Status::Ok,
                    format!(r#"{{"keys":{keys},"syncs":{syncs},"storage":"memory"}}"#).into_bytes(),
                )
            }
            Opcode::Sync => {
                self.syncs.fetch_add(1, Ordering::Relaxed);
                response(Status::Ok, Vec::new())
            }
        }
    }
}

/// A bound QUIC endpoint and its in-memory cache.
pub struct KacheServer {
    endpoint: Endpoint,
    certificate_der: CertificateDer<'static>,
    store: Arc<MemoryStore>,
}

impl KacheServer {
    /// Binds a server using an ephemeral self-signed certificate for `localhost`.
    pub fn bind(address: SocketAddr) -> Result<Self> {
        let generated = rcgen::generate_simple_self_signed(["localhost".to_string()])?;
        let certificate_der = generated.cert.der().clone();
        let private_key_der = PrivatePkcs8KeyDer::from(generated.signing_key.serialize_der());

        let mut tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate_der.clone()],
                PrivateKeyDer::Pkcs8(private_key_der),
            )?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls)?;
        let endpoint =
            Endpoint::server(quinn::ServerConfig::with_crypto(Arc::new(crypto)), address)?;
        Ok(Self {
            endpoint,
            certificate_der,
            store: Arc::new(MemoryStore::default()),
        })
    }

    /// Returns the UDP address selected by the operating system.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    /// Returns the self-signed certificate clients must trust for this run.
    pub fn certificate_der(&self) -> &[u8] {
        self.certificate_der.as_ref()
    }

    /// Accepts connections until `shutdown` resolves.
    pub async fn serve(self, shutdown: impl Future<Output = ()>) -> Result<()> {
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                incoming = self.endpoint.accept() => {
                    let Some(incoming) = incoming else {
                        break;
                    };
                    let store = Arc::clone(&self.store);
                    tokio::spawn(async move {
                        if let Ok(connection) = incoming.await {
                            serve_connection(connection, store).await;
                        }
                    });
                }
                () = &mut shutdown => break,
            }
        }
        self.endpoint
            .close(VarInt::from_u32(0), b"server shutting down");
        self.endpoint.wait_idle().await;
        Ok(())
    }
}

async fn serve_connection(connection: quinn::Connection, store: Arc<MemoryStore>) {
    while let Ok((send, receive)) = connection.accept_bi().await {
        serve_stream(send, receive, &store).await;
    }
}

async fn serve_stream(
    mut send: quinn::SendStream,
    mut receive: quinn::RecvStream,
    store: &MemoryStore,
) {
    let response = match receive.read_to_end(MAX_REQUEST_FRAME_BYTES).await {
        Ok(frame) => match Request::decode(&frame) {
            Ok(request) => store.execute(request).await,
            Err(error) => protocol_error_response(error),
        },
        Err(quinn::ReadToEndError::TooLong) => response(
            Status::TooLarge,
            b"request exceeds the protocol limit".to_vec(),
        ),
        Err(error) => response(Status::InvalidRequest, error.to_string().into_bytes()),
    };
    let Ok(frame) = response.encode() else {
        return;
    };
    if send.write_all(&frame).await.is_ok() {
        let _ = send.finish();
    }
}

fn protocol_error_response(error: ProtocolError) -> Response {
    let status = match error {
        ProtocolError::UnknownOpcode(_) => Status::UnsupportedOpcode,
        ProtocolError::KeyTooLarge { .. } | ProtocolError::ValueTooLarge { .. } => Status::TooLarge,
        _ => Status::InvalidRequest,
    };
    response(status, error.to_string().into_bytes())
}

fn response(status: Status, payload: Vec<u8>) -> Response {
    Response::new(status, payload).expect("server responses stay within protocol limits")
}

/// Errors produced while configuring or running the local QUIC server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("certificate generation failed: {0}")]
    Certificate(#[from] rcgen::Error),
    #[error("TLS configuration failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("QUIC TLS configuration failed: {0}")]
    QuicTls(#[from] quinn::crypto::rustls::NoInitialCipherSuite),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience result type for server lifecycle operations.
pub type Result<T> = std::result::Result<T, ServerError>;
