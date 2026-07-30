//! Fixed-size item keys and reusable keyed derivation helpers.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{Error, Result};

/// Bytes in every OpenKache item key.
pub const ITEM_KEY_BYTES: usize = openkache_protocol::ITEM_KEY_BYTES;

/// Bytes in an application-managed data protection key.
pub const DATA_PROTECTION_KEY_BYTES: usize = 32;

/// Exact fixed-size item key sent through the OpenKache protocol.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ItemKey([u8; ITEM_KEY_BYTES]);

impl ItemKey {
    /// Wraps an exact item key without hashing it again.
    pub const fn from_bytes(bytes: [u8; ITEM_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the exact wire bytes.
    pub const fn as_bytes(&self) -> &[u8; ITEM_KEY_BYTES] {
        &self.0
    }

    /// Consumes the key and returns its exact wire bytes.
    pub const fn into_bytes(self) -> [u8; ITEM_KEY_BYTES] {
        self.0
    }

    pub(crate) const fn into_protocol(self) -> openkache_protocol::ItemKey {
        openkache_protocol::ItemKey::new(self.0)
    }
}

impl AsRef<[u8]> for ItemKey {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Application-managed master secret used to hide keys and encrypt values.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DataProtectionKey {
    master_key: [u8; DATA_PROTECTION_KEY_BYTES],
    item_key: [u8; DATA_PROTECTION_KEY_BYTES],
}

impl DataProtectionKey {
    /// Creates a data protection key from exact random bytes.
    pub fn from_bytes(bytes: [u8; DATA_PROTECTION_KEY_BYTES]) -> Self {
        let item_key = derive_subkey(&bytes, b"openkache/v1/key");
        Self {
            master_key: bytes,
            item_key,
        }
    }

    /// Copies an exact data protection key from a language binding or configuration buffer.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Exactly 32 random secret bytes.
    ///
    /// # Returns
    ///
    /// An owned data protection key.
    ///
    /// # Errors
    ///
    /// Returns an error when `bytes` does not contain exactly 32 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let exact: &[u8; DATA_PROTECTION_KEY_BYTES] = bytes.try_into().map_err(|_| {
            Error::configuration(
                "data_protection_key",
                format!(
                    "must contain exactly {DATA_PROTECTION_KEY_BYTES} bytes, got {}",
                    bytes.len()
                ),
            )
        })?;
        Ok(Self::from_bytes(*exact))
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
        let engine = if encoded.ends_with('=') {
            &STANDARD
        } else {
            &STANDARD_NO_PAD
        };
        let decoded = Zeroizing::new(
            engine
                .decode(encoded)
                .map_err(|error| Error::configuration("data_protection_key", error.to_string()))?,
        );
        if decoded.len() != DATA_PROTECTION_KEY_BYTES {
            return Err(Error::configuration(
                "data_protection_key",
                format!(
                    "must decode to exactly {DATA_PROTECTION_KEY_BYTES} bytes, got {}",
                    decoded.len()
                ),
            ));
        }
        let mut bytes = [0; DATA_PROTECTION_KEY_BYTES];
        bytes.copy_from_slice(&decoded);
        Ok(Self::from_bytes(bytes))
    }

    /// Returns the canonical padded Base64 representation for secret storage.
    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.master_key)
    }

    /// Derives the deterministic HMAC-SHA-256 item key for application key bytes.
    pub fn derive_item_key(&self, application_key: impl AsRef<[u8]>) -> ItemKey {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.item_key)
            .expect("HMAC-SHA-256 accepts a 32-byte key");
        mac.update(application_key.as_ref());
        ItemKey::from_bytes(mac.finalize().into_bytes().into())
    }

    pub(crate) fn derive_value_key(&self) -> [u8; 32] {
        derive_subkey(&self.master_key, b"openkache/v1/value")
    }
}

fn derive_subkey(master_key: &[u8; DATA_PROTECTION_KEY_BYTES], context: &[u8]) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(Some(b"openkache/v1"), master_key);
    let mut output = [0; 32];
    hkdf.expand(context, &mut output)
        .expect("SHA-256 HKDF supports a 32-byte output");
    output
}
