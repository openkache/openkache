//! Typed-key conversion, canonical CBOR encoding, and Item ID derivation.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::internal_core::{Error, Result};

pub(crate) const PROTECTION_KEY_BYTES: usize =
    crate::internal_core::contract::VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES;

/// Bytes in an application-managed data protection key.
pub const DATA_PROTECTION_KEY_BYTES: usize = PROTECTION_KEY_BYTES;
/// Bytes in the client root key.
pub const CLIENT_ROOT_KEY_BYTES: usize = PROTECTION_KEY_BYTES;
/// Maximum canonical key bytes accepted by every conforming SDK.
pub use crate::internal_core::contract::MAX_CANONICAL_KEY_BYTES;
/// Maximum application key input bytes accepted by the transitional API.
///
/// This alias is retained for source compatibility.  Validation is performed
/// against the complete canonical CBOR item, not this pre-encoding length.
pub const MAX_KEY_INPUT_BYTES: usize = MAX_CANONICAL_KEY_BYTES;
/// Maximum Item ID bytes accepted by the wire protocol.
pub const MAX_ITEM_ID_BYTES: usize = crate::internal_protocol::MAX_ITEM_ID_BYTES;
const NAMESPACE_HASH_DOMAIN: &[u8] = b"openkache/item-id/namespace-hash/v1";
const PUBLIC_KEY_OR_HASH_DOMAIN: &[u8] = b"openkache/item-id/public-key-or-hash/v1";

/// Client-owned application-key to Item ID mapping profile.
///
/// This setting is local to a client. It is never serialized into a wire
/// frame or interpreted by the server.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum KeyFormat {
    /// Deterministic CBOR followed by namespace-bound BLAKE3 hashing.
    #[default]
    NamespaceHash,
    /// Preserve short canonical key bytes and hash longer canonical keys.
    PublicKeyOrHash,
    /// Compatibility profile that preserves the pre-contract raw-byte path.
    ///
    /// This profile is intentionally not an alias for [`Self::PublicKeyOrHash`]:
    /// existing callers may rely on raw bytes (rather than canonical CBOR) and
    /// the legacy keyed hash framing for oversized values.
    ByteKeyOrHash,
    /// Compatibility profile for the pre-domain-separation hash framing.
    ///
    /// New code must use [`Self::NamespaceHash`]. This variant remains
    /// explicit so callers that must address existing data do not silently
    /// reinterpret those Item IDs under the v1 domain-separated profile.
    Hash,
}

/// The language-neutral typed-key variant inferred for one operation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum KeyType {
    /// Mathematical signed or unsigned integer identity.
    Integer,
    /// Exact valid UTF-8 text identity.
    Text,
    /// Exact byte-string identity, including empty and NUL-containing values.
    Bytes,
}

impl KeyType {
    /// Returns the stable lower-case name used in diagnostics.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Integer => "integer",
            Self::Text => "text",
            Self::Bytes => "bytes",
        }
    }

    /// Parses the stable lower-case name used by language adapter options.
    pub fn from_name(value: &str) -> Option<Self> {
        match value {
            "integer" => Some(Self::Integer),
            "text" => Some(Self::Text),
            "bytes" => Some(Self::Bytes),
            _ => None,
        }
    }
}

/// Explicit key-type/profile resolver shared by low-level callers.
///
/// `KeySpace` is useful when an ABI or compatibility API carries an explicit
/// discriminator. It is not a namespace policy: high-level v1 operations
/// infer `TypedKey` per call and can use [`ResolvedKey`] directly.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeySpace {
    key_type: KeyType,
    format: KeyFormat,
}

impl KeySpace {
    /// Creates a resolver for one logical key type.
    pub const fn new(key_type: KeyType) -> Self {
        Self {
            key_type,
            format: KeyFormat::NamespaceHash,
        }
    }

    /// Creates a key space with an explicit mapping profile.
    pub const fn with_format(key_type: KeyType, format: KeyFormat) -> Self {
        Self { key_type, format }
    }

    /// Returns the logical key type enforced by this resolver.
    pub const fn key_type(self) -> KeyType {
        self.key_type
    }

    /// Returns the configured client-owned mapping profile.
    pub const fn format(self) -> KeyFormat {
        self.format
    }

    /// Validates that the selected mapping profile is compatible with the
    /// configured logical key type.
    pub fn validate(self) -> std::result::Result<(), KeyError> {
        if self.format == KeyFormat::ByteKeyOrHash && self.key_type != KeyType::Bytes {
            return Err(KeyError::InvalidFormatForKeyType {
                format: self.format,
                key_type: self.key_type,
            });
        }
        Ok(())
    }

    /// Resolves one typed logical key and owns its canonical representation.
    pub fn resolve(self, key: impl Into<TypedKey>) -> std::result::Result<ResolvedKey, KeyError> {
        self.validate()?;
        let key = key.into();
        ensure_key_type(self.key_type, key.key_type())?;
        if self.format == KeyFormat::ByteKeyOrHash {
            if let TypedKey::Bytes(bytes) = key {
                return ResolvedKey::from_direct_bytes(bytes);
            }
        }
        ResolvedKey::from_typed(key)
    }

    /// Resolves logical bytes using the configured key type.
    pub fn resolve_logical_bytes(self, value: &[u8]) -> std::result::Result<ResolvedKey, KeyError> {
        self.validate()?;
        if self.format == KeyFormat::ByteKeyOrHash {
            return ResolvedKey::from_direct_bytes(value.to_owned());
        }
        let key = typed_from_logical_bytes(self.key_type, value)?;
        ResolvedKey::from_typed(key)
    }

    /// Resolves one complete canonical key after enforcing this key type.
    pub fn resolve_canonical(
        self,
        canonical_key: &[u8],
    ) -> std::result::Result<ResolvedKey, KeyError> {
        self.validate()?;
        let typed = TypedKey::decode_canonical(canonical_key)?;
        ensure_key_type(self.key_type, typed.key_type())?;
        if self.format == KeyFormat::ByteKeyOrHash {
            if let TypedKey::Bytes(bytes) = typed {
                return ResolvedKey::from_direct_bytes(bytes);
            }
        }
        ResolvedKey::from_typed(typed)
    }
}

impl fmt::Display for KeyType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Signed-i64 integer used by [`TypedKey`].
///
/// The magnitude is stored as minimal big-endian bytes so native and ABI
/// callers can share one representation while the public encoder enforces the
/// v1 signed-i64 range. Zero is always non-negative.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TypedInteger {
    negative: bool,
    magnitude: Vec<u8>,
}

impl TypedInteger {
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
    /// resource limit. The signed-i64 range is checked when the key is
    /// encoded.
    pub fn from_parts(negative: bool, magnitude: &[u8]) -> std::result::Result<Self, KeyError> {
        let first_nonzero = magnitude
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(magnitude.len());
        let magnitude = magnitude[first_nonzero..].to_vec();
        if magnitude.len() > MAX_KEY_INPUT_BYTES {
            return Err(KeyError::TooLarge {
                size: magnitude.len(),
                maximum: MAX_KEY_INPUT_BYTES,
            });
        }
        Ok(Self {
            negative: negative && !magnitude.is_empty(),
            magnitude,
        })
    }

    /// Parses a signed decimal integer.
    ///
    /// Decimal text is the neutral representation used by the typed native
    /// ABI so bindings do not need to implement CBOR serialization.
    pub fn from_decimal(value: &str) -> std::result::Result<Self, KeyError> {
        let bytes = value.as_bytes();
        let (negative, digits) = match bytes.first().copied() {
            Some(b'-') => (true, &bytes[1..]),
            _ => (false, bytes),
        };
        if digits.is_empty()
            || digits.iter().any(|digit| !digit.is_ascii_digit())
            || (digits.len() > 1 && digits[0] == b'0')
        {
            return Err(KeyError::InvalidInteger);
        }
        if negative && digits == b"0" {
            return Err(KeyError::InvalidInteger);
        }
        let mut magnitude = Vec::new();
        for digit in digits {
            let mut carry = u16::from(*digit - b'0');
            for byte in magnitude.iter_mut().rev() {
                let product = u16::from(*byte) * 10 + carry;
                *byte = product as u8;
                carry = product >> 8;
            }
            if carry != 0 {
                magnitude.insert(0, carry as u8);
            }
            if magnitude.len() > MAX_KEY_INPUT_BYTES {
                return Err(KeyError::TooLarge {
                    size: magnitude.len(),
                    maximum: MAX_KEY_INPUT_BYTES,
                });
            }
        }
        Self::from_parts(negative, &magnitude)
    }

    /// Returns whether this integer is negative.
    pub const fn is_negative(&self) -> bool {
        self.negative
    }

    /// Returns the minimal big-endian magnitude.
    pub fn magnitude(&self) -> &[u8] {
        &self.magnitude
    }

    fn as_i64(&self) -> Option<i64> {
        let magnitude = as_u64(&self.magnitude)?;
        if self.negative {
            if magnitude == 1_u64 << 63 {
                Some(i64::MIN)
            } else {
                i64::try_from(magnitude)
                    .ok()
                    .and_then(|value| value.checked_neg())
            }
        } else {
            i64::try_from(magnitude).ok()
        }
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
            impl From<$type> for TypedKey {
                fn from(value: $type) -> Self {
                    Self::Integer(TypedInteger::from_i128(value as i128))
                }
            }
        )+
    };
}

macro_rules! impl_unsigned_integer {
    ($($type:ty),+ $(,)?) => {
        $(
            impl From<$type> for TypedKey {
                fn from(value: $type) -> Self {
                    Self::Integer(TypedInteger::from_u128(value as u128))
                }
            }
        )+
    };
}

/// The v1 key-only logical model.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum TypedKey {
    /// A signed-i64 integer.
    Integer(TypedInteger),
    /// Exact valid UTF-8 text.
    Text(String),
    /// Exact bytes, including empty and NUL-containing values.
    Bytes(Vec<u8>),
}

impl TypedKey {
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
        Self::Integer(TypedInteger::from_i128(value))
    }

    /// Returns the key's [`KeyType`].
    pub const fn key_type(&self) -> KeyType {
        match self {
            Self::Integer(_) => KeyType::Integer,
            Self::Text(_) => KeyType::Text,
            Self::Bytes(_) => KeyType::Bytes,
        }
    }

    /// Compatibility spelling for callers migrating from `PortableKey`.
    #[deprecated(note = "use key_type")]
    pub const fn spec(&self) -> KeyType {
        self.key_type()
    }

    /// Returns the key input length measured by the shared contract.
    pub fn input_len(&self) -> usize {
        match self {
            Self::Integer(value) => value.magnitude.len(),
            Self::Text(value) => value.len(),
            Self::Bytes(value) => value.len(),
        }
    }

    /// Validates the key input length before encoding or hashing.
    pub fn validate_input_len(&self) -> std::result::Result<(), KeyError> {
        let size = self.input_len();
        if size > MAX_KEY_INPUT_BYTES {
            return Err(KeyError::TooLarge {
                size,
                maximum: MAX_KEY_INPUT_BYTES,
            });
        }
        Ok(())
    }

    /// Encodes this key using deterministic CBOR preferred serialization.
    pub fn canonical_bytes(&self) -> std::result::Result<Vec<u8>, KeyError> {
        self.validate_input_len()?;
        if let Self::Integer(value) = self {
            if value.as_i64().is_none() {
                return Err(KeyError::IntegerOutOfRange);
            }
        }
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

fn typed_from_logical_bytes(
    key_type: KeyType,
    value: &[u8],
) -> std::result::Result<TypedKey, KeyError> {
    match key_type {
        KeyType::Text => String::from_utf8(value.to_owned())
            .map(TypedKey::Text)
            .map_err(|_| KeyError::InvalidUtf8),
        KeyType::Bytes => Ok(TypedKey::Bytes(value.to_owned())),
        KeyType::Integer => {
            let value = std::str::from_utf8(value).map_err(|_| KeyError::InvalidInteger)?;
            TypedInteger::from_decimal(value).map(TypedKey::Integer)
        }
    }
}

fn ensure_key_type(expected: KeyType, actual: KeyType) -> std::result::Result<(), KeyError> {
    if actual == expected {
        Ok(())
    } else {
        Err(KeyError::KeyTypeMismatch { expected, actual })
    }
}

/// A validated key at the boundary between language adapters and the core.
///
/// `ResolvedKey` is deliberately the only representation accepted by the
/// protection hot path. It keeps the key-space discriminator and exact
/// canonical bytes used for Item ID hashing, so a request never re-encodes or
/// re-parses a key as it moves through the client layers.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResolvedKey {
    key_type: KeyType,
    canonical: Vec<u8>,
    /// Raw bytes retained only for the explicitly deprecated
    /// `ByteKeyOrHash` compatibility profile.
    legacy_direct: Option<Vec<u8>>,
}

/// Borrowed canonical key used by the compatibility protection facade.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ValidatedCanonicalKey<'a> {
    bytes: &'a [u8],
    spec: KeyType,
}

impl<'a> ValidatedCanonicalKey<'a> {
    pub(crate) const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    pub(crate) const fn spec(self) -> KeyType {
        self.spec
    }
}

/// Validates one complete deterministic-CBOR key without allocating a second
/// key-sized buffer.
pub(crate) fn validate_canonical_key(
    bytes: &[u8],
) -> std::result::Result<ValidatedCanonicalKey<'_>, KeyError> {
    if bytes.len() > MAX_CANONICAL_KEY_BYTES {
        return Err(KeyError::TooLarge {
            size: bytes.len(),
            maximum: MAX_CANONICAL_KEY_BYTES,
        });
    }
    let typed = TypedKey::decode_canonical(bytes)?;
    Ok(ValidatedCanonicalKey {
        bytes,
        spec: typed.key_type(),
    })
}

impl ResolvedKey {
    /// Converts one typed logical key and stores its canonical bytes.
    pub fn from_typed(key: impl Into<TypedKey>) -> std::result::Result<Self, KeyError> {
        let typed = key.into();
        let canonical = typed.canonical_bytes()?;
        Ok(Self {
            key_type: typed.key_type(),
            canonical,
            legacy_direct: None,
        })
    }

    fn from_direct_bytes(bytes: Vec<u8>) -> std::result::Result<Self, KeyError> {
        if bytes.len() > MAX_KEY_INPUT_BYTES {
            return Err(KeyError::TooLarge {
                size: bytes.len(),
                maximum: MAX_KEY_INPUT_BYTES,
            });
        }
        let canonical = TypedKey::Bytes(bytes.clone()).canonical_bytes()?;
        Ok(Self {
            key_type: KeyType::Bytes,
            canonical,
            legacy_direct: Some(bytes),
        })
    }

    /// Validates and adopts one complete canonical v1 key item.
    pub fn from_canonical(bytes: &[u8]) -> std::result::Result<Self, KeyError> {
        let typed = TypedKey::decode_canonical(bytes)?;
        Ok(Self {
            key_type: typed.key_type(),
            canonical: bytes.to_owned(),
            legacy_direct: None,
        })
    }

    /// Returns the logical key's type.
    pub const fn key_type(&self) -> KeyType {
        self.key_type
    }

    /// Borrows the deterministic-CBOR key bytes used by the wire-independent
    /// Item ID derivation algorithm.
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    /// Consumes this key and returns its canonical bytes.
    pub fn into_canonical_bytes(self) -> Vec<u8> {
        self.canonical
    }
}

impl From<TypedInteger> for TypedKey {
    fn from(value: TypedInteger) -> Self {
        Self::Integer(value)
    }
}

impl From<String> for TypedKey {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for TypedKey {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<u8>> for TypedKey {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for TypedKey {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_owned())
    }
}

impl From<&Vec<u8>> for TypedKey {
    fn from(value: &Vec<u8>) -> Self {
        Self::Bytes(value.clone())
    }
}

impl<const N: usize> From<[u8; N]> for TypedKey {
    fn from(value: [u8; N]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl<const N: usize> From<&[u8; N]> for TypedKey {
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

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
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

    fn parse_bytes(&mut self, length: u64) -> std::result::Result<Vec<u8>, KeyError> {
        let length = usize::try_from(length).map_err(|_| KeyError::Overflow)?;
        if length > MAX_KEY_INPUT_BYTES {
            return Err(KeyError::TooLarge {
                size: length,
                maximum: MAX_KEY_INPUT_BYTES,
            });
        }
        Ok(self.read_exact(length)?.to_vec())
    }

    fn parse_key(&mut self) -> std::result::Result<TypedKey, KeyError> {
        let header = self
            .bytes
            .get(self.position)
            .copied()
            .ok_or(KeyError::Truncated)?;
        let major = header >> 5;
        if major == 6 {
            // The v1 typed-key subset contains only untagged signed integers,
            // byte strings, and text strings. Standard CBOR bignum tags are
            // deliberately rejected because the portable Integer is signed
            // i64, not arbitrary precision.
            return Err(KeyError::UnsupportedType);
        }

        let (major, argument) = self.parse_header()?;
        match major {
            0 => {
                if argument > i64::MAX as u64 {
                    return Err(KeyError::IntegerOutOfRange);
                }
                Ok(TypedKey::Integer(TypedInteger::from_u128(u128::from(
                    argument,
                ))))
            }
            1 => {
                if argument > i64::MAX as u64 {
                    return Err(KeyError::IntegerOutOfRange);
                }
                Ok(TypedKey::Integer(TypedInteger::from_parts(
                    true,
                    &u128::from(argument)
                        .checked_add(1)
                        .ok_or(KeyError::Overflow)?
                        .to_be_bytes(),
                )?))
            }
            2 => Ok(TypedKey::Bytes(self.parse_bytes(argument)?)),
            3 => {
                let bytes = self.parse_bytes(argument)?;
                let value = String::from_utf8(bytes).map_err(|_| KeyError::InvalidUtf8)?;
                Ok(TypedKey::Text(value))
            }
            _ => Err(KeyError::UnsupportedType),
        }
    }
}

/// Errors from key conversion or canonical key decoding.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum KeyError {
    /// A key exceeded the common SDK size limit.
    #[error("key input is too large: {size} bytes exceeds {maximum}")]
    TooLarge {
        /// Actual measured key-input byte length.
        size: usize,
        /// Maximum accepted byte length.
        maximum: usize,
    },
    /// A key did not match the configured key type.
    #[error("key type {actual} does not match configured key type {expected}")]
    KeyTypeMismatch {
        /// Configured key type.
        expected: KeyType,
        /// Supplied key type.
        actual: KeyType,
    },
    /// Compatibility spelling retained for the pre-contract `KeySpec` API.
    #[error("key type {actual} does not match key spec {expected}")]
    KeySpecMismatch {
        /// Configured key type.
        expected: KeyType,
        /// Supplied key type.
        actual: KeyType,
    },
    /// A mapping profile was paired with an incompatible logical key type.
    #[error("key format {format:?} requires key type bytes, got {key_type:?}")]
    InvalidFormatForKeyType {
        /// Selected client-owned mapping profile.
        format: KeyFormat,
        /// Configured logical key type.
        key_type: KeyType,
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
    /// An integer key was not canonical signed decimal text.
    #[error("integer key is not canonical signed decimal text")]
    InvalidInteger,
    /// A key's CBOR bytes ended before the complete item was read.
    #[error("canonical key is truncated")]
    Truncated,
    /// An integer key is outside the signed i64 contract range.
    #[error("integer key is outside the signed i64 range")]
    IntegerOutOfRange,
    /// A namespace ID of zero is reserved by the protocol.
    #[error("namespace ID must be a positive server-assigned identity")]
    InvalidNamespace,
    /// A CBOR length or integer argument exceeded the implementation range.
    #[error("canonical key length or integer argument overflows")]
    Overflow,
}

/// Converts and validates a key against one configured key type.
pub fn canonical_key_bytes(
    key_type: KeyType,
    key: impl Into<TypedKey>,
) -> std::result::Result<Vec<u8>, KeyError> {
    KeySpace::new(key_type)
        .resolve(key)
        .map(ResolvedKey::into_canonical_bytes)
}

/// Opaque variable-length item ID sent through the OpenKache protocol.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ItemId {
    len: u8,
    bytes: [u8; MAX_ITEM_ID_BYTES],
}

impl ItemId {
    /// Wraps a legacy maximum-width item ID without hashing it again.
    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> Self {
        Self::from_slice(bytes.as_ref()).expect("item ID exceeds the wire length limit")
    }

    /// Copies an exact item ID from a language binding or dynamic buffer.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Zero through 32 opaque item ID bytes.
    ///
    /// # Returns
    ///
    /// An item ID that preserves the supplied bytes without hashing.
    ///
    /// # Errors
    ///
    /// Returns an error when `bytes` contains more than 32 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_ITEM_ID_BYTES {
            return Err(Error::configuration(
                "item_id",
                format!(
                    "must contain at most {MAX_ITEM_ID_BYTES} bytes, got {}",
                    bytes.len()
                ),
            ));
        }
        let mut item_id = Self {
            len: bytes.len() as u8,
            bytes: [0; MAX_ITEM_ID_BYTES],
        };
        item_id.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(item_id)
    }

    /// Returns an exact Item ID while rejecting any length outside the wire
    /// contract.  This named constructor makes the no-mapping path explicit.
    pub fn exact(bytes: &[u8]) -> Result<Self> {
        Self::from_slice(bytes)
    }

    /// Returns the exact wire bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    /// Returns the exact number of wire bytes in this Item ID.
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Reports whether this Item ID contains no bytes.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Consumes the item ID and returns its exact wire bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }

    pub(crate) fn into_protocol(self) -> crate::internal_protocol::ItemId {
        crate::internal_protocol::ItemId::from_slice(self.as_bytes())
            .expect("client ItemId was validated before protocol conversion")
    }
}

impl AsRef<[u8]> for ItemId {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<[u8; MAX_ITEM_ID_BYTES]> for ItemId {
    fn from(bytes: [u8; MAX_ITEM_ID_BYTES]) -> Self {
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
    value_root_key: [u8; DATA_PROTECTION_KEY_BYTES],
}

impl ClientRootKey {
    /// Creates a client root key from exact bytes.
    pub fn from_bytes(bytes: [u8; DATA_PROTECTION_KEY_BYTES]) -> Self {
        let item_id_root = blake3::derive_key(
            crate::internal_core::contract::VALUE_FORMAT_ITEM_ID_ROOT_CONTEXT,
            &bytes,
        );
        let value_root_key = blake3::derive_key(
            crate::internal_core::contract::VALUE_FORMAT_VALUE_ROOT_CONTEXT,
            &bytes,
        );
        Self {
            master_key: bytes,
            item_id_root,
            value_root_key,
        }
    }

    /// Returns the public all-zero Item-ID root.
    ///
    /// Item IDs derived from this root are publicly derivable. Use this value
    /// only when application-key confidentiality is not required; value
    /// protection, when needed, must be configured with an independent
    /// [`crate::internal_core::ValueKeyring`].
    pub fn zero() -> Self {
        Self::from_bytes([0; DATA_PROTECTION_KEY_BYTES])
    }

    /// Returns the public all-zero Item-ID root.
    ///
    /// This named constructor makes the absence of application-key secrecy
    /// explicit at call sites. It is equivalent to [`Self::zero`].
    pub fn public() -> Self {
        Self::zero()
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

    /// Derives a namespace-bound Item ID for a typed key.
    ///
    /// # Arguments
    ///
    /// * `namespace_id` - Positive server-assigned namespace identity.
    /// * `key` - One v1 [`TypedKey`] value.
    ///
    /// # Returns
    ///
    /// The deterministic Item ID scoped to this root key and namespace.
    pub fn derive_item_id_in_namespace(
        &self,
        namespace_id: u64,
        key: impl Into<TypedKey>,
    ) -> std::result::Result<ItemId, KeyError> {
        self.derive_item_id_in_namespace_with_format(namespace_id, KeyFormat::NamespaceHash, key)
    }

    /// Derives an Item ID through one explicit mapping profile.
    pub fn derive_item_id_in_namespace_with_format(
        &self,
        namespace_id: u64,
        format: KeyFormat,
        key: impl Into<TypedKey>,
    ) -> std::result::Result<ItemId, KeyError> {
        let key = key.into();
        if format == KeyFormat::ByteKeyOrHash {
            let TypedKey::Bytes(bytes) = key else {
                return Err(KeyError::InvalidFormatForKeyType {
                    format,
                    key_type: key.key_type(),
                });
            };
            let key = ResolvedKey::from_direct_bytes(bytes)?;
            return self.derive_item_id_for_resolved_key(namespace_id, format, &key);
        }
        let key = ResolvedKey::from_typed(key)?;
        self.derive_item_id_for_resolved_key(namespace_id, format, &key)
    }

    /// Derives an Item ID from already canonical deterministic-CBOR key bytes.
    ///
    /// This is the boundary used by language adapters that perform native-value
    /// conversion outside the Rust core. The bytes are decoded and re-encoded
    /// before hashing, so a caller cannot smuggle a non-canonical representation.
    pub fn derive_item_id_from_canonical_key(
        &self,
        namespace_id: u64,
        canonical_key: &[u8],
    ) -> std::result::Result<ItemId, KeyError> {
        self.derive_item_id_from_canonical_key_with_format(
            namespace_id,
            KeyFormat::NamespaceHash,
            canonical_key,
        )
    }

    /// Derives an Item ID from canonical key bytes using one explicit mapping
    /// profile. The input is decoded and re-encoded before hashing or
    /// preservation, so non-canonical bytes are never silently reinterpreted.
    pub fn derive_item_id_from_canonical_key_with_format(
        &self,
        namespace_id: u64,
        format: KeyFormat,
        canonical_key: &[u8],
    ) -> std::result::Result<ItemId, KeyError> {
        let key = validate_canonical_key(canonical_key)?;
        if format == KeyFormat::ByteKeyOrHash {
            let typed = TypedKey::decode_canonical(key.bytes())?;
            return self.derive_item_id_in_namespace_with_format(namespace_id, format, typed);
        }
        self.derive_item_id_from_validated_canonical_key(namespace_id, format, key)
    }

    /// Derives an Item ID from a key that has already crossed the validated
    /// logical/canonical boundary.
    pub(crate) fn derive_item_id_for_resolved_key(
        &self,
        namespace_id: u64,
        format: KeyFormat,
        key: &ResolvedKey,
    ) -> std::result::Result<ItemId, KeyError> {
        if namespace_id == 0 {
            return Err(KeyError::InvalidNamespace);
        }
        if format == KeyFormat::ByteKeyOrHash {
            if let Some(direct) = key.legacy_direct.as_deref() {
                return Ok(self.legacy_byte_key_or_hash(namespace_id, direct));
            }
            return Err(KeyError::InvalidFormatForKeyType {
                format,
                key_type: key.key_type(),
            });
        }
        let canonical_key = key.canonical_bytes();
        if format == KeyFormat::PublicKeyOrHash && canonical_key.len() <= MAX_ITEM_ID_BYTES {
            return ItemId::from_slice(canonical_key).map_err(|_| KeyError::TooLarge {
                size: canonical_key.len(),
                maximum: MAX_ITEM_ID_BYTES,
            });
        }
        if format == KeyFormat::PublicKeyOrHash {
            return Ok(self.public_hash(canonical_key));
        }
        Ok(self.hash_canonical_key(namespace_id, format, canonical_key))
    }

    pub(crate) fn derive_item_id_from_validated_canonical_key(
        &self,
        namespace_id: u64,
        format: KeyFormat,
        key: ValidatedCanonicalKey<'_>,
    ) -> std::result::Result<ItemId, KeyError> {
        if namespace_id == 0 {
            return Err(KeyError::InvalidNamespace);
        }
        let canonical_key = key.bytes();
        if format == KeyFormat::ByteKeyOrHash {
            let typed = TypedKey::decode_canonical(canonical_key)?;
            let TypedKey::Bytes(bytes) = typed else {
                return Err(KeyError::InvalidFormatForKeyType {
                    format,
                    key_type: key.spec(),
                });
            };
            return Ok(self.legacy_byte_key_or_hash(namespace_id, &bytes));
        }
        if format == KeyFormat::PublicKeyOrHash && canonical_key.len() <= MAX_ITEM_ID_BYTES {
            return ItemId::from_slice(canonical_key).map_err(|_| KeyError::TooLarge {
                size: canonical_key.len(),
                maximum: MAX_ITEM_ID_BYTES,
            });
        }
        if format == KeyFormat::PublicKeyOrHash {
            return Ok(self.public_hash(canonical_key));
        }
        Ok(self.hash_canonical_key(namespace_id, format, canonical_key))
    }

    fn hash_canonical_key(
        &self,
        namespace_id: u64,
        format: KeyFormat,
        canonical_key: &[u8],
    ) -> ItemId {
        let mut hasher = blake3::Hasher::new_keyed(&self.item_id_root);
        if format == KeyFormat::NamespaceHash {
            hasher.update(NAMESPACE_HASH_DOMAIN);
        }
        hasher.update(&namespace_id.to_be_bytes());
        hasher.update(canonical_key);
        ItemId::from_bytes(*hasher.finalize().as_bytes())
    }

    fn legacy_byte_key_or_hash(&self, namespace_id: u64, direct_key: &[u8]) -> ItemId {
        if direct_key.len() <= MAX_ITEM_ID_BYTES {
            return ItemId::from_slice(direct_key).expect("direct key length was validated");
        }
        let mut hasher = blake3::Hasher::new_keyed(&self.item_id_root);
        hasher.update(&namespace_id.to_be_bytes());
        hasher.update(direct_key);
        ItemId::from_bytes(*hasher.finalize().as_bytes())
    }

    fn public_hash(&self, canonical_key: &[u8]) -> ItemId {
        let mut hasher = blake3::Hasher::new();
        hasher.update(PUBLIC_KEY_OR_HASH_DOMAIN);
        hasher.update(canonical_key);
        ItemId::from_bytes(*hasher.finalize().as_bytes())
    }

    /// Derives an Item ID with the public preserve-or-hash profile.
    pub fn derive_public_key_or_hash_in_namespace(
        &self,
        namespace_id: u64,
        key: impl Into<TypedKey>,
    ) -> std::result::Result<ItemId, KeyError> {
        let key = ResolvedKey::from_typed(key)?;
        self.derive_item_id_for_resolved_key(namespace_id, KeyFormat::PublicKeyOrHash, &key)
    }

    /// Derives an Item ID through the pre-contract hash framing.
    ///
    /// This is an explicitly named compatibility path for data written before
    /// the v1 `NamespaceHash` domain string was introduced.
    #[deprecated(note = "use derive_item_id_in_namespace for v1 mapping")]
    pub fn derive_legacy_hash_in_namespace(
        &self,
        namespace_id: u64,
        key: impl Into<TypedKey>,
    ) -> std::result::Result<ItemId, KeyError> {
        let key = ResolvedKey::from_typed(key)?;
        self.derive_item_id_for_resolved_key(namespace_id, KeyFormat::Hash, &key)
    }

    /// Derives an Item ID through the pre-contract raw-byte profile.
    ///
    /// This path preserves the supplied bytes directly when they fit and
    /// applies the old keyed hash framing otherwise. It does not canonicalize
    /// or reinterpret the input.
    #[deprecated(note = "use derive_public_key_or_hash_in_namespace for v1 mapping")]
    pub fn derive_legacy_byte_key_or_hash_in_namespace(
        &self,
        namespace_id: u64,
        direct_key: impl AsRef<[u8]>,
    ) -> std::result::Result<ItemId, KeyError> {
        if namespace_id == 0 {
            return Err(KeyError::InvalidNamespace);
        }
        let direct_key = direct_key.as_ref();
        if direct_key.len() > MAX_KEY_INPUT_BYTES {
            return Err(KeyError::TooLarge {
                size: direct_key.len(),
                maximum: MAX_KEY_INPUT_BYTES,
            });
        }
        Ok(self.legacy_byte_key_or_hash(namespace_id, direct_key))
    }

    /// Compatibility path for the old direct-byte profile.
    ///
    /// Unlike [`Self::derive_public_key_or_hash_in_namespace`], this method
    /// treats the supplied bytes as the complete legacy application key. It
    /// does not wrap them in canonical CBOR before preserving or hashing.
    #[deprecated(note = "use derive_public_key_or_hash_in_namespace")]
    pub fn derive_byte_key_or_hash_in_namespace(
        &self,
        namespace_id: u64,
        direct_key: impl AsRef<[u8]>,
    ) -> std::result::Result<ItemId, KeyError> {
        if namespace_id == 0 {
            return Err(KeyError::InvalidNamespace);
        }
        let direct_key = direct_key.as_ref();
        if direct_key.len() > MAX_KEY_INPUT_BYTES {
            return Err(KeyError::TooLarge {
                size: direct_key.len(),
                maximum: MAX_KEY_INPUT_BYTES,
            });
        }
        Ok(self.legacy_byte_key_or_hash(namespace_id, direct_key))
    }

    /// Fallible legacy byte-key convenience using namespace `1`.
    ///
    /// This method applies the deterministic `Hash` profile. Use
    /// [`Self::derive_byte_key_or_hash_in_namespace`] when the direct
    /// preserve-or-hash profile is required.
    pub fn try_derive_item_id(
        &self,
        application_key: impl AsRef<[u8]>,
    ) -> std::result::Result<ItemId, KeyError> {
        let key = ResolvedKey::from_typed(TypedKey::Bytes(application_key.as_ref().to_vec()))?;
        self.derive_item_id_for_resolved_key(1, KeyFormat::Hash, &key)
    }

    /// Legacy byte-key convenience using namespace `1`.
    ///
    /// New formatted clients should configure a [`KeyType`] and call
    /// [`Self::derive_item_id_in_namespace`]. This method remains available
    /// for legacy application callers while the public SDKs migrate. Prefer
    /// [`Self::try_derive_item_id`] for new code so an oversized application
    /// key is returned as a validation error instead of panicking.
    pub fn derive_item_id(&self, application_key: impl AsRef<[u8]>) -> ItemId {
        self.try_derive_item_id(application_key)
            .expect("legacy application key exceeds the key input limit")
    }

    /// Returns a zeroizing copy of the root secret used by value keyrings.
    ///
    /// The value envelope intentionally derives its compatibility keyring
    /// directly from the configured root. Keep this accessor crate-private so
    /// bindings cannot retain key material beyond the core-owned codec.
    pub(crate) fn master_key(&self) -> Zeroizing<[u8; DATA_PROTECTION_KEY_BYTES]> {
        Zeroizing::new(self.master_key)
    }
}

/// Backwards-compatible spelling retained while bindings migrate to
/// [`ClientRootKey`].
pub type DataProtectionKey = ClientRootKey;

/// Compatibility spelling retained while bindings migrate to [`KeyType`].
#[deprecated(note = "use KeyType")]
pub type KeySpec = KeyType;
/// Compatibility spelling retained while bindings migrate to [`TypedInteger`].
#[deprecated(note = "use TypedInteger")]
pub type PortableInteger = TypedInteger;
/// Compatibility spelling retained while bindings migrate to [`TypedKey`].
#[deprecated(note = "use TypedKey")]
pub type PortableKey = TypedKey;

impl TryFrom<&[u8]> for ClientRootKey {
    type Error = Error;

    fn try_from(bytes: &[u8]) -> Result<Self> {
        Self::from_slice(bytes)
    }
}
