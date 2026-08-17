//! Multi-threaded KV cache runtime. [`ThreadedKvkache`] manages a pool of
//! thread-per-core workers, each running the selected storage event loop. Handles
//! request routing by key hash and graceful shutdown.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::channel::{self, Sender};
use crate::observability::{NetworkWorkerId, ObservabilityState, Operation};
use crate::types::StoredItemValue;
use crate::*;

mod keyed_storage;
mod network_cache;
mod port;
mod retained_response;
mod scheduler;
mod storage_keys;
mod submission;
mod worker;
mod worker_contract;
mod worker_control;
mod worker_lifecycle;
pub(crate) use network_cache::NetworkWorkerCache;
pub(crate) use port::{completion, storage_port};
#[allow(unused_imports)]
pub(crate) use storage_keys::{
    DOMAIN_V2_CONTEXT, derive_domain_key, derive_storage_key, derive_storage_key_for_domain,
};
pub(crate) use storage_port::*;
#[allow(unused_imports)]
pub(crate) use worker::*;
pub use submission::{PendingStorageSubmission, StorageSubmission, SubmittedStorageValue};

use self::completion::CompletionSlab;

#[allow(unused_imports)]
pub(crate) use crate::storage_backend::{
    RUNNING_MARKER_FILE, SERVER_KEY_FILE, STORAGE_FORMAT_FILE,
};

#[derive(Debug)]
pub(in crate::runtime) enum WorkerControlResponse {
    Stats(String),
    Synced,
}

pub(in crate::runtime) type WorkerResponse =
    worker_contract::Response<keyed_storage::Response, WorkerControlResponse>;
pub(in crate::runtime) type WorkerResponseSender = worker_contract::ResponseSender<WorkerResponse>;
pub(in crate::runtime) type WorkerRequest =
    worker_contract::Request<StorageKey, keyed_storage::Command, WorkerControlRequest>;
pub(in crate::runtime) type WorkerControlRequest = worker_control::ControlRequest<WorkerResponse>;
pub(in crate::runtime) type DeferredWorkerResponse =
    worker_contract::DeferredResponse<WorkerResponse>;

#[derive(Clone, Copy)]
pub(crate) struct ServerSecret {
    pub(crate) id: [u8; 16],
    pub(crate) key: [u8; 32],
}

struct WorkerHandle {
    sender: Sender<WorkerRequest>,
    completions: CompletionSlab<Result<WorkerResponse>>,
    core_tasks: Option<Sender<CoreTask>>,
    thread: Option<std::thread::JoinHandle<Result<()>>>,
}

enum CoreTask {
    Run(Box<dyn FnOnce() + Send + 'static>),
    Shutdown,
}

pub struct ThreadedKvkache {
    config: crate::config::AppConfig,
    workers: Vec<WorkerHandle>,
    storage_domain_key: [u8; 32],
    storage_device_kind: crate::platform::StorageDeviceKind,
    observability: Option<Arc<ObservabilityState>>,
}

impl ThreadedKvkache {
    pub(crate) fn max_item_bytes(&self) -> usize {
        self.config.storage.max_item_size_mib * 1024 * 1024
    }

    pub fn start(config: crate::config::AppConfig) -> Result<Self> {
        config.validate()?;
        Self::start_validated(config)
    }

    pub(crate) fn start_validated(config: crate::config::AppConfig) -> Result<Self> {
        Self::start_validated_with_network_roles(config, false, None)
    }

    /// Starts storage workers and attaches overlapping server network tasks when supported.
    #[allow(dead_code)]
    pub(crate) fn start_validated_for_server(config: crate::config::AppConfig) -> Result<Self> {
        Self::start_validated_with_network_roles(config, true, None)
    }

    /// Starts server storage workers with shared low-overhead observability state.
    pub(crate) fn start_validated_for_server_with_observability(
        config: crate::config::AppConfig,
        observability: Arc<ObservabilityState>,
    ) -> Result<Self> {
        Self::start_validated_with_network_roles(config, true, Some(observability))
    }

    fn start_validated_with_network_roles(
        config: crate::config::AppConfig,
        attach_network_roles: bool,
        observability: Option<Arc<ObservabilityState>>,
    ) -> Result<Self> {
        let has_combined_worker = attach_network_roles
            && config
                .runtime
                .cpu_ids
                .iter()
                .any(|cpu_id| config.network.cpu_ids.contains(cpu_id));
        if has_combined_worker {
            let storage_runtime = crate::storage_runtime::NAME;
            let network_runtime = crate::network_runtime::name();
            if storage_runtime != network_runtime
                || !crate::storage_runtime::SUPPORTS_COMBINED_NETWORK_ROLE
                || !crate::network_runtime::supports_combined_network_role()
            {
                return Err(KvError::InvalidConfig(format!(
                    "network/storage CPU overlap requires the same combined-capable runtime instance; network runtime is {network_runtime}, storage runtime is {storage_runtime}"
                )));
            }
        }
        let combined_entries = has_combined_worker
            .then(|| {
                config
                    .io_uring
                    .entries_per_worker
                    .checked_add(config.network.io_uring_entries_per_worker)
                    .ok_or_else(|| {
                        KvError::InvalidConfig(
                            "combined worker io_uring entry count overflowed".into(),
                        )
                    })
            })
            .transpose()?;
        let startup = crate::storage_backend::startup(&config)?;
        Self::start_with_server_secret(
            config,
            startup.server_secret,
            startup.allow_checkpoint,
            combined_entries,
            observability,
        )
    }

    /// Starts storage workers with one caller-owned stable storage identity.
    ///
    /// This bypasses the persisted server-secret lifecycle and is intended for
    /// runtimes whose caller owns identity persistence.
    ///
    /// # Arguments
    ///
    /// * `config` - Complete storage-worker configuration.
    /// * `storage_identity` - Stable secret used to derive storage keys.
    ///
    /// # Returns
    ///
    /// A running worker set using the supplied storage identity.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration validation, storage startup, or
    /// worker startup fails.
    pub fn start_with_storage_identity(
        config: crate::config::AppConfig,
        storage_identity: [u8; 32],
    ) -> Result<Self> {
        config.validate()?;
        let allow_checkpoint = crate::storage_backend::startup_with_server_key(&config)?;
        let mut id = [0; 16];
        id.copy_from_slice(&storage_identity[..16]);
        Self::start_with_server_secret(
            config,
            ServerSecret {
                id,
                key: storage_identity,
            },
            allow_checkpoint,
            None,
            None,
        )
    }

    fn start_with_server_secret(
        config: crate::config::AppConfig,
        server_secret: ServerSecret,
        allow_checkpoint: bool,
        combined_entries: Option<u32>,
        observability: Option<Arc<ObservabilityState>>,
    ) -> Result<Self> {
        let storage_domain_key = storage_keys::derive_domain_key(&server_secret.key);
        let (started_tx, started_rx) = channel::bounded::<
            std::result::Result<crate::platform::StorageDeviceKind, String>,
        >(config.runtime.thread_count);
        let queue_capacity = config.io_uring.waiting_capacity()?.max(64);
        let mut workers = Vec::with_capacity(config.runtime.thread_count);
        let resource_guard = Arc::new(ResourceGuard::for_app_config(&config)?);

        for thread_id in 0..config.runtime.thread_count {
            let (sender, receiver) = channel::bounded_async(queue_capacity);
            let cpu_id = config.runtime.cpu_ids[thread_id];
            let combined = combined_entries.is_some() && config.network.cpu_ids.contains(&cpu_id);
            let (core_tasks, core_task_receiver) = if combined {
                let (sender, receiver) = channel::bounded_async(1);
                (Some(sender), Some(receiver))
            } else {
                (None, None)
            };
            let started_tx = started_tx.clone();
            let shard_config = config.worker_config(thread_id);
            let io_config = config.io_uring.clone();
            let entries = if combined {
                combined_entries.expect("combined ring capacity was validated")
            } else {
                io_config.entries_per_worker
            };
            let event_interval = if combined {
                config
                    .runtime
                    .event_interval
                    .min(config.network.event_interval)
            } else {
                config.runtime.event_interval
            };
            let storage_key_id = server_secret.id;
            let resource_guard = resource_guard.clone();
            let observability = observability.clone();
            let thread = std::thread::Builder::new()
                .name(format!("kvkache-worker-{thread_id}"))
                .spawn(move || {
                    let startup_error = started_tx.clone();
                    let runtime_config = crate::storage_runtime::RuntimeConfig {
                        worker_index: thread_id,
                        entries,
                        event_interval,
                        sqpoll: io_config.sqpoll,
                        sqpoll_cpu: io_config.sqpoll_cpu_ids.get(thread_id).copied(),
                        worker_cpu: cpu_id,
                        simulated_io_latency: Duration::from_micros(
                            config.runtime.simulated_io_latency_us,
                        ),
                    };
                    let sqpoll = runtime_config.sqpoll;
                    let result = crate::storage_runtime::run(runtime_config, async move {
                        if let Some(error) = crate::platform::cpu_assignment_error(
                            &format!("thread {thread_id}"),
                            cpu_id,
                        ) {
                            let _ = started_tx.send(Err(error.clone()));
                            return Err(KvError::Worker(error));
                        }
                        let cache = match Kvkache::open_with_validated_config(
                            shard_config,
                            storage_key_id,
                            resource_guard,
                            allow_checkpoint,
                        )
                        .await
                        {
                            Ok(cache) => cache,
                            Err(error) => {
                                let _ = started_tx.send(Err(
                                    crate::storage_runtime::storage_startup_error(sqpoll, &error),
                                ));
                                return Err(error);
                            }
                        };
                        let storage_device_kind = cache.storage_device_kind;
                        if let Some(receiver) = core_task_receiver {
                            crate::storage_runtime::spawn_detached(run_core_tasks(receiver));
                        }
                        if let Some(observability) = observability.as_ref() {
                            observability.storage_worker_started(thread_id, cpu_id);
                        }
                        let _ = started_tx.send(Ok(storage_device_kind));
                        let result = worker_loop(
                            cache,
                            receiver,
                            io_config,
                            thread_id,
                            cpu_id,
                            observability.clone(),
                        )
                        .await;
                        if let Err(error) = &result {
                            tracing::error!(
                                target: "openkache::storage",
                                worker = thread_id,
                                error = %error,
                                "storage worker stopped"
                            );
                        }
                        if let Some(observability) = observability.as_ref() {
                            observability.storage_worker_stopped(thread_id);
                        }
                        result
                    });
                    match result {
                        Ok(result) => result,
                        Err(error) => {
                            let _ = startup_error.send(Err(
                                crate::storage_runtime::storage_startup_error(sqpoll, &error),
                            ));
                            Err(error.into())
                        }
                    }
                })?;
            workers.push(WorkerHandle {
                sender,
                // Match idle completion retention to request-queue backpressure. Slots created
                // for callers beyond this bound are released after their request completes.
                completions: CompletionSlab::with_retained_capacity(queue_capacity),
                core_tasks,
                thread: Some(thread),
            });
        }
        drop(started_tx);

        let mut storage_device_kind = crate::platform::StorageDeviceKind::NotApplicable;
        for _ in 0..config.runtime.thread_count {
            match started_rx
                .recv()
                .map_err(|_| KvError::Worker("worker startup channel closed".into()))?
            {
                Ok(kind) => {
                    storage_device_kind = storage_device_kind.combine(kind);
                }
                Err(message) => {
                    for worker in &workers {
                        let _ = worker
                            .sender
                            .send(WorkerRequest::Control(WorkerControlRequest::Shutdown));
                    }
                    for worker in &mut workers {
                        if let Some(thread) = worker.thread.take() {
                            let _ = thread.join();
                        }
                    }
                    return Err(KvError::Worker(message));
                }
            }
        }

        Ok(Self {
            config,
            workers,
            storage_domain_key,
            storage_device_kind,
            observability,
        })
    }

    /// Returns the conservative classification of storage used by all workers.
    ///
    /// Native backends classify the files opened during startup. The simulated
    /// backend returns `NotApplicable` because it opens no physical files.
    pub(crate) fn storage_device_kind(&self) -> crate::platform::StorageDeviceKind {
        self.storage_device_kind
    }

    /// Reports whether the storage runtime pinned to `cpu_id` accepts a server role.
    ///
    /// Returns `false` when the CPU has no prepared combined storage worker.
    pub(crate) fn can_run_on_storage_cpu(&self, cpu_id: usize) -> bool {
        self.config
            .runtime
            .cpu_ids
            .iter()
            .position(|candidate| *candidate == cpu_id)
            .is_some_and(|worker_id| self.workers[worker_id].core_tasks.is_some())
    }

    /// Schedules one server role on the storage runtime pinned to `cpu_id`.
    ///
    /// Returns `Ok(false)` when the CPU has no prepared combined storage worker.
    pub(crate) fn run_on_storage_cpu(
        &self,
        cpu_id: usize,
        task: impl FnOnce() + Send + 'static,
    ) -> Result<bool> {
        let Some(worker_id) = self
            .config
            .runtime
            .cpu_ids
            .iter()
            .position(|candidate| *candidate == cpu_id)
        else {
            return Ok(false);
        };
        let Some(sender) = self.workers[worker_id].core_tasks.as_ref() else {
            return Ok(false);
        };
        sender
            .send(CoreTask::Run(Box::new(task)))
            .map_err(|_| KvError::Worker("combined core task queue disconnected".into()))?;
        Ok(true)
    }

    pub fn owner(&self, storage_key: &StorageKey) -> usize {
        storage_key.routing_hash() as usize % self.workers.len()
    }

    /// Derives a storage key from one opaque fixed-width identity.
    ///
    /// The identity is not a wire or API identifier. The returned
    /// key is valid only for this runtime's server-owned storage domain.
    pub fn storage_key_for_identity(
        &self,
        identity: &[u8; crate::types::STORAGE_KEY_BYTES],
    ) -> StorageKey {
        storage_keys::derive_storage_key(&self.storage_domain_key, identity)
    }

    /// Derives a storage key in one opaque storage domain.
    pub(crate) fn storage_key_for_domain_identity(
        &self,
        storage_domain_id: u64,
        identity: &[u8; crate::types::STORAGE_KEY_BYTES],
    ) -> StorageKey {
        storage_keys::derive_storage_key_for_domain(
            &self.storage_domain_key,
            storage_domain_id,
            identity,
        )
    }

    /// Sends one worker request using a reusable completion slot and bounded timeouts.
    async fn request(
        &self,
        worker: usize,
        operation: Operation,
        requester: Option<NetworkWorkerId>,
        build: impl FnOnce(WorkerResponseSender) -> WorkerRequest,
    ) -> Result<WorkerResponse> {
        let (response_tx, response_rx) = self.workers[worker].completions.register();
        let request_started = std::time::Instant::now();
        let enqueue = crate::network_runtime::timeout(
            Duration::from_micros(self.config.timeouts.input_max_time_us),
            self.workers[worker]
                .sender
                .send_async_network(build(response_tx)),
        )
        .await;
        let enqueue_result = match enqueue {
            Ok(result) => result.map_err(|_| KvError::Worker("request queue disconnected".into())),
            Err(_) => {
                if let Some(observability) = self.observability.as_ref() {
                    if let Some(requester) = requester {
                        observability.storage_queue_full(requester.index(), worker);
                    }
                }
                Err(KvError::Timeout("request input"))
            }
        };
        if let Err(error) = enqueue_result {
            if let Some(observability) = self.observability.as_ref() {
                if let Some(requester) = requester {
                    observability.record_storage_wait(
                        requester.index(),
                        worker,
                        operation,
                        request_started.elapsed(),
                    );
                }
            }
            return Err(error);
        }
        let elapsed = request_started.elapsed();
        let request_limit = Duration::from_micros(self.config.timeouts.request_max_time_us);
        let output_limit = Duration::from_micros(self.config.timeouts.output_max_time_us);
        let remaining = request_limit.saturating_sub(elapsed).min(output_limit);
        let result = match crate::network_runtime::timeout(
            remaining,
            response_rx.recv_async_network(),
        )
        .await
        {
            Ok(result) => result
                .map_err(|_| KvError::Worker("worker response disconnected".into()))
                .and_then(|result| result),
            Err(_) => Err(KvError::Timeout("request output")),
        };
        if let Some(observability) = self.observability.as_ref() {
            if let Some(requester) = requester {
                observability.record_storage_wait(
                    requester.index(),
                    worker,
                    operation,
                    request_started.elapsed(),
                );
            }
        }
        result
    }

    pub async fn get(&self, storage_key: StorageKey) -> Result<Option<Vec<u8>>> {
        self.get_stored(storage_key)
            .await
            .map(|value| value.map(StoredItemValue::into_bytes))
    }

    /// Retrieves a value without blocking the caller's async executor thread.
    pub(crate) async fn get_stored(
        &self,
        storage_key: StorageKey,
    ) -> Result<Option<StoredItemValue>> {
        self.get_storage_key_with_requester(storage_key, Operation::unknown(), None)
            .await
    }

    pub(crate) async fn get_storage_key_with_requester(
        &self,
        storage_key: StorageKey,
        operation: Operation,
        requester: Option<NetworkWorkerId>,
    ) -> Result<Option<StoredItemValue>> {
        let worker = self.owner(&storage_key);
        self.request(worker, operation, requester, |response| {
            WorkerRequest::Keyed {
                storage_key,
                command: keyed_storage::get(operation, response),
            }
        })
        .await
        .and_then(|response| keyed_storage::value_response(response, "storage get"))
    }

    pub(crate) async fn set_storage_key_with_requester(
        &self,
        storage_key: StorageKey,
        value: StoredItemValue,
        options: StorageWriteOptions,
        operation: Operation,
        requester: Option<NetworkWorkerId>,
    ) -> Result<SetOutcome> {
        let worker = self.owner(&storage_key);
        self.request(worker, operation, requester, |response| {
            WorkerRequest::Keyed {
                storage_key,
                command: keyed_storage::set(operation, value, options, response),
            }
        })
        .await
        .and_then(|response| keyed_storage::set_response(response, "storage set"))
    }

    pub(crate) async fn delete_storage_key_with_requester(
        &self,
        storage_key: StorageKey,
        operation: Operation,
        requester: Option<NetworkWorkerId>,
    ) -> Result<bool> {
        let worker = self.owner(&storage_key);
        self.request(worker, operation, requester, |response| {
            WorkerRequest::Keyed {
                storage_key,
                command: keyed_storage::delete(operation, response),
            }
        })
        .await
        .and_then(|response| keyed_storage::delete_response(response, "storage delete"))
    }

    pub async fn set(&self, storage_key: StorageKey, value: Vec<u8>) -> Result<SetOutcome> {
        self.set_with_options(
            storage_key,
            StoredItemValue::new(value),
            StorageWriteOptions::default(),
        )
        .await
    }

    pub(crate) async fn set_with_options(
        &self,
        storage_key: StorageKey,
        value: StoredItemValue,
        options: StorageWriteOptions,
    ) -> Result<SetOutcome> {
        self.set_storage_key_with_requester(
            storage_key,
            value,
            options,
            Operation::unknown(),
            None,
        )
        .await
    }

    pub async fn delete(&self, storage_key: StorageKey) -> Result<bool> {
        self.delete_storage_key_with_requester(storage_key, Operation::unknown(), None)
            .await
    }

    pub async fn stats(&self) -> Result<Vec<String>> {
        self.stats_async_with_requester(Operation::unknown(), None)
            .await
    }

    async fn stats_async_with_requester(
        &self,
        operation: Operation,
        requester: Option<NetworkWorkerId>,
    ) -> Result<Vec<String>> {
        let mut stats = Vec::with_capacity(self.workers.len());
        for thread_id in 0..self.workers.len() {
            match self
                .request(
                    thread_id,
                    operation,
                    requester,
                    |response| WorkerRequest::Control(WorkerControlRequest::Stats { response }),
                )
                .await?
            {
                WorkerResponse::Control(WorkerControlResponse::Stats(worker_stats)) => {
                    stats.push(format!("thread={thread_id} {worker_stats}"));
                }
                response => {
                    return Err(KvError::Worker(format!(
                        "unexpected stats response: {response:?}"
                    )));
                }
            }
        }
        Ok(stats)
    }

    pub async fn sync(&self) -> Result<()> {
        self.sync_async_with_requester(Operation::unknown(), None)
            .await
    }

    async fn sync_async_with_requester(
        &self,
        operation: Operation,
        requester: Option<NetworkWorkerId>,
    ) -> Result<()> {
        for thread_id in 0..self.workers.len() {
            match self
                .request(
                    thread_id,
                    operation,
                    requester,
                    |response| WorkerRequest::Control(WorkerControlRequest::Sync { response }),
                )
                .await?
            {
                WorkerResponse::Control(WorkerControlResponse::Synced) => {}
                response => {
                    return Err(KvError::Worker(format!(
                        "unexpected sync response: {response:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Flushes exactly the selected storage workers.
    #[allow(dead_code)]
    pub(crate) async fn sync_workers(&self, workers: &[usize]) -> Result<()> {
        self.sync_workers_async_with_requester(workers, Operation::unknown(), None)
            .await
    }

    async fn sync_workers_async_with_requester(
        &self,
        workers: &[usize],
        operation: Operation,
        requester: Option<NetworkWorkerId>,
    ) -> Result<()> {
        for &worker in workers {
            if worker >= self.workers.len() {
                return Err(KvError::Worker(format!(
                    "unknown storage worker {worker}"
                )));
            }
            match self
                .request(
                    worker,
                    operation,
                    requester,
                    |response| WorkerRequest::Control(WorkerControlRequest::Sync { response }),
                )
                .await?
            {
                WorkerResponse::Control(WorkerControlResponse::Synced) => {}
                response => {
                    return Err(KvError::Worker(format!(
                        "unexpected sync response: {response:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<()> {
        for worker in &self.workers {
            if let Some(sender) = &worker.core_tasks {
                let _ = sender.send(CoreTask::Shutdown);
            }
        }
        let mut shutdown_error = None;
        for worker in &self.workers {
            if worker
                .sender
                .send(WorkerRequest::Control(WorkerControlRequest::Shutdown))
                .is_err()
                && shutdown_error.is_none()
            {
                shutdown_error = Some(KvError::Worker(
                    "worker request queue disconnected during shutdown".into(),
                ));
            }
        }
        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                match thread.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) if shutdown_error.is_none() => shutdown_error = Some(error),
                    Err(_) if shutdown_error.is_none() => {
                        shutdown_error = Some(KvError::Worker("worker thread panicked".into()));
                    }
                    _ => {}
                }
            }
        }
        if let Some(error) = shutdown_error {
            return Err(error);
        }
        crate::storage_backend::finish(&self.config.storage.directory)?;
        Ok(())
    }
}

#[allow(dead_code)]
pub(crate) fn begin_storage_run(directory: &std::path::Path) -> Result<bool> {
    crate::storage_backend::begin_storage_run(directory)
}

#[allow(dead_code)]
pub(crate) fn finish_storage_run(directory: &std::path::Path) -> Result<()> {
    crate::storage_backend::finish_storage_run(directory)
}

#[allow(dead_code)]
pub(crate) fn load_or_create_server_secret(
    directory: &Path,
    existing_storage: bool,
) -> Result<ServerSecret> {
    crate::storage_backend::load_or_create_server_secret(directory, existing_storage)
}
