//! Dense Blob Segments paired one-to-one with Bucket-oriented Segments.
//!
//! A flushed Segment stores complete keys and either inline values or compact
//! [`BlobRef`] descriptors. External values are concatenated without per-value
//! padding in the paired Blob Segment. Only the end of the complete Blob write
//! is padded for `O_DIRECT`.

use std::time::Duration;

use compio::BufResult;
use compio::buf::{IntoInner, IoBuf};
use compio::fs::File;
use compio::io::{AsyncReadAt, AsyncWriteAt};

use super::{ResourceGuard, open_direct_file, require_complete_direct_io};
use crate::BUCKET_BYTES;
use crate::buffer::DirectIoBuffer;
use crate::config::Config;
use crate::error::{KvError, Result};
use crate::types::STORAGE_KEY_BYTES;

pub(crate) const BLOB_ITEM_THRESHOLD_BYTES: usize = 2 * 1024;
pub(crate) const BLOB_REF_BYTES: usize = 8;
pub(crate) const STORED_VALUE_TAG_BYTES: usize = 1;
pub(crate) const STORED_BLOB_REF_BYTES: usize = STORED_VALUE_TAG_BYTES + BLOB_REF_BYTES;

const INLINE_VALUE_TAG: u8 = 0;
const BLOB_VALUE_TAG: u8 = 1;
const COMPRESSED_VALUE_TAG: u8 = 1 << 1;
const ENCRYPTED_VALUE_TAG: u8 = 1 << 2;
const KNOWN_VALUE_TAG_BITS: u8 = BLOB_VALUE_TAG | COMPRESSED_VALUE_TAG | ENCRYPTED_VALUE_TAG;
const BLOB_WRITE_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlobRef {
    pub(crate) value_offset: u32,
    pub(crate) value_len: u32,
}

impl BlobRef {
    pub(crate) fn new(value_offset: usize, value_len: usize) -> Result<Self> {
        Ok(Self {
            value_offset: u32::try_from(value_offset)
                .map_err(|_| KvError::Usage("Blob value offset does not fit in u32".into()))?,
            value_len: u32::try_from(value_len)
                .map_err(|_| KvError::Usage("Blob value length does not fit in u32".into()))?,
        })
    }

    fn encode(self) -> [u8; BLOB_REF_BYTES] {
        let mut bytes = [0; BLOB_REF_BYTES];
        bytes[..4].copy_from_slice(&self.value_offset.to_le_bytes());
        bytes[4..].copy_from_slice(&self.value_len.to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        (bytes.len() == BLOB_REF_BYTES).then(|| Self {
            value_offset: u32::from_le_bytes(bytes[..4].try_into().unwrap()),
            value_len: u32::from_le_bytes(bytes[4..].try_into().unwrap()),
        })
    }
}

pub(crate) enum StoredValue<'a> {
    Inline(&'a [u8]),
    Blob(BlobRef),
}

pub(crate) struct DecodedStoredValue<'a> {
    pub(crate) value: StoredValue<'a>,
    pub(crate) flags: openkache_protocol::ValueFlags,
}

pub(crate) fn encode_inline_value(value: &[u8], flags: openkache_protocol::ValueFlags) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(STORED_VALUE_TAG_BYTES + value.len());
    encoded.push(encode_stored_value_tag(INLINE_VALUE_TAG, flags));
    encoded.extend_from_slice(value);
    encoded
}

pub(crate) fn encode_blob_ref(blob_ref: BlobRef, flags: openkache_protocol::ValueFlags) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(STORED_BLOB_REF_BYTES);
    encoded.push(encode_stored_value_tag(BLOB_VALUE_TAG, flags));
    encoded.extend_from_slice(&blob_ref.encode());
    encoded
}

pub(crate) fn decode_stored_value(encoded: &[u8]) -> Result<DecodedStoredValue<'_>> {
    let Some((&tag, body)) = encoded.split_first() else {
        return Err(KvError::Worker(
            "Segment Item has no stored-value tag".into(),
        ));
    };
    if tag & !KNOWN_VALUE_TAG_BITS != 0 {
        return Err(KvError::Worker(format!(
            "Segment Item has unknown stored-value tag {tag}"
        )));
    }
    let flags = openkache_protocol::ValueFlags::new(
        tag & COMPRESSED_VALUE_TAG != 0,
        tag & ENCRYPTED_VALUE_TAG != 0,
    );
    let value = match tag & BLOB_VALUE_TAG {
        INLINE_VALUE_TAG => StoredValue::Inline(body),
        BLOB_VALUE_TAG => BlobRef::decode(body)
            .map(StoredValue::Blob)
            .ok_or_else(|| KvError::Worker("Segment Item has a malformed BlobRef".into()))?,
        _ => unreachable!("the Blob tag occupies one bit"),
    };
    Ok(DecodedStoredValue { value, flags })
}

pub(crate) fn remove_stored_value_tag(encoded: &mut Vec<u8>) {
    debug_assert!(encoded.len() >= STORED_VALUE_TAG_BYTES);
    encoded.copy_within(STORED_VALUE_TAG_BYTES.., 0);
    encoded.truncate(encoded.len() - STORED_VALUE_TAG_BYTES);
}

fn encode_stored_value_tag(kind: u8, flags: openkache_protocol::ValueFlags) -> u8 {
    kind | if flags.is_compressed() {
        COMPRESSED_VALUE_TAG
    } else {
        0
    } | if flags.is_encrypted() {
        ENCRYPTED_VALUE_TAG
    } else {
        0
    }
}

pub(crate) struct BlobSegment {
    file: File,
    segment_capacity_bytes: u64,
    segment_logical_bytes: Vec<u64>,
    segment_physical_bytes: Vec<u64>,
    read_max_time_us: u64,
    write_max_time_us: u64,
}

impl BlobSegment {
    pub(crate) async fn open(config: &Config) -> Result<Self> {
        let path = config.blob_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = open_direct_file(&path).await?;
        file.set_len(config.blob_bytes()).await?;
        Ok(Self {
            file,
            segment_capacity_bytes: config.blob_segment_size as u64,
            segment_logical_bytes: vec![0; config.segment_count],
            segment_physical_bytes: vec![0; config.segment_count],
            read_max_time_us: config.read_max_time_us,
            write_max_time_us: config.write_max_time_us,
        })
    }

    pub(crate) async fn reserve_segment(
        &self,
        sg_index: usize,
        logical_bytes: usize,
        resource_guard: &ResourceGuard,
    ) -> Result<()> {
        self.validate_segment_index(sg_index)?;
        if logical_bytes == 0 {
            return Ok(());
        }
        let physical_bytes = logical_bytes
            .checked_next_multiple_of(BUCKET_BYTES)
            .ok_or_else(|| KvError::Usage("Blob reservation extent overflow".into()))?;
        let offset = (sg_index as u64)
            .checked_mul(self.segment_capacity_bytes)
            .ok_or_else(|| KvError::Usage("Blob reservation offset overflow".into()))?;
        super::reserve_file_range(&self.file, offset, physical_bytes as u64)
            .await
            .map_err(|error| super::storage_io_error(resource_guard, error))
    }

    /// Writes the logical concatenation of `values` into one paired Blob
    /// Segment and returns the physical byte count with the reusable staging
    /// buffer, when one was needed.
    pub(crate) async fn write_segment<'a>(
        &mut self,
        sg_index: usize,
        values: impl Clone + Iterator<Item = &'a [u8]>,
        buffer: Option<DirectIoBuffer>,
    ) -> Result<(u64, Option<DirectIoBuffer>)> {
        self.validate_segment_index(sg_index)?;
        let logical_bytes = values.clone().try_fold(0usize, |total, value| {
            total
                .checked_add(value.len())
                .ok_or_else(|| KvError::Usage("Blob Segment length overflow".into()))
        })?;
        if logical_bytes > self.segment_capacity_bytes as usize {
            return Err(KvError::BlobSegmentFull {
                required_bytes: logical_bytes as u64,
                remaining_bytes: self.segment_capacity_bytes,
            });
        }
        if logical_bytes == 0 {
            self.segment_logical_bytes[sg_index] = 0;
            self.segment_physical_bytes[sg_index] = 0;
            return Ok((0, buffer));
        }

        let chunk_capacity = BLOB_WRITE_BUFFER_BYTES.min(self.segment_capacity_bytes as usize);
        debug_assert!(chunk_capacity.is_multiple_of(BUCKET_BYTES));
        let segment_base = sg_index as u64 * self.segment_capacity_bytes;
        let mut logical_written = 0usize;
        let mut physical_written = 0u64;
        let mut values = values;
        let mut value = values.next();
        let mut value_offset = 0usize;
        let mut buffer = buffer.unwrap_or_else(|| DirectIoBuffer::zeroed(chunk_capacity));
        debug_assert_eq!(buffer.len(), chunk_capacity);

        while logical_written < logical_bytes {
            let chunk_logical_bytes = chunk_capacity.min(logical_bytes - logical_written);
            let chunk_physical_bytes =
                chunk_logical_bytes
                    .checked_next_multiple_of(BUCKET_BYTES)
                    .ok_or_else(|| KvError::Usage("Blob write extent overflow".into()))?;
            let mut destination_offset = 0usize;
            while destination_offset < chunk_logical_bytes {
                while value.is_some_and(|value| value_offset == value.len()) {
                    value = values.next();
                    value_offset = 0;
                }
                let source = value.ok_or_else(|| {
                    KvError::Worker("Blob write sources ended before their logical length".into())
                })?;
                let copy_bytes =
                    (source.len() - value_offset).min(chunk_logical_bytes - destination_offset);
                buffer[destination_offset..destination_offset + copy_bytes]
                    .copy_from_slice(&source[value_offset..value_offset + copy_bytes]);
                destination_offset += copy_bytes;
                value_offset += copy_bytes;
            }
            buffer[chunk_logical_bytes..chunk_physical_bytes].fill(0);

            let write_offset = segment_base + logical_written as u64;
            let write = self
                .file
                .write_at(buffer.slice(..chunk_physical_bytes), write_offset);
            let BufResult(result, returned) = compio::runtime::time::timeout(
                Duration::from_micros(self.write_max_time_us),
                write,
            )
            .await
            .map_err(|_| KvError::Timeout("Blob Segment write"))?;
            require_complete_direct_io("Blob Segment write", result?, chunk_physical_bytes)?;
            buffer = returned.into_inner();
            logical_written += chunk_logical_bytes;
            physical_written += chunk_physical_bytes as u64;
        }

        self.segment_logical_bytes[sg_index] = logical_bytes as u64;
        self.segment_physical_bytes[sg_index] = physical_written;
        Ok((physical_written, Some(buffer)))
    }

    pub(crate) async fn read(&self, sg_index: usize, blob_ref: BlobRef) -> Result<Vec<u8>> {
        self.validate_ref(sg_index, blob_ref)?;
        let value_start = blob_ref.value_offset as u64;
        let value_end = value_start + blob_ref.value_len as u64;
        let read_start = value_start / BUCKET_BYTES as u64 * BUCKET_BYTES as u64;
        let read_end = value_end
            .checked_next_multiple_of(BUCKET_BYTES as u64)
            .ok_or_else(|| KvError::Worker("BlobRef read extent overflow".into()))?;
        let read_len = (read_end - read_start) as usize;
        let segment_base = sg_index as u64 * self.segment_capacity_bytes;
        let read = self.file.read_at(
            DirectIoBuffer::for_read(read_len),
            segment_base + read_start,
        );
        let BufResult(result, bytes) =
            compio::runtime::time::timeout(Duration::from_micros(self.read_max_time_us), read)
                .await
                .map_err(|_| KvError::Timeout("Blob Segment read"))?;
        require_complete_direct_io("Blob Segment read", result?, read_len)?;
        let relative_start = (value_start - read_start) as usize;
        let relative_end = relative_start + blob_ref.value_len as usize;
        Ok(bytes[relative_start..relative_end].to_vec())
    }

    pub(crate) async fn sync(&self) -> Result<()> {
        self.file.sync_data().await?;
        Ok(())
    }

    pub(crate) fn release_segment(&mut self, sg_index: usize) {
        if sg_index < self.segment_logical_bytes.len() {
            self.segment_logical_bytes[sg_index] = 0;
            self.segment_physical_bytes[sg_index] = 0;
        }
    }

    pub(crate) fn recover_segment(&mut self, sg_index: usize, logical_bytes: usize) -> Result<()> {
        self.validate_segment_index(sg_index)?;
        if logical_bytes > self.segment_capacity_bytes as usize {
            return Err(KvError::Worker(
                "recovered Blob length exceeds its Segment capacity".into(),
            ));
        }
        let chunk_capacity = BLOB_WRITE_BUFFER_BYTES.min(self.segment_capacity_bytes as usize);
        let full_chunks = logical_bytes / chunk_capacity;
        let remainder = logical_bytes % chunk_capacity;
        let physical_bytes = full_chunks as u64 * chunk_capacity as u64
            + if remainder == 0 {
                0
            } else {
                remainder.next_multiple_of(BUCKET_BYTES) as u64
            };
        self.segment_logical_bytes[sg_index] = logical_bytes as u64;
        self.segment_physical_bytes[sg_index] = physical_bytes;
        Ok(())
    }

    pub(crate) fn physical_read_bytes(&self, blob_ref: BlobRef) -> u64 {
        let start = blob_ref.value_offset as u64;
        let end = start + blob_ref.value_len as u64;
        let aligned_start = start / BUCKET_BYTES as u64 * BUCKET_BYTES as u64;
        let aligned_end = end.next_multiple_of(BUCKET_BYTES as u64);
        aligned_end - aligned_start
    }

    pub(crate) fn capacity_bytes(&self) -> u64 {
        self.segment_capacity_bytes * self.segment_logical_bytes.len() as u64
    }

    pub(crate) fn used_bytes(&self) -> u64 {
        self.segment_physical_bytes.iter().sum()
    }

    pub(crate) fn logical_used_bytes(&self) -> u64 {
        self.segment_logical_bytes.iter().sum()
    }

    pub(crate) fn memory_bytes(&self) -> usize {
        self.segment_logical_bytes.capacity() * std::mem::size_of::<u64>()
            + self.segment_physical_bytes.capacity() * std::mem::size_of::<u64>()
    }

    fn validate_segment_index(&self, sg_index: usize) -> Result<()> {
        if sg_index >= self.segment_logical_bytes.len() {
            return Err(KvError::Worker(format!(
                "Blob Segment index {sg_index} is out of range"
            )));
        }
        Ok(())
    }

    fn validate_ref(&self, sg_index: usize, blob_ref: BlobRef) -> Result<()> {
        self.validate_segment_index(sg_index)?;
        let value_end = (blob_ref.value_offset as u64)
            .checked_add(blob_ref.value_len as u64)
            .ok_or_else(|| KvError::Worker("BlobRef range overflow".into()))?;
        if blob_ref.value_len == 0
            || value_end > self.segment_capacity_bytes
            || value_end > self.segment_logical_bytes[sg_index]
        {
            return Err(KvError::Worker(
                "BlobRef points outside the written Blob Segment".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn is_blob_item(value: &[u8]) -> bool {
    STORAGE_KEY_BYTES.saturating_add(value.len()) > BLOB_ITEM_THRESHOLD_BYTES
}
