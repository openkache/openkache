//! Shared byte-oriented types used across the KV cache.

use openkache_protocol::ValueFlags;
use std::ops::Range;
use std::sync::Arc;

use crate::store::DirectIoBuffer;

/// Number of bytes in every server-derived storage key.
pub const STORAGE_KEY_BYTES: usize = 32;

/// Variable-length value bytes associated with a storage key.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Value(Vec<u8>);

impl Value {
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

impl AsRef<[u8]> for Value {
    /// Borrows the value bytes for APIs accepting `AsRef<[u8]>`.
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<Vec<u8>> for Value {
    /// Wraps an owned byte vector as a value without copying.
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl From<&[u8]> for Value {
    /// Copies a borrowed byte slice into an owned value.
    fn from(bytes: &[u8]) -> Self {
        Self::new(bytes.to_vec())
    }
}

/// Opaque client value plus transformation bits propagated without server-side decoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EncodedValue {
    pub(crate) bytes: Vec<u8>,
    pub(crate) flags: ValueFlags,
}

impl EncodedValue {
    pub(crate) fn new(bytes: Vec<u8>, flags: ValueFlags) -> Self {
        Self { bytes, flags }
    }

    pub(crate) fn plain(bytes: Vec<u8>) -> Self {
        Self::new(bytes, ValueFlags::NONE)
    }
}

impl std::ops::Deref for EncodedValue {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}

#[derive(Debug)]
pub(crate) enum RetrievedValue {
    Owned(EncodedValue),
    Shared(Arc<EncodedValue>),
    Direct {
        buffer: DirectIoBuffer,
        value_range: Range<usize>,
        flags: ValueFlags,
    },
}

impl RetrievedValue {
    pub(crate) fn direct(
        buffer: DirectIoBuffer,
        value_range: Range<usize>,
        flags: ValueFlags,
    ) -> Self {
        debug_assert!(value_range.start <= value_range.end);
        debug_assert!(value_range.end <= buffer.len());
        Self::Direct {
            buffer,
            value_range,
            flags,
        }
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        match self {
            Self::Owned(value) => &value.bytes,
            Self::Shared(value) => &value.bytes,
            Self::Direct {
                buffer,
                value_range,
                ..
            } => &buffer[value_range.clone()],
        }
    }

    pub(crate) fn flags(&self) -> ValueFlags {
        match self {
            Self::Owned(value) => value.flags,
            Self::Shared(value) => value.flags,
            Self::Direct { flags, .. } => *flags,
        }
    }

    pub(crate) fn into_encoded(self) -> EncodedValue {
        match self {
            Self::Owned(value) => value,
            Self::Shared(value) => {
                Arc::try_unwrap(value).unwrap_or_else(|value| value.as_ref().clone())
            }
            Self::Direct {
                buffer,
                value_range,
                flags,
            } => EncodedValue::new(buffer[value_range].to_vec(), flags),
        }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.into_encoded().bytes
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
