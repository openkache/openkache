//! Reusable compression and authenticated item-value protection tools.

use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{Key as CipherKey, Tag, XChaCha20Poly1305, XNonce};
use openkache_protocol::MAX_VALUE_BYTES;
use zeroize::Zeroize;
use zstd_pure_rs::prelude::{
    ERR_getErrorName, ERR_isError, ZSTD_CONTENTSIZE_ERROR, ZSTD_CONTENTSIZE_UNKNOWN, ZSTD_compress,
    ZSTD_compressBound, ZSTD_decompress, ZSTD_getFrameContentSize,
};

use crate::{DataProtectionKey, ItemKey};

/// Bytes required for an XChaCha20-Poly1305 key.
pub const ENCRYPTION_KEY_BYTES: usize = 32;

const NONCE_BYTES: usize = 24;
const TAG_BYTES: usize = 16;
const ENCRYPTED_OVERHEAD_BYTES: usize = NONCE_BYTES + TAG_BYTES;
const VALUE_HEADER: [u8; 4] = [0x4f, 0x4b, 0x54, 0x01];
const VALUE_HEADER_BYTES: usize = VALUE_HEADER.len() + 1;
const VALUE_COMPRESSED_FLAG: u8 = 1 << 0;
const VALUE_ENCRYPTED_FLAG: u8 = 1 << 1;
const KNOWN_VALUE_FLAGS: u8 = VALUE_COMPRESSED_FLAG | VALUE_ENCRYPTED_FLAG;
const AAD_BYTES: usize = 32 + VALUE_HEADER_BYTES;

/// Client-owned encoded bytes stored opaquely by the server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemValue {
    bytes: Vec<u8>,
}

impl ItemValue {
    /// Wraps exact opaque bytes for raw storage.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Complete bytes to store without interpretation or transformation.
    ///
    /// # Returns
    ///
    /// An item value that preserves the supplied allocation.
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Wraps exact plaintext bytes for raw storage.
    pub const fn plaintext(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }

    /// Returns the exact opaque bytes stored by the server.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the value and returns its exact opaque bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Zstandard settings used before values are encrypted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ZstandardOptions {
    /// Compression level in the standard Zstandard range.
    pub level: i32,
    /// Values smaller than this many bytes bypass compression.
    pub minimum_input_size: usize,
    /// Compressed output must save at least this many bytes.
    pub minimum_savings: usize,
}

impl Default for ZstandardOptions {
    fn default() -> Self {
        Self {
            level: 1,
            minimum_input_size: 1_024,
            minimum_savings: 64,
        }
    }
}

/// Client-side compression policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Compression {
    /// Store values without compression.
    #[default]
    Disabled,
    /// Compress beneficial values with Zstandard.
    Zstandard(ZstandardOptions),
}

/// A reusable codec that transforms values before and after server storage.
pub struct ValueCodec {
    compression: Compression,
    cipher: Option<XChaCha20Poly1305>,
}

impl Default for ValueCodec {
    fn default() -> Self {
        Self::plaintext()
    }
}

impl ValueCodec {
    /// Creates a backwards-compatible codec that does not wrap values.
    ///
    /// # Returns
    ///
    /// A codec that sends and receives exact plaintext bytes.
    pub const fn plaintext() -> Self {
        Self {
            compression: Compression::Disabled,
            cipher: None,
        }
    }

    /// Creates a codec that compresses values without encrypting them.
    ///
    /// # Arguments
    ///
    /// * `compression` - Compression policy applied to stored values.
    ///
    /// # Returns
    ///
    /// A codec that stores values in the client-owned transformation envelope.
    ///
    /// # Errors
    ///
    /// Returns an error when the compression settings are outside supported bounds.
    pub fn compressed(compression: Compression) -> Result<Self> {
        validate_compression(compression)?;
        Ok(Self {
            compression,
            cipher: None,
        })
    }

    /// Creates a codec from the value-encryption subkey derived by a master secret.
    ///
    /// # Arguments
    ///
    /// * `key` - Master secret shared with the application key-hiding layer.
    /// * `compression` - Compression policy applied before encryption.
    ///
    /// # Returns
    ///
    /// A codec that encrypts values with a domain-separated key derived from `key`.
    ///
    /// # Errors
    ///
    /// Returns an error when the compression settings are outside supported bounds.
    pub fn protected(key: &DataProtectionKey, compression: Compression) -> Result<Self> {
        Self::encrypted(key.derive_value_key(), compression)
    }

    /// Creates a codec that compresses and then encrypts every value.
    ///
    /// # Arguments
    ///
    /// * `key` - Exact 32-byte XChaCha20-Poly1305 key.
    /// * `compression` - Compression policy applied before encryption.
    ///
    /// # Returns
    ///
    /// A codec whose values can only be opened with the same key.
    ///
    /// # Errors
    ///
    /// Returns an error when the compression settings are outside supported bounds.
    pub fn encrypted(
        mut key: [u8; ENCRYPTION_KEY_BYTES],
        compression: Compression,
    ) -> Result<Self> {
        validate_compression(compression)?;
        let cipher = XChaCha20Poly1305::new(
            &CipherKey::try_from(&key[..]).expect("encryption key has the required fixed length"),
        );
        key.zeroize();
        Ok(Self {
            compression,
            cipher: Some(cipher),
        })
    }

    /// Encodes a borrowed plaintext value for server storage.
    ///
    /// # Arguments
    ///
    /// * `key` - Item key used to bind ciphertext to its cache key.
    /// * `plaintext` - Exact application value.
    ///
    /// # Returns
    ///
    /// An encoded value no larger than the protocol value limit.
    ///
    /// # Errors
    ///
    /// Returns an error for oversized values, entropy failures, compression failures, or
    /// encryption failures.
    pub fn seal(&self, key: ItemKey, plaintext: &[u8]) -> Result<ItemValue> {
        self.seal_owned(key, plaintext.to_vec())
    }

    /// Encodes an owned plaintext value while reusing its allocation when practical.
    ///
    /// # Arguments
    ///
    /// * `key` - Item key used to bind ciphertext to its cache key.
    /// * `plaintext` - Owned application value whose allocation may be reused.
    ///
    /// # Returns
    ///
    /// An encoded value no larger than the protocol value limit.
    ///
    /// # Errors
    ///
    /// Returns an error for oversized values, entropy failures, compression failures, or
    /// encryption failures.
    pub fn seal_owned(&self, key: ItemKey, plaintext: Vec<u8>) -> Result<ItemValue> {
        if plaintext.len() > MAX_VALUE_BYTES {
            return Err(Error::PlaintextTooLarge {
                size: plaintext.len(),
                maximum: MAX_VALUE_BYTES,
            });
        }
        if self.cipher.is_none() && self.compression == Compression::Disabled {
            return Ok(ItemValue::new(plaintext));
        }

        let (mut body, compressed) = compress_if_beneficial(plaintext, self.compression)?;
        let encrypted = self.cipher.is_some();
        let flags = ((compressed as u8) * VALUE_COMPRESSED_FLAG)
            | ((encrypted as u8) * VALUE_ENCRYPTED_FLAG);
        if let Some(cipher) = &self.cipher {
            let mut nonce = [0_u8; NONCE_BYTES];
            getrandom::fill(&mut nonce).map_err(|error| Error::Entropy(error.to_string()))?;

            let body_length = body.len();
            body.reserve(ENCRYPTED_OVERHEAD_BYTES);
            body.resize(NONCE_BYTES + body_length, 0);
            body.copy_within(0..body_length, NONCE_BYTES);
            body[..NONCE_BYTES].copy_from_slice(&nonce);

            let nonce = XNonce::try_from(&nonce[..]).expect("nonce has the required fixed length");
            let aad = make_aad(key, flags);
            let tag = cipher
                .encrypt_inout_detached(&nonce, &aad, (&mut body[NONCE_BYTES..]).into())
                .map_err(|_| Error::Encryption)?;
            body.extend_from_slice(&tag);
        }
        let encoded_len =
            VALUE_HEADER_BYTES
                .checked_add(body.len())
                .ok_or(Error::EncodedValueTooLarge {
                    size: usize::MAX,
                    maximum: MAX_VALUE_BYTES,
                })?;
        if encoded_len > MAX_VALUE_BYTES {
            return Err(Error::EncodedValueTooLarge {
                size: encoded_len,
                maximum: MAX_VALUE_BYTES,
            });
        }
        body.reserve(VALUE_HEADER_BYTES);
        let body_len = body.len();
        body.resize(encoded_len, 0);
        body.copy_within(0..body_len, VALUE_HEADER_BYTES);
        body[..VALUE_HEADER.len()].copy_from_slice(&VALUE_HEADER);
        body[VALUE_HEADER.len()] = flags;
        Ok(ItemValue::new(body))
    }

    /// Decodes a value returned by the server.
    ///
    /// # Arguments
    ///
    /// * `key` - Item key that must match the key used while sealing.
    /// * `encoded` - Owned server payload whose allocation is reused when possible.
    ///
    /// # Returns
    ///
    /// The authenticated, decompressed application value.
    ///
    /// # Errors
    ///
    /// Returns an error when the encoded value is malformed, too large, cannot be authenticated,
    /// or cannot be decompressed.
    pub fn open(&self, key: ItemKey, encoded: ItemValue) -> Result<Vec<u8>> {
        let mut encoded = encoded.bytes;
        if encoded.len() > MAX_VALUE_BYTES {
            return Err(Error::EncodedValueTooLarge {
                size: encoded.len(),
                maximum: MAX_VALUE_BYTES,
            });
        }
        if self.cipher.is_none() && self.compression == Compression::Disabled {
            return Ok(encoded);
        }
        if encoded.len() < VALUE_HEADER_BYTES || encoded[..VALUE_HEADER.len()] != VALUE_HEADER {
            return Err(Error::InvalidEncodedValue(
                "client transformation header is missing or unsupported",
            ));
        }
        let flags = encoded[VALUE_HEADER.len()];
        if flags & !KNOWN_VALUE_FLAGS != 0 {
            return Err(Error::InvalidEncodedValue(
                "client transformation flags contain unknown bits",
            ));
        }
        let compressed = flags & VALUE_COMPRESSED_FLAG != 0;
        let encrypted = flags & VALUE_ENCRYPTED_FLAG != 0;
        if self.cipher.is_some() != encrypted {
            return Err(if encrypted {
                Error::EncryptionKeyRequired
            } else {
                Error::EncryptionRequired
            });
        }
        let body_len = encoded.len() - VALUE_HEADER_BYTES;
        encoded.copy_within(VALUE_HEADER_BYTES.., 0);
        encoded.truncate(body_len);
        if !encrypted && !compressed {
            return Ok(encoded);
        }
        let Some(cipher) = &self.cipher else {
            return decompress_zstandard(&encoded);
        };
        if encoded.len() < ENCRYPTED_OVERHEAD_BYTES {
            return Err(Error::InvalidEncodedValue(
                "nonce or authentication tag is truncated",
            ));
        }

        let tag_offset = encoded.len() - TAG_BYTES;
        let nonce: [u8; NONCE_BYTES] = encoded[..NONCE_BYTES]
            .try_into()
            .expect("validated nonce length");
        let tag = Tag::from(
            <[u8; TAG_BYTES]>::try_from(&encoded[tag_offset..])
                .expect("validated authentication tag length"),
        );
        let nonce = XNonce::try_from(&nonce[..]).expect("nonce has the required fixed length");
        let aad = make_aad(key, flags);
        cipher
            .decrypt_inout_detached(
                &nonce,
                &aad,
                (&mut encoded[NONCE_BYTES..tag_offset]).into(),
                &tag,
            )
            .map_err(|_| Error::Authentication)?;
        encoded.truncate(tag_offset);

        if compressed {
            return decompress_zstandard(&encoded[NONCE_BYTES..]);
        }

        let body_length = encoded.len() - NONCE_BYTES;
        encoded.copy_within(NONCE_BYTES.., 0);
        encoded.truncate(body_length);
        Ok(encoded)
    }
}

/// Client-side value transformation errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The configured Zstandard level is unsupported.
    #[error("Zstandard level {0} is outside the supported range 1..=22")]
    InvalidCompressionLevel(i32),
    /// Plaintext exceeded the protocol limit before transformation.
    #[error("plaintext is too large: {size} bytes exceeds {maximum}")]
    PlaintextTooLarge {
        /// Actual plaintext size.
        size: usize,
        /// Maximum accepted plaintext size.
        maximum: usize,
    },
    /// Encoded bytes exceeded the protocol limit.
    #[error("encoded value is too large: {size} bytes exceeds {maximum}")]
    EncodedValueTooLarge {
        /// Actual encoded size.
        size: usize,
        /// Maximum accepted encoded size.
        maximum: usize,
    },
    /// The operating system could not provide nonce entropy.
    #[error("operating-system entropy failed: {0}")]
    Entropy(String),
    /// XChaCha20-Poly1305 encryption failed.
    #[error("value encryption failed")]
    Encryption,
    /// Ciphertext, flags, or associated key authentication failed.
    #[error("value authentication failed")]
    Authentication,
    /// Encrypted input was provided to a codec without a key.
    #[error("encrypted value requires an encryption key")]
    EncryptionKeyRequired,
    /// Plain input was provided to a codec that requires encryption.
    #[error("client policy requires encrypted values")]
    EncryptionRequired,
    /// The encoded value was structurally malformed.
    #[error("invalid encoded value: {0}")]
    InvalidEncodedValue(&'static str),
    /// Zstandard compression or decompression failed.
    #[error("Zstandard {operation} failed: {message}")]
    Zstandard {
        /// Stable codec operation name.
        operation: &'static str,
        /// Human-readable codec detail.
        message: String,
    },
    /// A Zstandard frame produced a different length than declared.
    #[error("decoded value length mismatch: expected {expected} bytes, got {actual}")]
    DecompressedLength {
        /// Length declared by the frame.
        expected: usize,
        /// Length produced by decompression.
        actual: usize,
    },
}

/// Convenience result type for value transformations.
pub type Result<T> = std::result::Result<T, Error>;

fn validate_compression(compression: Compression) -> Result<()> {
    let Compression::Zstandard(options) = compression else {
        return Ok(());
    };
    if !(1..=22).contains(&options.level) {
        return Err(Error::InvalidCompressionLevel(options.level));
    }
    Ok(())
}

fn compress_if_beneficial(plaintext: Vec<u8>, compression: Compression) -> Result<(Vec<u8>, bool)> {
    let Compression::Zstandard(options) = compression else {
        return Ok((plaintext, false));
    };
    if plaintext.len() < options.minimum_input_size {
        return Ok((plaintext, false));
    }

    let mut compressed = vec![0_u8; ZSTD_compressBound(plaintext.len())];
    let compressed_length = ZSTD_compress(&mut compressed, &plaintext, options.level);
    check_zstandard("compression", compressed_length)?;
    if compressed_length >= plaintext.len()
        || plaintext.len() - compressed_length < options.minimum_savings
    {
        return Ok((plaintext, false));
    }
    compressed.truncate(compressed_length);
    Ok((compressed, true))
}

fn check_zstandard(operation: &'static str, result: usize) -> Result<()> {
    if ERR_isError(result) {
        Err(Error::Zstandard {
            operation,
            message: ERR_getErrorName(result).to_string(),
        })
    } else {
        Ok(())
    }
}

fn make_aad(key: ItemKey, flags: u8) -> [u8; AAD_BYTES] {
    let mut aad = [0_u8; AAD_BYTES];
    aad[..32].copy_from_slice(key.as_bytes());
    aad[32..32 + VALUE_HEADER.len()].copy_from_slice(&VALUE_HEADER);
    aad[AAD_BYTES - 1] = flags;
    aad
}

fn decompress_zstandard(compressed: &[u8]) -> Result<Vec<u8>> {
    let declared = ZSTD_getFrameContentSize(compressed);
    if declared == ZSTD_CONTENTSIZE_ERROR {
        return Err(Error::InvalidEncodedValue(
            "Zstandard frame header is invalid",
        ));
    }
    if declared == ZSTD_CONTENTSIZE_UNKNOWN {
        return Err(Error::InvalidEncodedValue(
            "Zstandard frame does not declare its content size",
        ));
    }
    let original_length = usize::try_from(declared).map_err(|_| Error::PlaintextTooLarge {
        size: usize::MAX,
        maximum: MAX_VALUE_BYTES,
    })?;
    if original_length > MAX_VALUE_BYTES {
        return Err(Error::PlaintextTooLarge {
            size: original_length,
            maximum: MAX_VALUE_BYTES,
        });
    }

    let mut plaintext = vec![0_u8; original_length];
    let decoded = ZSTD_decompress(&mut plaintext, compressed);
    check_zstandard("decompression", decoded)?;
    if decoded != original_length {
        return Err(Error::DecompressedLength {
            expected: original_length,
            actual: decoded,
        });
    }
    Ok(plaintext)
}
