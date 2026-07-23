// Breadcrumb fingerprint routing and compact physical-location index operations.

pub(crate) struct LocationBreadcrumb {
    front: Vec<PackedBucket>,
    back: Vec<PackedBucket>,
    front_layout: BucketLayout,
    back_layout: BucketLayout,
    back_group_count: usize,
    ratio: usize,
    fingerprint_bits: usize,
    fingerprint_hash_offset_bits: usize,
    region_bits: usize,
    len: usize,
}

impl LocationBreadcrumb {
    pub(crate) fn new(config: &Config) -> Result<Self> {
        let location_bits = config.region_bits + 1;
        let crumb_bits = (config.front_back_ratio * 2).ilog2() as usize;
        let front_layout = BucketLayout::new(
            config.mini_buckets,
            config.fingerprint_bits,
            location_bits,
            0,
        )?;
        let back_layout = BucketLayout::new(
            config.mini_buckets,
            config.fingerprint_bits,
            location_bits,
            crumb_bits,
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
            region_bits: config.region_bits,
            len: 0,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    fn slot_capacity(&self) -> usize {
        self.front.len() * self.front_layout.capacity + self.back.len() * self.back_layout.capacity
    }

    fn load_factor(&self) -> f64 {
        self.len as f64 / self.slot_capacity() as f64
    }

    fn memory_bytes(&self) -> usize {
        (self.front.len() + self.back.len()) * BUCKET_BYTES
    }

    fn fingerprint(&self, hash: &[u8; 32]) -> Fingerprint {
        let prefix = u128::from_le_bytes(hash[..16].try_into().unwrap())
            >> self.fingerprint_hash_offset_bits;
        let prefix = prefix as u64;
        let quotient_count = self.front.len() * self.front_layout.mini_buckets;
        let remainder_space = 1u64 << self.fingerprint_bits;
        let space = (quotient_count as u128) * remainder_space as u128;
        let fingerprint = (prefix as u128 % space) as u64;
        let quotient = (fingerprint / remainder_space) as usize;
        Fingerprint {
            front: quotient / self.front_layout.mini_buckets,
            mini: quotient % self.front_layout.mini_buckets,
            remainder: (fingerprint & (remainder_space - 1)) as u16,
        }
    }

    pub(crate) fn candidates(&self, hash: &[u8; 32]) -> Vec<Location> {
        let fingerprint = self.fingerprint(hash);
        let front = &self.front[fingerprint.front];
        let mut encoded = front
            .matching_slots(
                &self.front_layout,
                fingerprint.mini,
                fingerprint.remainder,
                None,
            )
            .into_iter()
            .map(|slot| front.entry(&self.front_layout, slot).location)
            .collect::<Vec<_>>();
        let (_, end) = front.bounds(&self.front_layout, fingerprint.mini);
        if end == self.front_layout.capacity {
            let [first, second] = self.back_locations(fingerprint.front);
            for location in [first, second] {
                let bucket = &self.back[location.bucket];
                encoded.extend(
                    bucket
                        .matching_slots(
                            &self.back_layout,
                            fingerprint.mini,
                            fingerprint.remainder,
                            Some(location.crumb),
                        )
                        .into_iter()
                        .map(|slot| bucket.entry(&self.back_layout, slot).location),
                );
            }
        }
        let mut seen = HashSet::new();
        encoded
            .into_iter()
            .map(Location::decode)
            .filter(|location| seen.insert(*location))
            .collect()
    }

    pub(crate) fn insert(&mut self, hash: &[u8; 32], location: Location) -> Result<()> {
        let fingerprint = self.fingerprint(hash);
        let entry = PackedEntry {
            mini: fingerprint.mini,
            remainder: fingerprint.remainder,
            location: location.encode(self.region_bits),
            crumb: 0,
        };
        let saved = self.front[fingerprint.front].clone();
        let overflow = self.front[fingerprint.front].insert_front(&self.front_layout, entry);
        let Some(mut overflow) = overflow else {
            self.len += 1;
            return Ok(());
        };
        let [first, second] = self.back_locations(fingerprint.front);
        let destination = if self.back[first.bucket].len(&self.back_layout)
            <= self.back[second.bucket].len(&self.back_layout)
        {
            first
        } else {
            second
        };
        overflow.crumb = destination.crumb;
        if !self.back[destination.bucket].insert_back(&self.back_layout, overflow) {
            self.front[fingerprint.front] = saved;
            return Err(KvError::IndexFull);
        }
        self.len += 1;
        Ok(())
    }

    pub(crate) fn remove(&mut self, hash: &[u8; 32], location: Location) -> bool {
        let fingerprint = self.fingerprint(hash);
        let encoded = location.encode(self.region_bits);
        let was_full =
            self.front[fingerprint.front].len(&self.front_layout) == self.front_layout.capacity;
        let front_slots = self.front[fingerprint.front].matching_slots(
            &self.front_layout,
            fingerprint.mini,
            fingerprint.remainder,
            None,
        );
        if let Some(slot) = front_slots.into_iter().find(|slot| {
            self.front[fingerprint.front]
                .entry(&self.front_layout, *slot)
                .location
                == encoded
        }) {
            self.front[fingerprint.front].remove_at(&self.front_layout, fingerprint.mini, slot);
            if was_full {
                self.promote(fingerprint.front);
            }
            self.len -= 1;
            return true;
        }
        if !was_full {
            return false;
        }
        for back in self.back_locations(fingerprint.front) {
            let slots = self.back[back.bucket].matching_slots(
                &self.back_layout,
                fingerprint.mini,
                fingerprint.remainder,
                Some(back.crumb),
            );
            if let Some(slot) = slots.into_iter().find(|slot| {
                self.back[back.bucket]
                    .entry(&self.back_layout, *slot)
                    .location
                    == encoded
            }) {
                self.back[back.bucket].remove_at(&self.back_layout, fingerprint.mini, slot);
                self.len -= 1;
                return true;
            }
        }
        false
    }

    fn replace_location(
        &mut self,
        hash: &[u8; 32],
        previous: Location,
        replacement: Location,
    ) -> bool {
        if previous == replacement {
            return true;
        }
        let fingerprint = self.fingerprint(hash);
        let previous = previous.encode(self.region_bits);
        let replacement = replacement.encode(self.region_bits);
        let front_slots = self.front[fingerprint.front].matching_slots(
            &self.front_layout,
            fingerprint.mini,
            fingerprint.remainder,
            None,
        );
        if let Some(slot) = front_slots.into_iter().find(|slot| {
            self.front[fingerprint.front]
                .entry(&self.front_layout, *slot)
                .location
                == previous
        }) {
            let mut entry = self.front[fingerprint.front].entry(&self.front_layout, slot);
            entry.location = replacement;
            self.front[fingerprint.front].write_entry(&self.front_layout, slot, entry);
            return true;
        }
        if self.front[fingerprint.front].len(&self.front_layout) < self.front_layout.capacity {
            return false;
        }
        for back in self.back_locations(fingerprint.front) {
            let slots = self.back[back.bucket].matching_slots(
                &self.back_layout,
                fingerprint.mini,
                fingerprint.remainder,
                Some(back.crumb),
            );
            if let Some(slot) = slots.into_iter().find(|slot| {
                self.back[back.bucket]
                    .entry(&self.back_layout, *slot)
                    .location
                    == previous
            }) {
                let mut entry = self.back[back.bucket].entry(&self.back_layout, slot);
                entry.location = replacement;
                self.back[back.bucket].write_entry(&self.back_layout, slot, entry);
                return true;
            }
        }
        false
    }

    fn promote(&mut self, front: usize) {
        let [first, second] = self.back_locations(front);
        let a = self.back[first.bucket].first_with_crumb(&self.back_layout, first.crumb);
        let b = self.back[second.bucket].first_with_crumb(&self.back_layout, second.crumb);
        let selected = match (a, b) {
            (None, None) => return,
            (Some(candidate), None) => (first, candidate),
            (None, Some(candidate)) => (second, candidate),
            (Some(a), Some(b)) if a.1.mini <= b.1.mini => (first, a),
            (Some(_), Some(b)) => (second, b),
        };
        let (back, (slot, mut entry)) = selected;
        self.back[back.bucket].remove_at(&self.back_layout, entry.mini, slot);
        entry.crumb = 0;
        let overflow = self.front[front].insert_front(&self.front_layout, entry);
        debug_assert!(overflow.is_none());
    }

    fn back_locations(&self, front: usize) -> [BackLocation; 2] {
        let upper = front / self.ratio;
        let low = front % self.ratio;
        let first = BackLocation {
            bucket: front / self.ratio,
            crumb: (self.ratio + low) as u8,
        };
        let second = BackLocation {
            bucket: upper / self.ratio + low * self.back_group_count,
            crumb: (upper % self.ratio) as u8,
        };
        debug_assert!(first.bucket < self.back.len());
        debug_assert!(second.bucket < self.back.len());
        [first, second]
    }
}
