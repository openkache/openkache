//! Typed-key conversion, canonical CBOR encoding, and Item ID derivation.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{Error, Result};

pub(crate) const PROTECTION_KEY_BYTES: usize =
    crate::contract::VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES;

/// Bytes in an application-managed data protection key.
pub const DATA_PROTECTION_KEY_BYTES: usize = PROTECTION_KEY_BYTES;
/// Bytes in the client root key.
pub const CLIENT_ROOT_KEY_BYTES: usize = PROTECTION_KEY_BYTES;
/// Maximum application key input bytes accepted by every conforming SDK.
pub const MAX_KEY_INPUT_BYTES: usize = 1_048_576;
/// Maximum Item ID bytes accepted by the wire protocol.
pub const MAX_ITEM_ID_BYTES: usize = 32;
/// Compatibility alias for the pre-contract name.
///
/// The limit is measured on logical input bytes, before canonical encoding.
#[deprecated(note = "use MAX_KEY_INPUT_BYTES")]
pub const MAX_CANONICAL_KEY_BYTES: usize = MAX_KEY_INPUT_BYTES;

/// Client-owned application-key to Item ID mapping profile.
///
/// This setting is local to a client. It is never serialized into a wire
/// frame or interpreted by the server.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum KeyFormat {
    /// Deterministic CBOR followed by namespace-bound BLAKE3 hashing.
    #[default]
    Hash,
    /// Preserve byte keys up to the wire limit and hash longer byte keys.
    ByteKeyOrHash,
}

/// The one typed-key variant selected for a formatted keyspace.
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

/// Configured logical key space shared by protection, FFI, and language adapters.
///
/// `KeySpace` owns the policy that turns a logical or typed key into one
/// validated [`ResolvedKey`]. Keeping that policy here means callers never
/// need to repeat type checks or canonical CBOR serialization.
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
            format: KeyFormat::Hash,
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
        match (self.format, key) {
            (KeyFormat::ByteKeyOrHash, TypedKey::Bytes(bytes)) => {
                ResolvedKey::from_direct_bytes(bytes)
            }
            (_, key) => ResolvedKey::from_typed(key),
        }
    }

    /// Resolves logical bytes using the configured key type.
    pub fn resolve_logical_bytes(self, value: &[u8]) -> std::result::Result<ResolvedKey, KeyError> {
        self.validate()?;
        if self.format == KeyFormat::ByteKeyOrHash && self.key_type == KeyType::Bytes {
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

/// Namespace and Item ID produced from one validated key.
///
/// The binding is kept next to key resolution so callers cannot accidentally
/// derive an Item ID with one namespace and protect the corresponding value
/// with another.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct KeyBinding {
    pub(crate) namespace_id: u64,
    pub(crate) item_id: ItemId,
}

/// Core-owned key policy used by protected clients.
///
/// `KeyResolver` is the only object that combines the configured logical
/// [`KeySpace`] with the client root secret. It keeps conversion, canonical
/// bytes, namespace binding, and Item ID derivation in the key module instead
/// of making the value-protection layer reimplement those steps.
pub(crate) struct KeyResolver {
    root: ClientRootKey,
    space: KeySpace,
}

impl KeyResolver {
    pub(crate) fn with_format(root: ClientRootKey, key_type: KeyType, format: KeyFormat) -> Self {
        Self {
            root,
            space: KeySpace::with_format(key_type, format),
        }
    }

    pub(crate) const fn key_type(&self) -> KeyType {
        self.space.key_type()
    }

    pub(crate) const fn format(&self) -> KeyFormat {
        self.space.format()
    }

    pub(crate) fn root(&self) -> &ClientRootKey {
        &self.root
    }

    /// Converts one internal key input through the single core-owned boundary.
    ///
    /// The compatibility `CanonicalKey` variant deliberately accepts any valid
    /// canonical v1 key because the original native ABI did not carry a
    /// configured key-space discriminator. New callers should use
    /// `CanonicalInSpace`, `ConfiguredLogical`, `TypedLogical`, or `Typed`,
    /// which all carry an explicit key-space policy.
    pub(crate) fn resolve_input(
        &self,
        input: KeyInput,
    ) -> std::result::Result<ResolvedKey, KeyError> {
        match input {
            KeyInput::Typed(key) => self.space.resolve(key),
            KeyInput::ConfiguredLogical(bytes) => self.space.resolve_logical_bytes(&bytes),
            #[cfg(feature = "ffi")]
            KeyInput::TypedLogical { key_type, bytes } => {
                KeySpace::with_format(key_type, self.format()).resolve_logical_bytes(&bytes)
            }
            #[cfg(feature = "ffi")]
            KeyInput::CanonicalKey(bytes) => ResolvedKey::from_canonical(&bytes),
            KeyInput::CanonicalInSpace(bytes) => self.space.resolve_canonical(&bytes),
        }
    }

    /// Resolves one key input and binds the exact resolved representation to
    /// one namespace.
    ///
    /// This is the only operation-facing key path. Keeping conversion,
    /// canonicalization, and Item ID derivation together prevents a caller
    /// from resolving one representation and protecting another.
    pub(crate) fn bind_input(
        &self,
        namespace_id: u64,
        input: KeyInput,
    ) -> std::result::Result<KeyBinding, KeyError> {
        let key = self.resolve_input(input)?;
        self.bind(namespace_id, &key)
    }

    pub(crate) fn bind(
        &self,
        namespace_id: u64,
        key: &ResolvedKey,
    ) -> std::result::Result<KeyBinding, KeyError> {
        Ok(KeyBinding {
            namespace_id,
            item_id: self
                .root
                .derive_item_id_for_resolved_key(namespace_id, key)?,
        })
    }

    /// Fallible legacy byte-key convenience using namespace `1`.
    ///
    /// The compatibility method keeps the configured mapping profile when it
    /// is the byte-key preserve-or-hash profile. Other configurations retain
    /// the historical deterministic-CBOR `Bytes` path. New callers should
    /// prefer this method so an oversized application key is rejected instead
    /// of panicking.
    pub(crate) fn try_legacy_item_id(
        &self,
        application_key: impl AsRef<[u8]>,
    ) -> std::result::Result<ItemId, KeyError> {
        let application_key = application_key.as_ref();
        let key = if self.space.key_type() == KeyType::Bytes
            && self.space.format() == KeyFormat::ByteKeyOrHash
        {
            ResolvedKey::from_direct_bytes(application_key.to_owned())
        } else {
            ResolvedKey::from_typed(TypedKey::Bytes(application_key.to_owned()))
        };
        let key = key?;
        self.root.derive_item_id_for_resolved_key(1, &key)
    }

    /// Legacy byte-key convenience using namespace `1`.
    ///
    /// This compatibility method retains the historical infallible API.
    /// Prefer [`Self::try_legacy_item_id`] for new code so an oversized
    /// application key is returned as a validation error.
    pub(crate) fn legacy_item_id(&self, application_key: impl AsRef<[u8]>) -> ItemId {
        self.try_legacy_item_id(application_key)
            .expect("legacy application key exceeds the key input limit")
    }
}

/// Converts an FFI key discriminator into the core key type.
///
/// The generated discriminator is an ABI concern, but the mapping belongs
/// next to the key model so FFI dispatchers do not grow a second key policy.
impl From<crate::contract::FfiKeySpec> for KeyType {
    fn from(value: crate::contract::FfiKeySpec) -> Self {
        match value {
            crate::contract::FfiKeySpec::Integer => Self::Integer,
            crate::contract::FfiKeySpec::Text => Self::Text,
            crate::contract::FfiKeySpec::Bytes => Self::Bytes,
        }
    }
}

/// Native and client request input before it crosses the core-owned
/// [`ResolvedKey`] boundary.
///
/// Every operation-facing key path uses this enum. Language adapters provide
/// neutral logical bytes, high-level Rust callers provide a [`TypedKey`],
/// compatibility callers provide a complete canonical key item. Operations
/// that already hold a [`ResolvedKey`] use the adjacent resolved-key methods
/// without another parse or allocation.
#[derive(Clone, Debug)]
pub(crate) enum KeyInput {
    Typed(TypedKey),
    /// Logical bytes interpreted using the resolver's configured key space.
    ConfiguredLogical(Vec<u8>),
    /// Logical bytes carrying an explicit ABI key-space discriminator.
    #[cfg(feature = "ffi")]
    TypedLogical {
        key_type: KeyType,
        bytes: Vec<u8>,
    },
    /// A complete canonical key item supplied through the compatibility FFI.
    ///
    /// This is distinct from [`CanonicalInSpace`](Self::CanonicalInSpace):
    /// compatibility callers carry no configured key-space discriminator, so
    /// the canonical item is validated but not compared with the resolver's
    /// configured [`KeyType`].
    #[cfg(feature = "ffi")]
    CanonicalKey(Vec<u8>),
    CanonicalInSpace(Vec<u8>),
}

impl KeyInput {
    pub(crate) fn typed(key: impl Into<TypedKey>) -> Self {
        Self::Typed(key.into())
    }

    pub(crate) fn configured_logical(bytes: Vec<u8>) -> Self {
        Self::ConfiguredLogical(bytes)
    }

    #[cfg(feature = "ffi")]
    pub(crate) fn typed_logical(key_type: KeyType, bytes: Vec<u8>) -> Self {
        Self::TypedLogical { key_type, bytes }
    }

    #[cfg(feature = "ffi")]
    pub(crate) fn canonical_key(bytes: Vec<u8>) -> Self {
        Self::CanonicalKey(bytes)
    }

    pub(crate) fn canonical_in_space(bytes: Vec<u8>) -> Self {
        Self::CanonicalInSpace(bytes)
    }

    /// Creates a logical native input from the generated ABI discriminator.
    ///
    /// The FFI dispatcher must not translate ABI enum values into the key
    /// model itself. Keeping that translation here makes the key module the
    /// only owner of the ABI-to-logical-key boundary.
    #[cfg(feature = "ffi")]
    pub(crate) fn from_ffi(key_spec: crate::contract::FfiKeySpec, bytes: Vec<u8>) -> Self {
        Self::typed_logical(key_spec.into(), bytes)
    }
}

impl From<TypedKey> for KeyInput {
    fn from(key: TypedKey) -> Self {
        Self::Typed(key)
    }
}

impl fmt::Display for KeyType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Arbitrary-precision mathematical integer used by [`TypedKey`].
///
/// The magnitude is stored as minimal big-endian bytes. Zero is always
/// non-negative. This representation lets native `u128` values and language
/// bindings with larger integer types use the same canonical bignum rules.
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
    /// resource limit.
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

    /// Parses an arbitrary-precision signed decimal integer.
    ///
    /// Decimal text is the neutral representation used by the typed native
    /// ABI so bindings do not need to implement bignum CBOR serialization.
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
    /// An arbitrary-precision mathematical integer.
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
        Ok(output)
    }

    /// Decodes exactly one canonical v1 key item.
    pub fn decode_canonical(bytes: &[u8]) -> std::result::Result<Self, KeyError> {
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
    direct: Option<Vec<u8>>,
}

impl ResolvedKey {
    /// Converts one typed logical key and stores its canonical bytes.
    pub fn from_typed(key: impl Into<TypedKey>) -> std::result::Result<Self, KeyError> {
        let typed = key.into();
        let canonical = typed.canonical_bytes()?;
        Ok(Self {
            key_type: typed.key_type(),
            canonical,
            direct: None,
        })
    }

    fn from_direct_bytes(bytes: Vec<u8>) -> std::result::Result<Self, KeyError> {
        if bytes.len() > MAX_KEY_INPUT_BYTES {
            return Err(KeyError::TooLarge {
                size: bytes.len(),
                maximum: MAX_KEY_INPUT_BYTES,
            });
        }
        Ok(Self {
            key_type: KeyType::Bytes,
            canonical: Vec::new(),
            direct: Some(bytes),
        })
    }

    /// Validates and adopts one complete canonical v1 key item.
    pub fn from_canonical(bytes: &[u8]) -> std::result::Result<Self, KeyError> {
        let typed = TypedKey::decode_canonical(bytes)?;
        Ok(Self {
            key_type: typed.key_type(),
            canonical: bytes.to_owned(),
            direct: None,
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

    fn direct_bytes(&self) -> Option<&[u8]> {
        self.direct.as_deref()
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
                TypedInteger::from_parts(false, &magnitude).map_err(|_| KeyError::TooLarge {
                    size: magnitude.len(),
                    maximum: MAX_KEY_INPUT_BYTES,
                })?
            } else {
                let actual_magnitude = add_one(&magnitude)?;
                if as_u64(&actual_magnitude).is_some() {
                    return Err(KeyError::NonCanonical);
                }
                TypedInteger::from_parts(true, &actual_magnitude).map_err(|_| {
                    KeyError::TooLarge {
                        size: actual_magnitude.len(),
                        maximum: MAX_KEY_INPUT_BYTES,
                    }
                })?
            };
            return Ok(TypedKey::Integer(integer));
        }

        let (major, argument) = self.parse_header()?;
        match major {
            0 => Ok(TypedKey::Integer(TypedInteger::from_u128(u128::from(
                argument,
            )))),
            1 => Ok(TypedKey::Integer(TypedInteger::from_parts(
                true,
                // A major-type-1 argument is a full u64.  `u64::MAX`
                // represents `-2^64`, whose minimal magnitude is the
                // nine-byte value `01 00 .. 00`; perform the +1 in u128
                // rather than rejecting that valid preferred encoding.
                &u128::from(argument)
                    .checked_add(1)
                    .ok_or(KeyError::Overflow)?
                    .to_be_bytes(),
            )?)),
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
    /// Wraps a legacy 32-byte item ID without hashing it again.
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

    pub(crate) fn into_protocol(self) -> openkache_protocol::ItemId {
        openkache_protocol::ItemId::from_slice(self.as_bytes())
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
        let item_id_root =
            blake3::derive_key(crate::contract::VALUE_FORMAT_ITEM_ID_ROOT_CONTEXT, &bytes);
        let value_root_key =
            blake3::derive_key(crate::contract::VALUE_FORMAT_VALUE_ROOT_CONTEXT, &bytes);
        Self {
            master_key: bytes,
            item_id_root,
            value_root_key,
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
        let key = ResolvedKey::from_typed(key)?;
        self.derive_item_id_for_resolved_key(namespace_id, &key)
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
        let key = ResolvedKey::from_canonical(canonical_key)?;
        self.derive_item_id_for_resolved_key(namespace_id, &key)
    }

    /// Derives an Item ID from a key that has already crossed the validated
    /// logical/canonical boundary.
    pub(crate) fn derive_item_id_for_resolved_key(
        &self,
        namespace_id: u64,
        key: &ResolvedKey,
    ) -> std::result::Result<ItemId, KeyError> {
        if namespace_id == 0 {
            return Err(KeyError::InvalidNamespace);
        }
        if let Some(direct) = key.direct_bytes() {
            return Ok(self.byte_key_or_hash(namespace_id, direct));
        }
        Ok(self.hash_canonical_key(namespace_id, key.canonical_bytes()))
    }

    fn hash_canonical_key(&self, namespace_id: u64, canonical_key: &[u8]) -> ItemId {
        let mut hasher = blake3::Hasher::new_keyed(&self.item_id_root);
        hasher.update(&namespace_id.to_be_bytes());
        hasher.update(canonical_key);
        ItemId::from_bytes(*hasher.finalize().as_bytes())
    }

    fn byte_key_or_hash(&self, namespace_id: u64, direct_key: &[u8]) -> ItemId {
        if direct_key.len() <= MAX_ITEM_ID_BYTES {
            return ItemId::from_slice(direct_key).expect("direct key length was validated");
        }
        let mut hasher = blake3::Hasher::new_keyed(&self.item_id_root);
        hasher.update(&namespace_id.to_be_bytes());
        hasher.update(direct_key);
        ItemId::from_bytes(*hasher.finalize().as_bytes())
    }

    /// Derives an Item ID with the direct-byte preserve-or-hash profile.
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
        Ok(self.byte_key_or_hash(namespace_id, direct_key))
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
        self.derive_item_id_in_namespace(1, TypedKey::Bytes(application_key.as_ref().to_vec()))
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

    pub(crate) fn value_root_key(&self) -> Zeroizing<[u8; DATA_PROTECTION_KEY_BYTES]> {
        Zeroizing::new(self.value_root_key)
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
