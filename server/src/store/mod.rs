//! Core cache engine and its Segment lifecycle.
//!
//! The in-memory Table is the only source of logical liveness. Deletes remove
//! an active Item physically; Items already on SSD remain stale until their
//! Segment is reused.

use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::time::Duration;

use compio::BufResult;
use compio::fs::{File, OpenOptions};
use compio::io::AsyncWriteAtExt;
use futures_util::stream::{FuturesUnordered, StreamExt};

use crate::types::HASHED_KEY_BYTES;
use crate::*;

mod blob;
mod bucket;
mod segment_io;

pub(crate) use self::blob::*;
pub(crate) use self::bucket::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetOutcome {
    Created,
    Replaced,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct KvkacheIoStats {
    pub(crate) data_written: u64,
    pub(crate) data_read: u64,
}

#[derive(Default)]
struct IoCounters {
    data_written: Cell<u64>,
    data_read: Cell<u64>,
}

struct LocatedItem {
    table_location: TableLocation,
    item: Item,
}

pub(crate) struct Kvkache {
    config: Config,
    data: File,
    pub(crate) table: Table,
    pub(crate) blob_segment: BlobSegment,
    pub(crate) blob_refs: HashMap<[u8; HASHED_KEY_BYTES], BlobRef>,
    pub(crate) active: Option<MutableSegment>,
    pub(crate) occupied_segments: Vec<bool>,
    next_segment_index: usize,
    pub(crate) segment_flushes: u64,
    pub(crate) segment_reuses: u64,
    io: IoCounters,
}

impl Kvkache {
    pub(crate) async fn open(config: Config) -> Result<Self> {
        config.validate()?;
        if let Some(parent) = config.data_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&config.data_path)
            .await?;
        data.set_len(config.data_bytes()).await?;
        let blob_segment = BlobSegment::open(&config).await?;

        Ok(Self {
            table: Table::new(&config)?,
            blob_segment,
            blob_refs: HashMap::new(),
            active: None,
            occupied_segments: vec![false; config.segment_count],
            next_segment_index: 0,
            segment_flushes: 0,
            segment_reuses: 0,
            io: IoCounters::default(),
            config,
            data,
        })
    }

    pub(crate) async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let hashed_key = Key::from(key).hashed_key().into_bytes();
        Ok(self
            .locate(&hashed_key)
            .await?
            .map(|located| located.item.value))
    }

    pub(crate) async fn get_many(&self, keys: Vec<Vec<u8>>) -> Vec<Result<Option<Vec<u8>>>> {
        let count = keys.len();
        let mut pending = FuturesUnordered::new();
        for (index, key) in keys.into_iter().enumerate() {
            pending.push(async move { (index, self.get(&key).await) });
        }
        let mut results = (0..count).map(|_| None).collect::<Vec<_>>();
        while let Some((index, result)) = pending.next().await {
            results[index] = Some(result);
        }
        results
            .into_iter()
            .map(|result| result.expect("every get future completes"))
            .collect()
    }

    pub(crate) async fn set(&mut self, key: &[u8], value: &[u8]) -> Result<SetOutcome> {
        let hashed_key = Key::from(key).hashed_key().into_bytes();
        if is_blob_item(key, value) {
            return self.set_blob(hashed_key, value).await;
        }
        self.set_bucket_item(hashed_key, value).await
    }

    async fn set_bucket_item(
        &mut self,
        hashed_key: [u8; HASHED_KEY_BYTES],
        value: &[u8],
    ) -> Result<SetOutcome> {
        let item_bytes = item_offsets_bytes(1) + ITEM_FIXED_BYTES + value.len();
        let capacity = BUCKET_BYTES - 1;
        if item_bytes > capacity {
            return Err(KvError::ItemTooLarge {
                bytes: item_bytes,
                capacity,
            });
        }
        let previous = self.locate(&hashed_key).await?;
        let item = Item {
            hashed_key,
            value: value.to_vec(),
        };
        let replacement = self
            .active
            .as_mut()
            .map(|active| active.replace(&hashed_key, item.clone(), true));
        let table_location = match replacement {
            Some(MutableItemReplace::Replaced(table_location)) => table_location,
            Some(MutableItemReplace::NotFound | MutableItemReplace::NoSpace) | None => {
                self.append_with_retry(item, true).await?
            }
        };
        if let Some(previous) = &previous {
            if !self
                .table
                .replace_location(&hashed_key, previous.table_location, table_location)
            {
                return Err(KvError::Worker(
                    "updated key is missing from the Table".into(),
                ));
            }
        } else {
            self.table.insert(&hashed_key, table_location)?;
        }
        if previous
            .as_ref()
            .is_some_and(|previous| previous.table_location.is_blob())
        {
            self.blob_refs.remove(&hashed_key);
        }
        Ok(if previous.is_some() {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        })
    }

    async fn set_blob(
        &mut self,
        hashed_key: [u8; HASHED_KEY_BYTES],
        value: &[u8],
    ) -> Result<SetOutcome> {
        let previous = self.locate(&hashed_key).await?;
        let blob_ref = self.blob_segment.append(&hashed_key, value).await?;
        self.io
            .data_written
            .set(self.io.data_written.get() + BLOB_HASHED_KEY_BYTES + blob_ref.value_len);

        let table_location = TableLocation::blob();
        if let Some(previous) = &previous {
            if !self
                .table
                .replace_location(&hashed_key, previous.table_location, table_location)
            {
                return Err(KvError::Worker(
                    "updated key is missing from the Table".into(),
                ));
            }
        } else {
            self.table.insert(&hashed_key, table_location)?;
        }
        if let Some(previous) = &previous
            && !previous.table_location.is_blob()
            && let Some(active) = self.active.as_mut()
            && active.sg_index == previous.table_location.sg_index as usize
        {
            let removed = active.remove(&hashed_key);
            debug_assert!(removed);
        }
        self.blob_refs.insert(hashed_key, blob_ref);
        Ok(if previous.is_some() {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        })
    }

    pub(crate) async fn delete(&mut self, key: &[u8]) -> Result<bool> {
        let hashed_key = Key::from(key).hashed_key().into_bytes();
        let Some(previous) = self.locate(&hashed_key).await? else {
            return Ok(false);
        };
        if previous.table_location.is_blob() {
            let removed = self.table.remove(&hashed_key, previous.table_location);
            debug_assert!(removed);
            self.blob_refs.remove(&hashed_key);
            return Ok(true);
        }
        if let Some(active) = self.active.as_mut()
            && active.sg_index == previous.table_location.sg_index as usize
        {
            let removed = active.remove(&hashed_key);
            debug_assert!(removed);
        }
        let removed = self.table.remove(&hashed_key, previous.table_location);
        debug_assert!(removed);
        Ok(true)
    }

    pub(crate) async fn sync(&mut self) -> Result<()> {
        self.blob_segment.sync().await?;
        self.flush_active_segment().await
    }

    async fn locate(&self, hashed_key: &[u8; 32]) -> Result<Option<LocatedItem>> {
        for table_location in self.table.candidate_locations(hashed_key) {
            if table_location.is_blob() {
                let Some(blob_ref) = self.blob_refs.get(hashed_key).copied() else {
                    continue;
                };
                let value = self.blob_segment.read(hashed_key, blob_ref).await?;
                self.io
                    .data_read
                    .set(self.io.data_read.get() + BLOB_HASHED_KEY_BYTES + blob_ref.value_len);
                return Ok(Some(LocatedItem {
                    table_location,
                    item: Item {
                        hashed_key: *hashed_key,
                        value,
                    },
                }));
            }
            let active = self
                .active
                .as_ref()
                .filter(|active| active.sg_index == table_location.sg_index as usize);
            let item = if let Some(active) = active {
                active.find(hashed_key, table_location.bucket_hash_index)
            } else {
                self.read_location(hashed_key, table_location).await?
            };
            if let Some(item) = item {
                return Ok(Some(LocatedItem {
                    table_location,
                    item,
                }));
            }
        }
        Ok(None)
    }

    async fn read_location(
        &self,
        hashed_key: &[u8; 32],
        table_location: TableLocation,
    ) -> Result<Option<Item>> {
        debug_assert!(!table_location.is_blob());
        let sg_index = table_location.sg_index as usize;
        if !self
            .occupied_segments
            .get(sg_index)
            .copied()
            .unwrap_or(false)
        {
            return Ok(None);
        }
        let bucket_index = bucket_hash(
            hashed_key,
            table_location.bucket_hash_index,
            self.config.bucket_count(),
        );
        let bytes = self.read_bucket(sg_index, bucket_index).await?;
        Ok(find_item_in_bucket(&bytes, hashed_key))
    }

    async fn append_with_retry(
        &mut self,
        item: Item,
        count_accepted: bool,
    ) -> Result<TableLocation> {
        loop {
            self.ensure_active_segment().await?;
            if let Some(table_location) = self
                .active
                .as_mut()
                .unwrap()
                .append(item.clone(), count_accepted)
            {
                return Ok(table_location);
            }
            self.flush_active_segment().await?;
        }
    }

    async fn ensure_active_segment(&mut self) -> Result<()> {
        if self.active.is_some() {
            return Ok(());
        }
        let sg_index = self.next_segment_index;
        if self.occupied_segments[sg_index] {
            self.prepare_segment_for_reuse(sg_index).await?;
        }
        self.active = Some(MutableSegment::new(&self.config, sg_index));
        Ok(())
    }

    async fn flush_active_segment(&mut self) -> Result<()> {
        let Some(active) = self.active.take() else {
            return Ok(());
        };
        if active.item_count == 0 {
            return Ok(());
        }
        let offset = active.sg_index as u64 * self.config.segment_size as u64;
        let write = self.data.write_all_at(active.bytes, offset);
        let BufResult(result, bytes) = compio::runtime::time::timeout(
            Duration::from_micros(self.config.write_max_time_us),
            write,
        )
        .await
        .map_err(|_| KvError::Timeout("Segment write"))?;
        result?;
        self.io
            .data_written
            .set(self.io.data_written.get() + bytes.len() as u64);
        self.data.sync_data().await?;
        self.occupied_segments[active.sg_index] = true;
        self.next_segment_index = (active.sg_index + 1) % self.config.segment_count;
        self.segment_flushes += 1;
        Ok(())
    }

    pub(crate) fn stats(&self) -> String {
        let io = self.io_stats();
        format!(
            "keys={} table_load={:.2}% table_memory={:.2}MiB ({:.3}B/planned-key) modeled_resident={:.2}MiB front_subtables={} front_capacity={} back_subtables={} back_capacity={} blob_refs={} blob_used={} blob_capacity={} next_segment_index={} occupied_segments={} flushes={} segment_reuses={} data_read={} data_written={}",
            self.table.entry_count,
            self.table.load_factor() * 100.0,
            self.table.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.table.memory_bytes() as f64 / self.config.table_capacity as f64,
            self.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.table.front_table.len(),
            self.table.front_subtable_layout.entry_capacity,
            self.table.back_table.len(),
            self.table.back_subtable_layout.entry_capacity,
            self.blob_refs.len(),
            self.blob_segment.used_bytes(),
            self.blob_segment.capacity_bytes(),
            self.next_segment_index,
            self.occupied_segments
                .iter()
                .filter(|value| **value)
                .count(),
            self.segment_flushes,
            self.segment_reuses,
            io.data_read,
            io.data_written,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn reset_io_stats(&self) {
        self.io.data_written.set(0);
        self.io.data_read.set(0);
    }

    pub(super) fn io_stats(&self) -> KvkacheIoStats {
        KvkacheIoStats {
            data_written: self.io.data_written.get(),
            data_read: self.io.data_read.get(),
        }
    }

    pub(super) fn memory_bytes(&self) -> usize {
        self.table.memory_bytes()
            + self.config.segment_size
            + self.blob_refs.capacity()
                * (std::mem::size_of::<[u8; HASHED_KEY_BYTES]>() + std::mem::size_of::<BlobRef>())
            + self.occupied_segments.capacity() * std::mem::size_of::<bool>()
    }
}
