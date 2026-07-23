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
    pub(crate) len: usize,
}

impl LocationBreadcrumb {
    pub(crate) fn new(config: &Config) -> Result<Self> {
        let front_layout = BucketLayout::new(
            config.index_target_load_percent,
            config.fingerprint_bits,
            config.front_back_ratio,
            false,
        );
        let back_layout = BucketLayout::new(
            config.index_target_load_percent,
            config.fingerprint_bits,
            config.front_back_ratio,
            true,
        );
        let back_group_count = config.front_back_ratio + 1;
        let front_count = config.index_capacity.div_ceil(front_layout.capacity).max(1);
        let back_count = front_count.div_ceil(config.front_back_ratio)
            + back_group_count * config.front_back_ratio;
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
        if self.back[back0].insert_back(&self.back_layout, mini, tag, entry, crumb0) {
            self.len += 1;
            return Ok(());
        }
        let (back1, crumb1) = self.back_location(front, 1);
        if back1 != back0
            && self.back[back1].insert_back(&self.back_layout, mini, tag, entry, crumb1)
        {
            self.len += 1;
            return Ok(());
        }
        Err(KvError::IndexFull)
    }

    pub(crate) fn remove(&mut self, hash: &[u8; 32], location: Location) -> bool {
        let (front, mini, tag) = self.fingerprint(hash);
        let entry = location.encode(self.front_layout.region_bits);
        if self.front[front].remove_front(&self.front_layout, mini, tag, entry) {
            let (back0, crumb0) = self.back_location(front, 0);
            let (back1, crumb1) = self.back_location(front, 1);
            if self.front[front].should_promote(&self.front_layout) {
                if let Some(candidate) =
                    self.back[back0].first_with_crumb(&self.back_layout, crumb0)
                {
                    let (slot, promoted_entry, promoted_tag) = candidate;
                    self.back[back0].remove_at(&self.back_layout, slot);
                    self.front[front].insert_front(
                        &self.front_layout,
                        mini,
                        promoted_tag as u8,
                        promoted_entry,
                    );
                } else if let Some(candidate) =
                    self.back[back1].first_with_crumb(&self.back_layout, crumb1)
                {
                    let (slot, promoted_entry, promoted_tag) = candidate;
                    self.back[back1].remove_at(&self.back_layout, slot);
                    self.front[front].insert_front(
                        &self.front_layout,
                        mini,
                        promoted_tag as u8,
                        promoted_entry,
                    );
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
        if !self.remove(hash, previous) {
            return false;
        }
        self.insert(hash, replacement).is_ok()
    }

    pub(crate) fn load_factor(&self) -> f64 {
        let capacity = self.front.len() * self.front_layout.capacity;
        if capacity == 0 {
            0.0
        } else {
            self.len as f64 / capacity as f64
        }
    }

    pub(crate) fn memory_bytes(&self) -> usize {
        (self.front.len() + self.back.len()) * BUCKET_BYTES
    }

    fn fingerprint(&self, hash: &[u8; 32]) -> (usize, usize, u8) {
        let quotient_count = self.front.len() * self.front_layout.mini_buckets;
        let fingerprint_space = quotient_count as u64 * (1u64 << self.fingerprint_bits);
        let hash_prefix = u64::from_le_bytes(hash[0..8].try_into().unwrap());
        let fingerprint = hash_prefix % fingerprint_space;
        let quotient = (fingerprint >> self.fingerprint_bits) as usize;
        let front = quotient / self.front_layout.mini_buckets;
        let mini = quotient % self.front_layout.mini_buckets;
        (front, mini, fingerprint as u8)
    }

    fn back_location(&self, front: usize, choice: usize) -> (usize, u8) {
        let upper = front / self.ratio;
        let low = front % self.ratio;
        let first = (upper, (low + self.ratio) as u8);
        let second = (
            upper / self.ratio + low * self.back_group_count,
            (upper % self.ratio) as u8,
        );
        if choice == 0 { first } else { second }
    }
}
