//! Two-tier front/back bucket index used for location breadcrumbs. The index maps
//! hashed keys to storage locations via [`LocationBreadcrumb`], which maintains a front
//! and back set of [`PackedBucket`]s and supports lookup, insert, delete, and promotion.

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
            len: 0,
        })
    }

    pub(crate) fn candidates(&self, hash: &[u8; 32]) -> Vec<Location> {
        let (front, mini, tag) = self.fingerprint(hash);

        let mut results = Vec::with_capacity(3);

        let extract = |entries: Vec<u16>| entries.into_iter().map(Location::decode);

        results.extend(extract(self.front[front].find_entries(
            &self.front_layout,
            mini,
            tag,
        )));

        let (back0, _crumb0) = self.back_location(front, 0);
        results.extend(extract(self.back[back0].find_entries(
            &self.back_layout,
            mini,
            tag,
        )));
        let (back1, _) = self.back_location(front, 1);
        if back1 != back0 {
            results.extend(extract(self.back[back1].find_entries(
                &self.back_layout,
                mini,
                tag,
            )));
        }
        results.sort_by_key(|loc| std::cmp::Reverse((loc.region, loc.page_choice)));
        results.dedup();
        results
    }

    pub(crate) fn insert(&mut self, hash: &[u8; 32], location: Location) -> Result<()> {
        let (front, mini, tag) = self.fingerprint(hash);
        let entry = location.encode(self.front_layout.region_bits);
        if self.front[front].insert_front(&self.front_layout, mini, tag, entry) {
            self.len += 1;
            return Ok(());
        }

        let (back0, crumb0) = self.back_location(front, 0);
        let (back1, crumb1) = self.back_location(front, 1);
        let (primary, primary_crumb, secondary, secondary_crumb) =
            if self.back[back0].len(&self.back_layout) <= self.back[back1].len(&self.back_layout) {
                (back0, crumb0, back1, crumb1)
            } else {
                (back1, crumb1, back0, crumb0)
            };
        if self.back[primary].insert_back(&self.back_layout, mini, tag, entry, primary_crumb) {
            self.len += 1;
            return Ok(());
        }
        if primary != secondary
            && self.back[secondary].insert_back(
                &self.back_layout,
                mini,
                tag,
                entry,
                secondary_crumb,
            )
        {
            self.len += 1;
            return Ok(());
        }
        Err(KvError::IndexFull)
    }

    pub(crate) fn remove(&mut self, hash: &[u8; 32], location: Location) -> bool {
        let (front, mini, tag) = self.fingerprint(hash);
        let entry = location.encode(self.front_layout.region_bits);
        let mini_was_full = self.front[front].mini_slots_free(&self.front_layout, mini) == 0;
        if self.front[front].remove_front(&self.front_layout, mini, tag, entry) {
            let (back0, crumb0) = self.back_location(front, 0);
            let (back1, crumb1) = self.back_location(front, 1);
            if mini_was_full {
                let mut promoted = false;
                if let Some(candidate) =
                    self.back[back0].first_with_crumb(&self.back_layout, crumb0)
                {
                    let (slot, promoted_entry, promoted_tag) = candidate;
                    let promoted_mini =
                        slot / (self.back_layout.capacity / self.back_layout.mini_buckets);
                    if self.front[front].mini_slots_free(&self.front_layout, promoted_mini) > 0 {
                        self.back[back0].remove_at(&self.back_layout, slot);
                        self.front[front].insert_front(
                            &self.front_layout,
                            promoted_mini,
                            promoted_tag,
                            promoted_entry,
                        );
                        promoted = true;
                    }
                }
                #[allow(clippy::collapsible_if)]
                if !promoted {
                    if let Some(candidate) =
                        self.back[back1].first_with_crumb(&self.back_layout, crumb1)
                    {
                        let (slot, promoted_entry, promoted_tag) = candidate;
                        let promoted_mini =
                            slot / (self.back_layout.capacity / self.back_layout.mini_buckets);
                        if self.front[front].mini_slots_free(&self.front_layout, promoted_mini) > 0
                        {
                            self.back[back1].remove_at(&self.back_layout, slot);
                            self.front[front].insert_front(
                                &self.front_layout,
                                promoted_mini,
                                promoted_tag,
                                promoted_entry,
                            );
                        }
                    }
                }
            }
            self.len -= 1;
            true
        } else {
            let (back0, crumb0) = self.back_location(front, 0);
            let (back1, crumb1) = self.back_location(front, 1);
            if self.back[back0].remove_back(&self.back_layout, mini, tag, crumb0, entry)
                || self.back[back1].remove_back(&self.back_layout, mini, tag, crumb1, entry)
            {
                self.len -= 1;
                true
            } else {
                false
            }
        }
    }

    pub(crate) fn replace_location(
        &mut self,
        hash: &[u8; 32],
        previous: Location,
        replacement: Location,
    ) -> bool {
        if previous == replacement {
            return true;
        }
        let (front, mini, tag) = self.fingerprint(hash);
        let old = previous.encode(self.front_layout.region_bits);
        let new = replacement.encode(self.front_layout.region_bits);
        if self.front[front].replace_entry(&self.front_layout, mini, tag, old, new) {
            return true;
        }
        let (back0, _) = self.back_location(front, 0);
        let (back1, _) = self.back_location(front, 1);
        if self.back[back0].replace_entry(&self.back_layout, mini, tag, old, new)
            || (back1 != back0
                && self.back[back1].replace_entry(&self.back_layout, mini, tag, old, new))
        {
            return true;
        }
        false
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
        let front = quotient / self.front_layout.mini_buckets;
        let mini = quotient % self.front_layout.mini_buckets;
        (front, mini, (fingerprint & (remainder_space - 1)) as u16)
    }

    fn back_location(&self, front: usize, choice: usize) -> (usize, u8) {
        let upper = front / self.ratio;
        let low = front % self.ratio;
        let first = (upper, (low + self.ratio) as u8);
        let second = (
            upper / self.ratio + low * self.back_group_count,
            (upper % self.ratio) as u8,
        );
        let result = if choice == 0 { first } else { second };
        debug_assert!(
            result.0 < self.back.len(),
            "back bucket {} out of bounds (front={front}, choice={choice})",
            result.0
        );
        result
    }
}
