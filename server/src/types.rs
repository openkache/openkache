//! Shared byte-oriented types used across the KV cache.

use std::ops::{Deref, Range};
use std::sync::Arc;

use crate::store::DirectIoBuffer;

/// Number of bytes in every server-derived storage key.
///
/// The storage key is the server-side representation of the protocol item ID,
/// so its size comes from the Smithy-generated protocol contract.
pub use openkache_protocol::ITEM_ID_BYTES as STORAGE_KEY_BYTES;

/// Variable-length application value associated with an item ID.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemValue(Vec<u8>);

impl ItemValue {
    /// Creates a value by taking ownership of its bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Returns the value as a borrowed byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the number of bytes in the value.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Reports whether the value contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Consumes the wrapper and returns the owned value bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl AsRef<[u8]> for ItemValue {
    /// Borrows the value bytes for APIs accepting `AsRef<[u8]>`.
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<Vec<u8>> for ItemValue {
    /// Wraps an owned byte vector as a value without copying.
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl From<&[u8]> for ItemValue {
    /// Copies a borrowed byte slice into an owned value.
    fn from(bytes: &[u8]) -> Self {
        Self::new(bytes.to_vec())
    }
}

/// Opaque client value propagated without server-side decoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StoredItemBytes {
    Owned(Arc<Vec<u8>>),
    Segment {
        segment: Arc<DirectIoBuffer>,
        range: Range<usize>,
    },
}

impl StoredItemBytes {
    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Segment { segment, range } => &segment[range.clone()],
        }
    }
}

impl Deref for StoredItemBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredItemValue {
    pub(crate) bytes: StoredItemBytes,
}

impl StoredItemValue {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: StoredItemBytes::Owned(Arc::new(bytes)),
        }
    }

    pub(crate) fn from_segment(segment: Arc<DirectIoBuffer>, range: Range<usize>) -> Self {
        debug_assert!(range.start <= range.end && range.end <= segment.len());
        Self {
            bytes: StoredItemBytes::Segment { segment, range },
        }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        match self.bytes {
            StoredItemBytes::Owned(bytes) => {
                Arc::try_unwrap(bytes).unwrap_or_else(|bytes| (*bytes).clone())
            }
            StoredItemBytes::Segment { segment, range } => segment[range].to_vec(),
        }
    }
}

impl Deref for StoredItemValue {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}

/// Canonical 32-byte server-derived key consumed by routing, indexes, and storage.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StorageKey([u8; STORAGE_KEY_BYTES]);

impl StorageKey {
    /// Wraps an already-derived 32-byte storage key without hashing it again.
    pub const fn new(bytes: [u8; STORAGE_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the complete fixed-size storage key bytes.
    pub const fn as_bytes(&self) -> &[u8; STORAGE_KEY_BYTES] {
        &self.0
    }

    /// Consumes the wrapper and returns the complete fixed-size storage key bytes.
    pub const fn into_bytes(self) -> [u8; STORAGE_KEY_BYTES] {
        self.0
    }
}

impl AsRef<[u8]> for StorageKey {
    /// Borrows the storage key as a byte slice.
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<[u8; STORAGE_KEY_BYTES]> for StorageKey {
    /// Wraps an existing server-derived key without hashing it again.
    fn from(bytes: [u8; STORAGE_KEY_BYTES]) -> Self {
        Self::new(bytes)
    }
}
