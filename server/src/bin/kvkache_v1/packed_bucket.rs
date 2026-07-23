// Bit-packed 64-byte bucket primitives shared by the front and back yards.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Location {
    pub(crate) region: u8,
    pub(crate) page_choice: u8,
}

impl Location {
    fn encode(self, region_bits: usize) -> u16 {
        debug_assert!((self.region as usize) < (1usize << region_bits));
        ((self.region as u16) << 1) | self.page_choice as u16
    }

    fn decode(value: u16) -> Self {
        Self {
            region: (value >> 1) as u8,
            page_choice: (value & 1) as u8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Fingerprint {
    front: usize,
    mini: usize,
    remainder: u16,
}

#[derive(Clone, Copy, Debug)]
struct BackLocation {
    bucket: usize,
    crumb: u8,
}

#[derive(Clone, Copy, Debug)]
struct PackedEntry {
    mini: usize,
    remainder: u16,
    location: u16,
    crumb: u8,
}

#[derive(Clone, Debug)]
struct BucketLayout {
    mini_buckets: usize,
    capacity: usize,
    remainder_bits: usize,
    location_bits: usize,
    crumb_bits: usize,
    metadata_bytes: usize,
    remainder_bit: usize,
    location_bit: usize,
    crumb_bit: usize,
}

impl BucketLayout {
    fn new(
        mini_buckets: usize,
        remainder_bits: usize,
        location_bits: usize,
        crumb_bits: usize,
    ) -> Result<Self> {
        let mut chosen = None;
        for capacity in 1..=64 {
            let metadata_bits = mini_buckets + capacity;
            if metadata_bits > 128 {
                break;
            }
            let metadata_bytes = metadata_bits.div_ceil(8);
            let remainder_bytes = (capacity * remainder_bits).div_ceil(8);
            let location_bytes = (capacity * location_bits).div_ceil(8);
            let crumb_bytes = (capacity * crumb_bits).div_ceil(8);
            if metadata_bytes + remainder_bytes + location_bytes + crumb_bytes <= BUCKET_BYTES {
                chosen = Some((capacity, metadata_bytes, remainder_bytes, location_bytes));
            }
        }
        let Some((capacity, metadata_bytes, remainder_bytes, location_bytes)) = chosen else {
            return Err(KvError::InvalidConfig(
                "fingerprint/location/mini-bucket fields do not fit in 64-byte buckets".into(),
            ));
        };
        let remainder_bit = metadata_bytes * 8;
        let location_bit = remainder_bit + remainder_bytes * 8;
        let crumb_bit = location_bit + location_bytes * 8;
        Ok(Self {
            mini_buckets,
            capacity,
            remainder_bits,
            location_bits,
            crumb_bits,
            metadata_bytes,
            remainder_bit,
            location_bit,
            crumb_bit,
        })
    }
}

#[repr(C, align(64))]
#[derive(Clone)]
struct PackedBucket {
    bytes: [u8; BUCKET_BYTES],
}

impl PackedBucket {
    fn new(layout: &BucketLayout) -> Self {
        let mut bucket = Self {
            bytes: [0; BUCKET_BYTES],
        };
        let separators = (1u128 << layout.mini_buckets) - 1;
        bucket.store_metadata(layout, separators);
        bucket
    }

    fn metadata(&self, layout: &BucketLayout) -> u128 {
        let mut bytes = [0u8; 16];
        bytes[..layout.metadata_bytes].copy_from_slice(&self.bytes[..layout.metadata_bytes]);
        u128::from_le_bytes(bytes)
    }

    fn store_metadata(&mut self, layout: &BucketLayout, value: u128) {
        self.bytes[..layout.metadata_bytes]
            .copy_from_slice(&value.to_le_bytes()[..layout.metadata_bytes]);
    }

    fn len(&self, layout: &BucketLayout) -> usize {
        let bits = self.metadata(layout);
        (128 - bits.leading_zeros() as usize) - layout.mini_buckets
    }

    fn bounds(&self, layout: &BucketLayout, mini: usize) -> (usize, usize) {
        let bits = self.metadata(layout);
        let end = select_one(bits, mini) - mini;
        let start = if mini == 0 {
            0
        } else {
            select_one(bits, mini - 1) - (mini - 1)
        };
        (start, end)
    }

    fn entry(&self, layout: &BucketLayout, slot: usize) -> PackedEntry {
        PackedEntry {
            mini: self.mini_at(layout, slot),
            remainder: get_bits(
                &self.bytes,
                layout.remainder_bit + slot * layout.remainder_bits,
                layout.remainder_bits,
            ) as u16,
            location: get_bits(
                &self.bytes,
                layout.location_bit + slot * layout.location_bits,
                layout.location_bits,
            ) as u16,
            crumb: if layout.crumb_bits == 0 {
                0
            } else {
                get_bits(
                    &self.bytes,
                    layout.crumb_bit + slot * layout.crumb_bits,
                    layout.crumb_bits,
                ) as u8
            },
        }
    }

    fn mini_at(&self, layout: &BucketLayout, slot: usize) -> usize {
        let bits = self.metadata(layout);
        for mini in 0..layout.mini_buckets {
            let (_, end) = metadata_bounds(bits, mini);
            if slot < end {
                return mini;
            }
        }
        layout.mini_buckets - 1
    }

    fn write_entry(&mut self, layout: &BucketLayout, slot: usize, entry: PackedEntry) {
        set_bits(
            &mut self.bytes,
            layout.remainder_bit + slot * layout.remainder_bits,
            layout.remainder_bits,
            entry.remainder as u64,
        );
        set_bits(
            &mut self.bytes,
            layout.location_bit + slot * layout.location_bits,
            layout.location_bits,
            entry.location as u64,
        );
        if layout.crumb_bits > 0 {
            set_bits(
                &mut self.bytes,
                layout.crumb_bit + slot * layout.crumb_bits,
                layout.crumb_bits,
                entry.crumb as u64,
            );
        }
    }

    fn clear_entry(&mut self, layout: &BucketLayout, slot: usize) {
        self.write_entry(
            layout,
            slot,
            PackedEntry {
                mini: 0,
                remainder: 0,
                location: 0,
                crumb: 0,
            },
        );
    }

    /// Front insertion retains the smallest mini-buckets and returns one overflow.
    fn insert_front(&mut self, layout: &BucketLayout, entry: PackedEntry) -> Option<PackedEntry> {
        let len = self.len(layout);
        let location = self.bounds(layout, entry.mini).0;
        if location == layout.capacity {
            return Some(entry);
        }
        let overflow = (len == layout.capacity).then(|| self.entry(layout, layout.capacity - 1));
        let shift_end = len.min(layout.capacity - 1);
        for slot in (location..shift_end).rev() {
            let moved = self.entry(layout, slot);
            self.write_entry(layout, slot + 1, moved);
        }
        self.write_entry(layout, location, entry);
        self.metadata_insert(layout, entry.mini, location);
        overflow
    }

    fn insert_back(&mut self, layout: &BucketLayout, entry: PackedEntry) -> bool {
        let len = self.len(layout);
        if len == layout.capacity {
            return false;
        }
        let location = self.bounds(layout, entry.mini).0;
        for slot in (location..len).rev() {
            let moved = self.entry(layout, slot);
            self.write_entry(layout, slot + 1, moved);
        }
        self.write_entry(layout, location, entry);
        self.metadata_insert(layout, entry.mini, location);
        true
    }

    fn matching_slots(
        &self,
        layout: &BucketLayout,
        mini: usize,
        remainder: u16,
        crumb: Option<u8>,
    ) -> Vec<usize> {
        let (start, end) = self.bounds(layout, mini);
        (start..end)
            .filter(|slot| {
                let entry = self.entry(layout, *slot);
                entry.remainder == remainder && crumb.is_none_or(|crumb| entry.crumb == crumb)
            })
            .collect()
    }

    fn first_with_crumb(&self, layout: &BucketLayout, crumb: u8) -> Option<(usize, PackedEntry)> {
        (0..self.len(layout)).find_map(|slot| {
            let entry = self.entry(layout, slot);
            (entry.crumb == crumb).then_some((slot, entry))
        })
    }

    fn remove_at(&mut self, layout: &BucketLayout, mini: usize, slot: usize) -> PackedEntry {
        let len = self.len(layout);
        let removed = self.entry(layout, slot);
        for index in slot..len - 1 {
            let moved = self.entry(layout, index + 1);
            self.write_entry(layout, index, moved);
        }
        self.clear_entry(layout, len - 1);
        self.metadata_remove(layout, mini, slot);
        removed
    }

    fn metadata_insert(&mut self, layout: &BucketLayout, mini: usize, location: usize) {
        let bits = self.metadata(layout);
        let full = self.len(layout) == layout.capacity;
        let bit_index = mini + location;
        let lower_mask = (1u128 << bit_index) - 1;
        let total_bits = layout.mini_buckets + layout.capacity;
        let active_mask = low_mask(total_bits);
        let mut shifted = ((bits & lower_mask) | ((bits & !lower_mask) << 1)) & active_mask;
        if full {
            let zeros = !shifted & active_mask;
            let last_zero = 127 - zeros.leading_zeros() as usize;
            shifted |= 1u128 << last_zero;
        }
        self.store_metadata(layout, shifted);
    }

    fn metadata_remove(&mut self, layout: &BucketLayout, mini: usize, location: usize) {
        let bits = self.metadata(layout);
        let bit_index = mini + location;
        let lower_mask = (1u128 << bit_index) - 1;
        let lower = bits & lower_mask;
        let upper = (bits >> (bit_index + 1)) << bit_index;
        self.store_metadata(layout, lower | upper);
    }
}

fn metadata_bounds(bits: u128, mini: usize) -> (usize, usize) {
    let end = select_one(bits, mini) - mini;
    let start = if mini == 0 {
        0
    } else {
        select_one(bits, mini - 1) - (mini - 1)
    };
    (start, end)
}

fn select_one(mut bits: u128, rank: usize) -> usize {
    for _ in 0..rank {
        bits &= bits - 1;
    }
    bits.trailing_zeros() as usize
}

fn low_mask(bits: usize) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

fn get_bits(bytes: &[u8], bit: usize, width: usize) -> u64 {
    let mut value = 0u64;
    for offset in 0..width {
        let source = bit + offset;
        value |= (((bytes[source / 8] >> (source % 8)) & 1) as u64) << offset;
    }
    value
}

fn set_bits(bytes: &mut [u8], bit: usize, width: usize, value: u64) {
    for offset in 0..width {
        let target = bit + offset;
        let mask = 1u8 << (target % 8);
        if value & (1u64 << offset) == 0 {
            bytes[target / 8] &= !mask;
        } else {
            bytes[target / 8] |= mask;
        }
    }
}
