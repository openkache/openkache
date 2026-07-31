//! Worker-local physical placement for variable-length Blob and SG generations.

use std::collections::VecDeque;

use crate::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GenerationLocation {
    pub(crate) logical_sg_id: u32,
    pub(crate) record_start: u64,
    pub(crate) blob_logical_len: u32,
    pub(crate) blob_padded_len: u32,
    pub(crate) sg_base: u64,
    pub(crate) record_len: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GenerationReservation {
    Reserved(GenerationLocation),
    EvictionRequired(GenerationLocation),
}

pub(crate) struct GenerationLog {
    capacity: u64,
    segment_size: u64,
    blob_staging_limit: u64,
    tail: u64,
    records: VecDeque<GenerationLocation>,
}

impl GenerationLog {
    pub(crate) fn new(
        capacity: u64,
        segment_size: usize,
        blob_staging_limit: usize,
    ) -> Result<Self> {
        let segment_size = segment_size as u64;
        let blob_staging_limit = blob_staging_limit as u64;
        if capacity == 0
            || segment_size == 0
            || !capacity.is_multiple_of(BUCKET_BYTES as u64)
            || !segment_size.is_multiple_of(BUCKET_BYTES as u64)
            || !blob_staging_limit.is_multiple_of(BUCKET_BYTES as u64)
        {
            return Err(KvError::InvalidConfig(
                "generation log sizes must be non-zero 4096-byte multiples".into(),
            ));
        }
        let maximum_record_len = segment_size
            .checked_add(blob_staging_limit)
            .ok_or_else(|| {
                KvError::InvalidConfig("maximum generation record size overflowed".into())
            })?;
        if maximum_record_len > capacity {
            return Err(KvError::InvalidConfig(format!(
                "generation log capacity {capacity} is smaller than maximum record {maximum_record_len}"
            )));
        }
        Ok(Self {
            capacity,
            segment_size,
            blob_staging_limit,
            tail: 0,
            records: VecDeque::new(),
        })
    }

    pub(crate) fn reserve(
        &mut self,
        logical_sg_id: u32,
        blob_logical_len: usize,
    ) -> Result<GenerationReservation> {
        let (blob_logical_len, blob_padded_len, record_len) =
            self.record_lengths(blob_logical_len)?;

        let Some(record_start) = self.next_record_start(record_len) else {
            return Ok(GenerationReservation::EvictionRequired(
                self.oldest_location()
                    .expect("a generation log without physical space has a physical record"),
            ));
        };

        let blob_padded_len = u32::try_from(blob_padded_len)
            .map_err(|_| KvError::Usage("padded Blob generation exceeds u32".into()))?;
        let blob_logical_len = u32::try_from(blob_logical_len)
            .map_err(|_| KvError::Usage("Blob generation exceeds u32".into()))?;
        let location = GenerationLocation {
            logical_sg_id,
            record_start,
            blob_logical_len,
            blob_padded_len,
            sg_base: record_start + u64::from(blob_padded_len),
            record_len,
        };
        self.tail = record_start + record_len;
        if self.tail == self.capacity {
            self.tail = 0;
        }
        self.records.push_back(location);
        Ok(GenerationReservation::Reserved(location))
    }

    pub(crate) fn oldest_location(&self) -> Option<GenerationLocation> {
        self.records.front().copied()
    }

    pub(crate) fn can_reserve(&self, blob_logical_len: usize) -> Result<bool> {
        let (_, _, record_len) = self.record_lengths(blob_logical_len)?;
        Ok(self.next_record_start(record_len).is_some())
    }

    pub(crate) fn release_oldest(&mut self, logical_sg_id: u32) -> Result<GenerationLocation> {
        let Some(oldest) = self.records.front().copied() else {
            return Err(KvError::Worker(
                "cannot release an empty generation log".into(),
            ));
        };
        if oldest.logical_sg_id != logical_sg_id {
            return Err(KvError::Worker(format!(
                "logical SG {logical_sg_id} is not the physically oldest generation"
            )));
        }
        self.records.pop_front();
        if self.records.is_empty() {
            self.tail = 0;
        }
        Ok(oldest)
    }

    pub(crate) fn capacity(&self) -> u64 {
        self.capacity
    }

    fn record_lengths(&self, blob_logical_len: usize) -> Result<(u64, u64, u64)> {
        let blob_logical_len = u64::try_from(blob_logical_len)
            .map_err(|_| KvError::Usage("Blob generation length exceeds u64".into()))?;
        if blob_logical_len > self.blob_staging_limit {
            return Err(KvError::BlobSegmentFull {
                required_bytes: blob_logical_len,
                remaining_bytes: self.blob_staging_limit,
            });
        }
        let blob_padded_len = align_to_direct_io(blob_logical_len)?;
        let record_len = blob_padded_len
            .checked_add(self.segment_size)
            .ok_or_else(|| KvError::Usage("generation record length overflowed".into()))?;
        Ok((blob_logical_len, blob_padded_len, record_len))
    }

    fn next_record_start(&self, record_len: u64) -> Option<u64> {
        let Some(oldest) = self.records.front() else {
            return Some(0);
        };
        let head = oldest.record_start;
        if self.tail == head {
            return None;
        }
        if self.tail > head {
            if record_len <= self.capacity - self.tail {
                return Some(self.tail);
            }
            return (record_len <= head).then_some(0);
        }
        (record_len <= head - self.tail).then_some(self.tail)
    }
}

fn align_to_direct_io(bytes: u64) -> Result<u64> {
    bytes
        .checked_next_multiple_of(BUCKET_BYTES as u64)
        .ok_or_else(|| KvError::Usage("Blob padding length overflowed".into()))
}
