//! Streamed SSD recovery, Bucket reads, and collision-safe Segment reuse.

use compio::buf::{IntoInner, IoBuf, Slice};

use crate::*;

const SEGMENT_READ_EXTENT_BYTES: usize = 1024 * 1024;

struct ReclaimItem {
    storage_key: StorageKey,
    table_location: TableLocation,
    is_tombstone: bool,
}

impl Kvkache {
    pub(super) async fn read_bucket(
        &self,
        sg_index: usize,
        bucket_index: usize,
    ) -> Result<DirectIoBuffer> {
        let offset =
            self.config.segment_data_offset(sg_index) + bucket_index as u64 * BUCKET_BYTES as u64;
        let bytes = read_exact_direct(
            &self.data,
            self.bucket_read_pool.take_bucket(),
            offset,
            BUCKET_BYTES,
            self.config.read_max_time_us,
            "Bucket read",
        )
        .await?;
        self.io
            .data_read
            .set(self.io.data_read.get() + bytes.len() as u64);
        Ok(bytes)
    }

    async fn read_segment_extent(
        &self,
        sg_index: usize,
        extent_offset: usize,
        buffer: DirectIoBuffer,
        operation: &'static str,
    ) -> Result<Slice<DirectIoBuffer>> {
        let extent_bytes = SEGMENT_READ_EXTENT_BYTES.min(self.config.segment_size - extent_offset);
        let offset = self.config.segment_data_offset(sg_index) + extent_offset as u64;
        let bytes = read_exact_direct(
            &self.data,
            buffer,
            offset,
            extent_bytes,
            self.config.read_max_time_us,
            operation,
        )
        .await?;
        self.io
            .data_read
            .set(self.io.data_read.get() + extent_bytes as u64);
        Ok(bytes.slice(..extent_bytes))
    }

    pub(super) async fn recover_segment(
        &mut self,
        commit: SegmentCommit,
        buffer: Option<DirectIoBuffer>,
    ) -> Result<DirectIoBuffer> {
        let mut extent_offset = 0usize;
        let mut buffer = buffer.unwrap_or_else(|| {
            DirectIoBuffer::for_read(SEGMENT_READ_EXTENT_BYTES.min(self.config.segment_size))
        });
        while extent_offset < self.config.segment_size {
            let bytes = self
                .read_segment_extent(
                    commit.sg_index,
                    extent_offset,
                    buffer,
                    "Segment recovery extent read",
                )
                .await?;
            let extent_bytes = bytes.len();

            for bucket_offset in (0..extent_bytes).step_by(BUCKET_BYTES) {
                let bucket_index = (extent_offset + bucket_offset) / BUCKET_BYTES;
                let bucket = &bytes[bucket_offset..bucket_offset + BUCKET_BYTES];
                for item in items(bucket) {
                    let Some(bucket_hash_index) = bucket_hash_index_for_bucket(
                        &item.storage_key,
                        bucket_index,
                        self.config.bucket_count(),
                        commit.bucket_choice_count,
                    ) else {
                        continue;
                    };
                    if !item.is_tombstone
                        && let StoredValue::Blob(blob_ref) = decode_stored_value(&item.value)?.value
                    {
                        validate_recovered_blob_ref(blob_ref, commit.blob_logical_len)?;
                    }
                    if self.recovered_key_exists(&item.storage_key).await? {
                        continue;
                    }
                    self.table.insert(
                        &item.storage_key,
                        TableLocation {
                            sg_index: commit.sg_index as u16,
                            bucket_hash_index,
                        },
                    )?;
                    if !item.is_tombstone {
                        self.stable_live_keys += 1;
                    }
                }
            }
            buffer = bytes.into_inner();
            extent_offset += extent_bytes;
        }
        Ok(buffer)
    }

    pub(super) async fn prepare_segment_for_reuse(&mut self, commit: SegmentCommit) -> Result<()> {
        let sg_index = commit.sg_index;
        let mut latest = Vec::new();
        let mut extent_offset = 0usize;
        let mut buffer =
            DirectIoBuffer::for_read(SEGMENT_READ_EXTENT_BYTES.min(self.config.segment_size));
        while extent_offset < self.config.segment_size {
            let bytes = self
                .read_segment_extent(sg_index, extent_offset, buffer, "Segment reuse extent read")
                .await?;
            let extent_bytes = bytes.len();

            for bucket_offset in (0..extent_bytes).step_by(BUCKET_BYTES) {
                let bucket_index = (extent_offset + bucket_offset) / BUCKET_BYTES;
                let bucket = &bytes[bucket_offset..bucket_offset + BUCKET_BYTES];
                for item in items(bucket) {
                    let Some(bucket_hash_index) = bucket_hash_index_for_bucket(
                        &item.storage_key,
                        bucket_index,
                        self.config.bucket_count(),
                        commit.bucket_choice_count,
                    ) else {
                        continue;
                    };
                    let table_location = TableLocation {
                        sg_index: sg_index as u16,
                        bucket_hash_index,
                    };
                    let is_latest = self
                        .locate_stable_record_with_bucket(
                            &item.storage_key,
                            Some((table_location, bucket)),
                        )
                        .await?
                        .is_some_and(|located| {
                            located.table_location == table_location && located.item == item
                        });
                    if is_latest {
                        latest.push(ReclaimItem {
                            storage_key: item.storage_key,
                            table_location,
                            is_tombstone: item.is_tombstone,
                        });
                    }
                }
            }
            buffer = bytes.into_inner();
            extent_offset += extent_bytes;
        }

        for item in latest {
            let removed = self.table.remove(&item.storage_key, item.table_location);
            debug_assert!(removed);
            if !item.is_tombstone {
                self.stable_live_keys = self.stable_live_keys.saturating_sub(1);
            }
        }
        self.occupied_segments[sg_index] = false;
        self.blob_segment.release_segment(sg_index);
        self.segment_reuses += 1;
        Ok(())
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
