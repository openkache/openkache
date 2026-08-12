//! Per-worker request/response types, the rolling keyed event loop
//! ([`worker_loop`]) and per-worker scheduler support types.

use std::collections::{HashMap, VecDeque};
use std::future::{Future, poll_fn};
use std::task::Poll;
use std::time::{Duration, Instant};

use crate::channel::{AsyncReceiver, TryRecvError};
use crate::observability::{ObservabilityState, Operation, StorageWorkerId};
use crate::protocol::SetOptions;
use crate::types::StoredItemValue;
use crate::*;
use futures_util::stream::{FuturesUnordered, StreamExt};
use openkache_protocol::Opcode;

use super::CoreTask;
use super::completion::CompletionSender;
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

pub(super) enum WorkerRequest {
    /// Keyed data-plane work routed through the per-key scheduler.
    ///
    /// The envelope keeps routing and completion generic at the worker
    /// boundary. Core actions currently retain their optimized command
    /// implementations inside [`StorageCommand`].
    Keyed {
        storage_key: StorageKey,
        command: StorageCommand,
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
    Value(Option<StoredItemValue>),
    Set(SetOutcome),
    Deleted(bool),
    Stats(String),
    Synced,
    #[allow(dead_code)]
    StorageResult(super::StorageTaskOutput),
    #[allow(dead_code)]
    StorageFailure(super::StorageError),
}

impl std::fmt::Debug for WorkerResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Value(value) => formatter.debug_tuple("Value").field(value).finish(),
            Self::Set(outcome) => formatter.debug_tuple("Set").field(outcome).finish(),
            Self::Deleted(deleted) => formatter.debug_tuple("Deleted").field(deleted).finish(),
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

    fn get(&self, id: SlotId) -> &T {
        let slot = self
            .slots
            .get(id.index as usize)
            .expect("waiting SlotId index is valid");
        assert_eq!(
            slot.generation, id.generation,
            "waiting SlotId generation is current"
        );
        slot.value
            .as_ref()
            .expect("waiting slot contains a command")
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

pub(super) enum StorageCommand {
    /// API-owned keyed work. It is ordered by the same key lane as the
    /// core actions but is deliberately non-collapsible.
    Custom {
        task: super::StorageTask,
        response: WorkerResponseSender,
    },
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

/// Runtime-facing action metadata.
///
/// The scheduler consumes this small descriptor instead of inferring
/// scheduling policy from a protocol opcode or an API-specific result type.
/// Core actions provide descriptors here for compatibility; new APIs can submit
/// a `StorageTask` with the same metadata surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StorageActionMetadata {
    operation: Operation,
    collapsible: bool,
}

/// Compatibility alias for private scheduler tests and migration code.
///
/// New runtime code should use [`StorageCommand`]; keeping this alias avoids
/// coupling the generic envelope migration to test fixture naming.
#[allow(dead_code)]
pub(super) type KeyedCommand = StorageCommand;

impl StorageCommand {
    fn metadata(&self, cache: &Kvkache) -> StorageActionMetadata {
        match self {
            Self::Custom { .. } => StorageActionMetadata {
                operation: Operation::unknown(),
                collapsible: false,
            },
            Self::Get { .. } => StorageActionMetadata {
                operation: Operation::from_opcode(Opcode::Get),
                collapsible: true,
            },
            Self::Set { value, options, .. } => StorageActionMetadata {
                operation: Operation::from_opcode(Opcode::Set),
                collapsible: *options == SetOptions::NONE && cache.can_collapse_set(value),
            },
            Self::Delete { .. } => StorageActionMetadata {
                operation: Operation::from_opcode(Opcode::Delete),
                collapsible: true,
            },
        }
    }

    fn into_parts(self) -> (KeyedOperation, WorkerResponseSender) {
        match self {
            Self::Custom { .. } => {
                unreachable!("custom storage tasks are executed through the task path")
            }
            Self::Get { response } => (KeyedOperation::Get, response),
            Self::Set {
                value,
                options,
                response,
            } => (KeyedOperation::Set { value, options }, response),
            Self::Delete { response } => (KeyedOperation::Delete, response),
        }
    }

    fn is_collapsible(&self, cache: &Kvkache) -> bool {
        self.metadata(cache).collapsible
    }
}

pub(super) struct DeferredWorkerResponse {
    pub(super) sender: WorkerResponseSender,
    pub(super) value: WorkerResponse,
}

pub(super) struct CollapsedLaneBatch {
    pub(super) operation: Option<KeyedOperation>,
    pub(super) responses: Vec<DeferredWorkerResponse>,
    pub(super) mutation_response_index: Option<usize>,
    pub(super) success_state: KeyedVisibleState,
    pub(super) failure_state: KeyedVisibleState,
}

impl CollapsedLaneBatch {
    pub(super) fn reduce(base: KeyedVisibleState, commands: Vec<StorageCommand>) -> Self {
        let base_present = matches!(base, KeyedVisibleState::Present(_));
        let mut current = base.clone();
        let mut responses = Vec::with_capacity(commands.len());
        let mut mutation_response_index = None;
        let mut mutated = false;

        for command in commands {
            let response_index = responses.len();
            let (sender, value) = match command {
                StorageCommand::Custom { .. } => {
                    unreachable!("custom storage tasks are never included in collapse batches")
                }
                StorageCommand::Get { response } => {
                    let value = match &current {
                        KeyedVisibleState::Missing => None,
                        KeyedVisibleState::Present(value) => Some(value.clone()),
                    };
                    (response, WorkerResponse::Value(value))
                }
                StorageCommand::Set {
                    value,
                    options,
                    response,
                } => {
                    debug_assert_eq!(options, SetOptions::NONE);
                    let outcome = match current {
                        KeyedVisibleState::Missing => SetOutcome::Created,
                        KeyedVisibleState::Present(_) => SetOutcome::Replaced,
                    };
                    current = KeyedVisibleState::Present(value);
                    mutated = true;
                    mutation_response_index = Some(response_index);
                    (response, WorkerResponse::Set(outcome))
                }
                StorageCommand::Delete { response } => {
                    let deleted = matches!(current, KeyedVisibleState::Present(_));
                    current = KeyedVisibleState::Missing;
                    mutated = true;
                    mutation_response_index = Some(response_index);
                    (response, WorkerResponse::Deleted(deleted))
                }
            };
            responses.push(DeferredWorkerResponse { sender, value });
        }

        let operation = if mutated {
            match &current {
                KeyedVisibleState::Present(value) => Some(KeyedOperation::Set {
                    value: value.clone(),
                    options: SetOptions::NONE,
                }),
                KeyedVisibleState::Missing if base_present => Some(KeyedOperation::Delete),
                KeyedVisibleState::Missing => None,
            }
        } else {
            None
        };
        let mutation_response_index = operation
            .as_ref()
            .map(|_| mutation_response_index.expect("collapsed mutation has a response"));

        Self {
            operation,
            responses,
            mutation_response_index,
            success_state: current,
            failure_state: base,
        }
    }
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
    waiting: WaitingSlab<StorageCommand>,
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

    fn ready_is_custom(&self) -> bool {
        let Some(storage_key) = self.ready.front() else {
            return false;
        };
        let lane = self.lanes.get(storage_key).expect("ready key has a lane");
        let head = lane.waiting_head.expect("ready lane has a waiting command");
        matches!(self.waiting.get(head), StorageCommand::Custom { .. })
    }

    fn take_ready(&mut self) -> Option<(StorageKey, StorageCommand)> {
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
        Some((storage_key, command))
    }

    fn take_ready_custom(
        &mut self,
    ) -> Option<(StorageKey, super::StorageTask, WorkerResponseSender)> {
        if !self.ready_is_custom() {
            return None;
        }
        let (storage_key, command) = self.take_ready()?;
        match command {
            StorageCommand::Custom { task, response } => Some((storage_key, task, response)),
            _ => unreachable!("ready custom command changed while taking scheduler head"),
        }
    }

    fn enqueue(&mut self, storage_key: StorageKey, command: StorageCommand) -> Result<()> {
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
        debug_assert!(!self.ready_is_custom());
        let (storage_key, command) = self.take_ready()?;
        let metadata = command.metadata(cache);
        let (operation, response) = command.into_parts();
        Some(RunningKeyedCommand {
            storage_key,
            completion: RunningCompletion::Direct(response),
            operation: metadata.operation,
            started_at: Instant::now(),
            job: cache.prepare_keyed(storage_key, operation),
        })
    }

    fn complete(
        &mut self,
        cache: &Kvkache,
        storage_key: StorageKey,
        visible_state: Option<KeyedVisibleState>,
    ) -> SchedulerCompletion {
        if let Some(base) = visible_state {
            let mut commands = Vec::new();
            loop {
                let head = self
                    .lanes
                    .get(&storage_key)
                    .expect("completed key has a lane")
                    .waiting_head;
                let Some(head) = head else {
                    break;
                };
                if !self.waiting.get(head).is_collapsible(cache) {
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
                let batch = CollapsedLaneBatch::reduce(base, commands);
                if batch.operation.is_some() {
                    return SchedulerCompletion {
                        immediate: Vec::new(),
                        collapsed: Some(batch),
                    };
                }
                let immediate = batch.responses;
                self.finish_running_lane(storage_key);
                return SchedulerCompletion {
                    immediate,
                    collapsed: None,
                };
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
    completion: RunningCompletion,
    job: KeyedJob,
    operation: Operation,
    started_at: Instant,
}

enum RunningCompletion {
    Direct(WorkerResponseSender),
    Collapsed {
        responses: Vec<DeferredWorkerResponse>,
        mutation_response_index: Option<usize>,
        success_state: KeyedVisibleState,
        failure_state: KeyedVisibleState,
    },
}

struct CompletedKeyedCommand {
    storage_key: StorageKey,
    completion: RunningCompletion,
    job: CompletedKeyedJob,
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
        success_state: Option<KeyedVisibleState>,
        failure_state: Option<KeyedVisibleState>,
    },
    Pending {
        storage_key: StorageKey,
        response: WorkerResponseSender,
    },
    CollapsedPending {
        storage_key: StorageKey,
        responses: Vec<DeferredWorkerResponse>,
        mutation_response_index: usize,
        failure_state: KeyedVisibleState,
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
    outcome: KeyedOutcome,
) {
    let response = responses
        .get_mut(mutation_response_index)
        .expect("collapsed mutation response index is in bounds");
    response.value = outcome_response(outcome);
    send_success(responses);
}

fn send_failure(responses: Vec<DeferredWorkerResponse>, message: &str) {
    for response in responses {
        let _ = response
            .sender
            .send(Err(KvError::Worker(message.to_string())));
    }
}

fn telemetry_operation(operation: &KeyedOperation) -> Operation {
    match operation {
        KeyedOperation::Get => Operation::from_opcode(Opcode::Get),
        KeyedOperation::Set { .. } => Operation::from_opcode(Opcode::Set),
        KeyedOperation::Delete => Operation::from_opcode(Opcode::Delete),
    }
}

fn finish_scheduler_lane(
    cache: &mut Kvkache,
    scheduler: &mut KeyScheduler,
    storage_key: StorageKey,
    visible_state: Option<KeyedVisibleState>,
) -> Option<RunningKeyedCommand> {
    let completion = scheduler.complete(cache, storage_key, visible_state);
    send_success(completion.immediate);
    completion.collapsed.map(|batch| {
        let CollapsedLaneBatch {
            operation,
            responses,
            mutation_response_index,
            success_state,
            failure_state,
        } = batch;
        let operation = operation.expect("collapsed storage batch has a final mutation");
        let telemetry_operation = telemetry_operation(&operation);
        RunningKeyedCommand {
            storage_key,
            completion: RunningCompletion::Collapsed {
                responses,
                mutation_response_index,
                success_state,
                failure_state,
            },
            operation: telemetry_operation,
            started_at: Instant::now(),
            job: cache.prepare_keyed(storage_key, operation),
        }
    })
}

fn outcome_response(outcome: KeyedOutcome) -> WorkerResponse {
    match outcome {
        KeyedOutcome::Value(value) => WorkerResponse::Value(value),
        KeyedOutcome::Set(outcome) => WorkerResponse::Set(outcome),
        KeyedOutcome::Deleted(deleted) => WorkerResponse::Deleted(deleted),
    }
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
                                let _ = response.send(Ok(outcome_response(result.outcome)));
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
                                    result.outcome,
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
            && let Some((storage_key, task, response)) = scheduler.take_ready_custom()
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
            if scheduler.ready_is_custom() {
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
                } = cache.finish_keyed(completed.job, include_visible_state);
                let completion = match completed.completion {
                    RunningCompletion::Direct(response) => match outcome {
                        Ok(outcome) if !flush_required => {
                            let _ = response.send(Ok(outcome_response(outcome)));
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
                                value: outcome_response(outcome),
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
