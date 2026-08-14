//! Two-tier lookup Table mapping storage keys to candidate storage locations.
//!
//! The quotient selects a Subtable and unary position. Entries store only the
//! fingerprint and candidate Segment/Bucket-hash location; storage verifies the
//! complete storage key.

use smallvec::SmallVec;
use std::num::NonZeroU64;

use crate::SUBTABLE_BYTES;
use crate::StorageKey;
use crate::config::Config;
use crate::error::{KvError, Result};

mod subtable;
pub(crate) use self::subtable::*;

pub(crate) struct Table {
    pub(crate) front_table: Vec<Subtable>,
    pub(crate) back_table: Vec<Subtable>,
    pub(crate) front_subtable_layout: SubtableLayout,
    pub(crate) back_subtable_layout: SubtableLayout,
    pub(crate) back_subtable_group_count: usize,
    front_back_ratio: usize,
    fingerprint_bits: usize,
    fingerprint_mask: u64,
    fingerprint_hash_offset_bits: usize,
    coordinate_modulus: Option<NonZeroU64>,
    sg_index_bits: usize,
    bucket_choice_bits: usize,
    pub(crate) entry_count: usize,
}

struct TableAllocation {
    front_subtable_layout: SubtableLayout,
    back_subtable_layout: SubtableLayout,
    front_subtable_count: usize,
    back_subtable_count: usize,
    back_subtable_group_count: usize,
}

impl TableAllocation {
    fn new(config: &Config) -> Result<Self> {
        let front_subtable_layout = SubtableLayout::new(
            config.fingerprint_bits,
            config.front_back_ratio,
            false,
            config.unary_count,
            config.sg_index_bits,
            config.bucket_choice_bits(),
        )?;
        let back_subtable_layout = SubtableLayout::new(
            config.fingerprint_bits,
            config.front_back_ratio,
            true,
            config.unary_count,
            config.sg_index_bits,
            config.bucket_choice_bits(),
        )?;
        let entries_per_front = front_subtable_layout.entry_capacity as f64
            + back_subtable_layout.entry_capacity as f64 / config.front_back_ratio as f64;
        let planned_per_front = entries_per_front * config.table_target_load_percent as f64 / 100.0;
        let front_subtable_count = (config.table_capacity as f64 / planned_per_front)
            .ceil()
            .max(1.0) as usize;
        let back_subtable_group_count = front_subtable_count
            .div_ceil(config.front_back_ratio * config.front_back_ratio)
            .max(1);
        let back_subtable_count = front_subtable_count
            .div_ceil(config.front_back_ratio)
            .max(config.front_back_ratio * back_subtable_group_count);
        Ok(Self {
            front_subtable_layout,
            back_subtable_layout,
            front_subtable_count,
            back_subtable_count,
            back_subtable_group_count,
        })
    }

    fn memory_bytes(&self) -> Result<usize> {
        self.front_subtable_count
            .checked_add(self.back_subtable_count)
            .and_then(|count| count.checked_mul(SUBTABLE_BYTES))
            .ok_or_else(|| KvError::InvalidConfig("Table allocation size is too large".into()))
    }
}

impl Table {
    pub(crate) fn new(config: &Config) -> Result<Self> {
        let allocation = TableAllocation::new(config)?;
        let fingerprint_space = 1u64 << config.fingerprint_bits;
        let coordinate_space = allocation.front_subtable_count as u128
            * config.unary_count as u128
            * fingerprint_space as u128;
        let coordinate_modulus = u64::try_from(coordinate_space)
            .ok()
            .and_then(NonZeroU64::new);
        let mut front_table = Vec::new();
        front_table
            .try_reserve_exact(allocation.front_subtable_count)
            .map_err(|error| {
                KvError::InvalidConfig(format!("front Table allocation failed: {error}"))
            })?;
        front_table.resize_with(allocation.front_subtable_count, || {
            Subtable::new(&allocation.front_subtable_layout)
        });
        let mut back_table = Vec::new();
        back_table
            .try_reserve_exact(allocation.back_subtable_count)
            .map_err(|error| {
                KvError::InvalidConfig(format!("back Table allocation failed: {error}"))
            })?;
        back_table.resize_with(allocation.back_subtable_count, || {
            Subtable::new(&allocation.back_subtable_layout)
        });
        Ok(Self {
            front_table,
            back_table,
            front_subtable_layout: allocation.front_subtable_layout,
            back_subtable_layout: allocation.back_subtable_layout,
            back_subtable_group_count: allocation.back_subtable_group_count,
            front_back_ratio: config.front_back_ratio,
            fingerprint_bits: config.fingerprint_bits,
            fingerprint_mask: fingerprint_space - 1,
            fingerprint_hash_offset_bits: config.fingerprint_hash_offset_bits,
            coordinate_modulus,
            sg_index_bits: config.sg_index_bits,
            bucket_choice_bits: config.bucket_choice_bits(),
            entry_count: 0,
        })
    }

    pub(crate) fn modeled_memory_bytes(config: &Config) -> Result<usize> {
        TableAllocation::new(config)?.memory_bytes()
    }

    pub(crate) fn candidate_locations(
        &self,
        storage_key: &StorageKey,
    ) -> SmallVec<[TableLocation; 4]> {
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(storage_key);
        let front_subtable = &self.front_table[front_subtable_index];
        let (start, end) = front_subtable.bounds(&self.front_subtable_layout, unary_index);
        let mut locations = SmallVec::new();
        for entry_slot in front_subtable.matching_entry_slots_in_range(
            &self.front_subtable_layout,
            start..end,
            fingerprint,
            None,
        ) {
            let location = TableLocation::decode(
                front_subtable.table_location(&self.front_subtable_layout, entry_slot),
                self.sg_index_bits,
                self.bucket_choice_bits,
            );
            if !locations.contains(&location) {
                locations.push(location);
            }
        }
        if end == self.front_subtable_layout.entry_capacity {
            for route in self.back_subtable_routes(front_subtable_index) {
                let back_subtable = &self.back_table[route.0];
                for entry_slot in back_subtable.matching_entry_slots(
                    &self.back_subtable_layout,
                    unary_index,
                    fingerprint,
                    Some(route.1),
                ) {
                    let location = TableLocation::decode(
                        back_subtable.table_location(&self.back_subtable_layout, entry_slot),
                        self.sg_index_bits,
                        self.bucket_choice_bits,
                    );
                    if !locations.contains(&location) {
                        locations.push(location);
                    }
                }
            }
        }
        locations
    }

    pub(crate) fn insert(
        &mut self,
        storage_key: &StorageKey,
        table_location: TableLocation,
    ) -> Result<()> {
        if !table_location.is_valid(self.sg_index_bits, self.bucket_choice_bits) {
            return Err(KvError::InvalidConfig(
                "Table Location does not fit configured Segment/Bucket-hash fields".into(),
            ));
        }
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(storage_key);
        let entry = SubtableEntry {
            unary_index,
            fingerprint,
            table_location: table_location.encode(self.sg_index_bits, self.bucket_choice_bits),
            crumb: 0,
        };
        let saved = self.front_table[front_subtable_index].clone();
        let overflow =
            self.front_table[front_subtable_index].insert_front(&self.front_subtable_layout, entry);
        let Some(mut overflow) = overflow else {
            self.entry_count += 1;
            return Ok(());
        };
        let [first, second] = self.back_subtable_routes(front_subtable_index);
        let destination = if self.back_table[first.0].entry_count(&self.back_subtable_layout)
            <= self.back_table[second.0].entry_count(&self.back_subtable_layout)
        {
            first
        } else {
            second
        };
        overflow.crumb = destination.1;
        if !self.back_table[destination.0].insert_back(&self.back_subtable_layout, overflow) {
            self.front_table[front_subtable_index] = saved;
            return Err(KvError::TableFull);
        }
        self.entry_count += 1;
        Ok(())
    }

    pub(crate) fn remove(
        &mut self,
        storage_key: &StorageKey,
        table_location: TableLocation,
    ) -> bool {
        if !table_location.is_valid(self.sg_index_bits, self.bucket_choice_bits) {
            return false;
        }
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(storage_key);
        let encoded = table_location.encode(self.sg_index_bits, self.bucket_choice_bits);
        let was_full = self.front_table[front_subtable_index]
            .entry_count(&self.front_subtable_layout)
            == self.front_subtable_layout.entry_capacity;
        let front_entry_slot = self.front_table[front_subtable_index]
            .matching_entry_slots(&self.front_subtable_layout, unary_index, fingerprint, None)
            .find(|entry_slot| {
                self.front_table[front_subtable_index]
                    .table_location(&self.front_subtable_layout, *entry_slot)
                    == encoded
            });
        if let Some(entry_slot) = front_entry_slot {
            self.front_table[front_subtable_index].remove_at(
                &self.front_subtable_layout,
                unary_index,
                entry_slot,
            );
            if was_full {
                self.promote(front_subtable_index);
            }
            self.entry_count -= 1;
            return true;
        }
        if !was_full {
            return false;
        }
        for route in self.back_subtable_routes(front_subtable_index) {
            let entry_slot = self.back_table[route.0]
                .matching_entry_slots(
                    &self.back_subtable_layout,
                    unary_index,
                    fingerprint,
                    Some(route.1),
                )
                .find(|entry_slot| {
                    self.back_table[route.0].table_location(&self.back_subtable_layout, *entry_slot)
                        == encoded
                });
            if let Some(entry_slot) = entry_slot {
                self.back_table[route.0].remove_at(
                    &self.back_subtable_layout,
                    unary_index,
                    entry_slot,
                );
                self.entry_count -= 1;
                return true;
            }
        }
        false
    }

    pub(crate) fn replace_location(
        &mut self,
        storage_key: &StorageKey,
        previous: TableLocation,
        replacement: TableLocation,
    ) -> bool {
        if !previous.is_valid(self.sg_index_bits, self.bucket_choice_bits)
            || !replacement.is_valid(self.sg_index_bits, self.bucket_choice_bits)
        {
            return false;
        }
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(storage_key);
        let old = previous.encode(self.sg_index_bits, self.bucket_choice_bits);
        let new = replacement.encode(self.sg_index_bits, self.bucket_choice_bits);
        let front_entry_slot = self.front_table[front_subtable_index]
            .matching_entry_slots(&self.front_subtable_layout, unary_index, fingerprint, None)
            .find(|entry_slot| {
                self.front_table[front_subtable_index]
                    .table_location(&self.front_subtable_layout, *entry_slot)
                    == old
            });
        if let Some(entry_slot) = front_entry_slot {
            let mut entry = self.front_table[front_subtable_index]
                .entry(&self.front_subtable_layout, entry_slot);
            entry.table_location = new;
            self.front_table[front_subtable_index].write_entry(
                &self.front_subtable_layout,
                entry_slot,
                entry,
            );
            return true;
        }
        if self.front_table[front_subtable_index].entry_count(&self.front_subtable_layout)
            < self.front_subtable_layout.entry_capacity
        {
            return false;
        }
        for route in self.back_subtable_routes(front_subtable_index) {
            let entry_slot = self.back_table[route.0]
                .matching_entry_slots(
                    &self.back_subtable_layout,
                    unary_index,
                    fingerprint,
                    Some(route.1),
                )
                .find(|entry_slot| {
                    self.back_table[route.0].table_location(&self.back_subtable_layout, *entry_slot)
                        == old
                });
            if let Some(entry_slot) = entry_slot {
                let mut entry =
                    self.back_table[route.0].entry(&self.back_subtable_layout, entry_slot);
                entry.table_location = new;
                self.back_table[route.0].write_entry(&self.back_subtable_layout, entry_slot, entry);
                return true;
            }
        }
        false
    }

    fn promote(&mut self, front_subtable_index: usize) {
        let [first, second] = self.back_subtable_routes(front_subtable_index);
        let first_entry =
            self.back_table[first.0].first_with_crumb(&self.back_subtable_layout, first.1);
        let second_entry =
            self.back_table[second.0].first_with_crumb(&self.back_subtable_layout, second.1);
        let selected = match (first_entry, second_entry) {
            (None, None) => return,
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
        debug_assert!(overflow.is_none());
    }

    pub(crate) fn load_factor(&self) -> f64 {
        let capacity = self.front_table.len() * self.front_subtable_layout.entry_capacity
            + self.back_table.len() * self.back_subtable_layout.entry_capacity;
        if capacity == 0 {
            0.0
        } else {
            self.entry_count as f64 / capacity as f64
        }
    }

    pub(crate) fn memory_bytes(&self) -> usize {
        (self.front_table.len() + self.back_table.len()) * SUBTABLE_BYTES
    }

    fn table_coordinates(&self, storage_key: &StorageKey) -> (usize, usize, u16) {
        let prefix = storage_key.table_hash() >> self.fingerprint_hash_offset_bits;
        let prefix = prefix as u64;
        let quotient_and_fingerprint = self
            .coordinate_modulus
            .map_or(prefix, |modulus| prefix % modulus.get());
        let quotient = (quotient_and_fingerprint >> self.fingerprint_bits) as usize;
        let front_subtable_index = quotient / self.front_subtable_layout.unary_count;
        let unary_index = quotient % self.front_subtable_layout.unary_count;
        (
            front_subtable_index,
            unary_index,
            (quotient_and_fingerprint & self.fingerprint_mask) as u16,
        )
    }

    fn back_subtable_routes(&self, front_subtable_index: usize) -> [(usize, u8); 2] {
        let upper = front_subtable_index / self.front_back_ratio;
        let low = front_subtable_index % self.front_back_ratio;
        let first = (upper, (self.front_back_ratio + low) as u8);
        let second = (
            upper / self.front_back_ratio + low * self.back_subtable_group_count,
            (upper % self.front_back_ratio) as u8,
        );
        debug_assert!(
            first.0 < self.back_table.len(),
            "back Subtable {} out of bounds (front={front_subtable_index})",
            first.0
        );
        debug_assert!(
            second.0 < self.back_table.len(),
            "back Subtable {} out of bounds (front={front_subtable_index})",
            second.0
        );
        [first, second]
    }
}
