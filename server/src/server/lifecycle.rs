use super::*;
use super::connection::{NetworkWorkerLimits, prepare_network_worker, run_selected_endpoint};

impl KacheServer {
    /// Installs API-owned capabilities before the server starts serving.
    ///
    /// The catalog is carried unchanged through every network worker and
    /// request lane. Generic dispatch only exposes its opaque lookup surface;
    /// compatibility services remain supplied by the composition root.
    ///
    /// # Arguments
    ///
    /// * `capabilities` - A thread-safe catalog whose values remain valid for
    ///   the server lifetime.
    ///
    /// # Returns
    ///
    /// The same server with the supplied capability catalog installed.
    pub fn with_capabilities(mut self, capabilities: Arc<dyn CapabilityCatalog>) -> Self {
        self.capabilities = capabilities;
        self
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
        if let Err(message) = operation_registrations::validate() {
            return Err(ServerError::NetworkWorker(message.into()));
        }
        let request_timeout = Duration::from_micros(config.timeouts.request_max_time_us);
        let max_item_bytes = config.storage.max_item_size_mib * 1024 * 1024;
        let network = config.network.clone();
        let storage_directory = config.storage.directory.clone();
        let observability = ObservabilityService::new(
            network.worker_count,
            config.runtime.thread_count,
            &config.observability,
        )?;
        let observability_state = observability.state();
        let existing_storage = crate::storage_backend::existing_storage(&config);
        let quic_backend = config.quic.selected_backend()?;
        ServerEndpoint::validate_backend(quic_backend)?;
        let mut cache = ThreadedKvkache::start_validated_for_server_with_observability(
            config,
            observability_state,
        )?;
        let namespaces = match NamespaceRegistry::load(&storage_directory, existing_storage) {
            Ok(registry) => registry,
            Err(error) => {
                cache.shutdown()?;
                return Err(ServerError::NamespaceMetadata(error.to_string()));
            }
        };
        if let Err(error) = namespaces.persist() {
            cache.shutdown()?;
            return Err(ServerError::NamespaceMetadata(error.to_string()));
        }
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
            namespaces: Arc::new(Mutex::new(namespaces)),
            network,
            request_timeout,
            max_item_bytes,
            observability,
            capabilities: Arc::new(EmptyCapabilityCatalog),
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

    /// Returns the bound observability management address, when enabled.
    ///
    /// # Returns
    ///
    /// The address bound for the management listener, or `None` when
    /// observability metrics are disabled.
    pub fn metrics_addr(&self) -> Option<SocketAddr> {
        self.observability.metrics_addr()
    }

    /// Returns the conservative classification of the files opened by the
    /// storage workers during bind.
    ///
    /// # Returns
    ///
    /// The aggregate classification of every data and large-value file opened
    /// by the storage workers.
    pub fn storage_device_kind(&self) -> StorageDeviceKind {
        self.cache.storage_device_kind()
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
            namespaces,
            network,
            request_timeout,
            max_item_bytes,
            observability: observability_service,
            capabilities,
            ..
        } = self;
        let observability = observability_service.state();
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
                worker_id,
                request_timeout,
                max_stream_lanes: network.max_stream_lanes_per_connection,
                request_budget: request_budget.clone(),
                max_item_bytes,
                namespaces: Arc::clone(&namespaces),
                observability: Arc::clone(&observability),
                capabilities: Arc::clone(&capabilities),
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
                    worker_id,
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
            observability.set_failed();
            let remaining_completions = workers.len();
            shutdown_network_workers_and_cache(workers, &finished_rx, remaining_completions, cache)
                .await?;
            return Err(ServerError::NetworkWorker(message));
        }

        let metrics_handle = observability_service.start();
        let shutdown = shutdown.fuse();
        let worker_finished = finished_rx.recv_async_network().fuse();
        pin_mut!(shutdown, worker_finished);
        let (worker_failure, completed_workers, failed_worker) = select! {
            () = shutdown => (None, 0, None),
            result = worker_finished => match result {
                Ok((worker_id, Ok(()))) => (
                    Some(format!("network worker {worker_id} exited unexpectedly")),
                    1,
                    Some(worker_id),
                ),
                Ok((worker_id, Err(message))) => (
                    Some(format!("network worker {worker_id} failed: {message}")),
                    1,
                    Some(worker_id),
                ),
                Err(_) => (Some("network worker completion channel closed".into()), 1, None),
            },
        };
        let remaining_completions = workers.len().saturating_sub(completed_workers);
        if let Some(worker_id) = failed_worker {
            observability.network_worker_failed(worker_id);
        } else if worker_failure.is_some() {
            observability.set_failed();
        } else {
            observability.set_draining();
        }
        metrics_handle.stop();
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
                limits.observability.network_worker_failed(worker_id);
                reporter.startup_failed(error.to_string());
                return None;
            }
        };
    if let Some(error) =
        crate::platform::cpu_assignment_error(&format!("network worker {worker_id}"), cpu_id)
    {
        limits.observability.network_worker_failed(worker_id);
        reporter.startup_failed(error);
        return None;
    }
    let capabilities = match prepare_network_worker(cache, &limits) {
        Ok(prepared) => prepared,
        Err(error) => {
            limits.observability.network_worker_failed(worker_id);
            reporter.startup_failed(error.to_owned());
            return None;
        }
    };
    let mut limits = limits;
    limits.capabilities = capabilities;
    if !reporter.started() {
        return None;
    }
    limits.observability.network_worker_started(worker_id);
    Some(
        run_selected_endpoint(endpoint, &access_policy, limits, stop)
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
        match finished.recv_async_network().await {
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
