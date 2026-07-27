//! SSD Bucket reads and circular Segment reuse.

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
        let offset = sg_index as u64 * self.config.segment_size as u64
            + bucket_index as u64 * BUCKET_BYTES as u64;
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

    async fn read_segment_items(&self, sg_index: usize) -> Result<Vec<(Item, TableLocation)>> {
        let mut result = Vec::new();
        for bucket_index in 0..self.config.bucket_count() {
            let bytes = self.read_bucket(sg_index, bucket_index).await?;
            result.extend(items(&bytes).into_iter().filter_map(|item| {
                let bucket_hash_index = bucket_hash_index_for_bucket(
                    &item.storage_key,
                    bucket_index,
                    self.config.bucket_count(),
                )?;
                Some((
                    item,
                    TableLocation {
                        is_blob: false,
                        sg_index: sg_index as u16,
                        bucket_hash_index,
                    },
                ))
            }));
        }
        Ok(result)
    }

    pub(super) async fn prepare_segment_for_reuse(&mut self, sg_index: usize) -> Result<()> {
        if self.regular_segment_occupied[sg_index] {
            for (item, table_location) in self.read_segment_items(sg_index).await? {
                let is_latest = self
                    .locate(&item.storage_key)
                    .await?
                    .is_some_and(|located| {
                        located.table_location == table_location && located.item == item
                    });
                if is_latest {
                    let removed = self.table.remove(&item.storage_key, table_location);
                    debug_assert!(removed);
                }
            }
        }
        let blob_logical_len = self.blob_segment_used_bytes[sg_index];
        if blob_logical_len != 0 {
            let (blob_refs, bytes_read) = self
                .blob_segment
                .read_segment_refs(sg_index, blob_logical_len)
                .await?;
            self.io.data_read.set(self.io.data_read.get() + bytes_read);
            for (storage_key, blob_ref) in blob_refs {
                if self.blob_refs.get(&storage_key) == Some(&blob_ref) {
                    let removed = self.table.remove(&storage_key, TableLocation::blob());
                    debug_assert!(removed);
                    self.blob_refs.remove(&storage_key);
                }
            }
        }
        self.regular_segment_occupied[sg_index] = false;
        self.blob_segment_used_bytes[sg_index] = 0;
        self.occupied_segments[sg_index] = false;
        self.segment_reuses += 1;
        Ok(())
    }
}

fn bucket_hash_index_for_bucket(
    storage_key: &StorageKey,
    bucket_index: usize,
    bucket_count: usize,
) -> Option<u8> {
    if bucket_hash(storage_key, 0, bucket_count) == bucket_index {
        Some(0)
    } else if bucket_hash(storage_key, 1, bucket_count) == bucket_index {
        Some(1)
    } else {
        None
    }
}
