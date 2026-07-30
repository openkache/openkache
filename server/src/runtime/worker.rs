//! Per-worker request/response types, the worker event loop ([`worker_loop`]),
//! rolling GET scheduling, and benchmark support types ([`BenchmarkOperation`],
//! [`BenchmarkBatchStats`]).

use std::collections::VecDeque;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::{ClientKeyDigest, SetOptions};

use crate::channel::{AsyncReceiver, Receiver, Sender, TryRecvError};
use crate::types::EncodedValue;
use crate::*;

use super::CoreTask;
use super::completion::CompletionSender;

pub(super) async fn run_core_tasks(receiver: AsyncReceiver<CoreTask>) {
    while let Ok(task) = receiver.recv_async().await {
        task();
    }
}

pub(super) enum WorkerResponseSender {
    Channel(Sender<Result<WorkerResponse>>),
    Completion(CompletionSender<Result<WorkerResponse>>),
}

impl WorkerResponseSender {
    pub(super) fn channel(sender: Sender<Result<WorkerResponse>>) -> Self {
        Self::Channel(sender)
    }

    pub(super) fn completion(sender: CompletionSender<Result<WorkerResponse>>) -> Self {
        Self::Completion(sender)
    }

    fn send(self, response: Result<WorkerResponse>) {
        match self {
            Self::Channel(sender) => {
                let _ = sender.send(response);
            }
            Self::Completion(sender) => {
                let _ = sender.send(response);
            }
        }
    }
}

pub(super) enum WorkerRequest {
    Get {
        storage_key: StorageKey,
        response: WorkerResponseSender,
    },
    Set {
        storage_key: StorageKey,
        value: EncodedValue,
        options: SetOptions,
        response: WorkerResponseSender,
    },
    Delete {
        storage_key: StorageKey,
        response: WorkerResponseSender,
    },
    Stats {
        response: WorkerResponseSender,
    },
    Sync {
        response: WorkerResponseSender,
    },
    Shutdown {
        response: WorkerResponseSender,
    },
}

#[derive(Debug)]
pub(super) enum WorkerResponse {
    Value(Option<EncodedValue>),
    Set(SetOutcome),
    Deleted(bool),
    Stats(String),
    Synced,
    Shutdown,
}

#[derive(Debug)]
pub enum BenchmarkOperation {
    Get(ClientKeyDigest),
    Set(ClientKeyDigest, Vec<u8>),
    Delete(ClientKeyDigest),
}

impl BenchmarkOperation {
    pub(crate) fn client_key_digest(&self) -> ClientKeyDigest {
        match self {
            Self::Get(client_key_digest)
            | Self::Delete(client_key_digest)
            | Self::Set(client_key_digest, _) => *client_key_digest,
        }
    }
}

#[derive(Debug, Default)]
pub struct BenchmarkBatchStats {
    pub operations: usize,
    pub gets: usize,
    pub hits: usize,
    pub sets: usize,
    pub creates: usize,
    pub replaces: usize,
    pub deletes: usize,
    pub deleted: usize,
    pub latency_ns: Vec<u64>,
}

#[derive(Clone, Debug)]
pub struct TraceBenchmarkIoConfig {
    pub max_inflight_per_worker: usize,
    pub sqpoll_cpu_ids: Vec<usize>,
}

impl Default for TraceBenchmarkIoConfig {
    fn default() -> Self {
        Self {
            max_inflight_per_worker: IoUringConfig::default().max_inflight_per_worker,
            sqpoll_cpu_ids: Vec::new(),
        }
    }
}

impl BenchmarkBatchStats {
    pub fn merge(&mut self, mut other: Self) {
        self.operations += other.operations;
        self.gets += other.gets;
        self.hits += other.hits;
        self.sets += other.sets;
        self.creates += other.creates;
        self.replaces += other.replaces;
        self.deletes += other.deletes;
        self.deleted += other.deleted;
        self.latency_ns.append(&mut other.latency_ns);
    }
}

#[derive(Clone, Copy)]
pub(super) enum BenchmarkResponseKind {
    Get,
    Set,
    Delete,
}

pub(super) struct PendingBenchmarkRequest {
    pub(super) response: Receiver<Result<WorkerResponse>>,
    pub(super) kind: BenchmarkResponseKind,
    pub(super) started: std::time::Instant,
}

pub(super) async fn worker_loop(
    mut cache: Kvkache,
    receiver: AsyncReceiver<WorkerRequest>,
    io_config: IoUringConfig,
) -> Result<()> {
    let mut batch = VecDeque::with_capacity(io_config.batch_size);
    loop {
        let first = receiver
            .recv_async()
            .await
            .map_err(|_| KvError::Worker("request queue disconnected".into()))?;
        let wait_us = io_config.batch_max_wait_us;
        batch.push_back(first);

        if batch.len() < io_config.batch_size
            && wait_us > 0
            && let Ok(Ok(request)) = compio::runtime::time::timeout(
                Duration::from_micros(wait_us),
                receiver.recv_async(),
            )
            .await
        {
            batch.push_back(request);
        }
        while batch.len() < io_config.batch_size {
            match receiver.try_recv() {
                Ok(request) => batch.push_back(request),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }

        if process_worker_batch(
            &mut cache,
            &receiver,
            &mut batch,
            io_config.max_inflight_per_worker,
        )
        .await?
        {
            return Ok(());
        }
    }
}

enum GetRunExit {
    QueueEmpty,
    Barrier(WorkerRequest),
    Disconnected,
}

enum GetRunEvent {
    Completed(WorkerResponseSender, Result<Option<EncodedValue>>),
    Request(Option<WorkerRequest>),
}

async fn process_get_run(
    cache: &Kvkache,
    receiver: &AsyncReceiver<WorkerRequest>,
    batch: &mut VecDeque<WorkerRequest>,
    first_storage_key: StorageKey,
    first_response: WorkerResponseSender,
    max_inflight: usize,
) -> GetRunExit {
    let mut pending = FuturesUnordered::new();
    let get =
        |storage_key, response| async move { (response, cache.get_encoded(&storage_key).await) };
    pending.push(get(first_storage_key, first_response));
    let mut barrier = None;
    let mut disconnected = false;

    loop {
        while barrier.is_none() && !disconnected && pending.len() < max_inflight {
            let request = if let Some(request) = batch.pop_front() {
                Ok(request)
            } else {
                receiver.try_recv()
            };
            match request {
                Ok(WorkerRequest::Get {
                    storage_key,
                    response,
                }) => pending.push(get(storage_key, response)),
                Ok(request) => barrier = Some(request),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }

        if pending.is_empty() {
            return if let Some(request) = barrier {
                GetRunExit::Barrier(request)
            } else if disconnected {
                GetRunExit::Disconnected
            } else {
                GetRunExit::QueueEmpty
            };
        }

        if barrier.is_some() || disconnected || pending.len() == max_inflight {
            let (response, result) = pending.next().await.unwrap();
            response.send(result.map(WorkerResponse::Value));
            continue;
        }

        let event = {
            let completed = pending.next().fuse();
            let incoming = receiver.recv_async().fuse();
            pin_mut!(completed, incoming);
            select! {
                completed = completed => {
                    let (response, result) = completed.unwrap();
                    GetRunEvent::Completed(response, result)
                }
                incoming = incoming => GetRunEvent::Request(incoming.ok()),
            }
        };
        match event {
            GetRunEvent::Completed(response, result) => {
                response.send(result.map(WorkerResponse::Value));
            }
            GetRunEvent::Request(Some(WorkerRequest::Get {
                storage_key,
                response,
            })) => pending.push(get(storage_key, response)),
            GetRunEvent::Request(Some(request)) => barrier = Some(request),
            GetRunEvent::Request(None) => disconnected = true,
        }
    }
}

async fn process_worker_batch(
    cache: &mut Kvkache,
    receiver: &AsyncReceiver<WorkerRequest>,
    batch: &mut VecDeque<WorkerRequest>,
    max_inflight: usize,
) -> Result<bool> {
    let mut shutdown_response = None;

    while let Some(request) = batch.pop_front() {
        match request {
            WorkerRequest::Get {
                storage_key,
                response,
            } => {
                match process_get_run(
                    &*cache,
                    receiver,
                    batch,
                    storage_key,
                    response,
                    max_inflight,
                )
                .await
                {
                    GetRunExit::QueueEmpty => {}
                    GetRunExit::Barrier(request) => batch.push_front(request),
                    GetRunExit::Disconnected => {
                        return Err(KvError::Worker("request queue disconnected".into()));
                    }
                }
            }
            WorkerRequest::Set {
                storage_key,
                value,
                options,
                response,
            } => match cache
                .set_encoded_with_options(storage_key, value, options)
                .await
            {
                Ok(outcome) => {
                    response.send(Ok(WorkerResponse::Set(outcome)));
                }
                Err(error) => {
                    response.send(Err(error));
                }
            },
            WorkerRequest::Delete {
                storage_key,
                response,
            } => match cache.delete(&storage_key).await {
                Ok(deleted) => {
                    response.send(Ok(WorkerResponse::Deleted(deleted)));
                }
                Err(error) => {
                    response.send(Err(error));
                }
            },
            WorkerRequest::Stats { response } => {
                let cpu = unsafe { libc::sched_getcpu() };
                response.send(Ok(WorkerResponse::Stats(format!(
                    "cpu_id={cpu} {}",
                    cache.stats()
                ))));
            }
            WorkerRequest::Sync { response } => {
                let result = cache.sync().await.map(|()| WorkerResponse::Synced);
                response.send(result);
            }
            WorkerRequest::Shutdown { response } => {
                shutdown_response = Some(response);
                break;
            }
        }
    }

    if shutdown_response.is_some() {
        let result = match cache.sync().await {
            Ok(()) => cache.checkpoint().await,
            Err(error) => Err(error),
        };
        match result {
            Ok(()) => {}
            Err(error) => {
                let message = error.to_string();
                if let Some(response) = shutdown_response {
                    response.send(Err(KvError::Worker(message.clone())));
                }
                return Err(KvError::Worker(message));
            }
        }
    }

    if let Some(response) = shutdown_response {
        response.send(Ok(WorkerResponse::Shutdown));
        return Ok(true);
    }
    Ok(false)
}
