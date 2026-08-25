//! hash를 1~32bit value 후보들로 연결하는 고정 크기 압축 Table이다.

use std::num::NonZeroU64;

const SUBTABLE_BYTES: usize = 64;
const TARGET_LOAD_PERCENT: usize = 88;
const MIN_UNARY_COUNT: usize = 2;
const MAX_UNARY_COUNT: usize = 96;
const FRONT_BACK_RATIOS: [usize; 4] = [2, 4, 8, 16];

/// Table이 담아야 할 Entry 수와 Entry 필드 크기다.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TableConfig {
    /// 지정한 목표 사용률에서 담아야 할 Entry 수다.
    pub(crate) max_entries: usize,
    /// Entry마다 저장할 value bit 수다.
    pub(crate) value_bits: u8,
    /// Entry마다 저장할 fingerprint bit 수다.
    pub(crate) fingerprint_bits: u8,
}

impl TableConfig {
    /// 외부에서 지정하는 세 값의 범위를 확인한다.
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

/// 고정 크기 Table을 만들지 못한 이유다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableCreateError {
    /// 외부에서 받은 설정값이 유효하지 않다.
    InvalidConfig(&'static str),
    /// 설정값을 담을 수 있는 64byte Subtable 배치가 없다.
    NoValidLayout,
    /// Front 또는 Back Subtable의 고정 배열을 확보하지 못했다.
    AllocationFailed,
}

/// 고정 크기 Table에 더 이상 Entry를 넣을 수 없다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TableFull;

/// Front와 Back Subtable의 개수 및 내부 배치를 계산한 결과다.
struct TableAllocation {
    /// Front Subtable 한 개의 64바이트 내부 배치다.
    front_subtable_layout: SubtableLayout,
    /// Back Subtable 한 개의 64바이트 내부 배치다.
    back_subtable_layout: SubtableLayout,
    /// 할당할 Front Subtable 개수다.
    front_subtable_count: usize,
    /// 할당할 Back Subtable 개수다.
    back_subtable_count: usize,
    /// 두 번째 Back 경로에서 한 묶음이 차지하는 Subtable 수다.
    back_subtable_group_count: usize,
    /// Front Subtable을 Back 경로에 묶는 단위다.
    front_back_ratio: usize,
}

impl TableAllocation {
    /// 가능한 배치를 전부 계산해 가장 적은 메모리로 목표 Entry를 담는 배치를 고른다.
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

/// 생성할 때 고정 크기로 할당되고 hash에 연결된 value만 보관한다.
pub(crate) struct Table {
    /// 모든 Entry가 먼저 들어가는 Front Subtable 배열이다.
    front_table: Box<[Subtable]>,
    /// Front에서 밀려난 Entry를 보관하는 Back Subtable 배열이다.
    back_table: Box<[Subtable]>,
    /// Front Subtable 내부의 bit 배치다.
    front_subtable_layout: SubtableLayout,
    /// Back Subtable 내부의 bit 배치다.
    back_subtable_layout: SubtableLayout,
    /// 두 번째 Back 경로에서 한 묶음이 차지하는 Subtable 수다.
    back_subtable_group_count: usize,
    /// Front Subtable들을 Back 경로에 묶는 단위다.
    front_back_ratio: usize,
    /// hash에서 fingerprint bit만 남기는 mask다.
    fingerprint_mask: u32,
    /// hash 좌표를 Front Subtable과 unary 그룹 범위에 접는 나머지 연산 값이다.
    coordinate_modulus: NonZeroU64,
    /// value의 허용된 bit만 1인 mask다.
    value_mask: u32,
}

impl Table {
    /// 자동으로 배치를 고른 뒤 Front와 Back 배열을 고정 크기로 할당한다.
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

    /// fingerprint가 맞는 value들을 별도 할당 없이 순회한다.
    pub(crate) fn values(&self, hash: u128) -> impl Iterator<Item = u32> + '_ {
        TableValues::new(self, hash)
    }

    /// hash와 value를 Front에 넣고 밀려난 Entry는 두 Back 후보 중 덜 찬 곳에 넣는다.
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

    /// fingerprint와 value가 모두 같은 Entry 하나를 제거한다.
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

    /// hash의 기존 value 하나를 새 value로 제자리에서 바꾼다.
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

    /// Back에 밀려난 Entry 하나를 Front의 빈 slot으로 되돌린다.
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

    /// hash를 Front Subtable index, unary index, fingerprint로 나눈다.
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

    /// Front Subtable가 사용할 수 있는 Back 후보 두 곳과 crumb를 계산한다.
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

/// Table의 Front와 필요한 Back 두 곳을 순차적으로 읽는 iterator다.
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

/// Subtable byte 배열에서 Entry 하나를 읽고 쓸 때 사용하는 형태다.
#[derive(Clone, Copy, Debug)]
struct SubtableEntry {
    /// 이 Entry가 속한 Subtable 내부 unary 그룹 번호다.
    unary_index: usize,
    /// 전체 key hash 중 Table에 저장하는 짧은 식별 값이다.
    fingerprint: u32,
    /// 외부에서 정한 의미를 Table이 해석하지 않고 그대로 보관하는 값이다.
    value: u32,
    /// 공유 Back Subtable 안에서 Entry의 Front 경로를 구분한다.
    crumb: u8,
}

/// 64바이트 Subtable 안의 각 영역 크기와 시작 bit를 보관한다.
#[derive(Clone, Copy, Debug)]
struct SubtableLayout {
    /// Subtable 하나의 unary 그룹 수다.
    unary_count: usize,
    /// Subtable 하나에 들어가는 최대 Entry 수다.
    entry_capacity: usize,
    /// Entry 하나의 fingerprint bit 수다.
    fingerprint_bits: usize,
    /// Entry 하나의 value bit 수다.
    value_bits: usize,
    /// Entry 하나의 crumb bit 수다. Front에서는 0이다.
    crumb_bits: usize,
    /// unary bit열에 예약된 byte 수다.
    unary_bytes: usize,
    /// Subtable 시작점에서 fingerprint 영역까지의 bit offset이다.
    fingerprint_bit: usize,
    /// Subtable 시작점에서 value 영역까지의 bit offset이다.
    value_bit: usize,
    /// Subtable 시작점에서 crumb 영역까지의 bit offset이다.
    crumb_bit: usize,
}

impl SubtableLayout {
    /// 설정된 필드가 64바이트에 들어가도록 최대 Entry 수를 계산한다.
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

/// 캐시라인 하나에 unary와 모든 Entry 필드를 압축해서 보관한다.
#[repr(C, align(64))]
#[derive(Clone)]
struct Subtable {
    /// `[unary][fingerprints][values][crumbs]`가 들어 있는 한 캐시라인이다.
    bytes: [u8; SUBTABLE_BYTES],
}

impl Subtable {
    /// Entry가 하나도 없는 Subtable을 만든다.
    fn new(layout: &SubtableLayout) -> Self {
        let mut subtable = Self {
            bytes: [0; SUBTABLE_BYTES],
        };
        subtable.store_unary(layout, (1u128 << layout.unary_count) - 1);
        subtable
    }

    /// byte 배열 앞의 unary 영역을 읽는다.
    fn unary(&self, layout: &SubtableLayout) -> u128 {
        let mut bytes = [0u8; 16];
        bytes[..layout.unary_bytes].copy_from_slice(&self.bytes[..layout.unary_bytes]);
        u128::from_le_bytes(bytes)
    }

    /// unary bit열을 byte 배열 앞에 저장한다.
    fn store_unary(&mut self, layout: &SubtableLayout, value: u128) {
        self.bytes[..layout.unary_bytes]
            .copy_from_slice(&value.to_le_bytes()[..layout.unary_bytes]);
    }

    /// 현재 들어 있는 전체 Entry 수를 반환한다.
    fn entry_count(&self, layout: &SubtableLayout) -> usize {
        let bits = self.unary(layout);
        (128 - bits.leading_zeros() as usize) - layout.unary_count
    }

    /// unary 그룹이 사용하는 Entry slot의 반열린 범위를 반환한다.
    fn bounds(&self, layout: &SubtableLayout, unary_index: usize) -> (usize, usize) {
        unary_bounds(self.unary(layout), unary_index)
    }

    /// 지정한 slot의 필드들을 SubtableEntry로 풀어서 반환한다.
    fn entry(&self, layout: &SubtableLayout, entry_slot: usize) -> SubtableEntry {
        SubtableEntry {
            unary_index: self.unary_index_at(layout, entry_slot),
            fingerprint: self.fingerprint(layout, entry_slot),
            value: self.value(layout, entry_slot),
            crumb: self.crumb(layout, entry_slot),
        }
    }

    /// 지정한 slot의 fingerprint를 읽는다.
    fn fingerprint(&self, layout: &SubtableLayout, entry_slot: usize) -> u32 {
        get_bits(
            &self.bytes,
            layout.fingerprint_bit + entry_slot * layout.fingerprint_bits,
            layout.fingerprint_bits,
        ) as u32
    }

    /// 지정한 slot의 value를 읽는다.
    fn value(&self, layout: &SubtableLayout, entry_slot: usize) -> u32 {
        get_bits(
            &self.bytes,
            layout.value_bit + entry_slot * layout.value_bits,
            layout.value_bits,
        ) as u32
    }

    /// 지정한 slot의 crumb를 읽는다.
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

    /// Entry slot이 속한 unary 그룹 번호를 복원한다.
    fn unary_index_at(&self, layout: &SubtableLayout, entry_slot: usize) -> usize {
        let bits = self.unary(layout);
        debug_assert!(entry_slot < self.entry_count(layout));
        let active_mask = low_mask(layout.unary_count + layout.entry_capacity);
        let entry_bit = select_one(!bits & active_mask, entry_slot);
        entry_bit - entry_slot
    }

    /// Entry의 fingerprint, value, crumb를 지정한 slot에 쓴다.
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

    /// 지정한 slot의 Entry 필드를 0으로 덮는다.
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

    /// Front에 Entry를 넣고 가득 찼다면 끝에서 밀려난 Entry를 반환한다.
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

    /// Back에 Entry를 넣고 가득 찼으면 변경 없이 false를 반환한다.
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

    /// unary 그룹에서 fingerprint와 선택적인 crumb가 같은 slot들을 반환한다.
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

    /// 지정된 slot 범위에서 fingerprint와 crumb가 같은 slot들을 반환한다.
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

    /// 지정한 crumb를 가진 첫 Entry의 slot과 내용을 반환한다.
    fn first_with_crumb(
        &self,
        layout: &SubtableLayout,
        crumb: u8,
    ) -> Option<(usize, SubtableEntry)> {
        let entry_slot = (0..self.entry_count(layout))
            .find(|entry_slot| self.crumb(layout, *entry_slot) == crumb)?;
        Some((entry_slot, self.entry(layout, entry_slot)))
    }

    /// 지정한 slot을 제거하고 뒤 Entry들을 앞으로 당긴다.
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

    /// unary bit열에 Entry를 뜻하는 0 bit 하나를 삽입한다.
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

    /// unary bit열에서 제거된 Entry의 0 bit 하나를 없앤다.
    fn unary_remove(&mut self, layout: &SubtableLayout, unary_index: usize, entry_slot: usize) {
        let bits = self.unary(layout);
        let bit_index = unary_index + entry_slot;
        let lower_mask_value = (1u128 << bit_index) - 1;
        let lower = bits & lower_mask_value;
        let upper = (bits >> (bit_index + 1)) << bit_index;
        self.store_unary(layout, lower | upper);
    }
}

/// `bits`에서 0부터 센 `rank`번째 1 bit 위치를 반환한다.
fn select_one(bits: u128, rank: usize) -> usize {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("bmi2") {
        return unsafe { select_one_bmi2(bits, rank) };
    }

    select_one_scalar(bits, rank)
}

/// 낮은 1 bit를 rank개 지운 뒤 다음 1 bit 위치를 반환한다.
fn select_one_scalar(mut bits: u128, rank: usize) -> usize {
    for _ in 0..rank {
        bits &= bits - 1;
    }
    bits.trailing_zeros() as usize
}

/// x86 BMI2의 PDEP 명령으로 rank번째 1 bit 위치를 찾는다.
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

/// 가장 낮은 `bits`개 bit만 1인 mask를 만든다.
fn low_mask(bits: usize) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

/// 낮은 `bits`개 bit만 1인 u32 mask를 만든다.
fn u32_mask(bits: usize) -> u32 {
    if bits == u32::BITS as usize {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    }
}

/// unary bit열에서 지정한 그룹의 Entry slot 범위를 계산한다.
fn unary_bounds(bits: u128, unary_index: usize) -> (usize, usize) {
    let end = select_one(bits, unary_index) - unary_index;
    let start = if unary_index == 0 {
        0
    } else {
        select_one(bits, unary_index - 1) - (unary_index - 1)
    };
    (start, end)
}

/// byte 배열의 임의 bit 위치에서 width만큼 읽는다.
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

/// byte 배열의 임의 bit 위치에 value의 낮은 width bit를 기록한다.
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
