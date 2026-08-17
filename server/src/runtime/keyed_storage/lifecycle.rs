//! Storage lifecycle adapter for the operation-neutral worker.

use std::task::{Context, Poll};

use crate::store::PendingKeyedResult;
use crate::{Kvkache, StorageKey};

use super::super::WorkerResponse;
use super::super::worker_lifecycle::{DeferredCompletion, WorkerLifecyclePort};
use super::VisibleState;

pub(in crate::runtime) struct Lifecycle;

impl WorkerLifecyclePort<Kvkache, StorageKey> for Lifecycle {
    type Response = WorkerResponse;
    type VisibleState = VisibleState;
    type DeferredCompletion = PendingKeyedResult;

    fn progress_deferred(
        lifecycle: &mut Kvkache,
        emit: impl FnMut(Self::DeferredCompletion),
    ) -> crate::Result<bool> {
        Kvkache::progress_capacity(lifecycle, emit)
    }

    fn project_deferred(
        completion: Self::DeferredCompletion,
    ) -> DeferredCompletion<StorageKey, Self::Response, Self::VisibleState> {
        DeferredCompletion {
            key: completion.storage_key,
            outcome: WorkerResponse::Data(completion.outcome),
            visible_state: completion.visible_state,
        }
    }

    fn cancel_deferred(lifecycle: &mut Kvkache, storage_key: StorageKey) {
        Kvkache::cancel_pending_keyed_mutation(lifecycle, storage_key);
    }

    fn has_background_work(lifecycle: &Kvkache) -> bool {
        Kvkache::has_background_work(lifecycle)
    }

    fn poll_background(
        lifecycle: &mut Kvkache,
        context: &mut Context<'_>,
    ) -> Poll<crate::Result<bool>> {
        Kvkache::poll_background(lifecycle, context)
    }
}
