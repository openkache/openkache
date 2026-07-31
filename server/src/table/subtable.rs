//! Cache-line-sized packed Subtable storage for the KVKache lookup Table.
//!
//! A Subtable stores unary occupancy, fingerprints, candidate locations, and
//! optional crumbs in one 64-byte cache line.

use crate::SUBTABLE_BYTES;
use crate::error::{KvError, Result};

#[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
use std::arch::x86_64::*;
#[cfg(all(
    target_arch = "aarch64",
    target_os = "linux",
    not(feature = "force-scalar")
))]
use std::arch::{aarch64::*, is_aarch64_feature_detected};

/// Candidate storage location encoded in a Table Entry.
///
/// The location selects a Segment and one of a key's Bucket hash functions.
/// The Bucket Item itself determines whether the value is inline or stored in
/// the paired Blob Segment.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TableLocation {
    pub(crate) sg_index: u16,
    pub(crate) bucket_hash_index: u8,
}

impl TableLocation {
    /// Returns whether all fields fit their configured packed ranges.
    pub(crate) fn is_valid(self, sg_index_bits: usize, bucket_choice_bits: usize) -> bool {
        (self.bucket_hash_index as usize) < (1usize << bucket_choice_bits)
            && (self.sg_index as usize) < (1usize << sg_index_bits)
    }

    /// Packs the Segment index and Bucket hash index.
    pub(crate) fn encode(self, sg_index_bits: usize, bucket_choice_bits: usize) -> u32 {
        debug_assert!(self.is_valid(sg_index_bits, bucket_choice_bits));
        ((self.sg_index as u32) << bucket_choice_bits) | self.bucket_hash_index as u32
    }

    /// Decodes a packed Table location.
    pub(crate) fn decode(value: u32, _sg_index_bits: usize, bucket_choice_bits: usize) -> Self {
        let bucket_choice_mask = (1u32 << bucket_choice_bits) - 1;
        Self {
            sg_index: (value >> bucket_choice_bits) as u16,
            bucket_hash_index: (value & bucket_choice_mask) as u8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SubtableEntry {
    pub(crate) unary_index: usize,
    pub(crate) fingerprint: u16,
    pub(crate) table_location: u32,
    pub(crate) crumb: u8,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SubtableLayout {
    pub(crate) unary_count: usize,
    pub(crate) entry_capacity: usize,
    pub(crate) fingerprint_bits: usize,
    pub(crate) table_location_bits: usize,
    pub(crate) crumb_bits: usize,
    pub(crate) unary_bytes: usize,
    pub(crate) fingerprint_bit: usize,
    pub(crate) table_location_bit: usize,
    pub(crate) crumb_bit: usize,
}

impl SubtableLayout {
    pub(crate) fn new(
        fingerprint_bits: usize,
        front_back_ratio: usize,
        is_back_table: bool,
        unary_count: usize,
        sg_index_bits: usize,
        bucket_choice_bits: usize,
    ) -> Result<Self> {
        let table_location_bits = sg_index_bits + bucket_choice_bits;
        let crumb_bits = if is_back_table {
            front_back_ratio.ilog2() as usize + 1
        } else {
            0
        };
        let mut chosen = None;
        for entry_capacity in 1..=64 {
            let unary_bits = unary_count + entry_capacity;
            if unary_bits > 128 {
                break;
            }
            let unary_bytes = unary_bits.div_ceil(8);
            let fingerprint_bytes = (entry_capacity * fingerprint_bits).div_ceil(8);
            let table_location_bytes = (entry_capacity * table_location_bits).div_ceil(8);
            let crumb_bytes = (entry_capacity * crumb_bits).div_ceil(8);
            if unary_bytes + fingerprint_bytes + table_location_bytes + crumb_bytes
                <= SUBTABLE_BYTES
            {
                chosen = Some((
                    entry_capacity,
                    unary_bytes,
                    fingerprint_bytes,
                    table_location_bytes,
                ));
            }
        }
        let Some((entry_capacity, unary_bytes, fingerprint_bytes, table_location_bytes)) = chosen
        else {
            return Err(KvError::InvalidConfig(
                "fingerprint/location/unary fields do not fit in a 64-byte Subtable".into(),
            ));
        };
        let fingerprint_bit = unary_bytes * 8;
        let table_location_bit = fingerprint_bit + fingerprint_bytes * 8;
        let crumb_bit = table_location_bit + table_location_bytes * 8;
        Ok(Self {
            unary_count,
            entry_capacity,
            fingerprint_bits,
            table_location_bits,
            crumb_bits,
            unary_bytes,
            fingerprint_bit,
            table_location_bit,
            crumb_bit,
        })
    }
}

#[repr(C, align(64))]
#[derive(Clone)]
pub(crate) struct Subtable {
    pub(crate) bytes: [u8; SUBTABLE_BYTES],
}

fn select_one(bits: u128, rank: usize) -> usize {
    #[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
    if is_x86_feature_detected!("bmi2") {
        return unsafe { select_one_bmi2(bits, rank) };
    }

    #[cfg(all(
        target_arch = "aarch64",
        target_os = "linux",
        not(feature = "force-scalar")
    ))]
    if is_aarch64_feature_detected!("sve2-bitperm") {
        return unsafe { select_one_sve2_bitperm(bits, rank) };
    }

    select_one_scalar(bits, rank)
}

fn select_one_scalar(mut bits: u128, rank: usize) -> usize {
    for _ in 0..rank {
        bits &= bits - 1;
    }
    bits.trailing_zeros() as usize
}

#[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
#[target_feature(enable = "bmi2")]
unsafe fn select_one_bmi2(bits: u128, rank: usize) -> usize {
    let low = bits as u64;
    let low_count = low.count_ones() as usize;
    if rank < low_count {
        _pdep_u64(1u64 << rank, low).trailing_zeros() as usize
    } else {
        let high = (bits >> 64) as u64;
        64 + _pdep_u64(1u64 << (rank - low_count), high).trailing_zeros() as usize
    }
}

#[cfg(all(
    target_arch = "aarch64",
    target_os = "linux",
    not(feature = "force-scalar")
))]
#[target_feature(enable = "sve,sve2,sve2-bitperm")]
unsafe fn select_one_sve2_bitperm(bits: u128, rank: usize) -> usize {
    let low = bits as u64;
    let low_count = low.count_ones() as usize;
    let (word, word_rank, offset) = if rank < low_count {
        (low, rank, 0)
    } else {
        ((bits >> 64) as u64, rank - low_count, 64)
    };
    let source = svdup_n_u64(1u64 << word_rank);
    let selected = svbdep_n_u64(source, word);
    offset + svlastb_u64(svptrue_b64(), selected).trailing_zeros() as usize
}

fn low_mask(bits: usize) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

impl Subtable {
    pub(crate) fn new(layout: &SubtableLayout) -> Self {
        let mut subtable = Self {
            bytes: [0; SUBTABLE_BYTES],
        };
        let separators = (1u128 << layout.unary_count) - 1;
        subtable.store_unary(layout, separators);
        subtable
    }

    fn unary(&self, layout: &SubtableLayout) -> u128 {
        let mut bytes = [0u8; 16];
        bytes[..layout.unary_bytes].copy_from_slice(&self.bytes[..layout.unary_bytes]);
        u128::from_le_bytes(bytes)
    }

    fn store_unary(&mut self, layout: &SubtableLayout, value: u128) {
        self.bytes[..layout.unary_bytes]
            .copy_from_slice(&value.to_le_bytes()[..layout.unary_bytes]);
    }

    pub(crate) fn entry_count(&self, layout: &SubtableLayout) -> usize {
        let bits = self.unary(layout);
        (128 - bits.leading_zeros() as usize) - layout.unary_count
    }

    pub(crate) fn bounds(&self, layout: &SubtableLayout, unary_index: usize) -> (usize, usize) {
        unary_bounds(self.unary(layout), unary_index)
    }

    pub(crate) fn entry(&self, layout: &SubtableLayout, entry_slot: usize) -> SubtableEntry {
        SubtableEntry {
            unary_index: self.unary_index_at(layout, entry_slot),
            fingerprint: self.fingerprint(layout, entry_slot),
            table_location: self.table_location(layout, entry_slot),
            crumb: self.crumb(layout, entry_slot),
        }
    }

    fn fingerprint(&self, layout: &SubtableLayout, entry_slot: usize) -> u16 {
        get_bits(
            &self.bytes,
            layout.fingerprint_bit + entry_slot * layout.fingerprint_bits,
            layout.fingerprint_bits,
        ) as u16
    }

    pub(crate) fn table_location(&self, layout: &SubtableLayout, entry_slot: usize) -> u32 {
        get_bits(
            &self.bytes,
            layout.table_location_bit + entry_slot * layout.table_location_bits,
            layout.table_location_bits,
        ) as u32
    }

    fn crumb(&self, layout: &SubtableLayout, entry_slot: usize) -> u8 {
        if layout.crumb_bits == 0 {
            0
        } else {
            get_bits(
                &self.bytes,
                layout.crumb_bit + entry_slot * layout.crumb_bits,
                layout.crumb_bits,
            ) as u8
        }
    }

    fn unary_index_at(&self, layout: &SubtableLayout, entry_slot: usize) -> usize {
        let bits = self.unary(layout);
        debug_assert!(entry_slot < self.entry_count(layout));
        let active_mask = low_mask(layout.unary_count + layout.entry_capacity);
        let entry_bit = select_one(!bits & active_mask, entry_slot);
        entry_bit - entry_slot
    }

    pub(crate) fn write_entry(
        &mut self,
        layout: &SubtableLayout,
        entry_slot: usize,
        entry: SubtableEntry,
    ) {
        set_bits(
            &mut self.bytes,
            layout.fingerprint_bit + entry_slot * layout.fingerprint_bits,
            layout.fingerprint_bits,
            entry.fingerprint as u64,
        );
        set_bits(
            &mut self.bytes,
            layout.table_location_bit + entry_slot * layout.table_location_bits,
            layout.table_location_bits,
            entry.table_location as u64,
        );
        if layout.crumb_bits > 0 {
            set_bits(
                &mut self.bytes,
                layout.crumb_bit + entry_slot * layout.crumb_bits,
                layout.crumb_bits,
                entry.crumb as u64,
            );
        }
    }

    fn clear_entry(&mut self, layout: &SubtableLayout, entry_slot: usize) {
        self.write_entry(
            layout,
            entry_slot,
            SubtableEntry {
                unary_index: 0,
                fingerprint: 0,
                table_location: 0,
                crumb: 0,
            },
        );
    }

    pub(crate) fn insert_front(
        &mut self,
        layout: &SubtableLayout,
        entry: SubtableEntry,
    ) -> Option<SubtableEntry> {
        let entry_count = self.entry_count(layout);
        let entry_slot = self.bounds(layout, entry.unary_index).0;
        if entry_slot == layout.entry_capacity {
            return Some(entry);
        }
        let overflow = (entry_count == layout.entry_capacity)
            .then(|| self.entry(layout, layout.entry_capacity - 1));
        let shift_end = entry_count.min(layout.entry_capacity - 1);
        for slot in (entry_slot..shift_end).rev() {
            let moved = self.entry(layout, slot);
            self.write_entry(layout, slot + 1, moved);
        }
        self.write_entry(layout, entry_slot, entry);
        self.unary_insert(layout, entry.unary_index, entry_slot);
        overflow
    }

    pub(crate) fn insert_back(&mut self, layout: &SubtableLayout, entry: SubtableEntry) -> bool {
        let entry_count = self.entry_count(layout);
        if entry_count == layout.entry_capacity {
            return false;
        }
        let entry_slot = self.bounds(layout, entry.unary_index).0;
        for slot in (entry_slot..entry_count).rev() {
            let moved = self.entry(layout, slot);
            self.write_entry(layout, slot + 1, moved);
        }
        self.write_entry(layout, entry_slot, entry);
        self.unary_insert(layout, entry.unary_index, entry_slot);
        true
    }

    pub(crate) fn matching_entry_slots<'a>(
        &'a self,
        layout: &'a SubtableLayout,
        unary_index: usize,
        fingerprint: u16,
        crumb: Option<u8>,
    ) -> impl Iterator<Item = usize> + 'a {
        let (start, end) = self.bounds(layout, unary_index);
        self.matching_entry_slots_in_range(layout, start..end, fingerprint, crumb)
    }

    pub(crate) fn matching_entry_slots_in_range<'a>(
        &'a self,
        layout: &'a SubtableLayout,
        range: std::ops::Range<usize>,
        fingerprint: u16,
        crumb: Option<u8>,
    ) -> impl Iterator<Item = usize> + 'a {
        range.filter(move |entry_slot| {
            self.fingerprint(layout, *entry_slot) == fingerprint
                && crumb.is_none_or(|candidate| self.crumb(layout, *entry_slot) == candidate)
        })
    }

    pub(crate) fn first_with_crumb(
        &self,
        layout: &SubtableLayout,
        crumb: u8,
    ) -> Option<(usize, SubtableEntry)> {
        let entry_slot = (0..self.entry_count(layout))
            .find(|entry_slot| self.crumb(layout, *entry_slot) == crumb)?;
        Some((entry_slot, self.entry(layout, entry_slot)))
    }

    pub(crate) fn remove_at(
        &mut self,
        layout: &SubtableLayout,
        unary_index: usize,
        entry_slot: usize,
    ) -> SubtableEntry {
        let entry_count = self.entry_count(layout);
        let removed = self.entry(layout, entry_slot);
        for slot in entry_slot..entry_count - 1 {
            let moved = self.entry(layout, slot + 1);
            self.write_entry(layout, slot, moved);
        }
        self.clear_entry(layout, entry_count - 1);
        self.unary_remove(layout, unary_index, entry_slot);
        removed
    }

    fn unary_insert(&mut self, layout: &SubtableLayout, unary_index: usize, entry_slot: usize) {
        let bits = self.unary(layout);
        let bit_index = unary_index + entry_slot;
        let lower_mask_value = (1u128 << bit_index) - 1;
        let total_bits = layout.unary_count + layout.entry_capacity;
        let active_mask = low_mask(total_bits);
        let mut shifted =
            ((bits & lower_mask_value) | ((bits & !lower_mask_value) << 1)) & active_mask;
        if self.entry_count(layout) == layout.entry_capacity {
            let zeros = !shifted & active_mask;
            let last_zero = 127 - zeros.leading_zeros() as usize;
            shifted |= 1u128 << last_zero;
        }
        self.store_unary(layout, shifted);
    }

    fn unary_remove(&mut self, layout: &SubtableLayout, unary_index: usize, entry_slot: usize) {
        let bits = self.unary(layout);
        let bit_index = unary_index + entry_slot;
        let lower_mask_value = (1u128 << bit_index) - 1;
        let lower = bits & lower_mask_value;
        let upper = (bits >> (bit_index + 1)) << bit_index;
        self.store_unary(layout, lower | upper);
    }
}

fn unary_bounds(bits: u128, unary_index: usize) -> (usize, usize) {
    let end = select_one(bits, unary_index) - unary_index;
    let start = if unary_index == 0 {
        0
    } else {
        select_one(bits, unary_index - 1) - (unary_index - 1)
    };
    (start, end)
}

fn get_bits(bytes: &[u8], bit: usize, width: usize) -> u64 {
    if width == 0 {
        return 0;
    }
    let byte = bit / 8;
    let shift = bit % 8;
    debug_assert!(shift + width <= u64::BITS as usize);
    let byte_len = (shift + width).div_ceil(8);
    let mut word = [0; 8];
    word[..byte_len].copy_from_slice(&bytes[byte..byte + byte_len]);
    let mask = if width == u64::BITS as usize {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    (u64::from_le_bytes(word) >> shift) & mask
}

fn set_bits(bytes: &mut [u8], bit: usize, width: usize, value: u64) {
    if width == 0 {
        return;
    }
    let byte = bit / 8;
    let shift = bit % 8;
    debug_assert!(shift + width <= u64::BITS as usize);
    let byte_len = (shift + width).div_ceil(8);
    let mut word = [0; 8];
    word[..byte_len].copy_from_slice(&bytes[byte..byte + byte_len]);
    let field_mask = if width == u64::BITS as usize {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    let shifted_mask = field_mask << shift;
    let updated = (u64::from_le_bytes(word) & !shifted_mask) | ((value << shift) & shifted_mask);
    bytes[byte..byte + byte_len].copy_from_slice(&updated.to_le_bytes()[..byte_len]);
}
