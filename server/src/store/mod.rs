//! Core cache engine and its Segment lifecycle.
//!
//! A bounded pending map coalesces the latest writes before flush. Live Items
//! and Tombstones form a circular SG log; each SG is reused with its paired
//! dense Blob generation.

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

use crate::types::EncodedValue;
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
    pub(crate) blob_data_written: u64,
    pub(crate) blob_data_read: u64,
}

#[derive(Default)]
struct IoCounters {
    data_written: Cell<u64>,
    data_read: Cell<u64>,
    blob_data_written: Cell<u64>,
    blob_data_read: Cell<u64>,
}

#[derive(Clone, Copy)]
enum SegmentFlushReason {
    Capacity,
    Sync,
}

struct LocatedItem {
    table_location: TableLocation,
    item: Item,
}

#[derive(Debug)]
pub(crate) struct PendingItem {
    pub(crate) value: Option<EncodedValue>,
    pub(crate) previous: Option<TableLocation>,
    pub(crate) previous_live: bool,
}

struct FlushRecord {
    storage_key: StorageKey,
    value: Option<EncodedValue>,
    previous: Option<TableLocation>,
    previous_live: bool,
    table_location: Option<TableLocation>,
    blob_ref: Option<BlobRef>,
}

pub(crate) struct Kvkache {
    config: Config,
    data: File,
    pub(crate) table: Table,
    pub(crate) blob_segment: BlobSegment,
    pub(crate) pending: HashMap<StorageKey, PendingItem>,
    pub(crate) pending_sg_bytes: usize,
    pub(crate) pending_blob_bytes: usize,
    pub(crate) occupied_segments: Vec<bool>,
    stable_live_keys: usize,
    next_segment_index: usize,
    next_generation: u64,
    storage_key_id: [u8; 16],
    pub(crate) segment_flushes: u64,
    pub(crate) segment_capacity_flushes: u64,
    pub(crate) segment_sync_flushes: u64,
    pub(crate) segment_fill_used_bytes: u64,
    pub(crate) segment_fill_capacity_bytes: u64,
    pub(crate) segment_capacity_fill_used_bytes: u64,
    pub(crate) segment_capacity_fill_capacity_bytes: u64,
    pub(crate) segment_sync_fill_used_bytes: u64,
    pub(crate) segment_sync_fill_capacity_bytes: u64,
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
            let expected = config.blob_bytes();
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
            pending: HashMap::new(),
            pending_sg_bytes: 0,
            pending_blob_bytes: 0,
            occupied_segments: vec![false; config.segment_count],
            stable_live_keys: 0,
            next_segment_index: recovery_state.next_segment_index,
            next_generation: recovery_state.next_generation,
            storage_key_id,
            segment_flushes: 0,
            segment_capacity_flushes: 0,
            segment_sync_flushes: 0,
            segment_fill_used_bytes: 0,
            segment_fill_capacity_bytes: 0,
            segment_capacity_fill_used_bytes: 0,
            segment_capacity_fill_capacity_bytes: 0,
            segment_sync_fill_used_bytes: 0,
            segment_sync_fill_capacity_bytes: 0,
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
            self.blob_segment
                .recover_segment(commit.sg_index, commit.blob_logical_len)?;
            for (item, table_location) in self.read_segment_items(commit.sg_index).await? {
                if !item.is_tombstone
                    && let StoredValue::Blob(blob_ref) = decode_stored_value(&item.value)?.value
                {
                    validate_recovered_blob_ref(blob_ref, commit.blob_logical_len)?;
                }
                let new_live = !item.is_tombstone;
                if let Some((previous, previous_live)) =
                    locations.insert(item.storage_key, (table_location, new_live))
                {
                    if !self
                        .table
                        .replace_location(&item.storage_key, previous, table_location)
                    {
                        return Err(KvError::Worker(
                            "recovery could not replace a Table location".into(),
                        ));
                    }
                    match (previous_live, new_live) {
                        (true, false) => {
                            self.stable_live_keys = self.stable_live_keys.saturating_sub(1);
                        }
                        (false, true) => self.stable_live_keys += 1,
                        _ => {}
                    }
                } else {
                    self.table.insert(&item.storage_key, table_location)?;
                    if new_live {
                        self.stable_live_keys += 1;
                    }
                }
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) async fn get(&self, storage_key: &StorageKey) -> Result<Option<Vec<u8>>> {
        Ok(self
            .get_encoded(storage_key)
            .await?
            .map(|value| value.bytes))
    }

    pub(crate) async fn get_encoded(
        &self,
        storage_key: &StorageKey,
    ) -> Result<Option<EncodedValue>> {
        if let Some(pending) = self.pending.get(storage_key) {
            return Ok(pending.value.clone());
        }
        let Some(located) = self.locate_stable_record(storage_key).await? else {
            return Ok(None);
        };
        if located.item.is_tombstone {
            return Ok(None);
        }
        self.read_stored_value(located.table_location, &located.item.value)
            .await
            .map(Some)
    }

    pub(crate) async fn get_many_encoded(
        &self,
        storage_keys: Vec<StorageKey>,
    ) -> Vec<Result<Option<EncodedValue>>> {
        let count = storage_keys.len();
        let mut pending = FuturesUnordered::new();
        for (index, storage_key) in storage_keys.into_iter().enumerate() {
            pending.push(async move { (index, self.get_encoded(&storage_key).await) });
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

    #[allow(dead_code)]
    pub(crate) async fn set(
        &mut self,
        storage_key: StorageKey,
        value: &[u8],
    ) -> Result<SetOutcome> {
        self.set_encoded(storage_key, EncodedValue::plain(value.to_vec()))
            .await
    }

    pub(crate) async fn set_encoded(
        &mut self,
        storage_key: StorageKey,
        value: EncodedValue,
    ) -> Result<SetOutcome> {
        self.validate_value(&value.bytes)?;
        let (previous, previous_live, outcome) =
            if let Some(pending) = self.take_pending(&storage_key) {
                let outcome = if pending.value.is_some() {
                    SetOutcome::Replaced
                } else {
                    SetOutcome::Created
                };
                (pending.previous, pending.previous_live, outcome)
            } else {
                let previous = self.locate_stable_record(&storage_key).await?;
                let previous_live = previous
                    .as_ref()
                    .is_some_and(|located| !located.item.is_tombstone);
                let location = previous.map(|located| located.table_location);
                let outcome = if previous_live {
                    SetOutcome::Replaced
                } else {
                    SetOutcome::Created
                };
                (location, previous_live, outcome)
            };
        self.insert_pending(
            storage_key,
            PendingItem {
                value: Some(value),
                previous,
                previous_live,
            },
        );
        if self.pending_should_flush() {
            self.flush_pending(SegmentFlushReason::Capacity).await?;
        }
        Ok(outcome)
    }

    pub(crate) async fn delete(&mut self, storage_key: &StorageKey) -> Result<bool> {
        if let Some(mut pending) = self.take_pending(storage_key) {
            if pending.value.is_none() {
                self.insert_pending(*storage_key, pending);
                return Ok(false);
            }
            if pending.previous.is_some() {
                pending.value = None;
                self.insert_pending(*storage_key, pending);
            }
            if self.pending_should_flush() {
                self.flush_pending(SegmentFlushReason::Capacity).await?;
            }
            return Ok(true);
        }
        let Some(previous) = self.locate_stable_record(storage_key).await? else {
            return Ok(false);
        };
        if previous.item.is_tombstone {
            return Ok(false);
        }
        self.insert_pending(
            *storage_key,
            PendingItem {
                value: None,
                previous: Some(previous.table_location),
                previous_live: true,
            },
        );
        if self.pending_should_flush() {
            self.flush_pending(SegmentFlushReason::Capacity).await?;
        }
        Ok(true)
    }

    pub(crate) async fn sync(&mut self) -> Result<()> {
        self.flush_pending(SegmentFlushReason::Sync).await?;
        self.blob_segment.sync().await?;
        self.data.sync_data().await?;
        Ok(())
    }

    async fn locate_stable_record(&self, storage_key: &StorageKey) -> Result<Option<LocatedItem>> {
        let mut newest: Option<(usize, LocatedItem)> = None;
        for table_location in self.table.candidate_locations(storage_key) {
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

    async fn read_stored_value(
        &self,
        table_location: TableLocation,
        encoded: &[u8],
    ) -> Result<EncodedValue> {
        let decoded = decode_stored_value(encoded)?;
        let bytes = match decoded.value {
            StoredValue::Inline(value) => value.to_vec(),
            StoredValue::Blob(blob_ref) => {
                let value = self
                    .blob_segment
                    .read(table_location.sg_index as usize, blob_ref)
                    .await?;
                let physical_bytes = self.blob_segment.physical_read_bytes(blob_ref);
                self.io
                    .data_read
                    .set(self.io.data_read.get() + physical_bytes);
                self.io
                    .blob_data_read
                    .set(self.io.blob_data_read.get() + physical_bytes);
                value
            }
        };
        Ok(EncodedValue::new(bytes, decoded.flags))
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

    async fn flush_pending(&mut self, reason: SegmentFlushReason) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let pending = std::mem::take(&mut self.pending);
        self.pending_sg_bytes = 0;
        self.pending_blob_bytes = 0;
        let mut remaining = pending
            .into_iter()
            .filter_map(|(storage_key, pending)| {
                (pending.value.is_some() || pending.previous.is_some()).then_some(FlushRecord {
                    storage_key,
                    value: pending.value,
                    previous: pending.previous,
                    previous_live: pending.previous_live,
                    table_location: None,
                    blob_ref: None,
                })
            })
            .collect::<Vec<_>>();
        remaining.sort_unstable_by(|left, right| {
            right
                .value
                .as_ref()
                .map_or(0, |value| value.bytes.len())
                .cmp(&left.value.as_ref().map_or(0, |value| value.bytes.len()))
                .then_with(|| left.storage_key.cmp(&right.storage_key))
        });

        while !remaining.is_empty() {
            let next_generation = match next_sg_generation(self.next_generation) {
                Ok(next_generation) => next_generation,
                Err(error) => {
                    self.restore_flush_records(remaining);
                    return Err(error);
                }
            };
            let sg_index = self.next_segment_index;
            let mut active = MutableSegment::new(&self.config, sg_index);
            let mut planned = Vec::new();
            let mut deferred = Vec::new();
            let mut blob_used = 0usize;

            for mut record in remaining {
                let (item, blob_ref) = match record.value.as_ref() {
                    None => (Item::tombstone(record.storage_key), None),
                    Some(value) if is_blob_item(&value.bytes) => {
                        let Some(blob_end) = blob_used.checked_add(value.bytes.len()) else {
                            deferred.push(record);
                            continue;
                        };
                        if blob_end > self.config.blob_segment_size {
                            deferred.push(record);
                            continue;
                        }
                        let blob_ref = BlobRef::new(blob_used, value.bytes.len())?;
                        (
                            Item::live(record.storage_key, encode_blob_ref(blob_ref, value.flags)),
                            Some(blob_ref),
                        )
                    }
                    Some(value) => (
                        Item::live(
                            record.storage_key,
                            encode_inline_value(&value.bytes, value.flags),
                        ),
                        None,
                    ),
                };
                if let Some(table_location) = active.append(item, false) {
                    if blob_ref.is_some() {
                        blob_used += record
                            .value
                            .as_ref()
                            .expect("Blob record has a live value")
                            .bytes
                            .len();
                    }
                    if let Some(value) = &record.value {
                        active.accepted_item_bytes +=
                            (crate::types::STORAGE_KEY_BYTES + value.bytes.len()) as u64;
                    }
                    record.table_location = Some(table_location);
                    record.blob_ref = blob_ref;
                    planned.push(record);
                } else {
                    deferred.push(record);
                }
            }

            if planned.is_empty() {
                self.restore_flush_records(deferred);
                return Err(KvError::Worker(
                    "pending Item cannot fit in an empty Segment generation".into(),
                ));
            }
            if self.occupied_segments[sg_index] {
                let invalidated =
                    match invalidate_segment(&mut self.data, &self.config, sg_index).await {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            self.restore_flush_records(planned.into_iter().chain(deferred));
                            return Err(error);
                        }
                    };
                self.io
                    .data_written
                    .set(self.io.data_written.get() + invalidated);
                let evicted = match self.prepare_segment_for_reuse(sg_index).await {
                    Ok(evicted) => evicted,
                    Err(error) => {
                        self.restore_flush_records(planned.into_iter().chain(deferred));
                        return Err(error);
                    }
                };
                if !evicted.is_empty() {
                    for record in planned.iter_mut().chain(&mut deferred) {
                        if evicted.contains(&record.storage_key) {
                            record.previous = None;
                            record.previous_live = false;
                        }
                    }
                }
            }

            let blob_values = planned
                .iter()
                .filter(|record| record.blob_ref.is_some())
                .map(|record| {
                    record
                        .value
                        .as_ref()
                        .expect("Blob record has a live value")
                        .bytes
                        .as_slice()
                })
                .collect::<Vec<_>>();
            let blob_physical_bytes = match self
                .blob_segment
                .write_segment(sg_index, &blob_values)
                .await
            {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.restore_flush_records(planned.into_iter().chain(deferred));
                    return Err(error);
                }
            };
            self.io
                .data_written
                .set(self.io.data_written.get() + blob_physical_bytes);
            self.io
                .blob_data_written
                .set(self.io.blob_data_written.get() + blob_physical_bytes);

            if let Err(error) = self
                .write_segment(active, reason, blob_used, next_generation)
                .await
            {
                self.restore_flush_records(planned.into_iter().chain(deferred));
                return Err(error);
            }

            let mut published = 0usize;
            let mut publish_error = None;
            for record in &planned {
                let table_location = record.table_location.unwrap();
                let new_live = record.value.is_some();
                let replaced = record.previous.is_some_and(|previous| {
                    self.table
                        .replace_location(&record.storage_key, previous, table_location)
                });
                if replaced {
                    match (record.previous_live, new_live) {
                        (true, false) => {
                            self.stable_live_keys = self.stable_live_keys.saturating_sub(1);
                        }
                        (false, true) => self.stable_live_keys += 1,
                        _ => {}
                    }
                } else if let Err(error) = self.table.insert(&record.storage_key, table_location) {
                    publish_error = Some(error);
                    break;
                } else if new_live {
                    self.stable_live_keys += 1;
                }
                published += 1;
            }
            if let Some(error) = publish_error {
                self.restore_flush_records(planned.into_iter().skip(published).chain(deferred));
                return Err(error);
            }
            remaining = deferred;
        }
        Ok(())
    }

    async fn write_segment(
        &mut self,
        active: MutableSegment,
        reason: SegmentFlushReason,
        blob_logical_len: usize,
        next_generation: u64,
    ) -> Result<()> {
        let fill_used_bytes = active.used_bytes() as u64;
        let offset = self.config.segment_data_offset(active.sg_index);
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
        self.blob_segment.sync().await?;
        self.data.sync_data().await?;
        let control_bytes = commit_segment(
            &mut self.data,
            &self.config,
            self.storage_key_id,
            SegmentCommit {
                sg_index: active.sg_index,
                generation: self.next_generation,
                blob_logical_len,
            },
        )
        .await?;
        self.io
            .data_written
            .set(self.io.data_written.get() + control_bytes);
        self.occupied_segments[active.sg_index] = true;
        self.next_segment_index = (active.sg_index + 1) % self.config.segment_count;
        self.next_generation = next_generation;
        self.segment_flushes += 1;
        match reason {
            SegmentFlushReason::Capacity => {
                self.segment_capacity_flushes += 1;
                self.segment_capacity_fill_used_bytes += fill_used_bytes;
                self.segment_capacity_fill_capacity_bytes += self.config.segment_size as u64;
            }
            SegmentFlushReason::Sync => {
                self.segment_sync_flushes += 1;
                self.segment_sync_fill_used_bytes += fill_used_bytes;
                self.segment_sync_fill_capacity_bytes += self.config.segment_size as u64;
            }
        }
        self.segment_fill_used_bytes += fill_used_bytes;
        self.segment_fill_capacity_bytes += self.config.segment_size as u64;
        Ok(())
    }

    fn validate_value(&self, value: &[u8]) -> Result<()> {
        if is_blob_item(value) {
            if value.len() > self.config.blob_segment_size || value.len() > u32::MAX as usize {
                return Err(KvError::BlobSegmentFull {
                    required_bytes: value.len() as u64,
                    remaining_bytes: self.config.blob_segment_size as u64,
                });
            }
            return Ok(());
        }
        let item_bytes =
            item_offsets_bytes(1) + ITEM_FIXED_BYTES + STORED_VALUE_TAG_BYTES + value.len();
        let capacity = BUCKET_BYTES - 1;
        if item_bytes > capacity {
            return Err(KvError::ItemTooLarge {
                bytes: item_bytes,
                capacity,
            });
        }
        Ok(())
    }

    fn insert_pending(&mut self, storage_key: StorageKey, pending: PendingItem) {
        let (sg_bytes, blob_bytes) =
            pending_accounted_bytes(pending.value.as_ref().map(|value| value.bytes.as_slice()));
        self.pending_sg_bytes = self.pending_sg_bytes.saturating_add(sg_bytes);
        self.pending_blob_bytes = self.pending_blob_bytes.saturating_add(blob_bytes);
        let replaced = self.pending.insert(storage_key, pending);
        debug_assert!(replaced.is_none());
    }

    fn take_pending(&mut self, storage_key: &StorageKey) -> Option<PendingItem> {
        let pending = self.pending.remove(storage_key)?;
        let (sg_bytes, blob_bytes) =
            pending_accounted_bytes(pending.value.as_ref().map(|value| value.bytes.as_slice()));
        self.pending_sg_bytes = self.pending_sg_bytes.saturating_sub(sg_bytes);
        self.pending_blob_bytes = self.pending_blob_bytes.saturating_sub(blob_bytes);
        Some(pending)
    }

    fn pending_should_flush(&self) -> bool {
        self.pending_sg_bytes >= self.config.segment_size
            || self.pending_blob_bytes >= self.config.blob_segment_size
    }

    fn restore_flush_records<I>(&mut self, records: I)
    where
        I: IntoIterator<Item = FlushRecord>,
    {
        for record in records {
            self.insert_pending(
                record.storage_key,
                PendingItem {
                    value: record.value,
                    previous: record.previous,
                    previous_live: record.previous_live,
                },
            );
        }
    }

    pub(crate) fn stats(&self) -> String {
        let io = self.io_stats();
        let segment_fill_percent = self.segment_fill_used_bytes as f64 * 100.0
            / self.segment_fill_capacity_bytes.max(1) as f64;
        let segment_capacity_fill_percent = self.segment_capacity_fill_used_bytes as f64 * 100.0
            / self.segment_capacity_fill_capacity_bytes.max(1) as f64;
        let segment_sync_fill_percent = self.segment_sync_fill_used_bytes as f64 * 100.0
            / self.segment_sync_fill_capacity_bytes.max(1) as f64;
        format!(
            "keys={} stable_keys={} pending_items={} pending_value_bytes={} table_load={:.2}% table_memory={:.2}MiB ({:.3}B/planned-key) modeled_resident={:.2}MiB front_subtables={} front_capacity={} back_subtables={} back_capacity={} bucket_choices={} bucket_selection={} blob_used={} blob_logical_used={} blob_capacity={} next_segment_index={} occupied_segments={} flushes={} capacity_flushes={} sync_flushes={} segment_reuses={} sg_fill_percent={:.3}% sg_fill_used_bytes={} sg_fill_capacity_bytes={} capacity_sg_fill_percent={:.3}% capacity_sg_fill_used_bytes={} capacity_sg_fill_capacity_bytes={} sync_sg_fill_percent={:.3}% sync_sg_fill_used_bytes={} sync_sg_fill_capacity_bytes={} data_read={} data_written={} blob_data_read={} blob_data_written={}",
            self.logical_key_count(),
            self.stable_live_keys,
            self.pending.len(),
            self.pending_value_bytes(),
            self.table.load_factor() * 100.0,
            self.table.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.table.memory_bytes() as f64 / self.config.table_capacity as f64,
            self.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.table.front_table.len(),
            self.table.front_subtable_layout.entry_capacity,
            self.table.back_table.len(),
            self.table.back_subtable_layout.entry_capacity,
            self.config.bucket_choice_count,
            self.config.bucket_selection_policy.as_str(),
            self.blob_segment.used_bytes(),
            self.blob_segment.logical_used_bytes(),
            self.blob_segment.capacity_bytes(),
            self.next_segment_index,
            self.occupied_segments
                .iter()
                .filter(|value| **value)
                .count(),
            self.segment_flushes,
            self.segment_capacity_flushes,
            self.segment_sync_flushes,
            self.segment_reuses,
            segment_fill_percent,
            self.segment_fill_used_bytes,
            self.segment_fill_capacity_bytes,
            segment_capacity_fill_percent,
            self.segment_capacity_fill_used_bytes,
            self.segment_capacity_fill_capacity_bytes,
            segment_sync_fill_percent,
            self.segment_sync_fill_used_bytes,
            self.segment_sync_fill_capacity_bytes,
            io.data_read,
            io.data_written,
            io.blob_data_read,
            io.blob_data_written,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn reset_io_stats(&self) {
        self.io.data_written.set(0);
        self.io.data_read.set(0);
        self.io.blob_data_written.set(0);
        self.io.blob_data_read.set(0);
    }

    pub(super) fn io_stats(&self) -> KvkacheIoStats {
        KvkacheIoStats {
            data_written: self.io.data_written.get(),
            data_read: self.io.data_read.get(),
            blob_data_written: self.io.blob_data_written.get(),
            blob_data_read: self.io.blob_data_read.get(),
        }
    }

    pub(super) fn memory_bytes(&self) -> usize {
        self.table.memory_bytes()
            + self.pending.capacity()
                * (std::mem::size_of::<StorageKey>() + std::mem::size_of::<PendingItem>())
            + self.pending_value_bytes()
            + self.occupied_segments.capacity() * std::mem::size_of::<bool>()
            + self.blob_segment.memory_bytes()
    }

    fn logical_key_count(&self) -> usize {
        let mut count = self.stable_live_keys;
        for pending in self.pending.values() {
            match (pending.previous_live, pending.value.is_some()) {
                (true, false) => count = count.saturating_sub(1),
                (false, true) => count += 1,
                _ => {}
            }
        }
        count
    }

    fn pending_value_bytes(&self) -> usize {
        self.pending
            .values()
            .filter_map(|pending| pending.value.as_ref())
            .map(|value| value.bytes.capacity())
            .sum()
    }
}

pub(crate) fn validate_recovered_blob_ref(
    blob_ref: BlobRef,
    blob_logical_len: usize,
) -> Result<()> {
    let value_end = (blob_ref.value_offset as usize)
        .checked_add(blob_ref.value_len as usize)
        .ok_or_else(|| KvError::Worker("recovered BlobRef range overflow".into()))?;
    if blob_ref.value_len == 0 || value_end > blob_logical_len {
        return Err(KvError::Worker(
            "recovered BlobRef is invalid or exceeds the committed Blob length".into(),
        ));
    }
    Ok(())
}

fn stored_payload_len(value: &[u8]) -> usize {
    if is_blob_item(value) {
        STORED_BLOB_REF_BYTES
    } else {
        STORED_VALUE_TAG_BYTES + value.len()
    }
}

fn pending_accounted_bytes(value: Option<&[u8]>) -> (usize, usize) {
    let Some(value) = value else {
        return (item_offsets_bytes(1) + ITEM_FIXED_BYTES, 0);
    };
    let sg_bytes = item_offsets_bytes(1) + ITEM_FIXED_BYTES + stored_payload_len(value);
    let blob_bytes = if is_blob_item(value) { value.len() } else { 0 };
    (sg_bytes, blob_bytes)
}
