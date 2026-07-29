//! QUIC server backed by the sharded SSD-first cache runtime.

use std::collections::HashSet;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use compio::driver::ProactorBuilder;
use compio::runtime::RuntimeBuilder;
use futures_util::stream::{FuturesUnordered, StreamExt};
use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::{
    MAX_REQUEST_FRAME_BYTES, Opcode, ProtocolError, Request, Response, Status, ValueFlags,
};
use rustls::pki_types::CertificateDer;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::channel::{self, AsyncReceiver, Sender};
use crate::transport::{
    Connection as TransportConnection, Endpoint as TransportEndpoint,
    Incoming as TransportIncoming, ReceiveStream, SendStream, ServerEndpoint, StreamReadError,
    TransportError,
};
use crate::{AppConfig, KvError, NetworkConfig, QuicBackend, SetOutcome, ThreadedKvkache};

enum NetworkWorkerPhase {
    Starting,
    Running,
    Finished,
}

pub(crate) struct NetworkWorkerReporter {
    worker_id: usize,
    phase: NetworkWorkerPhase,
    started: Option<Sender<std::result::Result<(), String>>>,
    finished: Option<Sender<(usize, std::result::Result<(), String>)>>,
}

impl NetworkWorkerReporter {
    pub(crate) fn new(
        worker_id: usize,
        started: Sender<std::result::Result<(), String>>,
        finished: Sender<(usize, std::result::Result<(), String>)>,
    ) -> Self {
        Self {
            worker_id,
            phase: NetworkWorkerPhase::Starting,
            started: Some(started),
            finished: Some(finished),
        }
    }

    fn startup_failed(mut self, message: String) {
        self.phase = NetworkWorkerPhase::Finished;
        self.finished.take();
        if let Some(started) = self.started.take() {
            let _ = started.send(Err(message));
        }
    }

    pub(crate) fn started(&mut self) -> bool {
        let reported = self
            .started
            .take()
            .is_some_and(|started| started.send(Ok(())).is_ok());
        if reported {
            self.phase = NetworkWorkerPhase::Running;
        } else {
            self.phase = NetworkWorkerPhase::Finished;
            self.finished.take();
        }
        reported
    }

    fn finish(mut self, result: std::result::Result<(), String>) {
        self.phase = NetworkWorkerPhase::Finished;
        if let Some(finished) = self.finished.take() {
            let _ = finished.send((self.worker_id, result));
        }
    }
}

impl Drop for NetworkWorkerReporter {
    fn drop(&mut self) {
        match self.phase {
            NetworkWorkerPhase::Starting => {
                if let Some(started) = self.started.take() {
                    let failure = if std::thread::panicking() {
                        format!("network worker {} panicked during startup", self.worker_id)
                    } else {
                        format!(
                            "network worker {} exited without reporting startup",
                            self.worker_id
                        )
                    };
                    let _ = started.send(Err(failure));
                }
            }
            NetworkWorkerPhase::Running => {
                if let Some(finished) = self.finished.take() {
                    let failure = if std::thread::panicking() {
                        "panicked"
                    } else {
                        "exited without reporting completion"
                    };
                    let _ = finished.send((self.worker_id, Err(failure.into())));
                }
            }
            NetworkWorkerPhase::Finished => {}
        }
    }
}

/// Bound reuse-port sockets and the sharded SSD-backed cache they serve.
pub struct KacheServer {
    sockets: Vec<std::net::UdpSocket>,
    local_addr: SocketAddr,
    quic_backend: QuicBackend,
    certificate_der: CertificateDer<'static>,
    private_key_der: Vec<u8>,
    cache: Arc<ThreadedKvkache>,
    network: NetworkConfig,
    request_timeout: Duration,
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
    /// A ready server containing bound sockets, a generated certificate, and cache workers.
    ///
    /// # Errors
    ///
    /// Returns an error when certificate generation, socket binding, or cache startup fails.
    pub async fn bind(address: SocketAddr) -> Result<Self> {
        Self::bind_with_config(address, AppConfig::default()).await
    }

    /// Binds a server with an explicit SSD cache configuration.
    ///
    /// # Arguments
    ///
    /// * `address` - UDP address on which the QUIC endpoint listens.
    /// * `config` - Network, storage, table, and timeout configuration.
    ///
    /// # Returns
    ///
    /// A ready server containing bound sockets, a generated certificate, and cache workers.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration validation, certificate generation, socket binding, or
    /// cache startup fails.
    pub async fn bind_with_config(address: SocketAddr, config: AppConfig) -> Result<Self> {
        config.validate()?;
        let request_timeout = Duration::from_micros(config.timeouts.request_max_time_us);
        let network = config.network.clone();
        let quic_backend = config.quic.selected_backend()?;
        ServerEndpoint::validate_backend(quic_backend)?;
        let generated = rcgen::generate_simple_self_signed(["localhost".to_string()])?;
        let certificate_der = generated.cert.der().clone();
        let private_key_der = generated.signing_key.serialize_der();
        let sockets = bind_reuse_port_sockets(address, network.worker_count)?;
        let local_addr = sockets[0].local_addr()?;
        let cache = Arc::new(ThreadedKvkache::start(config)?);
        Ok(Self {
            sockets,
            local_addr,
            quic_backend,
            certificate_der,
            private_key_der,
            cache,
            network,
            request_timeout,
        })
    }

    /// Returns the UDP address selected by the operating system.
    ///
    /// # Returns
    ///
    /// The bound local address shared by all reuse-port sockets.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored socket address cannot be reported.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
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
    /// Returns an error when a network worker fails or cache shutdown fails.
    pub async fn serve(self, shutdown: impl Future<Output = ()>) -> Result<()> {
        let Self {
            sockets,
            quic_backend,
            certificate_der,
            private_key_der,
            cache,
            network,
            request_timeout,
            ..
        } = self;
        let (started_tx, started_rx) =
            channel::bounded::<std::result::Result<(), String>>(network.worker_count);
        let (finished_tx, finished_rx) = channel::bounded_sync_async::<(
            usize,
            std::result::Result<(), String>,
        )>(network.worker_count);
        let mut workers = Vec::with_capacity(network.worker_count);

        for (worker_id, socket) in sockets.into_iter().enumerate() {
            let (stop_tx, stop_rx) = channel::bounded_sync_async(1);
            let started_tx = started_tx.clone();
            let finished_tx = finished_tx.clone();
            let certificate_der = certificate_der.clone();
            let private_key_der = private_key_der.clone();
            let worker_cache = Arc::clone(&cache);
            let cpu_id = network.cpu_ids[worker_id];
            let entries = network.io_uring_entries_per_worker;
            let event_interval = network.event_interval;
            let max_stream_lanes = network.max_stream_lanes_per_connection;
            let thread = match std::thread::Builder::new()
                .name(format!("openkache-network-{worker_id}"))
                .spawn(move || {
                    let mut reporter =
                        NetworkWorkerReporter::new(worker_id, started_tx, finished_tx);
                    let mut proactor = ProactorBuilder::new();
                    proactor.capacity(entries);
                    let mut builder = RuntimeBuilder::new();
                    builder
                        .with_proactor(proactor)
                        .thread_affinity(HashSet::from([cpu_id]))
                        .event_interval(event_interval);
                    let runtime = match builder.build() {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            reporter.startup_failed(error.to_string());
                            return;
                        }
                    };
                    runtime.block_on(async move {
                        let endpoint = match ServerEndpoint::bind(
                            quic_backend,
                            socket,
                            certificate_der.as_ref(),
                            &private_key_der,
                            max_stream_lanes,
                        )
                        .await
                        {
                            Ok(endpoint) => endpoint,
                            Err(error) => {
                                reporter.startup_failed(error.to_string());
                                return;
                            }
                        };
                        let actual_cpu = unsafe { libc::sched_getcpu() };
                        if actual_cpu < 0 || actual_cpu as usize != cpu_id {
                            reporter.startup_failed(format!(
                                "network worker {worker_id} expected CPU {cpu_id}, running on CPU {actual_cpu}"
                            ));
                            return;
                        }
                        if !reporter.started() {
                            return;
                        }
                        let result = run_selected_endpoint(
                            endpoint,
                            &worker_cache,
                            request_timeout,
                            max_stream_lanes,
                            stop_rx,
                        )
                        .await
                        .map_err(|error| error.to_string());
                        reporter.finish(result);
                    });
                }) {
                Ok(thread) => thread,
                Err(error) => {
                    shutdown_workers_and_cache(workers, cache)?;
                    return Err(error.into());
                }
            };
            workers.push((stop_tx, thread));
        }
        drop(started_tx);
        drop(finished_tx);

        for _ in 0..network.worker_count {
            match started_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(message)) => {
                    shutdown_workers_and_cache(workers, cache)?;
                    return Err(ServerError::NetworkWorker(message));
                }
                Err(_) => {
                    shutdown_workers_and_cache(workers, cache)?;
                    return Err(ServerError::NetworkWorker(
                        "network worker startup channel closed".into(),
                    ));
                }
            }
        }

        let shutdown = shutdown.fuse();
        let worker_finished = finished_rx.recv_async().fuse();
        pin_mut!(shutdown, worker_finished);
        let worker_failure = select! {
            () = shutdown => None,
            result = worker_finished => Some(match result {
                Ok((worker_id, Ok(()))) => {
                    format!("network worker {worker_id} exited unexpectedly")
                }
                Ok((worker_id, Err(message))) => {
                    format!("network worker {worker_id} failed: {message}")
                }
                Err(_) => "network worker completion channel closed".into(),
            }),
        };
        shutdown_workers_and_cache(workers, cache)?;
        match worker_failure {
            Some(message) => Err(ServerError::NetworkWorker(message)),
            None => Ok(()),
        }
    }
}

fn bind_reuse_port_sockets(
    address: SocketAddr,
    worker_count: usize,
) -> std::io::Result<Vec<std::net::UdpSocket>> {
    let mut sockets = Vec::with_capacity(worker_count);
    let mut bind_address = address;
    for worker_id in 0..worker_count {
        let socket = Socket::new(
            Domain::for_address(bind_address),
            Type::DGRAM,
            Some(Protocol::UDP),
        )?;
        socket.set_reuse_address(true)?;
        socket.set_reuse_port(true)?;
        socket.bind(&SockAddr::from(bind_address))?;
        let socket = std::net::UdpSocket::from(socket);
        if worker_id == 0 {
            bind_address = socket.local_addr()?;
        }
        sockets.push(socket);
    }
    Ok(sockets)
}

fn shutdown_workers_and_cache(
    workers: Vec<(Sender<()>, std::thread::JoinHandle<()>)>,
    cache: Arc<ThreadedKvkache>,
) -> Result<()> {
    let network_result = stop_network_workers(workers);
    let cache_result = shutdown_cache(cache);
    network_result?;
    cache_result
}

pub(crate) fn stop_network_workers(
    workers: Vec<(Sender<()>, std::thread::JoinHandle<()>)>,
) -> Result<()> {
    for (stop, _) in &workers {
        let _ = stop.send(());
    }
    let mut panicked_worker = None;
    for (_, thread) in workers {
        let name = thread
            .thread()
            .name()
            .unwrap_or("network worker")
            .to_owned();
        if thread.join().is_err() && panicked_worker.is_none() {
            panicked_worker = Some(name);
        }
    }
    if let Some(name) = panicked_worker {
        return Err(ServerError::NetworkWorker(format!("{name} panicked")));
    }
    Ok(())
}

fn shutdown_cache(cache: Arc<ThreadedKvkache>) -> Result<()> {
    let mut cache = Arc::try_unwrap(cache)
        .map_err(|_| ServerError::NetworkWorker("network cache handle leaked".into()))?;
    cache.shutdown()?;
    Ok(())
}

async fn run_selected_endpoint(
    endpoint: ServerEndpoint,
    cache: &ThreadedKvkache,
    request_timeout: Duration,
    max_stream_lanes: usize,
    stop: AsyncReceiver<()>,
) -> std::result::Result<(), TransportError> {
    match endpoint {
        #[cfg(feature = "quic-quinn")]
        ServerEndpoint::Quinn(endpoint) => {
            run_network_worker(endpoint, cache, request_timeout, max_stream_lanes, stop).await
        }
        #[cfg(feature = "quic-noq")]
        ServerEndpoint::Noq(endpoint) => {
            run_network_worker(endpoint, cache, request_timeout, max_stream_lanes, stop).await
        }
        #[cfg(feature = "quic-quiche")]
        ServerEndpoint::Quiche(endpoint) => {
            run_network_worker(endpoint, cache, request_timeout, max_stream_lanes, stop).await
        }
    }
}

async fn run_network_worker<E: TransportEndpoint>(
    endpoint: E,
    cache: &ThreadedKvkache,
    request_timeout: Duration,
    max_stream_lanes: usize,
    stop: AsyncReceiver<()>,
) -> std::result::Result<(), TransportError> {
    let mut connections = FuturesUnordered::new();
    loop {
        if connections.is_empty() {
            let incoming = endpoint.wait_incoming().fuse();
            let stopping = stop.recv_async().fuse();
            pin_mut!(incoming, stopping);
            select! {
                incoming = incoming => {
                    let Some(incoming) = incoming else { break };
                    connections.push(serve_incoming(
                        incoming, cache, request_timeout, max_stream_lanes,
                    ));
                }
                _ = stopping => break,
            }
        } else {
            let incoming = endpoint.wait_incoming().fuse();
            let completed = connections.next().fuse();
            let stopping = stop.recv_async().fuse();
            pin_mut!(incoming, completed, stopping);
            select! {
                incoming = incoming => {
                    let Some(incoming) = incoming else { break };
                    connections.push(serve_incoming(
                        incoming, cache, request_timeout, max_stream_lanes,
                    ));
                }
                _ = completed => {}
                _ = stopping => break,
            }
        }
    }
    endpoint.close(b"server shutting down");
    while connections.next().await.is_some() {}
    endpoint.shutdown().await
}

/// Completes one QUIC handshake and serves the accepted connection.
async fn serve_incoming<I: TransportIncoming>(
    incoming: I,
    cache: &ThreadedKvkache,
    request_timeout: Duration,
    max_stream_lanes: usize,
) {
    if let Ok(connection) = incoming.connect().await {
        serve_connection(connection, cache, request_timeout, max_stream_lanes).await;
    }
}

/// Multiplexes bounded reusable request lanes for one QUIC connection.
async fn serve_connection<C: TransportConnection>(
    connection: C,
    cache: &ThreadedKvkache,
    request_timeout: Duration,
    max_stream_lanes: usize,
) {
    let mut streams = FuturesUnordered::new();
    loop {
        if streams.len() >= max_stream_lanes {
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

/// Reuses one QUIC stream as a sequential request lane until either peer closes it.
async fn serve_stream<S: SendStream, R: ReceiveStream>(
    mut send: S,
    mut receive: R,
    cache: &ThreadedKvkache,
    request_timeout: Duration,
) {
    loop {
        let frame = match receive
            .read_request(MAX_REQUEST_FRAME_BYTES, request_timeout)
            .await
        {
            Ok(frame) => frame,
            Err(StreamReadError::Timeout) => {
                let _ = write_response(
                    &mut send,
                    response(Status::Timeout, b"request read timed out".to_vec()),
                    request_timeout,
                )
                .await;
                break;
            }
            Err(StreamReadError::TooLarge) => {
                let _ = write_response(
                    &mut send,
                    response(
                        Status::TooLarge,
                        b"request exceeds the protocol limit".to_vec(),
                    ),
                    request_timeout,
                )
                .await;
                break;
            }
            Err(StreamReadError::Protocol(error)) => {
                let _ = write_response(&mut send, protocol_error_response(error), request_timeout)
                    .await;
                break;
            }
            Err(StreamReadError::Transport(_)) => break,
        };
        let response = match Request::decode(&frame) {
            Ok(request) => {
                match compio::runtime::time::timeout(
                    request_timeout,
                    execute_request(cache, request),
                )
                .await
                {
                    Ok(response) => response,
                    Err(_) => response(Status::Timeout, b"request execution timed out".to_vec()),
                }
            }
            Err(error) => protocol_error_response(error),
        };
        if !write_response(&mut send, response, request_timeout).await {
            break;
        }
    }
}

async fn write_response<S: SendStream>(
    send: &mut S,
    response: Response,
    request_timeout: Duration,
) -> bool {
    let Ok(frame) = response.encode() else {
        return false;
    };
    send.write_response(frame, request_timeout).await.is_ok()
}

/// Dispatches a decoded protocol request to the SSD-backed worker runtime.
async fn execute_request(cache: &ThreadedKvkache, request: Request) -> Response {
    let Request {
        opcode,
        client_key_digest,
        value_flags,
        set_options,
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
            .set_async_with_options(
                client_key_digest.expect("SET requests have a validated key digest"),
                crate::types::EncodedValue::new(value, value_flags),
                set_options,
            )
            .await
            .map(|outcome| match outcome {
                SetOutcome::Created => response(Status::Created, Vec::new()),
                SetOutcome::Replaced => response(Status::Replaced, Vec::new()),
                SetOutcome::NotStored => response(Status::NotStored, Vec::new()),
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
        KvError::TableFull | KvError::CapacityExhausted { .. } => Status::Overloaded,
        KvError::ItemTooLarge { .. } | KvError::BlobSegmentFull { .. } => Status::TooLarge,
        KvError::InvalidRequest(_) => Status::InvalidRequest,
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
    #[error("QUIC transport failed: {0}")]
    Transport(#[from] TransportError),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("network worker failed: {0}")]
    NetworkWorker(String),
}

/// Convenience result type for server lifecycle operations.
pub type Result<T> = std::result::Result<T, ServerError>;
