//! Worker-local mutable Blob staging with seal-time live-value compaction.

use crate::*;

pub(crate) fn encode_blob_handle(handle: BlobHandle) -> Vec<u8> {
    encode_blob_ref(BlobRef {
        value_offset: handle.slot,
        value_len: handle.value_len,
    })
}

pub(crate) fn encode_large_value_handle(handle: BlobHandle) -> Vec<u8> {
    encode_large_value_ref(BlobRef {
        value_offset: handle.slot,
        value_len: handle.value_len,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlobHandle {
    pub(crate) slot: u32,
    pub(crate) value_len: u32,
}

#[derive(Clone, Copy, Debug)]
struct BlobSpan {
    offset: usize,
    len: usize,
    live: bool,
}

#[derive(Debug)]
pub(crate) struct PackedBlob {
    pub(crate) bytes: Vec<u8>,
    offsets: Vec<Option<u32>>,
}

impl PackedBlob {
    pub(crate) fn blob_ref(&self, handle: BlobHandle) -> Result<BlobRef> {
        let offset = self
            .offsets
            .get(handle.slot as usize)
            .copied()
            .flatten()
            .ok_or_else(|| KvError::Worker("mutable Blob handle is no longer live".into()))?;
        BlobRef::new(offset as usize, handle.value_len as usize)
    }
}

#[derive(Debug)]
pub(crate) struct BlobArena {
    limit: usize,
    bytes: Vec<u8>,
    spans: Vec<BlobSpan>,
    live_bytes: usize,
}

impl BlobArena {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            bytes: Vec::new(),
            spans: Vec::new(),
            live_bytes: 0,
        }
    }

    pub(crate) fn insert(&mut self, value: &[u8]) -> Result<BlobHandle> {
        let end = self
            .bytes
            .len()
            .checked_add(value.len())
            .ok_or_else(|| KvError::Usage("mutable Blob arena length overflowed".into()))?;
        if end > self.limit {
            return Err(KvError::BlobSegmentFull {
                required_bytes: value.len() as u64,
                remaining_bytes: self.limit.saturating_sub(self.bytes.len()) as u64,
            });
        }
        let slot = u32::try_from(self.spans.len())
            .map_err(|_| KvError::Usage("mutable Blob slot does not fit in u32".into()))?;
        let value_len = u32::try_from(value.len())
            .map_err(|_| KvError::Usage("mutable Blob value length does not fit in u32".into()))?;
        let offset = self.bytes.len();
        self.bytes.extend_from_slice(value);
        self.spans.push(BlobSpan {
            offset,
            len: value.len(),
            live: true,
        });
        self.live_bytes += value.len();
        Ok(BlobHandle { slot, value_len })
    }

    pub(crate) fn replace(&mut self, previous: BlobHandle, value: &[u8]) -> Result<BlobHandle> {
        let previous_slot = previous.slot as usize;
        if let Some(span) = self.spans.get_mut(previous_slot)
            && span.live
            && value.len() <= span.len
        {
            let end = span.offset + value.len();
            self.bytes[span.offset..end].copy_from_slice(value);
            self.live_bytes = self.live_bytes - span.len + value.len();
            span.len = value.len();
            return Ok(BlobHandle {
                slot: previous.slot,
                value_len: u32::try_from(value.len()).map_err(|_| {
                    KvError::Usage("mutable Blob value length does not fit in u32".into())
                })?,
            });
        }
        let replacement = self.insert(value)?;
        self.invalidate(previous);
        Ok(replacement)
    }

    pub(crate) fn invalidate(&mut self, handle: BlobHandle) {
        if let Some(span) = self.spans.get_mut(handle.slot as usize)
            && span.live
        {
            span.live = false;
            self.live_bytes = self.live_bytes.saturating_sub(span.len);
        }
    }

    pub(crate) fn get(&self, handle: BlobHandle) -> Option<&[u8]> {
        let span = self.spans.get(handle.slot as usize)?;
        if !span.live || span.len != handle.value_len as usize {
            return None;
        }
        Some(&self.bytes[span.offset..span.offset + span.len])
    }

    pub(crate) fn pack(&self) -> Result<PackedBlob> {
        let mut bytes = Vec::with_capacity(self.live_bytes);
        let mut offsets = vec![None; self.spans.len()];
        for (slot, span) in self.spans.iter().enumerate() {
            if !span.live {
                continue;
            }
            let offset = u32::try_from(bytes.len())
                .map_err(|_| KvError::Usage("packed Blob offset does not fit in u32".into()))?;
            bytes.extend_from_slice(&self.bytes[span.offset..span.offset + span.len]);
            offsets[slot] = Some(offset);
        }
        Ok(PackedBlob { bytes, offsets })
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn live_bytes(&self) -> usize {
        self.live_bytes
    }
}
