//! Per-worker request/response types, the worker event loop ([`worker_loop`]),
//! batch processing ([`process_worker_batch`]), and benchmark support types
//! ([`BenchmarkOperation`], [`BenchmarkBatchStats`]).

use std::collections::VecDeque;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use openkache_protocol::{ClientKeyDigest, SetOptions};

use crate::channel::{AsyncReceiver, Receiver, Sender, TryRecvError};
use crate::types::EncodedValue;
use crate::*;

pub(super) enum WorkerRequest {
    Get {
        storage_key: StorageKey,
        response: Sender<Result<WorkerResponse>>,
    },
    Set {
        storage_key: StorageKey,
        value: EncodedValue,
        options: SetOptions,
        response: Sender<Result<WorkerResponse>>,
    },
    Delete {
        storage_key: StorageKey,
        response: Sender<Result<WorkerResponse>>,
    },
    Stats {
        response: Sender<Result<WorkerResponse>>,
    },
    Sync {
        response: Sender<Result<WorkerResponse>>,
    },
    Shutdown {
        response: Sender<Result<WorkerResponse>>,
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

        if process_worker_batch(&mut cache, &mut batch, io_config.max_inflight_per_worker).await? {
            return Ok(());
        }
    }
}

async fn process_worker_batch(
    cache: &mut Kvkache,
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
                let mut pending = FuturesUnordered::new();
                let cache_ref = &*cache;
                let get = |storage_key, response| async move {
                    (response, cache_ref.get_encoded(&storage_key).await)
                };
                pending.push(get(storage_key, response));
                while pending.len() < max_inflight {
                    let Some(WorkerRequest::Get { .. }) = batch.front() else {
                        break;
                    };
                    let WorkerRequest::Get {
                        storage_key,
                        response,
                    } = batch.pop_front().unwrap()
                    else {
                        unreachable!()
                    };
                    pending.push(get(storage_key, response));
                }
                while let Some((response, result)) = pending.next().await {
                    let _ = response.send(result.map(WorkerResponse::Value));
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
                    let _ = response.send(Ok(WorkerResponse::Set(outcome)));
                }
                Err(error) => {
                    let _ = response.send(Err(error));
                }
            },
            WorkerRequest::Delete {
                storage_key,
                response,
            } => match cache.delete(&storage_key).await {
                Ok(deleted) => {
                    let _ = response.send(Ok(WorkerResponse::Deleted(deleted)));
                }
                Err(error) => {
                    let _ = response.send(Err(error));
                }
            },
            WorkerRequest::Stats { response } => {
                let cpu = unsafe { libc::sched_getcpu() };
                let _ = response.send(Ok(WorkerResponse::Stats(format!(
                    "cpu_id={cpu} {}",
                    cache.stats()
                ))));
            }
            WorkerRequest::Sync { response } => {
                let result = cache.sync().await.map(|()| WorkerResponse::Synced);
                let _ = response.send(result);
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
                    let _ = response.send(Err(KvError::Worker(message.clone())));
                }
                return Err(KvError::Worker(message));
            }
        }
    }

    if let Some(response) = shutdown_response {
        let _ = response.send(Ok(WorkerResponse::Shutdown));
        return Ok(true);
    }
    Ok(false)
}
