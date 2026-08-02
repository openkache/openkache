//! Canonical serialization, compression, and authenticated value protection.

use std::collections::HashSet;
use std::fmt;

use aes_gcm_siv::aead::{AeadInOut, KeyInit as _};
use aes_gcm_siv::{Aes256GcmSiv, Nonce, Tag};
use aes_siv::siv::Aes256Siv;
use openkache_protocol::{ITEM_ID_BYTES, MAX_VALUE_BYTES};
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use zeroize::{Zeroize, Zeroizing};
use zstd_pure_rs::prelude::{
    ERR_getErrorName, ERR_isError, ZSTD_CONTENTSIZE_UNKNOWN, ZSTD_FrameHeader, ZSTD_FrameType_e,
    ZSTD_compress, ZSTD_compressBound, ZSTD_decompress, ZSTD_findFrameCompressedSize,
    ZSTD_getFrameHeader,
};

use crate::contract::{
    DEFAULT_ZSTANDARD_LEVEL, DEFAULT_ZSTANDARD_LEVEL_MAX, DEFAULT_ZSTANDARD_LEVEL_MIN,
    DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES, DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES,
    VALUE_FORMAT_AAD_DOMAIN, VALUE_FORMAT_COMPACT_ENCRYPTION_CONTEXT,
    VALUE_FORMAT_COMPACT_MAC_CONTEXT, VALUE_FORMAT_COMPACT_SYNTHETIC_IV_BYTES,
    VALUE_FORMAT_COMPRESSION_MASK, VALUE_FORMAT_COMPRESSION_NONE,
    VALUE_FORMAT_COMPRESSION_ZSTANDARD, VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES,
    VALUE_FORMAT_ENCRYPTION_COMPACT, VALUE_FORMAT_ENCRYPTION_NONE, VALUE_FORMAT_ENCRYPTION_ROBUST,
    VALUE_FORMAT_ENCRYPTION_SHIFT, VALUE_FORMAT_FORMAT_BYTE_BYTES, VALUE_FORMAT_MAX_VU128_BYTES,
    VALUE_FORMAT_ROBUST_CONTEXT, VALUE_FORMAT_ROBUST_NONCE_BYTES, VALUE_FORMAT_ROBUST_TAG_BYTES,
    VALUE_FORMAT_SERIALIZATION_JSON, VALUE_FORMAT_SERIALIZATION_RAW, VALUE_FORMAT_VERSION,
    VALUE_FORMAT_VERSION_BYTES,
};
use crate::{DATA_PROTECTION_KEY_BYTES, DataProtectionKey, ItemId};

/// Current value-format version.
pub const VERSION: u128 = VALUE_FORMAT_VERSION;

/// Bytes required for an application data protection key.
pub const ENCRYPTION_KEY_BYTES: usize = VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES;

const VERSION_BYTES: &[u8] = VALUE_FORMAT_VERSION_BYTES;
const CONTAINER_HEADER_BYTES: usize = VERSION_BYTES.len() + VALUE_FORMAT_FORMAT_BYTE_BYTES;
const AAD_BYTES: usize = VALUE_FORMAT_AAD_DOMAIN.len()
    + ITEM_ID_BYTES
    + VERSION_BYTES.len()
    + VALUE_FORMAT_FORMAT_BYTE_BYTES;
const BINARY64_SIGNIFICAND_BITS: u32 = 53;

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
    /// * `bytes` - Complete bytes to store without interpretation.
    ///
    /// # Returns
    ///
    /// An item value that preserves the supplied allocation.
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Wraps exact plaintext bytes for raw protocol storage.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Exact plaintext bytes that bypass formatted-value processing.
    ///
    /// # Returns
    ///
    /// An item value that preserves the supplied allocation.
    pub const fn plaintext(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }

    /// Returns the exact opaque bytes stored by the server.
    ///
    /// # Returns
    ///
    /// A borrowed view of the complete item value.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the value and returns its exact opaque bytes.
    ///
    /// # Returns
    ///
    /// The complete owned item-value allocation.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Returns whether the opaque value contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Returns the number of opaque bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
}

impl AsRef<[u8]> for ItemValue {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<Vec<u8>> for ItemValue {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl From<ItemValue> for Vec<u8> {
    fn from(value: ItemValue) -> Self {
        value.into_bytes()
    }
}

/// Common logical JSON value shared by language adapters.
#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    /// JSON `null`.
    Null,
    /// JSON Boolean.
    Boolean(bool),
    /// Finite IEEE-754 binary64 number.
    Number(f64),
    /// Unicode string without normalization.
    String(String),
    /// Dense ordered array.
    Array(Vec<Self>),
    /// Object entries. Encoding rejects duplicate property names.
    Object(Vec<(String, Self)>),
}

impl JsonValue {
    /// Creates a finite JSON number.
    ///
    /// # Arguments
    ///
    /// * `value` - Finite IEEE-754 binary64 value to represent.
    ///
    /// # Returns
    ///
    /// A logical JSON number.
    ///
    /// # Errors
    ///
    /// Returns an error for NaN or positive or negative infinity.
    pub fn number(value: f64) -> Result<Self> {
        if value.is_finite() {
            Ok(Self::Number(value))
        } else {
            Err(Error::InvalidJson(
                "JSON numbers must be finite IEEE-754 values".into(),
            ))
        }
    }

    /// Creates a JSON object with unique property names.
    ///
    /// # Arguments
    ///
    /// * `entries` - Object properties in any order.
    ///
    /// # Returns
    ///
    /// A logical JSON object. Encoding later applies canonical property ordering.
    ///
    /// # Errors
    ///
    /// Returns an error when a property name occurs more than once.
    pub fn object(entries: Vec<(String, Self)>) -> Result<Self> {
        validate_object_keys(&entries)?;
        Ok(Self::Object(entries))
    }
}

impl Serialize for JsonValue {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Boolean(value) => serializer.serialize_bool(*value),
            Self::Number(value) => serializer.serialize_f64(*value),
            Self::String(value) => serializer.serialize_str(value),
            Self::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(value)?;
                }
                sequence.end()
            }
            Self::Object(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for JsonValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonValueVisitor)
    }
}

struct JsonValueVisitor;

impl<'de> Visitor<'de> for JsonValueVisitor {
    type Value = JsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an RFC 8785 JSON value")
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(JsonValue::Null)
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(JsonValue::Null)
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
        Ok(JsonValue::Boolean(value))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        visit_json_integer(value.unsigned_abs() as u128, value as f64).map_err(E::custom)
    }

    fn visit_i128<E>(self, value: i128) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        visit_json_integer(value.unsigned_abs(), value as f64).map_err(E::custom)
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        visit_json_integer(value as u128, value as f64).map_err(E::custom)
    }

    fn visit_u128<E>(self, value: u128) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        visit_json_integer(value, value as f64).map_err(E::custom)
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        JsonValue::number(value).map_err(E::custom)
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(JsonValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(JsonValue::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(JsonValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
        let mut keys = HashSet::with_capacity(entries.capacity());
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(serde::de::Error::custom("duplicate JSON object property"));
            }
            entries.push((key, map.next_value()?));
        }
        Ok(JsonValue::Object(entries))
    }
}

/// Core-owned logical value supported by the formatted API.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Exact application bytes.
    Raw(Vec<u8>),
    /// RFC 8785 canonical JSON.
    Json(JsonValue),
}

/// Zstandard settings applied before value encryption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ZstandardOptions {
    /// Compression level in the standard Zstandard range.
    pub level: i32,
    /// Serialized values smaller than this many bytes bypass compression.
    pub minimum_input_size: usize,
    /// A compressed frame must save at least this many bytes.
    pub minimum_savings: usize,
}

impl Default for ZstandardOptions {
    fn default() -> Self {
        Self {
            level: DEFAULT_ZSTANDARD_LEVEL,
            minimum_input_size: DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES,
            minimum_savings: DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES,
        }
    }
}

/// Client-side compression policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Compression {
    /// Store serialized values without compression.
    #[default]
    Disabled,
    /// Compress beneficial serialized values with Zstandard.
    Zstandard(ZstandardOptions),
}

/// Authenticated-encryption profile selected for formatted values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encryption {
    /// Store formatted values without authentication or encryption.
    Unprotected,
    /// Deterministic AES-256-SIV-CMAC with 16 bytes of cryptographic overhead.
    Compact,
    /// Randomized AES-256-GCM-SIV with 28 bytes of cryptographic overhead.
    Robust,
}

impl Encryption {
    const fn identifier(self) -> u8 {
        match self {
            Self::Unprotected => VALUE_FORMAT_ENCRYPTION_NONE,
            Self::Compact => VALUE_FORMAT_ENCRYPTION_COMPACT,
            Self::Robust => VALUE_FORMAT_ENCRYPTION_ROBUST,
        }
    }
}

/// Reusable value-format encoder and decoder.
pub struct ValueCodec {
    compression: Compression,
    encryption: Encryption,
    value_root_key: Option<Zeroizing<[u8; DATA_PROTECTION_KEY_BYTES]>>,
}

impl Default for ValueCodec {
    fn default() -> Self {
        Self::plaintext()
    }
}

impl ValueCodec {
    /// Creates an unprotected formatted codec without compression.
    ///
    /// # Returns
    ///
    /// A codec that emits version 1 containers without compression or encryption.
    pub const fn plaintext() -> Self {
        Self {
            compression: Compression::Disabled,
            encryption: Encryption::Unprotected,
            value_root_key: None,
        }
    }

    /// Creates an unprotected formatted codec with an explicit compression policy.
    ///
    /// # Arguments
    ///
    /// * `compression` - Compression policy applied to serialized values.
    ///
    /// # Returns
    ///
    /// An unprotected version 1 codec using the supplied compression policy.
    ///
    /// # Errors
    ///
    /// Returns an error when the compression level is unsupported.
    pub fn compressed(compression: Compression) -> Result<Self> {
        validate_compression(compression)?;
        Ok(Self {
            compression,
            encryption: Encryption::Unprotected,
            value_root_key: None,
        })
    }

    /// Creates the recommended Robust encrypted codec.
    ///
    /// # Arguments
    ///
    /// * `key` - Application-managed data protection key.
    /// * `compression` - Compression policy applied before encryption.
    ///
    /// # Returns
    ///
    /// A version 1 codec using Robust AES-256-GCM-SIV protection.
    ///
    /// # Errors
    ///
    /// Returns an error when the compression level is unsupported.
    pub fn protected(key: &DataProtectionKey, compression: Compression) -> Result<Self> {
        Self::protected_with_profile(key, compression, Encryption::Robust)
    }

    /// Creates an encrypted codec with the selected protection profile.
    ///
    /// # Arguments
    ///
    /// * `key` - Application-managed data protection key.
    /// * `compression` - Compression policy applied before encryption.
    /// * `encryption` - Compact or Robust authenticated-encryption profile.
    ///
    /// # Returns
    ///
    /// A version 1 codec using the selected protection profile.
    ///
    /// # Errors
    ///
    /// Returns an error for unprotected encryption or an unsupported compression level.
    pub fn protected_with_profile(
        key: &DataProtectionKey,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        validate_compression(compression)?;
        if encryption == Encryption::Unprotected {
            return Err(Error::InvalidEncryptionConfiguration);
        }
        Ok(Self {
            compression,
            encryption,
            value_root_key: Some(key.value_root_key()),
        })
    }

    /// Creates the recommended Robust codec from exact data-protection-key bytes.
    ///
    /// # Arguments
    ///
    /// * `key` - Exact 32-byte data protection key.
    /// * `compression` - Compression policy applied before encryption.
    ///
    /// # Returns
    ///
    /// A version 1 codec using Robust AES-256-GCM-SIV protection.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is all zeroes or the compression level is unsupported.
    pub fn encrypted(
        mut key: [u8; ENCRYPTION_KEY_BYTES],
        compression: Compression,
    ) -> Result<Self> {
        let protection_key = DataProtectionKey::from_bytes(key)
            .map_err(|error| Error::InvalidDataProtectionKey(error.to_string()))?;
        key.zeroize();
        Self::protected(&protection_key, compression)
    }

    /// Creates a protected codec from exact data-protection-key bytes and an explicit profile.
    ///
    /// # Arguments
    ///
    /// * `key` - Exact 32-byte data protection key.
    /// * `compression` - Compression policy applied before encryption.
    /// * `encryption` - Compact or Robust authenticated-encryption profile.
    ///
    /// # Returns
    ///
    /// A version 1 codec using the selected protection profile.
    ///
    /// # Errors
    ///
    /// Returns an error for an all-zero key, unprotected encryption, or an unsupported compression
    /// level.
    pub fn encrypted_with_profile(
        mut key: [u8; ENCRYPTION_KEY_BYTES],
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        let protection_key = DataProtectionKey::from_bytes(key)
            .map_err(|error| Error::InvalidDataProtectionKey(error.to_string()))?;
        key.zeroize();
        Self::protected_with_profile(&protection_key, compression, encryption)
    }

    /// Encodes a core logical value for opaque server storage.
    ///
    /// # Arguments
    ///
    /// * `item_id` - Exact item ID bound into authenticated encryption.
    /// * `value` - Raw or logical JSON value to serialize.
    ///
    /// # Returns
    ///
    /// A complete version 1 value-format container.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid logical values, size-limit violations, compression failures,
    /// entropy failures, or encryption failures.
    pub fn encode(&self, item_id: ItemId, value: Value) -> Result<ItemValue> {
        let serialized = serialize_value(value)?;
        if serialized.len() > MAX_VALUE_BYTES {
            return Err(Error::DecodedValueTooLarge {
                size: serialized.len(),
                maximum: MAX_VALUE_BYTES,
            });
        }

        let (transformed, compressed) = compress_if_beneficial(serialized, self.compression)?;
        let compression_id = if compressed {
            VALUE_FORMAT_COMPRESSION_ZSTANDARD
        } else {
            VALUE_FORMAT_COMPRESSION_NONE
        };
        let format =
            compression_id | (self.encryption.identifier() << VALUE_FORMAT_ENCRYPTION_SHIFT);
        let aad = make_aad(item_id, format);
        let body = match self.encryption {
            Encryption::Unprotected => transformed,
            Encryption::Compact => self.encrypt_compact(item_id, &aad, transformed)?,
            Encryption::Robust => self.encrypt_robust(item_id, &aad, transformed)?,
        };

        let encoded_length =
            CONTAINER_HEADER_BYTES
                .checked_add(body.len())
                .ok_or(Error::EncodedValueTooLarge {
                    size: usize::MAX,
                    maximum: MAX_VALUE_BYTES,
                })?;
        if encoded_length > MAX_VALUE_BYTES {
            return Err(Error::EncodedValueTooLarge {
                size: encoded_length,
                maximum: MAX_VALUE_BYTES,
            });
        }
        let mut encoded = Vec::with_capacity(encoded_length);
        encoded.extend_from_slice(VERSION_BYTES);
        encoded.push(format);
        encoded.extend_from_slice(&body);
        Ok(ItemValue::new(encoded))
    }

    /// Encodes exact application bytes as the standard Raw serialization.
    ///
    /// # Arguments
    ///
    /// * `item_id` - Exact item ID bound into authenticated encryption.
    /// * `plaintext` - Exact application bytes to copy into Raw serialization.
    ///
    /// # Returns
    ///
    /// A complete version 1 value-format container.
    ///
    /// # Errors
    ///
    /// Returns an error for size-limit violations, compression failures, entropy failures, or
    /// encryption failures.
    pub fn seal(&self, item_id: ItemId, plaintext: &[u8]) -> Result<ItemValue> {
        self.seal_owned(item_id, plaintext.to_vec())
    }

    /// Encodes owned application bytes as the standard Raw serialization.
    ///
    /// # Arguments
    ///
    /// * `item_id` - Exact item ID bound into authenticated encryption.
    /// * `plaintext` - Owned application bytes to use as Raw serialization.
    ///
    /// # Returns
    ///
    /// A complete version 1 value-format container.
    ///
    /// # Errors
    ///
    /// Returns an error for size-limit violations, compression failures, entropy failures, or
    /// encryption failures.
    pub fn seal_owned(&self, item_id: ItemId, plaintext: Vec<u8>) -> Result<ItemValue> {
        self.encode(item_id, Value::Raw(plaintext))
    }

    /// Authenticates and decodes a formatted value into the core logical model.
    ///
    /// # Arguments
    ///
    /// * `item_id` - Exact item ID expected by authenticated encryption.
    /// * `encoded` - Complete version 1 container returned by the raw client.
    ///
    /// # Returns
    ///
    /// The decoded Raw or logical JSON value.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, unsupported, unauthenticated, non-canonical, or oversized
    /// input.
    pub fn decode(&self, item_id: ItemId, encoded: ItemValue) -> Result<Value> {
        let mut encoded = encoded.into_bytes();
        if encoded.len() > MAX_VALUE_BYTES {
            return Err(Error::EncodedValueTooLarge {
                size: encoded.len(),
                maximum: MAX_VALUE_BYTES,
            });
        }

        let (version, version_length) = decode_vu128(&encoded, "format version")?;
        if version != VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let Some(&format) = encoded.get(version_length) else {
            return Err(Error::InvalidEncodedValue("format byte is truncated"));
        };
        let compression_id = format & VALUE_FORMAT_COMPRESSION_MASK;
        let encryption_id = format >> VALUE_FORMAT_ENCRYPTION_SHIFT;
        let compressed = match compression_id {
            VALUE_FORMAT_COMPRESSION_NONE => false,
            VALUE_FORMAT_COMPRESSION_ZSTANDARD => true,
            identifier => return Err(Error::UnsupportedCompression(identifier)),
        };
        let encryption = match encryption_id {
            VALUE_FORMAT_ENCRYPTION_NONE => Encryption::Unprotected,
            VALUE_FORMAT_ENCRYPTION_COMPACT => Encryption::Compact,
            VALUE_FORMAT_ENCRYPTION_ROBUST => Encryption::Robust,
            identifier => return Err(Error::UnsupportedEncryption(identifier)),
        };
        if encryption != self.encryption {
            return Err(match (self.encryption, encryption) {
                (Encryption::Unprotected, _) => Error::EncryptionKeyRequired,
                (_, Encryption::Unprotected) => Error::EncryptionRequired,
                _ => Error::EncryptionProfileMismatch {
                    expected: self.encryption,
                    actual: encryption,
                },
            });
        }

        let body_offset = version_length + VALUE_FORMAT_FORMAT_BYTE_BYTES;
        let body_length = encoded.len() - body_offset;
        encoded.copy_within(body_offset.., 0);
        encoded.truncate(body_length);
        let aad = make_aad(item_id, format);
        let transformed = match encryption {
            Encryption::Unprotected => {
                if encoded.is_empty() {
                    return Err(Error::InvalidEncodedValue("serialized body is missing"));
                }
                encoded
            }
            Encryption::Compact => {
                if encoded.len() < VALUE_FORMAT_COMPACT_SYNTHETIC_IV_BYTES + 1 {
                    return Err(Error::InvalidEncodedValue("Compact body is truncated"));
                }
                self.decrypt_compact(item_id, &aad, encoded)?
            }
            Encryption::Robust => {
                if encoded.len()
                    < VALUE_FORMAT_ROBUST_NONCE_BYTES + VALUE_FORMAT_ROBUST_TAG_BYTES + 1
                {
                    return Err(Error::InvalidEncodedValue("Robust body is truncated"));
                }
                self.decrypt_robust(item_id, &aad, encoded)?
            }
        };

        let serialized = if compressed {
            decompress_zstandard(&transformed)?
        } else {
            transformed
        };
        if serialized.len() > MAX_VALUE_BYTES {
            return Err(Error::DecodedValueTooLarge {
                size: serialized.len(),
                maximum: MAX_VALUE_BYTES,
            });
        }
        deserialize_value(&serialized)
    }

    /// Decodes a formatted Raw value and returns its exact application bytes.
    ///
    /// # Arguments
    ///
    /// * `item_id` - Exact item ID expected by authenticated encryption.
    /// * `encoded` - Complete version 1 container returned by the raw client.
    ///
    /// # Returns
    ///
    /// The exact Raw serialization payload.
    ///
    /// # Errors
    ///
    /// Returns an error for any value-format failure or a non-Raw serialization.
    pub fn open(&self, item_id: ItemId, encoded: ItemValue) -> Result<Vec<u8>> {
        match self.decode(item_id, encoded)? {
            Value::Raw(bytes) => Ok(bytes),
            Value::Json(_) => Err(Error::ExpectedRawValue),
        }
    }

    fn value_root_key(&self) -> Result<&[u8; DATA_PROTECTION_KEY_BYTES]> {
        self.value_root_key
            .as_deref()
            .ok_or(Error::EncryptionKeyRequired)
    }

    fn encrypt_compact(
        &self,
        item_id: ItemId,
        aad: &[u8],
        mut plaintext: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let material = item_id_material(self.value_root_key()?, item_id);
        let mac_key = Zeroizing::new(blake3::derive_key(
            VALUE_FORMAT_COMPACT_MAC_CONTEXT,
            material.as_slice(),
        ));
        let encryption_key = Zeroizing::new(blake3::derive_key(
            VALUE_FORMAT_COMPACT_ENCRYPTION_CONTEXT,
            material.as_slice(),
        ));
        let mut combined_key = Zeroizing::new([0_u8; ENCRYPTION_KEY_BYTES * 2]);
        combined_key[..ENCRYPTION_KEY_BYTES].copy_from_slice(mac_key.as_slice());
        combined_key[ENCRYPTION_KEY_BYTES..].copy_from_slice(encryption_key.as_slice());
        Aes256Siv::new((&*combined_key).into())
            .encrypt_in_place([aad], &mut plaintext)
            .map_err(|_| Error::Encryption)?;
        Ok(plaintext)
    }

    fn decrypt_compact(
        &self,
        item_id: ItemId,
        aad: &[u8],
        mut ciphertext: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let material = item_id_material(self.value_root_key()?, item_id);
        let mac_key = Zeroizing::new(blake3::derive_key(
            VALUE_FORMAT_COMPACT_MAC_CONTEXT,
            material.as_slice(),
        ));
        let encryption_key = Zeroizing::new(blake3::derive_key(
            VALUE_FORMAT_COMPACT_ENCRYPTION_CONTEXT,
            material.as_slice(),
        ));
        let mut combined_key = Zeroizing::new([0_u8; ENCRYPTION_KEY_BYTES * 2]);
        combined_key[..ENCRYPTION_KEY_BYTES].copy_from_slice(mac_key.as_slice());
        combined_key[ENCRYPTION_KEY_BYTES..].copy_from_slice(encryption_key.as_slice());
        if Aes256Siv::new((&*combined_key).into())
            .decrypt_in_place([aad], &mut ciphertext)
            .is_err()
        {
            ciphertext.zeroize();
            return Err(Error::Authentication);
        }
        Ok(ciphertext)
    }

    fn encrypt_robust(
        &self,
        item_id: ItemId,
        aad: &[u8],
        mut plaintext: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let material = item_id_material(self.value_root_key()?, item_id);
        let robust_key = Zeroizing::new(blake3::derive_key(
            VALUE_FORMAT_ROBUST_CONTEXT,
            material.as_slice(),
        ));
        let cipher = Aes256GcmSiv::new((&*robust_key).into());
        let mut nonce_bytes = [0_u8; VALUE_FORMAT_ROBUST_NONCE_BYTES];
        getrandom::fill(&mut nonce_bytes).map_err(|error| Error::Entropy(error.to_string()))?;
        let nonce = Nonce::from(nonce_bytes);
        let plaintext_length = plaintext.len();
        let body_length = VALUE_FORMAT_ROBUST_NONCE_BYTES
            .checked_add(plaintext_length)
            .and_then(|length| length.checked_add(VALUE_FORMAT_ROBUST_TAG_BYTES))
            .ok_or(Error::EncodedValueTooLarge {
                size: usize::MAX,
                maximum: MAX_VALUE_BYTES,
            })?;
        plaintext.resize(body_length, 0);
        plaintext.copy_within(0..plaintext_length, VALUE_FORMAT_ROBUST_NONCE_BYTES);
        let tag = cipher
            .encrypt_inout_detached(
                &nonce,
                aad,
                plaintext[VALUE_FORMAT_ROBUST_NONCE_BYTES
                    ..VALUE_FORMAT_ROBUST_NONCE_BYTES + plaintext_length]
                    .as_mut()
                    .into(),
            )
            .map_err(|_| Error::Encryption)?;
        plaintext[..VALUE_FORMAT_ROBUST_NONCE_BYTES].copy_from_slice(&nonce_bytes);
        plaintext[VALUE_FORMAT_ROBUST_NONCE_BYTES + plaintext_length..].copy_from_slice(&tag);
        Ok(plaintext)
    }

    fn decrypt_robust(&self, item_id: ItemId, aad: &[u8], mut encoded: Vec<u8>) -> Result<Vec<u8>> {
        let tag_offset = encoded
            .len()
            .checked_sub(VALUE_FORMAT_ROBUST_TAG_BYTES)
            .ok_or(Error::InvalidEncodedValue("Robust body is truncated"))?;
        let nonce_bytes: [u8; VALUE_FORMAT_ROBUST_NONCE_BYTES] = encoded
            .get(..VALUE_FORMAT_ROBUST_NONCE_BYTES)
            .ok_or(Error::InvalidEncodedValue("Robust nonce is truncated"))?
            .try_into()
            .map_err(|_| Error::InvalidEncodedValue("Robust nonce has an invalid length"))?;
        let tag_bytes: [u8; VALUE_FORMAT_ROBUST_TAG_BYTES] = encoded
            .get(tag_offset..)
            .ok_or(Error::InvalidEncodedValue("Robust tag is truncated"))?
            .try_into()
            .map_err(|_| Error::InvalidEncodedValue("Robust tag has an invalid length"))?;
        let ciphertext_length = tag_offset
            .checked_sub(VALUE_FORMAT_ROBUST_NONCE_BYTES)
            .ok_or(Error::InvalidEncodedValue("Robust ciphertext is truncated"))?;
        encoded.copy_within(VALUE_FORMAT_ROBUST_NONCE_BYTES..tag_offset, 0);
        encoded.truncate(ciphertext_length);
        let material = item_id_material(self.value_root_key()?, item_id);
        let robust_key = Zeroizing::new(blake3::derive_key(
            VALUE_FORMAT_ROBUST_CONTEXT,
            material.as_slice(),
        ));
        let cipher = Aes256GcmSiv::new((&*robust_key).into());
        let nonce = Nonce::from(nonce_bytes);
        let tag = Tag::from(tag_bytes);
        if cipher
            .decrypt_inout_detached(&nonce, aad, encoded.as_mut_slice().into(), &tag)
            .is_err()
        {
            encoded.zeroize();
            return Err(Error::Authentication);
        }
        Ok(encoded)
    }
}

/// Client-side value-format errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The configured Zstandard level is unsupported.
    #[error("Zstandard level {0} is outside the configured compression-level range")]
    InvalidCompressionLevel(i32),
    /// An unprotected profile was passed to a protected constructor.
    #[error("protected value codecs require Compact or Robust encryption")]
    InvalidEncryptionConfiguration,
    /// The supplied data-protection key was invalid.
    #[error("invalid data-protection key: {0}")]
    InvalidDataProtectionKey(String),
    /// The decoded serialized value exceeded its limit.
    #[error("decoded value is too large: {size} bytes exceeds {maximum}")]
    DecodedValueTooLarge {
        /// Actual decoded size.
        size: usize,
        /// Maximum accepted decoded size.
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
    /// Authenticated encryption failed before producing a container.
    #[error("value encryption failed")]
    Encryption,
    /// Authentication failed without exposing a more specific cause.
    #[error("value authentication failed")]
    Authentication,
    /// Encrypted input was provided to a codec without a key.
    #[error("encrypted value requires a data protection key")]
    EncryptionKeyRequired,
    /// Unencrypted input was provided to a codec that requires encryption.
    #[error("client policy requires encrypted values")]
    EncryptionRequired,
    /// Input encryption did not match the configured protected profile.
    #[error("value encryption profile mismatch: expected {expected:?}, got {actual:?}")]
    EncryptionProfileMismatch {
        /// Configured encryption profile.
        expected: Encryption,
        /// Stored encryption profile.
        actual: Encryption,
    },
    /// The encoded value was structurally malformed.
    #[error("invalid encoded value: {0}")]
    InvalidEncodedValue(&'static str),
    /// A VU128 field was non-canonical, truncated, or overflowing.
    #[error("invalid {field}: {reason}")]
    InvalidVu128 {
        /// Stable field name.
        field: &'static str,
        /// Stable validation reason.
        reason: &'static str,
    },
    /// The container version is not implemented.
    #[error("unsupported value-format version {0}")]
    UnsupportedVersion(u128),
    /// The compression identifier is reserved or unknown.
    #[error("unsupported value compression identifier {0}")]
    UnsupportedCompression(u8),
    /// The encryption identifier is reserved or unknown.
    #[error("unsupported value encryption identifier {0}")]
    UnsupportedEncryption(u8),
    /// The serialization identifier is reserved or unknown.
    #[error("unsupported value serialization identifier {0}")]
    UnsupportedSerialization(u128),
    /// The caller requested Raw bytes from another serialization.
    #[error("formatted value is not Raw serialization")]
    ExpectedRawValue,
    /// JSON could not be represented by the common logical model.
    #[error("invalid canonical JSON: {0}")]
    InvalidJson(String),
    /// JSON parsed successfully but was not its exact RFC 8785 representation.
    #[error("JSON payload is not canonical RFC 8785 JSON")]
    NonCanonicalJson,
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

/// Convenience result type for value-format operations.
pub type Result<T> = std::result::Result<T, Error>;

fn validate_compression(compression: Compression) -> Result<()> {
    let Compression::Zstandard(options) = compression else {
        return Ok(());
    };
    if !(DEFAULT_ZSTANDARD_LEVEL_MIN..=DEFAULT_ZSTANDARD_LEVEL_MAX).contains(&options.level) {
        return Err(Error::InvalidCompressionLevel(options.level));
    }
    Ok(())
}

fn serialize_value(value: Value) -> Result<Vec<u8>> {
    match value {
        Value::Raw(bytes) => prefix_vu128(VALUE_FORMAT_SERIALIZATION_RAW as u128, bytes),
        Value::Json(value) => {
            let payload = canonical_json_bytes(&value)?;
            prefix_vu128(VALUE_FORMAT_SERIALIZATION_JSON as u128, payload)
        }
    }
}

/// Serializes one logical JSON value using the shared RFC 8785 representation.
///
/// Language adapters call this helper instead of carrying a second canonicalizer
/// dependency. Validation and number/object-key rules therefore stay identical
/// for every binding.
pub fn canonical_json_bytes(value: &JsonValue) -> Result<Vec<u8>> {
    validate_json_value(value)?;
    serde_json_canonicalizer::to_vec(value).map_err(|error| Error::InvalidJson(error.to_string()))
}

fn deserialize_value(serialized: &[u8]) -> Result<Value> {
    let (identifier, identifier_length) = decode_vu128(serialized, "serialization identifier")?;
    let payload = &serialized[identifier_length..];
    match identifier {
        value if value == VALUE_FORMAT_SERIALIZATION_RAW as u128 => {
            Ok(Value::Raw(payload.to_vec()))
        }
        value if value == VALUE_FORMAT_SERIALIZATION_JSON as u128 => {
            decode_json(payload).map(Value::Json)
        }
        identifier => Err(Error::UnsupportedSerialization(identifier)),
    }
}

fn prefix_vu128(identifier: u128, payload: Vec<u8>) -> Result<Vec<u8>> {
    let mut encoded_identifier = [0_u8; VALUE_FORMAT_MAX_VU128_BYTES];
    let identifier_length = vu128::encode_u128(&mut encoded_identifier, identifier);
    let total_length =
        identifier_length
            .checked_add(payload.len())
            .ok_or(Error::EncodedValueTooLarge {
                size: usize::MAX,
                maximum: MAX_VALUE_BYTES,
            })?;
    let mut serialized = Vec::with_capacity(total_length);
    serialized.extend_from_slice(&encoded_identifier[..identifier_length]);
    serialized.extend_from_slice(&payload);
    Ok(serialized)
}

fn decode_vu128(input: &[u8], field: &'static str) -> Result<(u128, usize)> {
    let Some(&first) = input.first() else {
        return Err(Error::InvalidVu128 {
            field,
            reason: "field is truncated",
        });
    };
    let encoded_length = vu128::encoded_len(first);
    if encoded_length > VALUE_FORMAT_MAX_VU128_BYTES {
        return Err(Error::InvalidVu128 {
            field,
            reason: "field overflows u128",
        });
    }
    if input.len() < encoded_length {
        return Err(Error::InvalidVu128 {
            field,
            reason: "field is truncated",
        });
    }
    let mut encoded = [0_u8; VALUE_FORMAT_MAX_VU128_BYTES];
    encoded[..encoded_length].copy_from_slice(&input[..encoded_length]);
    let (value, decoded_length) = vu128::decode_u128(&encoded);
    if decoded_length != encoded_length {
        return Err(Error::InvalidVu128 {
            field,
            reason: "decoder returned an invalid length",
        });
    }

    let mut canonical = [0_u8; VALUE_FORMAT_MAX_VU128_BYTES];
    let canonical_length = vu128::encode_u128(&mut canonical, value);
    if canonical_length != encoded_length
        || canonical[..canonical_length] != input[..encoded_length]
    {
        return Err(Error::InvalidVu128 {
            field,
            reason: "field is not canonical",
        });
    }
    Ok((value, encoded_length))
}

fn validate_json_value(value: &JsonValue) -> Result<()> {
    match value {
        JsonValue::Number(number) if !number.is_finite() => Err(Error::InvalidJson(
            "JSON numbers must be finite IEEE-754 values".into(),
        )),
        JsonValue::Array(values) => {
            for value in values {
                validate_json_value(value)?;
            }
            Ok(())
        }
        JsonValue::Object(entries) => {
            validate_object_keys(entries)?;
            for (_, value) in entries {
                validate_json_value(value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn visit_json_integer(magnitude: u128, value: f64) -> Result<JsonValue> {
    let bit_length = u128::BITS - magnitude.leading_zeros();
    let exactly_representable = bit_length <= BINARY64_SIGNIFICAND_BITS || {
        let discarded_bits = bit_length - BINARY64_SIGNIFICAND_BITS;
        magnitude & ((1_u128 << discarded_bits) - 1) == 0
    };
    if !exactly_representable {
        return Err(Error::InvalidJson(
            "JSON integers must be exactly representable as IEEE-754 binary64 values".into(),
        ));
    }
    Ok(JsonValue::Number(value))
}

fn validate_object_keys(entries: &[(String, JsonValue)]) -> Result<()> {
    let mut keys = HashSet::with_capacity(entries.len());
    for (key, _) in entries {
        if !keys.insert(key.as_str()) {
            return Err(Error::InvalidJson(
                "JSON object property names must be unique".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn parse_json_input(payload: &[u8]) -> Result<JsonValue> {
    validate_json_integer_tokens(payload)?;
    let mut deserializer = serde_json::Deserializer::from_slice(payload);
    let value = JsonValue::deserialize(&mut deserializer)
        .map_err(|error| Error::InvalidJson(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| Error::InvalidJson(error.to_string()))?;
    Ok(value)
}

fn decode_json(payload: &[u8]) -> Result<JsonValue> {
    let value = parse_json_input(payload)?;
    let canonical = serde_json_canonicalizer::to_vec(&value)
        .map_err(|error| Error::InvalidJson(error.to_string()))?;
    if canonical != payload {
        return Err(Error::NonCanonicalJson);
    }
    Ok(value)
}

fn validate_json_integer_tokens(payload: &[u8]) -> Result<()> {
    let mut in_string = false;
    let mut index = 0;
    while index < payload.len() {
        let byte = payload[index];
        if in_string {
            match byte {
                b'\\' => index = index.saturating_add(2),
                b'"' => {
                    in_string = false;
                    index += 1;
                }
                _ => index += 1,
            }
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if byte == b'-' || byte.is_ascii_digit() {
            let start = index;
            index += 1;
            while let Some(&next) = payload.get(index) {
                if next.is_ascii_whitespace() || matches!(next, b',' | b']' | b'}') {
                    break;
                }
                index += 1;
            }
            validate_json_number_token(&payload[start..index])?;
            continue;
        }
        index += 1;
    }
    Ok(())
}

fn validate_json_number_token(token: &[u8]) -> Result<()> {
    let Ok(token) = std::str::from_utf8(token) else {
        return Ok(());
    };
    if token.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E')) {
        return Ok(());
    }
    let Ok(value) = token.parse::<f64>() else {
        return Ok(());
    };
    if !value.is_finite() {
        return Err(Error::InvalidJson(
            "JSON numbers must be finite IEEE-754 values".into(),
        ));
    }
    let Some(expected) = integer_token(token) else {
        return Ok(());
    };
    let actual = normalize_integer_string(&format!("{value:.0}"));
    if actual != expected {
        return Err(Error::InvalidJson(
            "JSON integers must be exactly representable as IEEE-754 binary64 values".into(),
        ));
    }
    Ok(())
}

fn integer_token(token: &str) -> Option<String> {
    let (negative, token) = token
        .strip_prefix('-')
        .map_or((false, token), |token| (true, token));
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let digits = token.trim_start_matches('0');
    if digits.is_empty() {
        return Some("0".to_owned());
    }
    let mut normalized = digits.to_owned();
    if negative {
        normalized.insert(0, '-');
    }
    Some(normalized)
}

fn normalize_integer_string(value: &str) -> String {
    let negative = value.starts_with('-');
    let digits = value.strip_prefix('-').unwrap_or(value);
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        "0".to_owned()
    } else if negative {
        format!("-{digits}")
    } else {
        digits.to_owned()
    }
}

fn compress_if_beneficial(
    serialized: Vec<u8>,
    compression: Compression,
) -> Result<(Vec<u8>, bool)> {
    let Compression::Zstandard(options) = compression else {
        return Ok((serialized, false));
    };
    if serialized.len() < options.minimum_input_size {
        return Ok((serialized, false));
    }

    let mut compressed = vec![0_u8; ZSTD_compressBound(serialized.len())];
    let compressed_length = ZSTD_compress(&mut compressed, &serialized, options.level);
    check_zstandard("compression", compressed_length)?;
    if compressed_length >= serialized.len()
        || serialized.len() - compressed_length < options.minimum_savings
    {
        return Ok((serialized, false));
    }
    compressed.truncate(compressed_length);
    Ok((compressed, true))
}

fn decompress_zstandard(compressed: &[u8]) -> Result<Vec<u8>> {
    let mut header = ZSTD_FrameHeader::default();
    let header_result = ZSTD_getFrameHeader(&mut header, compressed);
    check_zstandard("frame-header decoding", header_result)?;
    if header_result != 0 {
        return Err(Error::InvalidEncodedValue(
            "Zstandard frame header is truncated",
        ));
    }
    if header.frameType != ZSTD_FrameType_e::ZSTD_frame {
        return Err(Error::InvalidEncodedValue(
            "Zstandard skippable frames are not supported",
        ));
    }
    if header.frameContentSize == ZSTD_CONTENTSIZE_UNKNOWN {
        return Err(Error::InvalidEncodedValue(
            "Zstandard frame does not declare its content size",
        ));
    }
    if header.dictID != 0 {
        return Err(Error::InvalidEncodedValue(
            "Zstandard dictionaries are not supported",
        ));
    }
    if header.windowSize > MAX_VALUE_BYTES as u64 {
        return Err(Error::DecodedValueTooLarge {
            size: usize::try_from(header.windowSize).unwrap_or(usize::MAX),
            maximum: MAX_VALUE_BYTES,
        });
    }

    let original_length =
        usize::try_from(header.frameContentSize).map_err(|_| Error::DecodedValueTooLarge {
            size: usize::MAX,
            maximum: MAX_VALUE_BYTES,
        })?;
    if original_length > MAX_VALUE_BYTES {
        return Err(Error::DecodedValueTooLarge {
            size: original_length,
            maximum: MAX_VALUE_BYTES,
        });
    }
    let frame_length = ZSTD_findFrameCompressedSize(compressed);
    check_zstandard("frame-size validation", frame_length)?;
    if frame_length != compressed.len() {
        return Err(Error::InvalidEncodedValue(
            "Zstandard frame contains trailing bytes or another frame",
        ));
    }

    let mut serialized = vec![0_u8; original_length];
    let decoded = ZSTD_decompress(&mut serialized, compressed);
    check_zstandard("decompression", decoded)?;
    if decoded != original_length {
        return Err(Error::DecompressedLength {
            expected: original_length,
            actual: decoded,
        });
    }
    Ok(serialized)
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

fn make_aad(item_id: ItemId, format: u8) -> [u8; AAD_BYTES] {
    let mut aad = [0_u8; AAD_BYTES];
    let item_id_offset = VALUE_FORMAT_AAD_DOMAIN.len();
    let version_offset = item_id_offset + ITEM_ID_BYTES;
    let version_end = version_offset + VERSION_BYTES.len();
    aad[..item_id_offset].copy_from_slice(VALUE_FORMAT_AAD_DOMAIN);
    aad[item_id_offset..version_offset].copy_from_slice(item_id.as_bytes());
    aad[version_offset..version_end].copy_from_slice(VERSION_BYTES);
    aad[version_end..version_end + VALUE_FORMAT_FORMAT_BYTE_BYTES]
        .copy_from_slice(std::slice::from_ref(&format));
    aad
}

fn item_id_material(
    value_root_key: &[u8; DATA_PROTECTION_KEY_BYTES],
    item_id: ItemId,
) -> Zeroizing<[u8; DATA_PROTECTION_KEY_BYTES + ITEM_ID_BYTES]> {
    let mut material = Zeroizing::new([0_u8; DATA_PROTECTION_KEY_BYTES + ITEM_ID_BYTES]);
    material[..DATA_PROTECTION_KEY_BYTES].copy_from_slice(value_root_key);
    material[DATA_PROTECTION_KEY_BYTES..].copy_from_slice(item_id.as_bytes());
    material
}
