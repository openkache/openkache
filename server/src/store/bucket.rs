//! Fixed 4 KiB Buckets and the active in-memory Segment.
//!
//! A Bucket begins with a one-byte Item count. Packed 20-bit Item Offsets
//! follow immediately: the first storage-key byte is stored in 8 bits and the
//! Item byte offset in 12 bits. Item bodies grow backward from the end and
//! contain the remaining 31 storage-key bytes, a one-byte live/Tombstone
//! marker, and the value.

use crate::types::STORAGE_KEY_BYTES;
use crate::*;

pub(crate) const ITEM_OFFSET_BITS: usize = 20;
pub(crate) const ITEM_KEY_PREFIX_BITS: usize = 8;
pub(crate) const ITEM_BYTE_OFFSET_BITS: usize = 12;
pub(crate) const ITEM_STORAGE_KEY_SUFFIX_BYTES: usize = STORAGE_KEY_BYTES - 1;
pub(crate) const ITEM_KIND_BYTES: usize = 1;
pub(crate) const ITEM_FIXED_BYTES: usize = ITEM_STORAGE_KEY_SUFFIX_BYTES + ITEM_KIND_BYTES;
pub(crate) const TOMBSTONE_KIND: u8 = 0;
pub(crate) const LIVE_KIND: u8 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Item {
    pub(crate) storage_key: StorageKey,
    pub(crate) value: Vec<u8>,
    pub(crate) is_tombstone: bool,
}

impl Item {
    pub(crate) fn live(storage_key: StorageKey, value: Vec<u8>) -> Self {
        Self {
            storage_key,
            value,
            is_tombstone: false,
        }
    }

    pub(crate) fn tombstone(storage_key: StorageKey) -> Self {
        Self {
            storage_key,
            value: Vec::new(),
            is_tombstone: true,
        }
    }

    pub(crate) fn encoded_len(&self) -> usize {
        ITEM_FIXED_BYTES + self.value.len()
    }
}

pub(crate) struct MutableSegment {
    pub(crate) bytes: DirectIoBuffer,
    pub(crate) sg_index: usize,
    pub(crate) item_count: usize,
    pub(crate) accepted_item_bytes: u64,
}

pub(crate) enum MutableItemReplace {
    NotFound,
    Replaced(TableLocation),
    NoSpace,
}

impl MutableSegment {
    pub(crate) fn new(config: &Config, sg_index: usize) -> Self {
        Self {
            bytes: DirectIoBuffer::zeroed(config.segment_size),
            sg_index,
            item_count: 0,
            accepted_item_bytes: 0,
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

    pub(crate) fn choose_bucket(
        &self,
        storage_key: &StorageKey,
        encoded_len: usize,
    ) -> Option<(usize, u8)> {
        let bucket_count = self.bytes.len() / BUCKET_BYTES;
        let first = bucket_hash(storage_key, 0, bucket_count);
        let second = bucket_hash(storage_key, 1, bucket_count);
        let first_used = bucket_used_bytes(self.bucket(first));
        let second_used = bucket_used_bytes(self.bucket(second));
        let first_fits = bucket_can_fit(self.bucket(first), encoded_len);
        let second_fits = bucket_can_fit(self.bucket(second), encoded_len);
        match (first_fits, second_fits) {
            (false, false) => None,
            (true, false) => Some((first, 0)),
            (false, true) => Some((second, 1)),
            (true, true) if first_used <= second_used => Some((first, 0)),
            (true, true) => Some((second, 1)),
        }
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
            is_blob: false,
            sg_index: self.sg_index as u16,
            bucket_hash_index,
        })
    }

    pub(crate) fn replace(
        &mut self,
        storage_key: &StorageKey,
        item: Item,
        count_accepted: bool,
    ) -> MutableItemReplace {
        let bucket_count = self.bytes.len() / BUCKET_BYTES;
        let first = bucket_hash(storage_key, 0, bucket_count);
        let second = bucket_hash(storage_key, 1, bucket_count);
        let mut candidate_buckets = vec![first];
        if second != first {
            candidate_buckets.push(second);
        }
        let matches = candidate_buckets
            .iter()
            .flat_map(|&bucket_index| {
                matching_item_spans(self.bucket(bucket_index), storage_key)
                    .into_iter()
                    .map(move |span| (bucket_index, span))
            })
            .collect::<Vec<_>>();
        let Some(&(bucket_index, span)) = matches.last() else {
            return MutableItemReplace::NotFound;
        };

        if matches.len() == 1 {
            let mut replacement = self.bucket(bucket_index).to_vec();
            if replace_bucket_item(&mut replacement, span, &item) {
                self.bucket_mut(bucket_index).copy_from_slice(&replacement);
                if count_accepted {
                    self.accepted_item_bytes += (STORAGE_KEY_BYTES + item.value.len()) as u64;
                }
                return MutableItemReplace::Replaced(TableLocation {
                    is_blob: false,
                    sg_index: self.sg_index as u16,
                    bucket_hash_index: if bucket_index == first { 0 } else { 1 },
                });
            }
        }

        let saved_buckets = candidate_buckets
            .iter()
            .map(|&bucket_index| (bucket_index, self.bucket(bucket_index).to_vec()))
            .collect::<Vec<_>>();
        let saved_item_count = self.item_count;
        let saved_accepted_item_bytes = self.accepted_item_bytes;
        let removed = candidate_buckets
            .iter()
            .map(|&bucket_index| remove_key_from_bucket(self.bucket_mut(bucket_index), storage_key))
            .sum::<usize>();
        self.item_count -= removed;

        if let Some(location) = self.append(item, count_accepted) {
            return MutableItemReplace::Replaced(location);
        }
        for (bucket_index, bytes) in saved_buckets {
            self.bucket_mut(bucket_index).copy_from_slice(&bytes);
        }
        self.item_count = saved_item_count;
        self.accepted_item_bytes = saved_accepted_item_bytes;
        MutableItemReplace::NoSpace
    }

    pub(crate) fn remove(&mut self, storage_key: &StorageKey) -> bool {
        let bucket_count = self.bytes.len() / BUCKET_BYTES;
        let first = bucket_hash(storage_key, 0, bucket_count);
        let second = bucket_hash(storage_key, 1, bucket_count);
        let removed = remove_key_from_bucket(self.bucket_mut(first), storage_key)
            + if second == first {
                0
            } else {
                remove_key_from_bucket(self.bucket_mut(second), storage_key)
            };
        self.item_count -= removed;
        removed > 0
    }

    pub(crate) fn find(&self, storage_key: &StorageKey, bucket_hash_index: u8) -> Option<Item> {
        let bucket_index = bucket_hash(
            storage_key,
            bucket_hash_index,
            self.bytes.len() / BUCKET_BYTES,
        );
        find_item_in_bucket(self.bucket(bucket_index), storage_key)
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
    let storage_key_end = span.start + ITEM_STORAGE_KEY_SUFFIX_BYTES;
    storage_key_bytes[1..].copy_from_slice(&bucket[span.start..storage_key_end]);
    let is_tombstone = match bucket[storage_key_end] {
        TOMBSTONE_KIND => true,
        LIVE_KIND => false,
        _ => return None,
    };
    Some(Item {
        storage_key: StorageKey::new(storage_key_bytes),
        value: bucket[storage_key_end + ITEM_KIND_BYTES..span.end].to_vec(),
        is_tombstone,
    })
}

pub(crate) fn append_item_to_bucket(bucket: &mut [u8], item: &Item) -> bool {
    if bucket.len() != BUCKET_BYTES || !bucket_can_fit(bucket, item.encoded_len()) {
        return false;
    }
    let count = bucket_item_count(bucket);
    let end = items_start(bucket, count);
    let start = end - item.encoded_len();
    let storage_key_end = start + ITEM_STORAGE_KEY_SUFFIX_BYTES;
    bucket[start..storage_key_end].copy_from_slice(&item.storage_key.as_bytes()[1..]);
    bucket[storage_key_end] = if item.is_tombstone {
        TOMBSTONE_KIND
    } else {
        LIVE_KIND
    };
    bucket[storage_key_end + ITEM_KIND_BYTES..end].copy_from_slice(&item.value);
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
            (bucket[span.start..span.start + ITEM_STORAGE_KEY_SUFFIX_BYTES] == storage_key[1..])
                .then_some(span)
        })
        .collect()
}

fn rebuild_bucket(bucket: &mut [u8], items: &[Item]) -> bool {
    bucket.fill(0);
    items.iter().all(|item| append_item_to_bucket(bucket, item))
}

pub(crate) fn replace_bucket_item(bucket: &mut [u8], span: ItemSpan, item: &Item) -> bool {
    let mut decoded = items(bucket);
    if span.item_slot >= decoded.len() {
        return false;
    }
    decoded[span.item_slot] = item.clone();
    let mut rebuilt = vec![0; bucket.len()];
    if !rebuild_bucket(&mut rebuilt, &decoded) {
        return false;
    }
    bucket.copy_from_slice(&rebuilt);
    true
}

pub(crate) fn remove_key_from_bucket(bucket: &mut [u8], storage_key: &StorageKey) -> usize {
    let mut decoded = items(bucket);
    let old_len = decoded.len();
    decoded.retain(|item| item.storage_key != *storage_key);
    let removed = old_len - decoded.len();
    if removed > 0 {
        let rebuilt = rebuild_bucket(bucket, &decoded);
        debug_assert!(rebuilt);
    }
    removed
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
    let start = if bucket_hash_index == 0 { 16 } else { 24 };
    let storage_key = storage_key.as_bytes();
    u64::from_le_bytes(storage_key[start..start + 8].try_into().unwrap()) as usize % bucket_count
}
