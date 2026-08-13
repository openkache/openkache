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
    VALUE_FORMAT_COMPRESSION_NONE, VALUE_FORMAT_COMPRESSION_ZSTANDARD,
    VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES, VALUE_FORMAT_ENCRYPTION_COMPACT,
    VALUE_FORMAT_ENCRYPTION_NONE, VALUE_FORMAT_ENCRYPTION_ROBUST, VALUE_FORMAT_FORMAT_BYTE_BYTES,
    VALUE_FORMAT_MAX_VU128_BYTES, VALUE_FORMAT_ROBUST_CONTEXT, VALUE_FORMAT_ROBUST_NONCE_BYTES,
    VALUE_FORMAT_ROBUST_TAG_BYTES, VALUE_FORMAT_VERSION, VALUE_FORMAT_VERSION_BYTES,
};
use crate::{DATA_PROTECTION_KEY_BYTES, DataProtectionKey, ItemId};

/// Current value-format version.
pub const VERSION: u128 = VALUE_FORMAT_VERSION;

/// Bytes required for an application data protection key.
pub const ENCRYPTION_KEY_BYTES: usize = VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES;

const VERSION_BYTES: &[u8] = VALUE_FORMAT_VERSION_BYTES;
const CONTAINER_HEADER_BYTES: usize = VERSION_BYTES.len() + VALUE_FORMAT_FORMAT_BYTE_BYTES;
const NAMESPACE_ID_BYTES: usize = std::mem::size_of::<u64>();
const AAD_BYTES: usize = VALUE_FORMAT_AAD_DOMAIN.len()
    + NAMESPACE_ID_BYTES
    + ITEM_ID_BYTES
    + VERSION_BYTES.len()
    + VALUE_FORMAT_FORMAT_BYTE_BYTES;
const BINARY64_SIGNIFICAND_BITS: u32 = 53;

// The profile byte is intentionally decoded with explicit masks rather than
// treating the high and low nibbles as independent values.  Each selector is
// two bits wide; bits 6..7 are reserved and must remain zero.
const PROFILE_PROTECTION_MASK: u8 = 0x03;
const PROFILE_COMPRESSION_MASK: u8 = 0x0c;
const PROFILE_PAYLOAD_MASK: u8 = 0x30;
const PROFILE_RESERVED_MASK: u8 = 0xc0;
const PROFILE_COMPRESSION_SHIFT: u8 = 2;
const PROFILE_PAYLOAD_SHIFT: u8 = 4;
const PAYLOAD_OPAQUE_BYTES: u8 = 0;
const PAYLOAD_CBOR: u8 = 1;
const PAYLOAD_APPLICATION_DEFINED: u8 = 2;

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
    /// Exact application bytes (the v1 `OpaqueBytes` payload format).
    Raw(Vec<u8>),
    /// Exact CBOR bytes containing one accepted CBOR data item.
    ///
    /// The bytes are kept as supplied so a caller that needs a particular
    /// CBOR representation does not incur a decode/re-encode round trip.
    Cbor(Vec<u8>),
    /// Application-defined payload with its registered format identifier.
    ApplicationDefined {
        /// Client-configured application format identifier.
        format_id: u128,
        /// Bytes accepted by the registered application format.
        payload: Vec<u8>,
    },
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
    /// Application format IDs known to this codec. ID zero is the built-in
    /// canonical-JSON compatibility format used by the legacy `Value::Json`
    /// API; all other IDs must be explicitly registered.
    application_format_ids: Option<Vec<u128>>,
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
            application_format_ids: None,
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
            application_format_ids: None,
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
        if key.is_zero() {
            return Err(Error::InvalidEncryptionConfiguration);
        }
        Ok(Self {
            compression,
            encryption,
            value_root_key: Some(key.value_root_key()),
            application_format_ids: None,
        })
    }

    /// Registers an application-defined payload format ID for this codec.
    ///
    /// The ID-to-format mapping and payload validation remain owned by the
    /// application. The common codec uses this registry to reject values that
    /// cannot be interpreted by the configured client.
    pub fn register_application_format(&mut self, format_id: u128) -> Result<()> {
        if format_id == 0 {
            return Err(Error::InvalidEncodedValue(
                "application format ID zero is reserved for built-in JSON",
            ));
        }
        let format_ids = self.application_format_ids.get_or_insert_with(Vec::new);
        if !format_ids.contains(&format_id) {
            format_ids.push(format_id);
        }
        Ok(())
    }

    /// Returns an owned codec with the supplied application format IDs added.
    pub fn with_application_formats(
        mut self,
        format_ids: impl IntoIterator<Item = u128>,
    ) -> Result<Self> {
        for format_id in format_ids {
            self.register_application_format(format_id)?;
        }
        Ok(self)
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
    /// Returns an error when the compression level is unsupported.
    pub fn encrypted(
        mut key: [u8; ENCRYPTION_KEY_BYTES],
        compression: Compression,
    ) -> Result<Self> {
        let protection_key = DataProtectionKey::from_bytes(key);
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
    /// Returns an error for unprotected encryption or an unsupported compression level.
    pub fn encrypted_with_profile(
        mut key: [u8; ENCRYPTION_KEY_BYTES],
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        let protection_key = DataProtectionKey::from_bytes(key);
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
        self.encode_in_namespace(1, item_id, value)
    }

    /// Encodes a core logical value and binds it to a namespace and Item ID.
    ///
    /// Namespace zero is reserved and rejected.
    pub fn encode_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: Value,
    ) -> Result<ItemValue> {
        if namespace_id == 0 {
            return Err(Error::InvalidNamespace);
        }
        let (serialized, payload_id) = self.serialize_value(value)?;
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
        let format = self.encryption.identifier()
            | (compression_id << PROFILE_COMPRESSION_SHIFT)
            | (payload_id << PROFILE_PAYLOAD_SHIFT);
        if self.encryption != Encryption::Unprotected && item_id.len() != ITEM_ID_BYTES {
            return Err(Error::ProtectedItemIdLength {
                actual: item_id.len(),
                expected: ITEM_ID_BYTES,
            });
        }
        let aad = (self.encryption != Encryption::Unprotected)
            .then(|| make_aad(namespace_id, item_id, format));
        let body = match self.encryption {
            Encryption::Unprotected => transformed,
            Encryption::Compact => self.encrypt_compact(
                item_id,
                aad.as_ref().expect("protected codec creates AAD"),
                transformed,
            )?,
            Encryption::Robust => self.encrypt_robust(
                item_id,
                aad.as_ref().expect("protected codec creates AAD"),
                transformed,
            )?,
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
        self.seal_in_namespace(1, item_id, plaintext)
    }

    /// Encodes exact application bytes and binds them to a namespace and Item ID.
    pub fn seal_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        plaintext: &[u8],
    ) -> Result<ItemValue> {
        self.encode_in_namespace(namespace_id, item_id, Value::Raw(plaintext.to_vec()))
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
        self.seal_owned_in_namespace(1, item_id, plaintext)
    }

    /// Encodes owned application bytes and binds them to a namespace and Item ID.
    pub fn seal_owned_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        plaintext: Vec<u8>,
    ) -> Result<ItemValue> {
        self.encode_in_namespace(namespace_id, item_id, Value::Raw(plaintext))
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
    /// The decoded opaque, CBOR, application-defined, or logical JSON value.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, unsupported, unauthenticated, non-canonical, or oversized
    /// input.
    pub fn decode(&self, item_id: ItemId, encoded: ItemValue) -> Result<Value> {
        self.decode_in_namespace(1, item_id, encoded)
    }

    /// Authenticates and decodes a value bound to a namespace and Item ID.
    pub fn decode_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<Value> {
        if namespace_id == 0 {
            return Err(Error::InvalidNamespace);
        }
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
        if format & PROFILE_RESERVED_MASK != 0 {
            return Err(Error::InvalidEncodedValue(
                "profile byte has reserved bits set",
            ));
        }
        let encryption_id = format & PROFILE_PROTECTION_MASK;
        let compression_id = (format & PROFILE_COMPRESSION_MASK) >> PROFILE_COMPRESSION_SHIFT;
        let payload_id = (format & PROFILE_PAYLOAD_MASK) >> PROFILE_PAYLOAD_SHIFT;
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
        if encryption != Encryption::Unprotected && item_id.len() != ITEM_ID_BYTES {
            return Err(Error::ProtectedItemIdLength {
                actual: item_id.len(),
                expected: ITEM_ID_BYTES,
            });
        }
        let aad = (encryption != Encryption::Unprotected)
            .then(|| make_aad(namespace_id, item_id, format));
        let transformed = match encryption {
            Encryption::Unprotected => encoded,
            Encryption::Compact => {
                if encoded.len() < VALUE_FORMAT_COMPACT_SYNTHETIC_IV_BYTES {
                    return Err(Error::InvalidEncodedValue("Compact body is truncated"));
                }
                self.decrypt_compact(
                    item_id,
                    aad.as_ref().expect("protected codec creates AAD"),
                    encoded,
                )?
            }
            Encryption::Robust => {
                if encoded.len() < VALUE_FORMAT_ROBUST_NONCE_BYTES + VALUE_FORMAT_ROBUST_TAG_BYTES {
                    return Err(Error::InvalidEncodedValue("Robust body is truncated"));
                }
                self.decrypt_robust(
                    item_id,
                    aad.as_ref().expect("protected codec creates AAD"),
                    encoded,
                )?
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
        self.deserialize_value(payload_id, &serialized)
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
        self.open_in_namespace(1, item_id, encoded)
    }

    /// Authenticates and opens a Raw value bound to a namespace and Item ID.
    pub fn open_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<Vec<u8>> {
        match self.decode_in_namespace(namespace_id, item_id, encoded)? {
            Value::Raw(bytes) => Ok(bytes),
            _ => Err(Error::ExpectedRawValue),
        }
    }

    fn value_root_key(&self) -> Result<&[u8; DATA_PROTECTION_KEY_BYTES]> {
        self.value_root_key
            .as_deref()
            .ok_or(Error::EncryptionKeyRequired)
    }

    fn serialize_value(&self, value: Value) -> Result<(Vec<u8>, u8)> {
        match value {
            Value::Raw(bytes) => Ok((bytes, PAYLOAD_OPAQUE_BYTES)),
            Value::Cbor(bytes) => {
                validate_cbor_payload(&bytes)?;
                Ok((bytes, PAYLOAD_CBOR))
            }
            Value::ApplicationDefined { format_id, payload } => {
                if !self.application_format_registered(format_id) {
                    return Err(Error::UnknownApplicationFormat(format_id));
                }
                let bytes = prefix_vu128(format_id, payload)?;
                Ok((bytes, PAYLOAD_APPLICATION_DEFINED))
            }
            Value::Json(value) => {
                validate_json_value(&value)?;
                let payload = serde_json_canonicalizer::to_vec(&value)
                    .map_err(|error| Error::InvalidJson(error.to_string()))?;
                let bytes = prefix_vu128(0, payload)?;
                Ok((bytes, PAYLOAD_APPLICATION_DEFINED))
            }
        }
    }

    fn deserialize_value(&self, payload_id: u8, serialized: &[u8]) -> Result<Value> {
        match payload_id {
            PAYLOAD_OPAQUE_BYTES => Ok(Value::Raw(serialized.to_vec())),
            PAYLOAD_CBOR => {
                validate_cbor_payload(serialized)?;
                Ok(Value::Cbor(serialized.to_vec()))
            }
            PAYLOAD_APPLICATION_DEFINED => {
                let (format_id, id_length) = decode_vu128(serialized, "application format ID")?;
                if !self.application_format_registered(format_id) {
                    return Err(Error::UnknownApplicationFormat(format_id));
                }
                let payload = &serialized[id_length..];
                if format_id == 0 {
                    return decode_json(payload).map(Value::Json);
                }
                Ok(Value::ApplicationDefined {
                    format_id,
                    payload: payload.to_vec(),
                })
            }
            identifier => Err(Error::UnsupportedPayloadFormat(identifier)),
        }
    }

    fn application_format_registered(&self, format_id: u128) -> bool {
        format_id == 0
            || self
                .application_format_ids
                .as_ref()
                .is_some_and(|format_ids| format_ids.contains(&format_id))
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
    /// Namespace zero is reserved by the protocol.
    #[error("namespace ID must be a positive server-assigned identity")]
    InvalidNamespace,
    /// An unprotected profile was passed to a protected constructor.
    #[error("protected value codecs require Compact or Robust encryption")]
    InvalidEncryptionConfiguration,
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
    /// The protected value envelope currently binds exactly one 32-byte Item ID.
    #[error("protected value envelopes require {expected}-byte Item IDs, got {actual}")]
    ProtectedItemIdLength { actual: usize, expected: usize },
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
    /// The payload-format identifier is reserved or unknown.
    #[error("unsupported value payload-format identifier {0}")]
    UnsupportedPayloadFormat(u8),
    /// The CBOR payload is malformed or outside the v1 acceptance profile.
    #[error("invalid CBOR payload: {0}")]
    InvalidCbor(String),
    /// The application-defined format is not configured in this codec.
    #[error("unknown application-defined value format {0}")]
    UnknownApplicationFormat(u128),
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

/// Validates one complete CBOR item under the v1 acceptance profile.
///
/// This deliberately validates structure without assigning a Rust value to
/// the item. The CBOR payload is opaque to the common client core and is
/// returned byte-for-byte after validation.
fn validate_cbor_payload(payload: &[u8]) -> Result<()> {
    let end = parse_cbor_item(payload, 0, 0, false)?;
    if end != payload.len() {
        return Err(Error::InvalidCbor("trailing bytes after data item".into()));
    }
    Ok(())
}

fn parse_cbor_item(input: &[u8], offset: usize, depth: usize, _map_key: bool) -> Result<usize> {
    if depth > 128 {
        return Err(Error::InvalidCbor("nesting depth exceeds 128".into()));
    }
    let initial = *input
        .get(offset)
        .ok_or_else(|| Error::InvalidCbor("item is truncated".into()))?;
    let major = initial >> 5;
    let additional = initial & 0x1f;
    if additional == 31 {
        return Err(Error::InvalidCbor(
            "indefinite-length items are not supported".into(),
        ));
    }
    if major == 7 {
        let cursor = offset + 1;
        return match additional {
            0..=23 => Ok(cursor),
            24 => cursor
                .checked_add(1)
                .filter(|end| *end <= input.len())
                .ok_or_else(|| Error::InvalidCbor("simple value is truncated".into())),
            25 | 26 | 27 => {
                let width = match additional {
                    25 => 2,
                    26 => 4,
                    _ => 8,
                };
                cursor
                    .checked_add(width)
                    .filter(|end| *end <= input.len())
                    .ok_or_else(|| Error::InvalidCbor("floating-point value is truncated".into()))
            }
            _ => Err(Error::InvalidCbor("reserved simple value".into())),
        };
    }
    let (argument, mut cursor) = cbor_argument(input, offset + 1, additional)?;
    match major {
        0 | 1 => Ok(cursor),
        2 => {
            cursor = cursor
                .checked_add(usize::try_from(argument).map_err(|_| {
                    Error::InvalidCbor("byte string length exceeds platform limits".into())
                })?)
                .ok_or_else(|| Error::InvalidCbor("byte string length overflows".into()))?;
            if cursor > input.len() {
                return Err(Error::InvalidCbor("byte string is truncated".into()));
            }
            Ok(cursor)
        }
        3 => {
            let end = cursor
                .checked_add(usize::try_from(argument).map_err(|_| {
                    Error::InvalidCbor("text string length exceeds platform limits".into())
                })?)
                .ok_or_else(|| Error::InvalidCbor("text string length overflows".into()))?;
            if end > input.len() {
                return Err(Error::InvalidCbor("text string is truncated".into()));
            }
            std::str::from_utf8(&input[cursor..end])
                .map_err(|_| Error::InvalidCbor("text string is not UTF-8".into()))?;
            Ok(end)
        }
        4 => {
            for _ in 0..argument {
                cursor = parse_cbor_item(input, cursor, depth + 1, false)?;
            }
            Ok(cursor)
        }
        5 => {
            // We cannot resolve arbitrary CBOR keys into a common language
            // value without reimplementing the full data model. Reject exact
            // duplicate encodings, and reject a key when it cannot be bounded;
            // language adapters may apply stricter decoded-key checks.
            let pair_count = usize::try_from(argument)
                .map_err(|_| Error::InvalidCbor("map length exceeds platform limits".into()))?;
            let mut key_identities: Vec<Vec<u8>> = Vec::with_capacity(pair_count);
            for _ in 0..argument {
                let key_start = cursor;
                cursor = parse_cbor_item(input, cursor, depth + 1, true)?;
                let key = cbor_key_identity(&input[key_start..cursor])?.ok_or_else(|| {
                    Error::InvalidCbor("map key type cannot be compared in v1".into())
                })?;
                if key_identities.iter().any(|previous| *previous == key) {
                    return Err(Error::InvalidCbor("map contains duplicate keys".into()));
                }
                key_identities.push(key);
                cursor = parse_cbor_item(input, cursor, depth + 1, false)?;
            }
            Ok(cursor)
        }
        6 => Err(Error::InvalidCbor("tags are not assigned in v1".into())),
        _ => Err(Error::InvalidCbor("unknown CBOR major type".into())),
    }
}

/// Returns a semantic identity for the map-key types whose equality is
/// unambiguous without constructing a dynamic CBOR value. Complex keys are
/// rejected by the caller, as required by the v1 acceptance profile when a
/// decoder cannot determine uniqueness.
fn cbor_key_identity(key: &[u8]) -> Result<Option<Vec<u8>>> {
    let initial = *key
        .first()
        .ok_or_else(|| Error::InvalidCbor("map key is empty".into()))?;
    let major = initial >> 5;
    let additional = initial & 0x1f;
    if additional == 31 {
        return Err(Error::InvalidCbor(
            "indefinite-length map key is not supported".into(),
        ));
    }
    let (argument, cursor) = cbor_argument(key, 1, additional)?;
    let mut identity = Vec::new();
    match major {
        0 | 1 => {
            identity.push(major);
            identity.extend_from_slice(&argument.to_be_bytes());
        }
        2 | 3 => {
            let end = cursor
                .checked_add(usize::try_from(argument).map_err(|_| {
                    Error::InvalidCbor("map key length exceeds platform limits".into())
                })?)
                .ok_or_else(|| Error::InvalidCbor("map key length overflows".into()))?;
            if end != key.len() {
                return Err(Error::InvalidCbor("map key has trailing bytes".into()));
            }
            if major == 3 {
                std::str::from_utf8(&key[cursor..end])
                    .map_err(|_| Error::InvalidCbor("map key is not UTF-8".into()))?;
            }
            identity.push(major);
            identity.extend_from_slice(&key[cursor..end]);
        }
        7 if additional <= 23 => {
            // `false`, `true`, and `null` have stable simple-value identity.
            identity.extend_from_slice(&[major, additional]);
        }
        // Floating point equality (notably NaN) and compound-key equality
        // require a full CBOR semantic model. Reject them as ambiguous.
        _ => return Ok(None),
    }
    Ok(Some(identity))
}

fn cbor_argument(input: &[u8], offset: usize, additional: u8) -> Result<(u64, usize)> {
    match additional {
        0..=23 => Ok((additional as u64, offset)),
        24 => Ok((
            *input
                .get(offset)
                .ok_or_else(|| Error::InvalidCbor("argument is truncated".into()))?
                as u64,
            offset + 1,
        )),
        25 => {
            let bytes = input
                .get(offset..offset + 2)
                .ok_or_else(|| Error::InvalidCbor("argument is truncated".into()))?;
            Ok((u16::from_be_bytes([bytes[0], bytes[1]]) as u64, offset + 2))
        }
        26 => {
            let bytes = input
                .get(offset..offset + 4)
                .ok_or_else(|| Error::InvalidCbor("argument is truncated".into()))?;
            Ok((
                u32::from_be_bytes(bytes.try_into().expect("four-byte slice")) as u64,
                offset + 4,
            ))
        }
        27 => {
            let bytes = input
                .get(offset..offset + 8)
                .ok_or_else(|| Error::InvalidCbor("argument is truncated".into()))?;
            Ok((
                u64::from_be_bytes(bytes.try_into().expect("eight-byte slice")),
                offset + 8,
            ))
        }
        _ => Err(Error::InvalidCbor("reserved additional information".into())),
    }
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

fn make_aad(namespace_id: u64, item_id: ItemId, format: u8) -> [u8; AAD_BYTES] {
    let mut aad = [0_u8; AAD_BYTES];
    let item_id_offset = VALUE_FORMAT_AAD_DOMAIN.len();
    let namespace_end = item_id_offset + NAMESPACE_ID_BYTES;
    let version_offset = namespace_end + ITEM_ID_BYTES;
    let version_end = version_offset + VERSION_BYTES.len();
    aad[..item_id_offset].copy_from_slice(VALUE_FORMAT_AAD_DOMAIN);
    aad[item_id_offset..namespace_end].copy_from_slice(&namespace_id.to_be_bytes());
    aad[namespace_end..version_offset].copy_from_slice(item_id.as_bytes());
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
