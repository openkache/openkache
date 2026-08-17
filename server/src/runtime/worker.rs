//! Per-worker request/response types, the rolling keyed event loop
//! ([`worker_loop`]) and per-worker scheduler support types.

use std::future::{Future, poll_fn};
use std::task::Poll;
use std::time::{Duration, Instant};

use crate::channel::{AsyncReceiver, TryRecvError};
use crate::observability::{ObservabilityState, StorageWorkerId};
use crate::*;
use futures_util::stream::{FuturesUnordered, StreamExt};

use super::CoreTask;
use super::retained_response::RetainedResponseArena;
use super::scheduler::{KeyScheduler, ScheduledTask};
use super::worker_contract::{
    DeferredResponse, FinishedKeyedWork, KeyedWorkPort, PreparedKeyedWork, Request,
};
use super::worker_lifecycle::WorkerLifecyclePort;

mod completion;
use self::completion::{
    DeferredLaneCompletion, RunningCompletion, RunningKeyedCommand, finish_scheduler_lane,
    run_keyed_command, send_failure, send_pending_success, send_success,
};

const MAX_RETAINED_RESPONSE_METADATA_BYTES_PER_WORKER: usize = 64 * 1024 * 1024;

pub(super) fn validate_worker_metadata<C>(
    io_config: &IoUringConfig,
    stable_owner_pool_metadata_bytes: usize,
) -> Result<()>
where
    C: KeyedWorkPort<Kvkache, StorageKey>,
{
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
        .and_then(|bytes| bytes.checked_add(stable_owner_pool_metadata_bytes))
        .ok_or_else(|| KvError::InvalidConfig("worker response metadata exceeds usize".into()))?;
    if retained_metadata_bytes > MAX_RETAINED_RESPONSE_METADATA_BYTES_PER_WORKER {
        return Err(KvError::InvalidConfig(format!(
            "response metadata requires {retained_metadata_bytes} bytes per worker"
        )));
    }
    Ok(())
}

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

enum WorkerEvent<T, Q> {
    Background(Result<bool>),
    Completed(T),
    Request(Option<Q>),
}

pub(super) async fn worker_loop<C, X>(
    mut cache: Kvkache,
    receiver: AsyncReceiver<Request<StorageKey, C, X>>,
    io_config: IoUringConfig,
    stable_owner_pool_metadata_bytes: usize,
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
    validate_worker_metadata::<C>(&io_config, stable_owner_pool_metadata_bytes)?;
    let mut scheduler: KeyScheduler<StorageKey, C> =
        KeyScheduler::with_waiting_capacity(waiting_capacity);
    let mut retained_responses = RetainedResponseArena::new(retained_response_capacity);
    debug_assert_eq!(retained_responses.capacity(), retained_response_capacity);
    let mut inflight = FuturesUnordered::new();
    let mut barrier = None;
    let mut disconnected = false;
    let mut deferred_completions: Vec<DeferredLaneCompletion<C::Response, C::VisibleState>> =
        Vec::with_capacity(io_config.max_inflight_per_worker);
    let mut deferred_results: Vec<
        <C::Lifecycle as WorkerLifecyclePort<Kvkache, StorageKey>>::DeferredCompletion,
    > = Vec::with_capacity(io_config.max_inflight_per_worker);

    loop {
        if !deferred_completions.is_empty() {
            deferred_results.clear();
            match C::Lifecycle::progress_deferred(&mut cache, |result| {
                deferred_results.push(result);
            }) {
                Ok(deferred_ready) => {
                    for result in deferred_results.drain(..) {
                        let result = C::Lifecycle::project_deferred(result);
                        let index = deferred_completions
                            .iter()
                            .position(|completion| match completion {
                                DeferredLaneCompletion::Pending { storage_key, .. }
                                | DeferredLaneCompletion::CollapsedPending {
                                    storage_key, ..
                                } => *storage_key == result.key,
                                DeferredLaneCompletion::Batch { .. } => false,
                            })
                            .expect("completed deferred work has a retained response");
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
                                unreachable!("a deferred result matched a flush batch")
                            }
                        }
                    }
                    if deferred_ready {
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
                                        "lifecycle reported ready with unresolved deferred work"
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
                                C::Lifecycle::cancel_deferred(&mut cache, storage_key);
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
                                C::Lifecycle::cancel_deferred(&mut cache, storage_key);
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
                if C::Lifecycle::has_background_work(&cache)
                    && let Poll::Ready(result) =
                        C::Lifecycle::poll_background(&mut cache, context)
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
                if C::Lifecycle::has_background_work(&cache)
                    && let Poll::Ready(result) =
                        C::Lifecycle::poll_background(&mut cache, context)
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
                                C::Lifecycle::cancel_deferred(&mut cache, storage_key);
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
                                C::Lifecycle::cancel_deferred(&mut cache, storage_key);
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
                        unreachable!("a pending completion must require deferred work");
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
