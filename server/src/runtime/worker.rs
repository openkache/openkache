//! Per-worker request/response types, the rolling keyed event loop
//! ([`worker_loop`]), and benchmark support types ([`BenchmarkOperation`],
//! [`BenchmarkBatchStats`]).

use std::collections::{HashMap, VecDeque};
use std::future::{Future, poll_fn};
use std::task::Poll;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use openkache_protocol::{ItemId, SetOptions};

use crate::channel::{AsyncReceiver, Receiver, Sender, TryRecvError};
use crate::types::StoredItemValue;
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
        value: StoredItemValue,
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
    Value(Option<StoredItemValue>),
    Set(SetOutcome),
    Deleted(bool),
    Stats(String),
    Synced,
    Shutdown,
}

#[derive(Debug)]
pub enum BenchmarkOperation {
    Get(ItemId),
    Set(ItemId, Vec<u8>),
    Delete(ItemId),
}

impl BenchmarkOperation {
    pub(crate) fn item_id(&self) -> ItemId {
        match self {
            Self::Get(item_id) | Self::Delete(item_id) | Self::Set(item_id, _) => *item_id,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SlotId {
    index: u32,
    generation: u32,
}

struct WaitingSlot<T> {
    generation: u32,
    value: Option<T>,
    next: Option<SlotId>,
}

pub(super) struct WaitingSlab<T> {
    slots: Vec<WaitingSlot<T>>,
    free: Vec<u32>,
    capacity: usize,
}

impl<T> WaitingSlab<T> {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            free: Vec::new(),
            capacity,
        }
    }

    pub(super) fn has_capacity(&self) -> bool {
        self.slots.len() < self.capacity || !self.free.is_empty()
    }

    pub(super) fn insert(&mut self, value: T) -> Option<SlotId> {
        let index = if let Some(index) = self.free.pop() {
            index
        } else {
            if self.slots.len() == self.capacity {
                return None;
            }
            let index = u32::try_from(self.slots.len()).ok()?;
            self.slots.push(WaitingSlot {
                generation: 0,
                value: None,
                next: None,
            });
            index
        };
        let slot = &mut self.slots[index as usize];
        debug_assert!(slot.value.is_none());
        slot.value = Some(value);
        slot.next = None;
        Some(SlotId {
            index,
            generation: slot.generation,
        })
    }

    pub(super) fn link(&mut self, tail: SlotId, next: SlotId) {
        let slot = self.slot_mut(tail);
        debug_assert!(slot.next.is_none());
        slot.next = Some(next);
    }

    pub(super) fn take(&mut self, id: SlotId) -> (T, Option<SlotId>) {
        let slot = self.slot_mut(id);
        let value = slot.value.take().expect("waiting slot contains a command");
        let next = slot.next.take();
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.index);
        (value, next)
    }

    fn slot_mut(&mut self, id: SlotId) -> &mut WaitingSlot<T> {
        let slot = self
            .slots
            .get_mut(id.index as usize)
            .expect("waiting SlotId index is valid");
        assert_eq!(
            slot.generation, id.generation,
            "waiting SlotId generation is current"
        );
        slot
    }
}

enum KeyedCommand {
    Get {
        response: WorkerResponseSender,
    },
    Set {
        value: StoredItemValue,
        options: SetOptions,
        response: WorkerResponseSender,
    },
    Delete {
        response: WorkerResponseSender,
    },
}

impl KeyedCommand {
    fn into_parts(self) -> (KeyedOperation, WorkerResponseSender) {
        match self {
            Self::Get { response } => (KeyedOperation::Get, response),
            Self::Set {
                value,
                options,
                response,
            } => (KeyedOperation::Set { value, options }, response),
            Self::Delete { response } => (KeyedOperation::Delete, response),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaneState {
    Ready,
    Running,
}

struct KeyLane {
    state: LaneState,
    waiting_head: Option<SlotId>,
    waiting_tail: Option<SlotId>,
}

struct KeyScheduler {
    lanes: HashMap<StorageKey, KeyLane>,
    ready: VecDeque<StorageKey>,
    waiting: WaitingSlab<KeyedCommand>,
}

impl KeyScheduler {
    fn with_waiting_capacity(capacity: usize) -> Self {
        Self {
            lanes: HashMap::with_capacity(capacity.saturating_mul(2)),
            ready: VecDeque::with_capacity(capacity),
            waiting: WaitingSlab::with_capacity(capacity),
        }
    }

    fn has_waiting_capacity(&self) -> bool {
        self.waiting.has_capacity()
    }

    fn is_idle(&self) -> bool {
        self.lanes.is_empty()
    }

    fn enqueue(&mut self, storage_key: StorageKey, command: KeyedCommand) -> Result<()> {
        let slot = self
            .waiting
            .insert(command)
            .ok_or_else(|| KvError::Worker("waiting-command slab is full".into()))?;
        if let Some(lane) = self.lanes.get_mut(&storage_key) {
            if let Some(tail) = lane.waiting_tail {
                self.waiting.link(tail, slot);
            } else {
                debug_assert!(lane.waiting_head.is_none());
                lane.waiting_head = Some(slot);
            }
            lane.waiting_tail = Some(slot);
            return Ok(());
        }
        self.lanes.insert(
            storage_key,
            KeyLane {
                state: LaneState::Ready,
                waiting_head: Some(slot),
                waiting_tail: Some(slot),
            },
        );
        self.ready.push_back(storage_key);
        Ok(())
    }

    fn start_ready(&mut self, cache: &mut Kvkache) -> Option<RunningKeyedCommand> {
        let storage_key = self.ready.pop_front()?;
        let lane = self
            .lanes
            .get_mut(&storage_key)
            .expect("ready key has a lane");
        debug_assert_eq!(lane.state, LaneState::Ready);
        let head = lane.waiting_head.expect("ready lane has a waiting command");
        let (command, next) = self.waiting.take(head);
        lane.waiting_head = next;
        if next.is_none() {
            lane.waiting_tail = None;
        }
        lane.state = LaneState::Running;
        let (operation, response) = command.into_parts();
        Some(RunningKeyedCommand {
            storage_key,
            response,
            job: cache.prepare_keyed(storage_key, operation),
        })
    }

    fn complete(&mut self, storage_key: StorageKey) {
        let ready_again = {
            let lane = self
                .lanes
                .get_mut(&storage_key)
                .expect("completed key has a lane");
            debug_assert_eq!(lane.state, LaneState::Running);
            if lane.waiting_head.is_some() {
                lane.state = LaneState::Ready;
                true
            } else {
                false
            }
        };
        if ready_again {
            self.ready.push_back(storage_key);
        } else {
            self.lanes.remove(&storage_key);
        }
    }
}

struct RunningKeyedCommand {
    storage_key: StorageKey,
    response: WorkerResponseSender,
    job: KeyedJob,
}

struct CompletedKeyedCommand {
    storage_key: StorageKey,
    response: WorkerResponseSender,
    job: CompletedKeyedJob,
}

async fn run_keyed_command(running: RunningKeyedCommand) -> CompletedKeyedCommand {
    CompletedKeyedCommand {
        storage_key: running.storage_key,
        response: running.response,
        job: running.job.run().await,
    }
}

enum WorkerEvent {
    Completed(CompletedKeyedCommand),
    Request(Option<WorkerRequest>),
}

pub(super) async fn worker_loop(
    mut cache: Kvkache,
    receiver: AsyncReceiver<WorkerRequest>,
    io_config: IoUringConfig,
    affinity_id: usize,
) -> Result<()> {
    let waiting_capacity = io_config.max_inflight_per_worker;
    let mut scheduler = KeyScheduler::with_waiting_capacity(waiting_capacity);
    let mut inflight = FuturesUnordered::new();
    let mut barrier = None;
    let mut disconnected = false;
    let mut flush_requested = false;
    let mut deferred_responses: Vec<(WorkerResponseSender, WorkerResponse)> = Vec::new();

    loop {
        if !flush_requested {
            while inflight.len() < io_config.max_inflight_per_worker {
                let Some(running) = scheduler.start_ready(&mut cache) else {
                    break;
                };
                inflight.push(run_keyed_command(running));
            }
        }

        if inflight.is_empty() && flush_requested {
            let flush_result = cache.flush_capacity().await;
            match flush_result {
                Ok(()) => {
                    for (response, value) in deferred_responses.drain(..) {
                        response.send(Ok(value));
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    for (response, _) in deferred_responses.drain(..) {
                        response.send(Err(KvError::Worker(message.clone())));
                    }
                }
            }
            flush_requested = false;
            continue;
        }

        if inflight.is_empty() && scheduler.is_idle() {
            if let Some(request) = barrier.take() {
                if process_worker_barrier(&mut cache, request, affinity_id).await? {
                    return Ok(());
                }
                continue;
            }
            if disconnected {
                return Err(KvError::Worker("request queue disconnected".into()));
            }
        }

        let can_receive = barrier.is_none()
            && !disconnected
            && scheduler.has_waiting_capacity()
            && !flush_requested;
        let idle_before_event = inflight.is_empty() && scheduler.is_idle();
        let event = if can_receive {
            if inflight.is_empty() {
                WorkerEvent::Request(receiver.recv_async().await.ok())
            } else {
                let mut incoming = std::pin::pin!(receiver.recv_async());
                poll_fn(|context| {
                    if let Poll::Ready(Some(completed)) = inflight.poll_next_unpin(context) {
                        return Poll::Ready(WorkerEvent::Completed(completed));
                    }
                    match incoming.as_mut().poll(context) {
                        Poll::Ready(Ok(request)) => {
                            Poll::Ready(WorkerEvent::Request(Some(request)))
                        }
                        Poll::Ready(Err(_)) => Poll::Ready(WorkerEvent::Request(None)),
                        Poll::Pending => Poll::Pending,
                    }
                })
                .await
            }
        } else {
            WorkerEvent::Completed(
                inflight
                    .next()
                    .await
                    .expect("blocked admission has an in-flight keyed command"),
            )
        };

        match event {
            WorkerEvent::Completed(completed) => {
                let finished = cache.finish_keyed(completed.job);
                match finished.outcome {
                    Ok(outcome) => {
                        let response = match outcome {
                            KeyedOutcome::Value(value) => WorkerResponse::Value(value),
                            KeyedOutcome::Set(outcome) => WorkerResponse::Set(outcome),
                            KeyedOutcome::Deleted(deleted) => WorkerResponse::Deleted(deleted),
                        };
                        if finished.flush_required {
                            flush_requested = true;
                            deferred_responses.push((completed.response, response));
                        } else {
                            completed.response.send(Ok(response));
                        }
                    }
                    Err(error) => completed.response.send(Err(error)),
                }
                scheduler.complete(completed.storage_key);
            }
            WorkerEvent::Request(Some(request)) => {
                admit_worker_request(&mut scheduler, &mut barrier, request)?;
                let mut admitted = 1;
                if idle_before_event
                    && admitted < io_config.batch_size
                    && barrier.is_none()
                    && scheduler.has_waiting_capacity()
                    && io_config.batch_max_wait_us > 0
                {
                    match compio::runtime::time::timeout(
                        Duration::from_micros(io_config.batch_max_wait_us),
                        receiver.recv_async(),
                    )
                    .await
                    {
                        Ok(Ok(request)) => {
                            admit_worker_request(&mut scheduler, &mut barrier, request)?;
                            admitted += 1;
                        }
                        Ok(Err(_)) => disconnected = true,
                        Err(_) => {}
                    }
                }
                while admitted < io_config.batch_size
                    && barrier.is_none()
                    && scheduler.has_waiting_capacity()
                {
                    match receiver.try_recv() {
                        Ok(request) => {
                            admit_worker_request(&mut scheduler, &mut barrier, request)?;
                            admitted += 1;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
            }
            WorkerEvent::Request(None) => {
                disconnected = true;
            }
        }
    }
}

fn admit_worker_request(
    scheduler: &mut KeyScheduler,
    barrier: &mut Option<WorkerRequest>,
    request: WorkerRequest,
) -> Result<()> {
    match request {
        WorkerRequest::Get {
            storage_key,
            response,
        } => scheduler.enqueue(storage_key, KeyedCommand::Get { response }),
        WorkerRequest::Set {
            storage_key,
            value,
            options,
            response,
        } => scheduler.enqueue(
            storage_key,
            KeyedCommand::Set {
                value,
                options,
                response,
            },
        ),
        WorkerRequest::Delete {
            storage_key,
            response,
        } => scheduler.enqueue(storage_key, KeyedCommand::Delete { response }),
        request => {
            *barrier = Some(request);
            Ok(())
        }
    }
}

async fn process_worker_barrier(
    cache: &mut Kvkache,
    request: WorkerRequest,
    affinity_id: usize,
) -> Result<bool> {
    match request {
        WorkerRequest::Stats { response } => {
            response.send(Ok(WorkerResponse::Stats(format!(
                "{} {}",
                crate::platform::cpu_diagnostic(affinity_id),
                cache.stats()
            ))));
            Ok(false)
        }
        WorkerRequest::Sync { response } => {
            let result = cache.sync().await.map(|()| WorkerResponse::Synced);
            response.send(result);
            Ok(false)
        }
        WorkerRequest::Shutdown { response } => {
            let result = match cache.sync().await {
                Ok(()) => cache.checkpoint().await,
                Err(error) => Err(error),
            };
            match result {
                Ok(()) => {
                    response.send(Ok(WorkerResponse::Shutdown));
                    Ok(true)
                }
                Err(error) => {
                    let message = error.to_string();
                    response.send(Err(KvError::Worker(message.clone())));
                    Err(KvError::Worker(message))
                }
            }
        }
        WorkerRequest::Get { .. } | WorkerRequest::Set { .. } | WorkerRequest::Delete { .. } => {
            Err(KvError::Worker(
                "keyed request reached the worker barrier path".into(),
            ))
        }
    }
}
