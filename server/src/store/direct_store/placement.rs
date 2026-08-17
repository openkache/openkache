//! Mutable value placement and table publication.

use crate::types::StoredItemValue;
use crate::{KvError, Result, StorageKey};

use super::policy::ttl_deadline;
use super::values::{
    InPlaceValue, clear_mutable_value, live_item, mutable_generation_for_location,
    mutable_value_handle_for, same_physical_bucket, stage_mutable_value,
    try_replace_value_in_place,
};
use super::{
    BLOB_ITEM_THRESHOLD_BYTES, ITEM_EXPIRATION_BYTES, ITEM_FIXED_BYTES, Kvkache, LocatedItem,
    MutablePlacement, MutableValueHandle, ReadBacking, STORED_BLOB_REF_BYTES,
    STORED_LARGE_VALUE_REF_BYTES, STORED_VALUE_TAG_BYTES, TableLocation,
};

impl Kvkache {
    pub(super) fn try_append_value(
        &mut self,
        storage_key: StorageKey,
        value: &mut StoredItemValue,
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
            // Resolve the relative TTL immediately before the append. Capacity
            // work may have delayed this pending SET, so the deadline must not
            // start at request admission or while the value is being staged.
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

    pub(super) fn mutable_value_handle(&self, located: &LocatedItem) -> Option<MutableValueHandle> {
        let ReadBacking::Mutable { lane, .. } = &located.backing else {
            return None;
        };
        mutable_value_handle_for(*lane, located.table_location.sg_index, &located.item.value)
    }

    pub(super) fn publish_table_location(
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

    pub(super) fn remove_table_location(
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
