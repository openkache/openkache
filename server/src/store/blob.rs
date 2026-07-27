//! Fixed-capacity Blob Segment for Items larger than the Bucket-oriented path.
//!
//! The Segment is append-only for one process lifetime. It does not rotate or
//! reclaim superseded Items: an append that does not fit fails explicitly.

use std::time::Duration;

use compio::BufResult;
use compio::fs::{File, OpenOptions};
use compio::io::{AsyncReadAtExt, AsyncWriteAtExt};

use crate::types::STORAGE_KEY_BYTES;
use crate::*;

pub(crate) const BLOB_ITEM_THRESHOLD_BYTES: usize = 2 * 1024;
pub(crate) const BLOB_STORAGE_KEY_BYTES: u64 = STORAGE_KEY_BYTES as u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlobRef {
    pub(crate) item_offset: u64,
    pub(crate) value_len: u64,
}

pub(crate) struct BlobSegment {
    file: File,
    capacity_bytes: u64,
    next_item_offset: u64,
    read_max_time_us: u64,
    write_max_time_us: u64,
}

impl BlobSegment {
    pub(crate) async fn open(config: &Config) -> Result<Self> {
        let path = config.blob_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .await?;
        file.set_len(config.segment_size as u64).await?;
        Ok(Self {
            file,
            capacity_bytes: config.segment_size as u64,
            next_item_offset: 0,
            read_max_time_us: config.read_max_time_us,
            write_max_time_us: config.write_max_time_us,
        })
    }

    pub(crate) async fn append(
        &mut self,
        storage_key: &StorageKey,
        value: &[u8],
    ) -> Result<BlobRef> {
        let value_len = u64::try_from(value.len())
            .map_err(|_| KvError::Usage("blob value length does not fit in u64".into()))?;
        let encoded_len = BLOB_STORAGE_KEY_BYTES
            .checked_add(value_len)
            .ok_or_else(|| KvError::Usage("blob Item length overflow".into()))?;
        let item_end = self
            .next_item_offset
            .checked_add(encoded_len)
            .ok_or_else(|| KvError::Usage("blob Segment offset overflow".into()))?;
        if item_end > self.capacity_bytes {
            return Err(KvError::BlobSegmentFull {
                required_bytes: encoded_len,
                remaining_bytes: self.remaining_bytes(),
            });
        }

        let mut bytes = Vec::with_capacity(encoded_len as usize);
        bytes.extend_from_slice(storage_key.as_bytes());
        bytes.extend_from_slice(value);
        let item_offset = self.next_item_offset;
        let write = self.file.write_all_at(bytes, item_offset);
        let BufResult(result, _) =
            compio::runtime::time::timeout(Duration::from_micros(self.write_max_time_us), write)
                .await
                .map_err(|_| KvError::Timeout("Blob Segment write"))?;
        result?;
        self.next_item_offset = item_end;
        Ok(BlobRef {
            item_offset,
            value_len,
        })
    }

    pub(crate) async fn read(
        &self,
        storage_key: &StorageKey,
        blob_ref: BlobRef,
    ) -> Result<Vec<u8>> {
        self.validate_ref(blob_ref)?;
        let encoded_len = BLOB_STORAGE_KEY_BYTES + blob_ref.value_len;
        let read = self.file.read_exact_at(
            Vec::with_capacity(encoded_len as usize),
            blob_ref.item_offset,
        );
        let BufResult(result, bytes) =
            compio::runtime::time::timeout(Duration::from_micros(self.read_max_time_us), read)
                .await
                .map_err(|_| KvError::Timeout("Blob Segment read"))?;
        result?;
        if bytes[..STORAGE_KEY_BYTES] != storage_key.as_bytes()[..] {
            return Err(KvError::Worker(
                "BlobRef points to an Item with a different StorageKey".into(),
            ));
        }
        Ok(bytes[STORAGE_KEY_BYTES..].to_vec())
    }

    pub(crate) async fn sync(&self) -> Result<()> {
        self.file.sync_data().await?;
        Ok(())
    }

    pub(crate) const fn capacity_bytes(&self) -> u64 {
        self.capacity_bytes
    }

    pub(crate) const fn used_bytes(&self) -> u64 {
        self.next_item_offset
    }

    pub(crate) const fn remaining_bytes(&self) -> u64 {
        self.capacity_bytes - self.next_item_offset
    }

    fn validate_ref(&self, blob_ref: BlobRef) -> Result<()> {
        let item_end = blob_ref
            .item_offset
            .checked_add(BLOB_STORAGE_KEY_BYTES)
            .and_then(|offset| offset.checked_add(blob_ref.value_len))
            .ok_or_else(|| KvError::Worker("BlobRef range overflow".into()))?;
        if item_end > self.next_item_offset {
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
