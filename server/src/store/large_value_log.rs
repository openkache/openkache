//! Worker-local preallocated circular placement for values above the Blob tier.

use std::collections::VecDeque;

use crate::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LargeValueLocation {
    pub(crate) logical_sg_id: u32,
    pub(crate) record_start: u64,
    pub(crate) logical_len: u32,
    pub(crate) padded_len: u32,
}

pub(crate) struct LargeValueLog {
    capacity: u64,
    tail: u64,
    records: VecDeque<LargeValueLocation>,
}

impl LargeValueLog {
    pub(crate) fn new(capacity: usize) -> Result<Self> {
        let capacity = u64::try_from(capacity)
            .map_err(|_| KvError::InvalidConfig("large-value capacity exceeds u64".into()))?;
        if capacity == 0 || !capacity.is_multiple_of(BUCKET_BYTES as u64) {
            return Err(KvError::InvalidConfig(
                "large-value capacity must be a non-zero 4096-byte multiple".into(),
            ));
        }
        Ok(Self {
            capacity,
            tail: 0,
            records: VecDeque::new(),
        })
    }

    pub(crate) fn can_reserve(&self, logical_len: usize) -> Result<bool> {
        let padded_len = padded_len(logical_len)?;
        if padded_len > self.capacity {
            return Err(KvError::ItemTooLarge {
                bytes: logical_len,
                capacity: self.capacity as usize,
            });
        }
        Ok(self.next_record_start(padded_len).is_some())
    }

    pub(crate) fn reserve(
        &mut self,
        logical_sg_id: u32,
        logical_len: usize,
    ) -> Result<Option<LargeValueLocation>> {
        if logical_len == 0 {
            return Ok(None);
        }
        let padded_len = padded_len(logical_len)?;
        let record_start = self.next_record_start(padded_len).ok_or_else(|| {
            KvError::Worker("large-value reservation requires an earlier SG eviction".into())
        })?;
        let location = LargeValueLocation {
            logical_sg_id,
            record_start,
            logical_len: u32::try_from(logical_len)
                .map_err(|_| KvError::Usage("large-value extent exceeds u32".into()))?,
            padded_len: u32::try_from(padded_len)
                .map_err(|_| KvError::Usage("padded large-value extent exceeds u32".into()))?,
        };
        self.tail = record_start + padded_len;
        if self.tail == self.capacity {
            self.tail = 0;
        }
        self.records.push_back(location);
        Ok(Some(location))
    }

    pub(crate) fn release_oldest(&mut self, logical_sg_id: u32) -> Result<LargeValueLocation> {
        let Some(oldest) = self.records.front().copied() else {
            return Err(KvError::Worker(
                "cannot release an empty large-value log".into(),
            ));
        };
        if oldest.logical_sg_id != logical_sg_id {
            return Err(KvError::Worker(format!(
                "logical SG {logical_sg_id} does not own the oldest large-value extent"
            )));
        }
        self.records.pop_front();
        if self.records.is_empty() {
            self.tail = 0;
        }
        Ok(oldest)
    }

    pub(crate) fn restore(&mut self, location: LargeValueLocation) -> Result<()> {
        let end = location
            .record_start
            .checked_add(u64::from(location.padded_len))
            .ok_or_else(|| KvError::Worker("persisted large-value offset overflowed".into()))?;
        if location.logical_len > location.padded_len
            || !u64::from(location.padded_len).is_multiple_of(BUCKET_BYTES as u64)
            || end > self.capacity
        {
            return Err(KvError::Worker(
                "persisted large-value metadata does not match the configured geometry".into(),
            ));
        }
        self.tail = if end == self.capacity { 0 } else { end };
        self.records.push_back(location);
        Ok(())
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

fn padded_len(logical_len: usize) -> Result<u64> {
    u64::try_from(logical_len)
        .map_err(|_| KvError::Usage("large-value extent exceeds u64".into()))?
        .checked_next_multiple_of(BUCKET_BYTES as u64)
        .ok_or_else(|| KvError::Usage("large-value padding overflowed".into()))
}
