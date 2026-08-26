//! A mutable SG that owns contiguous 4 KiB Buckets and selects Bucket candidates for keys.

use super::StorageKey;
use super::bucket::{BUCKET_BYTES, Bucket, BucketValue};

/// An SG that has not yet been flushed to SSD, so Items can still be added or replaced.
pub(crate) struct MutableSg {
    /// The contiguous array of 4 KiB Buckets that forms the SG.
    buckets: Box<[Bucket]>,
    /// Number of Bucket hash candidates available to each key.
    bucket_choice_count: u8,
}

impl MutableSg {
    /// Allocates empty Buckets as one fixed-size contiguous array.
    pub(crate) fn new(bucket_count: usize, bucket_choice_count: u8) -> Self {
        assert!(bucket_count > 0, "an SG must contain at least one Bucket");
        assert!(
            (1..=32).contains(&bucket_choice_count),
            "Bucket choice count must be between 1 and 32"
        );

        let buckets = (0..bucket_count)
            .map(|_| Bucket::new())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            buckets,
            bucket_choice_count,
        }
    }

    /// Inserts an Item into the least-filled candidate Bucket with sufficient space.
    /// Returns the hash choice used to locate the selected Bucket again.
    pub(crate) fn insert(
        &mut self,
        storage_key: &StorageKey,
        value: BucketValue<'_>,
    ) -> Option<u8> {
        let mut selected = None;

        for bucket_choice in 0..self.bucket_choice_count {
            let bucket_index = self.bucket_index_for_choice(storage_key, bucket_choice);
            let bucket = &self.buckets[bucket_index];
            if !bucket.can_append(value) {
                continue;
            }

            let used_bytes = bucket.used_bytes();
            if selected.is_none_or(|(_, _, selected_used_bytes)| used_bytes < selected_used_bytes) {
                selected = Some((bucket_index, bucket_choice, used_bytes));
            }
        }

        let (bucket_index, bucket_choice, _) = selected?;
        let inserted = self.buckets[bucket_index].append(storage_key, value);
        debug_assert!(inserted);
        Some(bucket_choice)
    }

    /// Verifies the full StorageKey in the Bucket selected by the hash choice and returns its value.
    pub(crate) fn get(
        &self,
        storage_key: &StorageKey,
        bucket_choice: u8,
    ) -> Option<BucketValue<'_>> {
        if bucket_choice >= self.bucket_choice_count {
            return None;
        }
        let bucket_index = self.bucket_index_for_choice(storage_key, bucket_choice);
        self.buckets[bucket_index].get(storage_key)
    }

    /// Replaces a value with the same key in the Bucket selected by the hash choice.
    pub(crate) fn replace(
        &mut self,
        storage_key: &StorageKey,
        bucket_choice: u8,
        replacement: BucketValue<'_>,
    ) -> bool {
        if bucket_choice >= self.bucket_choice_count {
            return false;
        }
        let bucket_index = self.bucket_index_for_choice(storage_key, bucket_choice);
        self.buckets[bucket_index].replace(storage_key, replacement)
    }

    /// Removes one Item with the same key from the Bucket selected by the hash choice.
    pub(crate) fn remove(&mut self, storage_key: &StorageKey, bucket_choice: u8) -> bool {
        if bucket_choice >= self.bucket_choice_count {
            return false;
        }
        let bucket_index = self.bucket_index_for_choice(storage_key, bucket_choice);
        self.buckets[bucket_index].remove(storage_key)
    }

    /// Returns the contiguous SG byte region that can be passed directly to an io_uring write.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        let byte_len = self.buckets.len() * BUCKET_BYTES;
        // SAFETY: Each Bucket is exactly 4 KiB and the Buckets are allocated
        // contiguously without padding.
        unsafe { std::slice::from_raw_parts(self.buckets.as_ptr().cast(), byte_len) }
    }

    /// Clears every Bucket while retaining the allocation.
    pub(crate) fn clear(&mut self) {
        for bucket in &mut self.buckets {
            *bucket = Bucket::new();
        }
    }

    /// Computes a Bucket index within this SG from a StorageKey and hash choice.
    pub(crate) fn bucket_index_for_choice(
        &self,
        storage_key: &StorageKey,
        bucket_choice: u8,
    ) -> usize {
        let key = storage_key.as_bytes();
        let first = u64::from_le_bytes(key[16..24].try_into().unwrap());
        let second = u64::from_le_bytes(key[24..32].try_into().unwrap());
        let hash = match bucket_choice {
            0 => first,
            1 => second,
            choice => {
                first.wrapping_add(u64::from(choice).wrapping_mul(second.rotate_left(32) | 1))
            }
        };
        hash as usize % self.buckets.len()
    }
}
