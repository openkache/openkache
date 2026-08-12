//! Read planning and snapshot materialization for the direct store.
//!
//! The parent module owns mutation/flush state. This module keeps candidate
//! lookup, backend-specific read plans, and value ownership transitions
//! together, so changing SSD/RAM read policy does not enlarge the mutation
//! coordinator or duplicate extent handling in the mutation file.

use std::rc::Rc;
use std::sync::Arc;

use crate::storage_runtime::File;
use crate::types::StoredItemValue;
use crate::{BUCKET_BYTES, Config, KvError, Result, StorageKey};

use super::direct_store_mutations::{KeyedObservation, LocatedKeyState, ReadPurpose};
use super::direct_store_policy::item_state_is_live_now;
use super::{
    BlobArena, BlobHandle, BlobRef, DirectIoBuffer, DirectIoBufferLease, DirectStoreIo,
    GenerationLocation, INLINE_VALUE_TAG, Item, ItemState, JobPin, Kvkache, LargeValueLocation,
    LocatedItem, MAX_LEASED_SSD_VALUE_READ_BYTES, MutableGeneration, MutableValueHandle,
    RamBacking, ReadBacking, STORED_VALUE_TAG_BYTES, SsdBacking, StoredValue, TableLocation,
    bucket_hash, decode_stored_value, find_item_in_bucket, find_item_state_and_value_range,
    read_exact_direct, remove_stored_value_tag,
};

pub(super) enum PreparedReadBacking {
    Mutable {
        item: Option<Item>,
        value: Option<Result<StoredItemValue>>,
        mutable_value: Option<MutableValueHandle>,
    },
    Ram {
        backing: Rc<RamBacking>,
        _retirement_guard: Option<Rc<SsdBacking>>,
    },
    Ssd(Rc<SsdBacking>),
}

pub(super) struct PreparedReadCandidate {
    pub(super) table_location: TableLocation,
    pub(super) sequence: u64,
    pub(super) backing: PreparedReadBacking,
}

pub(super) enum ObservedValue {
    None,
    Encoded(Vec<u8>),
    OwnedInline(Vec<u8>),
    DirectInline {
        buffer: DirectIoBufferLease,
        range: std::ops::Range<usize>,
    },
    RamInline {
        segment: Arc<DirectIoBuffer>,
        range: std::ops::Range<usize>,
    },
}

pub(super) struct ObservedItem {
    pub(super) state: ItemState,
    pub(super) value: ObservedValue,
}

pub(super) struct DirectReadPlan {
    pub(super) data: File,
    pub(super) large_values: File,
    pub(super) config: Config,
    pub(super) storage_key: StorageKey,
    pub(super) candidates: Vec<PreparedReadCandidate>,
    pub(super) io: Rc<DirectStoreIo>,
    pub(super) _job_pins: Vec<JobPin>,
}

impl DirectReadPlan {
    pub(super) async fn read(self, purpose: ReadPurpose) -> Result<KeyedObservation> {
        let mut newest = None;
        for candidate in self.candidates {
            let item = candidate
                .read_item(
                    &self.data,
                    &self.config,
                    self.storage_key,
                    purpose,
                    &self.io,
                )
                .await?;
            let Some(item) = item else {
                continue;
            };
            if newest
                .as_ref()
                .is_none_or(|(current, _): &(PreparedReadCandidate, ObservedItem)| {
                    candidate.sequence > current.sequence
                })
            {
                newest = Some((candidate, item));
            }
        }
        let Some((candidate, item)) = newest else {
            return Ok(match purpose {
                ReadPurpose::Value => KeyedObservation::Value(None),
                ReadPurpose::State => KeyedObservation::State(None),
            });
        };
        match purpose {
            ReadPurpose::Value => {
                if !item_state_is_live_now(item.state) {
                    return Ok(KeyedObservation::Value(None));
                }
                candidate
                    .read_value(
                        &self.data,
                        &self.large_values,
                        &self.config,
                        item.value,
                        &self.io,
                    )
                    .await
                    .map(|value| KeyedObservation::Value(Some(value)))
            }
            ReadPurpose::State => Ok(KeyedObservation::State(Some(LocatedKeyState {
                table_location: candidate.table_location,
                item_state: item.state,
                mutable_value: candidate.mutable_value(),
            }))),
        }
    }
}

impl PreparedReadCandidate {
    async fn read_item(
        &self,
        data: &File,
        config: &Config,
        storage_key: StorageKey,
        purpose: ReadPurpose,
        io: &DirectStoreIo,
    ) -> Result<Option<ObservedItem>> {
        match &self.backing {
            PreparedReadBacking::Mutable { item, .. } => {
                Ok(item.as_ref().map(|item| ObservedItem {
                    state: ItemState {
                        is_tombstone: item.is_tombstone,
                        expires_at_ms: item.expires_at_ms,
                        eviction_protected: item.eviction_protected,
                    },
                    value: ObservedValue::None,
                }))
            }
            PreparedReadBacking::Ram { backing, .. } => {
                let bucket_index = bucket_hash(
                    &storage_key,
                    self.table_location.bucket_hash_index,
                    config.bucket_count(),
                );
                let start = bucket_index * BUCKET_BYTES;
                let bucket = &backing.segment[start..start + BUCKET_BYTES];
                let Some((state, range)) = find_item_state_and_value_range(bucket, &storage_key)
                else {
                    return Ok(None);
                };
                let range = start + range.start..start + range.end;
                let value = match purpose {
                    ReadPurpose::State => ObservedValue::None,
                    ReadPurpose::Value => {
                        match decode_stored_value(&backing.segment[range.clone()])? {
                            StoredValue::Inline(_) => ObservedValue::RamInline {
                                segment: Arc::clone(&backing.segment),
                                range: range.start + STORED_VALUE_TAG_BYTES..range.end,
                            },
                            StoredValue::Blob(_) | StoredValue::Large(_) => {
                                ObservedValue::Encoded(backing.segment[range].to_vec())
                            }
                        }
                    }
                };
                Ok(Some(ObservedItem { state, value }))
            }
            PreparedReadBacking::Ssd(backing) => {
                let bucket_index = bucket_hash(
                    &storage_key,
                    self.table_location.bucket_hash_index,
                    config.bucket_count(),
                );
                let bytes = read_exact_direct(
                    data,
                    io.bucket_read_pool.take_bucket().await,
                    backing.location.sg_base + (bucket_index * BUCKET_BYTES) as u64,
                    BUCKET_BYTES,
                    config.read_max_time_us,
                    "generation Bucket read",
                )
                .await?;
                io.data_read.set(io.data_read.get() + BUCKET_BYTES as u64);
                let observed =
                    find_item_state_and_value_range(&bytes, &storage_key).map(|(state, range)| {
                        let value = match purpose {
                            ReadPurpose::State => ObservedValue::None,
                            ReadPurpose::Value
                                if config.lease_ssd_read_buffer
                                    && bytes.get(range.start) == Some(&INLINE_VALUE_TAG) =>
                            {
                                ObservedValue::DirectInline {
                                    buffer: bytes,
                                    range: range.start + STORED_VALUE_TAG_BYTES..range.end,
                                }
                            }
                            ReadPurpose::Value
                                if config.copy_ssd_inline_value_once
                                    && bytes.get(range.start) == Some(&INLINE_VALUE_TAG) =>
                            {
                                ObservedValue::OwnedInline(
                                    bytes[range.start + STORED_VALUE_TAG_BYTES..range.end].to_vec(),
                                )
                            }
                            ReadPurpose::Value => ObservedValue::Encoded(bytes[range].to_vec()),
                        };
                        ObservedItem { state, value }
                    });
                Ok(observed)
            }
        }
    }

    fn mutable_value(&self) -> Option<MutableValueHandle> {
        match &self.backing {
            PreparedReadBacking::Mutable { mutable_value, .. } => *mutable_value,
            PreparedReadBacking::Ram { .. } | PreparedReadBacking::Ssd(_) => None,
        }
    }

    async fn read_value(
        self,
        data: &File,
        large_values: &File,
        config: &Config,
        value: ObservedValue,
        io: &DirectStoreIo,
    ) -> Result<StoredItemValue> {
        match self.backing {
            PreparedReadBacking::Mutable { value, .. } => value.ok_or_else(|| {
                KvError::Worker("mutable keyed read has no value snapshot".into())
            })?,
            PreparedReadBacking::Ram { backing, .. } => match value {
                ObservedValue::RamInline { segment, range } => {
                    Ok(StoredItemValue::from_segment(segment, range))
                }
                ObservedValue::Encoded(encoded) => {
                    let bytes = read_ram_value(encoded, &backing)?;
                    Ok(StoredItemValue::new(bytes))
                }
                ObservedValue::OwnedInline(_) => Err(KvError::Worker(
                    "RAM keyed read has an incompatible owned inline snapshot".into(),
                )),
                ObservedValue::DirectInline { .. } => Err(KvError::Worker(
                    "RAM keyed read has an incompatible direct-read snapshot".into(),
                )),
                ObservedValue::None => Err(KvError::Worker(
                    "RAM keyed read has no value snapshot".into(),
                )),
            },
            PreparedReadBacking::Ssd(backing) => match value {
                ObservedValue::OwnedInline(bytes) => Ok(StoredItemValue::new(bytes)),
                ObservedValue::DirectInline { buffer, range } => {
                    Ok(StoredItemValue::from_direct_read(buffer, range))
                }
                ObservedValue::Encoded(mut encoded) => {
                    read_ssd_value(data, large_values, config, &backing, &mut encoded, io).await
                }
                ObservedValue::None | ObservedValue::RamInline { .. } => Err(KvError::Worker(
                    "SSD keyed read has no value snapshot".into(),
                )),
            },
        }
    }
}

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
        encoded: Vec<u8>,
        backing: ReadBacking,
    ) -> Result<Vec<u8>> {
        match backing {
            ReadBacking::Mutable { lane, .. } => {
                let Some(generation) = self.mutable[lane].as_ref() else {
                    return Err(KvError::Worker("mutable SG is unavailable".into()));
                };
                read_arena_value(
                    encoded,
                    &generation.blob_arena,
                    &generation.large_value_arena,
                    "mutable Blob handle is invalid",
                    "mutable large-value handle is invalid",
                )
            }
            ReadBacking::Ram { backing, .. } => read_arena_value(
                encoded,
                &backing.blob_arena,
                &backing.large_value_arena,
                "sealed Blob handle is invalid",
                "sealed large-value handle is invalid",
            ),
            ReadBacking::Ssd(backing) => match decode_stored_value(&encoded)? {
                StoredValue::Inline(value) => Ok(value.to_vec()),
                StoredValue::Blob(blob_ref) => self.read_blob(&backing.location, blob_ref).await,
                StoredValue::Large(value_ref) => {
                    let location = backing.large_value_location.as_ref().ok_or_else(|| {
                        KvError::Worker("large-value Item has no SSD extent".into())
                    })?;
                    self.read_large_value(location, value_ref).await
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
            "large-value direct-read extent overflowed",
            &self.io,
        )
        .await
    }
}

async fn read_owned_extent(
    file: &File,
    record_start: u64,
    logical_len: u32,
    value_ref: BlobRef,
    read_max_time_us: u64,
    operation: &'static str,
    invalid_range: &'static str,
    overflow: &'static str,
    io: &DirectStoreIo,
) -> Result<Vec<u8>> {
    let logical_end = u64::from(value_ref.value_offset)
        .checked_add(u64::from(value_ref.value_len))
        .ok_or_else(|| KvError::Worker("stored-value range overflowed".into()))?;
    if logical_end > u64::from(logical_len) {
        return Err(KvError::Worker(invalid_range.into()));
    }
    if value_ref.value_len == 0 {
        return Ok(Vec::new());
    }
    let absolute = record_start + u64::from(value_ref.value_offset);
    let aligned_start = absolute / BUCKET_BYTES as u64 * BUCKET_BYTES as u64;
    let prefix = (absolute - aligned_start) as usize;
    let read_len = prefix
        .checked_add(value_ref.value_len as usize)
        .and_then(|len| len.checked_next_multiple_of(BUCKET_BYTES))
        .ok_or_else(|| KvError::Worker(overflow.into()))?;
    let bytes = read_exact_direct(
        file,
        DirectIoBuffer::for_read(read_len),
        aligned_start,
        read_len,
        read_max_time_us,
        operation,
    )
    .await?;
    io.data_read.set(io.data_read.get() + read_len as u64);
    Ok(bytes[prefix..prefix + value_ref.value_len as usize].to_vec())
}

pub(super) fn read_mutable_value(
    encoded: Vec<u8>,
    generation: &MutableGeneration,
) -> Result<Vec<u8>> {
    read_arena_value(
        encoded,
        &generation.blob_arena,
        &generation.large_value_arena,
        "mutable Blob handle is invalid",
        "mutable large-value handle is invalid",
    )
}

fn read_arena_value(
    mut encoded: Vec<u8>,
    blob_arena: &BlobArena,
    large_value_arena: &BlobArena,
    invalid_blob: &'static str,
    invalid_large_value: &'static str,
) -> Result<Vec<u8>> {
    match decode_stored_value(&encoded)? {
        StoredValue::Inline(_) => {
            remove_stored_value_tag(&mut encoded);
            Ok(encoded)
        }
        StoredValue::Blob(blob_ref) => blob_arena
            .get(BlobHandle {
                slot: blob_ref.value_offset,
                value_len: blob_ref.value_len,
            })
            .map(ToOwned::to_owned)
            .ok_or_else(|| KvError::Worker(invalid_blob.into())),
        StoredValue::Large(value_ref) => large_value_arena
            .get(BlobHandle {
                slot: value_ref.value_offset,
                value_len: value_ref.value_len,
            })
            .map(ToOwned::to_owned)
            .ok_or_else(|| KvError::Worker(invalid_large_value.into())),
    }
}

fn read_ram_value(encoded: Vec<u8>, backing: &RamBacking) -> Result<Vec<u8>> {
    read_arena_value(
        encoded,
        &backing.blob_arena,
        &backing.large_value_arena,
        "sealed Blob handle is invalid",
        "sealed large-value handle is invalid",
    )
}

async fn read_ssd_value(
    data: &File,
    large_values: &File,
    config: &Config,
    backing: &SsdBacking,
    encoded: &mut Vec<u8>,
    io: &DirectStoreIo,
) -> Result<StoredItemValue> {
    match decode_stored_value(encoded)? {
        StoredValue::Inline(value) => Ok(StoredItemValue::new(value.to_vec())),
        StoredValue::Blob(blob_ref) => {
            let logical_end = u64::from(blob_ref.value_offset)
                .checked_add(u64::from(blob_ref.value_len))
                .ok_or_else(|| KvError::Worker("BlobRef range overflowed".into()))?;
            if logical_end > u64::from(backing.location.blob_logical_len) {
                return Err(KvError::Worker(
                    "BlobRef exceeds its generation Blob extent".into(),
                ));
            }
            read_ssd_extent(
                data,
                backing.location.record_start,
                blob_ref,
                config.read_max_time_us,
                "generation Blob read",
                io,
                config.lease_ssd_read_buffer,
            )
            .await
        }
        StoredValue::Large(value_ref) => {
            let location = backing
                .large_value_location
                .as_ref()
                .ok_or_else(|| KvError::Worker("large-value Item has no SSD extent".into()))?;
            let logical_end = u64::from(value_ref.value_offset)
                .checked_add(u64::from(value_ref.value_len))
                .ok_or_else(|| KvError::Worker("large-value ref range overflowed".into()))?;
            if logical_end > u64::from(location.logical_len) {
                return Err(KvError::Worker(
                    "large-value ref exceeds its SSD extent".into(),
                ));
            }
            read_ssd_extent(
                large_values,
                location.record_start,
                value_ref,
                config.read_max_time_us,
                "large-value read",
                io,
                config.lease_ssd_read_buffer,
            )
            .await
        }
    }
}

async fn read_ssd_extent(
    file: &File,
    record_start: u64,
    value_ref: BlobRef,
    read_max_time_us: u64,
    operation: &'static str,
    io: &DirectStoreIo,
    lease_response: bool,
) -> Result<StoredItemValue> {
    if value_ref.value_len == 0 {
        return Ok(StoredItemValue::new(Vec::new()));
    }
    let absolute = record_start + u64::from(value_ref.value_offset);
    let aligned_start = absolute / BUCKET_BYTES as u64 * BUCKET_BYTES as u64;
    let prefix = (absolute - aligned_start) as usize;
    let read_len = prefix
        .checked_add(value_ref.value_len as usize)
        .and_then(|len| len.checked_next_multiple_of(BUCKET_BYTES))
        .ok_or_else(|| KvError::Worker("direct-read extent overflowed".into()))?;
    if lease_response && read_len <= MAX_LEASED_SSD_VALUE_READ_BYTES {
        let bytes = read_exact_direct(
            file,
            io.value_read_pool.take_buffer(read_len).await,
            aligned_start,
            read_len,
            read_max_time_us,
            operation,
        )
        .await?;
        io.data_read.set(io.data_read.get() + read_len as u64);
        return Ok(StoredItemValue::from_direct_read(
            bytes,
            prefix..prefix + value_ref.value_len as usize,
        ));
    }
    let bytes = read_exact_direct(
        file,
        DirectIoBuffer::for_read(read_len),
        aligned_start,
        read_len,
        read_max_time_us,
        operation,
    )
    .await?;
    io.data_read.set(io.data_read.get() + read_len as u64);
    Ok(StoredItemValue::new(
        bytes[prefix..prefix + value_ref.value_len as usize].to_vec(),
    ))
}
