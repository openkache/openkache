//! Fixed-size wire keys and the canonical SHA-256 derivation helper.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{Error, Result};

/// Bytes in every OpenKache wire key.
pub const KEY_BYTES: usize = openkache_protocol::KEY_BYTES;

/// Bytes in an application-managed data protection key.
pub const DATA_PROTECTION_KEY_BYTES: usize = 32;

/// Exact fixed-size key sent through the OpenKache protocol.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Key([u8; KEY_BYTES]);

impl Key {
    /// Derives the canonical wire key from arbitrary application key bytes.
    pub fn derive(application_key: impl AsRef<[u8]>) -> Self {
        Self(Sha256::digest(application_key.as_ref()).into())
    }

    /// Wraps an exact wire key without hashing it again.
    pub const fn from_bytes(bytes: [u8; KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the exact wire bytes.
    pub const fn as_bytes(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }

    /// Consumes the key and returns its exact wire bytes.
    pub const fn into_bytes(self) -> [u8; KEY_BYTES] {
        self.0
    }

    pub(crate) const fn into_protocol(self) -> openkache_protocol::Key {
        openkache_protocol::Key::new(self.0)
    }
}

impl AsRef<[u8]> for Key {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<openkache_protocol::Key> for Key {
    fn from(key: openkache_protocol::Key) -> Self {
        Self::from_bytes(key.into_bytes())
    }
}

/// Application-managed master secret used to hide keys and encrypt values.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DataProtectionKey([u8; DATA_PROTECTION_KEY_BYTES]);

impl DataProtectionKey {
    /// Creates a data protection key from exact random bytes.
    pub const fn from_bytes(bytes: [u8; DATA_PROTECTION_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// Decodes a Base64-encoded 32-byte random secret.
    ///
    /// # Arguments
    ///
    /// * `encoded` - Standard padded or unpadded Base64 text.
    ///
    /// # Returns
    ///
    /// An owned data protection key.
    ///
    /// # Errors
    ///
    /// Returns an error when Base64 decoding fails or does not produce exactly 32 bytes.
    pub fn from_base64(encoded: &str) -> Result<Self> {
        let decoded = STANDARD
            .decode(encoded)
            .or_else(|_| {
                STANDARD.decode(format!(
                    "{encoded}{}",
                    "=".repeat((4 - encoded.len() % 4) % 4)
                ))
            })
            .map_err(|error| Error::configuration("data_protection_key", error.to_string()))?;
        let bytes =
            <[u8; DATA_PROTECTION_KEY_BYTES]>::try_from(decoded).map_err(|decoded: Vec<u8>| {
                Error::configuration(
                    "data_protection_key",
                    format!(
                        "must decode to exactly {DATA_PROTECTION_KEY_BYTES} bytes, got {}",
                        decoded.len()
                    ),
                )
            })?;
        Ok(Self(bytes))
    }

    /// Returns the canonical padded Base64 representation for secret storage.
    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.0)
    }

    /// Derives the deterministic HMAC-SHA-256 wire key for application key bytes.
    pub fn derive_key(&self, application_key: impl AsRef<[u8]>) -> Key {
        let key = self.derive_subkey(b"openkache/v1/key");
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&key).expect("HMAC-SHA-256 accepts a 32-byte key");
        mac.update(application_key.as_ref());
        Key::from_bytes(mac.finalize().into_bytes().into())
    }

    pub(crate) fn derive_value_key(&self) -> [u8; 32] {
        self.derive_subkey(b"openkache/v1/value")
    }

    fn derive_subkey(&self, context: &[u8]) -> [u8; 32] {
        let hkdf = Hkdf::<Sha256>::new(Some(b"openkache/v1"), &self.0);
        let mut output = [0; 32];
        hkdf.expand(context, &mut output)
            .expect("SHA-256 HKDF supports a 32-byte output");
        output
    }
}
