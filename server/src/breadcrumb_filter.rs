//! A Rust implementation of the BCF53 configuration from
//! "Breadcrumb Filters: Fast Fully Featured Filters" (SIGMOD 2026).
//! It follows the [SALT Systems Lab reference implementation][reference]
//! (BSD-3-Clause, Copyright 2026 SALT Systems Lab).
//!
//! The public API consumes an already-computed [`HashedKey`] and turns its
//! first 64 bits directly into the `(front bucket, mini bucket, remainder)`
//! fingerprint used by the paper. It never hashes a key a second time.
//! Enabling the `force-scalar` Cargo feature disables architecture-specific
//! SIMD and bit-select dispatch so the portable path can be benchmarked.
//!
//! [reference]: https://github.com/saltsystemslab/BreadcrumbFilter

use std::fmt;
use std::mem::size_of;

use crate::HashedKey;

#[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
use std::arch::x86_64::*;
#[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
use std::arch::{aarch64::*, is_aarch64_feature_detected};

/// Number of logical quotient buckets encoded in every physical bucket.
const MINI_BUCKETS: usize = 53;
/// Number of 8-bit remainders stored in one 64-byte front bucket.
const FRONT_CAPACITY: usize = 51;
/// Number of remainder-and-crumb pairs stored in one 64-byte back bucket.
const BACK_CAPACITY: usize = 35;
/// Bytes needed for the front bucket's 53 separators and 51 key bits.
const FRONT_METADATA_BYTES: usize = 13;
/// Bytes needed for the back bucket's 53 separators and 35 key bits.
const BACK_METADATA_BYTES: usize = 11;
/// Bytes needed to pack 35 four-bit crumbs, rounded up to a whole byte.
const CRUMB_BYTES: usize = 18;
/// Number of front buckets represented by one back-bucket mapping group.
const FRONT_TO_BACK_RATIO: usize = 8;
/// Conservative number of requested items allocated per front bucket.
const SAFE_ITEMS_PER_FRONT_BUCKET: usize = 49;
/// Per-lane powers of two used to compress a NEON comparison into two bytes.
#[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
const NEON_MOVEMASK_WEIGHTS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
/// Runtime-selected implementation for comparing all bytes in a cache line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SimdBackend {
    /// One 512-bit comparison covers the complete bucket.
    #[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
    Avx512,
    /// Two 256-bit comparisons cover the complete bucket.
    #[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
    Avx2,
    /// Scalable comparison plus SVE2 bit-permutation metadata selection.
    #[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
    Sve2,
    /// Vector-length-agnostic comparison on SVE-capable AArch64 processors.
    #[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
    Sve,
    /// Four fixed-width 128-bit comparisons cover the complete bucket.
    #[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
    Neon,
    /// Portable byte-by-byte comparison used on unsupported architectures.
    Scalar,
}

impl SimdBackend {
    /// Selects the fastest comparison backend supported by the current CPU.
    fn detect() -> Self {
        #[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
        {
            if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512bw") {
                return Self::Avx512;
            }
            if is_x86_feature_detected!("avx2") {
                return Self::Avx2;
            }
        }
        #[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
        {
            if is_aarch64_feature_detected!("sve2") {
                return Self::Sve2;
            }
            if is_aarch64_feature_detected!("sve") {
                return Self::Sve;
            }
            if is_aarch64_feature_detected!("neon") {
                return Self::Neon;
            }
        }
        Self::Scalar
    }

    /// Returns a stable display name for diagnostics and benchmarks.
    fn name(self) -> &'static str {
        match self {
            #[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
            Self::Avx512 => "AVX-512BW",
            #[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
            Self::Avx2 => "AVX2",
            #[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
            Self::Sve2 => "SVE2",
            #[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
            Self::Sve => "SVE",
            #[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
            Self::Neon => "NEON",
            Self::Scalar => "scalar",
        }
    }
}

/// Returned when both candidate backyard buckets are full.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct InsertError;

impl fmt::Display for InsertError {
    /// Formats a capacity error with the required recovery action.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("breadcrumb filter is full; rebuild it with a larger capacity")
    }
}

impl std::error::Error for InsertError {}

/// A key hash split into the front bucket, logical mini-bucket, and 8-bit tag.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Fingerprint {
    /// Index of the single front bucket selected by the quotient.
    front: usize,
    /// Logical bucket within the front/back mini-filter metadata.
    mini: usize,
    /// Eight-bit approximate-membership tag stored in the bucket.
    remainder: u8,
}

/// One candidate backyard bucket and the 4-bit reverse-mapping crumb stored there.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct BackLocation {
    /// Physical backyard bucket index.
    bucket: usize,
    /// Four-bit value that identifies the originating front bucket.
    crumb: u8,
}

/// The largest front-yard fingerprint displaced by an insertion.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Overflow {
    /// Logical mini-bucket of the displaced remainder.
    mini: usize,
    /// Eight-bit remainder displaced from the front bucket.
    remainder: u8,
}

/// One physical BCF53 front-yard cache line: 13-byte unary metadata + 51 tags.
#[repr(C, align(64))]
#[derive(Clone)]
struct FrontBucket {
    /// Unary quotient metadata containing 53 one-bits and up to 51 zero-bits.
    metadata: [u8; FRONT_METADATA_BYTES],
    /// Remainders ordered by their logical mini-bucket.
    remainders: [u8; FRONT_CAPACITY],
}

impl Default for FrontBucket {
    /// Creates an empty front bucket with all 53 separators initialized.
    fn default() -> Self {
        let mut bucket = Self {
            metadata: [0; FRONT_METADATA_BYTES],
            remainders: [0; FRONT_CAPACITY],
        };
        initialize_metadata(&mut bucket.metadata);
        bucket
    }
}

impl FrontBucket {
    /// Returns the number of occupied remainder slots encoded by the metadata.
    fn len(&self) -> usize {
        metadata_len(&self.metadata)
    }

    /// Reports whether all 51 remainder slots are occupied.
    fn is_full(&self) -> bool {
        self.len() == FRONT_CAPACITY
    }

    /// Returns the half-open remainder range belonging to `mini`.
    fn bounds(&self, mini: usize) -> (usize, usize) {
        metadata_bounds(&self.metadata, mini)
    }

    /// Probes the front bucket and indicates whether a backyard probe is possible.
    fn contains(&self, mini: usize, remainder: u8, backend: SimdBackend) -> FrontQuery {
        let (start, end) = self.bounds(mini);
        let matches = byte_matches(self, remainder, backend) >> FRONT_METADATA_BYTES;
        if matches & range_mask(start, end) != 0 {
            FrontQuery::Present
        } else if end == FRONT_CAPACITY {
            FrontQuery::PotentialBackyard
        } else {
            FrontQuery::Absent
        }
    }

    /// Inserts a remainder while preserving mini-bucket order and the prefix invariant.
    ///
    /// A full bucket returns the largest displaced fingerprint for backyard insertion.
    fn insert(&mut self, mini: usize, remainder: u8) -> Option<Overflow> {
        let len = self.len();
        let location = self.bounds(mini).0;

        if location == FRONT_CAPACITY {
            return Some(Overflow { mini, remainder });
        }

        let overflow = if len == FRONT_CAPACITY {
            let overflow_mini = last_key_mini(&self.metadata, FRONT_CAPACITY);
            let overflow_remainder = self.remainders[FRONT_CAPACITY - 1];
            Some(Overflow {
                mini: overflow_mini,
                remainder: overflow_remainder,
            })
        } else {
            None
        };

        self.remainders
            .copy_within(location..len.min(FRONT_CAPACITY - 1), location + 1);
        self.remainders[location] = remainder;
        metadata_insert_key(&mut self.metadata, mini, location, FRONT_CAPACITY);
        overflow
    }

    /// Removes the first matching remainder from a logical mini-bucket.
    fn remove(&mut self, mini: usize, remainder: u8, backend: SimdBackend) -> bool {
        let (start, end) = self.bounds(mini);
        let candidates = (byte_matches(self, remainder, backend) >> FRONT_METADATA_BYTES)
            & range_mask(start, end);
        if candidates == 0 {
            return false;
        }

        let location = candidates.trailing_zeros() as usize;
        self.remove_at(mini, location);
        true
    }

    /// Removes the remainder at a known physical slot and returns its value.
    fn remove_at(&mut self, mini: usize, location: usize) -> u8 {
        let len = self.len();
        let removed = self.remainders[location];
        self.remainders.copy_within(location + 1..len, location);
        self.remainders[len - 1] = 0;
        metadata_remove_key(&mut self.metadata, mini, location);
        removed
    }
}

/// One physical backyard cache line: 11-byte metadata + 35 tags + 35 nibbles.
#[repr(C, align(64))]
#[derive(Clone)]
struct BackBucket {
    /// Unary quotient metadata containing 53 one-bits and up to 35 zero-bits.
    metadata: [u8; BACK_METADATA_BYTES],
    /// Eight-bit remainders ordered by logical mini-bucket.
    remainders: [u8; BACK_CAPACITY],
    /// Packed four-bit reverse mappings, with two crumbs per byte.
    crumbs: [u8; CRUMB_BYTES],
}

impl Default for BackBucket {
    /// Creates an empty back bucket with all 53 separators initialized.
    fn default() -> Self {
        let mut bucket = Self {
            metadata: [0; BACK_METADATA_BYTES],
            remainders: [0; BACK_CAPACITY],
            crumbs: [0; CRUMB_BYTES],
        };
        initialize_metadata(&mut bucket.metadata);
        bucket
    }
}

impl BackBucket {
    /// Returns the number of occupied remainder-and-crumb slots.
    fn len(&self) -> usize {
        metadata_len(&self.metadata)
    }

    /// Reports whether the exact mini-bucket, remainder, and crumb tuple is present.
    fn contains(&self, mini: usize, remainder: u8, crumb: u8, backend: SimdBackend) -> bool {
        self.find(mini, remainder, crumb, backend).is_some()
    }

    /// Finds the physical slot of an exact remainder-and-crumb match.
    fn find(&self, mini: usize, remainder: u8, crumb: u8, backend: SimdBackend) -> Option<usize> {
        let (start, end) = metadata_bounds(&self.metadata, mini);
        let mut candidates = (byte_matches(self, remainder, backend) >> BACK_METADATA_BYTES)
            & range_mask(start, end);
        while candidates != 0 {
            let location = candidates.trailing_zeros() as usize;
            if self.crumb(location) == crumb {
                return Some(location);
            }
            candidates &= candidates - 1;
        }
        None
    }

    /// Finds the smallest-mini-bucket entry carrying the requested front crumb.
    fn first_from_front(&self, crumb: u8) -> Option<(usize, usize, u8)> {
        let counts = metadata_counts(&self.metadata);
        for location in 0..self.len() {
            if self.crumb(location) == crumb {
                return Some((
                    mini_bucket_at(&counts, location),
                    location,
                    self.remainders[location],
                ));
            }
        }
        None
    }

    /// Inserts a remainder and crumb into their mini-bucket-ordered position.
    fn insert(&mut self, mini: usize, remainder: u8, crumb: u8) -> Result<(), InsertError> {
        let len = self.len();
        if len == BACK_CAPACITY {
            return Err(InsertError);
        }
        let location = metadata_bounds(&self.metadata, mini).0;
        for index in (location..len).rev() {
            self.remainders[index + 1] = self.remainders[index];
            let old_crumb = self.crumb(index);
            self.set_crumb(index + 1, old_crumb);
        }
        self.remainders[location] = remainder;
        self.set_crumb(location, crumb);
        metadata_insert_key(&mut self.metadata, mini, location, BACK_CAPACITY);
        Ok(())
    }

    /// Removes the first exact mini-bucket, remainder, and crumb match.
    fn remove(&mut self, mini: usize, remainder: u8, crumb: u8, backend: SimdBackend) -> bool {
        let Some(location) = self.find(mini, remainder, crumb, backend) else {
            return false;
        };
        self.remove_at(mini, location);
        true
    }

    /// Removes a known physical slot and compacts remainders and packed crumbs.
    fn remove_at(&mut self, mini: usize, location: usize) -> u8 {
        let len = self.len();
        let removed = self.remainders[location];
        for index in location..len - 1 {
            self.remainders[index] = self.remainders[index + 1];
            let next_crumb = self.crumb(index + 1);
            self.set_crumb(index, next_crumb);
        }
        self.remainders[len - 1] = 0;
        self.set_crumb(len - 1, 0);
        metadata_remove_key(&mut self.metadata, mini, location);
        removed
    }

    /// Reads the four-bit crumb stored at a physical slot.
    fn crumb(&self, location: usize) -> u8 {
        let byte = self.crumbs[location / 2];
        if location.is_multiple_of(2) {
            byte & 0x0f
        } else {
            byte >> 4
        }
    }

    /// Replaces the four-bit crumb at a physical slot without changing its neighbor.
    fn set_crumb(&mut self, location: usize, crumb: u8) {
        debug_assert!(crumb < 16);
        let byte = &mut self.crumbs[location / 2];
        if location.is_multiple_of(2) {
            *byte = (*byte & 0xf0) | crumb;
        } else {
            *byte = (*byte & 0x0f) | (crumb << 4);
        }
    }
}

/// Result of probing a front bucket, including whether a backyard probe is possible.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrontQuery {
    /// The requested fingerprint is present in the front bucket.
    Present,
    /// The prefix invariant permits the fingerprint to reside in the backyard.
    PotentialBackyard,
    /// The prefix invariant proves the fingerprint absent from both tiers.
    Absent,
}

/// Complete BCF53 table: 8-bit tags, 53 mini-buckets, and front/back bucket arrays.
pub struct BreadcrumbFilter {
    /// Large single-choice tier containing almost all fingerprints.
    front: Vec<FrontBucket>,
    /// Small two-choice tier containing front-bucket overflows.
    back: Vec<BackBucket>,
    /// CRUMB mapping stride used to derive the second backyard candidate.
    back_stride: usize,
    /// Number of successfully inserted fingerprints.
    len: usize,
    /// Runtime-selected cache-line comparison implementation.
    backend: SimdBackend,
}

impl BreadcrumbFilter {
    /// Allocates a filter sized for approximately `max_items` entries at a
    /// conservative load below the paper's measured failure threshold.
    ///
    /// # Panics
    ///
    /// Panics when `max_items` is zero.
    pub fn with_capacity(max_items: usize) -> Self {
        assert!(max_items > 0, "capacity must be non-zero");
        let front_count = max_items.div_ceil(SAFE_ITEMS_PER_FRONT_BUCKET).max(1);
        let back_count = front_count.div_ceil(FRONT_TO_BACK_RATIO) + FRONT_TO_BACK_RATIO * 2;
        let mut back_stride = front_count / 64 + 1;
        if back_stride.is_multiple_of(FRONT_TO_BACK_RATIO - 1) {
            back_stride += 1;
        }

        let filter = Self {
            front: vec![FrontBucket::default(); front_count],
            back: vec![BackBucket::default(); back_count],
            back_stride,
            len: 0,
            backend: SimdBackend::detect(),
        };
        debug_assert_eq!(size_of::<FrontBucket>(), 64);
        debug_assert_eq!(size_of::<BackBucket>(), 64);
        filter
    }

    /// Inserts an already-computed 32-byte hashed key into the filter.
    ///
    /// Rebuild the filter with a larger capacity if both backyard candidates
    /// are full and this method returns [`InsertError`].
    pub fn insert(&mut self, hashed_key: &HashedKey) -> Result<(), InsertError> {
        self.insert_fingerprint(self.fingerprint(hashed_key))
    }

    /// Returns `true` when the hashed key may be present and `false` when absent.
    ///
    /// A `true` result is probabilistic and can be a false positive.
    pub fn contains(&self, hashed_key: &HashedKey) -> bool {
        self.contains_fingerprint(self.fingerprint(hashed_key))
    }

    /// Deletion is safe for keys known to have been inserted. As with other
    /// fingerprint filters, deleting an arbitrary false positive can remove a
    /// colliding fingerprint.
    pub fn remove(&mut self, hashed_key: &HashedKey) -> bool {
        let fingerprint = self.fingerprint(hashed_key);
        let was_full = self.front[fingerprint.front].is_full();
        if self.front[fingerprint.front].remove(
            fingerprint.mini,
            fingerprint.remainder,
            self.backend,
        ) {
            if was_full {
                self.promote_from_backyard(fingerprint.front);
            }
            self.len -= 1;
            return true;
        }
        if !was_full {
            return false;
        }

        let [first, second] = self.back_locations(fingerprint.front);
        if self.back[first.bucket].remove(
            fingerprint.mini,
            fingerprint.remainder,
            first.crumb,
            self.backend,
        ) || self.back[second.bucket].remove(
            fingerprint.mini,
            fingerprint.remainder,
            second.crumb,
            self.backend,
        ) {
            self.len -= 1;
            true
        } else {
            false
        }
    }

    /// Returns the number of fingerprints currently stored in the filter.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Reports whether the filter contains no fingerprints.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns heap bytes used by front and back bucket arrays.
    pub fn memory_bytes(&self) -> usize {
        self.front.len() * size_of::<FrontBucket>() + self.back.len() * size_of::<BackBucket>()
    }

    /// Returns the runtime-selected byte-comparison backend name.
    pub fn simd_backend(&self) -> &'static str {
        self.backend.name()
    }

    /// Inserts an already-derived fingerprint and handles front overflow transactionally.
    fn insert_fingerprint(&mut self, fingerprint: Fingerprint) -> Result<(), InsertError> {
        let overflow =
            self.front[fingerprint.front].insert(fingerprint.mini, fingerprint.remainder);
        let Some(overflow) = overflow else {
            self.len += 1;
            return Ok(());
        };

        let [first, second] = self.back_locations(fingerprint.front);
        let first_len = self.back[first.bucket].len();
        let second_len = self.back[second.bucket].len();
        let destination = if first_len < second_len {
            first
        } else {
            second
        };

        if self.back[destination.bucket]
            .insert(overflow.mini, overflow.remainder, destination.crumb)
            .is_err()
        {
            // Restore the front bucket if insertion displaced an old entry.
            if overflow.mini != fingerprint.mini || overflow.remainder != fingerprint.remainder {
                let removed = self.front[fingerprint.front].remove(
                    fingerprint.mini,
                    fingerprint.remainder,
                    self.backend,
                );
                debug_assert!(removed);
                let restored =
                    self.front[fingerprint.front].insert(overflow.mini, overflow.remainder);
                debug_assert!(restored.is_none());
            }
            return Err(InsertError);
        }
        self.len += 1;
        Ok(())
    }

    /// Probes an already-derived fingerprint in the front and, if needed, backyard.
    fn contains_fingerprint(&self, fingerprint: Fingerprint) -> bool {
        match self.front[fingerprint.front].contains(
            fingerprint.mini,
            fingerprint.remainder,
            self.backend,
        ) {
            FrontQuery::Present => true,
            FrontQuery::Absent => false,
            FrontQuery::PotentialBackyard => {
                let [first, second] = self.back_locations(fingerprint.front);
                self.back[first.bucket].contains(
                    fingerprint.mini,
                    fingerprint.remainder,
                    first.crumb,
                    self.backend,
                ) || self.back[second.bucket].contains(
                    fingerprint.mini,
                    fingerprint.remainder,
                    second.crumb,
                    self.backend,
                )
            }
        }
    }

    /// Promotes the smallest eligible backyard entry after a front deletion.
    fn promote_from_backyard(&mut self, front_index: usize) {
        let [first, second] = self.back_locations(front_index);
        let first_candidate = self.back[first.bucket].first_from_front(first.crumb);
        let second_candidate = self.back[second.bucket].first_from_front(second.crumb);

        let choice = match (first_candidate, second_candidate) {
            (None, None) => return,
            (Some(candidate), None) => (first, candidate),
            (None, Some(candidate)) => (second, candidate),
            (Some(a), Some(b)) if a.0 < b.0 => (first, a),
            (Some(_), Some(b)) => (second, b),
        };
        let (location, (mini, slot, remainder)) = choice;
        self.back[location.bucket].remove_at(mini, slot);
        let overflow = self.front[front_index].insert(mini, remainder);
        debug_assert!(overflow.is_none());
    }

    /// Maps the first 64 hash bits directly into a BCF53 fingerprint.
    fn fingerprint(&self, hashed_key: &HashedKey) -> Fingerprint {
        let quotient_count = self.front.len() * MINI_BUCKETS;
        let fingerprint_space = quotient_count as u64 * 256;
        let bytes = hashed_key.as_bytes();
        let hash_prefix = u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]);
        let fingerprint = hash_prefix % fingerprint_space;
        let quotient = (fingerprint >> 8) as usize;
        Fingerprint {
            front: quotient / MINI_BUCKETS,
            mini: quotient % MINI_BUCKETS,
            remainder: fingerprint as u8,
        }
    }

    /// Derives the two CRUMB backyard candidates for a front bucket.
    fn back_locations(&self, front: usize) -> [BackLocation; 2] {
        let upper = front / FRONT_TO_BACK_RATIO;
        let low = front % FRONT_TO_BACK_RATIO;
        let first = BackLocation {
            bucket: front / FRONT_TO_BACK_RATIO,
            crumb: (low + FRONT_TO_BACK_RATIO) as u8,
        };
        let second = BackLocation {
            bucket: upper / FRONT_TO_BACK_RATIO + low * self.back_stride,
            crumb: (upper % FRONT_TO_BACK_RATIO) as u8,
        };
        debug_assert!(first.bucket < self.back.len());
        debug_assert!(second.bucket < self.back.len());
        [first, second]
    }
}

/// Initializes unary metadata with 53 contiguous separator bits and no keys.
fn initialize_metadata<const N: usize>(metadata: &mut [u8; N]) {
    let initial = (1u128 << MINI_BUCKETS) - 1;
    store_metadata(initial, metadata);
}

/// Loads a little-endian metadata byte array into a `u128` for bit operations.
fn metadata_value<const N: usize>(metadata: &[u8; N]) -> u128 {
    let mut bytes = [0u8; 16];
    bytes[..N].copy_from_slice(metadata);
    u128::from_le_bytes(bytes)
}

/// Stores the low `N` little-endian bytes of a metadata word.
fn store_metadata<const N: usize>(value: u128, metadata: &mut [u8; N]) {
    metadata.copy_from_slice(&value.to_le_bytes()[..N]);
}

/// Derives the number of zero-bit key entries from the final separator position.
fn metadata_len<const N: usize>(metadata: &[u8; N]) -> usize {
    let bits = metadata_value(metadata);
    (128 - bits.leading_zeros() as usize) - MINI_BUCKETS
}

/// Converts a logical mini-bucket into its half-open physical slot range.
fn metadata_bounds<const N: usize>(metadata: &[u8; N], mini: usize) -> (usize, usize) {
    debug_assert!(mini < MINI_BUCKETS);
    let bits = metadata_value(metadata);
    let end = select_one(bits, mini) - mini;
    let start = if mini == 0 {
        0
    } else {
        select_one(bits, mini - 1) - (mini - 1)
    };
    (start, end)
}

/// Decodes unary metadata into one key count per logical mini-bucket.
fn metadata_counts<const N: usize>(metadata: &[u8; N]) -> [usize; MINI_BUCKETS] {
    let mut counts = [0usize; MINI_BUCKETS];
    let mut separators = metadata_value(metadata);
    let mut previous_separator = None;
    for count in &mut counts {
        let separator = separators.trailing_zeros() as usize;
        *count = match previous_separator {
            Some(previous) => separator - previous - 1,
            None => separator,
        };
        previous_separator = Some(separator);
        separators &= separators - 1;
    }
    counts
}

/// Inserts one zero-bit key into unary metadata, evicting the largest key if full.
fn metadata_insert_key<const N: usize>(
    metadata: &mut [u8; N],
    mini: usize,
    location: usize,
    capacity: usize,
) {
    let bits = metadata_value(metadata);
    let full = metadata_len(metadata) == capacity;
    let bit_index = mini + location;
    let lower_mask = (1u128 << bit_index) - 1;
    let total_bits = MINI_BUCKETS + capacity;
    let active_mask = (1u128 << total_bits) - 1;
    let mut shifted = ((bits & lower_mask) | ((bits & !lower_mask) << 1)) & active_mask;

    if full {
        // The shift dropped the final separator. Turning the highest zero
        // into a separator evicts the largest fingerprint from the bucket.
        let zeros = !shifted & active_mask;
        let last_zero = 127 - zeros.leading_zeros() as usize;
        shifted |= 1u128 << last_zero;
    }
    store_metadata(shifted, metadata);
}

/// Removes one zero-bit key from unary metadata and closes the resulting gap.
fn metadata_remove_key<const N: usize>(metadata: &mut [u8; N], mini: usize, location: usize) {
    let bits = metadata_value(metadata);
    let bit_index = mini + location;
    debug_assert_eq!((bits >> bit_index) & 1, 0);
    let lower_mask = (1u128 << bit_index) - 1;
    let shifted = (bits & lower_mask) | ((bits >> 1) & !lower_mask);
    store_metadata(shifted, metadata);
}

/// Returns the logical mini-bucket of the largest key in a full bucket.
fn last_key_mini<const N: usize>(metadata: &[u8; N], capacity: usize) -> usize {
    let total_bits = MINI_BUCKETS + capacity;
    let active_mask = (1u128 << total_bits) - 1;
    let key_bits = !metadata_value(metadata) & active_mask;
    let last_key_position = 127 - key_bits.leading_zeros() as usize;
    last_key_position - (capacity - 1)
}

/// Maps a physical key slot to its logical mini-bucket using decoded counts.
fn mini_bucket_at(counts: &[usize; MINI_BUCKETS], location: usize) -> usize {
    let mut offset = 0usize;
    for (mini, count) in counts.iter().copied().enumerate() {
        if location < offset + count {
            return mini;
        }
        offset += count;
    }
    MINI_BUCKETS
}

/// Selects the bit position of the zero-based `rank`-th set bit.
///
/// BMI2 or SVE2 BitPerm is selected at runtime when available; other CPUs use
/// the scalar path.
fn select_one(bits: u128, rank: usize) -> usize {
    #[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
    if is_x86_feature_detected!("bmi2") {
        return unsafe { select_one_bmi2(bits, rank) };
    }

    #[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
    if is_aarch64_feature_detected!("sve2-bitperm") {
        return unsafe { select_one_sve2_bitperm(bits, rank) };
    }

    select_one_scalar(bits, rank)
}

/// Selects a set bit by repeatedly clearing lower set bits on portable targets.
fn select_one_scalar(mut bits: u128, rank: usize) -> usize {
    for _ in 0..rank {
        bits &= bits - 1;
    }
    bits.trailing_zeros() as usize
}

#[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
#[target_feature(enable = "bmi2")]
/// Selects a set bit with BMI2 `PDEP` across the low and high 64-bit halves.
///
/// # Safety
///
/// The caller must verify BMI2 support before entering this function.
unsafe fn select_one_bmi2(bits: u128, rank: usize) -> usize {
    let low = bits as u64;
    let low_count = low.count_ones() as usize;
    if rank < low_count {
        _pdep_u64(1u64 << rank, low).trailing_zeros() as usize
    } else {
        let high = (bits >> 64) as u64;
        64 + _pdep_u64(1u64 << (rank - low_count), high).trailing_zeros() as usize
    }
}

#[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
#[target_feature(enable = "sve,sve2,sve2-bitperm")]
/// Selects a set bit with SVE2 BitPerm `BDEP` on duplicated 64-bit lanes.
///
/// # Safety
///
/// The caller must verify SVE2 BitPerm support before entering this function.
unsafe fn select_one_sve2_bitperm(bits: u128, rank: usize) -> usize {
    let low = bits as u64;
    let low_count = low.count_ones() as usize;
    let (word, word_rank, offset) = if rank < low_count {
        (low, rank, 0)
    } else {
        ((bits >> 64) as u64, rank - low_count, 64)
    };
    let source = svdup_n_u64(1u64 << word_rank);
    let selected = svbdep_n_u64(source, word);
    offset + svlastb_u64(svptrue_b64(), selected).trailing_zeros() as usize
}

/// Builds a `u64` mask with bits in the half-open range `[start, end)` set.
fn range_mask(start: usize, end: usize) -> u64 {
    if start == end {
        0
    } else {
        ((1u64 << end) - 1) ^ ((1u64 << start) - 1)
    }
}

/// Compares a byte against all 64 bytes of an aligned bucket.
fn byte_matches<T>(bucket: &T, needle: u8, backend: SimdBackend) -> u64 {
    let pointer = bucket as *const T as *const u8;
    debug_assert_eq!((pointer as usize) & 63, 0);
    match backend {
        #[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
        SimdBackend::Avx512 => unsafe { byte_matches_avx512(pointer, needle) },
        #[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
        SimdBackend::Avx2 => unsafe { byte_matches_avx2(pointer, needle) },
        #[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
        SimdBackend::Sve2 | SimdBackend::Sve => unsafe { byte_matches_sve(pointer, needle) },
        #[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
        SimdBackend::Neon => unsafe { byte_matches_neon(pointer, needle) },
        _ => unsafe { byte_matches_scalar(pointer, needle) },
    }
}

/// Produces one match bit per cache-line byte without architecture intrinsics.
///
/// # Safety
///
/// `pointer` must be valid to read exactly 64 bytes.
unsafe fn byte_matches_scalar(pointer: *const u8, needle: u8) -> u64 {
    // SAFETY: caller guarantees pointer is valid for 64 bytes
    let bytes = unsafe { std::slice::from_raw_parts(pointer, 64) };
    bytes.iter().enumerate().fold(0u64, |mask, (index, byte)| {
        mask | (u64::from(*byte == needle) << index)
    })
}

#[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
#[target_feature(enable = "neon")]
/// Produces a 64-bit byte-match mask with four NEON comparisons.
///
/// # Safety
///
/// The caller must verify NEON support, and `pointer` must reference a
/// 64-byte-aligned readable cache line.
unsafe fn byte_matches_neon(pointer: *const u8, needle: u8) -> u64 {
    let target = vdupq_n_u8(needle);
    // SAFETY: caller guarantees NEON support and valid pointer
    let weights = unsafe { vld1q_u8(NEON_MOVEMASK_WEIGHTS.as_ptr()) };
    let mut mask = 0u64;

    for chunk in 0..4 {
        let bytes = unsafe { vld1q_u8(pointer.add(chunk * 16)) };
        let matches = vceqq_u8(bytes, target);
        let weighted = vmulq_u8(vshrq_n_u8::<7>(matches), weights);
        let low = vaddv_u8(vget_low_u8(weighted));
        let high = vaddv_u8(vget_high_u8(weighted));
        let chunk_mask = u64::from(low) | (u64::from(high) << 8);
        mask |= chunk_mask << (chunk * 16);
    }

    mask
}

#[cfg(all(target_arch = "aarch64", not(feature = "force-scalar")))]
#[target_feature(enable = "sve")]
/// Produces a 64-bit byte-match mask with vector-length-agnostic SVE predicates.
///
/// Sparse matches are enumerated directly from each comparison predicate, so
/// no fixed SVE vector length or temporary byte array is required.
///
/// # Safety
///
/// The caller must verify SVE support, and `pointer` must reference a
/// 64-byte-aligned readable cache line.
unsafe fn byte_matches_sve(pointer: *const u8, needle: u8) -> u64 {
    let vector_bytes = svcntb();
    let mut offset = 0u64;
    let mut mask = 0u64;

    while offset < 64 {
        let active = svwhilelt_b8_u64(offset, 64);
        let bytes = unsafe { svld1_u8(active, pointer.add(offset as usize)) };
        let mut matches = svcmpeq_n_u8(active, bytes, needle);

        while svptest_any(active, matches) {
            let before = svbrkb_b_z(active, matches);
            let index = svcntp_b8(active, before);
            mask |= 1u64 << (offset + index);
            let through_first = svbrka_b_z(active, matches);
            matches = svbic_b_z(active, matches, through_first);
        }

        offset += vector_bytes;
    }

    mask
}

#[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
#[target_feature(enable = "avx2")]
/// Produces a 64-bit byte-match mask with two aligned AVX2 loads.
///
/// # Safety
///
/// The caller must verify AVX2 support, and `pointer` must reference a
/// 64-byte-aligned readable cache line.
unsafe fn byte_matches_avx2(pointer: *const u8, needle: u8) -> u64 {
    let target = _mm256_set1_epi8(needle as i8);
    let low = unsafe { _mm256_load_si256(pointer.cast()) };
    let high = unsafe { _mm256_load_si256(pointer.add(32).cast()) };
    let low_mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(low, target)) as u32 as u64;
    let high_mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(high, target)) as u32 as u64;
    low_mask | (high_mask << 32)
}

#[cfg(all(target_arch = "x86_64", not(feature = "force-scalar")))]
#[target_feature(enable = "avx512f,avx512bw")]
/// Produces a 64-bit byte-match mask with one aligned AVX-512 load.
///
/// # Safety
///
/// The caller must verify AVX-512F and AVX-512BW support, and `pointer` must
/// reference a 64-byte-aligned readable cache line.
unsafe fn byte_matches_avx512(pointer: *const u8, needle: u8) -> u64 {
    let block = unsafe { _mm512_load_si512(pointer.cast()) };
    let target = _mm512_set1_epi8(needle as i8);
    _mm512_cmpeq_epi8_mask(block, target)
}
