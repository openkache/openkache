//! Per-worker request/response types, the rolling keyed event loop
//! ([`worker_loop`]) and per-worker scheduler support types.

use std::collections::{HashMap, VecDeque};
use std::future::{Future, poll_fn};
use std::task::Poll;
use std::time::{Duration, Instant};

use crate::channel::{AsyncReceiver, TryRecvError};
use crate::observability::{ObservabilityState, Operation, StorageWorkerId};
use crate::*;
use futures_util::stream::{FuturesUnordered, StreamExt};
use openkache_protocol::ItemId;

use super::CoreTask;
use super::completion::CompletionSender;
pub(super) use super::keyed_compatibility::{CollapsedLaneBatch, KeyedCommand};
use super::keyed_compatibility::{
    CompletedJob, KeyedFinish, PreparedJob, PreparedKeyedCommand, VisibleState, finish_keyed,
    pending_response, prepare_collapsed_batch,
};
use super::worker_control::{execute_storage_task, process_worker_barrier};

pub(super) async fn run_core_tasks(receiver: AsyncReceiver<CoreTask>) {
    while let Ok(task) = receiver.recv_async_storage().await {
        match task {
            CoreTask::Run(task) => task(),
            CoreTask::Shutdown => break,
        }
    }
}

pub(super) type WorkerResponseSender = CompletionSender<Result<WorkerResponse>>;

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

pub(super) enum WorkerRequest {
    /// Keyed data-plane work routed through the per-key scheduler.
    ///
    /// The envelope keeps routing and completion generic at the worker
    /// boundary. API-owned adapters retain their optimized command
    /// implementations behind the keyed-work descriptor.
    Keyed {
        storage_key: StorageKey,
        command: KeyedCommand,
    },
    /// Control-plane work is kept separate from keyed data-plane commands.
    ///
    /// Stats, sync, extension tasks, and shutdown all require a quiescent
    /// worker but do not participate in per-key scheduling or collapse.
    Control(WorkerControlRequest),
}

pub(super) enum WorkerControlRequest {
    Stats {
        response: WorkerResponseSender,
    },
    Sync {
        response: WorkerResponseSender,
    },
    /// Executes an API-owned storage task after all keyed work is quiescent.
    ///
    /// GET/SET/DELETE keep their keyed scheduler and collapse optimizations.
    /// Extensions use this escape hatch for batch, CAS, or other storage
    /// shapes without adding another cache-specific worker enum variant.
    StorageTask {
        task: super::StorageTask,
        response: WorkerResponseSender,
    },
    Shutdown,
}

pub(super) enum WorkerResponse {
    /// API-owned keyed result. The worker only transports this opaque
    /// projection and never selects a response shape by operation name.
    Keyed(super::keyed_compatibility::KeyedResponse),
    Stats(String),
    Synced,
    StorageResult(super::StorageTaskOutput),
    StorageFailure(super::StorageError),
}

impl std::fmt::Debug for WorkerResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keyed(_) => formatter.write_str("Keyed(..)"),
            Self::Stats(stats) => formatter.debug_tuple("Stats").field(stats).finish(),
            Self::Synced => formatter.write_str("Synced"),
            // API-owned task results are intentionally erased at the runtime
            // boundary and are therefore only identified, never formatted.
            Self::StorageResult(_) => formatter.write_str("StorageResult(..)"),
            Self::StorageFailure(error) => formatter
                .debug_tuple("StorageFailure")
                .field(error)
                .finish(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum BenchmarkResponseKind {
    Get,
    Set,
    Delete,
}

pub(super) struct DeferredWorkerResponse {
    pub(super) sender: WorkerResponseSender,
    pub(super) value: WorkerResponse,
}

struct SchedulerCompletion {
    immediate: Vec<DeferredWorkerResponse>,
    collapsed: Option<CollapsedLaneBatch>,
}

impl SchedulerCompletion {
    fn empty() -> Self {
        Self {
            immediate: Vec::new(),
            collapsed: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaneState {
    Ready,
    Running {
        collapse_group: &'static super::keyed_compatibility::CollapseGroup,
    },
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

    fn has_waiting(&self, storage_key: &StorageKey) -> bool {
        self.lanes
            .get(storage_key)
            .is_some_and(|lane| lane.waiting_head.is_some())
    }

    fn ready_is_exclusive(&self) -> bool {
        let Some(storage_key) = self.ready.front() else {
            return false;
        };
        let lane = self.lanes.get(storage_key).expect("ready key has a lane");
        let head = lane.waiting_head.expect("ready lane has a waiting command");
        self.waiting.get(head).is_exclusive()
    }

    fn take_ready(&mut self) -> Option<(StorageKey, KeyedCommand)> {
        let storage_key = self.ready.pop_front()?;
        let head = self
            .lanes
            .get(&storage_key)
            .expect("ready key has a lane")
            .waiting_head
            .expect("ready lane has a waiting command");
        let collapse_group = self.waiting.get(head).descriptor().collapse_group;
        let (command, next) = self.waiting.take(head);
        let lane = self
            .lanes
            .get_mut(&storage_key)
            .expect("ready key has a lane");
        debug_assert_eq!(lane.state, LaneState::Ready);
        lane.waiting_head = next;
        if next.is_none() {
            lane.waiting_tail = None;
        }
        lane.state = LaneState::Running { collapse_group };
        Some((storage_key, command))
    }

    fn take_ready_exclusive(
        &mut self,
    ) -> Option<(StorageKey, super::StorageTask, WorkerResponseSender)> {
        if !self.ready_is_exclusive() {
            return None;
        }
        let (storage_key, command) = self.take_ready()?;
        command
            .take_exclusive()
            .map(|(task, response)| (storage_key, task, response))
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
        debug_assert!(!self.ready_is_exclusive());
        let (storage_key, command) = self.take_ready()?;
        let metadata = command.metadata(cache);
        let PreparedKeyedCommand { response, job } = command.prepare(cache, storage_key);
        Some(RunningKeyedCommand {
            storage_key,
            completion: RunningCompletion::Direct(response),
            operation: metadata.operation,
            started_at: Instant::now(),
            job,
        })
    }

    fn complete(
        &mut self,
        cache: &Kvkache,
        storage_key: StorageKey,
        visible_state: Option<VisibleState>,
    ) -> SchedulerCompletion {
        if let Some(base) = visible_state {
            let mut commands = Vec::new();
            let collapse_group = match self.lanes.get(&storage_key) {
                Some(KeyLane {
                    state: LaneState::Running { collapse_group },
                    waiting_head: Some(head),
                    ..
                }) => {
                    let command = self.waiting.get(*head);
                    command
                        .belongs_to_collapse_group(*collapse_group)
                        .then_some((*collapse_group, command.descriptor().collapse))
                }
                Some(KeyLane {
                    state: LaneState::Running { .. },
                    waiting_head: None,
                    ..
                })
                | Some(KeyLane {
                    state: LaneState::Ready,
                    ..
                })
                | None => None,
            };
            if let Some((collapse_group, collapse)) = collapse_group {
                loop {
                    let head = self
                        .lanes
                        .get(&storage_key)
                        .expect("completed key has a lane")
                        .waiting_head;
                    let Some(head) = head else {
                        break;
                    };
                    let command = self.waiting.get(head);
                    if !command.belongs_to_collapse_group(collapse_group)
                        || !command.is_collapsible(cache)
                    {
                        break;
                    }
                    let (command, next) = self.waiting.take(head);
                    let lane = self
                        .lanes
                        .get_mut(&storage_key)
                        .expect("completed key has a lane");
                    lane.waiting_head = next;
                    if next.is_none() {
                        lane.waiting_tail = None;
                    }
                    commands.push(command);
                }
                if !commands.is_empty() {
                    let batch = collapse(base, commands);
                    if !batch.has_mutation() {
                        let immediate = batch.responses;
                        self.finish_running_lane(storage_key);
                        return SchedulerCompletion {
                            immediate,
                            collapsed: None,
                        };
                    }
                    return SchedulerCompletion {
                        immediate: Vec::new(),
                        collapsed: Some(batch),
                    };
                }
            }
        }

        self.finish_running_lane(storage_key);
        SchedulerCompletion::empty()
    }

    fn finish_running_lane(&mut self, storage_key: StorageKey) {
        let ready_again = {
            let lane = self
                .lanes
                .get_mut(&storage_key)
                .expect("completed key has a lane");
            debug_assert!(matches!(lane.state, LaneState::Running { .. }));
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
    completion: RunningCompletion,
    job: PreparedJob,
    operation: Operation,
    started_at: Instant,
}

enum RunningCompletion {
    Direct(WorkerResponseSender),
    Collapsed {
        responses: Vec<DeferredWorkerResponse>,
        mutation_response_index: Option<usize>,
        success_state: VisibleState,
        failure_state: VisibleState,
    },
}

struct CompletedKeyedCommand {
    storage_key: StorageKey,
    completion: RunningCompletion,
    job: CompletedJob,
    operation: Operation,
    started_at: Instant,
}

async fn run_keyed_command(running: RunningKeyedCommand) -> CompletedKeyedCommand {
    let RunningKeyedCommand {
        storage_key,
        completion,
        job,
        operation,
        started_at,
    } = running;
    CompletedKeyedCommand {
        storage_key,
        completion,
        job: job.run().await,
        operation,
        started_at,
    }
}

enum WorkerEvent {
    Background(Result<bool>),
    Completed(CompletedKeyedCommand),
    Request(Option<WorkerRequest>),
}

enum DeferredLaneCompletion {
    Batch {
        storage_key: StorageKey,
        responses: Vec<DeferredWorkerResponse>,
        success_state: Option<VisibleState>,
        failure_state: Option<VisibleState>,
    },
    Pending {
        storage_key: StorageKey,
        response: WorkerResponseSender,
    },
    CollapsedPending {
        storage_key: StorageKey,
        responses: Vec<DeferredWorkerResponse>,
        mutation_response_index: usize,
        failure_state: VisibleState,
    },
}

fn send_success(responses: Vec<DeferredWorkerResponse>) {
    for response in responses {
        let _ = response.sender.send(Ok(response.value));
    }
}

fn send_pending_success(
    mut responses: Vec<DeferredWorkerResponse>,
    mutation_response_index: usize,
    outcome: WorkerResponse,
) {
    let response = responses
        .get_mut(mutation_response_index)
        .expect("collapsed mutation response index is in bounds");
    response.value = outcome;
    send_success(responses);
}

fn send_failure(responses: Vec<DeferredWorkerResponse>, message: &str) {
    for response in responses {
        let _ = response
            .sender
            .send(Err(KvError::Worker(message.to_string())));
    }
}

fn finish_scheduler_lane(
    cache: &mut Kvkache,
    scheduler: &mut KeyScheduler,
    storage_key: StorageKey,
    visible_state: Option<VisibleState>,
) -> Option<RunningKeyedCommand> {
    let completion = scheduler.complete(cache, storage_key, visible_state);
    send_success(completion.immediate);
    let batch = completion.collapsed?;
    let prepared = prepare_collapsed_batch(cache, storage_key, batch);
    let super::keyed_compatibility::PreparedCollapsed {
        operation: telemetry_operation,
        job,
        responses,
        mutation_response_index,
        success_state,
        failure_state,
    } = prepared;
    Some(RunningKeyedCommand {
        storage_key,
        completion: RunningCompletion::Collapsed {
            responses,
            mutation_response_index,
            success_state,
            failure_state,
        },
        operation: telemetry_operation,
        started_at: Instant::now(),
        job,
    })
}

pub(super) async fn worker_loop(
    mut cache: Kvkache,
    receiver: AsyncReceiver<WorkerRequest>,
    io_config: IoUringConfig,
    worker_id: usize,
    affinity_id: usize,
    observability: Option<std::sync::Arc<ObservabilityState>>,
) -> Result<()> {
    let storage_shard = observability
        .as_deref()
        .map(|state| state.storage_shard(StorageWorkerId(worker_id)));
    let waiting_capacity = io_config
        .batch_size
        .saturating_mul(io_config.max_inflight_per_worker)
        .max(io_config.max_inflight_per_worker);
    let mut scheduler = KeyScheduler::with_waiting_capacity(waiting_capacity);
    let mut inflight = FuturesUnordered::new();
    let mut barrier = None;
    let mut disconnected = false;
    let mut deferred_completions: Vec<DeferredLaneCompletion> = Vec::new();

    loop {
        if !deferred_completions.is_empty() {
            match cache.progress_capacity() {
                Ok((capacity_ready, completed)) => {
                    for result in completed {
                        let index = deferred_completions
                            .iter()
                            .position(|completion| match completion {
                                DeferredLaneCompletion::Pending { storage_key, .. }
                                | DeferredLaneCompletion::CollapsedPending {
                                    storage_key, ..
                                } => *storage_key == result.storage_key,
                                DeferredLaneCompletion::Batch { .. } => false,
                            })
                            .expect("completed capacity mutation has a deferred response");
                        let completion = deferred_completions.swap_remove(index);
                        match completion {
                            DeferredLaneCompletion::Pending {
                                storage_key,
                                response,
                            } => {
                                let _ = response.send(Ok(pending_response(result.outcome)));
                                if let Some(running) = finish_scheduler_lane(
                                    &mut cache,
                                    &mut scheduler,
                                    storage_key,
                                    result.visible_state,
                                ) {
                                    inflight.push(run_keyed_command(running));
                                }
                            }
                            DeferredLaneCompletion::CollapsedPending {
                                storage_key,
                                responses,
                                mutation_response_index,
                                ..
                            } => {
                                send_pending_success(
                                    responses,
                                    mutation_response_index,
                                    pending_response(result.outcome),
                                );
                                if let Some(running) = finish_scheduler_lane(
                                    &mut cache,
                                    &mut scheduler,
                                    storage_key,
                                    result.visible_state,
                                ) {
                                    inflight.push(run_keyed_command(running));
                                }
                            }
                            DeferredLaneCompletion::Batch { .. } => {
                                unreachable!("capacity completion matched a flush batch")
                            }
                        }
                    }
                    if capacity_ready {
                        for completion in deferred_completions.drain(..) {
                            match completion {
                                DeferredLaneCompletion::Batch {
                                    storage_key,
                                    responses,
                                    success_state,
                                    ..
                                } => {
                                    send_success(responses);
                                    if let Some(running) = finish_scheduler_lane(
                                        &mut cache,
                                        &mut scheduler,
                                        storage_key,
                                        success_state,
                                    ) {
                                        inflight.push(run_keyed_command(running));
                                    }
                                }
                                DeferredLaneCompletion::Pending { .. }
                                | DeferredLaneCompletion::CollapsedPending { .. } => {
                                    unreachable!(
                                        "capacity reported ready with an unresolved mutation"
                                    );
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    for completion in deferred_completions.drain(..) {
                        let (storage_key, failure_state) = match completion {
                            DeferredLaneCompletion::Batch {
                                storage_key,
                                responses,
                                failure_state,
                                ..
                            } => {
                                send_failure(responses, &message);
                                (storage_key, failure_state)
                            }
                            DeferredLaneCompletion::Pending {
                                storage_key,
                                response,
                            } => {
                                cache.cancel_pending_keyed_mutation(storage_key);
                                let _ = response.send(Err(KvError::Worker(message.clone())));
                                (storage_key, None)
                            }
                            DeferredLaneCompletion::CollapsedPending {
                                storage_key,
                                responses,
                                failure_state,
                                ..
                            } => {
                                cache.cancel_pending_keyed_mutation(storage_key);
                                send_failure(responses, &message);
                                (storage_key, Some(failure_state))
                            }
                        };
                        if let Some(running) = finish_scheduler_lane(
                            &mut cache,
                            &mut scheduler,
                            storage_key,
                            failure_state,
                        ) {
                            inflight.push(run_keyed_command(running));
                        }
                    }
                }
            }
        }

        // A task borrows the worker-local cache for its full future lifetime,
        // so it cannot overlap an owned keyed read/write job. It still lives
        // in the keyed lane and therefore preserves per-key ordering; only
        // this extension action is serialized at the worker boundary.
        if inflight.is_empty()
            && let Some((storage_key, task, response)) = scheduler.take_ready_exclusive()
        {
            let operation = Operation::unknown();
            let started_at = Instant::now();
            if task.metadata().cancellation()
                == super::StorageTaskCancellation::CancelIfDisconnected
                && response.is_disconnected()
            {
                scheduler.finish_running_lane(storage_key);
                continue;
            }
            let result = execute_storage_task(&mut cache, task).await;
            let _ = response.send(Ok(result));
            if let Some(storage_shard) = storage_shard {
                storage_shard.record_operation(operation, started_at.elapsed());
            }
            scheduler.finish_running_lane(storage_key);
            continue;
        }

        while inflight.len() < io_config.max_inflight_per_worker {
            if scheduler.ready_is_exclusive() {
                break;
            }
            let Some(running) = scheduler.start_ready(&mut cache) else {
                break;
            };
            inflight.push(run_keyed_command(running));
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

        let can_receive = barrier.is_none() && !disconnected && scheduler.has_waiting_capacity();
        let idle_before_event = inflight.is_empty() && scheduler.is_idle();
        let event = if can_receive {
            let mut incoming = std::pin::pin!(receiver.recv_async_storage());
            poll_fn(|context| {
                if cache.has_background_work()
                    && let Poll::Ready(result) = cache.poll_background(context)
                {
                    return Poll::Ready(WorkerEvent::Background(result));
                }
                if let Poll::Ready(Some(completed)) = inflight.poll_next_unpin(context) {
                    return Poll::Ready(WorkerEvent::Completed(completed));
                }
                match incoming.as_mut().poll(context) {
                    Poll::Ready(Ok(request)) => Poll::Ready(WorkerEvent::Request(Some(request))),
                    Poll::Ready(Err(_)) => Poll::Ready(WorkerEvent::Request(None)),
                    Poll::Pending => Poll::Pending,
                }
            })
            .await
        } else {
            poll_fn(|context| {
                if cache.has_background_work()
                    && let Poll::Ready(result) = cache.poll_background(context)
                {
                    return Poll::Ready(WorkerEvent::Background(result));
                }
                if inflight.is_empty() {
                    Poll::Pending
                } else {
                    inflight.poll_next_unpin(context).map(|completed| {
                        WorkerEvent::Completed(
                            completed.expect("in-flight keyed command disappeared"),
                        )
                    })
                }
            })
            .await
        };

        match event {
            WorkerEvent::Background(result) => match result {
                Ok(_) => {}
                Err(KvError::NoCapacity) if !deferred_completions.is_empty() => {
                    for completion in deferred_completions.drain(..) {
                        let (storage_key, failure_state) = match completion {
                            DeferredLaneCompletion::Batch {
                                storage_key,
                                responses,
                                failure_state,
                                ..
                            } => {
                                send_failure(
                                    responses,
                                    "write cannot be admitted without evicting protected items",
                                );
                                (storage_key, failure_state)
                            }
                            DeferredLaneCompletion::Pending {
                                storage_key,
                                response,
                            } => {
                                cache.cancel_pending_keyed_mutation(storage_key);
                                let _ = response.send(Err(KvError::NoCapacity));
                                (storage_key, None)
                            }
                            DeferredLaneCompletion::CollapsedPending {
                                storage_key,
                                responses,
                                failure_state,
                                ..
                            } => {
                                cache.cancel_pending_keyed_mutation(storage_key);
                                send_failure(
                                    responses,
                                    "write cannot be admitted without evicting protected items",
                                );
                                (storage_key, Some(failure_state))
                            }
                        };
                        if let Some(running) = finish_scheduler_lane(
                            &mut cache,
                            &mut scheduler,
                            storage_key,
                            failure_state,
                        ) {
                            inflight.push(run_keyed_command(running));
                        }
                    }
                }
                Err(error) => return Err(error),
            },
            WorkerEvent::Completed(completed) => {
                if let Some(storage_shard) = storage_shard {
                    storage_shard
                        .record_operation(completed.operation, completed.started_at.elapsed());
                }
                let include_visible_state = scheduler.has_waiting(&completed.storage_key);
                let KeyedFinish {
                    outcome,
                    visible_state,
                    flush_required,
                    pending,
                } = finish_keyed(&mut cache, completed.job, include_visible_state);
                let completion = match completed.completion {
                    RunningCompletion::Direct(response) => match outcome {
                        Ok(outcome) if !flush_required => {
                            let _ = response.send(Ok(outcome));
                            if let Some(running) = finish_scheduler_lane(
                                &mut cache,
                                &mut scheduler,
                                completed.storage_key,
                                visible_state,
                            ) {
                                inflight.push(run_keyed_command(running));
                            }
                            continue;
                        }
                        Ok(_) if pending => DeferredLaneCompletion::Pending {
                            storage_key: completed.storage_key,
                            response,
                        },
                        Ok(outcome) => DeferredLaneCompletion::Batch {
                            storage_key: completed.storage_key,
                            responses: vec![DeferredWorkerResponse {
                                sender: response,
                                value: outcome,
                            }],
                            success_state: visible_state,
                            failure_state: None,
                        },
                        Err(error) => {
                            let _ = response.send(Err(error));
                            if let Some(running) = finish_scheduler_lane(
                                &mut cache,
                                &mut scheduler,
                                completed.storage_key,
                                None,
                            ) {
                                inflight.push(run_keyed_command(running));
                            }
                            continue;
                        }
                    },
                    RunningCompletion::Collapsed {
                        responses,
                        mutation_response_index,
                        failure_state,
                        ..
                    } if pending => DeferredLaneCompletion::CollapsedPending {
                        storage_key: completed.storage_key,
                        responses,
                        mutation_response_index: mutation_response_index
                            .expect("collapsed pending mutation has a response"),
                        failure_state,
                    },
                    RunningCompletion::Collapsed {
                        responses,
                        mutation_response_index: _,
                        success_state,
                        failure_state,
                    } => match outcome {
                        Ok(_) => DeferredLaneCompletion::Batch {
                            storage_key: completed.storage_key,
                            responses,
                            success_state: Some(success_state),
                            failure_state: Some(failure_state),
                        },
                        Err(error) => {
                            send_failure(responses, &error.to_string());
                            if let Some(running) = finish_scheduler_lane(
                                &mut cache,
                                &mut scheduler,
                                completed.storage_key,
                                Some(failure_state),
                            ) {
                                inflight.push(run_keyed_command(running));
                            }
                            continue;
                        }
                    },
                };
                if flush_required {
                    deferred_completions.push(completion);
                } else {
                    let DeferredLaneCompletion::Batch {
                        storage_key,
                        responses,
                        success_state,
                        ..
                    } = completion
                    else {
                        unreachable!("a pending completion must require capacity work");
                    };
                    send_success(responses);
                    if let Some(running) = finish_scheduler_lane(
                        &mut cache,
                        &mut scheduler,
                        storage_key,
                        success_state,
                    ) {
                        inflight.push(run_keyed_command(running));
                    }
                }
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
                    match crate::storage_runtime::timeout(
                        Duration::from_micros(io_config.batch_max_wait_us),
                        receiver.recv_async_storage(),
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
        WorkerRequest::Keyed {
            storage_key,
            command,
        } => scheduler.enqueue(storage_key, command),
        request => {
            *barrier = Some(request);
            Ok(())
        }
    }
}
