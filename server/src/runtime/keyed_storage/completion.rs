//! Projection of storage lifecycle completion into worker envelopes.

use crate::store::{KeyedFinish as StoreKeyedFinish, PendingKeyedResult};
use crate::{Kvkache, StorageKey};

use super::super::WorkerResponse;
use super::super::worker_contract::{CapacityCompletion, FinishedKeyedWork};
use super::{CompletedJob, VisibleState};

pub(super) fn finish(
    cache: &mut Kvkache,
    job: CompletedJob,
    include_visible_state: bool,
) -> FinishedKeyedWork<WorkerResponse, VisibleState> {
    let StoreKeyedFinish {
        outcome,
        visible_state,
        flush_required,
        pending,
    } = cache.finish_keyed(job, include_visible_state);
    FinishedKeyedWork {
        outcome: outcome.map(WorkerResponse::Data),
        visible_state,
        flush_required,
        pending,
    }
}

pub(super) fn capacity_completion(
    completion: PendingKeyedResult,
) -> CapacityCompletion<StorageKey, WorkerResponse, VisibleState> {
    CapacityCompletion {
        storage_key: completion.storage_key,
        outcome: WorkerResponse::Data(completion.outcome),
        visible_state: completion.visible_state,
    }
}
