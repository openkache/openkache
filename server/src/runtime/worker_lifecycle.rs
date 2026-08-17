//! Operation-neutral lifecycle work driven by the shared worker loop.

use std::convert::Infallible;
use std::marker::PhantomData;
use std::task::{Context, Poll};

use crate::{KvError, Result};

/// Completed lifecycle work projected into the worker's keyed response types.
pub(super) struct DeferredCompletion<K, R, S> {
    pub(super) key: K,
    pub(super) outcome: R,
    pub(super) visible_state: Option<S>,
}

/// Static adapter for deferred and background work owned by an API lifecycle.
pub(super) trait WorkerLifecyclePort<L, K> {
    type Response;
    type VisibleState;
    type DeferredCompletion;

    /// Advances deferred work and emits each completed key at most once.
    ///
    /// Returns `true` only when no deferred work remains. On error, the worker
    /// fails retained responses and asks the adapter to cancel unresolved work.
    fn progress_deferred(
        lifecycle: &mut L,
        emit: impl FnMut(Self::DeferredCompletion),
    ) -> Result<bool>;
    /// Projects one adapter-owned completion without cloning response state.
    fn project_deferred(
        completion: Self::DeferredCompletion,
    ) -> DeferredCompletion<K, Self::Response, Self::VisibleState>;
    fn cancel_deferred(lifecycle: &mut L, key: K);
    fn has_background_work(lifecycle: &L) -> bool;
    fn poll_background(
        lifecycle: &mut L,
        context: &mut Context<'_>,
    ) -> Poll<Result<bool>>;
}

/// Zero-cost lifecycle facet for APIs without deferred or background work.
///
/// Commands using this facet must never finish with `pending` or
/// `flush_required` set because there is no lifecycle work that can resolve
/// either state.
#[allow(dead_code)]
pub(super) struct NoopWorkerLifecycle<R, S>(PhantomData<fn() -> (R, S)>);

impl<L, K, R, S> WorkerLifecyclePort<L, K> for NoopWorkerLifecycle<R, S> {
    type Response = R;
    type VisibleState = S;
    type DeferredCompletion = Infallible;

    fn progress_deferred(
        _lifecycle: &mut L,
        _emit: impl FnMut(Self::DeferredCompletion),
    ) -> Result<bool> {
        Err(KvError::Worker(
            "deferred work reached a no-op worker lifecycle".into(),
        ))
    }

    fn project_deferred(
        completion: Self::DeferredCompletion,
    ) -> DeferredCompletion<K, Self::Response, Self::VisibleState> {
        match completion {}
    }

    fn cancel_deferred(_lifecycle: &mut L, _key: K) {}

    fn has_background_work(_lifecycle: &L) -> bool {
        false
    }

    fn poll_background(
        _lifecycle: &mut L,
        _context: &mut Context<'_>,
    ) -> Poll<Result<bool>> {
        Poll::Pending
    }
}
