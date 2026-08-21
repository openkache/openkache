//! Capacity eviction scanning and generation retirement.

use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use crate::storage_runtime::File;
use crate::{BUCKET_BYTES, Config, KvError, Result, StorageKey};
use futures_util::future::FutureExt;

use super::keyed::PendingKeyedMutation;
use super::policy::{item_state_is_live_at, unix_time_ms};
use super::{
    BucketHashSequence, DirectIoBuffer, EvictableLocation, EvictionExtent, EvictionWork,
    GenerationLocation, Kvkache, TableLocation, find_item_in_bucket, items, read_exact_direct,
    storage_operation_error,
};

fn schedule_eviction_read(data: &File, config: &Config, eviction: &mut EvictionWork) {
    const EXTENT_BYTES: usize = 1024 * 1024;
    if eviction.read.is_some()
        || eviction.prefetched.is_some()
        || eviction.next_read_offset >= config.segment_size
    {
        return;
    }
    let offset = eviction.next_read_offset;
    let len = EXTENT_BYTES.min(config.segment_size - offset);
    eviction.next_read_offset += len;
    let file = data.clone();
    let file_offset = eviction.victim.sg_base + offset as u64;
    let read_max_time_us = config.read_max_time_us.max(config.write_max_time_us);
    eviction.read = Some(
        async move {
            let result = read_exact_direct(
                &file,
                DirectIoBuffer::for_read(len),
                file_offset,
                len,
                read_max_time_us,
                "eviction SG extent read",
            )
            .await;
            (offset, result)
        }
        .boxed_local(),
    );
}

fn bucket_hash_index_for_bucket(
    storage_key: &StorageKey,
    bucket_index: usize,
    bucket_count: usize,
    bucket_choice_count: usize,
) -> Option<u8> {
    let hashes = BucketHashSequence::new(storage_key, bucket_count);
    (0..bucket_choice_count as u8).find(|index| hashes.get(*index) == bucket_index)
}

impl Kvkache {
    pub(super) fn start_eviction(&mut self, victim: GenerationLocation) -> Result<()> {
        let logical_sg_id = victim.logical_sg_id;
        self.stable_ram_segments
            .retain(|cached| *cached != logical_sg_id);
        let guard = self.directory.begin_eviction(logical_sg_id)?;
        let has_large_value_extent = guard.large_value_location.is_some();
        let mut eviction = EvictionWork {
            victim,
            reader_guard: Some(guard),
            current: None,
            prefetched: None,
            read: None,
            next_read_offset: 0,
            now_ms: unix_time_ms(),
            retiring: false,
            has_large_value_extent,
            protected_item_found: false,
            evictable_items: Vec::new(),
        };
        schedule_eviction_read(&self.data, &self.config, &mut eviction);
        self.eviction = Some(eviction);
        Ok(())
    }

    pub(super) fn poll_eviction(&mut self, context: &mut Context<'_>) -> Poll<Result<bool>> {
        let Some(mut eviction) = self.eviction.take() else {
            return Poll::Pending;
        };
        if eviction.retiring {
            if !self
                .directory
                .try_free_retiring(eviction.victim.logical_sg_id)?
            {
                self.eviction = Some(eviction);
                context.waker().wake_by_ref();
                return Poll::Pending;
            }
            self.generation_log
                .release_oldest(eviction.victim.logical_sg_id)?;
            if eviction.has_large_value_extent {
                self.large_value_log
                    .release_oldest(eviction.victim.logical_sg_id)?;
            }
            self.segment_reuses += 1;
            return Poll::Ready(Ok(true));
        }

        if let Some(read) = eviction.read.as_mut()
            && let Poll::Ready((offset, result)) = read.as_mut().poll(context)
        {
            let buffer = match result {
                Ok(buffer) => buffer,
                Err(KvError::Timeout(_)) => {
                    eviction.read = None;
                    eviction.next_read_offset = offset;
                    self.eviction_read_timeouts += 1;
                    schedule_eviction_read(&self.data, &self.config, &mut eviction);
                    self.eviction = Some(eviction);
                    context.waker().wake_by_ref();
                    return Poll::Ready(Ok(true));
                }
                Err(error) => {
                    return Poll::Ready(Err(storage_operation_error(&self.resource_guard, error)));
                }
            };
            self.io
                .data_read
                .set(self.io.data_read.get() + buffer.len() as u64);
            let extent = EvictionExtent {
                offset,
                buffer,
                next_bucket: 0,
            };
            if eviction.current.is_none() {
                eviction.current = Some(extent);
            } else {
                eviction.prefetched = Some(extent);
            }
            eviction.read = None;
            schedule_eviction_read(&self.data, &self.config, &mut eviction);
        }

        if eviction.current.is_none() {
            eviction.current = eviction.prefetched.take();
        }
        schedule_eviction_read(&self.data, &self.config, &mut eviction);
        if let Some(extent) = eviction.current.as_mut() {
            let deadline = Instant::now() + Duration::from_micros(50);
            let mut cleaned = 0usize;
            while extent.next_bucket * BUCKET_BYTES < extent.buffer.len()
                && (cleaned == 0 || (cleaned < 64 && Instant::now() < deadline))
            {
                self.clean_eviction_bucket(
                    eviction.victim.logical_sg_id,
                    eviction.now_ms,
                    extent,
                    &mut eviction.protected_item_found,
                    &mut eviction.evictable_items,
                )?;
                extent.next_bucket += 1;
                cleaned += 1;
            }
            if extent.next_bucket * BUCKET_BYTES == extent.buffer.len() {
                eviction.current = eviction.prefetched.take();
            }
            self.eviction = Some(eviction);
            context.waker().wake_by_ref();
            return Poll::Ready(Ok(true));
        }

        if eviction.read.is_some() {
            self.eviction = Some(eviction);
            return Poll::Pending;
        }

        if eviction.protected_item_found {
            // Do not partially apply capacity eviction when a protected item makes
            // admission impossible. The storage admission outcome guarantees
            // that the failed SET made no mutation, including no collateral evictions.
            return Poll::Ready(Err(KvError::NoCapacity));
        }

        for candidate in eviction.evictable_items.drain(..) {
            if self
                .table
                .remove(&candidate.storage_key, candidate.table_location)
                && candidate.live
            {
                self.live_keys = self.live_keys.saturating_sub(1);
            }
            if let Some(index) = self.persistent_index.as_mut() {
                index.remove(&candidate.storage_key);
            }
        }

        self.directory
            .begin_retiring(eviction.victim.logical_sg_id)?;
        eviction.reader_guard.take();
        eviction.retiring = true;
        self.eviction = Some(eviction);
        context.waker().wake_by_ref();
        Poll::Ready(Ok(true))
    }

    fn clean_eviction_bucket(
        &mut self,
        logical_sg_id: u32,
        now_ms: u64,
        extent: &EvictionExtent,
        protected_item_found: &mut bool,
        evictable_items: &mut Vec<EvictableLocation>,
    ) -> Result<()> {
        let bucket_offset = extent.next_bucket * BUCKET_BYTES;
        let bucket_index = (extent.offset + bucket_offset) / BUCKET_BYTES;
        let bucket = &extent.buffer[bucket_offset..bucket_offset + BUCKET_BYTES];
        for item in items(bucket) {
            if find_item_in_bucket(bucket, &item.storage_key).as_ref() != Some(&item) {
                continue;
            }
            let Some(bucket_hash_index) = bucket_hash_index_for_bucket(
                &item.storage_key,
                bucket_index,
                self.config.bucket_count(),
                self.config.bucket_choice_count,
            ) else {
                continue;
            };
            let location = TableLocation {
                sg_index: logical_sg_id,
                bucket_hash_index,
            };
            // Ignore stale records that no longer back the table entry. This is important for
            // protected items that were explicitly deleted or replaced while their generation
            // was stable.
            if !self
                .table
                .candidate_locations(&item.storage_key)
                .contains(&location)
            {
                continue;
            }
            let replacing_protected_item = self.pending_keyed_mutations.iter().any(|mutation| {
                matches!(
                    mutation,
                    PendingKeyedMutation::Set {
                        storage_key,
                        previous: Some(previous),
                        previous_state: Some(previous_state),
                        ..
                    } if *storage_key == item.storage_key
                        && previous.sg_index == logical_sg_id
                        && item_state_is_live_at(*previous_state, now_ms)
                )
            });
            if item.eviction_protected && item.is_live_at(now_ms) && !replacing_protected_item {
                // Capacity eviction may remove only resolved Evictable items. Keep protected
                // records in the generation and report NoCapacity once the victim has been
                // scanned; the generation log remains pinned so the protected value stays
                // readable.
                *protected_item_found = true;
                continue;
            }
            evictable_items.push(EvictableLocation {
                storage_key: item.storage_key,
                table_location: location,
                // `live_keys` counts non-tombstone table entries even after
                // their TTL has elapsed. Removing an expired entry must
                // decrement the counter just like removing a live value.
                live: !item.is_tombstone,
            });
        }
        Ok(())
    }
}
