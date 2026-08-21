//! Keyed operation contracts and completion coordination.
//!
//! This module is the storage-facing mutation adapter. It consumes typed
//! operation intent and returns storage outcomes; it does not encode protocol
//! bytes or inspect client/wire concerns.

use std::rc::Rc;
use std::time::Instant;

use crate::types::{StorageWriteCondition, StorageWriteOptions, StoredItemValue};
use crate::{BUCKET_BYTES, KvError, Result, StorageKey};

use super::policy::{item_state_is_live_at, set_condition_allows, unix_time_ms, validate_ttl};
use super::read_plan::{
    DirectReadPlan, KeyedObservation, LocatedKeyState, PreparedReadBacking, PreparedReadCandidate,
    ReadPurpose,
};
use super::value_reads::read_mutable_value;
use super::values::mutable_value_handle_for;
use super::{
    CAPACITY_CHECK_INTERVAL, ItemState, Kvkache, MutableValueHandle, ReadBacking,
    SegmentFlushReason, SetOutcome, TableLocation, bucket_hash, find_item_in_bucket,
};

pub(crate) enum KeyedOperation {
    Get,
    Set {
        value: StoredItemValue,
        options: StorageWriteOptions,
    },
    Delete,
    CompareExchange {
        expected: Option<StoredItemValue>,
        replacement: Option<StoredItemValue>,
        options: StorageWriteOptions,
    },
}

pub(crate) enum KeyedOutcome {
    Value(Option<StoredItemValue>),
    Set(SetOutcome),
    Deleted(bool),
    CompareExchange(bool),
}

#[derive(Clone)]
pub(crate) enum KeyedVisibleState {
    Missing,
    Present(StoredItemValue),
}

pub(crate) struct KeyedFinish {
    pub(crate) outcome: Result<KeyedOutcome>,
    pub(crate) visible_state: Option<KeyedVisibleState>,
    pub(crate) flush_required: bool,
    pub(crate) pending: bool,
}

pub(crate) struct PendingKeyedResult {
    pub(crate) storage_key: StorageKey,
    pub(crate) outcome: KeyedOutcome,
    pub(crate) visible_state: Option<KeyedVisibleState>,
}

enum PreparedKeyedOperation {
    Get,
    Set {
        value: StoredItemValue,
        options: StorageWriteOptions,
    },
    Delete,
    CompareExchange {
        expected: Option<StoredItemValue>,
        replacement: Option<StoredItemValue>,
        options: StorageWriteOptions,
    },
}

enum KeyedObservationPlan {
    Read(DirectReadPlan, ReadPurpose),
    Error(KvError),
}

pub(crate) struct KeyedJob {
    storage_key: StorageKey,
    operation: PreparedKeyedOperation,
    observation: KeyedObservationPlan,
}

pub(crate) struct CompletedKeyedJob {
    storage_key: StorageKey,
    operation: PreparedKeyedOperation,
    observation: Result<KeyedObservation>,
}

impl KeyedJob {
    pub(crate) async fn run(self) -> CompletedKeyedJob {
        let observation = match self.observation {
            KeyedObservationPlan::Read(plan, purpose) => plan.read(purpose).await,
            KeyedObservationPlan::Error(error) => Err(error),
        };
        CompletedKeyedJob {
            storage_key: self.storage_key,
            operation: self.operation,
            observation,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum PendingKeyedResponse {
    Set,
    CompareExchange,
}

pub(super) enum PendingKeyedMutation {
    Set {
        storage_key: StorageKey,
        value: StoredItemValue,
        ttl_ms: Option<u64>,
        eviction_protected: bool,
        previous: Option<TableLocation>,
        previous_mutable_value: Option<MutableValueHandle>,
        previous_state: Option<ItemState>,
        condition: StorageWriteCondition,
        include_visible_state: bool,
        response: PendingKeyedResponse,
    },
    Delete {
        storage_key: StorageKey,
        previous: TableLocation,
        previous_mutable_value: Option<MutableValueHandle>,
        previous_state: ItemState,
    },
}

impl Kvkache {
    pub(crate) fn prepare_keyed(
        &mut self,
        storage_key: StorageKey,
        operation: KeyedOperation,
    ) -> KeyedJob {
        let (operation, observation) = match operation {
            KeyedOperation::Get => (
                PreparedKeyedOperation::Get,
                self.keyed_read_plan(storage_key, ReadPurpose::Value),
            ),
            KeyedOperation::Set { value, options } => {
                let observation = if options.ttl_ms() == Some(0) {
                    KeyedObservationPlan::Error(KvError::InvalidRequest(
                        "SET TTL must be greater than zero milliseconds".into(),
                    ))
                } else if let Err(error) = validate_ttl(options.ttl_ms()) {
                    KeyedObservationPlan::Error(error)
                } else if let Err(error) =
                    self.validate_value(&value.bytes, options.ttl_ms().is_some())
                {
                    KeyedObservationPlan::Error(error)
                } else {
                    // Observe the current item state before admission. A
                    // conditional SET that will return NotStored must not be
                    // rejected by an unrelated capacity check.
                    self.keyed_read_plan(storage_key, ReadPurpose::State)
                };
                (PreparedKeyedOperation::Set { value, options }, observation)
            }
            KeyedOperation::Delete => (
                PreparedKeyedOperation::Delete,
                self.keyed_read_plan(storage_key, ReadPurpose::State),
            ),
            KeyedOperation::CompareExchange {
                expected,
                replacement,
                options,
            } => {
                let observation = if let Some(value) = replacement.as_ref() {
                    if options.ttl_ms() == Some(0) {
                        KeyedObservationPlan::Error(KvError::InvalidRequest(
                            "SET TTL must be greater than zero milliseconds".into(),
                        ))
                    } else if let Err(error) = validate_ttl(options.ttl_ms()) {
                        KeyedObservationPlan::Error(error)
                    } else if let Err(error) =
                        self.validate_value(&value.bytes, options.ttl_ms().is_some())
                    {
                        KeyedObservationPlan::Error(error)
                    } else {
                        self.keyed_read_plan(storage_key, ReadPurpose::CompareExchange)
                    }
                } else {
                    self.keyed_read_plan(storage_key, ReadPurpose::CompareExchange)
                };
                (
                    PreparedKeyedOperation::CompareExchange {
                        expected,
                        replacement,
                        options,
                    },
                    observation,
                )
            }
        };
        KeyedJob {
            storage_key,
            operation,
            observation,
        }
    }

    fn keyed_read_plan(
        &self,
        storage_key: StorageKey,
        purpose: ReadPurpose,
    ) -> KeyedObservationPlan {
        let mut candidates = Vec::new();
        let mut job_pins = Vec::new();
        for table_location in self.table.candidate_locations(&storage_key) {
            let Some(backing) = self.directory.read_backing(table_location.sg_index) else {
                continue;
            };
            let (sequence, backing) = match backing {
                ReadBacking::Mutable { lane, _job_pin } => {
                    job_pins.push(_job_pin);
                    let Some(generation) = self.mutable[lane].as_ref() else {
                        continue;
                    };
                    let bucket_index = bucket_hash(
                        &storage_key,
                        table_location.bucket_hash_index,
                        self.config.bucket_count(),
                    );
                    let start = bucket_index * BUCKET_BYTES;
                    let item = find_item_in_bucket(
                        &generation.segment.bytes[start..start + BUCKET_BYTES],
                        &storage_key,
                    );
                    let mutable_value = item.as_ref().and_then(|item| {
                        mutable_value_handle_for(lane, generation.logical_sg_id, &item.value)
                    });
                    let value = match (purpose, item.as_ref()) {
                        (ReadPurpose::Value | ReadPurpose::CompareExchange, Some(item))
                            if !item.is_tombstone =>
                        {
                            Some(read_mutable_value(item.value.clone(), generation))
                        }
                        _ => None,
                    };
                    (
                        generation.sequence,
                        PreparedReadBacking::Mutable {
                            item,
                            value,
                            mutable_value,
                        },
                    )
                }
                ReadBacking::Ram {
                    backing,
                    retirement_guard,
                } => (
                    backing.sequence,
                    PreparedReadBacking::Ram {
                        backing,
                        _retirement_guard: retirement_guard,
                    },
                ),
                ReadBacking::Ssd(backing) => (backing.sequence, PreparedReadBacking::Ssd(backing)),
            };
            candidates.push(PreparedReadCandidate {
                table_location,
                sequence,
                backing,
            });
        }
        KeyedObservationPlan::Read(
            DirectReadPlan {
                data: self.data.clone(),
                large_values: self.large_values.clone(),
                config: self.config.clone(),
                storage_key,
                candidates,
                io: Rc::clone(&self.io),
                _job_pins: job_pins,
            },
            purpose,
        )
    }

    pub(crate) fn finish_keyed(
        &mut self,
        completed: CompletedKeyedJob,
        include_visible_state: bool,
    ) -> KeyedFinish {
        let observation = match completed.observation {
            Ok(observation) => observation,
            Err(error) => {
                return KeyedFinish {
                    outcome: Err(error),
                    visible_state: None,
                    flush_required: false,
                    pending: false,
                };
            }
        };
        let (outcome, visible_state, flush_required, pending) =
            match (completed.operation, observation) {
                (PreparedKeyedOperation::Get, KeyedObservation::Value { mut value, expired }) => {
                    if let Some(expired) = expired {
                        if let Err(error) =
                            self.remove_expired_location(completed.storage_key, expired)
                        {
                            return KeyedFinish {
                                outcome: Err(error),
                                visible_state: None,
                                flush_required: false,
                                pending: false,
                            };
                        }
                    }
                    let visible_state = include_visible_state.then(|| match &mut value {
                        Some(value) => KeyedVisibleState::Present(value.clone_for_visible_state()),
                        None => KeyedVisibleState::Missing,
                    });
                    (Ok(KeyedOutcome::Value(value)), visible_state, false, false)
                }
                (
                    PreparedKeyedOperation::Set { value, options },
                    KeyedObservation::State(previous),
                ) => {
                    let evaluated_at_ms = unix_time_ms();
                    match self.finish_keyed_set(
                        completed.storage_key,
                        value,
                        options,
                        evaluated_at_ms,
                        previous,
                        include_visible_state,
                        PendingKeyedResponse::Set,
                    ) {
                        Ok((outcome, visible_state, flush_required, pending)) => (
                            Ok(KeyedOutcome::Set(outcome)),
                            visible_state,
                            flush_required,
                            pending,
                        ),
                        Err(error) => (Err(error), None, false, false),
                    }
                }
                (PreparedKeyedOperation::Delete, KeyedObservation::State(previous)) => {
                    match self.finish_keyed_delete(completed.storage_key, unix_time_ms(), previous)
                    {
                        Ok((deleted, pending)) => (
                            Ok(KeyedOutcome::Deleted(deleted)),
                            Some(KeyedVisibleState::Missing),
                            pending,
                            pending,
                        ),
                        Err(error) => (Err(error), None, false, false),
                    }
                }
                (
                    PreparedKeyedOperation::CompareExchange {
                        expected,
                        replacement,
                        options,
                    },
                    KeyedObservation::CompareExchange { state, mut value },
                ) => {
                    let matches = match (expected.as_ref(), value.as_ref()) {
                        (None, None) => true,
                        (Some(expected), Some(current)) => expected.bytes == current.bytes,
                        _ => false,
                    };
                    if !matches {
                        if let Some(state) = state {
                            if let Err(error) =
                                self.remove_expired_location(completed.storage_key, state)
                            {
                                return KeyedFinish {
                                    outcome: Err(error),
                                    visible_state: None,
                                    flush_required: false,
                                    pending: false,
                                };
                            }
                        }
                        let visible_state = include_visible_state.then(|| match &mut value {
                            Some(value) => {
                                KeyedVisibleState::Present(value.clone_for_visible_state())
                            }
                            None => KeyedVisibleState::Missing,
                        });
                        (
                            Ok(KeyedOutcome::CompareExchange(false)),
                            visible_state,
                            false,
                            false,
                        )
                    } else {
                        match replacement {
                            Some(replacement) => match self.finish_keyed_set(
                                completed.storage_key,
                                replacement,
                                options,
                                unix_time_ms(),
                                state,
                                include_visible_state,
                                PendingKeyedResponse::CompareExchange,
                            ) {
                                Ok((outcome, visible_state, flush_required, pending)) => (
                                    Ok(KeyedOutcome::CompareExchange(matches!(
                                        outcome,
                                        SetOutcome::Created | SetOutcome::Replaced
                                    ))),
                                    visible_state,
                                    flush_required,
                                    pending,
                                ),
                                Err(error) => (Err(error), None, false, false),
                            },
                            None => match self.finish_keyed_delete(
                                completed.storage_key,
                                unix_time_ms(),
                                state,
                            ) {
                                Ok((deleted, pending)) => (
                                    Ok(KeyedOutcome::CompareExchange(deleted)),
                                    Some(KeyedVisibleState::Missing),
                                    pending,
                                    pending,
                                ),
                                Err(error) => (Err(error), None, false, false),
                            },
                        }
                    }
                }
                _ => (
                    Err(KvError::Worker(
                        "keyed operation completed with an incompatible observation".into(),
                    )),
                    None,
                    false,
                    false,
                ),
            };
        KeyedFinish {
            outcome,
            visible_state,
            flush_required,
            pending,
        }
    }

    pub(crate) fn can_collapse_set(&self, value: &StoredItemValue) -> bool {
        self.validate_value(&value.bytes, false).is_ok()
    }

    fn finish_keyed_set(
        &mut self,
        storage_key: StorageKey,
        mut value: StoredItemValue,
        options: StorageWriteOptions,
        evaluated_at_ms: u64,
        previous: Option<LocatedKeyState>,
        include_visible_state: bool,
        pending_response: PendingKeyedResponse,
    ) -> Result<(SetOutcome, Option<KeyedVisibleState>, bool, bool)> {
        let previous_live = previous
            .as_ref()
            .is_some_and(|located| item_state_is_live_at(located.item_state, evaluated_at_ms));
        if !set_condition_allows(options.condition, previous_live) {
            if let Some(previous) = previous {
                self.remove_expired_location(storage_key, previous)?;
            }
            return Ok((SetOutcome::NotStored, None, false, false));
        }
        let previous_counted = previous.as_ref().is_some_and(|located| {
            !located.item_state.is_tombstone
                && self
                    .table
                    .candidate_locations(&storage_key)
                    .contains(&located.table_location)
        });
        self.admit_set()?;
        // Admission can wait for a capacity refresh. Re-evaluate the
        // condition at the mutation boundary so expiration is observed at
        // the same point that determines the replacement outcome.
        let previous_live = previous
            .as_ref()
            .is_some_and(|located| item_state_is_live_at(located.item_state, unix_time_ms()));
        if !set_condition_allows(options.condition, previous_live) {
            if let Some(previous) = previous {
                self.remove_expired_location(storage_key, previous)?;
            }
            return Ok((SetOutcome::NotStored, None, false, false));
        }
        let previous_location = previous.as_ref().map(|located| located.table_location);
        let previous_state = previous.as_ref().map(|located| located.item_state);
        let previous_mutable_value = previous.and_then(|located| located.mutable_value);
        if let Some(replacement) = self.try_append_value(
            storage_key,
            &mut value,
            options.ttl_ms(),
            options.eviction_protected(),
            previous_location,
            previous_mutable_value,
        )? {
            let previous_disappeared = self.publish_table_location(
                storage_key,
                previous_location,
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
            let visible_state = (include_visible_state
                && options == StorageWriteOptions::default())
            .then(|| KeyedVisibleState::Present(value.clone_for_visible_state()));
            return Ok((outcome, visible_state, false, false));
        }
        self.pending_keyed_mutations
            .push_back(PendingKeyedMutation::Set {
                storage_key,
                value,
                ttl_ms: options.ttl_ms(),
                eviction_protected: options.eviction_protected(),
                previous: previous_location,
                previous_mutable_value,
                previous_state,
                condition: options.condition,
                include_visible_state,
                response: pending_response,
            });
        // The operation has not linearized yet. The worker must defer its
        // response until capacity work appends the value and re-evaluates the
        // condition against the current expiration/eviction state.
        Ok((SetOutcome::NotStored, None, true, true))
    }

    pub(super) fn admit_set(&mut self) -> Result<()> {
        let now = Instant::now();
        let refresh_memory = now >= self.next_memory_capacity_check;
        if refresh_memory {
            self.next_memory_capacity_check = now + CAPACITY_CHECK_INTERVAL;
        }
        self.resource_guard.admit_set(refresh_memory)
    }

    fn finish_keyed_delete(
        &mut self,
        storage_key: StorageKey,
        evaluated_at_ms: u64,
        previous: Option<LocatedKeyState>,
    ) -> Result<(bool, bool)> {
        let Some(previous) = previous else {
            return Ok((false, false));
        };
        if !item_state_is_live_at(previous.item_state, evaluated_at_ms) {
            self.remove_expired_location(storage_key, previous)?;
            return Ok((false, false));
        }
        if let Some(replacement) = self.try_replace_tombstone_in_place(
            storage_key,
            previous.table_location,
            previous.mutable_value,
        )? {
            self.publish_tombstone_location(
                storage_key,
                Some(previous.table_location),
                previous.mutable_value,
                replacement,
            )?;
            self.live_keys = self.live_keys.saturating_sub(1);
            return Ok((true, false));
        }
        if let Some(replacement) = self.try_append_tombstone(storage_key)? {
            self.publish_tombstone_location(
                storage_key,
                Some(previous.table_location),
                previous.mutable_value,
                replacement,
            )?;
            self.live_keys = self.live_keys.saturating_sub(1);
            return Ok((true, false));
        }
        self.pending_keyed_mutations
            .push_back(PendingKeyedMutation::Delete {
                storage_key,
                previous: previous.table_location,
                previous_mutable_value: previous.mutable_value,
                previous_state: previous.item_state,
            });
        Ok((false, true))
    }

    pub(super) fn remove_expired_location(
        &mut self,
        storage_key: StorageKey,
        previous: LocatedKeyState,
    ) -> Result<()> {
        if previous.item_state.is_tombstone
            || previous.item_state.expires_at_ms == 0
            || previous.item_state.expires_at_ms > unix_time_ms()
        {
            return Ok(());
        }
        // A read may have been prepared before a newer write replaced the
        // table location. In that case the stale expiration observation must
        // not remove the live replacement.
        if !self
            .table
            .candidate_locations(&storage_key)
            .contains(&previous.table_location)
        {
            return Ok(());
        }
        let replacement = if let Some(replacement) = self.try_replace_tombstone_in_place(
            storage_key,
            previous.table_location,
            previous.mutable_value,
        )? {
            replacement
        } else {
            match self.try_append_tombstone(storage_key)? {
                Some(replacement) => replacement,
                None => {
                    // Expiration is discovered by a read, which cannot await
                    // the asynchronous capacity driver. Close one mutable
                    // generation synchronously when a flush slot is available,
                    // then publish the tombstone into the fresh generation.
                    // If all flush slots are occupied, preserve the existing
                    // NoCapacity signal so the caller can retry after
                    // background progress.
                    if self.active_flush_count() >= self.config.max_flushes_in_flight {
                        return Err(KvError::NoCapacity);
                    }
                    let lane = self.fullest_mutable_lane()?;
                    self.close_lane(lane, SegmentFlushReason::Capacity)?;
                    self.advance_closings()?;
                    self.advance_flushes()?;
                    self.try_append_tombstone(storage_key)?
                        .ok_or_else(|| KvError::NoCapacity)?
                }
            }
        };
        self.publish_tombstone_location(
            storage_key,
            Some(previous.table_location),
            previous.mutable_value,
            replacement,
        )?;
        self.live_keys = self.live_keys.saturating_sub(1);
        Ok(())
    }
}
