//! Worker-local slot-owned Blob staging with seal-time packing.

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
    payload_slots: Vec<Option<Vec<u8>>>,
    free_slots: Vec<u32>,
    live_bytes: usize,
}

impl BlobArena {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            payload_slots: Vec::new(),
            free_slots: Vec::new(),
            live_bytes: 0,
        }
    }

    pub(crate) fn insert(&mut self, value: &[u8]) -> Result<BlobHandle> {
        let live_bytes = self
            .live_bytes
            .checked_add(value.len())
            .ok_or_else(|| KvError::Usage("mutable Blob arena length overflowed".into()))?;
        if live_bytes > self.limit {
            return Err(KvError::BlobSegmentFull {
                required_bytes: value.len() as u64,
                remaining_bytes: self.limit.saturating_sub(self.live_bytes) as u64,
            });
        }
        let slot = if let Some(slot) = self.free_slots.pop() {
            self.payload_slots[slot as usize] = Some(value.to_vec());
            slot
        } else {
            let slot = u32::try_from(self.payload_slots.len())
                .map_err(|_| KvError::Usage("mutable Blob slot does not fit in u32".into()))?;
            self.payload_slots.push(Some(value.to_vec()));
            slot
        };
        let value_len = u32::try_from(value.len())
            .map_err(|_| KvError::Usage("mutable Blob value length does not fit in u32".into()))?;
        self.live_bytes = live_bytes;
        Ok(BlobHandle { slot, value_len })
    }

    pub(crate) fn replace(&mut self, previous: BlobHandle, value: &[u8]) -> Result<BlobHandle> {
        let value_len = u32::try_from(value.len())
            .map_err(|_| KvError::Usage("mutable Blob value length does not fit in u32".into()))?;
        let previous_value = self
            .payload_slots
            .get_mut(previous.slot as usize)
            .and_then(Option::as_mut)
            .filter(|previous_value| previous_value.len() == previous.value_len as usize)
            .ok_or_else(|| KvError::Worker("mutable Blob handle is no longer valid".into()))?;
        let live_bytes = self
            .live_bytes
            .checked_sub(previous_value.len())
            .and_then(|bytes| bytes.checked_add(value.len()))
            .ok_or_else(|| KvError::Usage("mutable Blob arena length overflowed".into()))?;
        if live_bytes > self.limit {
            return Err(KvError::BlobSegmentFull {
                required_bytes: value.len() as u64,
                remaining_bytes: self
                    .limit
                    .saturating_sub(self.live_bytes - previous_value.len())
                    as u64,
            });
        }
        *previous_value = value.to_vec();
        self.live_bytes = live_bytes;
        Ok(BlobHandle {
            slot: previous.slot,
            value_len,
        })
    }

    pub(crate) fn can_replace(&self, previous: BlobHandle, value_len: usize) -> bool {
        self.payload_slots
            .get(previous.slot as usize)
            .and_then(Option::as_ref)
            .filter(|value| value.len() == previous.value_len as usize)
            .and_then(|value| {
                self.live_bytes
                    .checked_sub(value.len())
                    .and_then(|bytes| bytes.checked_add(value_len))
            })
            .is_some_and(|live_bytes| live_bytes <= self.limit && value_len <= u32::MAX as usize)
    }

    pub(crate) fn remove(&mut self, handle: BlobHandle) -> bool {
        let Some(slot) = self.payload_slots.get_mut(handle.slot as usize) else {
            return false;
        };
        let Some(value) = slot.as_ref() else {
            return false;
        };
        if value.len() != handle.value_len as usize {
            return false;
        }
        self.live_bytes = self.live_bytes.saturating_sub(value.len());
        *slot = None;
        self.free_slots.push(handle.slot);
        true
    }

    pub(crate) fn get(&self, handle: BlobHandle) -> Option<&[u8]> {
        self.payload_slots
            .get(handle.slot as usize)?
            .as_deref()
            .filter(|value| value.len() == handle.value_len as usize)
    }

    pub(crate) fn pack(&self) -> Result<PackedBlob> {
        let mut bytes = Vec::with_capacity(self.live_bytes);
        let mut offsets = vec![None; self.payload_slots.len()];
        for (slot, value) in self.payload_slots.iter().enumerate() {
            let Some(value) = value else { continue };
            let offset = u32::try_from(bytes.len())
                .map_err(|_| KvError::Usage("packed Blob offset does not fit in u32".into()))?;
            bytes.extend_from_slice(value);
            offsets[slot] = Some(offset);
        }
        Ok(PackedBlob { bytes, offsets })
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.live_bytes
    }

    pub(crate) fn live_bytes(&self) -> usize {
        self.live_bytes
    }
}
