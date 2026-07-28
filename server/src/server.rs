//! QUIC server backed by the sharded SSD-first cache runtime.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use compio::BufResult;
use compio::io::{AsyncReadExt, AsyncWriteExt};
use compio_quic::{Endpoint, VarInt};
use futures_util::stream::{FuturesUnordered, StreamExt};
use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::{
    ALPN, MAX_REQUEST_FRAME_BYTES, Opcode, ProtocolError, Request, Response, Status, ValueFlags,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::{AppConfig, KvError, SetOutcome, ThreadedKvkache};

/// A bound QUIC endpoint and its sharded SSD-backed cache.
pub struct KacheServer {
    endpoint: Endpoint,
    certificate_der: CertificateDer<'static>,
    cache: ThreadedKvkache,
    request_timeout: Duration,
    max_inflight_streams_per_connection: usize,
}

impl KacheServer {
    /// Binds a server with the default SSD cache configuration.
    ///
    /// # Arguments
    ///
    /// * `address` - UDP address on which the QUIC endpoint listens.
    ///
    /// # Returns
    ///
    /// A ready server containing its bound endpoint, generated certificate, and cache workers.
    ///
    /// # Errors
    ///
    /// Returns an error when certificate generation, QUIC binding, or cache startup fails.
    pub async fn bind(address: SocketAddr) -> Result<Self> {
        Self::bind_with_config(address, AppConfig::default()).await
    }

    /// Binds a server with an explicit SSD cache configuration.
    ///
    /// # Arguments
    ///
    /// * `address` - UDP address on which the QUIC endpoint listens.
    /// * `config` - Worker, storage, table, and timeout configuration for the cache.
    ///
    /// # Returns
    ///
    /// A ready server containing its bound endpoint, generated certificate, and cache workers.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration validation, certificate generation, QUIC binding, or
    /// cache startup fails.
    pub async fn bind_with_config(address: SocketAddr, config: AppConfig) -> Result<Self> {
        config.validate()?;
        let request_timeout = Duration::from_micros(config.timeouts.request_max_time_us);
        let max_inflight_streams_per_connection = config.io_uring.max_inflight_per_worker;
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
        let crypto = compio_quic::crypto::rustls::QuicServerConfig::try_from(tls)?;
        let socket = compio::net::UdpSocket::bind(address).await?;
        let endpoint = Endpoint::new(
            socket,
            compio_quic::EndpointConfig::default(),
            Some(compio_quic::ServerConfig::with_crypto(Arc::new(crypto))),
            None,
        )?;
        let cache = ThreadedKvkache::start(config)?;
        Ok(Self {
            endpoint,
            certificate_der,
            cache,
            request_timeout,
            max_inflight_streams_per_connection,
        })
    }

    /// Returns the UDP address selected by the operating system.
    ///
    /// # Returns
    ///
    /// The bound local address for this server endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when the endpoint cannot report its local address.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    /// Returns the self-signed certificate clients must trust for this run.
    ///
    /// # Returns
    ///
    /// The generated certificate encoded as DER bytes.
    pub fn certificate_der(&self) -> &[u8] {
        self.certificate_der.as_ref()
    }

    /// Accepts connections until `shutdown` resolves, then flushes all cache workers.
    ///
    /// # Arguments
    ///
    /// * `shutdown` - Future whose completion initiates graceful server shutdown.
    ///
    /// # Returns
    ///
    /// `Ok(())` after active connections close and all cache workers flush and stop.
    ///
    /// # Errors
    ///
    /// Returns an error when endpoint shutdown or cache worker shutdown fails.
    pub async fn serve(mut self, shutdown: impl Future<Output = ()>) -> Result<()> {
        let shutdown = shutdown.fuse();
        pin_mut!(shutdown);
        let mut connections = FuturesUnordered::new();

        loop {
            if connections.is_empty() {
                let incoming = self.endpoint.wait_incoming().fuse();
                pin_mut!(incoming);
                select! {
                    incoming = incoming => {
                        let Some(incoming) = incoming else {
                            break;
                        };
                        connections.push(serve_incoming(
                            incoming,
                            &self.cache,
                            self.request_timeout,
                            self.max_inflight_streams_per_connection,
                        ));
                    }
                    () = &mut shutdown => break,
                }
            } else {
                let incoming = self.endpoint.wait_incoming().fuse();
                let completed = connections.next().fuse();
                pin_mut!(incoming, completed);
                select! {
                    incoming = incoming => {
                        let Some(incoming) = incoming else {
                            break;
                        };
                        connections.push(serve_incoming(
                            incoming,
                            &self.cache,
                            self.request_timeout,
                            self.max_inflight_streams_per_connection,
                        ));
                    }
                    _ = completed => {}
                    () = &mut shutdown => break,
                }
            }
        }

        self.endpoint
            .close(VarInt::from_u32(0), b"server shutting down");
        self.endpoint.shutdown().await?;
        while connections.next().await.is_some() {}
        drop(connections);
        self.cache.shutdown()?;
        Ok(())
    }
}

/// Completes one QUIC handshake and serves the accepted connection.
async fn serve_incoming(
    incoming: compio_quic::Incoming,
    cache: &ThreadedKvkache,
    request_timeout: Duration,
    max_inflight_streams: usize,
) {
    if let Ok(connection) = incoming.await {
        serve_connection(connection, cache, request_timeout, max_inflight_streams).await;
    }
}

/// Multiplexes bounded concurrent request streams for one QUIC connection.
async fn serve_connection(
    connection: compio_quic::Connection,
    cache: &ThreadedKvkache,
    request_timeout: Duration,
    max_inflight_streams: usize,
) {
    let mut streams = FuturesUnordered::new();
    loop {
        if streams.len() >= max_inflight_streams {
            let _ = streams.next().await;
            continue;
        }
        if streams.is_empty() {
            match connection.accept_bi().await {
                Ok((send, receive)) => {
                    streams.push(serve_stream(send, receive, cache, request_timeout));
                }
                Err(_) => break,
            }
        } else {
            let incoming = connection.accept_bi().fuse();
            let completed = streams.next().fuse();
            pin_mut!(incoming, completed);
            select! {
                incoming = incoming => match incoming {
                    Ok((send, receive)) => {
                        streams.push(serve_stream(send, receive, cache, request_timeout));
                    }
                    Err(_) => break,
                },
                _ = completed => {}
            }
        }
    }
    while streams.next().await.is_some() {}
}

/// Reads, executes, and responds to one request stream within the configured timeout.
async fn serve_stream(
    mut send: compio_quic::SendStream,
    receive: compio_quic::RecvStream,
    cache: &ThreadedKvkache,
    request_timeout: Duration,
) {
    let mut receive = receive.take((MAX_REQUEST_FRAME_BYTES + 1) as u64);
    let read =
        compio::runtime::time::timeout(request_timeout, receive.read_to_end(Vec::new())).await;
    let response = match read {
        Err(_) => response(Status::Timeout, b"request read timed out".to_vec()),
        Ok(BufResult(Ok(_), frame)) if frame.len() > MAX_REQUEST_FRAME_BYTES => response(
            Status::TooLarge,
            b"request exceeds the protocol limit".to_vec(),
        ),
        Ok(BufResult(Ok(_), frame)) => match Request::decode(&frame) {
            Ok(request) => execute_request(cache, request).await,
            Err(error) => protocol_error_response(error),
        },
        Ok(BufResult(Err(error), _)) => {
            response(Status::InvalidRequest, error.to_string().into_bytes())
        }
    };
    let Ok(frame) = response.encode() else {
        return;
    };
    let Ok(BufResult(write_result, _)) =
        compio::runtime::time::timeout(request_timeout, send.write_all(frame)).await
    else {
        return;
    };
    if write_result.is_ok() {
        let _ = send.finish();
    }
}

/// Dispatches a decoded protocol request to the SSD-backed worker runtime.
async fn execute_request(cache: &ThreadedKvkache, request: Request) -> Response {
    let Request {
        opcode,
        client_key_digest,
        value_flags,
        value,
    } = request;
    let result = match opcode {
        Opcode::Ping => return response(Status::Ok, b"PONG".to_vec()),
        Opcode::Get => cache
            .get_async(client_key_digest.expect("GET requests have a validated key digest"))
            .await
            .map(|value| match value {
                Some(value) => response_with_value_flags(Status::Ok, value.flags, value.bytes),
                None => response(Status::NotFound, Vec::new()),
            }),
        Opcode::Set => cache
            .set_async(
                client_key_digest.expect("SET requests have a validated key digest"),
                crate::types::EncodedValue::new(value, value_flags),
            )
            .await
            .map(|outcome| match outcome {
                SetOutcome::Created => response(Status::Created, Vec::new()),
                SetOutcome::Replaced => response(Status::Replaced, Vec::new()),
            }),
        Opcode::Delete => cache
            .delete_async(client_key_digest.expect("DELETE requests have a validated key digest"))
            .await
            .map(|deleted| {
                response(
                    if deleted {
                        Status::Deleted
                    } else {
                        Status::NotFound
                    },
                    Vec::new(),
                )
            }),
        Opcode::Stats => cache.stats_async().await.map(|workers| {
            let workers = workers
                .into_iter()
                .map(|worker| format!("{worker:?}"))
                .collect::<Vec<_>>()
                .join(",");
            response(
                Status::Ok,
                format!(r#"{{"storage":"ssd","workers":[{workers}]}}"#).into_bytes(),
            )
        }),
        Opcode::Sync => cache
            .sync_async()
            .await
            .map(|()| response(Status::Ok, Vec::new())),
    };
    result.unwrap_or_else(cache_error_response)
}

/// Maps cache failures to stable protocol statuses and messages.
fn cache_error_response(error: KvError) -> Response {
    let status = match error {
        KvError::Timeout(_) => Status::Timeout,
        KvError::TableFull => Status::Overloaded,
        KvError::ItemTooLarge { .. } | KvError::BlobSegmentFull { .. } => Status::TooLarge,
        KvError::Io(_) | KvError::InvalidConfig(_) | KvError::Worker(_) | KvError::Usage(_) => {
            Status::InternalError
        }
    };
    response(status, error.to_string().into_bytes())
}

/// Maps framing and validation failures to stable protocol statuses.
fn protocol_error_response(error: ProtocolError) -> Response {
    let status = match error {
        ProtocolError::UnknownOpcode(_) => Status::UnsupportedOpcode,
        ProtocolError::ValueTooLarge { .. } => Status::TooLarge,
        _ => Status::InvalidRequest,
    };
    response(status, error.to_string().into_bytes())
}

/// Constructs a protocol response whose payload is known to fit protocol limits.
fn response(status: Status, payload: Vec<u8>) -> Response {
    Response::new(status, payload).expect("server responses stay within protocol limits")
}

fn response_with_value_flags(
    status: Status,
    value_flags: ValueFlags,
    payload: Vec<u8>,
) -> Response {
    Response::new_with_value_flags(status, value_flags, payload)
        .expect("server responses stay within protocol limits")
}

/// Errors produced while configuring or running the QUIC server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("cache failed: {0}")]
    Cache(#[from] KvError),
    #[error("certificate generation failed: {0}")]
    Certificate(#[from] rcgen::Error),
    #[error("TLS configuration failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("QUIC TLS configuration failed: {0}")]
    QuicTls(#[from] compio_quic::crypto::rustls::NoInitialCipherSuite),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience result type for server lifecycle operations.
pub type Result<T> = std::result::Result<T, ServerError>;
