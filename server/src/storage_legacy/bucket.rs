//! Legacy storage format and operations for a 4 KiB Bucket.

use super::StorageKey;
use std::mem::MaybeUninit;

use compio::buf::{IoBuf, IoBufMut, SetLen};

pub(crate) const BUCKET_BYTES: usize = 4 * 1024;

const ITEM_COUNT_BYTES: usize = 1;
const INDEX_ENTRY_BITS: usize = 20;
const KEY_PREFIX_BITS: usize = 8;
const ITEM_OFFSET_BITS: usize = 12;
const ITEM_OFFSET_MASK: u32 = (1 << ITEM_OFFSET_BITS) - 1;
const KEY_SUFFIX_BYTES: usize = 31;
const VALUE_KIND_BYTES: usize = 1;
const ITEM_FIXED_BYTES: usize = KEY_SUFFIX_BYTES + VALUE_KIND_BYTES;

const TOMBSTONE: u8 = 0;
const LIVE_VALUE: u8 = 1;

/// A value read from a Bucket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BucketValue<'a> {
    /// An Item marking a deleted value.
    Tombstone,
    /// A borrowed value stored in the Bucket.
    Value(&'a [u8]),
}

impl BucketValue<'_> {
    pub(super) fn encoded_len(self) -> usize {
        ITEM_FIXED_BYTES
            + match self {
                Self::Tombstone => 0,
                Self::Value(value) => value.len(),
            }
    }
}

/// A temporary CPU representation of one unpacked 20-bit index entry.
/// The Bucket stores only an `8-bit key prefix + 12-bit offset`, not this struct.
#[derive(Clone, Copy)]
struct BucketIndexEntry {
    key_prefix: u8,
    item_byte_offset: u16,
}

/// A Bucket with the size and alignment of one direct I/O page.
///
/// The Item count and 20-bit index grow from the front, while Item bodies grow backward from the end.
#[repr(C, align(4096))]
pub(crate) struct Bucket {
    bytes: [u8; BUCKET_BYTES],
}

impl Bucket {
    /// Creates a zero-initialized Bucket containing no Items.
    pub(crate) const fn new() -> Self {
        Self {
            bytes: [0; BUCKET_BYTES],
        }
    }

    /// Returns the byte region Compio uses to read or write the entire Bucket.
    pub(crate) const fn as_bytes(&self) -> &[u8; BUCKET_BYTES] {
        &self.bytes
    }

    /// Returns the byte region Compio fills with an entire Bucket from the SSD.
    pub(crate) const fn as_bytes_mut(&mut self) -> &mut [u8; BUCKET_BYTES] {
        &mut self.bytes
    }

    /// Returns the current number of Items.
    pub(crate) const fn item_count(&self) -> usize {
        self.bytes[0] as usize
    }

    /// Returns the bytes occupied by the index and Item bodies.
    pub(crate) fn used_bytes(&self) -> usize {
        let item_count = self.item_count();
        Self::index_end(item_count) + BUCKET_BYTES - self.items_start(item_count)
    }

    /// Checks whether the index entry and body for this value fit in the remaining space.
    pub(crate) fn can_append(&self, value: BucketValue<'_>) -> bool {
        let item_count = self.item_count();
        item_count < u8::MAX as usize && self.can_fit(item_count, value.encoded_len())
    }

    /// Copies a key and value to the back of the Bucket and adds a 20-bit index entry.
    /// Returns false without modifying the Bucket when space is insufficient.
    pub(crate) fn append(&mut self, storage_key: &StorageKey, value: BucketValue<'_>) -> bool {
        let item_count = self.item_count();
        let item_len = value.encoded_len();
        if !self.can_append(value) {
            return false;
        }

        let end = self.items_start(item_count);
        let start = end - item_len;
        self.write_item(start, end, storage_key, value);
        self.write_index_entry(
            item_count,
            BucketIndexEntry {
                key_prefix: storage_key.as_bytes()[0],
                item_byte_offset: start as u16,
            },
        );
        self.bytes[0] = (item_count + 1) as u8;
        true
    }

    /// Finds the newest Item with the same key and borrows its value from the Bucket.
    pub(crate) fn get(&self, storage_key: &StorageKey) -> Option<BucketValue<'_>> {
        let item_slot = self.find_item_slot(storage_key)?;
        self.value_at(item_slot)
    }

    /// Replaces the newest Item with the same key and shifts bodies by the size difference.
    /// Preserves the original Item when a larger replacement cannot fit in 4 KiB.
    pub(crate) fn replace(
        &mut self,
        storage_key: &StorageKey,
        replacement: BucketValue<'_>,
    ) -> bool {
        let Some(replaced_slot) = self.find_item_slot(storage_key) else {
            return false;
        };
        let Some((previous_start, previous_end)) = self.item_span(replaced_slot) else {
            return false;
        };
        let previous_len = previous_end - previous_start;
        let replacement_len = replacement.encoded_len();
        let item_count = self.item_count();
        let current_items_start = self.items_start(item_count);

        if replacement_len > previous_len {
            let extra = replacement_len - previous_len;
            let Some(new_items_start) = current_items_start.checked_sub(extra) else {
                return false;
            };
            if new_items_start < Self::index_end(item_count) {
                return false;
            }
            self.bytes
                .copy_within(current_items_start..previous_start, new_items_start);
            for item_slot in replaced_slot + 1..item_count {
                let Some(mut entry) = self.read_index_entry(item_slot) else {
                    return false;
                };
                entry.item_byte_offset -= extra as u16;
                self.write_index_entry(item_slot, entry);
            }
        } else if replacement_len < previous_len {
            let reclaimed = previous_len - replacement_len;
            self.bytes.copy_within(
                current_items_start..previous_start,
                current_items_start + reclaimed,
            );
            self.bytes[current_items_start..current_items_start + reclaimed].fill(0);
            for item_slot in replaced_slot + 1..item_count {
                let Some(mut entry) = self.read_index_entry(item_slot) else {
                    return false;
                };
                entry.item_byte_offset += reclaimed as u16;
                self.write_index_entry(item_slot, entry);
            }
        }

        let replacement_start = previous_end - replacement_len;
        self.write_index_entry(
            replaced_slot,
            BucketIndexEntry {
                key_prefix: storage_key.as_bytes()[0],
                item_byte_offset: replacement_start as u16,
            },
        );
        self.write_item(replacement_start, previous_end, storage_key, replacement);
        true
    }

    /// Removes the newest Item with the same key and compacts the following Items once.
    pub(crate) fn remove(&mut self, storage_key: &StorageKey) -> bool {
        let Some(removed_slot) = self.find_item_slot(storage_key) else {
            return false;
        };
        let Some((removed_start, removed_end)) = self.item_span(removed_slot) else {
            return false;
        };
        let item_count = self.item_count();
        let removed_len = removed_end - removed_start;
        let current_items_start = self.items_start(item_count);

        self.bytes.copy_within(
            current_items_start..removed_start,
            current_items_start + removed_len,
        );
        self.bytes[current_items_start..current_items_start + removed_len].fill(0);

        for item_slot in removed_slot + 1..item_count {
            let Some(mut entry) = self.read_index_entry(item_slot) else {
                return false;
            };
            entry.item_byte_offset += removed_len as u16;
            self.write_index_entry(item_slot - 1, entry);
        }
        self.clear_index_entry(item_count - 1);
        self.bytes[0] = (item_count - 1) as u8;
        true
    }

    const fn index_bytes(item_count: usize) -> usize {
        (item_count * INDEX_ENTRY_BITS).div_ceil(u8::BITS as usize)
    }

    const fn index_end(item_count: usize) -> usize {
        ITEM_COUNT_BYTES + Self::index_bytes(item_count)
    }

    fn items_start(&self, item_count: usize) -> usize {
        if item_count == 0 {
            BUCKET_BYTES
        } else {
            self.read_index_entry(item_count - 1)
                .map_or(0, |entry| entry.item_byte_offset as usize)
        }
    }

    fn can_fit(&self, item_count: usize, item_len: usize) -> bool {
        item_len >= ITEM_FIXED_BYTES
            && self
                .items_start(item_count)
                .checked_sub(item_len)
                .is_some_and(|start| start >= Self::index_end(item_count + 1))
    }

    fn read_index_entry(&self, item_slot: usize) -> Option<BucketIndexEntry> {
        if item_slot >= self.item_count() {
            return None;
        }

        let bit_offset = u8::BITS as usize + item_slot * INDEX_ENTRY_BITS;
        let byte_offset = bit_offset / u8::BITS as usize;
        let shift = bit_offset % u8::BITS as usize;
        let packed = u32::from_le_bytes([
            *self.bytes.get(byte_offset)?,
            *self.bytes.get(byte_offset + 1)?,
            *self.bytes.get(byte_offset + 2)?,
            0,
        ]) >> shift;

        Some(BucketIndexEntry {
            key_prefix: packed as u8,
            item_byte_offset: ((packed >> KEY_PREFIX_BITS) & ITEM_OFFSET_MASK) as u16,
        })
    }

    fn write_index_entry(&mut self, item_slot: usize, entry: BucketIndexEntry) {
        debug_assert!((entry.item_byte_offset as usize) < BUCKET_BYTES);

        let bit_offset = u8::BITS as usize + item_slot * INDEX_ENTRY_BITS;
        let byte_offset = bit_offset / u8::BITS as usize;
        let shift = bit_offset % u8::BITS as usize;
        let packed =
            u32::from(entry.key_prefix) | (u32::from(entry.item_byte_offset) << KEY_PREFIX_BITS);
        let current = u32::from_le_bytes([
            self.bytes[byte_offset],
            self.bytes[byte_offset + 1],
            self.bytes[byte_offset + 2],
            0,
        ]);
        let mask = ((1u32 << INDEX_ENTRY_BITS) - 1) << shift;
        let updated = (current & !mask) | ((packed << shift) & mask);
        self.bytes[byte_offset..byte_offset + 3].copy_from_slice(&updated.to_le_bytes()[..3]);
    }

    fn clear_index_entry(&mut self, item_slot: usize) {
        let bit_offset = u8::BITS as usize + item_slot * INDEX_ENTRY_BITS;
        let byte_offset = bit_offset / u8::BITS as usize;
        let shift = bit_offset % u8::BITS as usize;
        let current = u32::from_le_bytes([
            self.bytes[byte_offset],
            self.bytes[byte_offset + 1],
            self.bytes[byte_offset + 2],
            0,
        ]);
        let mask = ((1u32 << INDEX_ENTRY_BITS) - 1) << shift;
        let updated = current & !mask;
        self.bytes[byte_offset..byte_offset + 3].copy_from_slice(&updated.to_le_bytes()[..3]);
    }

    fn item_span(&self, item_slot: usize) -> Option<(usize, usize)> {
        let item_count = self.item_count();
        let start = self.read_index_entry(item_slot)?.item_byte_offset as usize;
        let end = if item_slot == 0 {
            BUCKET_BYTES
        } else {
            self.read_index_entry(item_slot - 1)?.item_byte_offset as usize
        };

        (start >= Self::index_end(item_count)
            && start + ITEM_FIXED_BYTES <= end
            && end <= BUCKET_BYTES)
            .then_some((start, end))
    }

    fn value_at(&self, item_slot: usize) -> Option<BucketValue<'_>> {
        let (start, end) = self.item_span(item_slot)?;
        let kind_offset = start + KEY_SUFFIX_BYTES;
        match self.bytes[kind_offset] {
            TOMBSTONE => Some(BucketValue::Tombstone),
            LIVE_VALUE => Some(BucketValue::Value(
                &self.bytes[kind_offset + VALUE_KIND_BYTES..end],
            )),
            _ => None,
        }
    }

    fn find_item_slot(&self, storage_key: &StorageKey) -> Option<usize> {
        let key = storage_key.as_bytes();
        (0..self.item_count()).rev().find(|&item_slot| {
            let Some(entry) = self.read_index_entry(item_slot) else {
                return false;
            };
            if entry.key_prefix != key[0] {
                return false;
            }
            let Some((start, _)) = self.item_span(item_slot) else {
                return false;
            };
            self.bytes[start..start + KEY_SUFFIX_BYTES] == key[1..]
        })
    }

    fn write_item(
        &mut self,
        start: usize,
        end: usize,
        storage_key: &StorageKey,
        value: BucketValue<'_>,
    ) {
        let key_end = start + KEY_SUFFIX_BYTES;
        self.bytes[start..key_end].copy_from_slice(&storage_key.as_bytes()[1..]);
        match value {
            BucketValue::Tombstone => self.bytes[key_end] = TOMBSTONE,
            BucketValue::Value(value) => {
                self.bytes[key_end] = LIVE_VALUE;
                self.bytes[key_end + VALUE_KIND_BYTES..end].copy_from_slice(value);
            }
        }
    }
}

impl Default for Bucket {
    fn default() -> Self {
        Self::new()
    }
}

impl IoBuf for Bucket {
    fn as_init(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl IoBufMut for Bucket {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let bytes = self.as_bytes_mut();
        // SAFETY: u8 and MaybeUninit<u8> have identical size and alignment, and the
        // entire Bucket is writable.
        unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr().cast(), bytes.len()) }
    }
}

impl SetLen for Bucket {
    unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= BUCKET_BYTES);
    }
}
