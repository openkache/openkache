//! Persistence layer for `Kvkache`: checkpoint save/load, page-level read/write,
//! slot-group scanning, region eviction, and full recovery from data pages.
//! Implements the durable storage contract (index + data) for the cache engine.

use std::collections::HashMap;
use std::io;
use std::time::Duration;

use crate::types::HASHED_KEY_BYTES;
use crate::*;
use compio::BufResult;
use compio::fs::OpenOptions;
use compio::io::{AsyncReadAtExt, AsyncWriteAtExt};

pub(crate) const CHECKPOINT_MAGIC: &[u8; 8] = b"KVKIDX01";
pub(crate) const CHECKPOINT_VERSION: u32 = 3;
pub(crate) const NONE_GENERATION: u64 = u64::MAX;

/// Recovers the choice bit whose hash maps a stored key to `page`.
fn page_choice_for_page(hash: &[u8; 32], page: usize, pages: usize) -> Option<u8> {
    if page_hash(hash, 0, pages) == page {
        Some(0)
    } else if page_hash(hash, 1, pages) == page {
        Some(1)
    } else {
        None
    }
}

impl Kvkache {
    pub(crate) async fn evict_region(&mut self, region: usize) -> Result<()> {
        let records = self.read_sg_records(region).await?;
        let mut newest = HashMap::<[u8; 32], (Record, TableLocation)>::new();
        for (record, location) in records {
            newest.insert(record.key, (record, location));
        }
        for (record, location) in newest
            .into_values()
            .filter(|(record, _)| record.kind == RECORD_SET)
        {
            if self
                .locate(&record.key)
                .await?
                .is_some_and(|current| current.location == location)
            {
                let _ = self.table.remove(&record.key, location);
            }
        }
        self.slot_generations[region] = None;
        self.evictions += 1;
        Ok(())
    }

    pub(super) async fn read_page(&self, region: usize, page: usize) -> Result<Vec<u8>> {
        let offset =
            region as u64 * self.config.sg_size as u64 + page as u64 * self.config.page_size as u64;
        let read = self
            .data
            .read_exact_at(Vec::with_capacity(self.config.page_size), offset);
        let BufResult(result, bytes) = compio::runtime::time::timeout(
            Duration::from_micros(self.config.read_max_time_us),
            read,
        )
        .await
        .map_err(|_| KvError::Timeout("page read"))?;
        result?;
        self.io
            .data_read
            .set(self.io.data_read.get() + bytes.len() as u64);
        Ok(bytes)
    }

    async fn read_sg_records(&self, region: usize) -> Result<Vec<(Record, TableLocation)>> {
        let mut result = Vec::new();
        for page in 0..self.config.page_count() {
            let bytes = self.read_page(region, page).await?;
            if verify_page(&bytes) {
                result.extend(records(&bytes).into_iter().filter_map(|mut record| {
                    let page_choice =
                        page_choice_for_page(&record.key, page, self.config.page_count())?;
                    record.page_choice = page_choice;
                    Some((
                        record,
                        TableLocation {
                            is_blob: false,
                            sg_index: region as u8,
                            bucket_hash_index: page_choice,
                        },
                    ))
                }));
            }
        }
        Ok(result)
    }

    async fn scan_slot_generation(&self, region: usize) -> Result<Option<u64>> {
        let first = self.read_page(region, 0).await?;
        if !verify_page(&first) {
            return Ok(None);
        }
        let generation = get_u64(&first, 8);
        for page in 1..self.config.page_count() {
            let bytes = self.read_page(region, page).await?;
            if !verify_page(&bytes) || get_u64(&bytes, 8) != generation {
                return Ok(None);
            }
        }
        Ok(Some(generation))
    }

    pub(super) async fn rebuild_from_data(&mut self) -> Result<()> {
        let mut occupied = Vec::new();
        for region in 0..self.config.sg_count {
            if let Some(generation) = self.scan_slot_generation(region).await? {
                self.slot_generations[region] = Some(generation);
                occupied.push((generation, region));
            }
        }
        occupied.sort_unstable();
        let mut latest = HashMap::<[u8; 32], (Record, TableLocation)>::new();
        for (_, region) in &occupied {
            for (record, location) in self.read_sg_records(*region).await? {
                latest.insert(record.key, (record, location));
            }
        }
        self.table = Table::new(&self.config)?;
        for (record, location) in latest.into_values() {
            if record.kind == RECORD_SET {
                self.table.insert(&record.key, location)?;
            }
        }
        if let Some((generation, region)) = occupied.last().copied() {
            self.next_generation = generation + 1;
            self.next_slot = (region + 1) % self.config.sg_count;
        }
        Ok(())
    }

    pub(super) async fn save_checkpoint(&self) -> Result<()> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(CHECKPOINT_MAGIC);
        push_u32(&mut bytes, CHECKPOINT_VERSION);
        for value in self.config.signature() {
            push_u64(&mut bytes, value);
        }
        push_u64(&mut bytes, self.next_slot as u64);
        push_u64(&mut bytes, self.next_generation);
        push_u64(&mut bytes, self.table.len as u64);
        push_u64(&mut bytes, self.table.front.len() as u64);
        push_u64(&mut bytes, self.table.back.len() as u64);
        push_u64(&mut bytes, self.table.back_group_count as u64);
        for generation in &self.slot_generations {
            push_u64(&mut bytes, generation.unwrap_or(NONE_GENERATION));
        }
        for bucket in &self.table.front {
            bytes.extend_from_slice(&bucket.bytes);
        }
        for bucket in &self.table.back {
            bytes.extend_from_slice(&bucket.bytes);
        }
        push_u64(&mut bytes, self.blob_segment.used_bytes());
        let mut blob_refs = self.blob_refs.iter().collect::<Vec<_>>();
        blob_refs.sort_unstable_by_key(|(hashed_key, _)| **hashed_key);
        push_u64(&mut bytes, blob_refs.len() as u64);
        for (hashed_key, blob_ref) in blob_refs {
            bytes.extend_from_slice(hashed_key);
            push_u64(&mut bytes, blob_ref.item_offset);
            push_u64(&mut bytes, blob_ref.value_len);
        }
        let checksum = checksum64(&bytes);
        push_u64(&mut bytes, checksum);

        let temporary = self.config.index_path.with_extension("index.tmp");
        {
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&temporary)
                .await?;
            let write = file.write_all_at(bytes, 0);
            let BufResult(result, returned) = compio::runtime::time::timeout(
                Duration::from_micros(self.config.write_max_time_us),
                write,
            )
            .await
            .map_err(|_| KvError::Timeout("checkpoint write"))?;
            result?;
            bytes = returned;
            file.sync_all().await?;
            file.close().await?;
        }
        compio::fs::rename(&temporary, &self.config.index_path).await?;
        self.io
            .index_written
            .set(self.io.index_written.get() + bytes.len() as u64);
        if let Some(parent) = self.config.index_path.parent() {
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY)
                .open(parent)
                .await?;
            directory.sync_all().await?;
            directory.close().await?;
        }
        Ok(())
    }

    pub(super) async fn load_checkpoint(&mut self) -> Result<bool> {
        let checkpoint_read = compio::fs::read(&self.config.index_path);
        let mut bytes = match compio::runtime::time::timeout(
            Duration::from_micros(self.config.read_max_time_us),
            checkpoint_read,
        )
        .await
        .map_err(|_| KvError::Timeout("checkpoint read"))?
        {
            Ok(bytes) => {
                self.io
                    .index_read
                    .set(self.io.index_read.get() + bytes.len() as u64);
                bytes
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        if bytes.len() < 16 {
            return Ok(false);
        }
        let stored_checksum = u64::from_le_bytes(bytes[bytes.len() - 8..].try_into().unwrap());
        bytes.truncate(bytes.len() - 8);
        if checksum64(&bytes) != stored_checksum {
            return Ok(false);
        }
        let mut cursor = Cursor::new(&bytes);
        if cursor.take(8)? != CHECKPOINT_MAGIC || cursor.u32()? != CHECKPOINT_VERSION {
            return Ok(false);
        }
        for expected in self.config.signature() {
            if cursor.u64()? != expected {
                return Ok(false);
            }
        }
        let next_slot = cursor.u64()? as usize;
        let next_generation = cursor.u64()?;
        let len = cursor.u64()? as usize;
        let front_count = cursor.u64()? as usize;
        let back_count = cursor.u64()? as usize;
        let back_group_count = cursor.u64()? as usize;
        if front_count != self.table.front.len()
            || back_count != self.table.back.len()
            || back_group_count != self.table.back_group_count
        {
            return Ok(false);
        }
        let mut generations = Vec::with_capacity(self.config.sg_count);
        for region in 0..self.config.sg_count {
            let value = cursor.u64()?;
            let generation = (value != NONE_GENERATION).then_some(value);
            if self.scan_slot_generation(region).await? != generation {
                return Ok(false);
            }
            generations.push(generation);
        }
        for bucket in &mut self.table.front {
            bucket.bytes.copy_from_slice(cursor.take(BUCKET_BYTES)?);
        }
        for bucket in &mut self.table.back {
            bucket.bytes.copy_from_slice(cursor.take(BUCKET_BYTES)?);
        }
        let blob_used_bytes = cursor.u64()?;
        if blob_used_bytes > self.blob_segment.capacity_bytes() {
            return Ok(false);
        }
        let blob_ref_count = cursor.u64()? as usize;
        let mut blob_refs = HashMap::with_capacity(blob_ref_count);
        for _ in 0..blob_ref_count {
            let mut hashed_key = [0u8; HASHED_KEY_BYTES];
            hashed_key.copy_from_slice(cursor.take(HASHED_KEY_BYTES)?);
            let blob_ref = BlobRef {
                item_offset: cursor.u64()?,
                value_len: cursor.u64()?,
            };
            let item_end = blob_ref
                .item_offset
                .checked_add(BLOB_HASHED_KEY_BYTES)
                .and_then(|offset| offset.checked_add(blob_ref.value_len));
            if item_end.is_none_or(|item_end| item_end > blob_used_bytes)
                || blob_refs.insert(hashed_key, blob_ref).is_some()
            {
                return Ok(false);
            }
        }
        if !cursor.remaining().is_empty() {
            return Ok(false);
        }
        self.table.len = len;
        self.blob_segment.restore_used_bytes(blob_used_bytes)?;
        self.blob_refs = blob_refs;
        self.slot_generations = generations;
        self.next_slot = next_slot;
        self.next_generation = next_generation;
        Ok(true)
    }

    pub(crate) fn stats(&self) -> String {
        let io = self.io_stats();
        format!(
            "keys={} index_load={:.2}% index_memory={:.2}MiB ({:.3}B/planned-key) modeled_resident={:.2}MiB front_buckets={} front_capacity={} back_buckets={} back_capacity={} next_slot={} generations={} flushes={} evictions={} data_read={} data_written={} index_read={} index_written={}",
            self.table.len,
            self.table.load_factor() * 100.0,
            self.table.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.table.memory_bytes() as f64 / self.config.index_capacity as f64,
            self.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.table.front.len(),
            self.table.front_layout.capacity,
            self.table.back.len(),
            self.table.back_layout.capacity,
            self.next_slot,
            self.next_generation,
            self.data_flushes,
            self.evictions,
            io.data_read,
            io.data_written,
            io.index_read,
            io.index_written,
        )
    }

    #[allow(dead_code)]
    // Used by the cross-prototype benchmark; the standalone CLI only reports
    // cumulative counters and therefore does not reset them.
    pub(crate) fn reset_io_stats(&self) {
        self.io.data_written.set(0);
        self.io.data_read.set(0);
        self.io.index_written.set(0);
        self.io.index_read.set(0);
    }

    pub(super) fn io_stats(&self) -> KvkacheIoStats {
        KvkacheIoStats {
            data_written: self.io.data_written.get(),
            data_read: self.io.data_read.get(),
            index_written: self.io.index_written.get(),
            index_read: self.io.index_read.get(),
        }
    }

    pub(super) fn memory_bytes(&self) -> usize {
        self.table.memory_bytes()
            + self.config.sg_size
            + self.blob_refs.capacity()
                * (std::mem::size_of::<[u8; HASHED_KEY_BYTES]>() + std::mem::size_of::<BlobRef>())
            + self.slot_generations.capacity() * std::mem::size_of::<Option<u64>>()
    }
}
