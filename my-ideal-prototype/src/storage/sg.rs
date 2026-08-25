//! 연속된 4KiB Bucket들을 소유하고 key의 Bucket 후보를 고르는 mutable SG다.

use super::StorageKey;
use super::bucket::{BUCKET_BYTES, Bucket, BucketValue};

/// 아직 SSD에 flush되지 않아 Item을 추가하거나 교체할 수 있는 SG다.
pub(crate) struct MutableSg {
    /// SG를 이루는 연속된 4KiB Bucket 배열이다.
    buckets: Box<[Bucket]>,
    /// key 하나가 선택할 수 있는 Bucket hash 후보 개수다.
    bucket_choice_count: u8,
}

impl MutableSg {
    /// 빈 Bucket들을 하나의 고정 크기 연속 배열로 할당한다.
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

    /// 후보 Bucket 중 Item이 들어갈 수 있는 가장 덜 찬 Bucket에 추가한다.
    /// 선택한 Bucket을 다시 찾을 수 있는 hash choice 번호를 반환한다.
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

    /// hash choice가 지정한 Bucket에서 full StorageKey를 확인하고 값을 반환한다.
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

    /// hash choice가 지정한 Bucket에서 같은 key의 값을 교체한다.
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

    /// hash choice가 지정한 Bucket에서 같은 key의 Item 하나를 제거한다.
    pub(crate) fn remove(&mut self, storage_key: &StorageKey, bucket_choice: u8) -> bool {
        if bucket_choice >= self.bucket_choice_count {
            return false;
        }
        let bucket_index = self.bucket_index_for_choice(storage_key, bucket_choice);
        self.buckets[bucket_index].remove(storage_key)
    }

    /// io_uring write에 그대로 넘길 수 있는 연속된 SG byte 영역을 반환한다.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        let byte_len = self.buckets.len() * BUCKET_BYTES;
        // SAFETY: Bucket은 정확히 4KiB이고 padding 없이 연속 할당되어 있다.
        unsafe { std::slice::from_raw_parts(self.buckets.as_ptr().cast(), byte_len) }
    }

    /// 할당은 그대로 두고 모든 Bucket을 비운다.
    pub(crate) fn clear(&mut self) {
        for bucket in &mut self.buckets {
            *bucket = Bucket::new();
        }
    }

    /// StorageKey와 hash choice로 이 SG 안의 Bucket index를 계산한다.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> StorageKey {
        let mut bytes = [0; 32];
        bytes[0] = seed;
        bytes[16] = seed.wrapping_mul(17);
        bytes[24] = seed.wrapping_mul(31);
        StorageKey::new(bytes)
    }

    #[test]
    fn mutable_sg_owns_one_contiguous_region_of_buckets() {
        let sg = MutableSg::new(4, 2);
        assert_eq!(sg.as_bytes().len(), 4 * BUCKET_BYTES);
        assert_eq!(sg.as_bytes().as_ptr().align_offset(BUCKET_BYTES), 0);
    }

    #[test]
    fn bucket_choice_reads_and_mutates_the_selected_bucket() {
        let mut sg = MutableSg::new(8, 4);
        let storage_key = key(3);
        let bucket_choice = sg
            .insert(&storage_key, BucketValue::Value(b"first"))
            .unwrap();

        assert_eq!(
            sg.get(&storage_key, bucket_choice),
            Some(BucketValue::Value(b"first"))
        );
        assert!(sg.replace(
            &storage_key,
            bucket_choice,
            BucketValue::Value(b"replacement")
        ));
        assert_eq!(
            sg.get(&storage_key, bucket_choice),
            Some(BucketValue::Value(b"replacement"))
        );
        assert!(sg.remove(&storage_key, bucket_choice));
        assert_eq!(sg.get(&storage_key, bucket_choice), None);
    }

    #[test]
    fn clear_reuses_the_same_allocation() {
        let mut sg = MutableSg::new(8, 4);
        let storage_key = key(7);
        let bucket_choice = sg
            .insert(&storage_key, BucketValue::Value(b"value"))
            .unwrap();
        let allocation = sg.as_bytes().as_ptr();

        sg.clear();

        assert_eq!(sg.as_bytes().as_ptr(), allocation);
        assert_eq!(sg.get(&storage_key, bucket_choice), None);
    }
}
