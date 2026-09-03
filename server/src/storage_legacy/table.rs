//! A legacy fixed-size compressed Table that maps hashes to 1-32 bit value candidates.

use std::num::NonZeroU64;

const SUBTABLE_BYTES: usize = 64;
const TARGET_LOAD_PERCENT: usize = 88;
const MIN_UNARY_COUNT: usize = 2;
const MAX_UNARY_COUNT: usize = 96;
const FRONT_BACK_RATIOS: [usize; 4] = [2, 4, 8, 16];

/// Entry capacity and field widths for a Table.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TableConfig {
    /// Number of Entries the Table must hold at the target load factor.
    pub(crate) max_entries: usize,
    /// Number of value bits stored per Entry.
    pub(crate) value_bits: u8,
    /// Number of fingerprint bits stored per Entry.
    pub(crate) fingerprint_bits: u8,
}

impl TableConfig {
    /// Validates the ranges of the three externally supplied values.
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

/// Reason a fixed-size Table could not be created.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableCreateError {
    /// An externally supplied configuration value is invalid.
    InvalidConfig(&'static str),
    /// No 64-byte Subtable layout can represent the configuration.
    NoValidLayout,
    /// Allocation of the fixed Front or Back Subtable array failed.
    AllocationFailed,
}

/// The fixed-size Table cannot accept another Entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TableFull;

/// Computed counts and internal layouts for Front and Back Subtables.
struct TableAllocation {
    /// Internal 64-byte layout of one Front Subtable.
    front_subtable_layout: SubtableLayout,
    /// Internal 64-byte layout of one Back Subtable.
    back_subtable_layout: SubtableLayout,
    /// Number of Front Subtables to allocate.
    front_subtable_count: usize,
    /// Number of Back Subtables to allocate.
    back_subtable_count: usize,
    /// Number of Subtables in a group on the second Back route.
    back_subtable_group_count: usize,
    /// Grouping ratio between Front Subtables and Back routes.
    front_back_ratio: usize,
}

impl TableAllocation {
    /// Evaluates every valid layout and selects the smallest one that holds the target Entries.
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
                let front_subtable_count = numerator.div_ceil(denominator).max(1) as usize;
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
                                && (unary_count > *selected_unary_count
                                    || (unary_count == *selected_unary_count
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

/// A fixed-size Table allocated at creation that stores only values associated with hashes.
pub(crate) struct Table {
    /// Front Subtable array that receives every Entry first.
    front_table: Box<[Subtable]>,
    /// Back Subtable array that stores Entries evicted from Front.
    back_table: Box<[Subtable]>,
    /// Bit layout within a Front Subtable.
    front_subtable_layout: SubtableLayout,
    /// Bit layout within a Back Subtable.
    back_subtable_layout: SubtableLayout,
    /// Number of Subtables in a group on the second Back route.
    back_subtable_group_count: usize,
    /// Grouping ratio between Front Subtables and Back routes.
    front_back_ratio: usize,
    /// Mask that retains only fingerprint bits from a hash.
    fingerprint_mask: u32,
    /// Modulus that folds hash coordinates into the Front Subtable and unary-group range.
    coordinate_modulus: NonZeroU64,
    /// Mask whose set bits represent the permitted value width.
    value_mask: u32,
}

impl Table {
    /// Selects a layout automatically and allocates fixed-size Front and Back arrays.
    pub(crate) fn new(config: TableConfig) -> Result<Self, TableCreateError> {
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

        Ok(Self {
            front_table: front_table.into_boxed_slice(),
            back_table: back_table.into_boxed_slice(),
            front_subtable_layout: allocation.front_subtable_layout,
            back_subtable_layout: allocation.back_subtable_layout,
            back_subtable_group_count: allocation.back_subtable_group_count,
            front_back_ratio: allocation.front_back_ratio,
            fingerprint_mask: u32_mask(config.fingerprint_bits as usize),
            coordinate_modulus: coordinate_count,
            value_mask: u32_mask(config.value_bits as usize),
        })
    }

    /// Iterates over values with matching fingerprints without additional allocation.
    pub(crate) fn values(&self, hash: u128) -> impl Iterator<Item = u32> + '_ {
        TableValues::new(self, hash)
    }

    /// Inserts a hash and value into Front, placing any overflow in the less-full Back candidate.
    pub(crate) fn insert(&mut self, hash: u128, value: u32) -> Result<(), TableFull> {
        self.assert_value_fits(value);
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(hash);
        let entry = SubtableEntry {
            unary_index,
            fingerprint,
            value,
            crumb: 0,
        };

        let saved = self.front_table[front_subtable_index].clone();
        let overflow =
            self.front_table[front_subtable_index].insert_front(&self.front_subtable_layout, entry);
        let Some(mut overflow) = overflow else {
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
            return Err(TableFull);
        }
        Ok(())
    }

    /// Removes one Entry whose fingerprint and value both match.
    pub(crate) fn remove(&mut self, hash: u128, value: u32) -> bool {
        self.assert_value_fits(value);
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(hash);
        let was_full = self.front_table[front_subtable_index]
            .entry_count(&self.front_subtable_layout)
            == self.front_subtable_layout.entry_capacity;

        let front_entry_slot = self.front_table[front_subtable_index]
            .matching_entry_slots(&self.front_subtable_layout, unary_index, fingerprint, None)
            .find(|entry_slot| {
                self.front_table[front_subtable_index]
                    .value(&self.front_subtable_layout, *entry_slot)
                    == value
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
            return true;
        }
        if !was_full {
            return false;
        }

        for (back_subtable_index, crumb) in self.back_subtable_routes(front_subtable_index) {
            let entry_slot = self.back_table[back_subtable_index]
                .matching_entry_slots(
                    &self.back_subtable_layout,
                    unary_index,
                    fingerprint,
                    Some(crumb),
                )
                .find(|entry_slot| {
                    self.back_table[back_subtable_index]
                        .value(&self.back_subtable_layout, *entry_slot)
                        == value
                });
            if let Some(entry_slot) = entry_slot {
                self.back_table[back_subtable_index].remove_at(
                    &self.back_subtable_layout,
                    unary_index,
                    entry_slot,
                );
                return true;
            }
        }
        false
    }

    /// Replaces one existing value for a hash in place.
    pub(crate) fn replace(&mut self, hash: u128, old_value: u32, new_value: u32) -> bool {
        self.assert_value_fits(old_value);
        self.assert_value_fits(new_value);
        let (front_subtable_index, unary_index, fingerprint) = self.table_coordinates(hash);

        let front_entry_slot = self.front_table[front_subtable_index]
            .matching_entry_slots(&self.front_subtable_layout, unary_index, fingerprint, None)
            .find(|entry_slot| {
                self.front_table[front_subtable_index]
                    .value(&self.front_subtable_layout, *entry_slot)
                    == old_value
            });
        if let Some(entry_slot) = front_entry_slot {
            let mut entry = self.front_table[front_subtable_index]
                .entry(&self.front_subtable_layout, entry_slot);
            entry.value = new_value;
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
        for (back_subtable_index, crumb) in self.back_subtable_routes(front_subtable_index) {
            let entry_slot = self.back_table[back_subtable_index]
                .matching_entry_slots(
                    &self.back_subtable_layout,
                    unary_index,
                    fingerprint,
                    Some(crumb),
                )
                .find(|entry_slot| {
                    self.back_table[back_subtable_index]
                        .value(&self.back_subtable_layout, *entry_slot)
                        == old_value
                });
            if let Some(entry_slot) = entry_slot {
                let mut entry = self.back_table[back_subtable_index]
                    .entry(&self.back_subtable_layout, entry_slot);
                entry.value = new_value;
                self.back_table[back_subtable_index].write_entry(
                    &self.back_subtable_layout,
                    entry_slot,
                    entry,
                );
                return true;
            }
        }
        false
    }

    /// Promotes one overflow Entry from Back into a free Front slot.
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

    /// Splits a hash into a Front Subtable index, unary index, and fingerprint.
    fn table_coordinates(&self, hash: u128) -> (usize, usize, u32) {
        let fingerprint = hash as u32 & self.fingerprint_mask;
        let coordinate_hash = (hash >> self.front_subtable_layout.fingerprint_bits) as u64;
        let coordinate = (coordinate_hash % self.coordinate_modulus.get()) as usize;
        let front_subtable_index = coordinate / self.front_subtable_layout.unary_count;
        let unary_index = coordinate % self.front_subtable_layout.unary_count;
        (front_subtable_index, unary_index, fingerprint)
    }

    fn assert_value_fits(&self, value: u32) {
        assert!(
            value & !self.value_mask == 0,
            "Table value does not fit configured value_bits"
        );
    }

    /// Computes the two Back candidates and crumbs available to a Front Subtable.
    fn back_subtable_routes(&self, front_subtable_index: usize) -> [(usize, u8); 2] {
        let upper = front_subtable_index / self.front_back_ratio;
        let low = front_subtable_index % self.front_back_ratio;
        let first = (upper, (self.front_back_ratio + low) as u8);
        let second = (
            upper / self.front_back_ratio + low * self.back_subtable_group_count,
            (upper % self.front_back_ratio) as u8,
        );
        debug_assert!(first.0 < self.back_table.len());
        debug_assert!(second.0 < self.back_table.len());
        [first, second]
    }
}

/// An iterator that scans Table Front followed by the two relevant Back locations.
struct TableValues<'a> {
    table: &'a Table,
    front_subtable_index: usize,
    unary_index: usize,
    fingerprint: u32,
    back_routes: [(usize, u8); 2],
    search_back: bool,
    source: u8,
    next_entry_slot: usize,
    end_entry_slot: usize,
}

impl<'a> TableValues<'a> {
    fn new(table: &'a Table, hash: u128) -> Self {
        let (front_subtable_index, unary_index, fingerprint) = table.table_coordinates(hash);
        let (next_entry_slot, end_entry_slot) = table.front_table[front_subtable_index]
            .bounds(&table.front_subtable_layout, unary_index);
        Self {
            table,
            front_subtable_index,
            unary_index,
            fingerprint,
            back_routes: table.back_subtable_routes(front_subtable_index),
            search_back: end_entry_slot == table.front_subtable_layout.entry_capacity,
            source: 0,
            next_entry_slot,
            end_entry_slot,
        }
    }

    fn move_to_next_source(&mut self) -> bool {
        self.source += 1;
        if !self.search_back || self.source > 2 {
            return false;
        }
        let (back_subtable_index, _) = self.back_routes[self.source as usize - 1];
        (self.next_entry_slot, self.end_entry_slot) = self.table.back_table[back_subtable_index]
            .bounds(&self.table.back_subtable_layout, self.unary_index);
        true
    }
}

impl Iterator for TableValues<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            while self.next_entry_slot < self.end_entry_slot {
                let entry_slot = self.next_entry_slot;
                self.next_entry_slot += 1;

                let (subtable, layout, crumb) = if self.source == 0 {
                    (
                        &self.table.front_table[self.front_subtable_index],
                        &self.table.front_subtable_layout,
                        None,
                    )
                } else {
                    let (back_subtable_index, crumb) = self.back_routes[self.source as usize - 1];
                    (
                        &self.table.back_table[back_subtable_index],
                        &self.table.back_subtable_layout,
                        Some(crumb),
                    )
                };

                if subtable.fingerprint(layout, entry_slot) == self.fingerprint
                    && crumb.is_none_or(|crumb| subtable.crumb(layout, entry_slot) == crumb)
                {
                    return Some(subtable.value(layout, entry_slot));
                }
            }

            if !self.move_to_next_source() {
                return None;
            }
        }
    }
}

/// CPU representation used to read or write one Entry in a Subtable byte array.
#[derive(Clone, Copy, Debug)]
struct SubtableEntry {
    /// Unary-group index within the Subtable containing this Entry.
    unary_index: usize,
    /// Short identifier retained by the Table from the full key hash.
    fingerprint: u32,
    /// Opaque value whose externally defined meaning is not interpreted by the Table.
    value: u32,
    /// Distinguishes the Entry's Front route within a shared Back Subtable.
    crumb: u8,
}

/// Stores the size and starting bit of each region in a 64-byte Subtable.
#[derive(Clone, Copy, Debug)]
struct SubtableLayout {
    /// Number of unary groups in one Subtable.
    unary_count: usize,
    /// Maximum number of Entries in one Subtable.
    entry_capacity: usize,
    /// Number of fingerprint bits per Entry.
    fingerprint_bits: usize,
    /// Number of value bits per Entry.
    value_bits: usize,
    /// Number of crumb bits per Entry; zero for Front.
    crumb_bits: usize,
    /// Number of bytes reserved for the unary bit string.
    unary_bytes: usize,
    /// Bit offset from the start of the Subtable to the fingerprint region.
    fingerprint_bit: usize,
    /// Bit offset from the start of the Subtable to the value region.
    value_bit: usize,
    /// Bit offset from the start of the Subtable to the crumb region.
    crumb_bit: usize,
}

impl SubtableLayout {
    /// Computes the maximum number of Entries that fit with the configured fields in 64 bytes.
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
        })
    }
}

/// Compresses unary data and all Entry fields into one cache line.
#[repr(C, align(64))]
#[derive(Clone)]
struct Subtable {
    /// One cache line containing `[unary][fingerprints][values][crumbs]`.
    bytes: [u8; SUBTABLE_BYTES],
}

impl Subtable {
    /// Creates a Subtable containing no Entries.
    fn new(layout: &SubtableLayout) -> Self {
        let mut subtable = Self {
            bytes: [0; SUBTABLE_BYTES],
        };
        subtable.store_unary(layout, (1u128 << layout.unary_count) - 1);
        subtable
    }

    /// Reads the unary region at the front of the byte array.
    fn unary(&self, layout: &SubtableLayout) -> u128 {
        let mut bytes = [0u8; 16];
        bytes[..layout.unary_bytes].copy_from_slice(&self.bytes[..layout.unary_bytes]);
        u128::from_le_bytes(bytes)
    }

    /// Stores the unary bit string at the front of the byte array.
    fn store_unary(&mut self, layout: &SubtableLayout, value: u128) {
        self.bytes[..layout.unary_bytes]
            .copy_from_slice(&value.to_le_bytes()[..layout.unary_bytes]);
    }

    /// Returns the current total number of Entries.
    fn entry_count(&self, layout: &SubtableLayout) -> usize {
        let bits = self.unary(layout);
        (128 - bits.leading_zeros() as usize) - layout.unary_count
    }

    /// Returns the half-open Entry-slot range used by a unary group.
    fn bounds(&self, layout: &SubtableLayout, unary_index: usize) -> (usize, usize) {
        unary_bounds(self.unary(layout), unary_index)
    }

    /// Unpacks the fields at a slot into a SubtableEntry.
    fn entry(&self, layout: &SubtableLayout, entry_slot: usize) -> SubtableEntry {
        SubtableEntry {
            unary_index: self.unary_index_at(layout, entry_slot),
            fingerprint: self.fingerprint(layout, entry_slot),
            value: self.value(layout, entry_slot),
            crumb: self.crumb(layout, entry_slot),
        }
    }

    /// Reads the fingerprint at a slot.
    fn fingerprint(&self, layout: &SubtableLayout, entry_slot: usize) -> u32 {
        get_bits(
            &self.bytes,
            layout.fingerprint_bit + entry_slot * layout.fingerprint_bits,
            layout.fingerprint_bits,
        ) as u32
    }

    /// Reads the value at a slot.
    fn value(&self, layout: &SubtableLayout, entry_slot: usize) -> u32 {
        get_bits(
            &self.bytes,
            layout.value_bit + entry_slot * layout.value_bits,
            layout.value_bits,
        ) as u32
    }

    /// Reads the crumb at a slot.
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

    /// Recovers the unary-group index containing an Entry slot.
    fn unary_index_at(&self, layout: &SubtableLayout, entry_slot: usize) -> usize {
        let bits = self.unary(layout);
        debug_assert!(entry_slot < self.entry_count(layout));
        let active_mask = low_mask(layout.unary_count + layout.entry_capacity);
        let entry_bit = select_one(!bits & active_mask, entry_slot);
        entry_bit - entry_slot
    }

    /// Writes an Entry's fingerprint, value, and crumb to a slot.
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

    /// Clears the Entry fields at a slot to zero.
    fn clear_entry(&mut self, layout: &SubtableLayout, entry_slot: usize) {
        self.write_entry(
            layout,
            entry_slot,
            SubtableEntry {
                unary_index: 0,
                fingerprint: 0,
                value: 0,
                crumb: 0,
            },
        );
    }

    /// Inserts an Entry into Front and returns the last overflow Entry when full.
    fn insert_front(
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

    /// Inserts an Entry into Back, or returns false without changes when full.
    fn insert_back(&mut self, layout: &SubtableLayout, entry: SubtableEntry) -> bool {
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

    /// Returns slots in a unary group matching a fingerprint and optional crumb.
    fn matching_entry_slots<'a>(
        &'a self,
        layout: &'a SubtableLayout,
        unary_index: usize,
        fingerprint: u32,
        crumb: Option<u8>,
    ) -> impl Iterator<Item = usize> + 'a {
        let (start, end) = self.bounds(layout, unary_index);
        self.matching_entry_slots_in_range(layout, start..end, fingerprint, crumb)
    }

    /// Returns slots in a range matching a fingerprint and crumb.
    fn matching_entry_slots_in_range<'a>(
        &'a self,
        layout: &'a SubtableLayout,
        range: std::ops::Range<usize>,
        fingerprint: u32,
        crumb: Option<u8>,
    ) -> impl Iterator<Item = usize> + 'a {
        range.filter(move |entry_slot| {
            self.fingerprint(layout, *entry_slot) == fingerprint
                && crumb.is_none_or(|candidate| self.crumb(layout, *entry_slot) == candidate)
        })
    }

    /// Returns the slot and contents of the first Entry with a specified crumb.
    fn first_with_crumb(
        &self,
        layout: &SubtableLayout,
        crumb: u8,
    ) -> Option<(usize, SubtableEntry)> {
        let entry_slot = (0..self.entry_count(layout))
            .find(|entry_slot| self.crumb(layout, *entry_slot) == crumb)?;
        Some((entry_slot, self.entry(layout, entry_slot)))
    }

    /// Removes a slot and shifts subsequent Entries forward.
    fn remove_at(
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

    /// Inserts one zero bit representing an Entry into the unary bit string.
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

    /// Removes the zero bit for a deleted Entry from the unary bit string.
    fn unary_remove(&mut self, layout: &SubtableLayout, unary_index: usize, entry_slot: usize) {
        let bits = self.unary(layout);
        let bit_index = unary_index + entry_slot;
        let lower_mask_value = (1u128 << bit_index) - 1;
        let lower = bits & lower_mask_value;
        let upper = (bits >> (bit_index + 1)) << bit_index;
        self.store_unary(layout, lower | upper);
    }
}

/// Returns the position of the zero-based `rank`th set bit in `bits`.
fn select_one(bits: u128, rank: usize) -> usize {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("bmi2") {
        return unsafe { select_one_bmi2(bits, rank) };
    }

    select_one_scalar(bits, rank)
}

/// Clears the lowest `rank` set bits and returns the position of the next set bit.
fn select_one_scalar(mut bits: u128, rank: usize) -> usize {
    for _ in 0..rank {
        bits &= bits - 1;
    }
    bits.trailing_zeros() as usize
}

/// Finds the `rank`th set bit with the x86 BMI2 PDEP instruction.
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

/// Creates a mask with only the lowest `bits` bits set.
fn low_mask(bits: usize) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

/// Creates a u32 mask with only the lowest `bits` bits set.
fn u32_mask(bits: usize) -> u32 {
    if bits == u32::BITS as usize {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    }
}

/// Computes the Entry-slot range for a group in a unary bit string.
fn unary_bounds(bits: u128, unary_index: usize) -> (usize, usize) {
    let end = select_one(bits, unary_index) - unary_index;
    let start = if unary_index == 0 {
        0
    } else {
        select_one(bits, unary_index - 1) - (unary_index - 1)
    };
    (start, end)
}

/// Reads `width` bits at an arbitrary bit offset in a byte array.
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

/// Writes the lowest `width` bits of a value at an arbitrary bit offset in a byte array.
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
