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
        for (item, table_location) in self.read_segment_items(sg_index).await? {
            // TODO(storage-table-refactor): Table entries are probabilistic.
            // Exact SSD Item liveness must be added before a colliding stale
            // Item can be distinguished from a live entry at the same
            // candidate location.
            let _ = self.table.remove(&item.storage_key, table_location);
        }
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
