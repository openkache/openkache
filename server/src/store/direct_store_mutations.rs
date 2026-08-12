//! Keyed observation, mutation, and table-publication coordination.
//!
//! This module is the storage-facing mutation adapter. It consumes typed
//! operation intent and returns storage outcomes; it does not encode protocol
//! bytes or inspect client/wire concerns.

use std::rc::Rc;
use std::time::Instant;

use crate::protocol::{EvictionMode, SetOptions};
use crate::types::StoredItemValue;
use crate::{BUCKET_BYTES, KvError, Result, StorageKey};

use super::direct_store_policy::{
    item_is_live_now, item_state_is_live_at, set_condition_allows, ttl_deadline, unix_time_ms,
    validate_ttl,
};
use super::direct_store_reads::{
    DirectReadPlan, PreparedReadBacking, PreparedReadCandidate, read_mutable_value,
};
use super::direct_store_values::{
    InPlaceValue, clear_mutable_value, live_item, mutable_generation_for_location,
    mutable_value_handle_for, same_physical_bucket, stage_mutable_value,
    try_replace_value_in_place,
};
use super::{
    BLOB_ITEM_THRESHOLD_BYTES, CAPACITY_CHECK_INTERVAL, ITEM_EXPIRATION_BYTES, ITEM_FIXED_BYTES,
    ItemState, Kvkache, LocatedItem, MutablePlacement, MutableValueHandle, ReadBacking,
    STORED_BLOB_REF_BYTES, STORED_LARGE_VALUE_REF_BYTES, STORED_VALUE_TAG_BYTES,
    SegmentFlushReason, SetOutcome, TableLocation, bucket_hash, find_item_in_bucket,
};

pub(crate) enum KeyedOperation {
    Get,
    Set {
        value: StoredItemValue,
        options: SetOptions,
    },
    Delete,
}

pub(crate) enum KeyedOutcome {
    Value(Option<StoredItemValue>),
    Set(SetOutcome),
    Deleted(bool),
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
        options: SetOptions,
    },
    Delete,
}

#[derive(Clone, Copy)]
pub(super) struct LocatedKeyState {
    pub(super) table_location: TableLocation,
    pub(super) item_state: ItemState,
    pub(super) mutable_value: Option<MutableValueHandle>,
}

pub(super) enum KeyedObservation {
    Value(Option<StoredItemValue>),
    State(Option<LocatedKeyState>),
}

pub(super) enum KeyedObservationPlan {
    Read(DirectReadPlan, ReadPurpose),
    Error(KvError),
}

#[derive(Clone, Copy)]
pub(super) enum ReadPurpose {
    Value,
    State,
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

pub(super) enum PendingKeyedMutation {
    Set {
        storage_key: StorageKey,
        value: StoredItemValue,
        ttl_ms: Option<u64>,
        eviction_protected: bool,
        previous: Option<TableLocation>,
        previous_mutable_value: Option<MutableValueHandle>,
        previous_state: Option<ItemState>,
        condition: crate::protocol::SetCondition,
        include_visible_state: bool,
    },
}

enum PendingKeyedProgress {
    Complete(PendingKeyedResult),
    Pending(PendingKeyedMutation),
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
                let observation = if options.ttl_ms == Some(0) {
                    KeyedObservationPlan::Error(KvError::InvalidRequest(
                        "SET TTL must be greater than zero milliseconds".into(),
                    ))
                } else if let Err(error) = validate_ttl(options.ttl_ms) {
                    KeyedObservationPlan::Error(error)
                } else if let Err(error) =
                    self.validate_value(&value.bytes, options.ttl_ms.is_some())
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
                        (ReadPurpose::Value, Some(item)) if !item.is_tombstone => Some(
                            read_mutable_value(item.value.clone(), generation)
                                .map(StoredItemValue::new),
                        ),
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
                (PreparedKeyedOperation::Get, KeyedObservation::Value(mut value)) => {
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
                        Ok((deleted, flush_required)) => (
                            Ok(KeyedOutcome::Deleted(deleted)),
                            Some(KeyedVisibleState::Missing),
                            flush_required,
                            false,
                        ),
                        Err(error) => (Err(error), None, false, false),
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
        options: SetOptions,
        evaluated_at_ms: u64,
        previous: Option<LocatedKeyState>,
        include_visible_state: bool,
    ) -> Result<(SetOutcome, Option<KeyedVisibleState>, bool, bool)> {
        let previous_live = previous
            .as_ref()
            .is_some_and(|located| item_state_is_live_at(located.item_state, evaluated_at_ms));
        if !set_condition_allows(options.condition, previous_live) {
            return Ok((SetOutcome::NotStored, None, false, false));
        }
        self.admit_set()?;
        let previous_live = previous
            .as_ref()
            .is_some_and(|located| item_state_is_live_at(located.item_state, unix_time_ms()));
        if !set_condition_allows(options.condition, previous_live) {
            return Ok((SetOutcome::NotStored, None, false, false));
        }
        let previous_location = previous.as_ref().map(|located| located.table_location);
        let previous_state = previous.as_ref().map(|located| located.item_state);
        let previous_mutable_value = previous.and_then(|located| located.mutable_value);
        if let Some(replacement) = self.try_append_value(
            storage_key,
            &value.bytes,
            options.ttl_ms,
            matches!(options.eviction_mode, EvictionMode::EvictionProtected),
            previous_location,
            previous_mutable_value,
        )? {
            let previous_disappeared = self.publish_table_location(
                storage_key,
                previous_location,
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
            let visible_state = (include_visible_state && options == SetOptions::NONE)
                .then(|| KeyedVisibleState::Present(value.clone_for_visible_state()));
            return Ok((outcome, visible_state, false, false));
        }
        self.pending_keyed_mutations
            .push_back(PendingKeyedMutation::Set {
                storage_key,
                value,
                ttl_ms: options.ttl_ms,
                eviction_protected: matches!(
                    options.eviction_mode,
                    EvictionMode::EvictionProtected
                ),
                previous: previous_location,
                previous_mutable_value,
                previous_state,
                condition: options.condition,
                include_visible_state,
            });
        Ok((SetOutcome::NotStored, None, true, true))
    }

    fn admit_set(&mut self) -> Result<()> {
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
            return Ok((false, false));
        }
        self.remove_table_location(storage_key, previous.table_location, previous.mutable_value)?;
        self.live_keys = self.live_keys.saturating_sub(1);
        Ok((true, false))
    }

    pub(crate) fn progress_capacity(&mut self) -> Result<(bool, Vec<PendingKeyedResult>)> {
        self.advance_closings()?;
        self.advance_flushes()?;
        let mut completed = Vec::new();
        while let Some(mutation) = self.pending_keyed_mutations.pop_front() {
            match self.try_apply_pending_keyed_mutation(mutation)? {
                PendingKeyedProgress::Complete(result) => completed.push(result),
                PendingKeyedProgress::Pending(mutation) => {
                    self.pending_keyed_mutations.push_front(mutation);
                    if self.active_flush_count() >= self.config.max_flushes_in_flight {
                        return Ok((false, completed));
                    }
                    let lane = self.fullest_mutable_lane()?;
                    self.close_lane(lane, SegmentFlushReason::Capacity)?;
                    self.advance_closings()?;
                    self.advance_flushes()?;
                }
            }
        }
        Ok((true, completed))
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
                        outcome: KeyedOutcome::Set(SetOutcome::NotStored),
                        visible_state: None,
                    }));
                }
                let Some(replacement) = self.try_append_value(
                    storage_key,
                    &value.bytes,
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
                Ok(PendingKeyedProgress::Complete(PendingKeyedResult {
                    storage_key,
                    outcome: KeyedOutcome::Set(outcome),
                    visible_state,
                }))
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn get(&self, storage_key: &StorageKey) -> Result<Option<Vec<u8>>> {
        Ok(self
            .get_encoded(storage_key)
            .await?
            .map(StoredItemValue::into_bytes))
    }

    pub(crate) async fn get_encoded(
        &self,
        storage_key: &StorageKey,
    ) -> Result<Option<StoredItemValue>> {
        let Some(located) = self.locate_item(storage_key).await? else {
            return Ok(None);
        };
        if !item_is_live_now(&located.item) {
            return Ok(None);
        }
        let bytes = self.read_value(located.item.value, located.backing).await?;
        Ok(Some(StoredItemValue::new(bytes)))
    }

    #[allow(dead_code)]
    pub(crate) async fn set(
        &mut self,
        storage_key: StorageKey,
        value: &[u8],
    ) -> Result<SetOutcome> {
        self.set_encoded(storage_key, StoredItemValue::new(value.to_vec()))
            .await
    }

    pub(crate) async fn set_encoded(
        &mut self,
        storage_key: StorageKey,
        value: StoredItemValue,
    ) -> Result<SetOutcome> {
        self.set_encoded_with_options(storage_key, value, SetOptions::NONE)
            .await
    }

    pub(crate) async fn set_encoded_with_options(
        &mut self,
        storage_key: StorageKey,
        value: StoredItemValue,
        options: SetOptions,
    ) -> Result<SetOutcome> {
        self.drive_background_once().await?;
        if options.ttl_ms == Some(0) {
            return Err(KvError::InvalidRequest(
                "SET TTL must be greater than zero milliseconds".into(),
            ));
        }
        validate_ttl(options.ttl_ms)?;
        self.validate_value(&value.bytes, options.ttl_ms.is_some())?;
        let initial_previous = self.locate_item(&storage_key).await?;
        let initial_previous_live = initial_previous.as_ref().is_some_and(|located| {
            located.item.is_live_at(unix_time_ms())
                && self
                    .table
                    .candidate_locations(&storage_key)
                    .contains(&located.table_location)
        });
        if !set_condition_allows(options.condition, initial_previous_live) {
            return Ok(SetOutcome::NotStored);
        }
        drop(initial_previous);
        self.admit_set()?;
        let mut previous = self.locate_item(&storage_key).await?;
        let (new_location, previous_live, previous_location, previous_mutable_value) = loop {
            let previous_live = previous.as_ref().is_some_and(|located| {
                located.item.is_live_at(unix_time_ms())
                    && self
                        .table
                        .candidate_locations(&storage_key)
                        .contains(&located.table_location)
            });
            if !set_condition_allows(options.condition, previous_live) {
                return Ok(SetOutcome::NotStored);
            }
            let previous_location = previous.as_ref().map(|located| located.table_location);
            let previous_mutable_value = previous
                .as_ref()
                .and_then(|located| self.mutable_value_handle(located));
            if let Some(location) = self.try_append_value(
                storage_key,
                &value.bytes,
                options.ttl_ms,
                matches!(options.eviction_mode, EvictionMode::EvictionProtected),
                previous_location,
                previous_mutable_value,
            )? {
                break (
                    location,
                    previous_live,
                    previous_location,
                    previous_mutable_value,
                );
            }
            let lane = self.fullest_mutable_lane()?;
            self.flush_lane(lane, SegmentFlushReason::Capacity).await?;
            previous = self.locate_item(&storage_key).await?;
        };
        let previous_disappeared = self.publish_table_location(
            storage_key,
            previous_location,
            previous_mutable_value,
            new_location,
        )?;
        match (previous_live, true) {
            (false, true) => self.live_keys += 1,
            (true, true) => {}
            _ => unreachable!(),
        }
        if previous_live && previous_disappeared {
            self.live_keys += 1;
        }
        Ok(if previous_live {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        })
    }

    /// Compares the live value and applies the replacement while the store is
    /// exclusively borrowed by one worker lane.
    pub(crate) async fn compare_and_set_encoded_with_options(
        &mut self,
        storage_key: StorageKey,
        expected: Option<&[u8]>,
        replacement: Option<StoredItemValue>,
        options: SetOptions,
    ) -> Result<bool> {
        let current = self.get_encoded(&storage_key).await?;
        let matches = match (expected, current.as_ref()) {
            (None, None) => true,
            (Some(expected), Some(current)) => expected == current.as_ref(),
            _ => false,
        };
        if !matches {
            return Ok(false);
        }
        match replacement {
            Some(value) => Ok(matches!(
                self.set_encoded_with_options(storage_key, value, options)
                    .await?,
                SetOutcome::Created | SetOutcome::Replaced
            )),
            None => Ok(self.delete(&storage_key).await?),
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn delete(&mut self, storage_key: &StorageKey) -> Result<bool> {
        self.drive_background_once().await?;
        let now_ms = unix_time_ms();
        let previous = self.locate_item(storage_key).await?;
        let Some(previous) = previous else {
            return Ok(false);
        };
        if !previous.item.is_live_at(now_ms) {
            return Ok(false);
        }
        let previous_location = previous.table_location;
        let previous_mutable_value = self.mutable_value_handle(&previous);
        self.remove_table_location(*storage_key, previous_location, previous_mutable_value)?;
        self.live_keys = self.live_keys.saturating_sub(1);
        Ok(true)
    }

    pub(crate) async fn sync(&mut self) -> Result<()> {
        for lane in 0..self.mutable.len() {
            let should_flush = self.mutable[lane]
                .as_ref()
                .is_some_and(|generation| generation.segment.item_count != 0);
            if should_flush {
                self.flush_lane(lane, SegmentFlushReason::Sync).await?;
            }
        }
        while self.has_background_work() {
            self.wait_for_background_progress().await?;
        }
        Ok(())
    }

    fn try_append_value(
        &mut self,
        storage_key: StorageKey,
        value: &[u8],
        ttl_ms: Option<u64>,
        eviction_protected: bool,
        previous_location: Option<TableLocation>,
        previous_mutable_value: Option<MutableValueHandle>,
    ) -> Result<Option<MutablePlacement>> {
        let large = value.len() > self.config.large_value_threshold
            || value.len() > self.config.blob_segment_size;
        let blob = !large && value.len() > BLOB_ITEM_THRESHOLD_BYTES;
        let encoded_len = if large {
            STORED_LARGE_VALUE_REF_BYTES
        } else if blob {
            STORED_BLOB_REF_BYTES
        } else {
            STORED_VALUE_TAG_BYTES + value.len()
        };
        let has_expiration = ttl_ms.is_some();
        for lane in 0..self.mutable.len() {
            let Some(generation) = self.mutable[lane].as_mut() else {
                continue;
            };
            let fixed_item_bytes = ITEM_FIXED_BYTES
                + if has_expiration {
                    ITEM_EXPIRATION_BYTES
                } else {
                    0
                }
                + encoded_len;
            let previous_in_generation =
                previous_location.filter(|location| location.sg_index == generation.logical_sg_id);
            if let Some(previous_location) = previous_in_generation
                && let InPlaceValue::Replaced(mutable_value) = try_replace_value_in_place(
                    generation,
                    lane,
                    previous_location,
                    storage_key,
                    value,
                    ttl_ms,
                    eviction_protected,
                    large,
                    blob,
                    previous_mutable_value,
                )?
            {
                return Ok(Some(MutablePlacement {
                    table_location: previous_location,
                    mutable_value,
                    in_place: true,
                }));
            }
            if generation
                .segment
                .choose_bucket(&storage_key, fixed_item_bytes)
                .is_none()
            {
                continue;
            }
            let Some(staged) = stage_mutable_value(generation, lane, value, large, blob)? else {
                continue;
            };
            let expires_at_ms = match ttl_deadline(ttl_ms) {
                Ok(expires_at_ms) => expires_at_ms,
                Err(error) => {
                    clear_mutable_value(generation, lane, staged.mutable_value);
                    return Err(error);
                }
            };
            let item = live_item(
                storage_key,
                staged.encoded,
                expires_at_ms,
                eviction_protected,
            );
            let location = generation.segment.append(item, true).ok_or_else(|| {
                KvError::Worker("chosen mutable SG Bucket rejected an Item".into())
            })?;
            if previous_in_generation.is_some_and(|previous| {
                same_physical_bucket(&storage_key, previous, location, self.config.bucket_count())
            }) {
                generation.segment.remove(location, &storage_key);
                clear_mutable_value(generation, lane, staged.mutable_value);
                continue;
            }
            return Ok(Some(MutablePlacement {
                table_location: location,
                mutable_value: staged.mutable_value,
                in_place: false,
            }));
        }
        Ok(None)
    }

    fn mutable_value_handle(&self, located: &LocatedItem) -> Option<MutableValueHandle> {
        let ReadBacking::Mutable { lane, .. } = &located.backing else {
            return None;
        };
        mutable_value_handle_for(*lane, located.table_location.sg_index, &located.item.value)
    }

    fn publish_table_location(
        &mut self,
        storage_key: StorageKey,
        previous: Option<TableLocation>,
        previous_mutable_value: Option<MutableValueHandle>,
        replacement: MutablePlacement,
    ) -> Result<bool> {
        if replacement.in_place {
            debug_assert_eq!(previous, Some(replacement.table_location));
            return Ok(false);
        }
        let previous_disappeared = match previous {
            Some(previous)
                if self.table.replace_location(
                    &storage_key,
                    previous,
                    replacement.table_location,
                ) =>
            {
                false
            }
            Some(previous) => {
                if self
                    .table
                    .candidate_locations(&storage_key)
                    .contains(&previous)
                {
                    self.rollback_mutable_placement(&storage_key, &replacement);
                    return Err(KvError::Worker(
                        "Table location changed before the mutable replacement was published"
                            .into(),
                    ));
                }
                if let Err(error) = self.table.insert(&storage_key, replacement.table_location) {
                    self.rollback_mutable_placement(&storage_key, &replacement);
                    return Err(error);
                }
                true
            }
            None => {
                if let Err(error) = self.table.insert(&storage_key, replacement.table_location) {
                    self.rollback_mutable_placement(&storage_key, &replacement);
                    return Err(error);
                }
                false
            }
        };
        if let Some(previous) = previous {
            self.remove_previous_mutable_item(&storage_key, previous, previous_mutable_value);
        }
        Ok(previous_disappeared)
    }

    fn rollback_mutable_placement(
        &mut self,
        storage_key: &StorageKey,
        placement: &MutablePlacement,
    ) {
        let Some((lane, generation)) =
            mutable_generation_for_location(&mut self.mutable, placement.table_location)
        else {
            return;
        };
        if generation
            .segment
            .remove(placement.table_location, storage_key)
        {
            clear_mutable_value(generation, lane, placement.mutable_value);
        }
    }

    fn remove_previous_mutable_item(
        &mut self,
        storage_key: &StorageKey,
        previous: TableLocation,
        previous_mutable_value: Option<MutableValueHandle>,
    ) {
        let Some((lane, generation)) = mutable_generation_for_location(&mut self.mutable, previous)
        else {
            return;
        };
        if generation.segment.remove(previous, storage_key) {
            clear_mutable_value(generation, lane, previous_mutable_value);
        }
    }

    fn remove_table_location(
        &mut self,
        storage_key: StorageKey,
        previous: TableLocation,
        previous_mutable_value: Option<MutableValueHandle>,
    ) -> Result<()> {
        if !self.table.remove(&storage_key, previous) {
            return Err(KvError::Worker(
                "Table location changed before DELETE was published".into(),
            ));
        }
        self.remove_previous_mutable_item(&storage_key, previous, previous_mutable_value);
        Ok(())
    }
}
