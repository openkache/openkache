//! Direct keyed read, write, compare-and-set, and delete commands.

use crate::types::{StorageWriteOptions, StoredItemValue};
use crate::{KvError, Result, StorageKey};

use super::policy::{item_is_live_now, set_condition_allows, unix_time_ms, validate_ttl};
use super::{Kvkache, SegmentFlushReason, SetOutcome};

impl Kvkache {
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
        self.read_value(located.item.value, located.backing)
            .await
            .map(Some)
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
        self.set_encoded_with_options(storage_key, value, StorageWriteOptions::default())
            .await
    }

    pub(crate) async fn set_encoded_with_options(
        &mut self,
        storage_key: StorageKey,
        mut value: StoredItemValue,
        options: StorageWriteOptions,
    ) -> Result<SetOutcome> {
        self.drive_background_once().await?;
        if options.ttl_ms() == Some(0) {
            return Err(KvError::InvalidRequest(
                "SET TTL must be greater than zero milliseconds".into(),
            ));
        }
        validate_ttl(options.ttl_ms())?;
        self.validate_value(&value.bytes, options.ttl_ms().is_some())?;
        // Check the condition before admission so a request that is already
        // known to be NotStored does not fail because of unrelated capacity.
        let initial_previous = self.locate_item(&storage_key).await?;
        let initial_previous_live = initial_previous.as_ref().is_some_and(|located| {
            located.item.is_live_at(unix_time_ms())
                && self
                    .table
                    .candidate_locations(&storage_key)
                    .contains(&located.table_location)
        });
        if !set_condition_allows(options.condition, initial_previous_live) {
            if let Some(previous) = initial_previous {
                self.remove_expired_item(&storage_key, previous)?;
            }
            return Ok(SetOutcome::NotStored);
        }
        drop(initial_previous);
        self.admit_set()?;
        let mut previous = self.locate_item(&storage_key).await?;
        let (
            new_location,
            previous_live,
            previous_counted,
            previous_location,
            previous_mutable_value,
        ) = loop {
            let previous_live = previous.as_ref().is_some_and(|located| {
                located.item.is_live_at(unix_time_ms())
                    && self
                        .table
                        .candidate_locations(&storage_key)
                        .contains(&located.table_location)
            });
            if !set_condition_allows(options.condition, previous_live) {
                if let Some(previous) = previous {
                    self.remove_expired_item(&storage_key, previous)?;
                }
                return Ok(SetOutcome::NotStored);
            }
            let previous_counted = previous.as_ref().is_some_and(|located| {
                !located.item.is_tombstone
                    && self
                        .table
                        .candidate_locations(&storage_key)
                        .contains(&located.table_location)
            });
            let previous_location = previous.as_ref().map(|located| located.table_location);
            let previous_mutable_value = previous
                .as_ref()
                .and_then(|located| self.mutable_value_handle(located));
            if let Some(location) = self.try_append_value(
                storage_key,
                &mut value,
                options.ttl_ms(),
                options.eviction_protected(),
                previous_location,
                previous_mutable_value,
            )? {
                break (
                    location,
                    previous_live,
                    previous_counted,
                    previous_location,
                    previous_mutable_value,
                );
            }
            let lane = self.fullest_mutable_lane()?;
            self.flush_lane(lane, SegmentFlushReason::Capacity).await?;
            // Capacity work can evict or expire the item observed above. Refresh
            // the state before retrying so conditional SET semantics are
            // evaluated at the eventual mutation boundary.
            previous = self.locate_item(&storage_key).await?;
        };
        let previous_disappeared = self.publish_table_location(
            storage_key,
            previous_location,
            previous_mutable_value,
            new_location,
        )?;
        if !previous_counted || previous_disappeared {
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
    ///
    /// This mutable store borrow is the atomicity boundary. Callers cannot
    /// interleave another write between comparison and replacement.
    pub(crate) async fn compare_and_set_encoded_with_options(
        &mut self,
        storage_key: StorageKey,
        expected: Option<&[u8]>,
        replacement: Option<StoredItemValue>,
        options: StorageWriteOptions,
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
            self.remove_expired_item(storage_key, previous)?;
            return Ok(false);
        }
        let previous_location = previous.table_location;
        let previous_mutable_value = self.mutable_value_handle(&previous);
        self.remove_table_location(*storage_key, previous_location, previous_mutable_value)?;
        self.live_keys = self.live_keys.saturating_sub(1);
        Ok(true)
    }

    fn remove_expired_item(
        &mut self,
        storage_key: &StorageKey,
        previous: super::LocatedItem,
    ) -> Result<()> {
        if !previous.item.is_expired_at(unix_time_ms())
            || !self
                .table
                .candidate_locations(storage_key)
                .contains(&previous.table_location)
        {
            return Ok(());
        }
        let previous_mutable_value = self.mutable_value_handle(&previous);
        self.remove_table_location(
            *storage_key,
            previous.table_location,
            previous_mutable_value,
        )?;
        self.live_keys = self.live_keys.saturating_sub(1);
        Ok(())
    }
}
