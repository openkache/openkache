//! Generation close, packing, publication, and background flush coordination.

use std::sync::Arc;
use std::task::{Context, Poll};

use crate::storage_runtime::File;
use crate::{BUCKET_BYTES, Config, KvError, Result};
use futures_util::future::FutureExt;
use futures_util::stream::StreamExt;

use super::{
    BlobArena, BlobHandle, BlobRef, ClosingFlush, CommittedGenerationState, DirectIoBuffer,
    FlushCompletion, GenerationLocation, GenerationReservation, Kvkache, LargeValueLocation,
    MutableGeneration, MutableSegment, PreparedFlush, RamBacking, SegmentFlushReason, StoredValue,
    decode_stored_value, encode_blob_ref, encode_large_value_ref, rewrite_segment_values,
    storage_operation_error, sync_data, write_all_direct,
};

fn direct_buffer_from_bytes(bytes: &[u8]) -> Result<Option<DirectIoBuffer>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let len = bytes
        .len()
        .checked_next_multiple_of(BUCKET_BYTES)
        .ok_or_else(|| KvError::Usage("Blob write padding overflowed".into()))?;
    let mut buffer = DirectIoBuffer::zeroed(len);
    buffer[..bytes.len()].copy_from_slice(bytes);
    Ok(Some(buffer))
}

#[allow(clippy::too_many_arguments)]
async fn write_generation(
    data: File,
    large_values: File,
    config: Config,
    location: GenerationLocation,
    large_value_location: Option<LargeValueLocation>,
    blob_write: Option<DirectIoBuffer>,
    blob_physical_len: usize,
    large_value_write: Option<DirectIoBuffer>,
    large_value_physical_len: usize,
    segment_write: DirectIoBuffer,
) -> Result<u64> {
    let blob_future = async {
        match blob_write {
            Some(buffer) => write_all_direct(
                &data,
                buffer,
                location.record_start,
                blob_physical_len,
                config.write_max_time_us,
                "generation Blob write",
            )
            .await
            .map(Some),
            None => Ok(None),
        }
    };
    let segment_future = write_all_direct(
        &data,
        segment_write,
        location.sg_base,
        config.segment_size,
        config.write_max_time_us,
        "generation SG write",
    );
    let large_value_future = async {
        match (large_value_write, large_value_location) {
            (Some(buffer), Some(location)) => write_all_direct(
                &large_values,
                buffer,
                location.record_start,
                large_value_physical_len,
                config.write_max_time_us,
                "large-value write",
            )
            .await
            .map(Some),
            (None, None) => Ok(None),
            _ => Err(KvError::Worker(
                "large-value buffer and reservation disagree".into(),
            )),
        }
    };
    let (blob_result, segment_result, large_value_result) =
        futures_util::join!(blob_future, segment_future, large_value_future);
    let _blob_buffer = blob_result?;
    let _segment_buffer = segment_result?;
    let _large_value_buffer = large_value_result?;
    let (data_sync, large_values_sync) = futures_util::join!(
        sync_data(&data, config.write_max_time_us, "generation data sync"),
        sync_data(
            &large_values,
            config.write_max_time_us,
            "generation large-value sync",
        ),
    );
    data_sync?;
    large_values_sync?;
    Ok(blob_physical_len as u64 + config.segment_size as u64 + large_value_physical_len as u64)
}

impl Kvkache {
    pub(crate) async fn sync(&mut self) -> Result<()> {
        for lane in 0..self.mutable.len() {
            let should_flush = self.mutable[lane]
                .as_ref()
                .is_some_and(|generation| generation.segment.item_count != 0);
            if should_flush {
                self.flush_lane(lane, SegmentFlushReason::Sync).await?;
            }
        }
        while self.has_background_work() {
            self.wait_for_background_progress().await?;
        }
        Ok(())
    }

    pub(super) fn fullest_mutable_lane(&self) -> Result<usize> {
        self.mutable
            .iter()
            .enumerate()
            .filter_map(|(lane, generation)| {
                generation.as_ref().map(|generation| {
                    (
                        lane,
                        generation.segment.used_bytes()
                            + generation.blob_arena.allocated_bytes()
                            + generation.large_value_arena.allocated_bytes(),
                    )
                })
            })
            .max_by_key(|(_, bytes)| *bytes)
            .map(|(lane, _)| lane)
            .ok_or_else(|| KvError::Worker("worker has no mutable SG to seal".into()))
    }

    pub(super) async fn flush_lane(
        &mut self,
        lane: usize,
        reason: SegmentFlushReason,
    ) -> Result<()> {
        while self.active_flush_count() >= self.config.max_flushes_in_flight {
            self.wait_for_background_progress().await?;
        }
        self.close_lane(lane, reason)?;
        self.advance_closings()?;
        self.advance_flushes()?;
        self.drive_background_once().await?;
        Ok(())
    }

    pub(super) fn close_lane(&mut self, lane: usize, reason: SegmentFlushReason) -> Result<()> {
        let generation = self
            .mutable
            .get_mut(lane)
            .and_then(Option::take)
            .ok_or_else(|| KvError::Worker(format!("mutable SG lane {lane} is unavailable")))?;
        let logical_sg_id = generation.logical_sg_id;
        let fill_used_bytes = generation.segment.used_bytes() as u64;
        self.directory.close(
            logical_sg_id,
            RamBacking {
                sequence: generation.sequence,
                segment: Arc::new(generation.segment.bytes),
                blob_arena: generation.blob_arena,
                large_value_arena: generation.large_value_arena,
            },
        )?;
        self.closing_flushes.push_back(ClosingFlush {
            logical_sg_id,
            reason,
            fill_used_bytes,
        });
        let new_logical_sg_id = self.directory.allocate_mutable(lane)?;
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.mutable[lane] = Some(MutableGeneration {
            logical_sg_id: new_logical_sg_id,
            sequence,
            segment: MutableSegment::new(&self.config, new_logical_sg_id as usize),
            blob_arena: BlobArena::new(self.config.blob_segment_size),
            large_value_arena: BlobArena::new(self.config.large_value_capacity),
        });
        Ok(())
    }

    pub(super) fn advance_closings(&mut self) -> Result<bool> {
        let mut ready = None;
        for (index, closing) in self.closing_flushes.iter().enumerate() {
            if let Some(readable) = self
                .directory
                .closing_backing_if_ready(closing.logical_sg_id)?
            {
                ready = Some((index, readable));
                break;
            }
        }
        let Some((index, readable)) = ready else {
            return Ok(false);
        };
        let closing = self
            .closing_flushes
            .remove(index)
            .expect("the ready Closing SG was inspected above");
        let packed = readable.blob_arena.pack()?;
        let packed_large_values = readable.large_value_arena.pack()?;
        let mut segment_write = readable.segment.as_ref().clone();
        rewrite_segment_values(&mut segment_write, |encoded| {
            match decode_stored_value(encoded)? {
                StoredValue::Inline(_) => Ok(None),
                StoredValue::Blob(blob_ref) => {
                    let handle = BlobHandle {
                        slot: blob_ref.value_offset,
                        value_len: blob_ref.value_len,
                    };
                    let blob_ref = packed.blob_ref(handle).unwrap_or(BlobRef {
                        value_offset: 0,
                        value_len: 0,
                    });
                    Ok(Some(encode_blob_ref(blob_ref)))
                }
                StoredValue::Large(value_ref) => {
                    let handle = BlobHandle {
                        slot: value_ref.value_offset,
                        value_len: value_ref.value_len,
                    };
                    let value_ref = packed_large_values.blob_ref(handle).unwrap_or(BlobRef {
                        value_offset: 0,
                        value_len: 0,
                    });
                    Ok(Some(encode_large_value_ref(value_ref)))
                }
            }
        })?;
        let blob_logical_len = packed.bytes.len();
        let blob_write = direct_buffer_from_bytes(&packed.bytes)?;
        let blob_physical_len = blob_write.as_ref().map_or(0, DirectIoBuffer::capacity);
        let large_value_logical_len = packed_large_values.bytes.len();
        let large_value_write = direct_buffer_from_bytes(&packed_large_values.bytes)?;
        let large_value_physical_len = large_value_write
            .as_ref()
            .map_or(0, DirectIoBuffer::capacity);
        self.sealed_flushes.push_back(PreparedFlush {
            logical_sg_id: closing.logical_sg_id,
            reason: closing.reason,
            fill_used_bytes: closing.fill_used_bytes,
            blob_logical_len,
            blob_write,
            blob_physical_len,
            large_value_logical_len,
            large_value_write,
            large_value_physical_len,
            segment_write,
        });
        Ok(true)
    }

    pub(super) async fn drive_background_once(&mut self) -> Result<()> {
        std::future::poll_fn(|context| match self.poll_background(context) {
            Poll::Ready(result) => Poll::Ready(result.map(|_| ())),
            Poll::Pending => Poll::Ready(Ok(())),
        })
        .await
    }

    pub(super) async fn wait_for_background_progress(&mut self) -> Result<()> {
        std::future::poll_fn(|context| {
            self.poll_background(context)
                .map(|result| result.map(|_| ()))
        })
        .await
    }

    fn complete_flush(&mut self, completion: FlushCompletion) -> Result<()> {
        let (state, physical_bytes) = completion
            .result
            .map_err(|error| storage_operation_error(&self.resource_guard, error))?;
        self.io
            .data_written
            .set(self.io.data_written.get() + physical_bytes);
        let retain_ram = self.config.stable_ram_segment_count != 0;
        let logical_sg_id = state.location.logical_sg_id;
        let generation_capacity_bytes = state.location.record_len
            + state
                .large_value_location
                .map_or(0, |location| u64::from(location.padded_len));
        let large_value_logical_bytes = state
            .large_value_location
            .map_or(0, |location| u64::from(location.logical_len));
        self.directory.publish_stable(state, retain_ram)?;
        if retain_ram {
            self.stable_ram_segments.push_back(logical_sg_id);
            while self.stable_ram_segments.len() > self.config.stable_ram_segment_count {
                let logical_sg_id = self
                    .stable_ram_segments
                    .pop_front()
                    .expect("the stable RAM cache exceeded its configured capacity");
                self.directory.drop_stable_ram(logical_sg_id)?;
            }
        }
        self.segment_flushes += 1;
        match completion.reason {
            SegmentFlushReason::Capacity => self.segment_capacity_flushes += 1,
            SegmentFlushReason::Sync => self.segment_sync_flushes += 1,
        }
        self.generation_fill_used_bytes += completion.fill_used_bytes
            + completion.blob_logical_len as u64
            + large_value_logical_bytes;
        self.generation_fill_capacity_bytes += generation_capacity_bytes;
        Ok(())
    }

    pub(crate) fn has_background_work(&self) -> bool {
        !self.closing_flushes.is_empty()
            || !self.sealed_flushes.is_empty()
            || !self.inflight_flushes.is_empty()
            || self.eviction.is_some()
    }

    pub(crate) fn poll_background(&mut self, context: &mut Context<'_>) -> Poll<Result<bool>> {
        let mut progressed = false;
        if let Poll::Ready(Some(completion)) = self.inflight_flushes.poll_next_unpin(context) {
            if let Err(error) = self.complete_flush(completion) {
                return Poll::Ready(Err(error));
            }
            progressed = true;
        }
        match self.poll_eviction(context) {
            Poll::Ready(Ok(eviction_progress)) => progressed |= eviction_progress,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Pending => {}
        }
        match self.advance_closings() {
            Ok(closing_progress) => progressed |= closing_progress,
            Err(error) => return Poll::Ready(Err(error)),
        }
        match self.advance_flushes() {
            Ok(flush_progress) => progressed |= flush_progress,
            Err(error) => return Poll::Ready(Err(error)),
        }
        if progressed {
            Poll::Ready(Ok(true))
        } else {
            Poll::Pending
        }
    }

    pub(super) fn active_flush_count(&self) -> usize {
        self.closing_flushes.len() + self.sealed_flushes.len() + self.inflight_flushes.len()
    }

    pub(super) fn advance_flushes(&mut self) -> Result<bool> {
        if self.eviction.is_some() {
            return Ok(false);
        }
        let Some(prepared) = self.sealed_flushes.front() else {
            return Ok(false);
        };
        let generation_fits = self.generation_log.can_reserve(prepared.blob_logical_len)?;
        let large_values_fit = self
            .large_value_log
            .can_reserve(prepared.large_value_logical_len)?;
        if !generation_fits || !large_values_fit {
            let victim = self.generation_log.oldest_location().ok_or_else(|| {
                KvError::Worker("large-value space is exhausted without an SG victim".into())
            })?;
            if !self.directory.is_stable(victim.logical_sg_id) {
                return Ok(false);
            }
            self.start_eviction(victim)?;
            return Ok(true);
        }
        let GenerationReservation::Reserved(location) = self
            .generation_log
            .reserve(prepared.logical_sg_id, prepared.blob_logical_len)?
        else {
            return Err(KvError::Worker(
                "generation reservation changed after its successful preview".into(),
            ));
        };
        let large_value_location = self
            .large_value_log
            .reserve(prepared.logical_sg_id, prepared.large_value_logical_len)?;
        let prepared = self
            .sealed_flushes
            .pop_front()
            .expect("the prepared flush was inspected above");
        self.submit_flush(prepared, location, large_value_location)?;
        Ok(true)
    }

    fn submit_flush(
        &mut self,
        prepared: PreparedFlush,
        location: GenerationLocation,
        large_value_location: Option<LargeValueLocation>,
    ) -> Result<()> {
        let readable = self.directory.publish_inflight(
            prepared.logical_sg_id,
            location,
            large_value_location,
        )?;
        let sequence = readable.sequence;
        let file = self.data.clone();
        let large_values = self.large_values.clone();
        let config = self.config.clone();
        self.inflight_flushes.push(
            async move {
                let result = write_generation(
                    file,
                    large_values,
                    config,
                    location,
                    large_value_location,
                    prepared.blob_write,
                    prepared.blob_physical_len,
                    prepared.large_value_write,
                    prepared.large_value_physical_len,
                    prepared.segment_write,
                )
                .await
                .map(|physical_bytes| {
                    (
                        CommittedGenerationState {
                            sequence,
                            location,
                            large_value_location,
                        },
                        physical_bytes,
                    )
                });
                FlushCompletion {
                    reason: prepared.reason,
                    fill_used_bytes: prepared.fill_used_bytes,
                    blob_logical_len: prepared.blob_logical_len,
                    result,
                }
            }
            .boxed_local(),
        );
        Ok(())
    }
}
