//! Projection of storage lifecycle completion into worker envelopes.

use crate::store::KeyedFinish as StoreKeyedFinish;
use crate::Kvkache;

use super::super::WorkerResponse;
use super::super::worker_contract::FinishedKeyedWork;
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
