//! Core cache engine: `Kvkache` struct, public API (`get`, `set`, `delete`, `open`),
//! `SetOutcome` enum, `MutableSg` management, and the `flush_active` method.
//! This module drives the main cache lifecycle — lookups, mutations, slot-group management,
//! and I/O statistics collection.

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
mod codec;
mod page;
mod persistence;

pub(crate) use self::blob::*;
pub(crate) use self::codec::*;
pub(crate) use self::page::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetOutcome {
    Created,
    Replaced,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct KvkacheIoStats {
    pub(crate) data_written: u64,
    pub(crate) data_read: u64,
    pub(crate) index_written: u64,
    pub(crate) index_read: u64,
}

#[derive(Default)]
struct IoCounters {
    data_written: Cell<u64>,
    data_read: Cell<u64>,
    index_written: Cell<u64>,
    index_read: Cell<u64>,
}

#[derive(Clone)]
struct LocatedItem {
    location: TableLocation,
    generation: u64,
    record: Record,
}

pub(crate) struct Kvkache {
    config: Config,
    data: File,
    pub(crate) table: Table,
    pub(crate) blob_segment: BlobSegment,
    pub(crate) blob_refs: HashMap<[u8; HASHED_KEY_BYTES], BlobRef>,
    active: Option<MutableSg>,
    slot_generations: Vec<Option<u64>>,
    next_slot: usize,
    next_generation: u64,
    pub(crate) data_flushes: u64,
    evictions: u64,
    io: IoCounters,
}

impl Kvkache {
    pub(crate) async fn open(config: Config) -> Result<Self> {
        config.validate()?;
        if let Some(parent) = config.data_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(parent) = config.index_path.parent() {
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

        let mut cache = Self {
            table: Table::new(&config)?,
            blob_segment,
            blob_refs: HashMap::new(),
            active: None,
            slot_generations: vec![None; config.sg_count],
            next_slot: 0,
            next_generation: 0,
            data_flushes: 0,
            evictions: 0,
            io: IoCounters::default(),
            config,
            data,
        };
        if cache.config.recovery_enabled && !cache.load_checkpoint().await? {
            if !cache.config.fallback_to_sg_scan {
                return Err(KvError::Corrupt(
                    "checkpoint is absent or invalid and SG fallback is disabled".into(),
                ));
            }
            cache.rebuild_from_data().await?;
            if cache.slot_generations.iter().any(Option::is_some) {
                cache.save_checkpoint().await?;
            }
        }
        Ok(cache)
    }

    pub(crate) async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let hash = Key::from(key).hashed_key().into_bytes();
        Ok(self
            .locate(&hash)
            .await?
            .filter(|located| located.record.kind == RECORD_SET)
            .map(|located| located.record.value))
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
        let record_bytes = directory_bytes(1) + RECORD_FIXED_BYTES + value.len();
        if PAGE_HEADER + record_bytes > self.config.page_size {
            return Err(KvError::RecordTooLarge {
                bytes: record_bytes,
                capacity: self.config.page_size - PAGE_HEADER,
            });
        }
        let previous = self.locate(&hashed_key).await?;
        let record = Record {
            kind: RECORD_SET,
            page_choice: 0,
            key: hashed_key,
            value: value.to_vec(),
        };
        let replacement = self
            .active
            .as_mut()
            .map(|active| active.replace(&hashed_key, record.clone(), true));
        let location = match replacement {
            Some(MutableReplace::Replaced(location)) => location,
            Some(MutableReplace::NotFound | MutableReplace::NoSpace) | None => {
                self.append_with_retry(record, true).await?
            }
        };
        if let Some(previous) = &previous {
            if !self
                .table
                .replace_location(&hashed_key, previous.location, location)
            {
                return Err(KvError::Corrupt(
                    "updated key is missing from the Table".into(),
                ));
            }
        } else {
            self.table.insert(&hashed_key, location)?;
        }
        if previous
            .as_ref()
            .is_some_and(|previous| previous.location.is_blob())
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

        if previous
            .as_ref()
            .is_some_and(|previous| !previous.location.is_blob())
        {
            self.write_sg_tombstone(hashed_key).await?;
        }

        let location = TableLocation::blob();
        if let Some(previous) = &previous {
            if !self
                .table
                .replace_location(&hashed_key, previous.location, location)
            {
                return Err(KvError::Corrupt(
                    "updated key is missing from the Table".into(),
                ));
            }
        } else {
            self.table.insert(&hashed_key, location)?;
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
        if previous.location.is_blob() {
            let removed = self.table.remove(&hashed_key, previous.location);
            debug_assert!(removed);
            self.blob_refs.remove(&hashed_key);
            return Ok(true);
        }
        self.write_sg_tombstone(hashed_key).await?;
        let removed = self.table.remove(&hashed_key, previous.location);
        debug_assert!(removed);
        Ok(true)
    }

    async fn write_sg_tombstone(&mut self, hashed_key: [u8; HASHED_KEY_BYTES]) -> Result<()> {
        let tombstone = Record {
            kind: RECORD_DELETE,
            page_choice: 0,
            key: hashed_key,
            value: Vec::new(),
        };
        let replacement = self
            .active
            .as_mut()
            .map(|active| active.replace(&hashed_key, tombstone.clone(), true));
        match replacement {
            Some(MutableReplace::Replaced(_)) => {}
            Some(MutableReplace::NotFound | MutableReplace::NoSpace) | None => {
                self.append_with_retry(tombstone, true).await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn sync(&mut self) -> Result<()> {
        self.blob_segment.sync().await?;
        let checkpointed_by_flush = self.active.is_some() && self.config.checkpoint_on_sg_flush;
        self.flush_active().await?;
        if !checkpointed_by_flush {
            self.save_checkpoint().await?;
        }
        Ok(())
    }

    async fn locate(&self, hash: &[u8; 32]) -> Result<Option<LocatedItem>> {
        let mut latest: Option<LocatedItem> = None;
        for location in self.table.candidates(hash) {
            if location.is_blob() {
                let Some(blob_ref) = self.blob_refs.get(hash).copied() else {
                    continue;
                };
                let value = self.blob_segment.read(hash, blob_ref).await?;
                self.io
                    .data_read
                    .set(self.io.data_read.get() + BLOB_HASHED_KEY_BYTES + blob_ref.value_len);
                latest = Some(LocatedItem {
                    location,
                    generation: u64::MAX,
                    record: Record {
                        kind: RECORD_SET,
                        page_choice: 0,
                        key: *hash,
                        value,
                    },
                });
                continue;
            }
            let active = self
                .active
                .as_ref()
                .filter(|active| active.region == location.sg_index as usize);
            let (generation, record) = if let Some(active) = active {
                (
                    active.generation,
                    active.find(hash, location.bucket_hash_index),
                )
            } else {
                let Some(generation) = self.slot_generations[location.sg_index as usize] else {
                    continue;
                };
                (generation, self.read_location(hash, location).await?)
            };
            if let Some(record) = record
                && latest
                    .as_ref()
                    .is_none_or(|current| generation > current.generation)
            {
                latest = Some(LocatedItem {
                    location,
                    generation,
                    record,
                });
            }
        }
        Ok(latest)
    }

    async fn read_location(
        &self,
        hash: &[u8; 32],
        location: TableLocation,
    ) -> Result<Option<Record>> {
        if self.slot_generations[location.sg_index as usize].is_none() {
            return Ok(None);
        }
        let page = page_hash(hash, location.bucket_hash_index, self.config.page_count());
        let bytes = self.read_page(location.sg_index as usize, page).await?;
        let mut record = latest_in_page(&bytes, hash);
        if let Some(record) = &mut record {
            record.page_choice = location.bucket_hash_index;
        }
        Ok(record)
    }

    async fn append_with_retry(
        &mut self,
        record: Record,
        count_logical: bool,
    ) -> Result<TableLocation> {
        loop {
            self.ensure_active().await?;
            if let Some(location) = self
                .active
                .as_mut()
                .unwrap()
                .append(record.clone(), count_logical)
            {
                return Ok(location);
            }
            self.flush_active().await?;
        }
    }

    async fn ensure_active(&mut self) -> Result<()> {
        if self.active.is_some() {
            return Ok(());
        }
        let region = self.next_slot;
        if self.slot_generations[region].is_some() {
            self.evict_region(region).await?;
        }
        let generation = self.next_generation;
        self.next_generation += 1;
        self.active = Some(MutableSg::new(&self.config, region, generation));
        Ok(())
    }

    async fn flush_active(&mut self) -> Result<()> {
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        if active.record_count == 0 {
            return Ok(());
        }
        active.finalize();
        let offset = active.region as u64 * self.config.sg_size as u64;
        let bytes = active.bytes;
        let write = self.data.write_all_at(bytes, offset);
        let BufResult(result, bytes) = compio::runtime::time::timeout(
            Duration::from_micros(self.config.write_max_time_us),
            write,
        )
        .await
        .map_err(|_| KvError::Timeout("SG write"))?;
        result?;
        self.io
            .data_written
            .set(self.io.data_written.get() + bytes.len() as u64);
        self.data.sync_data().await?;
        self.slot_generations[active.region] = Some(active.generation);
        self.next_slot = (active.region + 1) % self.config.sg_count;
        self.data_flushes += 1;
        if self.config.checkpoint_on_sg_flush {
            self.save_checkpoint().await?;
        }
        Ok(())
    }
}
