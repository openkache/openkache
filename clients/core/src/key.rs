//! Portable key conversion, canonical CBOR encoding, and Item ID derivation.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{Error, ITEM_ID_BYTES, Result};

pub(crate) const PROTECTION_KEY_BYTES: usize =
    crate::contract::VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES;

/// Bytes in an application-managed data protection key.
pub const DATA_PROTECTION_KEY_BYTES: usize = PROTECTION_KEY_BYTES;
/// Bytes in the client root key.
pub const CLIENT_ROOT_KEY_BYTES: usize = PROTECTION_KEY_BYTES;
/// Maximum canonical key bytes accepted by every conforming v1 SDK.
pub const MAX_CANONICAL_KEY_BYTES: usize = 1_048_576;

/// The one native key type selected for a formatted keyspace.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum KeySpec {
    /// Mathematical signed or unsigned integer identity.
    Integer,
    /// Exact valid UTF-8 text identity.
    Text,
    /// Exact byte-string identity, including empty and NUL-containing values.
    Bytes,
}

impl KeySpec {
    /// Returns the stable lower-case name used in diagnostics.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Integer => "integer",
            Self::Text => "text",
            Self::Bytes => "bytes",
        }
    }
}

impl fmt::Display for KeySpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Arbitrary-precision mathematical integer used by [`PortableKey`].
///
/// The magnitude is stored as minimal big-endian bytes. Zero is always
/// non-negative. This representation lets native `u128` values and language
/// bindings with larger integer types use the same canonical bignum rules.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PortableInteger {
    negative: bool,
    magnitude: Vec<u8>,
}

impl PortableInteger {
    /// Creates an integer from a signed native value.
    pub fn from_i128(value: i128) -> Self {
        if value < 0 {
            Self::from_parts(true, &value.unsigned_abs().to_be_bytes())
                .expect("a native i128 magnitude is always valid")
        } else {
            Self::from_parts(false, &(value as u128).to_be_bytes())
                .expect("a native i128 magnitude is always valid")
        }
    }

    /// Creates an integer from an unsigned native value.
    pub fn from_u128(value: u128) -> Self {
        Self::from_parts(false, &value.to_be_bytes())
            .expect("a native u128 magnitude is always valid")
    }

    /// Creates an integer from a sign and big-endian magnitude.
    ///
    /// Leading zeroes are normalized away. An empty magnitude represents zero;
    /// zero is normalized to a positive value.
    ///
    /// # Errors
    ///
    /// Returns an error when `magnitude` is larger than the configured key
    /// resource limit.
    pub fn from_parts(negative: bool, magnitude: &[u8]) -> std::result::Result<Self, KeyError> {
        if magnitude.len() > MAX_CANONICAL_KEY_BYTES {
            return Err(KeyError::TooLarge {
                size: magnitude.len(),
                maximum: MAX_CANONICAL_KEY_BYTES,
            });
        }
        let first_nonzero = magnitude
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(magnitude.len());
        let magnitude = magnitude[first_nonzero..].to_vec();
        Ok(Self {
            negative: negative && !magnitude.is_empty(),
            magnitude,
        })
    }

    /// Returns whether this integer is negative.
    pub const fn is_negative(&self) -> bool {
        self.negative
    }

    /// Returns the minimal big-endian magnitude.
    pub fn magnitude(&self) -> &[u8] {
        &self.magnitude
    }

    fn cbor_bytes(&self, output: &mut Vec<u8>) {
        if !self.negative {
            if let Some(value) = as_u64(&self.magnitude) {
                encode_argument(output, 0, value);
            } else {
                output.push(0xc2);
                encode_argument(output, 2, self.magnitude.len() as u64);
                output.extend_from_slice(&self.magnitude);
            }
            return;
        }

        let transformed = subtract_one(&self.magnitude);
        if let Some(value) = as_u64(&transformed) {
            encode_argument(output, 1, value);
        } else {
            output.push(0xc3);
            encode_argument(output, 2, transformed.len() as u64);
            output.extend_from_slice(&transformed);
        }
    }
}

macro_rules! impl_signed_integer {
    ($($type:ty),+ $(,)?) => {
        $(
            impl From<$type> for PortableKey {
                fn from(value: $type) -> Self {
                    Self::Integer(PortableInteger::from_i128(value as i128))
                }
            }
        )+
    };
}

macro_rules! impl_unsigned_integer {
    ($($type:ty),+ $(,)?) => {
        $(
            impl From<$type> for PortableKey {
                fn from(value: $type) -> Self {
                    Self::Integer(PortableInteger::from_u128(value as u128))
                }
            }
        )+
    };
}

/// The v1 key-only logical model.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum PortableKey {
    /// An arbitrary-precision mathematical integer.
    Integer(PortableInteger),
    /// Exact valid UTF-8 text.
    Text(String),
    /// Exact bytes, including empty and NUL-containing values.
    Bytes(Vec<u8>),
}

impl PortableKey {
    /// Creates a text key.
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    /// Creates an exact byte-string key.
    pub fn bytes(value: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(value.into())
    }

    /// Creates an integer key from a signed native value.
    pub fn integer(value: i128) -> Self {
        Self::Integer(PortableInteger::from_i128(value))
    }

    /// Returns the key's logical [`KeySpec`].
    pub const fn spec(&self) -> KeySpec {
        match self {
            Self::Integer(_) => KeySpec::Integer,
            Self::Text(_) => KeySpec::Text,
            Self::Bytes(_) => KeySpec::Bytes,
        }
    }

    /// Encodes this key using deterministic CBOR preferred serialization.
    pub fn canonical_bytes(&self) -> std::result::Result<Vec<u8>, KeyError> {
        let mut output = Vec::with_capacity(self.estimated_cbor_size());
        match self {
            Self::Integer(value) => value.cbor_bytes(&mut output),
            Self::Text(value) => {
                encode_argument(&mut output, 3, value.len() as u64);
                output.extend_from_slice(value.as_bytes());
            }
            Self::Bytes(value) => {
                encode_argument(&mut output, 2, value.len() as u64);
                output.extend_from_slice(value);
            }
        }
        if output.len() > MAX_CANONICAL_KEY_BYTES {
            return Err(KeyError::TooLarge {
                size: output.len(),
                maximum: MAX_CANONICAL_KEY_BYTES,
            });
        }
        Ok(output)
    }

    /// Decodes exactly one canonical v1 key item.
    pub fn decode_canonical(bytes: &[u8]) -> std::result::Result<Self, KeyError> {
        if bytes.len() > MAX_CANONICAL_KEY_BYTES {
            return Err(KeyError::TooLarge {
                size: bytes.len(),
                maximum: MAX_CANONICAL_KEY_BYTES,
            });
        }
        let mut cursor = Cursor::new(bytes);
        let key = cursor.parse_key()?;
        if !cursor.is_empty() {
            return Err(KeyError::TrailingBytes);
        }
        let canonical = key.canonical_bytes()?;
        if canonical != bytes {
            return Err(KeyError::NonCanonical);
        }
        Ok(key)
    }

    fn estimated_cbor_size(&self) -> usize {
        match self {
            Self::Integer(value) => value.magnitude.len().saturating_add(10),
            Self::Text(value) => value.len().saturating_add(9),
            Self::Bytes(value) => value.len().saturating_add(9),
        }
    }
}

impl From<PortableInteger> for PortableKey {
    fn from(value: PortableInteger) -> Self {
        Self::Integer(value)
    }
}

impl From<String> for PortableKey {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for PortableKey {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<u8>> for PortableKey {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for PortableKey {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_owned())
    }
}

impl From<&Vec<u8>> for PortableKey {
    fn from(value: &Vec<u8>) -> Self {
        Self::Bytes(value.clone())
    }
}

impl<const N: usize> From<[u8; N]> for PortableKey {
    fn from(value: [u8; N]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl<const N: usize> From<&[u8; N]> for PortableKey {
    fn from(value: &[u8; N]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl_signed_integer!(i8, i16, i32, i64, i128, isize);
impl_unsigned_integer!(u8, u16, u32, u64, u128, usize);

fn encode_argument(output: &mut Vec<u8>, major: u8, value: u64) {
    debug_assert!(major <= 7);
    let prefix = major << 5;
    if value <= 23 {
        output.push(prefix | value as u8);
    } else if value <= u8::MAX as u64 {
        output.push(prefix | 24);
        output.push(value as u8);
    } else if value <= u16::MAX as u64 {
        output.push(prefix | 25);
        output.extend_from_slice(&(value as u16).to_be_bytes());
    } else if value <= u32::MAX as u64 {
        output.push(prefix | 26);
        output.extend_from_slice(&(value as u32).to_be_bytes());
    } else {
        output.push(prefix | 27);
        output.extend_from_slice(&value.to_be_bytes());
    }
}

fn as_u64(magnitude: &[u8]) -> Option<u64> {
    if magnitude.len() > 8 {
        return None;
    }
    let mut value = 0_u64;
    for byte in magnitude {
        value = (value << 8) | u64::from(*byte);
    }
    Some(value)
}

fn subtract_one(magnitude: &[u8]) -> Vec<u8> {
    debug_assert!(!magnitude.is_empty());
    let mut output = magnitude.to_vec();
    for byte in output.iter_mut().rev() {
        if *byte != 0 {
            *byte -= 1;
            break;
        }
        *byte = u8::MAX;
    }
    let first_nonzero = output
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(output.len());
    output[first_nonzero..].to_vec()
}

fn add_one(magnitude: &[u8]) -> std::result::Result<Vec<u8>, KeyError> {
    let mut output = magnitude.to_vec();
    if output.is_empty() {
        return Err(KeyError::NonCanonical);
    }
    for byte in output.iter_mut().rev() {
        if *byte != u8::MAX {
            *byte += 1;
            return Ok(output);
        }
        *byte = 0;
    }
    output.insert(0, 1);
    Ok(output)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct ValidatedCanonicalKey<'a> {
    bytes: &'a [u8],
    spec: KeySpec,
}

impl<'a> ValidatedCanonicalKey<'a> {
    const fn generated(bytes: &'a [u8], spec: KeySpec) -> Self {
        Self { bytes, spec }
    }

    pub(crate) const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    pub(crate) const fn spec(self) -> KeySpec {
        self.spec
    }
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    const fn is_empty(&self) -> bool {
        self.position == self.bytes.len()
    }

    fn read_byte(&mut self) -> std::result::Result<u8, KeyError> {
        let byte = self
            .bytes
            .get(self.position)
            .copied()
            .ok_or(KeyError::Truncated)?;
        self.position += 1;
        Ok(byte)
    }

    fn read_exact(&mut self, length: usize) -> std::result::Result<&'a [u8], KeyError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(KeyError::Overflow)?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or(KeyError::Truncated)?;
        self.position = end;
        Ok(bytes)
    }

    fn read_argument(&mut self, additional: u8) -> std::result::Result<(u64, bool), KeyError> {
        match additional {
            value @ 0..=23 => Ok((u64::from(value), true)),
            24 => {
                let value = u64::from(self.read_byte()?);
                Ok((value, value >= 24))
            }
            25 => {
                let bytes: [u8; 2] = self.read_exact(2)?.try_into().unwrap();
                let value = u64::from(u16::from_be_bytes(bytes));
                Ok((value, value > u64::from(u8::MAX)))
            }
            26 => {
                let bytes: [u8; 4] = self.read_exact(4)?.try_into().unwrap();
                let value = u64::from(u32::from_be_bytes(bytes));
                Ok((value, value > u64::from(u16::MAX)))
            }
            27 => {
                let bytes: [u8; 8] = self.read_exact(8)?.try_into().unwrap();
                let value = u64::from_be_bytes(bytes);
                Ok((value, value > u64::from(u32::MAX)))
            }
            _ => Err(KeyError::UnsupportedType),
        }
    }

    fn parse_header(&mut self) -> std::result::Result<(u8, u64), KeyError> {
        let header = self.read_byte()?;
        let major = header >> 5;
        let additional = header & 0x1f;
        let (value, preferred) = self.read_argument(additional)?;
        if !preferred {
            return Err(KeyError::NonCanonical);
        }
        Ok((major, value))
    }

    fn read_borrowed_bytes(&mut self, length: u64) -> std::result::Result<&'a [u8], KeyError> {
        let length = usize::try_from(length).map_err(|_| KeyError::Overflow)?;
        if length > MAX_CANONICAL_KEY_BYTES {
            return Err(KeyError::TooLarge {
                size: length,
                maximum: MAX_CANONICAL_KEY_BYTES,
            });
        }
        self.read_exact(length)
    }

    fn parse_bytes(&mut self, length: u64) -> std::result::Result<Vec<u8>, KeyError> {
        Ok(self.read_borrowed_bytes(length)?.to_vec())
    }

    fn validate_key(&mut self) -> std::result::Result<KeySpec, KeyError> {
        let header = self
            .bytes
            .get(self.position)
            .copied()
            .ok_or(KeyError::Truncated)?;
        let major = header >> 5;
        if major == 6 {
            let _ = self.read_byte()?;
            let (tag, preferred) = self.read_argument(header & 0x1f)?;
            if !preferred || !matches!(tag, 2 | 3) {
                return Err(KeyError::UnsupportedType);
            }
            let (byte_major, length) = self.parse_header()?;
            if byte_major != 2 {
                return Err(KeyError::UnsupportedType);
            }
            let magnitude = self.read_borrowed_bytes(length)?;
            if magnitude.is_empty() || magnitude[0] == 0 || as_u64(magnitude).is_some() {
                return Err(KeyError::NonCanonical);
            }
            return Ok(KeySpec::Integer);
        }

        let (major, argument) = self.parse_header()?;
        match major {
            0 | 1 => Ok(KeySpec::Integer),
            2 => {
                let _ = self.read_borrowed_bytes(argument)?;
                Ok(KeySpec::Bytes)
            }
            3 => {
                let bytes = self.read_borrowed_bytes(argument)?;
                std::str::from_utf8(bytes).map_err(|_| KeyError::InvalidUtf8)?;
                Ok(KeySpec::Text)
            }
            _ => Err(KeyError::UnsupportedType),
        }
    }

    fn parse_key(&mut self) -> std::result::Result<PortableKey, KeyError> {
        let header = self
            .bytes
            .get(self.position)
            .copied()
            .ok_or(KeyError::Truncated)?;
        let major = header >> 5;
        if major == 6 {
            let _ = self.read_byte()?;
            let (tag, preferred) = self.read_argument(header & 0x1f)?;
            if !preferred || !matches!(tag, 2 | 3) {
                return Err(KeyError::UnsupportedType);
            }
            let (byte_major, length) = self.parse_header()?;
            if byte_major != 2 {
                return Err(KeyError::UnsupportedType);
            }
            let magnitude = self.parse_bytes(length)?;
            if magnitude.is_empty() || magnitude[0] == 0 {
                return Err(KeyError::NonCanonical);
            }
            let integer = if tag == 2 {
                if as_u64(&magnitude).is_some() {
                    return Err(KeyError::NonCanonical);
                }
                PortableInteger::from_parts(false, &magnitude).map_err(|_| KeyError::TooLarge {
                    size: magnitude.len(),
                    maximum: MAX_CANONICAL_KEY_BYTES,
                })?
            } else {
                let actual_magnitude = add_one(&magnitude)?;
                if as_u64(&actual_magnitude).is_some() {
                    return Err(KeyError::NonCanonical);
                }
                PortableInteger::from_parts(true, &actual_magnitude).map_err(|_| {
                    KeyError::TooLarge {
                        size: actual_magnitude.len(),
                        maximum: MAX_CANONICAL_KEY_BYTES,
                    }
                })?
            };
            return Ok(PortableKey::Integer(integer));
        }

        let (major, argument) = self.parse_header()?;
        match major {
            0 => Ok(PortableKey::Integer(PortableInteger::from_u128(
                u128::from(argument),
            ))),
            1 => Ok(PortableKey::Integer(PortableInteger::from_parts(
                true,
                &u128::from(argument)
                    .checked_add(1)
                    .ok_or(KeyError::Overflow)?
                    .to_be_bytes(),
            )?)),
            2 => Ok(PortableKey::Bytes(self.parse_bytes(argument)?)),
            3 => {
                let bytes = self.parse_bytes(argument)?;
                let value = String::from_utf8(bytes).map_err(|_| KeyError::InvalidUtf8)?;
                Ok(PortableKey::Text(value))
            }
            _ => Err(KeyError::UnsupportedType),
        }
    }
}

pub(crate) fn validate_canonical_key(
    bytes: &[u8],
) -> std::result::Result<ValidatedCanonicalKey<'_>, KeyError> {
    if bytes.len() > MAX_CANONICAL_KEY_BYTES {
        return Err(KeyError::TooLarge {
            size: bytes.len(),
            maximum: MAX_CANONICAL_KEY_BYTES,
        });
    }
    let mut cursor = Cursor::new(bytes);
    let spec = cursor.validate_key()?;
    if !cursor.is_empty() {
        return Err(KeyError::TrailingBytes);
    }
    Ok(ValidatedCanonicalKey { bytes, spec })
}

/// Errors from key conversion or canonical key decoding.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum KeyError {
    /// A key exceeded the common SDK size limit.
    #[error("canonical key is too large: {size} bytes exceeds {maximum}")]
    TooLarge {
        /// Actual canonical key byte length.
        size: usize,
        /// Maximum accepted byte length.
        maximum: usize,
    },
    /// A key did not match the configured keyspace specification.
    #[error("key type {actual} does not match key spec {expected}")]
    KeySpecMismatch {
        /// Configured keyspace type.
        expected: KeySpec,
        /// Supplied logical key type.
        actual: KeySpec,
    },
    /// A canonical key contained bytes after its complete CBOR item.
    #[error("canonical key contains trailing bytes")]
    TrailingBytes,
    /// A key used a non-preferred or otherwise invalid CBOR representation.
    #[error("canonical key is not deterministic preferred CBOR")]
    NonCanonical,
    /// A key used an unsupported CBOR major type or tag.
    #[error("canonical key contains an unsupported CBOR item")]
    UnsupportedType,
    /// A key contained invalid UTF-8 text.
    #[error("text key is not valid UTF-8")]
    InvalidUtf8,
    /// A key's CBOR bytes ended before the complete item was read.
    #[error("canonical key is truncated")]
    Truncated,
    /// A namespace ID of zero is reserved by the protocol.
    #[error("namespace ID must be a positive server-assigned identity")]
    InvalidNamespace,
    /// A CBOR length or integer argument exceeded the implementation range.
    #[error("canonical key length or integer argument overflows")]
    Overflow,
}

/// Converts and validates a key against one configured keyspace specification.
pub fn canonical_key_bytes(
    spec: KeySpec,
    key: impl Into<PortableKey>,
) -> std::result::Result<Vec<u8>, KeyError> {
    let key = key.into();
    if key.spec() != spec {
        return Err(KeyError::KeySpecMismatch {
            expected: spec,
            actual: key.spec(),
        });
    }
    key.canonical_bytes()
}

/// Exact fixed-size item ID sent through the OpenKache protocol.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ItemId([u8; ITEM_ID_BYTES]);

impl ItemId {
    /// Wraps an exact item ID without hashing it again.
    pub const fn from_bytes(bytes: [u8; ITEM_ID_BYTES]) -> Self {
        Self(bytes)
    }

    /// Copies an exact item ID from a language binding or dynamic buffer.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Exactly 32 opaque item ID bytes.
    ///
    /// # Returns
    ///
    /// An item ID that preserves the supplied bytes without hashing.
    ///
    /// # Errors
    ///
    /// Returns an error when `bytes` does not contain exactly 32 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let exact: &[u8; ITEM_ID_BYTES] = bytes.try_into().map_err(|_| {
            Error::configuration(
                "item_id",
                format!(
                    "must contain exactly {ITEM_ID_BYTES} bytes, got {}",
                    bytes.len()
                ),
            )
        })?;
        Ok(Self::from_bytes(*exact))
    }

    /// Returns the exact wire bytes.
    pub const fn as_bytes(&self) -> &[u8; ITEM_ID_BYTES] {
        &self.0
    }

    /// Consumes the item ID and returns its exact wire bytes.
    pub const fn into_bytes(self) -> [u8; ITEM_ID_BYTES] {
        self.0
    }

    pub(crate) const fn into_protocol(self) -> openkache_protocol::ItemId {
        openkache_protocol::ItemId::new(self.0)
    }
}

impl AsRef<[u8]> for ItemId {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<[u8; ITEM_ID_BYTES]> for ItemId {
    fn from(bytes: [u8; ITEM_ID_BYTES]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl TryFrom<&[u8]> for ItemId {
    type Error = Error;

    fn try_from(bytes: &[u8]) -> Result<Self> {
        Self::from_slice(bytes)
    }
}

/// Application-managed root secret used to derive Item IDs and value keys.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ClientRootKey {
    master_key: [u8; DATA_PROTECTION_KEY_BYTES],
    item_id_root: [u8; DATA_PROTECTION_KEY_BYTES],
}

impl ClientRootKey {
    /// Creates a client root key from exact bytes.
    pub fn from_bytes(bytes: [u8; DATA_PROTECTION_KEY_BYTES]) -> Self {
        let item_id_root =
            blake3::derive_key(crate::contract::VALUE_FORMAT_ITEM_ID_ROOT_CONTEXT, &bytes);
        Self {
            master_key: bytes,
            item_id_root,
        }
    }

    /// Returns the all-zero root used for the unprotected formatted default.
    pub fn zero() -> Self {
        Self::from_bytes([0; DATA_PROTECTION_KEY_BYTES])
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.master_key.iter().all(|byte| *byte == 0)
    }

    /// Copies an exact root key from a language binding or configuration buffer.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Exactly 32 random secret bytes.
    ///
    /// # Returns
    ///
    /// An owned client root key.
    ///
    /// # Errors
    ///
    /// Returns an error when `bytes` does not contain exactly 32 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let exact: &[u8; DATA_PROTECTION_KEY_BYTES] = bytes.try_into().map_err(|_| {
            Error::configuration(
                "client_root_key",
                format!(
                    "must contain exactly {DATA_PROTECTION_KEY_BYTES} bytes, got {}",
                    bytes.len()
                ),
            )
        })?;
        Ok(Self::from_bytes(*exact))
    }

    /// Decodes a Base64-encoded 32-byte root secret.
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
                .map_err(|error| Error::configuration("client_root_key", error.to_string()))?,
        );
        if decoded.len() != DATA_PROTECTION_KEY_BYTES {
            return Err(Error::configuration(
                "client_root_key",
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

    /// Derives a namespace-bound Item ID for a typed portable key.
    ///
    /// # Arguments
    ///
    /// * `namespace_id` - Positive server-assigned namespace identity.
    /// * `key` - One v1 [`PortableKey`] value.
    ///
    /// # Returns
    ///
    /// The deterministic Item ID scoped to this root key and namespace.
    pub fn derive_item_id_in_namespace(
        &self,
        namespace_id: u64,
        key: impl Into<PortableKey>,
    ) -> std::result::Result<ItemId, KeyError> {
        if namespace_id == 0 {
            return Err(KeyError::InvalidNamespace);
        }
        let key = key.into();
        let spec = key.spec();
        let canonical_key = key.canonical_bytes()?;
        self.derive_item_id_from_validated_canonical_key(
            namespace_id,
            ValidatedCanonicalKey::generated(&canonical_key, spec),
        )
    }

    /// Derives an Item ID from already canonical deterministic-CBOR key bytes.
    ///
    /// This is the boundary used by language adapters that perform native-value
    /// conversion outside the Rust core. The bytes are validated before hashing,
    /// so a caller cannot smuggle a non-canonical representation.
    pub fn derive_item_id_from_canonical_key(
        &self,
        namespace_id: u64,
        canonical_key: &[u8],
    ) -> std::result::Result<ItemId, KeyError> {
        if namespace_id == 0 {
            return Err(KeyError::InvalidNamespace);
        }
        let canonical_key = validate_canonical_key(canonical_key)?;
        self.derive_item_id_from_validated_canonical_key(namespace_id, canonical_key)
    }

    pub(crate) fn derive_item_id_from_validated_canonical_key(
        &self,
        namespace_id: u64,
        canonical_key: ValidatedCanonicalKey<'_>,
    ) -> std::result::Result<ItemId, KeyError> {
        if namespace_id == 0 {
            return Err(KeyError::InvalidNamespace);
        }
        let mut hasher = blake3::Hasher::new_keyed(&self.item_id_root);
        hasher.update(&namespace_id.to_be_bytes());
        hasher.update(canonical_key.bytes());
        Ok(ItemId::from_bytes(*hasher.finalize().as_bytes()))
    }

    /// Legacy byte-key convenience using namespace `1`.
    ///
    /// New formatted clients should configure a [`KeySpec`] and call
    /// [`Self::derive_item_id_in_namespace`]. This method remains available
    /// for raw-byte application callers while the public SDKs migrate.
    pub fn derive_item_id(&self, application_key: impl AsRef<[u8]>) -> ItemId {
        self.derive_item_id_in_namespace(1, PortableKey::Bytes(application_key.as_ref().to_vec()))
            .expect("legacy application key exceeds the v1 key limit")
    }

    pub(crate) fn master_key(&self) -> Zeroizing<[u8; DATA_PROTECTION_KEY_BYTES]> {
        Zeroizing::new(self.master_key)
    }
}

/// Backwards-compatible spelling retained while bindings migrate to
/// [`ClientRootKey`].
pub type DataProtectionKey = ClientRootKey;

impl TryFrom<&[u8]> for ClientRootKey {
    type Error = Error;

    fn try_from(bytes: &[u8]) -> Result<Self> {
        Self::from_slice(bytes)
    }
}
