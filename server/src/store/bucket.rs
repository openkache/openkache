//! Fixed 4 KiB Buckets and flush-time mutable Segment construction.
//!
//! A Bucket begins with a one-byte Item count. Packed 20-bit Item Offsets
//! follow immediately: the first storage-key byte is stored in 8 bits and the
//! Item byte offset in 12 bits. Item bodies grow backward from the end and
//! contain the remaining 31 storage-key bytes followed by the encoded stored
//! value, with a separate one-byte live/Tombstone marker.

use crate::types::STORAGE_KEY_BYTES;
use crate::*;

pub(crate) const ITEM_OFFSET_BITS: usize = 20;
pub(crate) const STORAGE_KEY_PREFIX_BITS: usize = 8;
pub(crate) const ITEM_BYTE_OFFSET_BITS: usize = 12;
pub(crate) const ITEM_STORAGE_KEY_SUFFIX_BYTES: usize = STORAGE_KEY_BYTES - 1;
pub(crate) const ITEM_KIND_BYTES: usize = 1;
pub(crate) const ITEM_CHECKSUM_BYTES: usize = std::mem::size_of::<u32>();
/// Key suffix, kind, and the per-Item checksum trailer.
pub(crate) const ITEM_FIXED_BYTES: usize =
    ITEM_STORAGE_KEY_SUFFIX_BYTES + ITEM_KIND_BYTES + ITEM_CHECKSUM_BYTES;
pub(crate) const ITEM_EXPIRATION_BYTES: usize = std::mem::size_of::<u64>();
pub(crate) const TOMBSTONE_KIND: u8 = 0;
pub(crate) const LIVE_KIND: u8 = 1;
pub(crate) const EXPIRING_LIVE_KIND: u8 = 2;
pub(crate) const PROTECTED_LIVE_KIND: u8 = 3;
pub(crate) const PROTECTED_EXPIRING_LIVE_KIND: u8 = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Item {
    pub(crate) storage_key: StorageKey,
    pub(crate) value: Vec<u8>,
    pub(crate) is_tombstone: bool,
    pub(crate) expires_at_ms: u64,
    pub(crate) eviction_protected: bool,
}

impl Item {
    #[allow(dead_code)]
    pub(crate) fn live(storage_key: StorageKey, value: Vec<u8>) -> Self {
        Self::live_with_eviction(storage_key, value, false)
    }

    pub(crate) fn live_with_eviction(
        storage_key: StorageKey,
        value: Vec<u8>,
        eviction_protected: bool,
    ) -> Self {
        Self {
            storage_key,
            value,
            is_tombstone: false,
            expires_at_ms: 0,
            eviction_protected,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn live_expiring(
        storage_key: StorageKey,
        value: Vec<u8>,
        expires_at_ms: u64,
    ) -> Self {
        Self::live_expiring_with_eviction(storage_key, value, expires_at_ms, false)
    }

    pub(crate) fn live_expiring_with_eviction(
        storage_key: StorageKey,
        value: Vec<u8>,
        expires_at_ms: u64,
        eviction_protected: bool,
    ) -> Self {
        debug_assert!(expires_at_ms > 0);
        Self {
            storage_key,
            value,
            is_tombstone: false,
            expires_at_ms,
            eviction_protected,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn tombstone(storage_key: StorageKey) -> Self {
        Self {
            storage_key,
            value: Vec::new(),
            is_tombstone: true,
            expires_at_ms: 0,
            eviction_protected: false,
        }
    }

    pub(crate) fn encoded_len(&self) -> usize {
        ITEM_FIXED_BYTES
            + if self.expires_at_ms == 0 {
                0
            } else {
                ITEM_EXPIRATION_BYTES
            }
            + self.value.len()
    }

    pub(crate) fn is_expired_at(&self, now_ms: u64) -> bool {
        !self.is_tombstone && self.expires_at_ms != 0 && self.expires_at_ms <= now_ms
    }

    pub(crate) fn is_live_at(&self, now_ms: u64) -> bool {
        !self.is_tombstone && !self.is_expired_at(now_ms)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ItemState {
    pub(super) is_tombstone: bool,
    pub(super) expires_at_ms: u64,
    pub(super) eviction_protected: bool,
}

pub(crate) struct MutableSegment {
    pub(crate) bytes: DirectIoBuffer,
    pub(crate) sg_index: usize,
    pub(crate) item_count: usize,
    pub(crate) accepted_item_bytes: u64,
    used_bytes: usize,
    bucket_choice_count: usize,
    bucket_selection_policy: BucketSelectionPolicy,
}

impl MutableSegment {
    pub(crate) fn new(config: &Config, sg_index: usize) -> Self {
        Self::with_bytes(
            config,
            sg_index,
            DirectIoBuffer::zeroed(config.segment_size),
        )
    }

    #[allow(dead_code)]
    pub(crate) fn reuse(config: &Config, sg_index: usize, mut bytes: DirectIoBuffer) -> Self {
        debug_assert_eq!(bytes.len(), config.segment_size);
        bytes.fill(0);
        Self::with_bytes(config, sg_index, bytes)
    }

    fn with_bytes(config: &Config, sg_index: usize, bytes: DirectIoBuffer) -> Self {
        let used_bytes = bytes.len() / BUCKET_BYTES;
        Self {
            bytes,
            sg_index,
            item_count: 0,
            accepted_item_bytes: 0,
            used_bytes,
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
        self.used_bytes
    }

    pub(crate) fn choose_bucket(
        &self,
        storage_key: &StorageKey,
        encoded_len: usize,
    ) -> Option<(usize, u8)> {
        let bucket_count = self.bytes.len() / BUCKET_BYTES;
        let hashes = BucketHashSequence::new(storage_key, bucket_count);
        let mut best = None;
        for bucket_hash_index in 0..self.bucket_choice_count as u8 {
            let bucket_index = hashes.get(bucket_hash_index);
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
        let previous_used_bytes = bucket_used_bytes(self.bucket(bucket_index));
        if !append_item_to_bucket(self.bucket_mut(bucket_index), &item) {
            return None;
        }
        self.used_bytes += bucket_used_bytes(self.bucket(bucket_index)) - previous_used_bytes;
        self.item_count += 1;
        if count_accepted {
            self.accepted_item_bytes += (STORAGE_KEY_BYTES + item.value.len()) as u64;
        }
        Some(TableLocation {
            sg_index: self.sg_index as u32,
            bucket_hash_index,
        })
    }

    pub(crate) fn replace(
        &mut self,
        table_location: TableLocation,
        item: Item,
        count_accepted: bool,
    ) -> bool {
        if table_location.sg_index != self.sg_index as u32 {
            return false;
        }
        let bucket_index = bucket_hash(
            &item.storage_key,
            table_location.bucket_hash_index,
            self.bytes.len() / BUCKET_BYTES,
        );
        let previous_used_bytes = bucket_used_bytes(self.bucket(bucket_index));
        if !replace_item_in_bucket(self.bucket_mut(bucket_index), &item) {
            return false;
        }
        self.used_bytes =
            self.used_bytes - previous_used_bytes + bucket_used_bytes(self.bucket(bucket_index));
        if count_accepted {
            self.accepted_item_bytes += (STORAGE_KEY_BYTES + item.value.len()) as u64;
        }
        true
    }

    pub(crate) fn remove(
        &mut self,
        table_location: TableLocation,
        storage_key: &StorageKey,
    ) -> bool {
        if table_location.sg_index != self.sg_index as u32 {
            return false;
        }
        let bucket_index = bucket_hash(
            storage_key,
            table_location.bucket_hash_index,
            self.bytes.len() / BUCKET_BYTES,
        );
        let previous_used_bytes = bucket_used_bytes(self.bucket(bucket_index));
        if !remove_item_from_bucket(self.bucket_mut(bucket_index), storage_key) {
            return false;
        }
        self.used_bytes =
            self.used_bytes - previous_used_bytes + bucket_used_bytes(self.bucket(bucket_index));
        self.item_count = self.item_count.saturating_sub(1);
        true
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
    let byte = bit / 8;
    let shift = bit % 8;
    let packed = u32::from_le_bytes([
        *bucket.get(byte)?,
        *bucket.get(byte + 1)?,
        *bucket.get(byte + 2)?,
        0,
    ]) >> shift;
    Some(ItemOffset {
        key_prefix: packed as u8,
        item_byte_offset: ((packed >> STORAGE_KEY_PREFIX_BITS) & ((1 << ITEM_BYTE_OFFSET_BITS) - 1))
            as usize,
    })
}

fn write_item_offset(bucket: &mut [u8], item_slot: usize, entry: ItemOffset) {
    let bit = 8 + item_slot * ITEM_OFFSET_BITS;
    let byte = bit / 8;
    let shift = bit % 8;
    let packed =
        u32::from(entry.key_prefix) | ((entry.item_byte_offset as u32) << STORAGE_KEY_PREFIX_BITS);
    let current = u32::from_le_bytes([bucket[byte], bucket[byte + 1], bucket[byte + 2], 0]);
    let mask = ((1u32 << ITEM_OFFSET_BITS) - 1) << shift;
    let updated = (current & !mask) | ((packed << shift) & mask);
    bucket[byte..byte + 3].copy_from_slice(&updated.to_le_bytes()[..3]);
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
    if !item_checksum_is_valid(bucket, span) {
        return None;
    }
    let mut storage_key_bytes = [0u8; STORAGE_KEY_BYTES];
    storage_key_bytes[0] = entry.key_prefix;
    let storage_key_end = span.start + ITEM_STORAGE_KEY_SUFFIX_BYTES;
    storage_key_bytes[1..].copy_from_slice(&bucket[span.start..storage_key_end]);
    let (state, value_start) = item_state_at(bucket, span)?;
    let value_end = span.end.checked_sub(ITEM_CHECKSUM_BYTES)?;
    Some(Item {
        storage_key: StorageKey::new(storage_key_bytes),
        value: bucket[value_start..value_end].to_vec(),
        is_tombstone: state.is_tombstone,
        expires_at_ms: state.expires_at_ms,
        eviction_protected: state.eviction_protected,
    })
}

fn item_state_at(bucket: &[u8], span: ItemSpan) -> Option<(ItemState, usize)> {
    let storage_key_end = span.start + ITEM_STORAGE_KEY_SUFFIX_BYTES;
    let value_end = span.end.checked_sub(ITEM_CHECKSUM_BYTES)?;
    let (state, value_start) = match bucket[storage_key_end] {
        TOMBSTONE_KIND => (
            ItemState {
                is_tombstone: true,
                expires_at_ms: 0,
                eviction_protected: false,
            },
            storage_key_end + ITEM_KIND_BYTES,
        ),
        LIVE_KIND => (
            ItemState {
                is_tombstone: false,
                expires_at_ms: 0,
                eviction_protected: false,
            },
            storage_key_end + ITEM_KIND_BYTES,
        ),
        EXPIRING_LIVE_KIND => {
            let expiration_start = storage_key_end + ITEM_KIND_BYTES;
            let expiration_end = expiration_start + ITEM_EXPIRATION_BYTES;
            if expiration_end > value_end {
                return None;
            }
            let expires_at_ms =
                u64::from_le_bytes(bucket[expiration_start..expiration_end].try_into().unwrap());
            if expires_at_ms == 0 {
                return None;
            }
            (
                ItemState {
                    is_tombstone: false,
                    expires_at_ms,
                    eviction_protected: false,
                },
                expiration_end,
            )
        }
        PROTECTED_LIVE_KIND => (
            ItemState {
                is_tombstone: false,
                expires_at_ms: 0,
                eviction_protected: true,
            },
            storage_key_end + ITEM_KIND_BYTES,
        ),
        PROTECTED_EXPIRING_LIVE_KIND => {
            let expiration_start = storage_key_end + ITEM_KIND_BYTES;
            let expiration_end = expiration_start + ITEM_EXPIRATION_BYTES;
            if expiration_end > value_end {
                return None;
            }
            let expires_at_ms =
                u64::from_le_bytes(bucket[expiration_start..expiration_end].try_into().unwrap());
            if expires_at_ms == 0 {
                return None;
            }
            (
                ItemState {
                    is_tombstone: false,
                    expires_at_ms,
                    eviction_protected: true,
                },
                expiration_end,
            )
        }
        _ => return None,
    };
    Some((state, value_start))
}

pub(crate) fn rewrite_segment_values(
    segment: &mut [u8],
    mut rewrite: impl FnMut(&[u8]) -> Result<Option<Vec<u8>>>,
) -> Result<()> {
    for bucket in segment.chunks_exact_mut(BUCKET_BYTES) {
        validate_bucket(bucket)?;
        let count = bucket_item_count(bucket);
        for item_slot in 0..count {
            let span = item_span(bucket, item_slot)
                .ok_or_else(|| KvError::Worker("mutable SG contains a malformed Item".into()))?;
            let (state, value_start) = item_state_at(bucket, span).ok_or_else(|| {
                KvError::Worker("mutable SG contains an invalid Item state".into())
            })?;
            if state.is_tombstone {
                continue;
            }
            let value_end = span
                .end
                .checked_sub(ITEM_CHECKSUM_BYTES)
                .ok_or_else(|| KvError::Worker("mutable SG Item checksum is truncated".into()))?;
            let encoded = bucket[value_start..value_end].to_vec();
            let Some(replacement) = rewrite(&encoded)? else {
                continue;
            };
            if replacement.len() != encoded.len() {
                return Err(KvError::Worker(
                    "seal-time value rewrite changed the encoded Item length".into(),
                ));
            }
            bucket[value_start..value_end].copy_from_slice(&replacement);
            write_item_checksum(bucket, span);
        }
    }
    Ok(())
}

pub(crate) fn append_item_to_bucket(bucket: &mut [u8], item: &Item) -> bool {
    if bucket.len() != BUCKET_BYTES || !bucket_can_fit(bucket, item.encoded_len()) {
        return false;
    }
    let count = bucket_item_count(bucket);
    let end = items_start(bucket, count);
    let start = end - item.encoded_len();
    write_item_body(bucket, start, end, item);
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

fn write_item_body(bucket: &mut [u8], start: usize, end: usize, item: &Item) {
    debug_assert_eq!(end - start, item.encoded_len());
    let checksum_start = end - ITEM_CHECKSUM_BYTES;
    let storage_key_end = start + ITEM_STORAGE_KEY_SUFFIX_BYTES;
    bucket[start..storage_key_end].copy_from_slice(&item.storage_key.as_bytes()[1..]);
    bucket[storage_key_end] = match (
        item.is_tombstone,
        item.eviction_protected,
        item.expires_at_ms != 0,
    ) {
        (true, _, _) => TOMBSTONE_KIND,
        (false, false, false) => LIVE_KIND,
        (false, false, true) => EXPIRING_LIVE_KIND,
        (false, true, false) => PROTECTED_LIVE_KIND,
        (false, true, true) => PROTECTED_EXPIRING_LIVE_KIND,
    };
    let mut value_start = storage_key_end + ITEM_KIND_BYTES;
    if item.expires_at_ms != 0 {
        let expiration_end = value_start + ITEM_EXPIRATION_BYTES;
        bucket[value_start..expiration_end].copy_from_slice(&item.expires_at_ms.to_le_bytes());
        value_start = expiration_end;
    }
    bucket[value_start..checksum_start].copy_from_slice(&item.value);
    write_item_checksum(
        bucket,
        ItemSpan {
            item_slot: 0,
            start,
            end,
        },
    );
}

pub(crate) fn replace_item_in_bucket(bucket: &mut [u8], replacement: &Item) -> bool {
    if bucket.len() != BUCKET_BYTES {
        return false;
    }
    let Some(span) = find_item_span_in_bucket(bucket, &replacement.storage_key) else {
        return false;
    };
    let previous_len = span.end - span.start;
    let replacement_len = replacement.encoded_len();
    if replacement_len == previous_len {
        write_item_body(bucket, span.start, span.end, replacement);
        return true;
    }
    if replacement_len < previous_len {
        let reclaimed = previous_len - replacement_len;
        let count = bucket_item_count(bucket);
        let current_items_start = items_start(bucket, count);
        bucket.copy_within(
            current_items_start..span.start,
            current_items_start + reclaimed,
        );
        bucket[current_items_start..current_items_start + reclaimed].fill(0);
        for item_slot in span.item_slot + 1..count {
            let mut offset =
                item_offset(bucket, item_slot).expect("mutable Bucket offset is valid");
            offset.item_byte_offset += reclaimed;
            write_item_offset(bucket, item_slot, offset);
        }
        let start = span.end - replacement_len;
        write_item_offset(
            bucket,
            span.item_slot,
            ItemOffset {
                key_prefix: replacement.storage_key.as_bytes()[0],
                item_byte_offset: start,
            },
        );
        write_item_body(bucket, start, span.end, replacement);
        return true;
    }

    let mut scratch = [0u8; BUCKET_BYTES];
    let replacement_slot = span.item_slot;
    for (item_slot, item) in items(bucket).enumerate() {
        let item = if item_slot == replacement_slot {
            replacement
        } else {
            &item
        };
        if !append_item_to_bucket(&mut scratch, item) {
            return false;
        }
    }
    bucket.copy_from_slice(&scratch);
    true
}

pub(crate) fn remove_item_from_bucket(bucket: &mut [u8], storage_key: &StorageKey) -> bool {
    if bucket.len() != BUCKET_BYTES {
        return false;
    }
    let Some(removed) = find_item_span_in_bucket(bucket, storage_key) else {
        return false;
    };
    let mut scratch = [0u8; BUCKET_BYTES];
    for (item_slot, item) in items(bucket).enumerate() {
        if item_slot != removed.item_slot && !append_item_to_bucket(&mut scratch, &item) {
            return false;
        }
    }
    bucket.copy_from_slice(&scratch);
    true
}

pub(crate) fn find_item_span_in_bucket(
    bucket: &[u8],
    storage_key: &StorageKey,
) -> Option<ItemSpan> {
    let storage_key = storage_key.as_bytes();
    (0..bucket_item_count(bucket))
        .rev()
        .find_map(|slot| matching_item_span(bucket, storage_key, slot))
}

pub(crate) fn items(bucket: &[u8]) -> impl Iterator<Item = Item> + '_ {
    let item_count = if bucket.len() == BUCKET_BYTES {
        bucket_item_count(bucket)
    } else {
        0
    };
    (0..item_count).map_while(|slot| item_at(bucket, slot))
}

pub(crate) fn find_item_in_bucket(bucket: &[u8], storage_key: &StorageKey) -> Option<Item> {
    let span = find_item_span_in_bucket(bucket, storage_key)?;
    item_at(bucket, span.item_slot)
}

pub(crate) fn find_item_state_and_value_range(
    bucket: &[u8],
    storage_key: &StorageKey,
) -> Option<(ItemState, std::ops::Range<usize>)> {
    let span = find_item_span_in_bucket(bucket, storage_key)?;
    if !item_checksum_is_valid(bucket, span) {
        return None;
    }
    let (state, value_start) = item_state_at(bucket, span)?;
    let value_end = span.end.checked_sub(ITEM_CHECKSUM_BYTES)?;
    Some((state, value_start..value_end))
}

/// Validates every encoded Item in one Bucket before callers use it as
/// recovery or lookup input. A malformed offset, state, or checksum is a hard
/// corruption error rather than an apparent cache miss.
pub(crate) fn validate_bucket(bucket: &[u8]) -> Result<()> {
    if bucket.len() != BUCKET_BYTES {
        return Err(KvError::Worker("Segment Bucket has an invalid length".into()));
    }
    let count = bucket_item_count(bucket);
    for item_slot in 0..count {
        let span = item_span(bucket, item_slot)
            .ok_or_else(|| KvError::Worker("Segment Bucket contains a malformed Item".into()))?;
        if !item_checksum_is_valid(bucket, span) {
            return Err(KvError::Worker(
                "Segment Item checksum does not match committed bytes".into(),
            ));
        }
        item_state_at(bucket, span)
            .ok_or_else(|| KvError::Worker("Segment Item has an invalid state".into()))?;
    }
    Ok(())
}

fn item_checksum_is_valid(bucket: &[u8], span: ItemSpan) -> bool {
    let Some(checksum_start) = span.end.checked_sub(ITEM_CHECKSUM_BYTES) else {
        return false;
    };
    let Some(stored) = bucket.get(checksum_start..span.end) else {
        return false;
    };
    let Ok(stored) = <[u8; ITEM_CHECKSUM_BYTES]>::try_from(stored) else {
        return false;
    };
    crc32fast::hash(&bucket[span.start..checksum_start]) == u32::from_le_bytes(stored)
}

fn write_item_checksum(bucket: &mut [u8], span: ItemSpan) {
    let checksum_start = span.end - ITEM_CHECKSUM_BYTES;
    let checksum = crc32fast::hash(&bucket[span.start..checksum_start]);
    bucket[checksum_start..span.end].copy_from_slice(&checksum.to_le_bytes());
}

fn matching_item_span(
    bucket: &[u8],
    storage_key: &[u8; STORAGE_KEY_BYTES],
    item_slot: usize,
) -> Option<ItemSpan> {
    let entry = item_offset(bucket, item_slot)?;
    if entry.key_prefix != storage_key[0] {
        return None;
    }
    let span = item_span(bucket, item_slot)?;
    (bucket[span.start..span.start + ITEM_STORAGE_KEY_SUFFIX_BYTES] == storage_key[1..])
        .then_some(span)
}

pub(crate) fn bucket_hash(
    storage_key: &StorageKey,
    bucket_hash_index: u8,
    bucket_count: usize,
) -> usize {
    BucketHashSequence::new(storage_key, bucket_count).get(bucket_hash_index)
}

pub(crate) struct BucketHashSequence {
    first: u64,
    second: u64,
    bucket_count: usize,
}

impl BucketHashSequence {
    pub(crate) fn new(storage_key: &StorageKey, bucket_count: usize) -> Self {
        let storage_key = storage_key.as_bytes();
        Self {
            first: u64::from_le_bytes(storage_key[16..24].try_into().unwrap()),
            second: u64::from_le_bytes(storage_key[24..32].try_into().unwrap()),
            bucket_count,
        }
    }

    pub(crate) fn get(&self, bucket_hash_index: u8) -> usize {
        let hash = match bucket_hash_index {
            0 => self.first,
            1 => self.second,
            index => self
                .first
                .wrapping_add(u64::from(index).wrapping_mul(self.second.rotate_left(32) | 1)),
        };
        hash as usize % self.bucket_count
    }
}
