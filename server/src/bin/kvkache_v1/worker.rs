// Worker request protocol, bounded batches, and benchmark request aggregation.

enum WorkerRequest {
    Get {
        key: Vec<u8>,
        response: flume::Sender<Result<WorkerResponse>>,
    },
    Set {
        key: Vec<u8>,
        value: Vec<u8>,
        response: flume::Sender<Result<WorkerResponse>>,
    },
    Delete {
        key: Vec<u8>,
        response: flume::Sender<Result<WorkerResponse>>,
    },
    Stats {
        response: flume::Sender<Result<WorkerResponse>>,
    },
    Sync {
        response: flume::Sender<Result<WorkerResponse>>,
    },
    Shutdown {
        response: flume::Sender<Result<WorkerResponse>>,
    },
}

#[derive(Debug)]
enum WorkerResponse {
    Value(Option<Vec<u8>>),
    Set(SetOutcome),
    Deleted(bool),
    Stats(String),
    Synced,
    Shutdown,
}

#[derive(Debug)]
pub(crate) enum BenchmarkOperation {
    Get(Vec<u8>),
    Set(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
}

impl BenchmarkOperation {
    fn key(&self) -> &[u8] {
        match self {
            Self::Get(key) | Self::Delete(key) | Self::Set(key, _) => key,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct BenchmarkBatchStats {
    pub(crate) operations: usize,
    pub(crate) gets: usize,
    pub(crate) hits: usize,
    pub(crate) sets: usize,
    pub(crate) creates: usize,
    pub(crate) replaces: usize,
    pub(crate) deletes: usize,
    pub(crate) deleted: usize,
    pub(crate) latency_ns: Vec<u64>,
}

impl BenchmarkBatchStats {
    pub(crate) fn merge(&mut self, mut other: Self) {
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
enum BenchmarkResponseKind {
    Get,
    Set,
    Delete,
}

struct PendingBenchmarkRequest {
    response: flume::Receiver<Result<WorkerResponse>>,
    kind: BenchmarkResponseKind,
    started: std::time::Instant,
}

async fn worker_loop(
    mut cache: Kvkache,
    receiver: flume::Receiver<WorkerRequest>,
    io_config: IoUringConfig,
) -> Result<()> {
    loop {
        let first = receiver
            .recv_async()
            .await
            .map_err(|_| KvError::Worker("request queue disconnected".into()))?;
        let wait_us = io_config.batch_max_wait_us;
        let mut batch = VecDeque::with_capacity(io_config.batch_size);
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
                Err(flume::TryRecvError::Empty | flume::TryRecvError::Disconnected) => break,
            }
        }

        if process_worker_batch(&mut cache, batch, io_config.max_inflight_per_worker).await? {
            return Ok(());
        }
    }
}

async fn process_worker_batch(
    cache: &mut Kvkache,
    mut batch: VecDeque<WorkerRequest>,
    max_inflight: usize,
) -> Result<bool> {
    let mut shutdown_response = None;

    while let Some(request) = batch.pop_front() {
        match request {
            WorkerRequest::Get { key, response } => {
                let mut keys = vec![key];
                let mut responses = vec![response];
                while keys.len() < max_inflight {
                    let Some(WorkerRequest::Get { .. }) = batch.front() else {
                        break;
                    };
                    let WorkerRequest::Get { key, response } = batch.pop_front().unwrap() else {
                        unreachable!()
                    };
                    keys.push(key);
                    responses.push(response);
                }
                let results = cache.get_many(keys).await;
                for (response, result) in responses.into_iter().zip(results) {
                    let _ = response.send(result.map(WorkerResponse::Value));
                }
            }
            WorkerRequest::Set {
                key,
                value,
                response,
            } => match cache.set(&key, &value).await {
                Ok(outcome) => {
                    let _ = response.send(Ok(WorkerResponse::Set(outcome)));
                }
                Err(error) => {
                    let _ = response.send(Err(error));
                }
            },
            WorkerRequest::Delete { key, response } => match cache.delete(&key).await {
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
        match cache.sync().await {
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
