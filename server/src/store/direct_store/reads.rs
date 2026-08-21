//! Candidate lookup and backend-independent value reads.

use crate::types::StoredItemValue;
use crate::{BUCKET_BYTES, KvError, Result, StorageKey};

use super::value_reads::{read_arena_value, read_owned_extent};
use super::{
    BlobHandle, BlobRef, GenerationLocation, Item, Kvkache, LargeValueLocation, LocatedItem,
    ReadBacking, StoredValue, TableLocation, bucket_hash, decode_stored_value, find_item_in_bucket,
    read_exact_direct, remove_stored_value_tag,
};

impl Kvkache {
    pub(super) async fn locate_item(
        &self,
        storage_key: &StorageKey,
    ) -> Result<Option<LocatedItem>> {
        let mut newest = None;
        for table_location in self.table.candidate_locations(storage_key) {
            let Some(backing) = self.directory.read_backing(table_location.sg_index) else {
                continue;
            };
            let sequence = match &backing {
                ReadBacking::Mutable { lane, .. } => self.mutable[*lane]
                    .as_ref()
                    .map_or(0, |generation| generation.sequence),
                ReadBacking::Ram { backing, .. } => backing.sequence,
                ReadBacking::Ssd(backing) => backing.sequence,
            };
            let Some(item) = self
                .read_candidate(storage_key, table_location, &backing)
                .await?
            else {
                continue;
            };
            if newest
                .as_ref()
                .is_none_or(|located: &LocatedItem| sequence > located.sequence)
            {
                newest = Some(LocatedItem {
                    table_location,
                    item,
                    backing,
                    sequence,
                });
            }
        }
        Ok(newest)
    }

    async fn read_candidate(
        &self,
        storage_key: &StorageKey,
        table_location: TableLocation,
        backing: &ReadBacking,
    ) -> Result<Option<Item>> {
        let bucket_index = bucket_hash(
            storage_key,
            table_location.bucket_hash_index,
            self.config.bucket_count(),
        );
        match backing {
            ReadBacking::Mutable { lane, .. } => {
                let Some(generation) = self.mutable[*lane].as_ref() else {
                    return Ok(None);
                };
                let start = bucket_index * BUCKET_BYTES;
                Ok(find_item_in_bucket(
                    &generation.segment.bytes[start..start + BUCKET_BYTES],
                    storage_key,
                ))
            }
            ReadBacking::Ram { backing, .. } => {
                let start = bucket_index * BUCKET_BYTES;
                Ok(find_item_in_bucket(
                    &backing.segment[start..start + BUCKET_BYTES],
                    storage_key,
                ))
            }
            ReadBacking::Ssd(backing) => {
                let bytes = read_exact_direct(
                    &self.data,
                    self.io.bucket_read_pool.take_bucket().await,
                    backing.location.sg_base + (bucket_index * BUCKET_BYTES) as u64,
                    BUCKET_BYTES,
                    self.config.read_max_time_us,
                    "generation Bucket read",
                )
                .await?;
                self.io
                    .data_read
                    .set(self.io.data_read.get() + BUCKET_BYTES as u64);
                let item = find_item_in_bucket(&bytes, storage_key);
                Ok(item)
            }
        }
    }

    pub(super) async fn read_value(
        &self,
        mut encoded: Vec<u8>,
        backing: ReadBacking,
    ) -> Result<StoredItemValue> {
        match backing {
            ReadBacking::Mutable { lane, .. } => match decode_stored_value(&encoded)? {
                StoredValue::Inline(_) => {
                    remove_stored_value_tag(&mut encoded);
                    Ok(StoredItemValue::new(encoded))
                }
                StoredValue::Blob(blob_ref) => self.mutable[lane]
                    .as_ref()
                    .and_then(|generation| {
                        generation.blob_arena.get_value(BlobHandle {
                            slot: blob_ref.value_offset,
                            value_len: blob_ref.value_len,
                        })
                    })
                    .ok_or_else(|| KvError::Worker("mutable Blob handle is invalid".into())),
                StoredValue::Large(value_ref) => self.mutable[lane]
                    .as_ref()
                    .and_then(|generation| {
                        generation.large_value_arena.get_value(BlobHandle {
                            slot: value_ref.value_offset,
                            value_len: value_ref.value_len,
                        })
                    })
                    .ok_or_else(|| KvError::Worker("mutable large-value handle is invalid".into())),
            },
            ReadBacking::Ram { backing, .. } => read_arena_value(
                encoded,
                &backing.blob_arena,
                &backing.large_value_arena,
                "sealed Blob handle is invalid",
                "sealed large-value handle is invalid",
            ),
            ReadBacking::Ssd(backing) => match decode_stored_value(&encoded)? {
                StoredValue::Inline(value) => Ok(StoredItemValue::new(value.to_vec())),
                StoredValue::Blob(blob_ref) => self
                    .read_blob(&backing.location, blob_ref)
                    .await
                    .map(StoredItemValue::new),
                StoredValue::Large(value_ref) => {
                    let location = backing.large_value_location.as_ref().ok_or_else(|| {
                        KvError::Worker("large-value Item has no SSD extent".into())
                    })?;
                    self.read_large_value(location, value_ref)
                        .await
                        .map(StoredItemValue::new)
                }
            },
        }
    }

    async fn read_blob(&self, location: &GenerationLocation, blob_ref: BlobRef) -> Result<Vec<u8>> {
        read_owned_extent(
            &self.data,
            location.record_start,
            location.blob_logical_len,
            blob_ref,
            self.config.read_max_time_us,
            "generation Blob read",
            "BlobRef exceeds its generation Blob extent",
            "BlobRef range overflowed",
            "Blob direct-read extent overflowed",
            &self.io,
        )
        .await
    }

    async fn read_large_value(
        &self,
        location: &LargeValueLocation,
        value_ref: BlobRef,
    ) -> Result<Vec<u8>> {
        read_owned_extent(
            &self.large_values,
            location.record_start,
            location.logical_len,
            value_ref,
            self.config.read_max_time_us,
            "large-value read",
            "large-value ref exceeds its SSD extent",
            "large-value ref range overflowed",
            "large-value direct-read extent overflowed",
            &self.io,
        )
        .await
    }
}
