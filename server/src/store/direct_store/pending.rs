//! Pending keyed mutation progress and capacity coordination.

use crate::{Result, StorageKey};

use super::keyed::{
    KeyedOutcome, KeyedVisibleState, PendingKeyedMutation, PendingKeyedResponse, PendingKeyedResult,
};
use super::policy::{item_state_is_live_at, set_condition_allows, unix_time_ms};
use super::{Kvkache, SegmentFlushReason, SetOutcome};

enum PendingKeyedProgress {
    Complete(PendingKeyedResult),
    Pending(PendingKeyedMutation),
}

impl Kvkache {
    pub(crate) fn progress_capacity(
        &mut self,
        mut emit: impl FnMut(PendingKeyedResult),
    ) -> Result<bool> {
        self.advance_closings()?;
        self.advance_flushes()?;
        while let Some(mutation) = self.pending_keyed_mutations.pop_front() {
            match self.try_apply_pending_keyed_mutation(mutation)? {
                PendingKeyedProgress::Complete(result) => emit(result),
                PendingKeyedProgress::Pending(mutation) => {
                    self.pending_keyed_mutations.push_front(mutation);
                    if self.active_flush_count() >= self.config.max_flushes_in_flight {
                        return Ok(false);
                    }
                    let lane = self.fullest_mutable_lane()?;
                    self.close_lane(lane, SegmentFlushReason::Capacity)?;
                    self.advance_closings()?;
                    self.advance_flushes()?;
                }
            }
        }
        Ok(true)
    }

    pub(crate) fn cancel_pending_keyed_mutation(&mut self, storage_key: StorageKey) {
        self.pending_keyed_mutations.retain(|mutation| {
            !matches!(
                mutation,
                PendingKeyedMutation::Set {
                    storage_key: pending_key,
                    ..
                } if *pending_key == storage_key
            )
        });
    }

    fn try_apply_pending_keyed_mutation(
        &mut self,
        mutation: PendingKeyedMutation,
    ) -> Result<PendingKeyedProgress> {
        match mutation {
            PendingKeyedMutation::Set {
                storage_key,
                mut value,
                ttl_ms,
                eviction_protected,
                previous,
                previous_mutable_value,
                previous_state,
                condition,
                include_visible_state,
                response,
            } => {
                let previous_live = previous_state.is_some_and(|state| {
                    item_state_is_live_at(state, unix_time_ms())
                        && previous.is_some_and(|location| {
                            self.table
                                .candidate_locations(&storage_key)
                                .contains(&location)
                        })
                });
                if !set_condition_allows(condition, previous_live) {
                    return Ok(PendingKeyedProgress::Complete(PendingKeyedResult {
                        storage_key,
                        outcome: pending_outcome(response, SetOutcome::NotStored),
                        visible_state: None,
                    }));
                }
                let Some(replacement) = self.try_append_value(
                    storage_key,
                    &mut value,
                    ttl_ms,
                    eviction_protected,
                    previous,
                    previous_mutable_value,
                )?
                else {
                    return Ok(PendingKeyedProgress::Pending(PendingKeyedMutation::Set {
                        storage_key,
                        value,
                        ttl_ms,
                        eviction_protected,
                        previous,
                        previous_mutable_value,
                        previous_state,
                        condition,
                        include_visible_state,
                        response,
                    }));
                };
                let previous_disappeared = self.publish_table_location(
                    storage_key,
                    previous,
                    previous_mutable_value,
                    replacement,
                )?;
                if !previous_live || previous_disappeared {
                    self.live_keys += 1;
                }
                let outcome = if previous_live {
                    SetOutcome::Replaced
                } else {
                    SetOutcome::Created
                };
                let visible_state = include_visible_state
                    .then(|| KeyedVisibleState::Present(value.clone_for_visible_state()));
                return Ok(PendingKeyedProgress::Complete(PendingKeyedResult {
                    storage_key,
                    outcome: pending_outcome(response, outcome),
                    visible_state,
                }));
            }
        }
    }
}

fn pending_outcome(response: PendingKeyedResponse, outcome: SetOutcome) -> KeyedOutcome {
    match response {
        PendingKeyedResponse::Set => KeyedOutcome::Set(outcome),
        PendingKeyedResponse::CompareExchange => KeyedOutcome::CompareExchange(matches!(
            outcome,
            SetOutcome::Created | SetOutcome::Replaced
        )),
    }
}
