//! Fixed 4 KiB Buckets and flush-time mutable Segment construction.
//!
//! A Bucket begins with a one-byte Item count. Packed 20-bit Item Offsets
//! follow immediately: the first storage-key byte is stored in 8 bits and the
//! Item byte offset in 12 bits. Item bodies grow backward from the end and
//! contain the remaining 31 storage-key bytes followed by the encoded stored
//! value.

use crate::types::STORAGE_KEY_BYTES;
use crate::*;

pub(crate) const ITEM_OFFSET_BITS: usize = 20;
pub(crate) const ITEM_KEY_PREFIX_BITS: usize = 8;
pub(crate) const ITEM_BYTE_OFFSET_BITS: usize = 12;
pub(crate) const ITEM_FIXED_BYTES: usize = STORAGE_KEY_BYTES - 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Item {
    pub(crate) storage_key: StorageKey,
    pub(crate) value: Vec<u8>,
}

impl Item {
    pub(crate) fn encoded_len(&self) -> usize {
        ITEM_FIXED_BYTES + self.value.len()
    }
}

pub(crate) struct MutableSegment {
    pub(crate) bytes: DirectIoBuffer,
    pub(crate) sg_index: usize,
    pub(crate) item_count: usize,
    pub(crate) accepted_item_bytes: u64,
    bucket_choice_count: usize,
    bucket_selection_policy: BucketSelectionPolicy,
}

impl MutableSegment {
    pub(crate) fn new(config: &Config, sg_index: usize) -> Self {
        Self {
            bytes: DirectIoBuffer::zeroed(config.segment_size),
            sg_index,
            item_count: 0,
            accepted_item_bytes: 0,
            bucket_choice_count: config.bucket_choice_count,
            bucket_selection_policy: config.bucket_selection_policy,
        }
    }

    fn bucket(&self, bucket_index: usize) -> &[u8] {
        let start = bucket_index * BUCKET_BYTES;
        &self.bytes[start..start + BUCKET_BYTES]
    }

    fn bucket_mut(&mut self, bucket_index: usize) -> &mut [u8] {
        let start = bucket_index * BUCKET_BYTES;
        &mut self.bytes[start..start + BUCKET_BYTES]
    }

    pub(crate) fn used_bytes(&self) -> usize {
        let (buckets, remainder) = self.bytes.as_chunks::<BUCKET_BYTES>();
        debug_assert!(remainder.is_empty());
        buckets.iter().map(|bucket| bucket_used_bytes(bucket)).sum()
    }

    pub(crate) fn choose_bucket(
        &self,
        storage_key: &StorageKey,
        encoded_len: usize,
    ) -> Option<(usize, u8)> {
        let bucket_count = self.bytes.len() / BUCKET_BYTES;
        let mut best = None;
        for bucket_hash_index in 0..self.bucket_choice_count as u8 {
            let bucket_index = bucket_hash(storage_key, bucket_hash_index, bucket_count);
            let bucket = self.bucket(bucket_index);
            if !bucket_can_fit(bucket, encoded_len) {
                continue;
            }
            let used = bucket_used_bytes(bucket);
            let preferred =
                best.is_none_or(|(_, _, best_used)| match self.bucket_selection_policy {
                    BucketSelectionPolicy::LeastUsed => used < best_used,
                    BucketSelectionPolicy::MostUsed => used > best_used,
                });
            if preferred {
                best = Some((bucket_index, bucket_hash_index, used));
            }
        }
        best.map(|(bucket_index, bucket_hash_index, _)| (bucket_index, bucket_hash_index))
    }

    pub(crate) fn append(&mut self, item: Item, count_accepted: bool) -> Option<TableLocation> {
        let (bucket_index, bucket_hash_index) =
            self.choose_bucket(&item.storage_key, item.encoded_len())?;
        if !append_item_to_bucket(self.bucket_mut(bucket_index), &item) {
            return None;
        }
        self.item_count += 1;
        if count_accepted {
            self.accepted_item_bytes += (STORAGE_KEY_BYTES + item.value.len()) as u64;
        }
        Some(TableLocation {
            sg_index: self.sg_index as u16,
            bucket_hash_index,
        })
    }
}

pub(crate) fn bucket_item_count(bucket: &[u8]) -> usize {
    bucket.first().copied().unwrap_or_default() as usize
}

pub(crate) const fn item_offsets_bytes(count: usize) -> usize {
    (count * ITEM_OFFSET_BITS).div_ceil(8)
}

fn item_offsets_end(count: usize) -> usize {
    1 + item_offsets_bytes(count)
}

fn items_start(bucket: &[u8], count: usize) -> usize {
    if count == 0 {
        bucket.len()
    } else {
        item_offset(bucket, count - 1)
            .map(|entry| entry.item_byte_offset)
            .unwrap_or_default()
    }
}

pub(crate) fn bucket_used_bytes(bucket: &[u8]) -> usize {
    let count = bucket_item_count(bucket);
    item_offsets_end(count) + bucket.len().saturating_sub(items_start(bucket, count))
}

fn bucket_can_fit(bucket: &[u8], encoded_len: usize) -> bool {
    let count = bucket_item_count(bucket);
    count < u8::MAX as usize
        && encoded_len >= ITEM_FIXED_BYTES
        && items_start(bucket, count)
            .checked_sub(encoded_len)
            .is_some_and(|start| start >= item_offsets_end(count + 1))
}

#[derive(Clone, Copy)]
pub(crate) struct ItemOffset {
    pub(crate) key_prefix: u8,
    pub(crate) item_byte_offset: usize,
}

pub(crate) fn item_offset(bucket: &[u8], item_slot: usize) -> Option<ItemOffset> {
    let bit = 8 + item_slot * ITEM_OFFSET_BITS;
    let key_prefix = get_packed_bits(bucket, bit, ITEM_KEY_PREFIX_BITS)? as u8;
    let item_byte_offset =
        get_packed_bits(bucket, bit + ITEM_KEY_PREFIX_BITS, ITEM_BYTE_OFFSET_BITS)? as usize;
    Some(ItemOffset {
        key_prefix,
        item_byte_offset,
    })
}

fn write_item_offset(bucket: &mut [u8], item_slot: usize, entry: ItemOffset) {
    let bit = 8 + item_slot * ITEM_OFFSET_BITS;
    set_packed_bits(bucket, bit, ITEM_KEY_PREFIX_BITS, entry.key_prefix as u16);
    set_packed_bits(
        bucket,
        bit + ITEM_KEY_PREFIX_BITS,
        ITEM_BYTE_OFFSET_BITS,
        entry.item_byte_offset as u16,
    );
}

fn get_packed_bits(bytes: &[u8], bit: usize, width: usize) -> Option<u16> {
    if bit + width > bytes.len() * 8 {
        return None;
    }
    let mut value = 0u16;
    for offset in 0..width {
        value |= (((bytes[(bit + offset) / 8] >> ((bit + offset) % 8)) & 1) as u16) << offset;
    }
    Some(value)
}

fn set_packed_bits(bytes: &mut [u8], bit: usize, width: usize, value: u16) {
    for offset in 0..width {
        let target = bit + offset;
        let mask = 1u8 << (target % 8);
        if value & (1u16 << offset) == 0 {
            bytes[target / 8] &= !mask;
        } else {
            bytes[target / 8] |= mask;
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ItemSpan {
    pub(crate) item_slot: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

fn item_span(bucket: &[u8], item_slot: usize) -> Option<ItemSpan> {
    let count = bucket_item_count(bucket);
    if item_slot >= count {
        return None;
    }
    let start = item_offset(bucket, item_slot)?.item_byte_offset;
    let end = if item_slot == 0 {
        bucket.len()
    } else {
        item_offset(bucket, item_slot - 1)?.item_byte_offset
    };
    (start >= item_offsets_end(count) && start + ITEM_FIXED_BYTES <= end && end <= bucket.len())
        .then_some(ItemSpan {
            item_slot,
            start,
            end,
        })
}

fn item_at(bucket: &[u8], item_slot: usize) -> Option<Item> {
    let entry = item_offset(bucket, item_slot)?;
    let span = item_span(bucket, item_slot)?;
    let mut storage_key_bytes = [0u8; STORAGE_KEY_BYTES];
    storage_key_bytes[0] = entry.key_prefix;
    storage_key_bytes[1..].copy_from_slice(&bucket[span.start..span.start + ITEM_FIXED_BYTES]);
    Some(Item {
        storage_key: StorageKey::new(storage_key_bytes),
        value: bucket[span.start + ITEM_FIXED_BYTES..span.end].to_vec(),
    })
}

pub(crate) fn append_item_to_bucket(bucket: &mut [u8], item: &Item) -> bool {
    if bucket.len() != BUCKET_BYTES || !bucket_can_fit(bucket, item.encoded_len()) {
        return false;
    }
    let count = bucket_item_count(bucket);
    let end = items_start(bucket, count);
    let start = end - item.encoded_len();
    bucket[start..start + ITEM_FIXED_BYTES].copy_from_slice(&item.storage_key.as_bytes()[1..]);
    bucket[start + ITEM_FIXED_BYTES..end].copy_from_slice(&item.value);
    write_item_offset(
        bucket,
        count,
        ItemOffset {
            key_prefix: item.storage_key.as_bytes()[0],
            item_byte_offset: start,
        },
    );
    bucket[0] = (count + 1) as u8;
    true
}

pub(crate) fn matching_item_spans(bucket: &[u8], storage_key: &StorageKey) -> Vec<ItemSpan> {
    let storage_key = storage_key.as_bytes();
    (0..bucket_item_count(bucket))
        .filter_map(|slot| {
            let entry = item_offset(bucket, slot)?;
            if entry.key_prefix != storage_key[0] {
                return None;
            }
            let span = item_span(bucket, slot)?;
            (bucket[span.start..span.start + ITEM_FIXED_BYTES] == storage_key[1..]).then_some(span)
        })
        .collect()
}

pub(crate) fn items(bucket: &[u8]) -> Vec<Item> {
    if bucket.len() != BUCKET_BYTES {
        return Vec::new();
    }
    (0..bucket_item_count(bucket))
        .map_while(|slot| item_at(bucket, slot))
        .collect()
}

pub(crate) fn find_item_in_bucket(bucket: &[u8], storage_key: &StorageKey) -> Option<Item> {
    let slot = matching_item_spans(bucket, storage_key).last()?.item_slot;
    item_at(bucket, slot)
}

pub(crate) fn bucket_hash(
    storage_key: &StorageKey,
    bucket_hash_index: u8,
    bucket_count: usize,
) -> usize {
    let storage_key = storage_key.as_bytes();
    let first = u64::from_le_bytes(storage_key[16..24].try_into().unwrap());
    let second = u64::from_le_bytes(storage_key[24..32].try_into().unwrap());
    let hash = match bucket_hash_index {
        0 => first,
        1 => second,
        index => first.wrapping_add(u64::from(index).wrapping_mul(second.rotate_left(32) | 1)),
    };
    hash as usize % bucket_count
}
