//! Per-worker request/response types, the worker event loop ([`worker_loop`]),
//! batch processing ([`process_worker_batch`]), and benchmark support types
//! ([`BenchmarkOperation`], [`BenchmarkBatchStats`]).

use std::collections::VecDeque;
use std::future::{Future, poll_fn};
use std::task::Poll;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use openkache_protocol::{ClientKeyDigest, SetOptions};

use crate::channel::{AsyncReceiver, Receiver, Sender, TryRecvError};
use crate::types::{EncodedValue, RetrievedValue};
use crate::*;

use super::completion::CompletionSender;

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
    Value(Option<RetrievedValue>),
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

pub(super) enum GetRunAdmission {
    Get {
        storage_key: StorageKey,
        response: WorkerResponseSender,
    },
    Barrier(WorkerRequest),
    Empty,
    Disconnected,
}

pub(super) fn next_get_run_admission(
    receiver: &AsyncReceiver<WorkerRequest>,
    batch: &mut VecDeque<WorkerRequest>,
) -> GetRunAdmission {
    let request = if let Some(request) = batch.pop_front() {
        Ok(request)
    } else {
        receiver.try_recv()
    };
    match request {
        Ok(WorkerRequest::Get {
            storage_key,
            response,
        }) => GetRunAdmission::Get {
            storage_key,
            response,
        },
        Ok(request) => GetRunAdmission::Barrier(request),
        Err(TryRecvError::Empty) => GetRunAdmission::Empty,
        Err(TryRecvError::Disconnected) => GetRunAdmission::Disconnected,
    }
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
            match next_get_run_admission(receiver, batch) {
                GetRunAdmission::Get {
                    storage_key,
                    response,
                } => pending.push(get(storage_key, response)),
                GetRunAdmission::Barrier(request) => barrier = Some(request),
                GetRunAdmission::Empty => break,
                GetRunAdmission::Disconnected => disconnected = true,
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
            let (response, result) = pending
                .next()
                .await
                .expect("a non-empty GET run has a pending request");
            response.send(result.map(WorkerResponse::Value));
            continue;
        }

        let mut incoming = std::pin::pin!(receiver.recv_async());
        poll_fn(|context| {
            if let Poll::Ready(Some((response, result))) = pending.poll_next_unpin(context) {
                response.send(result.map(WorkerResponse::Value));
                return Poll::Ready(());
            }
            match incoming.as_mut().poll(context) {
                Poll::Ready(Ok(WorkerRequest::Get {
                    storage_key,
                    response,
                })) => pending.push(get(storage_key, response)),
                Poll::Ready(Ok(request)) => barrier = Some(request),
                Poll::Ready(Err(_)) => disconnected = true,
                Poll::Pending => return Poll::Pending,
            }
            Poll::Ready(())
        })
        .await;
    }
}

async fn process_worker_batch(
    cache: &mut Kvkache,
    receiver: &AsyncReceiver<WorkerRequest>,
    batch: &mut VecDeque<WorkerRequest>,
    max_inflight: usize,
) -> Result<bool> {
    cache.finish_inflight_if_ready().await?;
    let mut shutdown_response = None;

    while !batch.is_empty() {
        let request = batch
            .pop_front()
            .expect("a non-empty worker batch has a front request");
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

    if shutdown_response.is_none() && cache.has_inflight_flush() {
        yield_to_background_io().await;
        cache.finish_inflight_if_ready().await?;
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

async fn yield_to_background_io() {
    let mut yielded = false;
    poll_fn(move |context| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await
}
