//! Keyed snapshot selection and zero-copy value projection.
//!
//! Candidate lookup and extent reads remain in sibling modules. This module
//! selects the newest immutable candidate and projects stable owners without
//! copying their payloads.

use std::rc::Rc;
use std::sync::Arc;

use crate::storage_runtime::File;
use crate::types::StoredItemValue;
use crate::{BUCKET_BYTES, Config, KvError, Result, StorageKey};

use super::policy::item_state_is_live_now;
use super::{
    DirectIoBuffer, DirectIoBufferLease, DirectStoreIo, INLINE_VALUE_TAG, Item, ItemState, JobPin,
    MutableValueHandle, RamBacking, STORED_VALUE_TAG_BYTES, SsdBacking, StoredValue, TableLocation,
    bucket_hash, decode_stored_value, find_item_state_and_value_range, read_exact_direct,
};

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

#[derive(Clone, Copy)]
pub(super) enum ReadPurpose {
    Value,
    State,
}

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

enum ObservedValue {
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

struct ObservedItem {
    state: ItemState,
    value: ObservedValue,
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
                    let bytes = super::value_reads::read_ram_value(encoded, &backing)?;
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
                    super::value_reads::read_ssd_value(
                        data,
                        large_values,
                        config,
                        &backing,
                        &mut encoded,
                        io,
                    )
                    .await
                }
                ObservedValue::None | ObservedValue::RamInline { .. } => Err(KvError::Worker(
                    "SSD keyed read has no value snapshot".into(),
                )),
            },
        }
    }
}
