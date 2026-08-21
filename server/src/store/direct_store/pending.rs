//! Pending keyed mutation progress and capacity coordination.

use crate::types::StorageWriteCondition;
use crate::{Result, StorageKey};

use super::keyed::{
    KeyedOutcome, KeyedVisibleState, PendingKeyedMutation, PendingKeyedResponse, PendingKeyedResult,
};
use super::policy::{item_state_is_live_at, set_condition_allows, unix_time_ms};
use super::read_plan::LocatedKeyState;
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
                }
                    | PendingKeyedMutation::Delete {
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
                let previous_observed_live = previous_state
                    .is_some_and(|state| item_state_is_live_at(state, unix_time_ms()));
                let previous_location_current = previous.is_some_and(|location| {
                    self.table
                        .candidate_locations(&storage_key)
                        .contains(&location)
                });
                let previous_live = match condition {
                    StorageWriteCondition::Any => previous_observed_live,
                    StorageWriteCondition::IfAbsent | StorageWriteCondition::IfPresent => {
                        previous_observed_live && previous_location_current
                    }
                };
                if !set_condition_allows(condition, previous_live) {
                    if let (Some(previous), Some(previous_state)) = (previous, previous_state) {
                        self.remove_expired_location(
                            storage_key,
                            LocatedKeyState {
                                table_location: previous,
                                item_state: previous_state,
                                mutable_value: previous_mutable_value,
                            },
                        )?;
                    }
                    return Ok(PendingKeyedProgress::Complete(PendingKeyedResult {
                        storage_key,
                        outcome: pending_outcome(response, SetOutcome::NotStored),
                        visible_state: None,
                    }));
                }
                let previous_counted = previous_state.is_some_and(|state| {
                    !state.is_tombstone
                        && previous.is_some_and(|location| {
                            self.table
                                .candidate_locations(&storage_key)
                                .contains(&location)
                        })
                });
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
                if !previous_counted || previous_disappeared {
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
            PendingKeyedMutation::Delete {
                storage_key,
                previous,
                previous_mutable_value,
                previous_state,
            } => {
                // Capacity work may have evicted the observed generation
                // before the tombstone became appendable. In that case the
                // deleted record is already gone and no second live-key
                // decrement or Table publication is needed.
                if !self
                    .table
                    .candidate_locations(&storage_key)
                    .contains(&previous)
                {
                    // Capacity work may have evicted the observed location
                    // before the pending tombstone became appendable. The
                    // item is then already logically absent, so report the
                    // same result as a delete that observed a miss.
                    return Ok(PendingKeyedProgress::Complete(PendingKeyedResult {
                        storage_key,
                        outcome: KeyedOutcome::Deleted(false),
                        visible_state: Some(KeyedVisibleState::Missing),
                    }));
                }
                let Some(replacement) = self.try_append_tombstone(storage_key)? else {
                    return Ok(PendingKeyedProgress::Pending(
                        PendingKeyedMutation::Delete {
                            storage_key,
                            previous,
                            previous_mutable_value,
                            previous_state,
                        },
                    ));
                };
                // Publishing the tombstone is the mutation linearization
                // point. Resolve TTL immediately before it so a value that
                // expired while capacity work was in flight reports the same
                // result as an already-expired delete.
                let deleted = item_state_is_live_at(previous_state, unix_time_ms());
                self.publish_tombstone_location(
                    storage_key,
                    Some(previous),
                    previous_mutable_value,
                    replacement,
                )?;
                self.live_keys = self.live_keys.saturating_sub(1);
                return Ok(PendingKeyedProgress::Complete(PendingKeyedResult {
                    storage_key,
                    outcome: KeyedOutcome::Deleted(deleted),
                    visible_state: Some(KeyedVisibleState::Missing),
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
