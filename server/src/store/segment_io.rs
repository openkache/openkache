//! SSD Bucket reads and collision-safe circular Segment reuse.

use std::collections::HashSet;
use std::time::Duration;

use compio::BufResult;
use compio::buf::{IntoInner, IoBuf};
use compio::io::AsyncReadAt;

use super::{
    BucketHashSequence, DirectIoBuffer, Item, KvError, Kvkache, Result, items,
    require_complete_direct_io,
};
use crate::BUCKET_BYTES;
use crate::table::TableLocation;
use crate::types::StorageKey;

const SEGMENT_READ_EXTENT_BYTES: usize = 1024 * 1024;

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
        let mut extent_offset = 0usize;
        let mut buffer =
            DirectIoBuffer::for_read(SEGMENT_READ_EXTENT_BYTES.min(self.config.segment_size));
        while extent_offset < self.config.segment_size {
            let extent_bytes =
                SEGMENT_READ_EXTENT_BYTES.min(self.config.segment_size - extent_offset);
            let offset = self.config.segment_data_offset(sg_index) + extent_offset as u64;
            let read = self.data.read_at(buffer.slice(..extent_bytes), offset);
            let BufResult(read_result, bytes) = compio::runtime::time::timeout(
                Duration::from_micros(self.config.read_max_time_us),
                read,
            )
            .await
            .map_err(|_| KvError::Timeout("Segment extent read"))?;
            require_complete_direct_io("Segment extent read", read_result?, extent_bytes)?;
            self.io
                .data_read
                .set(self.io.data_read.get() + bytes.len() as u64);

            for bucket_offset in (0..extent_bytes).step_by(BUCKET_BYTES) {
                let bucket_index = (extent_offset + bucket_offset) / BUCKET_BYTES;
                let bucket = &bytes[bucket_offset..bucket_offset + BUCKET_BYTES];
                result.extend(items(bucket).filter_map(|item| {
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
            buffer = bytes.into_inner();
            extent_offset += extent_bytes;
        }
        Ok(result)
    }

    pub(super) async fn prepare_segment_for_reuse(
        &mut self,
        sg_index: usize,
    ) -> Result<HashSet<StorageKey>> {
        let mut latest = self.read_segment_items(sg_index).await?;
        let mut index = 0;
        while index < latest.len() {
            let (item, table_location) = &latest[index];
            let is_latest = self
                .locate_stable_record(&item.storage_key)
                .await?
                .is_some_and(|located| {
                    located.table_location == *table_location && located.item == *item
                });
            if is_latest {
                index += 1;
            } else {
                latest.swap_remove(index);
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
    let hashes = BucketHashSequence::new(storage_key, bucket_count);
    (0..bucket_choice_count as u8).find(|&index| hashes.get(index) == bucket_index)
}
