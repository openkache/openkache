//! Multi-threaded KV cache runtime. [`ThreadedKvkache`] manages a pool of
//! thread-per-core workers, each running a `compio`-based event loop. Handles
//! request routing by key hash, benchmark batch execution, and graceful shutdown.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::time::Duration;

use compio::driver::ProactorBuilder;
use compio::runtime::RuntimeBuilder;
use openkache_protocol::ClientKeyDigest;

use crate::*;

mod worker;
pub use worker::*;

struct WorkerHandle {
    sender: flume::Sender<WorkerRequest>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub(crate) fn derive_storage_key(
    server_hash_key: &[u8; blake3::KEY_LEN],
    client_key_digest: ClientKeyDigest,
) -> StorageKey {
    StorageKey::new(*blake3::keyed_hash(server_hash_key, client_key_digest.as_bytes()).as_bytes())
}

pub struct ThreadedKvkache {
    config: crate::config::AppConfig,
    workers: Vec<WorkerHandle>,
    server_hash_key: [u8; blake3::KEY_LEN],
}

impl ThreadedKvkache {
    pub fn start(config: crate::config::AppConfig) -> Result<Self> {
        config.validate()?;
        fs::create_dir_all(&config.storage.directory)?;
        let server_hash_key = rand::random::<[u8; blake3::KEY_LEN]>();
        let (started_tx, started_rx) =
            flume::bounded::<std::result::Result<(), String>>(config.runtime.thread_count);
        let queue_capacity = config
            .io_uring
            .batch_size
            .saturating_mul(config.io_uring.max_inflight_per_worker)
            .max(64);
        let mut workers = Vec::with_capacity(config.runtime.thread_count);

        for thread_id in 0..config.runtime.thread_count {
            let (sender, receiver) = flume::bounded(queue_capacity);
            let started_tx = started_tx.clone();
            let shard_config = config.worker_config(thread_id);
            let io_config = config.io_uring.clone();
            let cpu_id = config.runtime.cpu_ids[thread_id];
            let event_interval = config.runtime.event_interval;
            let thread = std::thread::Builder::new()
                .name(format!("kvkache-worker-{thread_id}"))
                .spawn(move || {
                    let mut proactor = ProactorBuilder::new();
                    proactor.capacity(io_config.entries_per_worker);
                    let cpus = HashSet::from([cpu_id]);
                    let runtime = RuntimeBuilder::new()
                        .with_proactor(proactor)
                        .thread_affinity(cpus)
                        .event_interval(event_interval)
                        .build();
                    let runtime = match runtime {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            let _ = started_tx.send(Err(error.to_string()));
                            return;
                        }
                    };
                    runtime.block_on(async move {
                        let actual_cpu = unsafe { libc::sched_getcpu() };
                        if actual_cpu < 0 || actual_cpu as usize != cpu_id {
                            let _ = started_tx.send(Err(format!(
                                "thread {thread_id} expected CPU {cpu_id}, running on CPU {actual_cpu}"
                            )));
                            return;
                        }
                        let cache = match Kvkache::open(shard_config).await {
                            Ok(cache) => cache,
                            Err(error) => {
                                let _ = started_tx.send(Err(error.to_string()));
                                return;
                            }
                        };
                        let _ = started_tx.send(Ok(()));
                        if let Err(error) = worker_loop(cache, receiver, io_config).await {
                            eprintln!("worker {thread_id} stopped: {error}");
                        }
                    });
                })?;
            workers.push(WorkerHandle {
                sender,
                thread: Some(thread),
            });
        }
        drop(started_tx);

        for _ in 0..config.runtime.thread_count {
            match started_rx
                .recv()
                .map_err(|_| KvError::Worker("worker startup channel closed".into()))?
            {
                Ok(()) => {}
                Err(message) => {
                    for worker in &workers {
                        let (response, _) = flume::bounded(1);
                        let _ = worker.sender.send(WorkerRequest::Shutdown { response });
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
            server_hash_key,
        })
    }

    pub fn owner(&self, storage_key: &StorageKey) -> usize {
        u64::from_le_bytes(storage_key.as_bytes()[..8].try_into().unwrap()) as usize
            % self.workers.len()
    }

    fn storage_key(&self, client_key_digest: ClientKeyDigest) -> StorageKey {
        derive_storage_key(&self.server_hash_key, client_key_digest)
    }

    fn request(
        &self,
        worker: usize,
        build: impl FnOnce(flume::Sender<Result<WorkerResponse>>) -> WorkerRequest,
    ) -> Result<WorkerResponse> {
        let (response_tx, response_rx) = flume::bounded(1);
        let request_started = std::time::Instant::now();
        self.workers[worker]
            .sender
            .send_timeout(
                build(response_tx),
                Duration::from_micros(self.config.timeouts.input_max_time_us),
            )
            .map_err(|_| KvError::Timeout("request input"))?;
        let elapsed = request_started.elapsed();
        let request_limit = Duration::from_micros(self.config.timeouts.request_max_time_us);
        let output_limit = Duration::from_micros(self.config.timeouts.output_max_time_us);
        let remaining = request_limit.saturating_sub(elapsed).min(output_limit);
        response_rx
            .recv_timeout(remaining)
            .map_err(|_| KvError::Timeout("request output"))?
    }

    /// Sends one worker request using async channel operations and bounded timeouts.
    async fn request_async(
        &self,
        worker: usize,
        build: impl FnOnce(flume::Sender<Result<WorkerResponse>>) -> WorkerRequest,
    ) -> Result<WorkerResponse> {
        let (response_tx, response_rx) = flume::bounded(1);
        let request_started = std::time::Instant::now();
        compio::runtime::time::timeout(
            Duration::from_micros(self.config.timeouts.input_max_time_us),
            self.workers[worker].sender.send_async(build(response_tx)),
        )
        .await
        .map_err(|_| KvError::Timeout("request input"))?
        .map_err(|_| KvError::Worker("request queue disconnected".into()))?;
        let elapsed = request_started.elapsed();
        let request_limit = Duration::from_micros(self.config.timeouts.request_max_time_us);
        let output_limit = Duration::from_micros(self.config.timeouts.output_max_time_us);
        let remaining = request_limit.saturating_sub(elapsed).min(output_limit);
        compio::runtime::time::timeout(remaining, response_rx.recv_async())
            .await
            .map_err(|_| KvError::Timeout("request output"))?
            .map_err(|_| KvError::Worker("response queue disconnected".into()))?
    }

    pub fn get(&self, client_key_digest: ClientKeyDigest) -> Result<Option<Vec<u8>>> {
        let storage_key = self.storage_key(client_key_digest);
        let worker = self.owner(&storage_key);
        match self.request(worker, |response| WorkerRequest::Get {
            storage_key,
            response,
        })? {
            WorkerResponse::Value(value) => Ok(value),
            response => Err(KvError::Worker(format!(
                "unexpected get response: {response:?}"
            ))),
        }
    }

    /// Retrieves a value without blocking the caller's async executor thread.
    pub(crate) async fn get_async(
        &self,
        client_key_digest: ClientKeyDigest,
    ) -> Result<Option<Vec<u8>>> {
        let storage_key = self.storage_key(client_key_digest);
        let worker = self.owner(&storage_key);
        match self
            .request_async(worker, |response| WorkerRequest::Get {
                storage_key,
                response,
            })
            .await?
        {
            WorkerResponse::Value(value) => Ok(value),
            response => Err(KvError::Worker(format!(
                "unexpected get response: {response:?}"
            ))),
        }
    }

    pub fn set(&self, client_key_digest: ClientKeyDigest, value: Vec<u8>) -> Result<SetOutcome> {
        let storage_key = self.storage_key(client_key_digest);
        let worker = self.owner(&storage_key);
        match self.request(worker, |response| WorkerRequest::Set {
            storage_key,
            value,
            response,
        })? {
            WorkerResponse::Set(outcome) => Ok(outcome),
            response => Err(KvError::Worker(format!(
                "unexpected set response: {response:?}"
            ))),
        }
    }

    /// Stores a value without blocking the caller's async executor thread.
    pub(crate) async fn set_async(
        &self,
        client_key_digest: ClientKeyDigest,
        value: Vec<u8>,
    ) -> Result<SetOutcome> {
        let storage_key = self.storage_key(client_key_digest);
        let worker = self.owner(&storage_key);
        match self
            .request_async(worker, |response| WorkerRequest::Set {
                storage_key,
                value,
                response,
            })
            .await?
        {
            WorkerResponse::Set(outcome) => Ok(outcome),
            response => Err(KvError::Worker(format!(
                "unexpected set response: {response:?}"
            ))),
        }
    }

    pub fn delete(&self, client_key_digest: ClientKeyDigest) -> Result<bool> {
        let storage_key = self.storage_key(client_key_digest);
        let worker = self.owner(&storage_key);
        match self.request(worker, |response| WorkerRequest::Delete {
            storage_key,
            response,
        })? {
            WorkerResponse::Deleted(deleted) => Ok(deleted),
            response => Err(KvError::Worker(format!(
                "unexpected delete response: {response:?}"
            ))),
        }
    }

    /// Deletes a value without blocking the caller's async executor thread.
    pub(crate) async fn delete_async(&self, client_key_digest: ClientKeyDigest) -> Result<bool> {
        let storage_key = self.storage_key(client_key_digest);
        let worker = self.owner(&storage_key);
        match self
            .request_async(worker, |response| WorkerRequest::Delete {
                storage_key,
                response,
            })
            .await?
        {
            WorkerResponse::Deleted(deleted) => Ok(deleted),
            response => Err(KvError::Worker(format!(
                "unexpected delete response: {response:?}"
            ))),
        }
    }

    pub fn for_trace_benchmark(
        directory: std::path::PathBuf,
        cpu_ids: Vec<usize>,
        total_segment_count: usize,
        total_table_capacity: usize,
    ) -> Result<Self> {
        let thread_count = cpu_ids.len();
        if thread_count == 0 {
            return Err(KvError::InvalidConfig(
                "benchmark requires at least one CPU".into(),
            ));
        }
        if total_table_capacity / thread_count.max(1) > 750_000 {
            return Err(KvError::InvalidConfig(
                "benchmark window is too large".into(),
            ));
        }
        Self::start(crate::config::AppConfig::for_trace_benchmark(
            directory,
            cpu_ids,
            total_segment_count,
            total_table_capacity,
        )?)
    }

    pub fn run_benchmark_batch(
        &self,
        operations: Vec<BenchmarkOperation>,
        max_outstanding_per_worker: usize,
    ) -> Result<BenchmarkBatchStats> {
        if max_outstanding_per_worker == 0 {
            return Err(KvError::InvalidConfig(
                "benchmark max outstanding per worker must be non-zero".into(),
            ));
        }
        let max_outstanding = max_outstanding_per_worker
            .checked_mul(self.workers.len())
            .ok_or_else(|| KvError::InvalidConfig("benchmark window is too large".into()))?;
        let mut pending = VecDeque::with_capacity(max_outstanding);
        let mut stats = BenchmarkBatchStats {
            latency_ns: Vec::with_capacity(operations.len()),
            ..BenchmarkBatchStats::default()
        };

        for operation in operations {
            if pending.len() == max_outstanding {
                self.finish_benchmark_request(pending.pop_front().unwrap(), &mut stats)?;
            }
            let storage_key = self.storage_key(operation.client_key_digest());
            let worker = self.owner(&storage_key);
            let (response_tx, response_rx) = flume::bounded(1);
            let (request, kind) = match operation {
                BenchmarkOperation::Get(_) => (
                    WorkerRequest::Get {
                        storage_key,
                        response: response_tx,
                    },
                    BenchmarkResponseKind::Get,
                ),
                BenchmarkOperation::Set(_, value) => (
                    WorkerRequest::Set {
                        storage_key,
                        value,
                        response: response_tx,
                    },
                    BenchmarkResponseKind::Set,
                ),
                BenchmarkOperation::Delete(_) => (
                    WorkerRequest::Delete {
                        storage_key,
                        response: response_tx,
                    },
                    BenchmarkResponseKind::Delete,
                ),
            };
            let started = std::time::Instant::now();
            self.workers[worker]
                .sender
                .send_timeout(
                    request,
                    Duration::from_micros(self.config.timeouts.input_max_time_us),
                )
                .map_err(|_| KvError::Timeout("benchmark request input"))?;
            pending.push_back(PendingBenchmarkRequest {
                response: response_rx,
                kind,
                started,
            });
        }
        while let Some(request) = pending.pop_front() {
            self.finish_benchmark_request(request, &mut stats)?;
        }
        Ok(stats)
    }

    fn finish_benchmark_request(
        &self,
        pending: PendingBenchmarkRequest,
        stats: &mut BenchmarkBatchStats,
    ) -> Result<()> {
        let request_limit = Duration::from_micros(self.config.timeouts.request_max_time_us);
        let output_limit = Duration::from_micros(self.config.timeouts.output_max_time_us);
        let remaining = request_limit
            .saturating_sub(pending.started.elapsed())
            .min(output_limit);
        let response = pending
            .response
            .recv_timeout(remaining)
            .map_err(|_| KvError::Timeout("benchmark request output"))??;
        stats.operations += 1;
        stats
            .latency_ns
            .push(pending.started.elapsed().as_nanos() as u64);
        match (pending.kind, response) {
            (BenchmarkResponseKind::Get, WorkerResponse::Value(value)) => {
                stats.gets += 1;
                stats.hits += value.is_some() as usize;
            }
            (BenchmarkResponseKind::Set, WorkerResponse::Set(outcome)) => {
                stats.sets += 1;
                match outcome {
                    SetOutcome::Created => stats.creates += 1,
                    SetOutcome::Replaced => stats.replaces += 1,
                }
            }
            (BenchmarkResponseKind::Delete, WorkerResponse::Deleted(deleted)) => {
                stats.deletes += 1;
                stats.deleted += deleted as usize;
            }
            (_, response) => {
                return Err(KvError::Worker(format!(
                    "unexpected benchmark response: {response:?}"
                )));
            }
        }
        Ok(())
    }

    pub fn stats(&self) -> Result<Vec<String>> {
        self.workers
            .iter()
            .enumerate()
            .map(|(thread_id, _)| {
                match self.request(thread_id, |response| WorkerRequest::Stats { response })? {
                    WorkerResponse::Stats(stats) => Ok(format!("thread={thread_id} {stats}")),
                    response => Err(KvError::Worker(format!(
                        "unexpected stats response: {response:?}"
                    ))),
                }
            })
            .collect()
    }

    /// Collects worker statistics without blocking the caller's async executor thread.
    pub(crate) async fn stats_async(&self) -> Result<Vec<String>> {
        let mut stats = Vec::with_capacity(self.workers.len());
        for thread_id in 0..self.workers.len() {
            match self
                .request_async(thread_id, |response| WorkerRequest::Stats { response })
                .await?
            {
                WorkerResponse::Stats(worker_stats) => {
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

    pub fn sync(&self) -> Result<()> {
        for thread_id in 0..self.workers.len() {
            match self.request(thread_id, |response| WorkerRequest::Sync { response })? {
                WorkerResponse::Synced => {}
                response => {
                    return Err(KvError::Worker(format!(
                        "unexpected sync response: {response:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Flushes every worker without blocking the caller's async executor thread.
    pub(crate) async fn sync_async(&self) -> Result<()> {
        for thread_id in 0..self.workers.len() {
            match self
                .request_async(thread_id, |response| WorkerRequest::Sync { response })
                .await?
            {
                WorkerResponse::Synced => {}
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
        for thread_id in 0..self.workers.len() {
            match self.request(thread_id, |response| WorkerRequest::Shutdown { response })? {
                WorkerResponse::Shutdown => {}
                response => {
                    return Err(KvError::Worker(format!(
                        "unexpected shutdown response: {response:?}"
                    )));
                }
            }
        }
        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                thread
                    .join()
                    .map_err(|_| KvError::Worker("worker thread panicked".into()))?;
            }
        }
        Ok(())
    }
}
