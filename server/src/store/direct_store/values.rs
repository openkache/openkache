//! Mutable value-tier placement and replacement helpers.
//!
//! These helpers know about the in-memory segment/blob representation only.
//! They deliberately do not know about API operations, protocol framing, or
//! capacity eviction policy.

use crate::types::StoredItemValue;
use crate::{KvError, Result, StorageKey};

use super::policy::ttl_deadline;
use super::{
    BlobHandle, Item, MutableGeneration, MutableValueHandle, StoredValue, TableLocation,
    bucket_hash, decode_stored_value, encode_blob_handle, encode_inline_value,
    encode_large_value_handle,
};

pub(super) fn mutable_value_handle_for(
    lane: usize,
    logical_sg_id: u32,
    encoded: &[u8],
) -> Option<MutableValueHandle> {
    match decode_stored_value(encoded).ok()? {
        StoredValue::Inline(_) => None,
        StoredValue::Blob(blob_ref) => Some(MutableValueHandle::Blob {
            lane,
            logical_sg_id,
            handle: BlobHandle {
                slot: blob_ref.value_offset,
                value_len: blob_ref.value_len,
                value_checksum: blob_ref.value_checksum,
            },
        }),
        StoredValue::Large(value_ref) => Some(MutableValueHandle::Large {
            lane,
            logical_sg_id,
            handle: BlobHandle {
                slot: value_ref.value_offset,
                value_len: value_ref.value_len,
                value_checksum: value_ref.value_checksum,
            },
        }),
    }
}

pub(super) enum InPlaceValue {
    Replaced(Option<MutableValueHandle>),
    NotReplaced,
}

pub(super) struct StagedMutableValue {
    pub(super) encoded: Vec<u8>,
    pub(super) mutable_value: Option<MutableValueHandle>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn try_replace_value_in_place(
    generation: &mut MutableGeneration,
    lane: usize,
    previous_location: TableLocation,
    storage_key: StorageKey,
    value: &mut StoredItemValue,
    ttl_ms: Option<u64>,
    eviction_protected: bool,
    large: bool,
    blob: bool,
    previous: Option<MutableValueHandle>,
) -> Result<InPlaceValue> {
    let value_len = u32::try_from(value.len())
        .map_err(|_| KvError::Usage("mutable value length does not fit in u32".into()))?;
    let same_slot = match previous {
        Some(MutableValueHandle::Blob {
            lane: previous_lane,
            logical_sg_id,
            handle,
        }) if previous_lane == lane
            && logical_sg_id == generation.logical_sg_id
            && blob
            && generation.blob_arena.can_replace(handle, value.len()) =>
        {
            Some((
                MutableValueHandle::Blob {
                    lane,
                    logical_sg_id,
                    handle: BlobHandle {
                        slot: handle.slot,
                        value_len,
                        value_checksum: crc32fast::hash(value.as_ref()),
                    },
                },
                encode_blob_handle(BlobHandle {
                    slot: handle.slot,
                    value_len,
                    value_checksum: crc32fast::hash(value.as_ref()),
                }),
            ))
        }
        Some(MutableValueHandle::Large {
            lane: previous_lane,
            logical_sg_id,
            handle,
        }) if previous_lane == lane
            && logical_sg_id == generation.logical_sg_id
            && large
            && generation
                .large_value_arena
                .can_replace(handle, value.len()) =>
        {
            Some((
                MutableValueHandle::Large {
                    lane,
                    logical_sg_id,
                    handle: BlobHandle {
                        slot: handle.slot,
                        value_len,
                        value_checksum: crc32fast::hash(value.as_ref()),
                    },
                },
                encode_large_value_handle(BlobHandle {
                    slot: handle.slot,
                    value_len,
                    value_checksum: crc32fast::hash(value.as_ref()),
                }),
            ))
        }
        _ => None,
    };
    if let Some((mutable_value, encoded)) = same_slot {
        // This replacement is the mutation linearization point for the
        // in-place path; resolve the relative TTL immediately before it.
        let expires_at_ms = ttl_deadline(ttl_ms)?;
        let item = live_item(storage_key, encoded, expires_at_ms, eviction_protected);
        if generation.segment.replace(previous_location, item, true) {
            match (previous, mutable_value) {
                (
                    Some(MutableValueHandle::Blob {
                        handle: previous, ..
                    }),
                    MutableValueHandle::Blob { handle, .. },
                ) => {
                    let replaced = generation.blob_arena.replace(previous, value)?;
                    debug_assert_eq!(replaced, handle);
                }
                (
                    Some(MutableValueHandle::Large {
                        handle: previous, ..
                    }),
                    MutableValueHandle::Large { handle, .. },
                ) => {
                    let replaced = generation.large_value_arena.replace(previous, value)?;
                    debug_assert_eq!(replaced, handle);
                }
                _ => unreachable!("same-slot replacement preserves the mutable value tier"),
            }
            return Ok(InPlaceValue::Replaced(Some(mutable_value)));
        }
    }

    let Some(staged) = stage_mutable_value(generation, lane, value, large, blob)? else {
        return Ok(InPlaceValue::NotReplaced);
    };
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
    if !generation.segment.replace(previous_location, item, true) {
        clear_mutable_value(generation, lane, staged.mutable_value);
        return Ok(InPlaceValue::NotReplaced);
    }
    clear_mutable_value(
        generation,
        lane,
        previous.filter(|previous| Some(*previous) != staged.mutable_value),
    );
    Ok(InPlaceValue::Replaced(staged.mutable_value))
}

pub(super) fn stage_mutable_value(
    generation: &mut MutableGeneration,
    lane: usize,
    value: &mut StoredItemValue,
    large: bool,
    blob: bool,
) -> Result<Option<StagedMutableValue>> {
    if large {
        let handle = match generation.large_value_arena.insert(value) {
            Ok(handle) => handle,
            Err(KvError::BlobSegmentFull { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let mutable_value = MutableValueHandle::Large {
            lane,
            logical_sg_id: generation.logical_sg_id,
            handle,
        };
        return Ok(Some(StagedMutableValue {
            encoded: encode_large_value_handle(handle),
            mutable_value: Some(mutable_value),
        }));
    }
    if blob {
        let handle = match generation.blob_arena.insert(value) {
            Ok(handle) => handle,
            Err(KvError::BlobSegmentFull { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let mutable_value = MutableValueHandle::Blob {
            lane,
            logical_sg_id: generation.logical_sg_id,
            handle,
        };
        return Ok(Some(StagedMutableValue {
            encoded: encode_blob_handle(handle),
            mutable_value: Some(mutable_value),
        }));
    }
    Ok(Some(StagedMutableValue {
        encoded: encode_inline_value(value.as_ref()),
        mutable_value: None,
    }))
}

pub(super) fn clear_mutable_value(
    generation: &mut MutableGeneration,
    lane: usize,
    mutable_value: Option<MutableValueHandle>,
) {
    match mutable_value {
        Some(MutableValueHandle::Blob {
            lane: value_lane,
            logical_sg_id,
            handle,
        }) if value_lane == lane && logical_sg_id == generation.logical_sg_id => {
            generation.blob_arena.remove(handle);
        }
        Some(MutableValueHandle::Large {
            lane: value_lane,
            logical_sg_id,
            handle,
        }) if value_lane == lane && logical_sg_id == generation.logical_sg_id => {
            generation.large_value_arena.remove(handle);
        }
        _ => {}
    }
}

pub(super) fn live_item(
    storage_key: StorageKey,
    encoded: Vec<u8>,
    expires_at_ms: u64,
    eviction_protected: bool,
) -> Item {
    if expires_at_ms == 0 {
        Item::live_with_eviction(storage_key, encoded, eviction_protected)
    } else {
        Item::live_expiring_with_eviction(storage_key, encoded, expires_at_ms, eviction_protected)
    }
}

pub(super) fn same_physical_bucket(
    storage_key: &StorageKey,
    first: TableLocation,
    second: TableLocation,
    bucket_count: usize,
) -> bool {
    first.sg_index == second.sg_index
        && bucket_hash(storage_key, first.bucket_hash_index, bucket_count)
            == bucket_hash(storage_key, second.bucket_hash_index, bucket_count)
}

pub(super) fn mutable_generation_for_location(
    mutable: &mut [Option<MutableGeneration>],
    location: TableLocation,
) -> Option<(usize, &mut MutableGeneration)> {
    mutable
        .iter_mut()
        .enumerate()
        .find_map(|(lane, generation)| {
            generation
                .as_mut()
                .filter(|generation| generation.logical_sg_id == location.sg_index)
                .map(|generation| (lane, generation))
        })
}
