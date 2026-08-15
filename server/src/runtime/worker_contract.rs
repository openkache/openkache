//! Operation-neutral worker request, response, and completion envelopes.

use std::future::Future;

use super::completion::CompletionSender;
use super::scheduler::ScheduledTask;
use crate::Result;
use crate::observability::Operation;

/// Work routed through either the keyed scheduler or quiescent control path.
pub(super) enum Request<K, C, X> {
    /// Keyed data-plane work routed through the per-key scheduler.
    Keyed { storage_key: K, command: C },
    /// Control work that requires a quiescent worker.
    Control(X),
}

/// Worker result envelope with an API-owned data-plane projection.
pub(super) enum Response<D> {
    Data(D),
    Stats(String),
    Synced,
}

impl<D> std::fmt::Debug for Response<D> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Data(_) => formatter.write_str("Data(..)"),
            Self::Stats(stats) => formatter.debug_tuple("Stats").field(stats).finish(),
            Self::Synced => formatter.write_str("Synced"),
        }
    }
}

pub(super) type ResponseSender<R> = CompletionSender<Result<R>>;

pub(super) struct DeferredResponse<R> {
    pub(super) sender: ResponseSender<R>,
    pub(super) value: R,
}

#[derive(Clone, Copy)]
pub(super) struct KeyedWorkMetadata {
    pub(super) operation: Operation,
    pub(super) collapsible: bool,
}

pub(super) struct PreparedKeyedWork<R, J> {
    pub(super) response: ResponseSender<R>,
    pub(super) job: J,
}

pub(super) enum CollapsedKeyedWork<R, J, S> {
    Complete(Vec<DeferredResponse<R>>),
    Prepared {
        operation: Operation,
        job: J,
        responses: Vec<DeferredResponse<R>>,
        mutation_response_index: usize,
        success_state: S,
        failure_state: S,
    },
}

pub(super) struct FinishedKeyedWork<R, S> {
    pub(super) outcome: Result<R>,
    pub(super) visible_state: Option<S>,
    pub(super) flush_required: bool,
    pub(super) pending: bool,
}

pub(super) struct CapacityCompletion<K, R, S> {
    pub(super) storage_key: K,
    pub(super) outcome: R,
    pub(super) visible_state: Option<S>,
}

/// Static API adapter boundary used by the keyed worker loop.
///
/// Associated types preserve monomorphized jobs, responses, and visible state.
/// The capacity completion remains opaque until it is consumed, allowing an
/// adapter to reuse a storage backend's existing result vector without
/// remapping or reallocating it.
pub(super) trait KeyedWorkPort<L, K>: ScheduledTask + Sized {
    type Response;
    type PreparedJob;
    type CompletedJob;
    type VisibleState;
    type CapacityCompletion;

    fn metadata(&self, lifecycle: &L) -> KeyedWorkMetadata;
    fn prepare(
        self,
        lifecycle: &mut L,
        storage_key: K,
    ) -> PreparedKeyedWork<Self::Response, Self::PreparedJob>;
    fn run(job: Self::PreparedJob) -> impl Future<Output = Self::CompletedJob>;
    fn collapse(
        lifecycle: &mut L,
        storage_key: K,
        base: Self::VisibleState,
        commands: Vec<Self>,
    ) -> CollapsedKeyedWork<Self::Response, Self::PreparedJob, Self::VisibleState>;
    fn finish(
        lifecycle: &mut L,
        job: Self::CompletedJob,
        include_visible_state: bool,
    ) -> FinishedKeyedWork<Self::Response, Self::VisibleState>;
    fn progress_capacity(lifecycle: &mut L) -> Result<(bool, Vec<Self::CapacityCompletion>)>;
    fn capacity_completion(
        completion: Self::CapacityCompletion,
    ) -> CapacityCompletion<K, Self::Response, Self::VisibleState>;
    fn cancel_pending(lifecycle: &mut L, storage_key: K);
}
