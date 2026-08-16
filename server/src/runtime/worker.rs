//! Per-worker request/response types, the rolling keyed event loop
//! ([`worker_loop`]) and per-worker scheduler support types.

use std::future::{Future, poll_fn};
use std::task::Poll;
use std::time::{Duration, Instant};

use crate::channel::{AsyncReceiver, TryRecvError};
use crate::observability::{ObservabilityState, Operation, StorageWorkerId};
use crate::*;
use futures_util::stream::{FuturesUnordered, StreamExt};

use super::CoreTask;
use super::retained_response::{ResponseBatch, ResponseReservation, RetainedResponseArena};
use super::scheduler::{KeyScheduler, ScheduledTask};
use super::worker_contract::{
    CollapsedKeyedWork, DeferredResponse, FinishedKeyedWork, KeyedWorkPort, PreparedKeyedWork,
    Request, ResponseSender,
};

const MAX_RETAINED_RESPONSE_METADATA_BYTES_PER_WORKER: usize = 64 * 1024 * 1024;

pub(super) async fn run_core_tasks(receiver: AsyncReceiver<CoreTask>) {
    while let Ok(task) = receiver.recv_async_storage().await {
        match task {
            CoreTask::Run(task) => task(),
            CoreTask::Shutdown => break,
        }
    }
}

pub(super) trait ControlPort<C> {
    fn execute_control(
        &mut self,
        command: C,
        affinity_id: usize,
    ) -> impl Future<Output = Result<ControlFlow>> + '_;
}

pub(super) enum ControlFlow {
    Continue,
    Stop,
}

async fn execute_control_work<L, C>(
    lifecycle: &mut L,
    command: C,
    affinity_id: usize,
) -> Result<ControlFlow>
where
    L: ControlPort<C>,
{
    lifecycle.execute_control(command, affinity_id).await
}

struct RunningKeyedCommand<J, R, S> {
    storage_key: StorageKey,
    completion: RunningCompletion<R, S>,
    job: J,
    operation: Operation,
    started_at: Instant,
}

enum RunningCompletion<R, S> {
    Direct {
        response: ResponseSender<R>,
        reservation: ResponseReservation,
    },
    Collapsed {
        responses: ResponseBatch,
        mutation_response_index: usize,
        success_state: S,
        failure_state: S,
    },
}

struct CompletedKeyedCommand<J, R, S> {
    storage_key: StorageKey,
    completion: RunningCompletion<R, S>,
    job: J,
    operation: Operation,
    started_at: Instant,
}

async fn run_keyed_command<C>(
    running: RunningKeyedCommand<C::PreparedJob, C::Response, C::VisibleState>,
) -> CompletedKeyedCommand<C::CompletedJob, C::Response, C::VisibleState>
where
    C: KeyedWorkPort<Kvkache, StorageKey>,
{
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
        job: C::run(job).await,
        operation,
        started_at,
    }
}

enum WorkerEvent<T, Q> {
    Background(Result<bool>),
    Completed(T),
    Request(Option<Q>),
}

enum DeferredLaneCompletion<R, S> {
    Batch {
        storage_key: StorageKey,
        responses: ResponseBatch,
        success_state: Option<S>,
        failure_state: Option<S>,
    },
    Pending {
        storage_key: StorageKey,
        response: ResponseSender<R>,
        reservation: ResponseReservation,
    },
    CollapsedPending {
        storage_key: StorageKey,
        responses: ResponseBatch,
        mutation_response_index: usize,
        failure_state: S,
    },
}

fn send_success<R>(
    arena: &mut RetainedResponseArena<DeferredResponse<R>>,
    responses: ResponseBatch,
) {
    for response in arena.drain(responses) {
        let _ = response.sender.send(Ok(response.value));
    }
}

fn send_pending_success<R>(
    arena: &mut RetainedResponseArena<DeferredResponse<R>>,
    responses: ResponseBatch,
    mutation_response_index: usize,
    outcome: R,
) {
    let response = arena
        .get_mut(&responses, mutation_response_index)
        .expect("collapsed mutation response index is in bounds");
    response.value = outcome;
    send_success(arena, responses);
}

fn send_failure<R>(
    arena: &mut RetainedResponseArena<DeferredResponse<R>>,
    responses: ResponseBatch,
    message: &str,
) {
    for response in arena.drain(responses) {
        let _ = response
            .sender
            .send(Err(KvError::Worker(message.to_string())));
    }
}

fn finish_scheduler_lane<C>(
    cache: &mut Kvkache,
    scheduler: &mut KeyScheduler<StorageKey, C>,
    arena: &mut RetainedResponseArena<DeferredResponse<C::Response>>,
    storage_key: StorageKey,
    visible_state: Option<C::VisibleState>,
) -> Option<RunningKeyedCommand<C::PreparedJob, C::Response, C::VisibleState>>
where
    C: KeyedWorkPort<Kvkache, StorageKey>,
{
    let Some(base) = visible_state else {
        scheduler.finish_running_lane(storage_key);
        return None;
    };
    let commands = scheduler.drain_collapsible_up_to(storage_key, arena.available(), |command| {
        command.metadata(cache).collapsible
    });
    if commands.len() == 0 {
        drop(commands);
        scheduler.finish_running_lane(storage_key);
        return None;
    }
    let command_count = commands.len();
    let mut responses = arena.batch();
    let work = C::collapse(cache, storage_key, base, commands, |response| {
        responses
            .push(response)
            .unwrap_or_else(|_| unreachable!("bounded collapse exceeded response capacity"))
    });
    assert_eq!(
        responses.len(),
        command_count,
        "collapse must defer one response per command"
    );
    let responses = responses.commit();
    let (operation, job, mutation_response_index, success_state, failure_state) = match work {
        CollapsedKeyedWork::Complete => {
            send_success(arena, responses);
            scheduler.finish_running_lane(storage_key);
            return None;
        }
        CollapsedKeyedWork::Prepared {
            operation,
            job,
            mutation_response_index,
            success_state,
            failure_state,
        } => (
            operation,
            job,
            mutation_response_index,
            success_state,
            failure_state,
        ),
    };
    Some(RunningKeyedCommand {
        storage_key,
        completion: RunningCompletion::Collapsed {
            responses,
            mutation_response_index,
            success_state,
            failure_state,
        },
        operation,
        started_at: Instant::now(),
        job,
    })
}

pub(super) async fn worker_loop<C, X>(
    mut cache: Kvkache,
    receiver: AsyncReceiver<Request<StorageKey, C, X>>,
    io_config: IoUringConfig,
    worker_id: usize,
    affinity_id: usize,
    observability: Option<std::sync::Arc<ObservabilityState>>,
) -> Result<()>
where
    C: KeyedWorkPort<Kvkache, StorageKey> + Send + Unpin + 'static,
    X: Send + Unpin + 'static,
    Kvkache: ControlPort<X>,
{
    let storage_shard = observability
        .as_deref()
        .map(|state| state.storage_shard(StorageWorkerId(worker_id)));
    let waiting_capacity = io_config.waiting_capacity()?;
    let retained_response_capacity = io_config.retained_response_capacity()?;
    let retained_response_bytes =
        RetainedResponseArena::<DeferredResponse<C::Response>>::allocation_bytes(
            retained_response_capacity,
        )
        .ok_or_else(|| {
            KvError::InvalidConfig("io_uring retained-response arena size exceeds usize".into())
        })?;
    let retained_metadata_bytes = io_config
        .max_inflight_per_worker
        .checked_mul(std::mem::size_of::<
            DeferredLaneCompletion<C::Response, C::VisibleState>,
        >())
        .and_then(|bytes| bytes.checked_add(retained_response_bytes))
        .ok_or_else(|| {
            KvError::InvalidConfig("io_uring retained-response metadata exceeds usize".into())
        })?;
    if retained_metadata_bytes > MAX_RETAINED_RESPONSE_METADATA_BYTES_PER_WORKER {
        return Err(KvError::InvalidConfig(format!(
            "io_uring retained-response metadata requires {retained_metadata_bytes} bytes per worker"
        )));
    }
    let mut scheduler: KeyScheduler<StorageKey, C> =
        KeyScheduler::with_waiting_capacity(waiting_capacity);
    let mut retained_responses = RetainedResponseArena::new(retained_response_capacity);
    debug_assert_eq!(retained_responses.capacity(), retained_response_capacity);
    let mut inflight = FuturesUnordered::new();
    let mut barrier = None;
    let mut disconnected = false;
    let mut deferred_completions: Vec<DeferredLaneCompletion<C::Response, C::VisibleState>> =
        Vec::with_capacity(io_config.max_inflight_per_worker);

    loop {
        if !deferred_completions.is_empty() {
            match C::progress_capacity(&mut cache) {
                Ok((capacity_ready, completed)) => {
                    for result in completed {
                        let result = C::capacity_completion(result);
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
                                reservation,
                            } => {
                                retained_responses.release(reservation);
                                let _ = response.send(Ok(result.outcome));
                                if let Some(running) = finish_scheduler_lane(
                                    &mut cache,
                                    &mut scheduler,
                                    &mut retained_responses,
                                    storage_key,
                                    result.visible_state,
                                ) {
                                    inflight.push(run_keyed_command::<C>(running));
                                }
                            }
                            DeferredLaneCompletion::CollapsedPending {
                                storage_key,
                                responses,
                                mutation_response_index,
                                ..
                            } => {
                                send_pending_success(
                                    &mut retained_responses,
                                    responses,
                                    mutation_response_index,
                                    result.outcome,
                                );
                                if let Some(running) = finish_scheduler_lane(
                                    &mut cache,
                                    &mut scheduler,
                                    &mut retained_responses,
                                    storage_key,
                                    result.visible_state,
                                ) {
                                    inflight.push(run_keyed_command::<C>(running));
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
                                    send_success(&mut retained_responses, responses);
                                    if let Some(running) = finish_scheduler_lane(
                                        &mut cache,
                                        &mut scheduler,
                                        &mut retained_responses,
                                        storage_key,
                                        success_state,
                                    ) {
                                        inflight.push(run_keyed_command::<C>(running));
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
                                send_failure(&mut retained_responses, responses, &message);
                                (storage_key, failure_state)
                            }
                            DeferredLaneCompletion::Pending {
                                storage_key,
                                response,
                                reservation,
                            } => {
                                C::cancel_pending(&mut cache, storage_key);
                                retained_responses.release(reservation);
                                let _ = response.send(Err(KvError::Worker(message.clone())));
                                (storage_key, None)
                            }
                            DeferredLaneCompletion::CollapsedPending {
                                storage_key,
                                responses,
                                failure_state,
                                ..
                            } => {
                                C::cancel_pending(&mut cache, storage_key);
                                send_failure(&mut retained_responses, responses, &message);
                                (storage_key, Some(failure_state))
                            }
                        };
                        if let Some(running) = finish_scheduler_lane(
                            &mut cache,
                            &mut scheduler,
                            &mut retained_responses,
                            storage_key,
                            failure_state,
                        ) {
                            inflight.push(run_keyed_command::<C>(running));
                        }
                    }
                }
            }
        }

        while inflight.len() + deferred_completions.len() < io_config.max_inflight_per_worker {
            let Some(reservation) = retained_responses.reserve() else {
                break;
            };
            let Some((storage_key, command)) = scheduler.take_ready() else {
                retained_responses.release(reservation);
                break;
            };
            let metadata = command.metadata(&cache);
            let PreparedKeyedWork { response, job } = command.prepare(&mut cache, storage_key);
            let running = RunningKeyedCommand {
                storage_key,
                completion: RunningCompletion::Direct {
                    response,
                    reservation,
                },
                operation: metadata.operation,
                started_at: Instant::now(),
                job,
            };
            inflight.push(run_keyed_command::<C>(running));
        }

        if inflight.is_empty() && scheduler.is_idle() {
            if let Some(command) = barrier.take() {
                if matches!(
                    execute_control_work(&mut cache, command, affinity_id).await?,
                    ControlFlow::Stop
                ) {
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
                                    &mut retained_responses,
                                    responses,
                                    "write cannot be admitted without evicting protected items",
                                );
                                (storage_key, failure_state)
                            }
                            DeferredLaneCompletion::Pending {
                                storage_key,
                                response,
                                reservation,
                            } => {
                                C::cancel_pending(&mut cache, storage_key);
                                retained_responses.release(reservation);
                                let _ = response.send(Err(KvError::NoCapacity));
                                (storage_key, None)
                            }
                            DeferredLaneCompletion::CollapsedPending {
                                storage_key,
                                responses,
                                failure_state,
                                ..
                            } => {
                                C::cancel_pending(&mut cache, storage_key);
                                send_failure(
                                    &mut retained_responses,
                                    responses,
                                    "write cannot be admitted without evicting protected items",
                                );
                                (storage_key, Some(failure_state))
                            }
                        };
                        if let Some(running) = finish_scheduler_lane(
                            &mut cache,
                            &mut scheduler,
                            &mut retained_responses,
                            storage_key,
                            failure_state,
                        ) {
                            inflight.push(run_keyed_command::<C>(running));
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
                let FinishedKeyedWork {
                    outcome,
                    visible_state,
                    flush_required,
                    pending,
                } = C::finish(&mut cache, completed.job, include_visible_state);
                let completion = match completed.completion {
                    RunningCompletion::Direct {
                        response,
                        reservation,
                    } => match outcome {
                        Ok(outcome) if !flush_required => {
                            retained_responses.release(reservation);
                            let _ = response.send(Ok(outcome));
                            if let Some(running) = finish_scheduler_lane(
                                &mut cache,
                                &mut scheduler,
                                &mut retained_responses,
                                completed.storage_key,
                                visible_state,
                            ) {
                                inflight.push(run_keyed_command::<C>(running));
                            }
                            continue;
                        }
                        Ok(_) if pending => DeferredLaneCompletion::Pending {
                            storage_key: completed.storage_key,
                            response,
                            reservation,
                        },
                        Ok(outcome) => DeferredLaneCompletion::Batch {
                            storage_key: completed.storage_key,
                            responses: retained_responses.complete(
                                reservation,
                                DeferredResponse {
                                    sender: response,
                                    value: outcome,
                                },
                            ),
                            success_state: visible_state,
                            failure_state: None,
                        },
                        Err(error) => {
                            retained_responses.release(reservation);
                            let _ = response.send(Err(error));
                            if let Some(running) = finish_scheduler_lane(
                                &mut cache,
                                &mut scheduler,
                                &mut retained_responses,
                                completed.storage_key,
                                None,
                            ) {
                                inflight.push(run_keyed_command::<C>(running));
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
                        mutation_response_index,
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
                            send_failure(&mut retained_responses, responses, &error.to_string());
                            if let Some(running) = finish_scheduler_lane(
                                &mut cache,
                                &mut scheduler,
                                &mut retained_responses,
                                completed.storage_key,
                                Some(failure_state),
                            ) {
                                inflight.push(run_keyed_command::<C>(running));
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
                    send_success(&mut retained_responses, responses);
                    if let Some(running) = finish_scheduler_lane(
                        &mut cache,
                        &mut scheduler,
                        &mut retained_responses,
                        storage_key,
                        success_state,
                    ) {
                        inflight.push(run_keyed_command::<C>(running));
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

fn admit_worker_request<C, X>(
    scheduler: &mut KeyScheduler<StorageKey, C>,
    barrier: &mut Option<X>,
    request: Request<StorageKey, C, X>,
) -> Result<()>
where
    C: ScheduledTask,
{
    match request {
        Request::Keyed {
            storage_key,
            command,
        } => scheduler
            .enqueue(storage_key, command)
            .map_err(|error| match error {
                super::scheduler::SchedulerError::Full => {
                    KvError::Worker("waiting-command slab is full".into())
                }
            }),
        Request::Control(command) => {
            *barrier = Some(command);
            Ok(())
        }
    }
}
