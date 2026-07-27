//! Core cache engine and its Segment lifecycle.
//!
//! The in-memory Table is the only source of logical liveness. Deletes remove
//! an active Item physically; Items already on SSD remain stale until their
//! Segment is reused.

use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::time::Duration;

use compio::BufResult;
use compio::buf::{IoBuf, IoBufMut, SetLen};
use compio::fs::{File, OpenOptions};
use compio::io::AsyncWriteAt;
use futures_util::stream::{FuturesUnordered, StreamExt};

use crate::*;

mod blob;
mod bucket;
mod segment_io;

pub(crate) use self::blob::*;
pub(crate) use self::bucket::*;

#[repr(C, align(4096))]
#[derive(Clone, Debug, Eq, PartialEq)]
struct DirectIoPage([u8; BUCKET_BYTES]);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirectIoBuffer {
    pages: Vec<DirectIoPage>,
    initialized_len: usize,
}

impl DirectIoBuffer {
    pub(crate) fn zeroed(len: usize) -> Self {
        assert!(len > 0 && len.is_multiple_of(BUCKET_BYTES));
        Self {
            pages: (0..len / BUCKET_BYTES)
                .map(|_| DirectIoPage([0; BUCKET_BYTES]))
                .collect(),
            initialized_len: len,
        }
    }

    pub(crate) fn for_read(len: usize) -> Self {
        let mut buffer = Self::zeroed(len);
        buffer.initialized_len = 0;
        buffer
    }

    fn capacity(&self) -> usize {
        self.pages.len() * BUCKET_BYTES
    }

    fn as_ptr(&self) -> *const u8 {
        self.pages.as_ptr().cast()
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.pages.as_mut_ptr().cast()
    }
}

impl Deref for DirectIoBuffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        // SAFETY: every DirectIoPage byte is initialized when allocated, and
        // initialized_len never exceeds the allocation.
        unsafe { std::slice::from_raw_parts(self.as_ptr(), self.initialized_len) }
    }
}

impl DerefMut for DirectIoBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: the allocation is exclusively borrowed and initialized_len
        // never exceeds its capacity.
        unsafe { std::slice::from_raw_parts_mut(self.as_mut_ptr(), self.initialized_len) }
    }
}

impl IoBuf for DirectIoBuffer {
    fn as_init(&self) -> &[u8] {
        self
    }
}

impl IoBufMut for DirectIoBuffer {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let capacity = self.capacity();
        // SAFETY: the contiguous page allocation contains capacity bytes.
        // Treating initialized bytes as MaybeUninit is permitted.
        unsafe {
            std::slice::from_raw_parts_mut(self.as_mut_ptr().cast::<MaybeUninit<u8>>(), capacity)
        }
    }
}

impl SetLen for DirectIoBuffer {
    unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= self.capacity());
        self.initialized_len = len;
    }
}

pub(crate) async fn open_direct_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .custom_flags(libc::O_DIRECT)
        .open(path)
        .await
}

pub(crate) fn require_complete_direct_io(
    operation: &str,
    completed: usize,
    expected: usize,
) -> Result<()> {
    if completed != expected {
        return Err(KvError::Worker(format!(
            "short direct {operation}: completed {completed} of {expected} bytes"
        )));
    }
    Ok(())
}

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
    pub(crate) blob_refs: HashMap<StorageKey, BlobRef>,
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
        let data = open_direct_file(&config.data_path).await?;
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

    pub(crate) async fn get(&self, storage_key: &StorageKey) -> Result<Option<Vec<u8>>> {
        Ok(self
            .locate(storage_key)
            .await?
            .map(|located| located.item.value))
    }

    pub(crate) async fn get_many(
        &self,
        storage_keys: Vec<StorageKey>,
    ) -> Vec<Result<Option<Vec<u8>>>> {
        let count = storage_keys.len();
        let mut pending = FuturesUnordered::new();
        for (index, storage_key) in storage_keys.into_iter().enumerate() {
            pending.push(async move { (index, self.get(&storage_key).await) });
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

    pub(crate) async fn set(
        &mut self,
        storage_key: StorageKey,
        value: &[u8],
    ) -> Result<SetOutcome> {
        if is_blob_item(value) {
            return self.set_blob(storage_key, value).await;
        }
        self.set_bucket_item(storage_key, value).await
    }

    async fn set_bucket_item(
        &mut self,
        storage_key: StorageKey,
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
        let previous = self.locate(&storage_key).await?;
        let item = Item {
            storage_key,
            value: value.to_vec(),
        };
        let replacement = self
            .active
            .as_mut()
            .map(|active| active.replace(&storage_key, item.clone(), true));
        let table_location = match replacement {
            Some(MutableItemReplace::Replaced(table_location)) => table_location,
            Some(MutableItemReplace::NotFound | MutableItemReplace::NoSpace) | None => {
                self.append_with_retry(item, true).await?
            }
        };
        if let Some(previous) = &previous {
            if !self
                .table
                .replace_location(&storage_key, previous.table_location, table_location)
            {
                return Err(KvError::Worker(
                    "updated key is missing from the Table".into(),
                ));
            }
        } else {
            self.table.insert(&storage_key, table_location)?;
        }
        if previous
            .as_ref()
            .is_some_and(|previous| previous.table_location.is_blob())
        {
            self.blob_refs.remove(&storage_key);
        }
        Ok(if previous.is_some() {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        })
    }

    async fn set_blob(&mut self, storage_key: StorageKey, value: &[u8]) -> Result<SetOutcome> {
        let previous = self.locate(&storage_key).await?;
        let blob_ref = self.blob_segment.append(&storage_key, value).await?;
        self.io
            .data_written
            .set(self.io.data_written.get() + blob_ref.extent_len);

        let table_location = TableLocation::blob();
        if let Some(previous) = &previous {
            if !self
                .table
                .replace_location(&storage_key, previous.table_location, table_location)
            {
                return Err(KvError::Worker(
                    "updated key is missing from the Table".into(),
                ));
            }
        } else {
            self.table.insert(&storage_key, table_location)?;
        }
        if let Some(previous) = &previous
            && !previous.table_location.is_blob()
            && let Some(active) = self.active.as_mut()
            && active.sg_index == previous.table_location.sg_index as usize
        {
            let removed = active.remove(&storage_key);
            debug_assert!(removed);
        }
        self.blob_refs.insert(storage_key, blob_ref);
        Ok(if previous.is_some() {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        })
    }

    pub(crate) async fn delete(&mut self, storage_key: &StorageKey) -> Result<bool> {
        let Some(previous) = self.locate(storage_key).await? else {
            return Ok(false);
        };
        if previous.table_location.is_blob() {
            let removed = self.table.remove(storage_key, previous.table_location);
            debug_assert!(removed);
            self.blob_refs.remove(storage_key);
            return Ok(true);
        }
        if let Some(active) = self.active.as_mut()
            && active.sg_index == previous.table_location.sg_index as usize
        {
            let removed = active.remove(storage_key);
            debug_assert!(removed);
        }
        let removed = self.table.remove(storage_key, previous.table_location);
        debug_assert!(removed);
        Ok(true)
    }

    pub(crate) async fn sync(&mut self) -> Result<()> {
        self.blob_segment.sync().await?;
        self.flush_active_segment().await
    }

    async fn locate(&self, storage_key: &StorageKey) -> Result<Option<LocatedItem>> {
        for table_location in self.table.candidate_locations(storage_key) {
            if table_location.is_blob() {
                let Some(blob_ref) = self.blob_refs.get(storage_key).copied() else {
                    continue;
                };
                let value = self.blob_segment.read(storage_key, blob_ref).await?;
                self.io
                    .data_read
                    .set(self.io.data_read.get() + blob_ref.extent_len);
                return Ok(Some(LocatedItem {
                    table_location,
                    item: Item {
                        storage_key: *storage_key,
                        value,
                    },
                }));
            }
            let active = self
                .active
                .as_ref()
                .filter(|active| active.sg_index == table_location.sg_index as usize);
            let item = if let Some(active) = active {
                active.find(storage_key, table_location.bucket_hash_index)
            } else {
                self.read_location(storage_key, table_location).await?
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
        storage_key: &StorageKey,
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
            storage_key,
            table_location.bucket_hash_index,
            self.config.bucket_count(),
        );
        let bytes = self.read_bucket(sg_index, bucket_index).await?;
        Ok(find_item_in_bucket(&bytes, storage_key))
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
        let expected = active.bytes.len();
        let write = self.data.write_at(active.bytes, offset);
        let BufResult(result, bytes) = compio::runtime::time::timeout(
            Duration::from_micros(self.config.write_max_time_us),
            write,
        )
        .await
        .map_err(|_| KvError::Timeout("Segment write"))?;
        require_complete_direct_io("Segment write", result?, expected)?;
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
                * (std::mem::size_of::<StorageKey>() + std::mem::size_of::<BlobRef>())
            + self.occupied_segments.capacity() * std::mem::size_of::<bool>()
    }
}
