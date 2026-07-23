//! Bit-packed bucket storage for the two-tier index. Defines the [`PackedBucket`] type
//! (a fixed-size byte array with per-slot metadata), [`Location`] (region + page choice),
//! [`BucketLayout`] (fingerprint/region/mini-bucket layout parameters), and
//! [`PackedEntry`] (decoded tag + location).

use crate::BUCKET_BYTES;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Location {
    pub(crate) region: u8,
    #[allow(dead_code)]
    pub(crate) page_choice: u8,
}

impl Location {
    pub(crate) fn encode(self, region_bits: usize) -> u16 {
        debug_assert!((self.region as usize) < (1usize << region_bits));
        ((self.region as u16) << 1) | self.page_choice as u16
    }

    pub(crate) fn decode(entry: u16) -> Self {
        Self {
            region: (entry >> 1) as u8,
            page_choice: (entry & 1) as u8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BucketLayout {
    pub(crate) fingerprint_bits: usize,
    pub(crate) region_bits: usize,
    pub(crate) mini_buckets: usize,
    pub(crate) capacity: usize,
    pub(crate) metadata_bits: usize,
    pub(crate) metadata_bytes: usize,
    pub(crate) crumb_bits: usize,
}

impl BucketLayout {
    pub(crate) fn new(
        target_load_percent: usize,
        fingerprint_bits: usize,
        front_back_ratio: usize,
        is_back: bool,
    ) -> Self {
        let location_bytes = std::mem::size_of::<u16>();
        let crumb_bits = if is_back {
            front_back_ratio.ilog2() as usize + 1
        } else {
            0
        };
        let _crumb_bytes = if is_back {
            (BUCKET_BYTES * 8 * crumb_bits).div_ceil(8)
        } else {
            0
        };
        let tag_bits = fingerprint_bits;
        let entry_bits = tag_bits + location_bytes * 8 + crumb_bits;
        let capacity = (BUCKET_BYTES * 8 * target_load_percent / 100) / entry_bits;
        let metadata_bits = capacity + 1;
        let metadata_bytes = metadata_bits.div_ceil(8);
        let used_bytes = metadata_bytes + capacity * entry_bits / 8;
        let mut mini_buckets = 1;
        while mini_buckets * used_bytes < BUCKET_BYTES {
            mini_buckets *= 2;
        }
        mini_buckets = if mini_buckets <= 1 {
            1
        } else {
            mini_buckets / 2
        };
        let adjusted_capacity = capacity / mini_buckets * mini_buckets;
        let adjusted_metadata_bits = adjusted_capacity + 1;
        let adjusted_metadata_bytes = adjusted_metadata_bits.div_ceil(8);
        let region_bits = if fingerprint_bits > 0 { 8 } else { 0 };
        Self {
            fingerprint_bits,
            region_bits,
            mini_buckets,
            capacity: adjusted_capacity,
            metadata_bits: adjusted_metadata_bits,
            metadata_bytes: adjusted_metadata_bytes,
            crumb_bits,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PackedEntry {
    pub(crate) tag: u16,
    pub(crate) location: u16,
    pub(crate) region: u8,
    #[allow(dead_code)]
    pub(crate) page_choice: u8,
}

#[derive(Clone)]
pub(crate) struct PackedBucket {
    pub(crate) bytes: [u8; BUCKET_BYTES],
}

impl PackedBucket {
    pub(crate) fn new(_layout: &BucketLayout) -> Self {
        Self {
            bytes: [0; BUCKET_BYTES],
        }
    }

    fn metadata(&self, layout: &BucketLayout) -> &[u8] {
        &self.bytes[..layout.metadata_bytes]
    }

    pub(crate) fn len(&self, layout: &BucketLayout) -> usize {
        let metadata = self.metadata(layout);
        metadata
            .iter()
            .flat_map(|byte| (0..8u8).map(move |i| (byte >> i) & 1))
            .take(layout.metadata_bits)
            .map(|b| b as usize)
            .sum()
    }

    pub(crate) fn entry(&self, layout: &BucketLayout, slot: usize) -> PackedEntry {
        let entry_bits = layout.fingerprint_bits + 16 + layout.crumb_bits;
        let bit_offset = slot * entry_bits;
        let byte_offset = bit_offset / 8 + layout.metadata_bytes;
        let bit_remainder = bit_offset % 8;
        let read_bytes = (entry_bits + bit_remainder).div_ceil(8).min(8);

        let mut raw = 0u64;
        for i in 0..read_bytes {
            if byte_offset + i < BUCKET_BYTES {
                raw |= (self.bytes[byte_offset + i] as u64) << (i * 8);
            }
        }
        let value = raw >> bit_remainder;
        let tag = (value & ((1u64 << layout.fingerprint_bits) - 1)) as u16;
        let entry = (value >> layout.fingerprint_bits) as u16;
        let location = Location::decode(entry);
        let crumb = if layout.crumb_bits > 0 {
            ((value >> (layout.fingerprint_bits + 16)) & ((1u64 << layout.crumb_bits) - 1)) as u8
        } else {
            0
        };
        PackedEntry {
            tag,
            location: entry,
            region: crumb,
            page_choice: location.page_choice,
        }
    }

    fn set_entry(&mut self, layout: &BucketLayout, slot: usize, entry: u16, tag: u16, crumb: u8) {
        let entry_bits = layout.fingerprint_bits + 16 + layout.crumb_bits;
        let tag_mask = (1u64 << layout.fingerprint_bits).wrapping_sub(1);
        let crumb_mask = if layout.crumb_bits > 0 {
            (1u64 << layout.crumb_bits).wrapping_sub(1)
        } else {
            0
        };
        let combined = ((tag as u64) & tag_mask)
            | ((entry as u64) << layout.fingerprint_bits)
            | (((crumb as u64) & crumb_mask) << (layout.fingerprint_bits + 16));
        let bit_offset = slot * entry_bits;
        let byte_offset = bit_offset / 8 + layout.metadata_bytes;
        let bit_remainder = bit_offset % 8;
        let write_bytes = (entry_bits + bit_remainder).div_ceil(8).min(8);

        let mut old = 0u64;
        for i in 0..write_bytes {
            if byte_offset + i < BUCKET_BYTES {
                old |= (self.bytes[byte_offset + i] as u64) << (i * 8);
            }
        }

        let entry_mask = ((1u64 << entry_bits) - 1) << bit_remainder;
        old &= !entry_mask;
        old |= combined << bit_remainder;

        for i in 0..write_bytes {
            if byte_offset + i < BUCKET_BYTES {
                self.bytes[byte_offset + i] = (old >> (i * 8)) as u8;
            }
        }
    }

    fn metadata_set(&mut self, layout: &BucketLayout, slot: usize) {
        let byte_index = slot / 8;
        let bit_index = slot % 8;
        if byte_index < layout.metadata_bytes {
            self.bytes[byte_index] |= 1 << bit_index;
        }
    }

    fn metadata_clear(&mut self, layout: &BucketLayout, slot: usize) {
        let byte_index = slot / 8;
        let bit_index = slot % 8;
        if byte_index < layout.metadata_bytes {
            self.bytes[byte_index] &= !(1u8 << bit_index);
        }
    }

    fn metadata_is_set(&self, layout: &BucketLayout, slot: usize) -> bool {
        let byte_index = slot / 8;
        let bit_index = slot % 8;
        byte_index < layout.metadata_bytes && (self.bytes[byte_index] & (1 << bit_index)) != 0
    }

    fn allocate_slot(&mut self, layout: &BucketLayout) -> Option<usize> {
        for i in 0..layout.capacity {
            if !self.metadata_is_set(layout, i) {
                self.metadata_set(layout, i);
                return Some(i);
            }
        }
        None
    }

    pub(crate) fn find_entries(&self, layout: &BucketLayout, _mini: usize, tag: u8) -> Vec<u16> {
        let mut results = Vec::new();
        for slot in 0..layout.capacity {
            if self.metadata_is_set(layout, slot) {
                let entry = self.entry(layout, slot);
                if entry.tag as u8 == tag || layout.fingerprint_bits == 0 {
                    results.push(entry.location);
                }
            }
        }
        results
    }

    pub(crate) fn insert_front(
        &mut self,
        layout: &BucketLayout,
        _mini: usize,
        tag: u8,
        entry: u16,
    ) -> bool {
        match self.allocate_slot(layout) {
            Some(slot) => {
                self.set_entry(layout, slot, entry, tag.into(), 0);
                true
            }
            None => false,
        }
    }

    pub(crate) fn insert_back(
        &mut self,
        layout: &BucketLayout,
        _mini: usize,
        tag: u8,
        entry: u16,
        crumb: u8,
    ) -> bool {
        let Some(slot) = self.allocate_slot(layout) else {
            return false;
        };
        self.set_entry(layout, slot, entry, tag.into(), crumb);
        true
    }

    pub(crate) fn remove_front(
        &mut self,
        layout: &BucketLayout,
        _mini: usize,
        tag: u8,
        target_entry: u16,
    ) -> bool {
        for slot in 0..layout.capacity {
            if self.metadata_is_set(layout, slot) {
                let stored = self.entry(layout, slot);
                if stored.location == target_entry
                    && (stored.tag as u8 == tag || layout.fingerprint_bits == 0)
                {
                    self.metadata_clear(layout, slot);
                    return true;
                }
            }
        }
        false
    }

    pub(crate) fn remove_back(
        &mut self,
        layout: &BucketLayout,
        _mini: usize,
        tag: u8,
        crumb: u8,
        target_entry: u16,
    ) -> bool {
        for slot in 0..layout.capacity {
            if self.metadata_is_set(layout, slot) {
                let stored = self.entry(layout, slot);
                if stored.location == target_entry
                    && (stored.tag as u8 == tag
                        || (layout.fingerprint_bits == 0 && stored.region == crumb))
                {
                    self.metadata_clear(layout, slot);
                    return true;
                }
            }
        }
        false
    }

    pub(crate) fn should_promote(&self, layout: &BucketLayout) -> bool {
        self.len(layout) < layout.capacity / 2
    }

    pub(crate) fn first_with_crumb(
        &self,
        layout: &BucketLayout,
        crumb: u8,
    ) -> Option<(usize, u16, u16)> {
        for slot in 0..layout.capacity {
            if self.metadata_is_set(layout, slot) {
                let stored = self.entry(layout, slot);
                if stored.region == crumb {
                    return Some((slot, stored.location, stored.tag));
                }
            }
        }
        None
    }

    pub(crate) fn remove_at(&mut self, layout: &BucketLayout, slot: usize) -> u16 {
        let entry = self.entry(layout, slot);
        self.metadata_clear(layout, slot);
        entry.location
    }
}
