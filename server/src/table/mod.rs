//! Two-tier lookup Table mapping storage keys to candidate storage locations.
//!
//! The quotient selects a Subtable and unary position. Entries store only the
//! fingerprint and candidate Segment/Bucket-hash location; storage verifies the
//! complete storage key.

use std::collections::HashSet;

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
    fingerprint_hash_offset_bits: usize,
    sg_index_bits: usize,
    pub(crate) entry_count: usize,
}

impl Table {
    pub(crate) fn new(config: &Config) -> Result<Self> {
        let front_subtable_layout = SubtableLayout::new(
            config.fingerprint_bits,
            config.front_back_ratio,
            false,
            config.unary_count,
            config.sg_index_bits,
        )?;
        let back_subtable_layout = SubtableLayout::new(
            config.fingerprint_bits,
            config.front_back_ratio,
            true,
            config.unary_count,
            config.sg_index_bits,
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
            front_table: (0..front_subtable_count)
                .map(|_| Subtable::new(&front_subtable_layout))
                .collect(),
            back_table: (0..back_subtable_count)
                .map(|_| Subtable::new(&back_subtable_layout))
                .collect(),
            front_subtable_layout,
            back_subtable_layout,
            back_subtable_group_count,
            front_back_ratio: config.front_back_ratio,
            fingerprint_bits: config.fingerprint_bits,
            fingerprint_hash_offset_bits: config.fingerprint_hash_offset_bits,
            sg_index_bits: config.sg_index_bits,
            entry_count: 0,
        })
    }

    pub(crate) fn candidate_locations(&self, storage_key: &StorageKey) -> Vec<TableLocation> {
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(storage_key);
        let front_subtable = &self.front_table[front_subtable_index];
        let mut encoded = front_subtable
            .matching_entry_slots(&self.front_subtable_layout, unary_index, fingerprint, None)
            .into_iter()
            .map(|entry_slot| {
                front_subtable
                    .entry(&self.front_subtable_layout, entry_slot)
                    .table_location
            })
            .collect::<Vec<_>>();
        let (_, end) = front_subtable.bounds(&self.front_subtable_layout, unary_index);
        if end == self.front_subtable_layout.entry_capacity {
            for route in self.back_subtable_routes(front_subtable_index) {
                let back_subtable = &self.back_table[route.0];
                encoded.extend(
                    back_subtable
                        .matching_entry_slots(
                            &self.back_subtable_layout,
                            unary_index,
                            fingerprint,
                            Some(route.1),
                        )
                        .into_iter()
                        .map(|entry_slot| {
                            back_subtable
                                .entry(&self.back_subtable_layout, entry_slot)
                                .table_location
                        }),
                );
            }
        }
        let mut seen = HashSet::new();
        encoded
            .into_iter()
            .map(|value| TableLocation::decode(value, self.sg_index_bits))
            .filter(|location| seen.insert(*location))
            .collect()
    }

    pub(crate) fn insert(
        &mut self,
        storage_key: &StorageKey,
        table_location: TableLocation,
    ) -> Result<()> {
        if !table_location.is_valid(self.sg_index_bits) {
            return Err(KvError::InvalidConfig(
                "Table Location does not fit configured Segment/Bucket-hash fields".into(),
            ));
        }
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(storage_key);
        let entry = SubtableEntry {
            unary_index,
            fingerprint,
            table_location: table_location.encode(self.sg_index_bits),
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
        if !table_location.is_valid(self.sg_index_bits) {
            return false;
        }
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(storage_key);
        let encoded = table_location.encode(self.sg_index_bits);
        let was_full = self.front_table[front_subtable_index]
            .entry_count(&self.front_subtable_layout)
            == self.front_subtable_layout.entry_capacity;
        let front_entry_slots = self.front_table[front_subtable_index].matching_entry_slots(
            &self.front_subtable_layout,
            unary_index,
            fingerprint,
            None,
        );
        if let Some(entry_slot) = front_entry_slots.into_iter().find(|entry_slot| {
            self.front_table[front_subtable_index]
                .entry(&self.front_subtable_layout, *entry_slot)
                .table_location
                == encoded
        }) {
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
            let entry_slots = self.back_table[route.0].matching_entry_slots(
                &self.back_subtable_layout,
                unary_index,
                fingerprint,
                Some(route.1),
            );
            if let Some(entry_slot) = entry_slots.into_iter().find(|entry_slot| {
                self.back_table[route.0]
                    .entry(&self.back_subtable_layout, *entry_slot)
                    .table_location
                    == encoded
            }) {
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
        if !previous.is_valid(self.sg_index_bits) || !replacement.is_valid(self.sg_index_bits) {
            return false;
        }
        if previous == replacement {
            return true;
        }
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(storage_key);
        let old = previous.encode(self.sg_index_bits);
        let new = replacement.encode(self.sg_index_bits);
        let front_entry_slots = self.front_table[front_subtable_index].matching_entry_slots(
            &self.front_subtable_layout,
            unary_index,
            fingerprint,
            None,
        );
        if let Some(entry_slot) = front_entry_slots.into_iter().find(|entry_slot| {
            self.front_table[front_subtable_index]
                .entry(&self.front_subtable_layout, *entry_slot)
                .table_location
                == old
        }) {
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
            let entry_slots = self.back_table[route.0].matching_entry_slots(
                &self.back_subtable_layout,
                unary_index,
                fingerprint,
                Some(route.1),
            );
            if let Some(entry_slot) = entry_slots.into_iter().find(|entry_slot| {
                self.back_table[route.0]
                    .entry(&self.back_subtable_layout, *entry_slot)
                    .table_location
                    == old
            }) {
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
            (Some(first), Some(second)) if first.1.unary_index <= second.1.unary_index => {
                (first.0, first)
            }
            (Some(_), Some(second)) => (second.0, second),
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
        let storage_key = storage_key.as_bytes();
        let prefix = u128::from_le_bytes(storage_key[..16].try_into().unwrap())
            >> self.fingerprint_hash_offset_bits;
        let prefix = prefix as u64;
        let quotient_count = self.front_table.len() * self.front_subtable_layout.unary_count;
        let fingerprint_space = 1u64 << self.fingerprint_bits;
        let space = quotient_count as u128 * fingerprint_space as u128;
        let quotient_and_fingerprint = (prefix as u128 % space) as u64;
        let quotient = (quotient_and_fingerprint / fingerprint_space) as usize;
        let front_subtable_index = quotient / self.front_subtable_layout.unary_count;
        let unary_index = quotient % self.front_subtable_layout.unary_count;
        (
            front_subtable_index,
            unary_index,
            (quotient_and_fingerprint & (fingerprint_space - 1)) as u16,
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
