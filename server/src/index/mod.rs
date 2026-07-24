use std::collections::HashSet;

use crate::BUCKET_BYTES;
use crate::config::Config;
use crate::error::{KvError, Result};

mod packed_bucket;
pub(crate) use self::packed_bucket::*;

pub(crate) struct LocationBreadcrumb {
    pub(crate) front: Vec<PackedBucket>,
    pub(crate) back: Vec<PackedBucket>,
    pub(crate) front_layout: BucketLayout,
    pub(crate) back_layout: BucketLayout,
    pub(crate) back_group_count: usize,
    ratio: usize,
    fingerprint_bits: usize,
    fingerprint_hash_offset_bits: usize,
    sg_index_bits: usize,
    pub(crate) len: usize,
}

impl LocationBreadcrumb {
    pub(crate) fn new(config: &Config) -> Result<Self> {
        let front_layout = BucketLayout::new(
            config.index_target_load_percent,
            config.fingerprint_bits,
            config.front_back_ratio,
            false,
            config.mini_buckets,
            config.region_bits,
        )?;
        let back_layout = BucketLayout::new(
            config.index_target_load_percent,
            config.fingerprint_bits,
            config.front_back_ratio,
            true,
            config.mini_buckets,
            config.region_bits,
        )?;
        let slots_per_front = front_layout.capacity as f64
            + back_layout.capacity as f64 / config.front_back_ratio as f64;
        let planned_per_front = slots_per_front * config.index_target_load_percent as f64 / 100.0;
        let front_count = (config.index_capacity as f64 / planned_per_front)
            .ceil()
            .max(1.0) as usize;
        let back_group_count = front_count
            .div_ceil(config.front_back_ratio * config.front_back_ratio)
            .max(1);
        let back_count = front_count
            .div_ceil(config.front_back_ratio)
            .max(config.front_back_ratio * back_group_count);
        Ok(Self {
            front: (0..front_count)
                .map(|_| PackedBucket::new(&front_layout))
                .collect(),
            back: (0..back_count)
                .map(|_| PackedBucket::new(&back_layout))
                .collect(),
            front_layout,
            back_layout,
            back_group_count,
            ratio: config.front_back_ratio,
            fingerprint_bits: config.fingerprint_bits,
            fingerprint_hash_offset_bits: config.fingerprint_hash_offset_bits,
            sg_index_bits: config.region_bits,
            len: 0,
        })
    }

    pub(crate) fn candidates(&self, hash: &[u8; 32]) -> Vec<TableLocation> {
        let (front, mini, tag) = self.fingerprint(hash);
        let front_bucket = &self.front[front];
        let mut encoded = front_bucket
            .matching_slots(&self.front_layout, mini, tag, None)
            .into_iter()
            .map(|slot| front_bucket.entry(&self.front_layout, slot).location)
            .collect::<Vec<_>>();
        let (_, end) = front_bucket.bounds(&self.front_layout, mini);
        if end == self.front_layout.capacity {
            let [first, second] = self.back_locations(front);
            for bl in [first, second] {
                let bucket = &self.back[bl.0];
                encoded.extend(
                    bucket
                        .matching_slots(&self.back_layout, mini, tag, Some(bl.1))
                        .into_iter()
                        .map(|slot| bucket.entry(&self.back_layout, slot).location),
                );
            }
        }
        let mut seen = HashSet::new();
        encoded
            .into_iter()
            .map(|encoded_location| TableLocation::decode(encoded_location, self.sg_index_bits))
            .filter(|location| seen.insert(*location))
            .collect()
    }

    pub(crate) fn insert(&mut self, hash: &[u8; 32], location: TableLocation) -> Result<()> {
        let (front, mini, tag) = self.fingerprint(hash);
        let entry = PackedEntry {
            mini,
            tag,
            location: location.encode(self.sg_index_bits),
            crumb: 0,
        };
        let saved = self.front[front].clone();
        let overflow = self.front[front].insert_front(&self.front_layout, entry);
        let Some(mut overflow) = overflow else {
            self.len += 1;
            return Ok(());
        };
        let [first, second] = self.back_locations(front);
        let destination = if self.back[first.0].len(&self.back_layout)
            <= self.back[second.0].len(&self.back_layout)
        {
            first
        } else {
            second
        };
        overflow.crumb = destination.1;
        if !self.back[destination.0].insert_back(&self.back_layout, overflow) {
            self.front[front] = saved;
            return Err(KvError::IndexFull);
        }
        self.len += 1;
        Ok(())
    }

    pub(crate) fn remove(&mut self, hash: &[u8; 32], location: TableLocation) -> bool {
        let (front, mini, tag) = self.fingerprint(hash);
        let encoded = location.encode(self.sg_index_bits);
        let was_full = self.front[front].len(&self.front_layout) == self.front_layout.capacity;
        let front_slots = self.front[front].matching_slots(&self.front_layout, mini, tag, None);
        if let Some(slot) = front_slots
            .into_iter()
            .find(|slot| self.front[front].entry(&self.front_layout, *slot).location == encoded)
        {
            self.front[front].remove_at(&self.front_layout, mini, slot);
            if was_full {
                self.promote(front);
            }
            self.len -= 1;
            return true;
        }
        if !was_full {
            return false;
        }
        for bl in self.back_locations(front) {
            let slots = self.back[bl.0].matching_slots(&self.back_layout, mini, tag, Some(bl.1));
            if let Some(slot) = slots
                .into_iter()
                .find(|slot| self.back[bl.0].entry(&self.back_layout, *slot).location == encoded)
            {
                self.back[bl.0].remove_at(&self.back_layout, mini, slot);
                self.len -= 1;
                return true;
            }
        }
        false
    }

    pub(crate) fn replace_location(
        &mut self,
        hash: &[u8; 32],
        previous: TableLocation,
        replacement: TableLocation,
    ) -> bool {
        if previous == replacement {
            return true;
        }
        let (front, mini, tag) = self.fingerprint(hash);
        let old = previous.encode(self.sg_index_bits);
        let new = replacement.encode(self.sg_index_bits);
        let front_slots = self.front[front].matching_slots(&self.front_layout, mini, tag, None);
        if let Some(slot) = front_slots
            .into_iter()
            .find(|slot| self.front[front].entry(&self.front_layout, *slot).location == old)
        {
            let mut entry = self.front[front].entry(&self.front_layout, slot);
            entry.location = new;
            self.front[front].write_entry(&self.front_layout, slot, entry);
            return true;
        }
        if self.front[front].len(&self.front_layout) < self.front_layout.capacity {
            return false;
        }
        for bl in self.back_locations(front) {
            let slots = self.back[bl.0].matching_slots(&self.back_layout, mini, tag, Some(bl.1));
            if let Some(slot) = slots
                .into_iter()
                .find(|slot| self.back[bl.0].entry(&self.back_layout, *slot).location == old)
            {
                let mut entry = self.back[bl.0].entry(&self.back_layout, slot);
                entry.location = new;
                self.back[bl.0].write_entry(&self.back_layout, slot, entry);
                return true;
            }
        }
        false
    }

    fn promote(&mut self, front: usize) {
        let [first, second] = self.back_locations(front);
        let a = self.back[first.0].first_with_crumb(&self.back_layout, first.1);
        let b = self.back[second.0].first_with_crumb(&self.back_layout, second.1);
        let selected = match (a, b) {
            (None, None) => return,
            (Some(candidate), None) => (first.0, candidate),
            (None, Some(candidate)) => (second.0, candidate),
            (Some(a), Some(b)) if a.1.mini <= b.1.mini => (first.0, a),
            (Some(_), Some(b)) => (second.0, b),
        };
        let (back_idx, (slot, mut entry)) = selected;
        self.back[back_idx].remove_at(&self.back_layout, entry.mini, slot);
        entry.crumb = 0;
        let overflow = self.front[front].insert_front(&self.front_layout, entry);
        debug_assert!(overflow.is_none());
    }

    pub(crate) fn load_factor(&self) -> f64 {
        let capacity = self.front.len() * self.front_layout.capacity
            + self.back.len() * self.back_layout.capacity;
        if capacity == 0 {
            0.0
        } else {
            self.len as f64 / capacity as f64
        }
    }

    pub(crate) fn memory_bytes(&self) -> usize {
        (self.front.len() + self.back.len()) * BUCKET_BYTES
    }

    fn fingerprint(&self, hash: &[u8; 32]) -> (usize, usize, u16) {
        let prefix = u128::from_le_bytes(hash[..16].try_into().unwrap())
            >> self.fingerprint_hash_offset_bits;
        let prefix = prefix as u64;
        let quotient_count = self.front.len() * self.front_layout.mini_buckets;
        let remainder_space = 1u64 << self.fingerprint_bits;
        let space = (quotient_count as u128) * remainder_space as u128;
        let fingerprint = (prefix as u128 % space) as u64;
        let quotient = (fingerprint / remainder_space) as usize;
        let front_idx = quotient / self.front_layout.mini_buckets;
        let mini = quotient % self.front_layout.mini_buckets;
        (
            front_idx,
            mini,
            (fingerprint & (remainder_space - 1)) as u16,
        )
    }

    fn back_locations(&self, front: usize) -> [(usize, u8); 2] {
        let upper = front / self.ratio;
        let low = front % self.ratio;
        let first = (upper, (self.ratio + low) as u8);
        let second = (
            upper / self.ratio + low * self.back_group_count,
            (upper % self.ratio) as u8,
        );
        debug_assert!(
            first.0 < self.back.len(),
            "back bucket {} out of bounds (front={front})",
            first.0
        );
        debug_assert!(
            second.0 < self.back.len(),
            "back bucket {} out of bounds (front={front})",
            second.0
        );
        [first, second]
    }
}

pub(crate) type Table = LocationBreadcrumb;
