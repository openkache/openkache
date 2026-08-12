//! Mutable, RAM, and SSD value materialization helpers.

use crate::storage_runtime::File;
use crate::types::StoredItemValue;
use crate::{BUCKET_BYTES, Config, KvError, Result};

use super::{
    BlobArena, BlobHandle, BlobRef, DirectIoBuffer, DirectStoreIo, MAX_LEASED_SSD_VALUE_READ_BYTES,
    MutableGeneration, SsdBacking, StoredValue, decode_stored_value, read_exact_direct,
    remove_stored_value_tag,
};

pub(super) async fn read_owned_extent(
    file: &File,
    record_start: u64,
    logical_len: u32,
    value_ref: BlobRef,
    read_max_time_us: u64,
    operation: &'static str,
    invalid_range: &'static str,
    range_overflow: &'static str,
    overflow: &'static str,
    io: &DirectStoreIo,
) -> Result<Vec<u8>> {
    let logical_end = u64::from(value_ref.value_offset)
        .checked_add(u64::from(value_ref.value_len))
        .ok_or_else(|| KvError::Worker(range_overflow.into()))?;
    if logical_end > u64::from(logical_len) {
        return Err(KvError::Worker(invalid_range.into()));
    }
    if value_ref.value_len == 0 {
        return Ok(Vec::new());
    }
    let absolute = record_start + u64::from(value_ref.value_offset);
    let aligned_start = absolute / BUCKET_BYTES as u64 * BUCKET_BYTES as u64;
    let prefix = (absolute - aligned_start) as usize;
    let read_len = prefix
        .checked_add(value_ref.value_len as usize)
        .and_then(|len| len.checked_next_multiple_of(BUCKET_BYTES))
        .ok_or_else(|| KvError::Worker(overflow.into()))?;
    let bytes = read_exact_direct(
        file,
        DirectIoBuffer::for_read(read_len),
        aligned_start,
        read_len,
        read_max_time_us,
        operation,
    )
    .await?;
    io.data_read.set(io.data_read.get() + read_len as u64);
    Ok(bytes[prefix..prefix + value_ref.value_len as usize].to_vec())
}

pub(super) fn read_mutable_value(
    encoded: Vec<u8>,
    generation: &MutableGeneration,
) -> Result<Vec<u8>> {
    read_arena_value(
        encoded,
        &generation.blob_arena,
        &generation.large_value_arena,
        "mutable Blob handle is invalid",
        "mutable large-value handle is invalid",
    )
}

pub(super) fn read_arena_value(
    mut encoded: Vec<u8>,
    blob_arena: &BlobArena,
    large_value_arena: &BlobArena,
    invalid_blob: &'static str,
    invalid_large_value: &'static str,
) -> Result<Vec<u8>> {
    match decode_stored_value(&encoded)? {
        StoredValue::Inline(_) => {
            remove_stored_value_tag(&mut encoded);
            Ok(encoded)
        }
        StoredValue::Blob(blob_ref) => blob_arena
            .get(BlobHandle {
                slot: blob_ref.value_offset,
                value_len: blob_ref.value_len,
            })
            .map(ToOwned::to_owned)
            .ok_or_else(|| KvError::Worker(invalid_blob.into())),
        StoredValue::Large(value_ref) => large_value_arena
            .get(BlobHandle {
                slot: value_ref.value_offset,
                value_len: value_ref.value_len,
            })
            .map(ToOwned::to_owned)
            .ok_or_else(|| KvError::Worker(invalid_large_value.into())),
    }
}

pub(super) fn read_ram_value(encoded: Vec<u8>, backing: &super::RamBacking) -> Result<Vec<u8>> {
    read_arena_value(
        encoded,
        &backing.blob_arena,
        &backing.large_value_arena,
        "sealed Blob handle is invalid",
        "sealed large-value handle is invalid",
    )
}

pub(super) async fn read_ssd_value(
    data: &File,
    large_values: &File,
    config: &Config,
    backing: &SsdBacking,
    encoded: &mut Vec<u8>,
    io: &DirectStoreIo,
) -> Result<StoredItemValue> {
    match decode_stored_value(encoded)? {
        StoredValue::Inline(value) => Ok(StoredItemValue::new(value.to_vec())),
        StoredValue::Blob(blob_ref) => {
            let logical_end = u64::from(blob_ref.value_offset)
                .checked_add(u64::from(blob_ref.value_len))
                .ok_or_else(|| KvError::Worker("BlobRef range overflowed".into()))?;
            if logical_end > u64::from(backing.location.blob_logical_len) {
                return Err(KvError::Worker(
                    "BlobRef exceeds its generation Blob extent".into(),
                ));
            }
            read_ssd_extent(
                data,
                backing.location.record_start,
                blob_ref,
                config.read_max_time_us,
                "generation Blob read",
                io,
                config.lease_ssd_read_buffer,
            )
            .await
        }
        StoredValue::Large(value_ref) => {
            let location = backing
                .large_value_location
                .as_ref()
                .ok_or_else(|| KvError::Worker("large-value Item has no SSD extent".into()))?;
            let logical_end = u64::from(value_ref.value_offset)
                .checked_add(u64::from(value_ref.value_len))
                .ok_or_else(|| KvError::Worker("large-value ref range overflowed".into()))?;
            if logical_end > u64::from(location.logical_len) {
                return Err(KvError::Worker(
                    "large-value ref exceeds its SSD extent".into(),
                ));
            }
            read_ssd_extent(
                large_values,
                location.record_start,
                value_ref,
                config.read_max_time_us,
                "large-value read",
                io,
                config.lease_ssd_read_buffer,
            )
            .await
        }
    }
}

async fn read_ssd_extent(
    file: &File,
    record_start: u64,
    value_ref: BlobRef,
    read_max_time_us: u64,
    operation: &'static str,
    io: &DirectStoreIo,
    lease_response: bool,
) -> Result<StoredItemValue> {
    if value_ref.value_len == 0 {
        return Ok(StoredItemValue::new(Vec::new()));
    }
    let absolute = record_start + u64::from(value_ref.value_offset);
    let aligned_start = absolute / BUCKET_BYTES as u64 * BUCKET_BYTES as u64;
    let prefix = (absolute - aligned_start) as usize;
    let read_len = prefix
        .checked_add(value_ref.value_len as usize)
        .and_then(|len| len.checked_next_multiple_of(BUCKET_BYTES))
        .ok_or_else(|| KvError::Worker("direct-read extent overflowed".into()))?;
    if lease_response && read_len <= MAX_LEASED_SSD_VALUE_READ_BYTES {
        let bytes = read_exact_direct(
            file,
            io.value_read_pool.take_buffer(read_len).await,
            aligned_start,
            read_len,
            read_max_time_us,
            operation,
        )
        .await?;
        io.data_read.set(io.data_read.get() + read_len as u64);
        return Ok(StoredItemValue::from_direct_read(
            bytes,
            prefix..prefix + value_ref.value_len as usize,
        ));
    }
    let bytes = read_exact_direct(
        file,
        DirectIoBuffer::for_read(read_len),
        aligned_start,
        read_len,
        read_max_time_us,
        operation,
    )
    .await?;
    io.data_read.set(io.data_read.get() + read_len as u64);
    Ok(StoredItemValue::new(
        bytes[prefix..prefix + value_ref.value_len as usize].to_vec(),
    ))
}
