//! Core cache engine and its Segment lifecycle.
//!
//! Live Items and Tombstones form a circular append log. The active RAM
//! Segment is newest; immutable SSD records are ordered by their circular
//! distance behind the write cursor.

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
mod recovery;
mod segment_io;

pub(crate) use self::blob::*;
pub(crate) use self::bucket::*;
pub(crate) use self::recovery::*;

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
    pub(crate) active_blob: Option<MutableBlobSegment>,
    pub(crate) occupied_segments: Vec<bool>,
    pub(crate) regular_segment_occupied: Vec<bool>,
    pub(crate) blob_segment_used_bytes: Vec<usize>,
    next_segment_index: usize,
    next_generation: u64,
    storage_key_id: [u8; 16],
    pub(crate) segment_flushes: u64,
    pub(crate) segment_reuses: u64,
    io: IoCounters,
}

impl Kvkache {
    #[allow(dead_code)]
    pub(crate) async fn open(config: Config) -> Result<Self> {
        Self::open_with_storage_key_id(config, [0; 16]).await
    }

    pub(crate) async fn open_with_storage_key_id(
        config: Config,
        storage_key_id: [u8; 16],
    ) -> Result<Self> {
        config.validate()?;
        if let Some(parent) = config.data_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data_exists = config.data_path.exists();
        let blob_exists = config.blob_path().exists();
        if !data_exists && blob_exists {
            return Err(KvError::Worker(
                "Blob storage exists but the Segment file is missing".into(),
            ));
        }
        if data_exists {
            let actual = fs::metadata(&config.data_path)?.len();
            let expected = config.segment_file_bytes()?;
            if actual != expected {
                return Err(KvError::Worker(format!(
                    "Segment file has length {actual}, expected {expected}; legacy files require repopulation"
                )));
            }
        }
        if blob_exists {
            let actual = fs::metadata(config.blob_path())?.len();
            let expected = config.data_bytes();
            if actual != expected {
                return Err(KvError::Worker(format!(
                    "Blob file has length {actual}, expected {expected}"
                )));
            }
        }
        let mut data = open_direct_file(&config.data_path).await?;
        if !data_exists {
            initialize_segment_file(&mut data, &config, storage_key_id).await?;
        }
        let recovery_state = recover_state(&data, &config, storage_key_id).await?;
        if !blob_exists && !recovery_state.commits.is_empty() {
            return Err(KvError::Worker(
                "committed Segment state exists but the Blob file is missing".into(),
            ));
        }
        let blob_segment = BlobSegment::open(&config).await?;

        let mut cache = Self {
            table: Table::new(&config)?,
            blob_segment,
            blob_refs: HashMap::new(),
            active: None,
            active_blob: None,
            occupied_segments: vec![false; config.segment_count],
            regular_segment_occupied: vec![false; config.segment_count],
            blob_segment_used_bytes: vec![0; config.segment_count],
            next_segment_index: recovery_state.next_segment_index,
            next_generation: recovery_state.next_generation,
            storage_key_id,
            segment_flushes: 0,
            segment_reuses: 0,
            io: IoCounters::default(),
            config,
            data,
        };
        cache.recover(recovery_state.commits).await?;
        Ok(cache)
    }

    async fn recover(&mut self, commits: Vec<SegmentCommit>) -> Result<()> {
        let mut locations = HashMap::new();
        for commit in commits {
            self.occupied_segments[commit.sg_index] = true;
            self.regular_segment_occupied[commit.sg_index] = commit.regular_occupied;
            self.blob_segment_used_bytes[commit.sg_index] = commit.blob_logical_len;
            if commit.regular_occupied {
                for (item, table_location) in self.read_segment_items(commit.sg_index).await? {
                    self.recover_location(item.storage_key, table_location, None, &mut locations)?;
                }
            }
            if commit.blob_logical_len != 0 {
                let (blob_refs, bytes_read) = self
                    .blob_segment
                    .read_segment_refs(commit.sg_index, commit.blob_logical_len)
                    .await?;
                self.io.data_read.set(self.io.data_read.get() + bytes_read);
                for (storage_key, blob_ref) in blob_refs {
                    self.recover_location(
                        storage_key,
                        TableLocation::blob(),
                        Some(blob_ref),
                        &mut locations,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn recover_location(
        &mut self,
        storage_key: StorageKey,
        table_location: TableLocation,
        blob_ref: Option<BlobRef>,
        locations: &mut HashMap<StorageKey, TableLocation>,
    ) -> Result<()> {
        if let Some(previous) = locations.insert(storage_key, table_location) {
            if !self
                .table
                .replace_location(&storage_key, previous, table_location)
            {
                return Err(KvError::Worker(
                    "recovery could not replace a Table location".into(),
                ));
            }
        } else {
            self.table.insert(&storage_key, table_location)?;
        }
        if let Some(blob_ref) = blob_ref {
            self.blob_refs.insert(storage_key, blob_ref);
        } else {
            self.blob_refs.remove(&storage_key);
        }
        Ok(())
    }

    pub(crate) async fn get(&self, storage_key: &StorageKey) -> Result<Option<Vec<u8>>> {
        Ok(self
            .locate(storage_key)
            .await?
            .and_then(|located| (!located.item.is_tombstone).then_some(located.item.value)))
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
        let was_live = previous
            .as_ref()
            .is_some_and(|located| !located.item.is_tombstone);
        let item = Item::live(storage_key, value.to_vec());
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
                // Opening the next active Segment can reuse the Segment that held
                // `previous`, removing its Table entry before this append finishes.
                if let Err(error) = self.table.insert(&storage_key, table_location) {
                    if table_location != previous.table_location
                        && let Some(active) = self.active.as_mut()
                    {
                        active.remove(&storage_key);
                    }
                    return Err(error);
                }
            }
        } else {
            self.table.insert(&storage_key, table_location)?;
        }
        if previous
            .as_ref()
            .is_some_and(|previous| previous.table_location.is_blob())
            && let Some(blob_ref) = self.blob_refs.remove(&storage_key)
        {
            self.remove_active_blob(&storage_key, blob_ref);
        }
        Ok(if was_live {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        })
    }

    async fn set_blob(&mut self, storage_key: StorageKey, value: &[u8]) -> Result<SetOutcome> {
        let previous = self.locate(&storage_key).await?;
        let was_live = previous
            .as_ref()
            .is_some_and(|located| !located.item.is_tombstone);
        let previous_blob_ref = self.blob_refs.get(&storage_key).copied();
        let replacement = previous_blob_ref.map_or(MutableBlobReplace::NotFound, |blob_ref| {
            self.active_blob
                .as_mut()
                .map_or(MutableBlobReplace::NotFound, |active| {
                    active.replace(&storage_key, blob_ref, value)
                })
        });
        let blob_ref = match replacement {
            MutableBlobReplace::Replaced => previous_blob_ref.expect("replacement has a BlobRef"),
            MutableBlobReplace::NotFound | MutableBlobReplace::NoSpace => {
                self.append_blob_with_retry(storage_key, value).await?
            }
        };
        // A rotation while appending can convert the previous active reference
        // to a stored reference. Read it again before publishing the replacement.
        let superseded_blob_ref = self.blob_refs.get(&storage_key).copied();

        let table_location = TableLocation::blob();
        if let Some(previous) = &previous {
            if !self
                .table
                .replace_location(&storage_key, previous.table_location, table_location)
                && let Err(error) = self.table.insert(&storage_key, table_location)
            {
                if superseded_blob_ref != Some(blob_ref) {
                    self.remove_active_blob(&storage_key, blob_ref);
                }
                return Err(error);
            }
        } else {
            if let Err(error) = self.table.insert(&storage_key, table_location) {
                self.remove_active_blob(&storage_key, blob_ref);
                return Err(error);
            }
        }
        if let Some(previous) = &previous
            && !previous.table_location.is_blob()
            && let Some(active) = self.active.as_mut()
            && active.sg_index == previous.table_location.sg_index as usize
        {
            let removed = active.remove(&storage_key);
            debug_assert!(removed);
        }
        if let Some(previous_blob_ref) = superseded_blob_ref
            && previous_blob_ref != blob_ref
        {
            self.remove_active_blob(&storage_key, previous_blob_ref);
        }
        self.blob_refs.insert(storage_key, blob_ref);
        Ok(if was_live {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        })
    }

    pub(crate) async fn delete(&mut self, storage_key: &StorageKey) -> Result<bool> {
        let Some(previous) = self.locate(storage_key).await? else {
            return Ok(false);
        };
        if previous.item.is_tombstone {
            return Ok(false);
        }
        let tombstone = Item::tombstone(*storage_key);
        let replacement = self
            .active
            .as_mut()
            .map(|active| active.replace(storage_key, tombstone.clone(), false));
        let table_location = match replacement {
            Some(MutableItemReplace::Replaced(table_location)) => table_location,
            Some(MutableItemReplace::NotFound | MutableItemReplace::NoSpace) | None => {
                self.append_with_retry(tombstone, false).await?
            }
        };
        if !self
            .table
            .replace_location(storage_key, previous.table_location, table_location)
        {
            // The circular reuse needed to open this active Segment may have
            // removed `previous`; in that case the Tombstone becomes a new entry.
            if let Err(error) = self.table.insert(storage_key, table_location) {
                if table_location != previous.table_location
                    && let Some(active) = self.active.as_mut()
                {
                    active.remove(storage_key);
                }
                return Err(error);
            }
        }
        if previous.table_location.is_blob()
            && let Some(blob_ref) = self.blob_refs.remove(storage_key)
        {
            self.remove_active_blob(storage_key, blob_ref);
        }
        Ok(true)
    }

    pub(crate) async fn sync(&mut self) -> Result<()> {
        self.flush_active_segment().await
    }

    async fn locate(&self, storage_key: &StorageKey) -> Result<Option<LocatedItem>> {
        let candidates = self.table.candidate_locations(storage_key);
        if let Some(active) = &self.active {
            for table_location in candidates.iter().copied().filter(|location| {
                !location.is_blob() && location.sg_index as usize == active.sg_index
            }) {
                if let Some(item) = active.find(storage_key, table_location.bucket_hash_index) {
                    return Ok(Some(LocatedItem {
                        table_location,
                        item,
                    }));
                }
            }
        }
        if candidates.iter().any(|location| location.is_blob())
            && let Some(blob_ref) = self.blob_refs.get(storage_key).copied()
        {
            let value = match blob_ref {
                BlobRef::Active { .. } => self
                    .active_blob
                    .as_ref()
                    .ok_or_else(|| KvError::Worker("active Blob SG is missing".into()))?
                    .read(storage_key, blob_ref)?,
                BlobRef::Stored { sg_index, .. } => {
                    let (value, bytes_read) = self
                        .blob_segment
                        .read(
                            storage_key,
                            blob_ref,
                            self.blob_segment_used_bytes[sg_index as usize],
                        )
                        .await?;
                    self.io.data_read.set(self.io.data_read.get() + bytes_read);
                    value
                }
            };
            return Ok(Some(LocatedItem {
                table_location: TableLocation::blob(),
                item: Item::live(*storage_key, value),
            }));
        }
        let mut newest: Option<(usize, LocatedItem)> = None;
        for table_location in candidates.into_iter().filter(|location| {
            !location.is_blob()
                && self
                    .active
                    .as_ref()
                    .is_none_or(|active| location.sg_index as usize != active.sg_index)
        }) {
            let Some(item) = self.read_location(storage_key, table_location).await? else {
                continue;
            };
            let age = self.ssd_segment_age(table_location.sg_index as usize);
            if newest
                .as_ref()
                .is_none_or(|(newest_age, _)| age < *newest_age)
            {
                newest = Some((
                    age,
                    LocatedItem {
                        table_location,
                        item,
                    },
                ));
            }
        }
        Ok(newest.map(|(_, located)| located))
    }

    fn ssd_segment_age(&self, sg_index: usize) -> usize {
        let count = self.config.segment_count;
        let newest_ssd = (self.next_segment_index + count - 1) % count;
        (newest_ssd + count - sg_index) % count
    }

    async fn read_location(
        &self,
        storage_key: &StorageKey,
        table_location: TableLocation,
    ) -> Result<Option<Item>> {
        debug_assert!(!table_location.is_blob());
        let sg_index = table_location.sg_index as usize;
        if !self
            .regular_segment_occupied
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

    async fn append_blob_with_retry(
        &mut self,
        storage_key: StorageKey,
        value: &[u8],
    ) -> Result<BlobRef> {
        let required_bytes = BLOB_ITEM_FIXED_BYTES
            .checked_add(value.len())
            .ok_or_else(|| KvError::Usage("Blob Item length overflow".into()))?;
        if required_bytes > self.config.segment_size {
            return Err(KvError::BlobSegmentFull {
                required_bytes: required_bytes as u64,
                remaining_bytes: self.config.segment_size as u64,
            });
        }
        loop {
            self.ensure_active_segment().await?;
            if let Some(blob_ref) = self
                .active_blob
                .as_mut()
                .expect("active Blob SG accompanies active Bucket SG")
                .append(storage_key, value)
            {
                return Ok(blob_ref);
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
            let bytes_written = invalidate_segment(&mut self.data, &self.config, sg_index).await?;
            self.io
                .data_written
                .set(self.io.data_written.get() + bytes_written);
            self.prepare_segment_for_reuse(sg_index).await?;
        }
        self.active = Some(MutableSegment::new(&self.config, sg_index));
        self.active_blob = Some(MutableBlobSegment::new(&self.config, sg_index));
        Ok(())
    }

    async fn flush_active_segment(&mut self) -> Result<()> {
        let Some(active) = self.active.take() else {
            debug_assert!(self.active_blob.is_none());
            return Ok(());
        };
        let active_blob = self
            .active_blob
            .take()
            .expect("active Blob SG accompanies active Bucket SG");
        debug_assert_eq!(active.sg_index, active_blob.sg_index);
        if active.item_count == 0 && active_blob.is_empty() {
            return Ok(());
        }
        let sg_index = active.sg_index;
        let regular_occupied = active.item_count != 0;
        if regular_occupied {
            let offset = self.config.segment_data_offset(sg_index);
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
        }

        let encoded_blob = active_blob.encode()?;
        let blob_logical_len = encoded_blob
            .as_ref()
            .map_or(0, |encoded| encoded.logical_len);
        if let Some(encoded_blob) = encoded_blob {
            let bytes_written = self
                .blob_segment
                .write_segment(sg_index, encoded_blob.bytes)
                .await?;
            self.io
                .data_written
                .set(self.io.data_written.get() + bytes_written);
            self.blob_segment.sync().await?;
            for (storage_key, active_ref, stored_ref) in encoded_blob.refs {
                if self.blob_refs.get(&storage_key) == Some(&active_ref) {
                    self.blob_refs.insert(storage_key, stored_ref);
                }
            }
        }
        let control_bytes = commit_segment(
            &mut self.data,
            &self.config,
            self.storage_key_id,
            SegmentCommit {
                sg_index,
                generation: self.next_generation,
                regular_occupied,
                blob_logical_len,
            },
        )
        .await?;
        self.io
            .data_written
            .set(self.io.data_written.get() + control_bytes);
        self.regular_segment_occupied[sg_index] = regular_occupied;
        self.blob_segment_used_bytes[sg_index] = blob_logical_len;
        self.occupied_segments[sg_index] = true;
        self.next_segment_index = (sg_index + 1) % self.config.segment_count;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or_else(|| KvError::Worker("storage state generation is exhausted".into()))?;
        self.segment_flushes += 1;
        Ok(())
    }

    fn remove_active_blob(&mut self, storage_key: &StorageKey, blob_ref: BlobRef) {
        if let Some(active_blob) = self.active_blob.as_mut() {
            active_blob.remove(storage_key, blob_ref);
        }
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
            self.blob_used_bytes(),
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
            + self.config.segment_size * 2
            + self.blob_refs.capacity()
                * (std::mem::size_of::<StorageKey>() + std::mem::size_of::<BlobRef>())
            + self.occupied_segments.capacity() * std::mem::size_of::<bool>()
            + self.regular_segment_occupied.capacity() * std::mem::size_of::<bool>()
            + self.blob_segment_used_bytes.capacity() * std::mem::size_of::<usize>()
    }

    fn blob_used_bytes(&self) -> u64 {
        let stored = self
            .blob_segment_used_bytes
            .iter()
            .copied()
            .map(|len| len.next_multiple_of(BUCKET_BYTES) as u64)
            .sum::<u64>();
        let active = self.active_blob.as_ref().map_or(0, |active| {
            active.logical_len().next_multiple_of(BUCKET_BYTES) as u64
        });
        stored + active
    }
}
