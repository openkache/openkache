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

#[cfg(test)]
mod tests {
    //! Representation and conversion tests for shared KV types.

    use super::*;
    use std::mem::{align_of, size_of};

    /// Verifies `HashedKey` has no padding beyond its 32-byte array.
    #[test]
    fn hashed_key_is_exactly_32_bytes() {
        assert_eq!(size_of::<HashedKey>(), HASHED_KEY_BYTES);
        assert_eq!(align_of::<HashedKey>(), 1);
    }

    /// Verifies `Key::hashed_key` against the standard SHA-256 `abc` vector.
    #[test]
    fn key_hash_matches_sha256_test_vector() {
        let key = Key::from("abc");
        let expected = [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ];

        assert_eq!(key.hashed_key(), HashedKey::new(expected));
    }

    /// Verifies key and value wrappers preserve their original bytes.
    #[test]
    fn key_and_value_round_trip_bytes() {
        let key = Key::from(&b"key"[..]);
        let value = Value::from(&b"value"[..]);

        assert_eq!(key.as_bytes(), b"key");
        assert_eq!(value.as_bytes(), b"value");
        assert_eq!(key.into_bytes(), b"key".to_vec());
        assert_eq!(value.into_bytes(), b"value".to_vec());
    }
}
