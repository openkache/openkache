//! Operation-neutral worker request, response, and completion envelopes.

use std::future::Future;

use super::completion::CompletionSender;
use super::scheduler::ScheduledTask;
use super::worker_lifecycle::WorkerLifecyclePort;
use crate::Result;
use crate::observability::Operation;

/// Work routed through either the keyed scheduler or quiescent control path.
pub(super) enum Request<K, C, X> {
    /// Keyed data-plane work routed through the per-key scheduler.
    Keyed { storage_key: K, command: C },
    /// Control work that requires a quiescent worker.
    Control(X),
}

/// Worker result envelope with API-owned data and control projections.
pub(super) enum Response<D, C> {
    Data(D),
    Control(C),
}

impl<D, C: std::fmt::Debug> std::fmt::Debug for Response<D, C> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Data(_) => formatter.write_str("Data(..)"),
            Self::Control(control) => formatter.debug_tuple("Control").field(control).finish(),
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

pub(super) enum CollapsedKeyedWork<J, S> {
    Complete,
    Prepared {
        operation: Operation,
        job: J,
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

/// Static API adapter boundary used by the keyed worker loop.
///
/// Associated types preserve monomorphized jobs, responses, and visible state.
/// Lifecycle work remains opaque until consumed, allowing an adapter to reuse
/// backend result storage without remapping or reallocating it.
pub(super) trait KeyedWorkPort<L, K>: ScheduledTask + Sized {
    type Response;
    type PreparedJob;
    type CompletedJob;
    type VisibleState;
    type Lifecycle: WorkerLifecyclePort<
            L,
            K,
            Response = Self::Response,
            VisibleState = Self::VisibleState,
        >;

    fn metadata(&self, lifecycle: &L) -> KeyedWorkMetadata;
    fn prepare(
        self,
        lifecycle: &mut L,
        storage_key: K,
    ) -> PreparedKeyedWork<Self::Response, Self::PreparedJob>;
    fn run(job: Self::PreparedJob) -> impl Future<Output = Self::CompletedJob>;
    /// Reduces commands in order, deferring exactly one response for each command.
    fn collapse(
        lifecycle: &mut L,
        storage_key: K,
        base: Self::VisibleState,
        commands: impl ExactSizeIterator<Item = Self>,
        defer: impl FnMut(DeferredResponse<Self::Response>) -> usize,
    ) -> CollapsedKeyedWork<Self::PreparedJob, Self::VisibleState>;
    fn finish(
        lifecycle: &mut L,
        job: Self::CompletedJob,
        include_visible_state: bool,
    ) -> FinishedKeyedWork<Self::Response, Self::VisibleState>;
}
