//! Completion state and scheduler-lane draining for one storage worker.

use std::time::Instant;

use crate::KvError;
use crate::observability::Operation;

use super::super::retained_response::{ResponseBatch, ResponseReservation, RetainedResponseArena};
use super::super::scheduler::KeyScheduler;
use super::super::worker_contract::{
    CollapsedKeyedWork, DeferredResponse, KeyedWorkPort, ResponseSender,
};

pub(super) struct RunningKeyedCommand<K, J, R, S> {
    pub(super) storage_key: K,
    pub(super) completion: RunningCompletion<R, S>,
    pub(super) job: J,
    pub(super) operation: Operation,
    pub(super) started_at: Instant,
}

pub(super) enum RunningCompletion<R, S> {
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

pub(super) struct CompletedKeyedCommand<K, J, R, S> {
    pub(super) storage_key: K,
    pub(super) completion: RunningCompletion<R, S>,
    pub(super) job: J,
    pub(super) operation: Operation,
    pub(super) started_at: Instant,
}

pub(super) async fn run_keyed_command<L, K, C>(
    running: RunningKeyedCommand<K, C::PreparedJob, C::Response, C::VisibleState>,
) -> CompletedKeyedCommand<K, C::CompletedJob, C::Response, C::VisibleState>
where
    C: KeyedWorkPort<L, K>,
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

pub(super) enum DeferredLaneCompletion<K, R, S> {
    Batch {
        storage_key: K,
        responses: ResponseBatch,
        success_state: Option<S>,
        failure_state: Option<S>,
    },
    Pending {
        storage_key: K,
        response: ResponseSender<R>,
        reservation: ResponseReservation,
    },
    CollapsedPending {
        storage_key: K,
        responses: ResponseBatch,
        mutation_response_index: usize,
        failure_state: S,
    },
}

pub(super) fn send_success<R>(
    arena: &mut RetainedResponseArena<DeferredResponse<R>>,
    responses: ResponseBatch,
) {
    for response in arena.drain(responses) {
        let _ = response.sender.send(Ok(response.value));
    }
}

pub(super) fn send_pending_success<R>(
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

pub(super) fn send_failure<R>(
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

pub(super) fn finish_scheduler_lane<L, K, C>(
    lifecycle: &mut L,
    scheduler: &mut KeyScheduler<K, C>,
    arena: &mut RetainedResponseArena<DeferredResponse<C::Response>>,
    storage_key: K,
    visible_state: Option<C::VisibleState>,
) -> Option<RunningKeyedCommand<K, C::PreparedJob, C::Response, C::VisibleState>>
where
    K: Eq + std::hash::Hash + Clone,
    C: KeyedWorkPort<L, K>,
{
    let Some(base) = visible_state else {
        scheduler.finish_running_lane(storage_key);
        return None;
    };
    let commands =
        scheduler.drain_collapsible_up_to(storage_key.clone(), arena.available(), |command| {
            command.metadata(lifecycle).collapsible
        });
    if commands.len() == 0 {
        drop(commands);
        scheduler.finish_running_lane(storage_key);
        return None;
    }
    let command_count = commands.len();
    let mut responses = arena.batch();
    let work = C::collapse(lifecycle, storage_key.clone(), base, commands, |response| {
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
