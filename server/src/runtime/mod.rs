//! Multi-threaded KV cache runtime. [`ThreadedKvkache`] manages a pool of
//! thread-per-core workers, each running a `compio`-based event loop. Handles
//! request routing by key hash, benchmark batch execution, and graceful shutdown.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

use aes::{
    Aes256,
    cipher::{Block, BlockCipherEncrypt, KeyInit},
};
use compio::driver::ProactorBuilder;
use compio::runtime::RuntimeBuilder;
use openkache_protocol::ClientKeyDigest;

use crate::types::EncodedValue;
use crate::*;

mod worker;
pub use worker::*;

pub(crate) const SERVER_KEY_FILE: &str = ".openkache-key";
const SERVER_KEY_MAGIC: &[u8; 8] = b"OKKEY\0\0\0";
const SERVER_KEY_VERSION: u32 = 1;
const SERVER_KEY_FILE_BYTES: usize = 64;

#[derive(Clone, Copy)]
pub(crate) struct ServerSecret {
    pub(crate) id: [u8; 16],
    pub(crate) key: [u8; 32],
}

struct WorkerHandle {
    sender: flume::Sender<WorkerRequest>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub(crate) fn derive_storage_key(
    server_cipher: &Aes256,
    client_key_digest: ClientKeyDigest,
) -> StorageKey {
    let mut bytes = client_key_digest.into_bytes();

    // SAFETY: `Block<Aes256>` is layout-identical to `[u8; 16]`, so two blocks exactly cover
    // the 32-byte digest buffer while preserving its alignment and exclusive borrow.
    let blocks = unsafe { &mut *(bytes.as_mut_ptr() as *mut [Block<Aes256>; 2]) };

    // AES-MDS-AES keeps this fixed-size derivation in place and on the AES hardware path: two
    // parallel AES passes surround an invertible MDS layer so each digest half influences both
    // output blocks. Keyed BLAKE3 is the simpler non-permutation alternative when portability
    // and a conventional keyed hash matter more than minimizing AES-backed latency.
    server_cipher.encrypt_blocks(blocks);

    let [first_block, second_block] = blocks;
    for (first, second) in first_block.iter_mut().zip(second_block.iter_mut()) {
        let first_byte = *first;
        let second_byte = *second;
        *first = first_byte ^ second_byte;
        *second = first_byte ^ gf_double(second_byte);
    }

    server_cipher.encrypt_blocks(blocks);

    StorageKey::new(bytes)
}

fn gf_double(byte: u8) -> u8 {
    (byte << 1) ^ (0x1b & 0u8.wrapping_sub(byte >> 7))
}

pub struct ThreadedKvkache {
    config: crate::config::AppConfig,
    workers: Vec<WorkerHandle>,
    server_cipher: Aes256,
}

impl ThreadedKvkache {
    pub fn start(config: crate::config::AppConfig) -> Result<Self> {
        config.validate()?;
        fs::create_dir_all(&config.storage.directory)?;
        let existing_storage = (0..config.runtime.thread_count).any(|thread_id| {
            let worker = config.worker_config(thread_id);
            worker.data_path.exists() || worker.blob_path().exists()
        });
        let server_secret =
            load_or_create_server_secret(&config.storage.directory, existing_storage)?;
        Self::start_with_server_secret(config, server_secret)
    }

    fn start_with_server_key(
        config: crate::config::AppConfig,
        server_key: [u8; 32],
    ) -> Result<Self> {
        let mut id = [0; 16];
        id.copy_from_slice(&server_key[..16]);
        Self::start_with_server_secret(
            config,
            ServerSecret {
                id,
                key: server_key,
            },
        )
    }

    fn start_with_server_secret(
        config: crate::config::AppConfig,
        server_secret: ServerSecret,
    ) -> Result<Self> {
        config.validate()?;
        fs::create_dir_all(&config.storage.directory)?;
        let server_cipher = Aes256::new(&server_secret.key.into());
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
            let storage_key_id = server_secret.id;
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
                        let cache = match Kvkache::open_with_storage_key_id(
                            shard_config,
                            storage_key_id,
                        )
                        .await
                        {
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
            server_cipher,
        })
    }

    pub fn owner(&self, storage_key: &StorageKey) -> usize {
        u64::from_le_bytes(storage_key.as_bytes()[..8].try_into().unwrap()) as usize
            % self.workers.len()
    }

    fn storage_key(&self, client_key_digest: ClientKeyDigest) -> StorageKey {
        derive_storage_key(&self.server_cipher, client_key_digest)
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
            WorkerResponse::Value(value) => Ok(value.map(|value| value.bytes)),
            response => Err(KvError::Worker(format!(
                "unexpected get response: {response:?}"
            ))),
        }
    }

    /// Retrieves a value without blocking the caller's async executor thread.
    pub(crate) async fn get_async(
        &self,
        client_key_digest: ClientKeyDigest,
    ) -> Result<Option<EncodedValue>> {
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
            value: EncodedValue::plain(value),
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
        value: EncodedValue,
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
        Self::for_trace_benchmark_with_bucket_choices(
            directory,
            cpu_ids,
            total_segment_count,
            total_table_capacity,
            2,
        )
    }

    /// Starts a deterministic benchmark runtime with a configurable Bucket-choice count.
    ///
    /// # Arguments
    ///
    /// * `directory` - Fresh storage directory owned by this benchmark instance.
    /// * `cpu_ids` - One pinned CPU identifier per worker.
    /// * `total_segment_count` - Segment count divided evenly across workers.
    /// * `total_table_capacity` - Planned key capacity divided across workers.
    /// * `bucket_choice_count` - Power-of-two candidate count from 1 through 32.
    ///
    /// # Returns
    ///
    /// A running worker set whose storage-key secret is fixed for reproducible placement.
    ///
    /// # Errors
    ///
    /// Returns an error when benchmark sizing or worker configuration is invalid, or when
    /// worker startup fails.
    pub fn for_trace_benchmark_with_bucket_choices(
        directory: std::path::PathBuf,
        cpu_ids: Vec<usize>,
        total_segment_count: usize,
        total_table_capacity: usize,
        bucket_choice_count: usize,
    ) -> Result<Self> {
        Self::for_trace_benchmark_with_bucket_policy(
            directory,
            cpu_ids,
            total_segment_count,
            total_table_capacity,
            bucket_choice_count,
            crate::config::BucketSelectionPolicy::LeastUsed,
        )
    }

    /// Starts a deterministic benchmark runtime with configurable Bucket placement.
    ///
    /// # Arguments
    ///
    /// * `directory` - Fresh storage directory owned by this benchmark instance.
    /// * `cpu_ids` - One pinned CPU identifier per worker.
    /// * `total_segment_count` - Segment count divided evenly across workers.
    /// * `total_table_capacity` - Planned key capacity divided across workers.
    /// * `bucket_choice_count` - Power-of-two candidate count from 1 through 32.
    /// * `bucket_selection_policy` - Whether fitting Items spread or pack across candidates.
    ///
    /// # Returns
    ///
    /// A running worker set whose storage-key secret is fixed for reproducible placement.
    ///
    /// # Errors
    ///
    /// Returns an error when benchmark sizing or worker configuration is invalid, or when
    /// worker startup fails.
    pub fn for_trace_benchmark_with_bucket_policy(
        directory: std::path::PathBuf,
        cpu_ids: Vec<usize>,
        total_segment_count: usize,
        total_table_capacity: usize,
        bucket_choice_count: usize,
        bucket_selection_policy: crate::config::BucketSelectionPolicy,
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
        let mut config = crate::config::AppConfig::for_trace_benchmark(
            directory,
            cpu_ids,
            total_segment_count,
            total_table_capacity,
        )?;
        config.table.bucket_choice_count = bucket_choice_count;
        config.table.bucket_selection_policy = bucket_selection_policy;
        Self::start_with_server_key(config, [0; 32])
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
                        value: EncodedValue::plain(value),
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

pub(crate) fn load_or_create_server_secret(
    directory: &Path,
    existing_storage: bool,
) -> Result<ServerSecret> {
    let path = directory.join(SERVER_KEY_FILE);
    match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(mut file) => {
            let metadata = file.metadata()?;
            if !metadata.file_type().is_file() {
                return Err(KvError::Worker(format!(
                    "server key file {} must be a regular file",
                    path.display()
                )));
            }
            let permissions = metadata.permissions().mode() & 0o777;
            if permissions & 0o077 != 0 {
                return Err(KvError::Worker(format!(
                    "server key file {} must not be accessible by group or other users",
                    path.display()
                )));
            }
            let mut bytes = Vec::with_capacity(SERVER_KEY_FILE_BYTES);
            file.read_to_end(&mut bytes)?;
            return decode_server_secret(&bytes);
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if existing_storage {
        return Err(KvError::Worker(format!(
            "server key file {} is missing for existing storage",
            path.display()
        )));
    }

    let secret = ServerSecret {
        id: rand::random(),
        key: rand::random(),
    };
    let bytes = encode_server_secret(secret);
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::OpenOptions::new()
        .read(true)
        .open(directory)?
        .sync_all()?;
    Ok(secret)
}

fn encode_server_secret(secret: ServerSecret) -> [u8; SERVER_KEY_FILE_BYTES] {
    let mut bytes = [0; SERVER_KEY_FILE_BYTES];
    bytes[..8].copy_from_slice(SERVER_KEY_MAGIC);
    bytes[8..12].copy_from_slice(&SERVER_KEY_VERSION.to_le_bytes());
    bytes[12..28].copy_from_slice(&secret.id);
    bytes[28..60].copy_from_slice(&secret.key);
    let checksum = server_key_checksum(&bytes[..60]);
    bytes[60..64].copy_from_slice(&checksum.to_le_bytes());
    bytes
}

fn decode_server_secret(bytes: &[u8]) -> Result<ServerSecret> {
    if bytes.len() != SERVER_KEY_FILE_BYTES
        || &bytes[..8] != SERVER_KEY_MAGIC
        || u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != SERVER_KEY_VERSION
        || server_key_checksum(&bytes[..60])
            != u32::from_le_bytes(bytes[60..64].try_into().unwrap())
    {
        return Err(KvError::Worker("server key file is invalid".into()));
    }
    Ok(ServerSecret {
        id: bytes[12..28].try_into().unwrap(),
        key: bytes[28..60].try_into().unwrap(),
    })
}

fn server_key_checksum(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}
