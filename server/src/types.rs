//! Shared byte-oriented types used across the KV cache.

use std::hash::{Hash, Hasher};
use std::ops::{Deref, Range};
use std::sync::Arc;

use openkache_protocol::{OwnedRange, StableByteOwner};

use crate::store::{DirectIoBuffer, DirectIoBufferLease};

/// Number of bytes in every server-derived storage key.
///
/// This is a storage-format invariant. Protocol adapters may derive this fixed
/// identity from wire keys of any supported width, but wire model changes must
/// not resize persisted storage records.
pub const STORAGE_KEY_BYTES: usize = 32;

/// Existence condition for a storage mutation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StorageWriteCondition {
    #[default]
    Any,
    IfAbsent,
    IfPresent,
}

/// Concrete expiration selection accepted by storage.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StorageWriteExpiration {
    #[default]
    Inherit,
    NoExpiry,
    Ttl(u64),
}

/// Concrete eviction selection accepted by storage.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StorageWriteEviction {
    #[default]
    Inherit,
    Evictable,
    Protected,
}

/// Operation-neutral policy for storing one value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StorageWriteOptions {
    /// Required relationship between the key and existing storage state.
    pub condition: StorageWriteCondition,
    /// Expiration policy applied to the stored value.
    pub expiration: StorageWriteExpiration,
    /// Eviction policy applied to the stored value.
    pub eviction: StorageWriteEviction,
}

impl StorageWriteOptions {
    pub(crate) const fn ttl_ms(self) -> Option<u64> {
        match self.expiration {
            StorageWriteExpiration::Ttl(ttl_ms) => Some(ttl_ms),
            StorageWriteExpiration::Inherit | StorageWriteExpiration::NoExpiry => None,
        }
    }

    pub(crate) const fn eviction_protected(self) -> bool {
        matches!(self.eviction, StorageWriteEviction::Protected)
    }
}

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
pub(crate) enum StoredItemBytes {
    Owned(Arc<Vec<u8>>),
    RangedOwned {
        buffer: Arc<Vec<u8>>,
        range: Range<usize>,
    },
    Segment {
        segment: Arc<DirectIoBuffer>,
        range: Range<usize>,
    },
    DirectRead {
        buffer: Option<DirectIoBufferLease>,
        range: Range<usize>,
    },
    SharedDirectRead {
        buffer: Arc<DirectIoBufferLease>,
        range: Range<usize>,
    },
}

impl StoredItemBytes {
    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::RangedOwned { buffer, range } => &buffer[range.clone()],
            Self::Segment { segment, range } => &segment[range.clone()],
            Self::DirectRead { buffer, range } => &buffer
                .as_ref()
                .expect("unique direct-read value has a buffer")[range.clone()],
            Self::SharedDirectRead { buffer, range } => &buffer[range.clone()],
        }
    }
}

impl StableByteOwner for StoredItemBytes {
    fn as_bytes(&self) -> &[u8] {
        self.as_slice()
    }
}

impl StableByteOwner for DirectIoBuffer {
    fn as_bytes(&self) -> &[u8] {
        self
    }
}

impl StableByteOwner for DirectIoBufferLease {
    fn as_bytes(&self) -> &[u8] {
        self
    }
}

impl Clone for StoredItemBytes {
    fn clone(&self) -> Self {
        match self {
            Self::Owned(bytes) => Self::Owned(Arc::clone(bytes)),
            Self::RangedOwned { buffer, range } => Self::RangedOwned {
                buffer: Arc::clone(buffer),
                range: range.clone(),
            },
            Self::Segment { segment, range } => Self::Segment {
                segment: Arc::clone(segment),
                range: range.clone(),
            },
            Self::DirectRead { .. } => Self::Owned(Arc::new(self.as_slice().to_vec())),
            Self::SharedDirectRead { buffer, range } => Self::SharedDirectRead {
                buffer: Arc::clone(buffer),
                range: range.clone(),
            },
        }
    }
}

impl std::fmt::Debug for StoredItemBytes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("StoredItemBytes")
            .field(&self.as_slice())
            .finish()
    }
}

impl PartialEq for StoredItemBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for StoredItemBytes {}

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

    #[allow(dead_code)]
    pub(crate) fn from_owned_range(bytes: OwnedRange) -> Self {
        let (buffer, range) = bytes.into_parts();
        debug_assert!(range.start <= range.end && range.end <= buffer.len());
        if range.start == 0 && range.end == buffer.len() {
            return Self::new(buffer);
        }
        Self {
            bytes: StoredItemBytes::RangedOwned {
                buffer: Arc::new(buffer),
                range,
            },
        }
    }

    pub(crate) fn from_segment(segment: Arc<DirectIoBuffer>, range: Range<usize>) -> Self {
        debug_assert!(range.start <= range.end && range.end <= segment.len());
        Self {
            bytes: StoredItemBytes::Segment { segment, range },
        }
    }

    pub(crate) fn from_direct_read(buffer: DirectIoBufferLease, range: Range<usize>) -> Self {
        debug_assert!(range.start <= range.end && range.end <= buffer.len());
        Self {
            bytes: StoredItemBytes::DirectRead {
                buffer: Some(buffer),
                range,
            },
        }
    }

    /// Shares the current owner without copying its visible bytes.
    ///
    /// A unique direct-read lease is promoted to shared ownership once; all
    /// other representations only clone an existing reference-counted owner.
    pub(crate) fn clone_for_retention(&mut self) -> Self {
        if let StoredItemBytes::DirectRead { buffer, range } = &mut self.bytes {
            let range = range.clone();
            let buffer = Arc::new(
                buffer
                    .take()
                    .expect("unique direct-read value has a buffer"),
            );
            self.bytes = StoredItemBytes::SharedDirectRead {
                buffer: Arc::clone(&buffer),
                range: range.clone(),
            };
            return Self {
                bytes: StoredItemBytes::SharedDirectRead { buffer, range },
            };
        }
        self.clone()
    }

    pub(crate) fn clone_for_visible_state(&mut self) -> Self {
        self.clone_for_retention()
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        match self.bytes {
            StoredItemBytes::Owned(bytes) => {
                Arc::try_unwrap(bytes).unwrap_or_else(|bytes| (*bytes).clone())
            }
            StoredItemBytes::RangedOwned { buffer, range } => {
                let mut buffer = match Arc::try_unwrap(buffer) {
                    Ok(buffer) => buffer,
                    Err(buffer) => return buffer[range].to_vec(),
                };
                if range.start == 0 && range.end == buffer.len() {
                    return buffer;
                }
                let len = range.len();
                buffer.copy_within(range, 0);
                buffer.truncate(len);
                buffer
            }
            StoredItemBytes::Segment { segment, range } => segment[range].to_vec(),
            StoredItemBytes::DirectRead { buffer, range } => {
                buffer.expect("unique direct-read value has a buffer")[range].to_vec()
            }
            StoredItemBytes::SharedDirectRead { buffer, range } => buffer[range].to_vec(),
        }
    }
}

impl Deref for StoredItemValue {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}

impl AsRef<[u8]> for StoredItemValue {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

/// Compact shared ownership used while a value is retained by RAM storage.
///
/// The owner preserves the incoming allocation and logical range. Mutable
/// replacement publishes a new owner, so readers can retain the previous
/// bytes without copying them.
pub(crate) enum RetainedItemValue {
    Owned {
        buffer: Arc<Vec<u8>>,
        range: Range<usize>,
    },
    Segment {
        segment: Arc<DirectIoBuffer>,
        range: Range<usize>,
    },
    DirectRead {
        buffer: Arc<DirectIoBufferLease>,
        range: Range<usize>,
    },
}

const _: () = assert!(std::mem::size_of::<Option<RetainedItemValue>>() <= 32);

impl RetainedItemValue {
    pub(crate) fn share(value: &mut StoredItemValue) -> Self {
        if matches!(&value.bytes, StoredItemBytes::DirectRead { .. }) {
            let _ = value.clone_for_retention();
        }
        match &value.bytes {
            StoredItemBytes::Owned(buffer) => Self::Owned {
                range: 0..buffer.len(),
                buffer: Arc::clone(buffer),
            },
            StoredItemBytes::RangedOwned { buffer, range } => Self::Owned {
                buffer: Arc::clone(buffer),
                range: range.clone(),
            },
            StoredItemBytes::Segment { segment, range } => Self::Segment {
                segment: Arc::clone(segment),
                range: range.clone(),
            },
            StoredItemBytes::DirectRead { .. } => {
                unreachable!("direct-read values become shared before retention")
            }
            StoredItemBytes::SharedDirectRead { buffer, range } => Self::DirectRead {
                buffer: Arc::clone(buffer),
                range: range.clone(),
            },
        }
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Owned { buffer, range } => &buffer[range.clone()],
            Self::Segment { segment, range } => &segment[range.clone()],
            Self::DirectRead { buffer, range } => &buffer[range.clone()],
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.as_slice().len()
    }

    pub(crate) fn to_stored_value(&self) -> StoredItemValue {
        let bytes = match self {
            Self::Owned { buffer, range } if range.start == 0 && range.end == buffer.len() => {
                StoredItemBytes::Owned(Arc::clone(buffer))
            }
            Self::Owned { buffer, range } => StoredItemBytes::RangedOwned {
                buffer: Arc::clone(buffer),
                range: range.clone(),
            },
            Self::Segment { segment, range } => StoredItemBytes::Segment {
                segment: Arc::clone(segment),
                range: range.clone(),
            },
            Self::DirectRead { buffer, range } => StoredItemBytes::SharedDirectRead {
                buffer: Arc::clone(buffer),
                range: range.clone(),
            },
        };
        StoredItemValue { bytes }
    }
}

impl std::fmt::Debug for RetainedItemValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("RetainedItemValue")
            .field(&self.as_slice())
            .finish()
    }
}

/// Canonical 32-byte server-derived key consumed by routing, indexes, and storage.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
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

    pub(crate) fn routing_hash(&self) -> u64 {
        u64::from_le_bytes(self.0[8..16].try_into().unwrap())
    }

    pub(crate) fn table_hash(&self) -> u128 {
        u128::from_le_bytes(self.0[8..24].try_into().unwrap())
    }

    /// Consumes the wrapper and returns the complete fixed-size storage key bytes.
    pub const fn into_bytes(self) -> [u8; STORAGE_KEY_BYTES] {
        self.0
    }
}

impl Hash for StorageKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // The recoverable domain prefix is intentionally excluded from every
        // storage distribution path; this digest word owns routing entropy.
        state.write_u64(self.routing_hash());
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
