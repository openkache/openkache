//! SG-scoped storage for Items larger than the Bucket-oriented path.
//!
//! Blob Items are packed in RAM while their SG is active. Flushing the SG
//! compacts its live Blob Items into the matching fixed-size Blob region and
//! pads only the tail of that batch for direct I/O.

use std::time::Duration;

use compio::BufResult;
use compio::fs::File;
use compio::io::{AsyncReadAt, AsyncWriteAt};

use crate::types::STORAGE_KEY_BYTES;
use crate::*;

pub(crate) const BLOB_ITEM_THRESHOLD_BYTES: usize = 2 * 1024;
pub(crate) const BLOB_VALUE_LEN_BYTES: usize = size_of::<u64>();
pub(crate) const BLOB_ITEM_FIXED_BYTES: usize = STORAGE_KEY_BYTES + BLOB_VALUE_LEN_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlobRef {
    Active {
        sg_index: u16,
        item_index: u32,
    },
    Stored {
        sg_index: u16,
        item_offset: u64,
        value_len: u64,
    },
}

#[derive(Clone)]
struct BlobItem {
    storage_key: StorageKey,
    value: Vec<u8>,
}

pub(crate) enum MutableBlobReplace {
    NotFound,
    Replaced,
    NoSpace,
}

pub(crate) struct EncodedBlobSegment {
    pub(crate) bytes: DirectIoBuffer,
    pub(crate) logical_len: usize,
    pub(crate) refs: Vec<(StorageKey, BlobRef, BlobRef)>,
}

pub(crate) struct MutableBlobSegment {
    pub(crate) sg_index: usize,
    items: Vec<Option<BlobItem>>,
    logical_len: usize,
    capacity_bytes: usize,
}

impl MutableBlobSegment {
    pub(crate) fn new(config: &Config, sg_index: usize) -> Self {
        Self {
            sg_index,
            items: Vec::new(),
            logical_len: 0,
            capacity_bytes: config.segment_size,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.logical_len == 0
    }

    pub(crate) fn logical_len(&self) -> usize {
        self.logical_len
    }

    pub(crate) fn append(&mut self, storage_key: StorageKey, value: &[u8]) -> Option<BlobRef> {
        let encoded_len = encoded_blob_item_len(value.len())?;
        if self.logical_len.checked_add(encoded_len)? > self.capacity_bytes {
            return None;
        }
        let item_index = u32::try_from(self.items.len()).ok()?;
        self.items.push(Some(BlobItem {
            storage_key,
            value: value.to_vec(),
        }));
        self.logical_len += encoded_len;
        Some(BlobRef::Active {
            sg_index: self.sg_index as u16,
            item_index,
        })
    }

    pub(crate) fn replace(
        &mut self,
        storage_key: &StorageKey,
        blob_ref: BlobRef,
        value: &[u8],
    ) -> MutableBlobReplace {
        let BlobRef::Active {
            sg_index,
            item_index,
        } = blob_ref
        else {
            return MutableBlobReplace::NotFound;
        };
        if sg_index as usize != self.sg_index {
            return MutableBlobReplace::NotFound;
        }
        let Some(Some(item)) = self.items.get_mut(item_index as usize) else {
            return MutableBlobReplace::NotFound;
        };
        if item.storage_key != *storage_key {
            return MutableBlobReplace::NotFound;
        }
        let Some(old_len) = encoded_blob_item_len(item.value.len()) else {
            return MutableBlobReplace::NoSpace;
        };
        let Some(new_len) = encoded_blob_item_len(value.len()) else {
            return MutableBlobReplace::NoSpace;
        };
        let Some(new_logical_len) = self
            .logical_len
            .checked_sub(old_len)
            .and_then(|len| len.checked_add(new_len))
        else {
            return MutableBlobReplace::NoSpace;
        };
        if new_logical_len > self.capacity_bytes {
            return MutableBlobReplace::NoSpace;
        }
        item.value = value.to_vec();
        self.logical_len = new_logical_len;
        MutableBlobReplace::Replaced
    }

    pub(crate) fn remove(&mut self, storage_key: &StorageKey, blob_ref: BlobRef) -> bool {
        let BlobRef::Active {
            sg_index,
            item_index,
        } = blob_ref
        else {
            return false;
        };
        if sg_index as usize != self.sg_index {
            return false;
        }
        let Some(slot) = self.items.get_mut(item_index as usize) else {
            return false;
        };
        if slot
            .as_ref()
            .is_none_or(|item| item.storage_key != *storage_key)
        {
            return false;
        }
        let item = slot.take().expect("checked as present");
        self.logical_len -=
            encoded_blob_item_len(item.value.len()).expect("stored Blob Item length is valid");
        true
    }

    pub(crate) fn read(&self, storage_key: &StorageKey, blob_ref: BlobRef) -> Result<Vec<u8>> {
        let BlobRef::Active {
            sg_index,
            item_index,
        } = blob_ref
        else {
            return Err(KvError::Worker(
                "stored BlobRef cannot address an active Blob Item".into(),
            ));
        };
        let item = (sg_index as usize == self.sg_index)
            .then(|| self.items.get(item_index as usize))
            .flatten()
            .and_then(Option::as_ref)
            .filter(|item| item.storage_key == *storage_key)
            .ok_or_else(|| {
                KvError::Worker("BlobRef points to a different active Blob Item".into())
            })?;
        Ok(item.value.clone())
    }

    pub(crate) fn encode(self) -> Result<Option<EncodedBlobSegment>> {
        if self.logical_len == 0 {
            return Ok(None);
        }
        let physical_len = self
            .logical_len
            .checked_next_multiple_of(BUCKET_BYTES)
            .ok_or_else(|| KvError::Usage("Blob SG extent length overflow".into()))?;
        let mut bytes = DirectIoBuffer::zeroed(physical_len);
        let mut cursor = 0;
        let mut refs = Vec::new();
        for (item_index, item) in self.items.into_iter().enumerate() {
            let Some(item) = item else {
                continue;
            };
            let value_len = u64::try_from(item.value.len())
                .map_err(|_| KvError::Usage("Blob value length does not fit in u64".into()))?;
            let item_offset = cursor;
            bytes[cursor..cursor + STORAGE_KEY_BYTES].copy_from_slice(item.storage_key.as_bytes());
            cursor += STORAGE_KEY_BYTES;
            bytes[cursor..cursor + BLOB_VALUE_LEN_BYTES].copy_from_slice(&value_len.to_le_bytes());
            cursor += BLOB_VALUE_LEN_BYTES;
            bytes[cursor..cursor + item.value.len()].copy_from_slice(&item.value);
            cursor += item.value.len();
            refs.push((
                item.storage_key,
                BlobRef::Active {
                    sg_index: self.sg_index as u16,
                    item_index: item_index as u32,
                },
                BlobRef::Stored {
                    sg_index: self.sg_index as u16,
                    item_offset: item_offset as u64,
                    value_len,
                },
            ));
        }
        debug_assert_eq!(cursor, self.logical_len);
        Ok(Some(EncodedBlobSegment {
            bytes,
            logical_len: self.logical_len,
            refs,
        }))
    }
}

pub(crate) struct BlobSegment {
    file: File,
    segment_size: usize,
    segment_count: usize,
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
        file.set_len(config.data_bytes()).await?;
        Ok(Self {
            file,
            segment_size: config.segment_size,
            segment_count: config.segment_count,
            read_max_time_us: config.read_max_time_us,
            write_max_time_us: config.write_max_time_us,
        })
    }

    pub(crate) async fn write_segment(
        &mut self,
        sg_index: usize,
        bytes: DirectIoBuffer,
    ) -> Result<u64> {
        self.validate_segment(sg_index)?;
        if bytes.is_empty()
            || bytes.len() > self.segment_size
            || !bytes.len().is_multiple_of(BUCKET_BYTES)
        {
            return Err(KvError::Worker(
                "Blob SG write must be a non-empty aligned SG-sized range".into(),
            ));
        }
        let expected = bytes.len();
        let offset = self.segment_offset(sg_index);
        let write = self.file.write_at(bytes, offset);
        let BufResult(result, bytes) =
            compio::runtime::time::timeout(Duration::from_micros(self.write_max_time_us), write)
                .await
                .map_err(|_| KvError::Timeout("Blob SG write"))?;
        require_complete_direct_io("Blob SG write", result?, expected)?;
        Ok(bytes.len() as u64)
    }

    pub(crate) async fn read(
        &self,
        storage_key: &StorageKey,
        blob_ref: BlobRef,
        segment_logical_len: usize,
    ) -> Result<(Vec<u8>, u64)> {
        let BlobRef::Stored {
            sg_index,
            item_offset,
            value_len,
        } = blob_ref
        else {
            return Err(KvError::Worker(
                "active BlobRef cannot address an SSD Blob Item".into(),
            ));
        };
        let sg_index = sg_index as usize;
        self.validate_stored_ref(sg_index, item_offset, value_len, segment_logical_len)?;
        let logical_start = item_offset as usize;
        let logical_end = logical_start + BLOB_ITEM_FIXED_BYTES + value_len as usize;
        let read_start = logical_start / BUCKET_BYTES * BUCKET_BYTES;
        let read_end = logical_end
            .checked_next_multiple_of(BUCKET_BYTES)
            .ok_or_else(|| KvError::Worker("Blob read range overflow".into()))?;
        let extent_len = read_end - read_start;
        let offset = self.segment_offset(sg_index) + read_start as u64;
        let read = self
            .file
            .read_at(DirectIoBuffer::for_read(extent_len), offset);
        let BufResult(result, bytes) =
            compio::runtime::time::timeout(Duration::from_micros(self.read_max_time_us), read)
                .await
                .map_err(|_| KvError::Timeout("Blob SG read"))?;
        require_complete_direct_io("Blob SG read", result?, extent_len)?;
        let relative = logical_start - read_start;
        let key_end = relative + STORAGE_KEY_BYTES;
        if bytes[relative..key_end] != storage_key.as_bytes()[..] {
            return Err(KvError::Worker(
                "BlobRef points to an Item with a different StorageKey".into(),
            ));
        }
        let len_end = key_end + BLOB_VALUE_LEN_BYTES;
        let stored_value_len = u64::from_le_bytes(
            bytes[key_end..len_end]
                .try_into()
                .expect("Blob value length field has fixed width"),
        );
        if stored_value_len != value_len {
            return Err(KvError::Worker(
                "BlobRef value length differs from the stored Blob Item".into(),
            ));
        }
        let value_end = len_end + value_len as usize;
        Ok((bytes[len_end..value_end].to_vec(), extent_len as u64))
    }

    pub(crate) async fn read_segment_refs(
        &self,
        sg_index: usize,
        logical_len: usize,
    ) -> Result<(Vec<(StorageKey, BlobRef)>, u64)> {
        self.validate_segment(sg_index)?;
        if logical_len == 0 {
            return Ok((Vec::new(), 0));
        }
        if logical_len > self.segment_size {
            return Err(KvError::Worker(
                "Blob SG logical length exceeds its region".into(),
            ));
        }
        let physical_len = logical_len
            .checked_next_multiple_of(BUCKET_BYTES)
            .ok_or_else(|| KvError::Worker("Blob SG read length overflow".into()))?;
        let read = self.file.read_at(
            DirectIoBuffer::for_read(physical_len),
            self.segment_offset(sg_index),
        );
        let BufResult(result, bytes) =
            compio::runtime::time::timeout(Duration::from_micros(self.read_max_time_us), read)
                .await
                .map_err(|_| KvError::Timeout("Blob SG reuse read"))?;
        require_complete_direct_io("Blob SG reuse read", result?, physical_len)?;

        let mut refs = Vec::new();
        let mut cursor = 0;
        while cursor < logical_len {
            let header_end = cursor
                .checked_add(BLOB_ITEM_FIXED_BYTES)
                .filter(|end| *end <= logical_len)
                .ok_or_else(|| KvError::Worker("truncated Blob Item header".into()))?;
            let storage_key = StorageKey::new(
                bytes[cursor..cursor + STORAGE_KEY_BYTES]
                    .try_into()
                    .expect("Blob key field has fixed width"),
            );
            let value_len = u64::from_le_bytes(
                bytes[cursor + STORAGE_KEY_BYTES..header_end]
                    .try_into()
                    .expect("Blob value length field has fixed width"),
            );
            let item_end = header_end
                .checked_add(
                    usize::try_from(value_len)
                        .map_err(|_| KvError::Worker("Blob value length is too large".into()))?,
                )
                .filter(|end| *end <= logical_len)
                .ok_or_else(|| KvError::Worker("truncated Blob Item value".into()))?;
            refs.push((
                storage_key,
                BlobRef::Stored {
                    sg_index: sg_index as u16,
                    item_offset: cursor as u64,
                    value_len,
                },
            ));
            cursor = item_end;
        }
        Ok((refs, physical_len as u64))
    }

    pub(crate) async fn sync(&self) -> Result<()> {
        self.file.sync_data().await?;
        Ok(())
    }

    pub(crate) fn capacity_bytes(&self) -> u64 {
        (self.segment_size * self.segment_count) as u64
    }

    fn validate_stored_ref(
        &self,
        sg_index: usize,
        item_offset: u64,
        value_len: u64,
        segment_logical_len: usize,
    ) -> Result<()> {
        self.validate_segment(sg_index)?;
        let item_offset = usize::try_from(item_offset)
            .map_err(|_| KvError::Worker("BlobRef offset is too large".into()))?;
        let value_len = usize::try_from(value_len)
            .map_err(|_| KvError::Worker("BlobRef value length is too large".into()))?;
        let item_end = item_offset
            .checked_add(BLOB_ITEM_FIXED_BYTES)
            .and_then(|end| end.checked_add(value_len));
        if segment_logical_len > self.segment_size
            || item_end.is_none_or(|end| end > segment_logical_len)
        {
            return Err(KvError::Worker(
                "BlobRef points outside the written Blob SG".into(),
            ));
        }
        Ok(())
    }

    fn validate_segment(&self, sg_index: usize) -> Result<()> {
        if sg_index >= self.segment_count {
            return Err(KvError::Worker("Blob SG index is out of range".into()));
        }
        Ok(())
    }

    fn segment_offset(&self, sg_index: usize) -> u64 {
        (sg_index * self.segment_size) as u64
    }
}

fn encoded_blob_item_len(value_len: usize) -> Option<usize> {
    BLOB_ITEM_FIXED_BYTES.checked_add(value_len)
}

pub(crate) fn is_blob_item(value: &[u8]) -> bool {
    STORAGE_KEY_BYTES.saturating_add(value.len()) > BLOB_ITEM_THRESHOLD_BYTES
}
