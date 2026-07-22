//! Shared byte-oriented types used across the KV cache.

use sha2::{Digest, Sha256};

/// Number of bytes in every SHA-256 hashed key.
pub const HASHED_KEY_BYTES: usize = 32;

/// User-provided, variable-length key bytes before hashing.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Key(Vec<u8>);

impl Key {
    /// Creates a key by taking ownership of its bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Returns the original key as a borrowed byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the number of bytes in the original key.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Reports whether the original key contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Consumes the wrapper and returns the owned key bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Computes the key's single canonical SHA-256 representation.
    pub fn hashed_key(&self) -> HashedKey {
        HashedKey::from(self)
    }
}

impl AsRef<[u8]> for Key {
    /// Borrows the key bytes for APIs accepting `AsRef<[u8]>`.
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<Vec<u8>> for Key {
    /// Wraps an owned byte vector as a key without copying.
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl From<&[u8]> for Key {
    /// Copies a borrowed byte slice into an owned key.
    fn from(bytes: &[u8]) -> Self {
        Self::new(bytes.to_vec())
    }
}

impl From<String> for Key {
    /// Converts an owned UTF-8 string into its key bytes without copying.
    fn from(value: String) -> Self {
        Self::new(value.into_bytes())
    }
}

impl From<&str> for Key {
    /// Copies a borrowed UTF-8 string into an owned key.
    fn from(value: &str) -> Self {
        Self::from(value.as_bytes())
    }
}

/// Variable-length value bytes associated with a [`Key`].
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

/// Canonical 32-byte SHA-256 key consumed by indexes and filters.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HashedKey([u8; HASHED_KEY_BYTES]);

impl HashedKey {
    /// Wraps an already-computed 32-byte hash without hashing it again.
    pub const fn new(bytes: [u8; HASHED_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the complete fixed-size hash bytes.
    pub const fn as_bytes(&self) -> &[u8; HASHED_KEY_BYTES] {
        &self.0
    }

    /// Consumes the wrapper and returns the complete fixed-size hash bytes.
    pub const fn into_bytes(self) -> [u8; HASHED_KEY_BYTES] {
        self.0
    }
}

impl AsRef<[u8]> for HashedKey {
    /// Borrows the hash as a byte slice.
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<[u8; HASHED_KEY_BYTES]> for HashedKey {
    /// Wraps an existing SHA-256 output without hashing it again.
    fn from(bytes: [u8; HASHED_KEY_BYTES]) -> Self {
        Self::new(bytes)
    }
}

impl From<&Key> for HashedKey {
    /// Computes SHA-256 once from the original key bytes.
    fn from(key: &Key) -> Self {
        Self::new(Sha256::digest(key.as_bytes()).into())
    }
}
