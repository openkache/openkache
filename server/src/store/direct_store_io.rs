//! Direct-I/O buffer preparation and generation write helpers.

use crate::storage_runtime::File;
use crate::{BUCKET_BYTES, Config, KvError, Result, StorageKey};
use futures_util::future::FutureExt;

use super::{
    BucketHashSequence, DirectIoBuffer, EvictionWork, GenerationLocation, LargeValueLocation,
    read_exact_direct, write_all_direct,
};

pub(super) fn direct_buffer_from_bytes(bytes: &[u8]) -> Result<Option<DirectIoBuffer>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let len = bytes
        .len()
        .checked_next_multiple_of(BUCKET_BYTES)
        .ok_or_else(|| KvError::Usage("Blob write padding overflowed".into()))?;
    let mut buffer = DirectIoBuffer::zeroed(len);
    buffer[..bytes.len()].copy_from_slice(bytes);
    Ok(Some(buffer))
}

pub(super) fn schedule_eviction_read(data: &File, config: &Config, eviction: &mut EvictionWork) {
    const EXTENT_BYTES: usize = 1024 * 1024;
    if eviction.read.is_some()
        || eviction.prefetched.is_some()
        || eviction.next_read_offset >= config.segment_size
    {
        return;
    }
    let offset = eviction.next_read_offset;
    let len = EXTENT_BYTES.min(config.segment_size - offset);
    eviction.next_read_offset += len;
    let file = data.clone();
    let file_offset = eviction.victim.sg_base + offset as u64;
    let read_max_time_us = config.read_max_time_us.max(config.write_max_time_us);
    eviction.read = Some(
        async move {
            let result = read_exact_direct(
                &file,
                DirectIoBuffer::for_read(len),
                file_offset,
                len,
                read_max_time_us,
                "eviction SG extent read",
            )
            .await;
            (offset, result)
        }
        .boxed_local(),
    );
}

pub(super) async fn write_generation(
    data: File,
    large_values: File,
    config: Config,
    location: GenerationLocation,
    large_value_location: Option<LargeValueLocation>,
    blob_write: Option<DirectIoBuffer>,
    blob_physical_len: usize,
    large_value_write: Option<DirectIoBuffer>,
    large_value_physical_len: usize,
    segment_write: DirectIoBuffer,
) -> Result<u64> {
    let blob_future = async {
        match blob_write {
            Some(buffer) => write_all_direct(
                &data,
                buffer,
                location.record_start,
                blob_physical_len,
                config.write_max_time_us,
                "generation Blob write",
            )
            .await
            .map(Some),
            None => Ok(None),
        }
    };
    let segment_future = write_all_direct(
        &data,
        segment_write,
        location.sg_base,
        config.segment_size,
        config.write_max_time_us,
        "generation SG write",
    );
    let large_value_future = async {
        match (large_value_write, large_value_location) {
            (Some(buffer), Some(location)) => write_all_direct(
                &large_values,
                buffer,
                location.record_start,
                large_value_physical_len,
                config.write_max_time_us,
                "large-value write",
            )
            .await
            .map(Some),
            (None, None) => Ok(None),
            _ => Err(KvError::Worker(
                "large-value buffer and reservation disagree".into(),
            )),
        }
    };
    let (blob_result, segment_result, large_value_result) =
        futures_util::join!(blob_future, segment_future, large_value_future);
    let _blob_buffer = blob_result?;
    let _segment_buffer = segment_result?;
    let _large_value_buffer = large_value_result?;
    Ok(blob_physical_len as u64 + config.segment_size as u64 + large_value_physical_len as u64)
}

pub(super) fn bucket_hash_index_for_bucket(
    storage_key: &StorageKey,
    bucket_index: usize,
    bucket_count: usize,
    bucket_choice_count: usize,
) -> Option<u8> {
    let hashes = BucketHashSequence::new(storage_key, bucket_count);
    (0..bucket_choice_count as u8).find(|index| hashes.get(*index) == bucket_index)
}
