//! Fixed-size Breadcrumb Table from a key hash to opaque packed values.
//!
//! Keep this module independent of `StorageState`, `StorageKey`, and the meaning of a value.
//! The caller supplies only the metadata bit widths, a `u128` key hash, and a packed `u32`.
//!
//! Implementation rules:
//!
//! - keep the five public operations top-down and make their state transitions visible;
//! - extract only repeated bit-packing or Front/Back mechanics, never one-use wrappers;
//! - keep all layout, routing, collision, and eviction decisions inside this module;
//! - never return `TableFull`: insertion evicts the maximum caller score from the current
//!   Front and its two physical Back candidates, then must admit the new Entry;
//! - treat `(table coordinates, fingerprint, value)` as the exact compact identity;
//! - preserve multiplicity: `remove_one` removes one exact Entry and `remove_all` removes all.

use std::mem::size_of;
use std::num::NonZeroU64;

use smallvec::SmallVec;

const SUBTABLE_BYTES: usize = 64;
const TARGET_LOAD_PERCENT: usize = 88;
const MIN_UNARY_COUNT: usize = 2;
const MAX_UNARY_COUNT: usize = 96;
const FRONT_BACK_RATIOS: [usize; 4] = [2, 4, 8, 16];

/// Entry count and metadata widths used to derive the smallest valid Table layout.
#[derive(Clone, Copy, Debug)]
pub(super) struct TableConfig {
    pub(super) max_entries: usize,
    pub(super) value_bits: u8,
    pub(super) fingerprint_bits: u8,
}

impl TableConfig {
    fn validate(self) -> Result<(), TableCreateError> {
        if self.max_entries == 0 {
            return Err(TableCreateError::InvalidConfig(
                "max_entries must be non-zero",
            ));
        }
        if !(1..=32).contains(&self.value_bits) {
            return Err(TableCreateError::InvalidConfig(
                "value_bits must be between 1 and 32",
            ));
        }
        if !(1..=32).contains(&self.fingerprint_bits) {
            return Err(TableCreateError::InvalidConfig(
                "fingerprint_bits must be between 1 and 32",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TableCreateError {
    InvalidConfig(&'static str),
    NoValidLayout,
    AllocationFailed,
}

struct TableAllocation {
    front_subtable_layout: SubtableLayout,
    back_subtable_layout: SubtableLayout,
    front_subtable_count: usize,
    back_subtable_count: usize,
    back_subtable_group_count: usize,
    front_back_ratio: usize,
}

impl TableAllocation {
    fn new(config: TableConfig) -> Result<Self, TableCreateError> {
        let mut selected: Option<(usize, usize, Self)> = None;

        for front_back_ratio in FRONT_BACK_RATIOS {
            for unary_count in MIN_UNARY_COUNT..=MAX_UNARY_COUNT {
                let Some(front_subtable_layout) = SubtableLayout::new(
                    config.fingerprint_bits as usize,
                    config.value_bits as usize,
                    unary_count,
                    0,
                ) else {
                    continue;
                };
                let Some(back_subtable_layout) = SubtableLayout::new(
                    config.fingerprint_bits as usize,
                    config.value_bits as usize,
                    unary_count,
                    front_back_ratio.ilog2() as usize + 1,
                ) else {
                    continue;
                };

                let entry_units = front_subtable_layout.entry_capacity * front_back_ratio
                    + back_subtable_layout.entry_capacity;
                let numerator = config.max_entries as u128 * front_back_ratio as u128 * 100;
                let denominator = entry_units as u128 * TARGET_LOAD_PERCENT as u128;
                let Ok(front_subtable_count) = usize::try_from(numerator.div_ceil(denominator))
                else {
                    return Err(TableCreateError::NoValidLayout);
                };
                let front_subtable_count = front_subtable_count.max(1);
                let back_subtable_group_count = front_subtable_count
                    .div_ceil(front_back_ratio * front_back_ratio)
                    .max(1);
                let back_subtable_count = front_subtable_count
                    .div_ceil(front_back_ratio)
                    .max(front_back_ratio * back_subtable_group_count);
                let subtable_count = front_subtable_count
                    .checked_add(back_subtable_count)
                    .ok_or(TableCreateError::NoValidLayout)?;
                let allocation = Self {
                    front_subtable_layout,
                    back_subtable_layout,
                    front_subtable_count,
                    back_subtable_count,
                    back_subtable_group_count,
                    front_back_ratio,
                };

                let replace = selected.as_ref().is_none_or(
                    |(selected_count, selected_unary_count, selected_allocation)| {
                        subtable_count < *selected_count
                            || (subtable_count == *selected_count
                                && (unary_count.is_power_of_two()
                                    && !selected_unary_count.is_power_of_two()
                                    || unary_count.is_power_of_two()
                                        == selected_unary_count.is_power_of_two()
                                        && (unary_count > *selected_unary_count
                                            || unary_count == *selected_unary_count
                                                && front_back_ratio
                                                    < selected_allocation.front_back_ratio)))
                    },
                );
                if replace {
                    selected = Some((subtable_count, unary_count, allocation));
                }
            }
        }

        selected
            .map(|(_, _, allocation)| allocation)
            .ok_or(TableCreateError::NoValidLayout)
    }
}

/// A fixed-size, cache-line-partitioned compact index.
pub(super) struct Table {
    front_table: Box<[Subtable]>,
    back_table: Box<[Subtable]>,
    front_subtable_layout: SubtableLayout,
    back_subtable_layout: SubtableLayout,
    back_subtable_group_count: usize,
    front_back_shift: u32,
    fingerprint_mask: u32,
    coordinate_modulus: NonZeroU64,
    value_mask: u32,
    avx512_byte_match: bool,
}

impl Table {
    /// Derives the cache-line layout from the configured fingerprint and value widths.
    pub(super) fn new(config: TableConfig) -> Result<Self, TableCreateError> {
        config.validate()?;
        let allocation = TableAllocation::new(config)?;
        let coordinate_count = allocation
            .front_subtable_count
            .checked_mul(allocation.front_subtable_layout.unary_count)
            .and_then(|count| u64::try_from(count).ok())
            .and_then(NonZeroU64::new)
            .ok_or(TableCreateError::NoValidLayout)?;

        let mut front_table = Vec::new();
        front_table
            .try_reserve_exact(allocation.front_subtable_count)
            .map_err(|_| TableCreateError::AllocationFailed)?;
        front_table.resize_with(allocation.front_subtable_count, || {
            Subtable::new(&allocation.front_subtable_layout)
        });

        let mut back_table = Vec::new();
        back_table
            .try_reserve_exact(allocation.back_subtable_count)
            .map_err(|_| TableCreateError::AllocationFailed)?;
        back_table.resize_with(allocation.back_subtable_count, || {
            Subtable::new(&allocation.back_subtable_layout)
        });

        let avx512_byte_match = config.fingerprint_bits == 8 && {
            #[cfg(target_arch = "x86_64")]
            {
                is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512bw")
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                false
            }
        };

        Ok(Self {
            front_table: front_table.into_boxed_slice(),
            back_table: back_table.into_boxed_slice(),
            front_subtable_layout: allocation.front_subtable_layout,
            back_subtable_layout: allocation.back_subtable_layout,
            back_subtable_group_count: allocation.back_subtable_group_count,
            front_back_shift: allocation.front_back_ratio.trailing_zeros(),
            fingerprint_mask: u32_mask(config.fingerprint_bits as usize),
            coordinate_modulus: coordinate_count,
            value_mask: u32_mask(config.value_bits as usize),
            avx512_byte_match,
        })
    }

    /// Returns matching packed values inline, spilling only after four candidates.
    pub(super) fn candidates(&self, hash: u128) -> SmallVec<[u32; 4]> {
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(hash);
        let mut candidates = SmallVec::new();
        let front_unary = self.front_table[front_subtable_index].unary(&self.front_subtable_layout);
        let front_entry_count =
            (128 - front_unary.leading_zeros() as usize) - self.front_subtable_layout.unary_count;

        self.front_table[front_subtable_index].append_candidates(
            &self.front_subtable_layout,
            front_unary,
            unary_index,
            fingerprint,
            None,
            self.avx512_byte_match,
            &mut candidates,
        );

        if front_entry_count != self.front_subtable_layout.entry_capacity {
            return candidates;
        }
        for (back_subtable_index, crumb) in self.back_subtable_routes(front_subtable_index) {
            let back_unary = self.back_table[back_subtable_index].unary(&self.back_subtable_layout);
            self.back_table[back_subtable_index].append_candidates(
                &self.back_subtable_layout,
                back_unary,
                unary_index,
                fingerprint,
                Some(crumb),
                self.avx512_byte_match,
                &mut candidates,
            );
        }
        candidates
    }

    /// Inserts unconditionally, evicting the maximum-scored local Entry when saturated.
    pub(super) fn insert_with_eviction_score(
        &mut self,
        hash: u128,
        value: u32,
        eviction_score: impl Fn(u32) -> usize,
    ) {
        self.assert_value_fits(value);
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(hash);
        let entry = SubtableEntry {
            unary_index,
            fingerprint,
            value,
            crumb: 0,
        };

        if self.insert_without_eviction(front_subtable_index, entry) {
            return;
        }

        // A failed normal insertion means Front and both Back choices are full. A Front victim
        // wins score ties because replacing it avoids a second Front-to-Back displacement.
        let mut victim = None;
        for entry_slot in 0..self.front_subtable_layout.entry_capacity {
            let candidate_value = self.front_table[front_subtable_index]
                .value(&self.front_subtable_layout, entry_slot);
            let score = eviction_score(candidate_value);
            if victim
                .as_ref()
                .is_none_or(|(best_score, _, _, _)| score > *best_score)
            {
                victim = Some((score, false, front_subtable_index, entry_slot));
            }
        }

        let [first, second] = self.back_subtable_routes(front_subtable_index);
        for back_subtable_index in [first.0, second.0]
            .into_iter()
            .take(if first.0 == second.0 { 1 } else { 2 })
        {
            for entry_slot in 0..self.back_subtable_layout.entry_capacity {
                let candidate_value = self.back_table[back_subtable_index]
                    .value(&self.back_subtable_layout, entry_slot);
                let score = eviction_score(candidate_value);
                if victim
                    .as_ref()
                    .is_none_or(|(best_score, _, _, _)| score > *best_score)
                {
                    victim = Some((score, true, back_subtable_index, entry_slot));
                }
            }
        }

        let (_, from_back, subtable_index, entry_slot) =
            victim.expect("a saturated Table route must contain an eviction victim");
        if from_back {
            let unary_index = self.back_table[subtable_index]
                .unary_index_at(&self.back_subtable_layout, entry_slot);
            self.back_table[subtable_index].remove_at(
                &self.back_subtable_layout,
                unary_index,
                entry_slot,
            );
        } else {
            let unary_index = self.front_table[subtable_index]
                .unary_index_at(&self.front_subtable_layout, entry_slot);
            self.front_table[subtable_index].remove_at(
                &self.front_subtable_layout,
                unary_index,
                entry_slot,
            );
        }

        assert!(
            self.insert_without_eviction(front_subtable_index, entry),
            "freeing one local slot must make the new Table Entry admissible"
        );
    }

    /// Removes the first exact compact identity, preserving duplicate multiplicity.
    pub(super) fn remove_one(&mut self, hash: u128, value: u32) -> bool {
        self.assert_value_fits(value);
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(hash);
        let front_unary = self.front_table[front_subtable_index].unary(&self.front_subtable_layout);
        let was_full = (128 - front_unary.leading_zeros() as usize)
            - self.front_subtable_layout.unary_count
            == self.front_subtable_layout.entry_capacity;

        let mut matches = self.front_table[front_subtable_index].matching_entry_mask(
            &self.front_subtable_layout,
            front_unary,
            unary_index,
            fingerprint,
            None,
            self.avx512_byte_match,
        );
        while matches != 0 {
            let entry_slot = matches.trailing_zeros() as usize;
            matches &= matches - 1;
            if self.front_table[front_subtable_index].value(&self.front_subtable_layout, entry_slot)
                == value
            {
                self.front_table[front_subtable_index].remove_at(
                    &self.front_subtable_layout,
                    unary_index,
                    entry_slot,
                );
                if was_full {
                    self.promote(front_subtable_index);
                }
                return true;
            }
        }
        if !was_full {
            return false;
        }

        for (back_subtable_index, crumb) in self.back_subtable_routes(front_subtable_index) {
            let back_unary = self.back_table[back_subtable_index].unary(&self.back_subtable_layout);
            let mut matches = self.back_table[back_subtable_index].matching_entry_mask(
                &self.back_subtable_layout,
                back_unary,
                unary_index,
                fingerprint,
                Some(crumb),
                self.avx512_byte_match,
            );
            while matches != 0 {
                let entry_slot = matches.trailing_zeros() as usize;
                matches &= matches - 1;
                if self.back_table[back_subtable_index]
                    .value(&self.back_subtable_layout, entry_slot)
                    == value
                {
                    self.back_table[back_subtable_index].remove_at(
                        &self.back_subtable_layout,
                        unary_index,
                        entry_slot,
                    );
                    return true;
                }
            }
        }
        false
    }

    /// Removes every Entry with the exact compact identity in one Front route.
    pub(super) fn remove_all(&mut self, hash: u128, value: u32) -> usize {
        self.assert_value_fits(value);
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(hash);
        let mut removed = 0;
        let mut removed_from_front = 0;

        let front_unary = self.front_table[front_subtable_index].unary(&self.front_subtable_layout);
        let mut matches = self.front_table[front_subtable_index].matching_entry_mask(
            &self.front_subtable_layout,
            front_unary,
            unary_index,
            fingerprint,
            None,
            self.avx512_byte_match,
        );
        while matches != 0 {
            let entry_slot = u64::BITS as usize - 1 - matches.leading_zeros() as usize;
            matches &= !(1u64 << entry_slot);
            if self.front_table[front_subtable_index].value(&self.front_subtable_layout, entry_slot)
                == value
            {
                self.front_table[front_subtable_index].remove_at(
                    &self.front_subtable_layout,
                    unary_index,
                    entry_slot,
                );
                removed += 1;
                removed_from_front += 1;
            }
        }

        for (back_subtable_index, crumb) in self.back_subtable_routes(front_subtable_index) {
            let back_unary = self.back_table[back_subtable_index].unary(&self.back_subtable_layout);
            let mut matches = self.back_table[back_subtable_index].matching_entry_mask(
                &self.back_subtable_layout,
                back_unary,
                unary_index,
                fingerprint,
                Some(crumb),
                self.avx512_byte_match,
            );
            while matches != 0 {
                let entry_slot = u64::BITS as usize - 1 - matches.leading_zeros() as usize;
                matches &= !(1u64 << entry_slot);
                if self.back_table[back_subtable_index]
                    .value(&self.back_subtable_layout, entry_slot)
                    == value
                {
                    self.back_table[back_subtable_index].remove_at(
                        &self.back_subtable_layout,
                        unary_index,
                        entry_slot,
                    );
                    removed += 1;
                }
            }
        }

        if removed_from_front != 0 {
            while self.front_table[front_subtable_index].entry_count(&self.front_subtable_layout)
                < self.front_subtable_layout.entry_capacity
                && self.promote(front_subtable_index)
            {}
        }
        removed
    }

    /// Attempts the ordinary Breadcrumb Front insertion and restores Front on Back overflow.
    fn insert_without_eviction(
        &mut self,
        front_subtable_index: usize,
        entry: SubtableEntry,
    ) -> bool {
        let front_entry_count =
            self.front_table[front_subtable_index].entry_count(&self.front_subtable_layout);
        if front_entry_count < self.front_subtable_layout.entry_capacity {
            let overflow = self.front_table[front_subtable_index]
                .insert_front(&self.front_subtable_layout, entry);
            debug_assert!(overflow.is_none());
            return true;
        }

        let [first, second] = self.back_subtable_routes(front_subtable_index);
        let first_entry_count = self.back_table[first.0].entry_count(&self.back_subtable_layout);
        let second_entry_count = self.back_table[second.0].entry_count(&self.back_subtable_layout);
        let (destination, destination_entry_count) = if first_entry_count <= second_entry_count {
            (first, first_entry_count)
        } else {
            (second, second_entry_count)
        };
        if destination_entry_count == self.back_subtable_layout.entry_capacity {
            return false;
        }

        let mut overflow = self.front_table[front_subtable_index]
            .insert_front(&self.front_subtable_layout, entry)
            .expect("a full Front must displace one Entry");
        overflow.crumb = destination.1;
        let inserted =
            self.back_table[destination.0].insert_back(&self.back_subtable_layout, overflow);
        assert!(
            inserted,
            "a Back candidate checked below capacity must accept one Entry"
        );
        true
    }

    /// Promotes one Back Entry routed to this Front, if one exists.
    fn promote(&mut self, front_subtable_index: usize) -> bool {
        let [first, second] = self.back_subtable_routes(front_subtable_index);
        let first_entry =
            self.back_table[first.0].first_with_crumb(&self.back_subtable_layout, first.1);
        let second_entry =
            self.back_table[second.0].first_with_crumb(&self.back_subtable_layout, second.1);
        let selected = match (first_entry, second_entry) {
            (None, None) => return false,
            (Some(candidate), None) => (first.0, candidate),
            (None, Some(candidate)) => (second.0, candidate),
            (Some(first_entry), Some(second_entry))
                if first_entry.1.unary_index <= second_entry.1.unary_index =>
            {
                (first.0, first_entry)
            }
            (Some(_), Some(second_entry)) => (second.0, second_entry),
        };
        let (back_subtable_index, (entry_slot, mut entry)) = selected;
        self.back_table[back_subtable_index].remove_at(
            &self.back_subtable_layout,
            entry.unary_index,
            entry_slot,
        );
        entry.crumb = 0;
        let overflow =
            self.front_table[front_subtable_index].insert_front(&self.front_subtable_layout, entry);
        assert!(overflow.is_none(), "promotion requires one free Front slot");
        true
    }

    fn table_coordinates(&self, hash: u128) -> (usize, usize, u32) {
        let fingerprint = hash as u32 & self.fingerprint_mask;
        let coordinate_hash = (hash >> self.front_subtable_layout.fingerprint_bits) as u64;
        let coordinate = ((u128::from(coordinate_hash) * u128::from(self.coordinate_modulus.get()))
            >> u64::BITS) as usize;
        let unary_count = self.front_subtable_layout.unary_count;
        let (front_subtable_index, unary_index) = if unary_count.is_power_of_two() {
            let shift = unary_count.trailing_zeros();
            (coordinate >> shift, coordinate & (unary_count - 1))
        } else {
            (coordinate / unary_count, coordinate % unary_count)
        };
        (front_subtable_index, unary_index, fingerprint)
    }

    fn assert_value_fits(&self, value: u32) {
        assert!(
            value & !self.value_mask == 0,
            "Table value does not fit configured value_bits"
        );
    }

    fn back_subtable_routes(&self, front_subtable_index: usize) -> [(usize, u8); 2] {
        let front_back_ratio = 1usize << self.front_back_shift;
        let low_mask = front_back_ratio - 1;
        let upper = front_subtable_index >> self.front_back_shift;
        let low = front_subtable_index & low_mask;
        let first = (upper, (front_back_ratio + low) as u8);
        let second = (
            (upper >> self.front_back_shift) + low * self.back_subtable_group_count,
            (upper & low_mask) as u8,
        );
        debug_assert!(first.0 < self.back_table.len());
        debug_assert!(second.0 < self.back_table.len());
        [first, second]
    }
}

#[derive(Clone, Copy, Debug)]
struct SubtableEntry {
    unary_index: usize,
    fingerprint: u32,
    value: u32,
    crumb: u8,
}

#[derive(Clone, Copy, Debug)]
struct SubtableLayout {
    unary_count: usize,
    entry_capacity: usize,
    fingerprint_bits: usize,
    value_bits: usize,
    crumb_bits: usize,
    unary_bytes: usize,
    fingerprint_bit: usize,
    value_bit: usize,
    crumb_bit: usize,
    bmi2_select: bool,
}

impl SubtableLayout {
    fn new(
        fingerprint_bits: usize,
        value_bits: usize,
        unary_count: usize,
        crumb_bits: usize,
    ) -> Option<Self> {
        let mut chosen = None;
        for entry_capacity in 1..=64 {
            let unary_bits = unary_count + entry_capacity;
            if unary_bits > 128 {
                break;
            }
            let unary_bytes = unary_bits.div_ceil(8);
            let fingerprint_bytes = (entry_capacity * fingerprint_bits).div_ceil(8);
            let value_bytes = (entry_capacity * value_bits).div_ceil(8);
            let crumb_bytes = (entry_capacity * crumb_bits).div_ceil(8);
            if unary_bytes + fingerprint_bytes + value_bytes + crumb_bytes <= SUBTABLE_BYTES {
                chosen = Some((entry_capacity, unary_bytes, fingerprint_bytes, value_bytes));
            }
        }
        let (entry_capacity, unary_bytes, fingerprint_bytes, value_bytes) = chosen?;
        let fingerprint_bit = unary_bytes * 8;
        let value_bit = fingerprint_bit + fingerprint_bytes * 8;
        let crumb_bit = value_bit + value_bytes * 8;
        let bmi2_select = {
            #[cfg(target_arch = "x86_64")]
            {
                is_x86_feature_detected!("bmi2")
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                false
            }
        };
        Some(Self {
            unary_count,
            entry_capacity,
            fingerprint_bits,
            value_bits,
            crumb_bits,
            unary_bytes,
            fingerprint_bit,
            value_bit,
            crumb_bit,
            bmi2_select,
        })
    }
}

#[repr(C, align(64))]
#[derive(Clone)]
struct Subtable {
    bytes: [u8; SUBTABLE_BYTES],
}

impl Subtable {
    fn new(layout: &SubtableLayout) -> Self {
        let mut subtable = Self {
            bytes: [0; SUBTABLE_BYTES],
        };
        subtable.store_unary(layout, (1u128 << layout.unary_count) - 1);
        subtable
    }

    fn unary(&self, layout: &SubtableLayout) -> u128 {
        if layout.unary_bytes <= size_of::<u64>() {
            // SAFETY: every Subtable contains at least eight bytes from its aligned base.
            let bits = u64::from_le(unsafe { self.bytes.as_ptr().cast::<u64>().read_unaligned() });
            let width = layout.unary_bytes * u8::BITS as usize;
            let mask = if width == u64::BITS as usize {
                u64::MAX
            } else {
                (1u64 << width) - 1
            };
            return u128::from(bits & mask);
        }

        // SAFETY: every Subtable contains at least 16 bytes from its aligned base.
        let bits = u128::from_le(unsafe { self.bytes.as_ptr().cast::<u128>().read_unaligned() });
        bits & low_mask(layout.unary_bytes * u8::BITS as usize)
    }

    fn store_unary(&mut self, layout: &SubtableLayout, value: u128) {
        let width = layout.unary_bytes * u8::BITS as usize;
        if layout.unary_bytes <= size_of::<u64>() {
            let mask = if width == u64::BITS as usize {
                u64::MAX
            } else {
                (1u64 << width) - 1
            };
            let pointer = self.bytes.as_mut_ptr().cast::<u64>();
            // SAFETY: every Subtable contains at least eight bytes from its aligned base.
            let current = u64::from_le(unsafe { pointer.read_unaligned() });
            let updated = (current & !mask) | (value as u64 & mask);
            unsafe { pointer.write_unaligned(updated.to_le()) };
            return;
        }

        let mask = low_mask(width);
        let pointer = self.bytes.as_mut_ptr().cast::<u128>();
        // SAFETY: every Subtable contains at least 16 bytes from its aligned base.
        let current = u128::from_le(unsafe { pointer.read_unaligned() });
        let updated = (current & !mask) | (value & mask);
        unsafe { pointer.write_unaligned(updated.to_le()) };
    }

    fn entry_count(&self, layout: &SubtableLayout) -> usize {
        let bits = self.unary(layout);
        (128 - bits.leading_zeros() as usize) - layout.unary_count
    }

    fn entry(&self, layout: &SubtableLayout, entry_slot: usize) -> SubtableEntry {
        SubtableEntry {
            unary_index: self.unary_index_at(layout, entry_slot),
            fingerprint: self.fingerprint(layout, entry_slot),
            value: self.value(layout, entry_slot),
            crumb: self.crumb(layout, entry_slot),
        }
    }

    fn fingerprint(&self, layout: &SubtableLayout, entry_slot: usize) -> u32 {
        get_bits(
            &self.bytes,
            layout.fingerprint_bit + entry_slot * layout.fingerprint_bits,
            layout.fingerprint_bits,
        ) as u32
    }

    fn value(&self, layout: &SubtableLayout, entry_slot: usize) -> u32 {
        get_bits(
            &self.bytes,
            layout.value_bit + entry_slot * layout.value_bits,
            layout.value_bits,
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
        let entry_bit = select_one(!bits & active_mask, entry_slot, layout.bmi2_select);
        entry_bit - entry_slot
    }

    fn write_entry(&mut self, layout: &SubtableLayout, entry_slot: usize, entry: SubtableEntry) {
        set_bits(
            &mut self.bytes,
            layout.fingerprint_bit + entry_slot * layout.fingerprint_bits,
            layout.fingerprint_bits,
            entry.fingerprint as u64,
        );
        set_bits(
            &mut self.bytes,
            layout.value_bit + entry_slot * layout.value_bits,
            layout.value_bits,
            entry.value as u64,
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

    fn insert_front(
        &mut self,
        layout: &SubtableLayout,
        entry: SubtableEntry,
    ) -> Option<SubtableEntry> {
        let unary = self.unary(layout);
        let entry_count = (128 - unary.leading_zeros() as usize) - layout.unary_count;
        let entry_slot = unary_bounds(unary, entry.unary_index, layout.bmi2_select).0;
        if entry_slot == layout.entry_capacity {
            return Some(entry);
        }
        let overflow = (entry_count == layout.entry_capacity)
            .then(|| self.entry(layout, layout.entry_capacity - 1));
        let shift_end = entry_count.min(layout.entry_capacity - 1);
        shift_bits_right(
            &mut self.bytes,
            layout.fingerprint_bit + entry_slot * layout.fingerprint_bits,
            layout.fingerprint_bit + shift_end * layout.fingerprint_bits,
            layout.fingerprint_bits,
        );
        shift_bits_right(
            &mut self.bytes,
            layout.value_bit + entry_slot * layout.value_bits,
            layout.value_bit + shift_end * layout.value_bits,
            layout.value_bits,
        );
        shift_bits_right(
            &mut self.bytes,
            layout.crumb_bit + entry_slot * layout.crumb_bits,
            layout.crumb_bit + shift_end * layout.crumb_bits,
            layout.crumb_bits,
        );
        self.write_entry(layout, entry_slot, entry);
        self.unary_insert(layout, unary, entry_count, entry.unary_index, entry_slot);
        overflow
    }

    fn insert_back(&mut self, layout: &SubtableLayout, entry: SubtableEntry) -> bool {
        let unary = self.unary(layout);
        let entry_count = (128 - unary.leading_zeros() as usize) - layout.unary_count;
        if entry_count == layout.entry_capacity {
            return false;
        }
        let entry_slot = unary_bounds(unary, entry.unary_index, layout.bmi2_select).0;
        shift_bits_right(
            &mut self.bytes,
            layout.fingerprint_bit + entry_slot * layout.fingerprint_bits,
            layout.fingerprint_bit + entry_count * layout.fingerprint_bits,
            layout.fingerprint_bits,
        );
        shift_bits_right(
            &mut self.bytes,
            layout.value_bit + entry_slot * layout.value_bits,
            layout.value_bit + entry_count * layout.value_bits,
            layout.value_bits,
        );
        shift_bits_right(
            &mut self.bytes,
            layout.crumb_bit + entry_slot * layout.crumb_bits,
            layout.crumb_bit + entry_count * layout.crumb_bits,
            layout.crumb_bits,
        );
        self.write_entry(layout, entry_slot, entry);
        self.unary_insert(layout, unary, entry_count, entry.unary_index, entry_slot);
        true
    }

    fn matching_entry_mask(
        &self,
        layout: &SubtableLayout,
        unary: u128,
        unary_index: usize,
        fingerprint: u32,
        crumb: Option<u8>,
        avx512_byte_match: bool,
    ) -> u64 {
        let (start, end) = unary_bounds(unary, unary_index, layout.bmi2_select);

        let mut matches = if layout.fingerprint_bits == 8 {
            if avx512_byte_match {
                let end_mask = if end == u64::BITS as usize {
                    u64::MAX
                } else {
                    (1u64 << end) - 1
                };
                let range_mask = end_mask & !((1u64 << start) - 1);
                #[cfg(target_arch = "x86_64")]
                {
                    // SAFETY: Table creation enables this path only after detecting AVX-512F/BW.
                    range_mask
                        & unsafe {
                            fingerprint_matches_avx512(&self.bytes, fingerprint as u8)
                                >> (layout.fingerprint_bit / 8)
                        }
                }
                #[cfg(not(target_arch = "x86_64"))]
                {
                    unreachable!("AVX-512 byte matching is x86_64-only")
                }
            } else {
                let fingerprints = &self.bytes[layout.fingerprint_bit / 8..];
                (start..end)
                    .filter(|entry_slot| fingerprints[*entry_slot] == fingerprint as u8)
                    .fold(0, |mask, entry_slot| mask | (1u64 << entry_slot))
            }
        } else {
            (start..end)
                .filter(|entry_slot| self.fingerprint(layout, *entry_slot) == fingerprint)
                .fold(0, |mask, entry_slot| mask | (1u64 << entry_slot))
        };

        if let Some(crumb) = crumb {
            let mut unchecked = matches;
            while unchecked != 0 {
                let entry_slot = unchecked.trailing_zeros() as usize;
                unchecked &= unchecked - 1;
                if self.crumb(layout, entry_slot) != crumb {
                    matches &= !(1u64 << entry_slot);
                }
            }
        }
        matches
    }

    fn append_candidates(
        &self,
        layout: &SubtableLayout,
        unary: u128,
        unary_index: usize,
        fingerprint: u32,
        crumb: Option<u8>,
        avx512_byte_match: bool,
        candidates: &mut SmallVec<[u32; 4]>,
    ) {
        if layout.fingerprint_bits == 8 && avx512_byte_match {
            let mut matches =
                self.matching_entry_mask(layout, unary, unary_index, fingerprint, crumb, true);
            while matches != 0 {
                let entry_slot = matches.trailing_zeros() as usize;
                matches &= matches - 1;
                candidates.push(self.value(layout, entry_slot));
            }
            return;
        }

        let (start, end) = unary_bounds(unary, unary_index, layout.bmi2_select);
        for entry_slot in start..end {
            let fingerprint_matches = if layout.fingerprint_bits == 8 {
                self.bytes[layout.fingerprint_bit / 8 + entry_slot] == fingerprint as u8
            } else {
                self.fingerprint(layout, entry_slot) == fingerprint
            };
            if fingerprint_matches
                && crumb.is_none_or(|crumb| self.crumb(layout, entry_slot) == crumb)
            {
                candidates.push(self.value(layout, entry_slot));
            }
        }
    }

    fn first_with_crumb(
        &self,
        layout: &SubtableLayout,
        crumb: u8,
    ) -> Option<(usize, SubtableEntry)> {
        let entry_slot = (0..self.entry_count(layout))
            .find(|entry_slot| self.crumb(layout, *entry_slot) == crumb)?;
        Some((entry_slot, self.entry(layout, entry_slot)))
    }

    fn remove_at(&mut self, layout: &SubtableLayout, unary_index: usize, entry_slot: usize) {
        let unary = self.unary(layout);
        let entry_count = (128 - unary.leading_zeros() as usize) - layout.unary_count;
        shift_bits_left(
            &mut self.bytes,
            layout.fingerprint_bit + (entry_slot + 1) * layout.fingerprint_bits,
            layout.fingerprint_bit + entry_count * layout.fingerprint_bits,
            layout.fingerprint_bits,
        );
        shift_bits_left(
            &mut self.bytes,
            layout.value_bit + (entry_slot + 1) * layout.value_bits,
            layout.value_bit + entry_count * layout.value_bits,
            layout.value_bits,
        );
        shift_bits_left(
            &mut self.bytes,
            layout.crumb_bit + (entry_slot + 1) * layout.crumb_bits,
            layout.crumb_bit + entry_count * layout.crumb_bits,
            layout.crumb_bits,
        );
        self.write_entry(
            layout,
            entry_count - 1,
            SubtableEntry {
                unary_index: 0,
                fingerprint: 0,
                value: 0,
                crumb: 0,
            },
        );
        let bit_index = unary_index + entry_slot;
        let lower_mask_value = (1u128 << bit_index) - 1;
        let lower = unary & lower_mask_value;
        let upper = (unary >> (bit_index + 1)) << bit_index;
        self.store_unary(layout, lower | upper);
    }

    fn unary_insert(
        &mut self,
        layout: &SubtableLayout,
        bits: u128,
        entry_count: usize,
        unary_index: usize,
        entry_slot: usize,
    ) {
        let bit_index = unary_index + entry_slot;
        let lower_mask_value = (1u128 << bit_index) - 1;
        let total_bits = layout.unary_count + layout.entry_capacity;
        let active_mask = low_mask(total_bits);
        let mut shifted =
            ((bits & lower_mask_value) | ((bits & !lower_mask_value) << 1)) & active_mask;
        if entry_count == layout.entry_capacity {
            let zeros = !shifted & active_mask;
            let last_zero = 127 - zeros.leading_zeros() as usize;
            shifted |= 1u128 << last_zero;
        }
        self.store_unary(layout, shifted);
    }
}

fn shift_bits_right(bytes: &mut [u8], start: usize, end: usize, amount: usize) {
    if amount == 0 {
        return;
    }
    let mut source_end = end;
    while source_end > start {
        let width = (source_end - start).min(56);
        let source_start = source_end - width;
        let value = get_bits(bytes, source_start, width);
        set_bits(bytes, source_start + amount, width, value);
        source_end = source_start;
    }
}

fn shift_bits_left(bytes: &mut [u8], start: usize, end: usize, amount: usize) {
    if amount == 0 {
        return;
    }
    let mut source_start = start;
    while source_start < end {
        let width = (end - source_start).min(56);
        let value = get_bits(bytes, source_start, width);
        set_bits(bytes, source_start - amount, width, value);
        source_start += width;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn fingerprint_matches_avx512(bytes: &[u8; SUBTABLE_BYTES], fingerprint: u8) -> u64 {
    use std::arch::asm;

    let matches: u64;
    // SAFETY: the target features are checked before this function is called, `bytes` spans one
    // aligned 64-byte Subtable, and the assembly reads exactly that cache line.
    unsafe {
        asm!(
            "vpbroadcastb zmm0, {fingerprint:e}",
            "vpcmpeqb k1, zmm0, zmmword ptr [{bytes}]",
            "kmovq {matches}, k1",
            "vzeroupper",
            bytes = in(reg) bytes.as_ptr(),
            fingerprint = in(reg) fingerprint as u32,
            matches = lateout(reg) matches,
            out("zmm0") _,
            out("k1") _,
            options(readonly, nostack, preserves_flags),
        );
    }
    matches
}

fn select_one(bits: u128, rank: usize, bmi2_select: bool) -> usize {
    #[cfg(target_arch = "x86_64")]
    if bmi2_select {
        // SAFETY: Table creation sets this flag only after detecting BMI2.
        return unsafe { select_one_bmi2(bits, rank) };
    }

    select_one_scalar(bits, rank)
}

fn select_one_scalar(mut bits: u128, rank: usize) -> usize {
    for _ in 0..rank {
        bits &= bits - 1;
    }
    bits.trailing_zeros() as usize
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "bmi2")]
unsafe fn select_one_bmi2(bits: u128, rank: usize) -> usize {
    use std::arch::x86_64::_pdep_u64;

    let low = bits as u64;
    let low_count = low.count_ones() as usize;
    if rank < low_count {
        _pdep_u64(1u64 << rank, low).trailing_zeros() as usize
    } else {
        let high = (bits >> 64) as u64;
        64 + _pdep_u64(1u64 << (rank - low_count), high).trailing_zeros() as usize
    }
}

fn low_mask(bits: usize) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

fn u32_mask(bits: usize) -> u32 {
    if bits == u32::BITS as usize {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    }
}

fn unary_bounds(bits: u128, unary_index: usize, bmi2_select: bool) -> (usize, usize) {
    #[cfg(target_arch = "x86_64")]
    if bmi2_select {
        // SAFETY: Table creation sets this flag only after detecting BMI2.
        return unsafe { unary_bounds_bmi2(bits, unary_index) };
    }

    let end = select_one_scalar(bits, unary_index) - unary_index;
    let start = (unary_index != 0)
        .then(|| select_one_scalar(bits, unary_index - 1) - (unary_index - 1))
        .unwrap_or(0);
    (start, end)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "bmi2")]
unsafe fn unary_bounds_bmi2(bits: u128, unary_index: usize) -> (usize, usize) {
    use std::arch::x86_64::_pdep_u64;

    if bits >> u64::BITS == 0 {
        let ranks = if unary_index == 0 {
            1
        } else {
            3u64 << (unary_index - 1)
        };
        let separators = _pdep_u64(ranks, bits as u64);
        let end = u64::BITS as usize - 1 - separators.leading_zeros() as usize - unary_index;
        let start = (unary_index != 0)
            .then(|| separators.trailing_zeros() as usize - (unary_index - 1))
            .unwrap_or(0);
        return (start, end);
    }

    // SAFETY: this function itself requires BMI2.
    let end = unsafe { select_one_bmi2(bits, unary_index) } - unary_index;
    let start = (unary_index != 0)
        .then(|| unsafe { select_one_bmi2(bits, unary_index - 1) } - (unary_index - 1))
        .unwrap_or(0);
    (start, end)
}

fn get_bits(bytes: &[u8], bit: usize, width: usize) -> u64 {
    if width == 0 {
        return 0;
    }
    let byte = bit / 8;
    let shift = bit % 8;
    debug_assert!(shift + width <= u64::BITS as usize);
    let mask = if width == u64::BITS as usize {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    let word = if byte + size_of::<u64>() <= bytes.len() {
        // SAFETY: the bounds check above covers the full unaligned u64 load.
        u64::from_le(unsafe { bytes.as_ptr().add(byte).cast::<u64>().read_unaligned() })
    } else {
        let byte_len = (shift + width).div_ceil(8);
        let mut word = [0; 8];
        word[..byte_len].copy_from_slice(&bytes[byte..byte + byte_len]);
        u64::from_le_bytes(word)
    };
    (word >> shift) & mask
}

fn set_bits(bytes: &mut [u8], bit: usize, width: usize, value: u64) {
    if width == 0 {
        return;
    }
    let byte = bit / 8;
    let shift = bit % 8;
    debug_assert!(shift + width <= u64::BITS as usize);
    let field_mask = if width == u64::BITS as usize {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    let shifted_mask = field_mask << shift;
    if byte + size_of::<u64>() <= bytes.len() {
        let pointer = unsafe { bytes.as_mut_ptr().add(byte).cast::<u64>() };
        // SAFETY: the bounds check above covers both unaligned accesses to the same u64.
        let word = u64::from_le(unsafe { pointer.read_unaligned() });
        let updated = (word & !shifted_mask) | ((value << shift) & shifted_mask);
        unsafe { pointer.write_unaligned(updated.to_le()) };
    } else {
        let byte_len = (shift + width).div_ceil(8);
        let mut word = [0; 8];
        word[..byte_len].copy_from_slice(&bytes[byte..byte + byte_len]);
        let updated =
            (u64::from_le_bytes(word) & !shifted_mask) | ((value << shift) & shifted_mask);
        bytes[byte..byte + byte_len].copy_from_slice(&updated.to_le_bytes()[..byte_len]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Table {
        Table::new(TableConfig {
            max_entries: 128,
            value_bits: 12,
            fingerprint_bits: 12,
        })
        .unwrap()
    }

    fn hash_for(table: &Table, front_subtable_index: usize, unary_index: usize, fp: u32) -> u128 {
        let coordinate =
            front_subtable_index * table.front_subtable_layout.unary_count + unary_index;
        let coordinate_hash =
            ((coordinate as u128) << u64::BITS).div_ceil(table.coordinate_modulus.get() as u128);
        debug_assert_eq!(
            coordinate,
            ((coordinate_hash * table.coordinate_modulus.get() as u128) >> u64::BITS) as usize
        );
        (coordinate_hash << table.front_subtable_layout.fingerprint_bits) | u128::from(fp)
    }

    #[test]
    fn candidates_preserve_exact_value_multiplicity() {
        let mut table = table();
        let hash = hash_for(&table, 0, 0, 7);
        table.insert_with_eviction_score(hash, 11, |_| 0);
        table.insert_with_eviction_score(hash, 12, |_| 0);
        table.insert_with_eviction_score(hash, 11, |_| 0);

        let values = table.candidates(hash);
        assert_eq!(values.as_slice(), [11, 12, 11]);
        assert!(!values.spilled());
        assert!(table.remove_one(hash, 11));
        assert_eq!(
            table
                .candidates(hash)
                .iter()
                .filter(|value| **value == 11)
                .count(),
            1
        );
        assert_eq!(table.remove_all(hash, 11), 1);
        assert_eq!(table.candidates(hash).as_slice(), [12]);
    }

    #[test]
    fn configured_bit_widths_round_trip() {
        for (fingerprint_bits, value_bits) in [
            (1, 1),
            (3, 5),
            (7, 13),
            (8, 16),
            (12, 12),
            (17, 24),
            (32, 32),
        ] {
            let mut table = Table::new(TableConfig {
                max_entries: 32,
                value_bits,
                fingerprint_bits,
            })
            .unwrap();
            let fingerprint = u32_mask(fingerprint_bits as usize);
            let value = u32_mask(value_bits as usize);
            let hash = ((table.front_subtable_layout.unary_count as u128 - 1) << fingerprint_bits)
                | fingerprint as u128;

            table.insert_with_eviction_score(hash, value, |_| 0);
            assert_eq!(table.candidates(hash).as_slice(), [value]);
            assert_eq!(table.remove_all(hash, value), 1);
            assert_eq!(table.candidates(hash).len(), 0);
        }
    }

    #[test]
    fn remove_all_covers_front_and_both_back_routes() {
        let mut table = table();
        let hash = hash_for(&table, 0, 0, 23);
        let [first, second] = table.back_subtable_routes(0);
        let local_capacity = table.front_subtable_layout.entry_capacity
            + table.back_subtable_layout.entry_capacity * if first.0 == second.0 { 1 } else { 2 };

        for _ in 0..local_capacity {
            table.insert_with_eviction_score(hash, 91, |_| 0);
        }
        assert_eq!(table.candidates(hash).len(), local_capacity);
        assert_eq!(table.remove_all(hash, 91), local_capacity);
        assert_eq!(table.candidates(hash).len(), 0);
    }

    #[test]
    fn saturated_route_evicts_maximum_score_and_admits_new_entry() {
        let mut table = table();
        let [first, second] = table.back_subtable_routes(0);
        let local_capacity = table.front_subtable_layout.entry_capacity
            + table.back_subtable_layout.entry_capacity * if first.0 == second.0 { 1 } else { 2 };
        let mut inserted = Vec::with_capacity(local_capacity);

        for value in 1..=local_capacity as u32 {
            let unary_index = value as usize % table.front_subtable_layout.unary_count;
            let hash = hash_for(&table, 0, unary_index, value);
            table.insert_with_eviction_score(hash, value, |_| 0);
            inserted.push((hash, value));
        }

        let new_hash = hash_for(&table, 0, 0, table.fingerprint_mask);
        table.insert_with_eviction_score(new_hash, table.value_mask, |value| value as usize);
        assert!(table.candidates(new_hash).contains(&table.value_mask));

        let (evicted_hash, evicted_value) = inserted.pop().unwrap();
        assert!(!table.candidates(evicted_hash).contains(&evicted_value));
    }

    #[test]
    fn avx512_byte_matches_equal_scalar_matches() {
        let mut table = Table::new(TableConfig {
            max_entries: 128,
            value_bits: 16,
            fingerprint_bits: 8,
        })
        .unwrap();
        let hash = hash_for(&table, 0, 0, 31);
        for value in 0..table.front_subtable_layout.entry_capacity as u32 {
            table.insert_with_eviction_score(hash, value, |_| 0);
        }
        if !table.avx512_byte_match {
            return;
        }

        let scalar = table.front_table[0].matching_entry_mask(
            &table.front_subtable_layout,
            table.front_table[0].unary(&table.front_subtable_layout),
            0,
            31,
            None,
            false,
        );
        let avx512 = table.front_table[0].matching_entry_mask(
            &table.front_subtable_layout,
            table.front_table[0].unary(&table.front_subtable_layout),
            0,
            31,
            None,
            true,
        );
        assert_eq!(avx512, scalar);
    }

    #[test]
    fn twelve_bit_packing_survives_insert_remove_churn() {
        let mut table = Table::new(TableConfig {
            max_entries: 4096,
            value_bits: 12,
            fingerprint_bits: 12,
        })
        .unwrap();
        let mut hashes = Vec::with_capacity(1024);
        let mut state = 0x243f_6a88_85a3_08d3u64;

        for value in 0..1024u32 {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut low = state;
            low = (low ^ (low >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            low = (low ^ (low >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            low ^= low >> 31;
            let high = low.rotate_left(29) ^ 0xd6e8_feb8_6659_fd93;
            let hash = (u128::from(high) << 64) | u128::from(low);
            table.insert_with_eviction_score(hash, value, |_| 0);
            hashes.push(hash);
        }

        for (value, hash) in hashes.iter().copied().enumerate() {
            assert!(table.candidates(hash).contains(&(value as u32)));
        }
        for (value, hash) in hashes.into_iter().enumerate() {
            assert!(table.remove_one(hash, value as u32));
            assert!(!table.candidates(hash).contains(&(value as u32)));
        }
    }

    #[test]
    fn bmi2_unary_bounds_equal_scalar_select() {
        #[cfg(target_arch = "x86_64")]
        if is_x86_feature_detected!("bmi2") {
            for bits in [
                u128::MAX,
                0xaaaa_5555_f0f0_0f0fu128,
                (0xfedc_ba98_7654_3210u128 << 64) | 0x0123_4567_89ab_cdef,
                (1u128 << 127) | (1u128 << 65) | (1u128 << 63) | 1,
            ] {
                for unary_index in 0..bits.count_ones() as usize {
                    let end = select_one_scalar(bits, unary_index) - unary_index;
                    let start = (unary_index != 0)
                        .then(|| select_one_scalar(bits, unary_index - 1) - (unary_index - 1))
                        .unwrap_or(0);
                    // SAFETY: this branch runs only after detecting BMI2.
                    assert_eq!(
                        unsafe { unary_bounds_bmi2(bits, unary_index) },
                        (start, end)
                    );
                }
            }
        }
    }
}
