//! SSD Bucket reads and collision-safe circular Segment reuse.

use std::collections::HashSet;
use std::time::Duration;

use compio::BufResult;
use compio::io::AsyncReadAt;

use crate::*;

impl Kvkache {
    pub(super) async fn read_bucket(
        &self,
        sg_index: usize,
        bucket_index: usize,
    ) -> Result<DirectIoBuffer> {
        let offset =
            self.config.segment_data_offset(sg_index) + bucket_index as u64 * BUCKET_BYTES as u64;
        let read = self
            .data
            .read_at(DirectIoBuffer::for_read(BUCKET_BYTES), offset);
        let BufResult(result, bytes) = compio::runtime::time::timeout(
            Duration::from_micros(self.config.read_max_time_us),
            read,
        )
        .await
        .map_err(|_| KvError::Timeout("Bucket read"))?;
        require_complete_direct_io("Bucket read", result?, BUCKET_BYTES)?;
        self.io
            .data_read
            .set(self.io.data_read.get() + bytes.len() as u64);
        Ok(bytes)
    }

    pub(super) async fn read_segment_items(
        &self,
        sg_index: usize,
    ) -> Result<Vec<(Item, TableLocation)>> {
        let mut result = Vec::new();
        for bucket_index in 0..self.config.bucket_count() {
            let bytes = self.read_bucket(sg_index, bucket_index).await?;
            result.extend(items(&bytes).into_iter().filter_map(|item| {
                let bucket_hash_index = bucket_hash_index_for_bucket(
                    &item.storage_key,
                    bucket_index,
                    self.config.bucket_count(),
                    self.config.bucket_choice_count,
                )?;
                Some((
                    item,
                    TableLocation {
                        sg_index: sg_index as u16,
                        bucket_hash_index,
                    },
                ))
            }));
        }
        Ok(result)
    }

    pub(super) async fn prepare_segment_for_reuse(
        &mut self,
        sg_index: usize,
    ) -> Result<HashSet<StorageKey>> {
        let mut latest = Vec::new();
        for (item, table_location) in self.read_segment_items(sg_index).await? {
            let is_latest = self
                .locate_stable_record(&item.storage_key)
                .await?
                .is_some_and(|located| {
                    located.table_location == table_location && located.item == item
                });
            if is_latest {
                latest.push((item, table_location));
            }
        }

        let mut evicted = HashSet::with_capacity(latest.len());
        for (item, table_location) in latest {
            let removed = self.table.remove(&item.storage_key, table_location);
            debug_assert!(removed);
            if !item.is_tombstone {
                self.stable_live_keys = self.stable_live_keys.saturating_sub(1);
            }
            evicted.insert(item.storage_key);
        }
        self.occupied_segments[sg_index] = false;
        self.blob_segment.release_segment(sg_index);
        self.segment_reuses += 1;
        Ok(evicted)
    }
}

fn bucket_hash_index_for_bucket(
    storage_key: &StorageKey,
    bucket_index: usize,
    bucket_count: usize,
    bucket_choice_count: usize,
) -> Option<u8> {
    (0..bucket_choice_count as u8)
        .find(|&index| bucket_hash(storage_key, index, bucket_count) == bucket_index)
}
