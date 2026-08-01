//! QUIC server backed by the sharded SSD-first cache runtime.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use compio::driver::ProactorBuilder;
use compio::runtime::RuntimeBuilder;
use futures_util::stream::{FuturesUnordered, StreamExt};
use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::{
    MAX_REQUEST_FRAME_BYTES, Opcode, ProtocolError, Request, Response, Status,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use zeroize::Zeroizing;

use crate::channel::{self, AsyncReceiver, Sender};
use crate::mutation::{MutationDecision, MutationDedupeStore};
use crate::transport::{
    Connection as TransportConnection, Endpoint as TransportEndpoint,
    Incoming as TransportIncoming, ReceiveStream, RequestBudget, SendStream, ServerEndpoint,
    ServerTlsConfig, StreamReadError, TransportError,
};
use crate::{
    AppConfig, KvError, NetworkConfig, QuicBackend, SetOutcome, ThreadedKvkache, TlsConfig,
};

pub(crate) type NetworkWorkerCompletion = (usize, std::result::Result<(), String>);

pub(crate) struct NetworkWorkerHandle {
    stop: Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub(crate) struct NetworkRolePlacement {
    cpu_id: usize,
    thread_name: String,
    entries: u32,
    event_interval: usize,
    stop: Sender<()>,
}

impl NetworkRolePlacement {
    pub(crate) fn new(
        cpu_id: usize,
        thread_name: String,
        entries: u32,
        event_interval: usize,
        stop: Sender<()>,
    ) -> Self {
        Self {
            cpu_id,
            thread_name,
            entries,
            event_interval,
            stop,
        }
    }
}

pub(crate) struct NetworkWorkerReporter {
    worker_id: usize,
    started: Option<Sender<std::result::Result<(), String>>>,
    finished: Option<Sender<NetworkWorkerCompletion>>,
}

impl NetworkWorkerReporter {
    pub(crate) fn new(
        worker_id: usize,
        started: Sender<std::result::Result<(), String>>,
        finished: Sender<(usize, std::result::Result<(), String>)>,
    ) -> Self {
        Self {
            worker_id,
            started: Some(started),
            finished: Some(finished),
        }
    }

    pub(crate) fn startup_failed(mut self, message: String) {
        if let Some(started) = self.started.take() {
            let _ = started.send(Err(message));
        }
    }

    pub(crate) fn started(&mut self) -> bool {
        self.started
            .take()
            .is_some_and(|started| started.send(Ok(())).is_ok())
    }

    fn take_completion_sender(&mut self) -> Sender<NetworkWorkerCompletion> {
        self.finished
            .take()
            .expect("network worker completion sender is available at launch")
    }
}

impl Drop for NetworkWorkerReporter {
    fn drop(&mut self) {
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
}

pub(crate) struct NetworkTaskReporter {
    worker_id: usize,
    finished: Option<Sender<NetworkWorkerCompletion>>,
}

impl NetworkTaskReporter {
    pub(crate) fn new(worker_id: usize, finished: Sender<NetworkWorkerCompletion>) -> Self {
        Self {
            worker_id,
            finished: Some(finished),
        }
    }

    fn finish(mut self, result: std::result::Result<(), String>) {
        if let Some(finished) = self.finished.take() {
            let _ = finished.send((self.worker_id, result));
        }
    }
}

impl Drop for NetworkTaskReporter {
    fn drop(&mut self) {
        if let Some(finished) = self.finished.take() {
            let failure = if std::thread::panicking() {
                "panicked"
            } else {
                "exited without reporting completion"
            };
            let _ = finished.send((self.worker_id, Err(failure.into())));
        }
    }
}

async fn run_network_role_task<F, Fut>(
    task_reporter: NetworkTaskReporter,
    reporter: NetworkWorkerReporter,
    role: F,
) where
    F: FnOnce(NetworkWorkerReporter) -> Fut,
    Fut: Future<Output = Option<std::result::Result<(), String>>>,
{
    let result = role(reporter).await.unwrap_or(Ok(()));
    task_reporter.finish(result);
}

pub(crate) fn launch_network_role<F, Fut>(
    cache: &ThreadedKvkache,
    placement: NetworkRolePlacement,
    mut reporter: NetworkWorkerReporter,
    role: F,
) -> Result<NetworkWorkerHandle>
where
    F: FnOnce(NetworkWorkerReporter) -> Fut + Send + 'static,
    Fut: Future<Output = Option<std::result::Result<(), String>>> + 'static,
{
    let NetworkRolePlacement {
        cpu_id,
        thread_name,
        entries,
        event_interval,
        stop,
    } = placement;
    let worker_id = reporter.worker_id;
    let finished = reporter.take_completion_sender();
    if cache.can_run_on_storage_cpu(cpu_id) {
        let attached = cache.run_on_storage_cpu(cpu_id, move || {
            let task_reporter = NetworkTaskReporter::new(worker_id, finished);
            compio::runtime::spawn(run_network_role_task(task_reporter, reporter, role)).detach();
        })?;
        if !attached {
            return Err(ServerError::NetworkWorker(format!(
                "storage runtime on CPU {cpu_id} rejected its prepared network role"
            )));
        }
        return Ok(NetworkWorkerHandle { stop, thread: None });
    }

    let thread = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let task_reporter = NetworkTaskReporter::new(worker_id, finished);
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
                    task_reporter.finish(Ok(()));
                    return;
                }
            };
            runtime.block_on(run_network_role_task(task_reporter, reporter, role));
        })?;
    Ok(NetworkWorkerHandle {
        stop,
        thread: Some(thread),
    })
}

enum AccessPolicy {
    InsecureDevelopment,
    MutualTls {
        admin_client_certificates: Vec<CertificateDer<'static>>,
    },
}

impl AccessPolicy {
    fn permits_administration(&self, peer_certificate: Option<&CertificateDer<'_>>) -> bool {
        match self {
            Self::InsecureDevelopment => true,
            Self::MutualTls {
                admin_client_certificates,
            } => peer_certificate.is_some_and(|peer| {
                admin_client_certificates
                    .iter()
                    .any(|administrator| administrator.as_ref() == peer.as_ref())
            }),
        }
    }
}

fn load_production_tls(config: &TlsConfig) -> Result<(ServerTlsConfig, AccessPolicy)> {
    let certificate_chain = load_certificates(
        config
            .certificate_chain
            .as_deref()
            .expect("validated production TLS certificate path"),
    )?;
    let private_key = load_private_key(
        config
            .private_key
            .as_deref()
            .expect("validated production TLS private key path"),
    )?;
    let client_ca = load_certificates(
        config
            .client_ca
            .as_deref()
            .expect("validated production TLS client CA path"),
    )?;
    let mut admin_client_certificates = Vec::with_capacity(config.admin_client_certificates.len());
    for path in &config.admin_client_certificates {
        let certificate = load_certificates(path)?
            .into_iter()
            .next()
            .expect("certificate loader rejects empty files");
        admin_client_certificates.push(certificate);
    }
    Ok((
        ServerTlsConfig {
            certificate_chain,
            private_key,
            client_ca,
        },
        AccessPolicy::MutualTls {
            admin_client_certificates,
        },
    ))
}

fn load_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let bytes = std::fs::read(path).map_err(|error| ServerError::TlsIdentity {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let certificates = if bytes.starts_with(b"-----BEGIN") {
        CertificateDer::pem_slice_iter(&bytes)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| ServerError::TlsIdentity {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?
    } else {
        vec![CertificateDer::from(bytes)]
    };
    if certificates.is_empty() {
        return Err(ServerError::TlsIdentity {
            path: path.to_path_buf(),
            message: "no certificates found".into(),
        });
    }
    Ok(certificates)
}

fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let bytes = Zeroizing::new(
        std::fs::read(path).map_err(|error| ServerError::TlsIdentity {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?,
    );
    if bytes.starts_with(b"-----BEGIN") {
        PrivateKeyDer::from_pem_slice(&bytes).map_err(|error| ServerError::TlsIdentity {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
    } else {
        PrivateKeyDer::try_from(bytes.to_vec()).map_err(|message| ServerError::TlsIdentity {
            path: path.to_path_buf(),
            message: message.into(),
        })
    }
}

/// Bound reuse-port sockets and the sharded SSD-backed cache they serve.
pub struct KacheServer {
    sockets: Vec<std::net::UdpSocket>,
    local_addr: SocketAddr,
    quic_backend: QuicBackend,
    tls: Arc<ServerTlsConfig>,
    access_policy: Arc<AccessPolicy>,
    cache: Arc<ThreadedKvkache>,
    network: NetworkConfig,
    request_timeout: Duration,
    max_item_bytes: usize,
    mutation_dedupe: Arc<Mutex<MutationDedupeStore>>,
}

impl KacheServer {
    /// Binds a server with an explicit SSD cache configuration.
    ///
    /// # Arguments
    ///
    /// * `address` - UDP address on which the QUIC endpoint listens.
    /// * `config` - Network, storage, table, and timeout configuration.
    ///
    /// # Returns
    ///
    /// A ready server containing bound sockets, configured TLS identity, and cache workers.
    ///
    /// # Errors
    ///
    /// Returns an error when production TLS is missing or invalid, configuration validation or
    /// socket binding fails, or cache startup fails.
    pub async fn bind_with_config(address: SocketAddr, config: AppConfig) -> Result<Self> {
        config.validate()?;
        if !config.tls.is_configured() {
            return Err(ServerError::ProductionTlsRequired(address));
        }
        let (tls, access_policy) = load_production_tls(&config.tls)?;
        Self::bind_with_security(address, config, tls, access_policy).await
    }

    /// Binds with a generated certificate and no peer authentication for development only.
    ///
    /// This mode grants every connected peer administrative access and must not be used for
    /// production deployments.
    ///
    /// # Arguments
    ///
    /// * `address` - UDP address on which the QUIC endpoint listens.
    /// * `config` - Network, storage, table, and timeout configuration.
    ///
    /// # Returns
    ///
    /// A ready server containing bound sockets, an ephemeral certificate, and cache workers.
    ///
    /// # Errors
    ///
    /// Returns an error when production TLS is also configured, configuration validation,
    /// certificate generation, socket binding, or cache startup fails.
    pub async fn bind_insecure_for_development(
        address: SocketAddr,
        config: AppConfig,
    ) -> Result<Self> {
        if config.tls.is_configured() {
            return Err(ServerError::ConflictingSecurityModes);
        }
        config.validate()?;
        let mut subject_alt_names = vec!["localhost".to_string()];
        if !address.ip().is_unspecified() {
            subject_alt_names.push(address.ip().to_string());
        }
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(subject_alt_names)?;
        let tls = ServerTlsConfig {
            certificate_chain: vec![cert.into()],
            private_key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                signing_key.serialize_der(),
            )),
            client_ca: Vec::new(),
        };
        Self::bind_with_security(address, config, tls, AccessPolicy::InsecureDevelopment).await
    }

    async fn bind_with_security(
        address: SocketAddr,
        config: AppConfig,
        tls: ServerTlsConfig,
        access_policy: AccessPolicy,
    ) -> Result<Self> {
        let request_timeout = Duration::from_micros(config.timeouts.request_max_time_us);
        let max_item_bytes = config.storage.max_item_size_mib * 1024 * 1024;
        let network = config.network.clone();
        let quic_backend = config.quic.selected_backend()?;
        ServerEndpoint::validate_backend(quic_backend)?;
        let mut cache = ThreadedKvkache::start_validated_for_server(config)?;
        let sockets = match bind_reuse_port_sockets(address, network.worker_count) {
            Ok(sockets) => sockets,
            Err(error) => {
                cache.shutdown()?;
                return Err(error.into());
            }
        };
        let local_addr = sockets[0].local_addr()?;
        let cache = Arc::new(cache);
        Ok(Self {
            sockets,
            local_addr,
            quic_backend,
            tls: Arc::new(tls),
            access_policy: Arc::new(access_policy),
            cache,
            network,
            request_timeout,
            max_item_bytes,
            mutation_dedupe: Arc::new(Mutex::new(MutationDedupeStore::default())),
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

    /// Returns the leaf certificate clients must trust directly or through its issuing CA.
    ///
    /// # Returns
    ///
    /// The configured or generated leaf certificate encoded as DER bytes.
    pub fn certificate_der(&self) -> &[u8] {
        self.tls
            .certificate_chain
            .first()
            .expect("validated TLS certificate chain")
            .as_ref()
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
            tls,
            access_policy,
            cache,
            network,
            request_timeout,
            max_item_bytes,
            mutation_dedupe,
            ..
        } = self;
        let (started_tx, started_rx) =
            channel::bounded::<std::result::Result<(), String>>(network.worker_count);
        let (finished_tx, finished_rx) =
            channel::bounded_sync_async::<NetworkWorkerCompletion>(network.worker_count);
        let mut workers = Vec::with_capacity(network.worker_count);
        let mut launch_error = None;
        let request_budget = RequestBudget::new(network.max_inflight_value_mib * 1024 * 1024);

        for (worker_id, socket) in sockets.into_iter().enumerate() {
            let (stop_tx, stop_rx) = channel::bounded_sync_async(1);
            let started_tx = started_tx.clone();
            let finished_tx = finished_tx.clone();
            let worker_tls = Arc::clone(&tls);
            let worker_access_policy = Arc::clone(&access_policy);
            let worker_cache = Arc::clone(&cache);
            let cpu_id = network.cpu_ids[worker_id];
            let entries = network.io_uring_entries_per_worker;
            let event_interval = network.event_interval;
            let limits = NetworkWorkerLimits {
                request_timeout,
                max_stream_lanes: network.max_stream_lanes_per_connection,
                request_budget: request_budget.clone(),
                max_item_bytes,
                mutation_dedupe: Arc::clone(&mutation_dedupe),
            };
            let reporter = NetworkWorkerReporter::new(worker_id, started_tx, finished_tx);
            let role = QuicNetworkRole {
                worker_id,
                cpu_id,
                socket,
                quic_backend,
                tls: worker_tls,
                access_policy: worker_access_policy,
                cache: worker_cache,
                limits,
                stop: stop_rx,
            };
            match launch_network_role(
                &cache,
                NetworkRolePlacement::new(
                    cpu_id,
                    format!("openkache-network-{worker_id}"),
                    entries,
                    event_interval,
                    stop_tx,
                ),
                reporter,
                move |reporter| run_quic_role(role, reporter),
            ) {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    launch_error.get_or_insert_with(|| error.to_string());
                }
            }
        }
        drop(started_tx);
        drop(finished_tx);

        let mut startup_error = launch_error;
        for _ in 0..network.worker_count {
            match started_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(message)) => {
                    startup_error.get_or_insert(message);
                }
                Err(_) => {
                    startup_error
                        .get_or_insert_with(|| "network worker startup channel closed".into());
                    break;
                }
            }
        }
        if let Some(message) = startup_error {
            let remaining_completions = workers.len();
            shutdown_network_workers_and_cache(workers, &finished_rx, remaining_completions, cache)
                .await?;
            return Err(ServerError::NetworkWorker(message));
        }

        let shutdown = shutdown.fuse();
        let worker_finished = finished_rx.recv_async().fuse();
        pin_mut!(shutdown, worker_finished);
        let (worker_failure, completed_workers) = select! {
            () = shutdown => (None, 0),
            result = worker_finished => (Some(match result {
                Ok((worker_id, Ok(()))) => {
                    format!("network worker {worker_id} exited unexpectedly")
                }
                Ok((worker_id, Err(message))) => {
                    format!("network worker {worker_id} failed: {message}")
                }
                Err(_) => "network worker completion channel closed".into(),
            }), 1),
        };
        let remaining_completions = workers.len().saturating_sub(completed_workers);
        shutdown_network_workers_and_cache(workers, &finished_rx, remaining_completions, cache)
            .await?;
        match worker_failure {
            Some(message) => Err(ServerError::NetworkWorker(message)),
            None => Ok(()),
        }
    }
}

struct QuicNetworkRole {
    worker_id: usize,
    cpu_id: usize,
    socket: std::net::UdpSocket,
    quic_backend: QuicBackend,
    tls: Arc<ServerTlsConfig>,
    access_policy: Arc<AccessPolicy>,
    cache: Arc<ThreadedKvkache>,
    limits: NetworkWorkerLimits,
    stop: AsyncReceiver<()>,
}

async fn run_quic_role(
    role: QuicNetworkRole,
    mut reporter: NetworkWorkerReporter,
) -> Option<std::result::Result<(), String>> {
    let QuicNetworkRole {
        worker_id,
        cpu_id,
        socket,
        quic_backend,
        tls,
        access_policy,
        cache,
        limits,
        stop,
    } = role;
    let endpoint =
        match ServerEndpoint::bind(quic_backend, socket, tls, limits.max_stream_lanes).await {
            Ok(endpoint) => endpoint,
            Err(error) => {
                reporter.startup_failed(error.to_string());
                return None;
            }
        };
    if let Some(error) =
        crate::platform::cpu_assignment_error(&format!("network worker {worker_id}"), cpu_id)
    {
        reporter.startup_failed(error);
        return None;
    }
    if !reporter.started() {
        return None;
    }
    Some(
        run_selected_endpoint(endpoint, cache, &access_policy, limits, stop)
            .await
            .map_err(|error| error.to_string()),
    )
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

pub(crate) async fn shutdown_network_workers_and_cache(
    workers: Vec<NetworkWorkerHandle>,
    finished: &AsyncReceiver<NetworkWorkerCompletion>,
    remaining_completions: usize,
    cache: Arc<ThreadedKvkache>,
) -> Result<()> {
    for worker in &workers {
        let _ = worker.stop.send(());
    }
    let mut network_failure = None;
    for _ in 0..remaining_completions {
        match finished.recv_async().await {
            Ok((_worker_id, Ok(()))) => {}
            Ok((worker_id, Err(message))) => {
                network_failure.get_or_insert_with(|| {
                    format!("network worker {worker_id} failed during shutdown: {message}")
                });
            }
            Err(_) => {
                network_failure
                    .get_or_insert_with(|| "network worker completion channel closed".into());
                break;
            }
        }
    }
    let join_result = join_network_workers(workers);
    let cache_result = shutdown_cache(cache);
    if let Some(message) = network_failure {
        return Err(ServerError::NetworkWorker(message));
    }
    join_result?;
    cache_result
}

fn join_network_workers(workers: Vec<NetworkWorkerHandle>) -> Result<()> {
    let threads = workers
        .into_iter()
        .filter_map(|worker| worker.thread)
        .collect();
    join_network_threads(threads)
}

pub(crate) fn join_network_threads(threads: Vec<std::thread::JoinHandle<()>>) -> Result<()> {
    let mut panicked_worker = None;
    for thread in threads {
        let worker = thread.thread().clone();
        if thread.join().is_err() && panicked_worker.is_none() {
            panicked_worker = Some(worker.name().unwrap_or("network worker").to_owned());
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

#[derive(Clone)]
struct NetworkWorkerLimits {
    request_timeout: Duration,
    max_stream_lanes: usize,
    request_budget: RequestBudget,
    max_item_bytes: usize,
    mutation_dedupe: Arc<Mutex<MutationDedupeStore>>,
}

async fn run_selected_endpoint(
    endpoint: ServerEndpoint,
    cache: Arc<ThreadedKvkache>,
    access_policy: &AccessPolicy,
    limits: NetworkWorkerLimits,
    stop: AsyncReceiver<()>,
) -> std::result::Result<(), TransportError> {
    match endpoint {
        #[cfg(feature = "quic-quinn")]
        ServerEndpoint::Quinn(endpoint) => {
            run_network_worker(endpoint, cache, access_policy, limits, stop).await
        }
        #[cfg(feature = "quic-noq")]
        ServerEndpoint::Noq(endpoint) => {
            run_network_worker(endpoint, cache, access_policy, limits, stop).await
        }
        #[cfg(feature = "quic-quiche")]
        ServerEndpoint::Quiche(endpoint) => {
            run_network_worker(endpoint, cache, access_policy, limits, stop).await
        }
    }
}

async fn run_network_worker<E: TransportEndpoint>(
    endpoint: E,
    cache: Arc<ThreadedKvkache>,
    access_policy: &AccessPolicy,
    limits: NetworkWorkerLimits,
    stop: AsyncReceiver<()>,
) -> std::result::Result<(), TransportError> {
    let NetworkWorkerLimits {
        request_timeout,
        max_stream_lanes,
        request_budget,
        max_item_bytes,
        mutation_dedupe,
    } = limits;
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
                        incoming, Arc::clone(&cache), access_policy, request_timeout, max_stream_lanes,
                        request_budget.clone(), max_item_bytes, Arc::clone(&mutation_dedupe),
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
                        incoming, Arc::clone(&cache), access_policy, request_timeout, max_stream_lanes,
                        request_budget.clone(), max_item_bytes, Arc::clone(&mutation_dedupe),
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
#[allow(clippy::too_many_arguments)]
async fn serve_incoming<I: TransportIncoming>(
    incoming: I,
    cache: Arc<ThreadedKvkache>,
    access_policy: &AccessPolicy,
    request_timeout: Duration,
    max_stream_lanes: usize,
    request_budget: RequestBudget,
    max_item_bytes: usize,
    mutation_dedupe: Arc<Mutex<MutationDedupeStore>>,
) {
    if let Ok(mut connection) = incoming.connect().await {
        let administrator =
            access_policy.permits_administration(connection.take_peer_certificate().as_ref());
        serve_connection(
            connection,
            cache,
            administrator,
            request_timeout,
            max_stream_lanes,
            request_budget,
            max_item_bytes,
            mutation_dedupe,
        )
        .await;
    }
}

/// Multiplexes bounded reusable request lanes for one QUIC connection.
#[allow(clippy::too_many_arguments)]
async fn serve_connection<C: TransportConnection>(
    connection: C,
    cache: Arc<ThreadedKvkache>,
    administrator: bool,
    request_timeout: Duration,
    max_stream_lanes: usize,
    request_budget: RequestBudget,
    max_item_bytes: usize,
    mutation_dedupe: Arc<Mutex<MutationDedupeStore>>,
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
                    streams.push(serve_stream(
                        send,
                        receive,
                        Arc::clone(&cache),
                        administrator,
                        request_timeout,
                        request_budget.clone(),
                        max_item_bytes,
                        Arc::clone(&mutation_dedupe),
                    ));
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
                        streams.push(serve_stream(
                            send,
                            receive,
                            Arc::clone(&cache),
                            administrator,
                            request_timeout,
                            request_budget.clone(),
                            max_item_bytes,
                            Arc::clone(&mutation_dedupe),
                        ));
                    }
                    Err(_) => break,
                },
                _ = completed => {}
            }
        }
    }
    while streams.next().await.is_some() {}
}

struct PendingMutationGuard {
    mutation_dedupe: Arc<Mutex<MutationDedupeStore>>,
    mutation_id: openkache_protocol::MutationId,
    fingerprint: [u8; 32],
    generation: u64,
    armed: bool,
}

impl PendingMutationGuard {
    fn new(
        mutation_dedupe: Arc<Mutex<MutationDedupeStore>>,
        mutation_id: openkache_protocol::MutationId,
        fingerprint: [u8; 32],
        generation: u64,
    ) -> Self {
        Self {
            mutation_dedupe,
            mutation_id,
            fingerprint,
            generation,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingMutationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = self
            .mutation_dedupe
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .release_pending_with_reservation(
                self.mutation_id,
                self.fingerprint,
                self.generation,
                std::time::Instant::now(),
            );
    }
}

/// Reuses one QUIC stream as a sequential request lane until either peer closes it.
#[allow(clippy::too_many_arguments)]
async fn serve_stream<S: SendStream, R: ReceiveStream>(
    mut send: S,
    mut receive: R,
    cache: Arc<ThreadedKvkache>,
    administrator: bool,
    request_timeout: Duration,
    request_budget: RequestBudget,
    max_item_bytes: usize,
    mutation_dedupe: Arc<Mutex<MutationDedupeStore>>,
) {
    let mut mutation_tasks = FuturesUnordered::new();
    loop {
        // A timed-out mutation continues in the storage worker so its final
        // result can be replayed. Reap completed tasks while the stream
        // remains active, and join the remaining bounded tasks on exit.
        while let Some(Some(_)) = mutation_tasks.next().now_or_never() {}
        let mut frame = match receive
            .read_request(MAX_REQUEST_FRAME_BYTES, request_timeout, &request_budget)
            .await
        {
            Ok(frame) => frame,
            Err(StreamReadError::Timeout) => {
                let _ = write_response(
                    &mut send,
                    response_bytes(Status::Timeout, b"request read timed out"),
                    request_timeout,
                )
                .await;
                break;
            }
            Err(StreamReadError::TooLarge) => {
                let _ = write_response(
                    &mut send,
                    response_bytes(Status::TooLarge, b"request exceeds the protocol limit"),
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
        let request_bytes = std::mem::take(&mut frame.bytes);
        let fingerprint: [u8; 32] = Sha256::digest(&request_bytes).into();
        let (response, _response_permit) = match Request::decode_owned(request_bytes) {
            Ok(request) => {
                let mutation_id = request.mutation_id;
                let mutation_check = mutation_id.map(|mutation_id| {
                    mutation_dedupe
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .check_with_reservation(mutation_id, fingerprint, std::time::Instant::now())
                });
                let mut should_record = mutation_check
                    .as_ref()
                    .is_some_and(|(decision, _)| matches!(decision, MutationDecision::New));
                let reservation_generation = mutation_check
                    .as_ref()
                    .and_then(|(_, generation)| *generation);
                let mut reservation_guard = match (mutation_id, should_record) {
                    (Some(mutation_id), true) => reservation_generation.map(|generation| {
                        PendingMutationGuard::new(
                            Arc::clone(&mutation_dedupe),
                            mutation_id,
                            fingerprint,
                            generation,
                        )
                    }),
                    _ => None,
                };
                let response_permit = if request.opcode == Opcode::Get {
                    match request_budget
                        .acquire(max_item_bytes, request_timeout)
                        .await
                    {
                        Ok(permit) => Some(permit),
                        Err(StreamReadError::Timeout) => {
                            let response = response_bytes(
                                Status::Timeout,
                                b"response memory budget timed out",
                            );
                            if !write_response(&mut send, response, request_timeout).await {
                                break;
                            }
                            continue;
                        }
                        Err(_) => {
                            let response = response_bytes(
                                Status::Overloaded,
                                b"response exceeds the server memory budget",
                            );
                            if !write_response(&mut send, response, request_timeout).await {
                                break;
                            }
                            continue;
                        }
                    }
                } else {
                    None
                };
                let response = match mutation_check.map(|(decision, _)| decision) {
                    Some(MutationDecision::Replay { status, payload }) => response(
                        Status::try_from(status).unwrap_or(Status::InternalError),
                        payload,
                    ),
                    Some(MutationDecision::Pending) => {
                        wait_for_mutation_result(
                            &mutation_dedupe,
                            mutation_id.expect("pending mutations carry a token"),
                            fingerprint,
                            request_timeout,
                        )
                        .await
                    }
                    Some(MutationDecision::Conflict) => response_bytes(
                        Status::MutationConflict,
                        b"mutation token was already used for a different request",
                    ),
                    Some(MutationDecision::Capacity) => {
                        response_bytes(Status::Overloaded, b"mutation replay store is at capacity")
                    }
                    Some(MutationDecision::New) => {
                        should_record = false;
                        let mutation_cache = Arc::clone(&cache);
                        let mutation_store = Arc::clone(&mutation_dedupe);
                        let reservation = reservation_guard.take();
                        let generation = reservation_generation
                            .expect("new mutations carry a reservation generation");
                        mutation_tasks.push(compio::runtime::spawn(async move {
                            let response =
                                execute_request(&mutation_cache, request, administrator).await;
                            let _ = mutation_store
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .record_with_reservation(
                                    mutation_id.expect("new mutations carry a token"),
                                    fingerprint,
                                    generation,
                                    response.status as u8,
                                    response.payload,
                                    std::time::Instant::now(),
                                );
                            if let Some(mut reservation) = reservation {
                                reservation.disarm();
                            }
                        }));
                        wait_for_mutation_result(
                            &mutation_dedupe,
                            mutation_id.expect("new mutations carry a token"),
                            fingerprint,
                            request_timeout,
                        )
                        .await
                    }
                    None => match compio::runtime::time::timeout(
                        request_timeout,
                        execute_request(&cache, request, administrator),
                    )
                    .await
                    {
                        Ok(response) => response,
                        Err(_) => response_bytes(Status::Timeout, b"request execution timed out"),
                    },
                };
                if should_record && let Some(mutation_id) = mutation_id {
                    mutation_dedupe
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .record(
                            mutation_id,
                            fingerprint,
                            response.status as u8,
                            response.payload.clone(),
                            std::time::Instant::now(),
                        );
                    if let Some(guard) = reservation_guard.as_mut() {
                        guard.disarm();
                    }
                }
                (response, response_permit)
            }
            Err(error) => (protocol_error_response(error), None),
        };
        if !write_response(&mut send, response, request_timeout).await {
            break;
        }
    }
    while mutation_tasks.next().await.is_some() {}
}

async fn wait_for_mutation_result(
    mutation_dedupe: &Arc<Mutex<MutationDedupeStore>>,
    mutation_id: openkache_protocol::MutationId,
    fingerprint: [u8; 32],
    request_timeout: Duration,
) -> Response {
    let deadline = std::time::Instant::now()
        .checked_add(request_timeout)
        .unwrap_or_else(std::time::Instant::now);
    loop {
        let decision = mutation_dedupe
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .lookup(mutation_id, fingerprint, std::time::Instant::now());
        match decision {
            MutationDecision::Replay { status, payload } => {
                return response(
                    Status::try_from(status).unwrap_or(Status::InternalError),
                    payload,
                );
            }
            MutationDecision::Conflict => {
                return response_bytes(
                    Status::MutationConflict,
                    b"mutation token was already used for a different request",
                );
            }
            MutationDecision::Capacity => {
                return response_bytes(Status::Overloaded, b"mutation replay store is at capacity");
            }
            MutationDecision::New => {
                return response_bytes(
                    Status::Timeout,
                    b"mutation result reservation expired before replay",
                );
            }
            MutationDecision::Pending => {}
        }

        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return response_bytes(Status::Timeout, b"mutation replay timed out");
        }
        compio::runtime::time::sleep(remaining.min(Duration::from_millis(1))).await;
    }
}

async fn write_response<S: SendStream>(
    send: &mut S,
    response: Response,
    request_timeout: Duration,
) -> bool {
    let Ok(frame) = response.into_encoded() else {
        return false;
    };
    send.write_response(frame, request_timeout).await.is_ok()
}

/// Dispatches a decoded protocol request to the SSD-backed worker runtime.
async fn execute_request(
    cache: &ThreadedKvkache,
    request: Request,
    administrator: bool,
) -> Response {
    let Request {
        opcode,
        item_id,
        set_options,
        value,
        mutation_id: _,
    } = request;
    let result = match opcode {
        Opcode::Ping => return response_bytes(Status::Ok, b"PONG"),
        Opcode::Get => cache
            .get_async(item_id.expect("GET requests have a validated item ID"))
            .await
            .map(|value| match value {
                Some(value) => response(Status::Ok, value.bytes),
                None => response(Status::NotFound, Vec::new()),
            }),
        Opcode::Set => cache
            .set_async_with_options(
                item_id.expect("SET requests have a validated item ID"),
                crate::types::StoredItemValue::new(value),
                set_options,
            )
            .await
            .map(|outcome| match outcome {
                SetOutcome::Created => response(Status::Created, Vec::new()),
                SetOutcome::Replaced => response(Status::Replaced, Vec::new()),
                SetOutcome::NotStored => response(Status::NotStored, Vec::new()),
            }),
        Opcode::Delete => cache
            .delete_async(item_id.expect("DELETE requests have a validated item ID"))
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
        Opcode::Stats if !administrator => {
            return response_bytes(
                Status::Forbidden,
                b"STATS requires administrator authorization",
            );
        }
        Opcode::Stats => cache.stats_async().await.map(|workers| {
            let worker_bytes = workers.iter().map(String::len).sum::<usize>();
            let mut payload = String::with_capacity(32 + worker_bytes);
            payload.push_str(r#"{"storage":"ssd","workers":["#);
            for (index, worker) in workers.into_iter().enumerate() {
                if index > 0 {
                    payload.push(',');
                }
                write!(payload, "{worker:?}").expect("writing to a String cannot fail");
            }
            payload.push_str("]}");
            response(Status::Ok, payload.into_bytes())
        }),
        Opcode::Sync if !administrator => {
            return response_bytes(
                Status::Forbidden,
                b"SYNC requires administrator authorization",
            );
        }
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
    response_display(status, error)
}

/// Maps framing and validation failures to stable protocol statuses.
fn protocol_error_response(error: ProtocolError) -> Response {
    let status = match error {
        ProtocolError::UnknownOpcode(_) => Status::UnsupportedOpcode,
        ProtocolError::ValueTooLarge { .. } => Status::TooLarge,
        _ => Status::InvalidRequest,
    };
    response_display(status, error)
}

fn response_display(status: Status, value: impl std::fmt::Display) -> Response {
    let mut payload = String::with_capacity(
        openkache_protocol::RESPONSE_FIXED_BYTES + openkache_protocol::MAX_VARUINT_BYTES + 64,
    );
    write!(payload, "{value}").expect("writing to a String cannot fail");
    response(status, payload.into_bytes())
}

/// Constructs a protocol response whose payload is known to fit protocol limits.
fn response(status: Status, payload: Vec<u8>) -> Response {
    Response::new(status, payload).expect("server responses stay within protocol limits")
}

fn response_bytes(status: Status, payload: &[u8]) -> Response {
    let mut owned = Vec::with_capacity(
        openkache_protocol::RESPONSE_FIXED_BYTES
            + openkache_protocol::MAX_VARUINT_BYTES
            + payload.len(),
    );
    owned.extend_from_slice(payload);
    response(status, owned)
}

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
