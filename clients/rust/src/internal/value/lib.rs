//! Owned cross-language values and the `StructuredValue-CBOR-v1` codec.
//!
//! The value crate is intentionally independent from client transport, cache
//! storage, and the outer value envelope. It owns logical value semantics and
//! one bounded, definite-length CBOR payload profile.

use std::cmp::Ordering;
use std::collections::{HashMap, hash_map::DefaultHasher};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::mem::size_of;
use std::str::FromStr;

/// Maximum payload size used by the default codec limits.
pub const MAX_VALUE_BYTES: usize = 67_108_864;
/// Default maximum nesting depth for a value.
pub const DEFAULT_MAX_DEPTH: usize = 128;
/// Absolute maximum nesting depth accepted by the bounded codec.
///
/// The codec traverses iteratively, but callers must not be able to turn a
/// configuration knob into an effectively unbounded worklist or destructor.
pub const MAX_ALLOWED_DEPTH: usize = 1_000_000;
/// Default maximum number of model nodes in one value.
pub const DEFAULT_MAX_ITEMS: usize = 1_000_000;
/// Default maximum bignum magnitude size.
pub const DEFAULT_MAX_INTEGER_BYTES: usize = 1 << 20;

/// A bounded resource budget for encoding and decoding.
///
/// Limits are checked before a declared CBOR length is allocated or traversed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Maximum complete CBOR payload bytes.
    pub max_bytes: usize,
    /// Maximum nested array/map depth.
    pub max_depth: usize,
    /// Maximum number of model nodes, including containers.
    pub max_items: usize,
    /// Maximum integer magnitude bytes.
    pub max_integer_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: MAX_VALUE_BYTES,
            max_depth: DEFAULT_MAX_DEPTH,
            max_items: DEFAULT_MAX_ITEMS,
            max_integer_bytes: DEFAULT_MAX_INTEGER_BYTES,
        }
    }
}

/// The sign of an arbitrary-precision [`Integer`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sign {
    /// A non-negative integer.
    Positive,
    /// A negative integer.
    Negative,
}

/// An exact arbitrary-precision signed integer.
///
/// The magnitude is stored in minimal big-endian form. Zero always has the
/// positive sign, so equal mathematical integers have one representation.
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct Integer {
    negative: bool,
    magnitude: Vec<u8>,
}

impl Integer {
    /// Returns zero.
    pub const fn zero() -> Self {
        Self {
            negative: false,
            magnitude: Vec::new(),
        }
    }

    /// Constructs an integer from a sign and a big-endian magnitude.
    ///
    /// Leading zero bytes are removed. An empty or all-zero magnitude is
    /// normalized to positive zero.
    pub fn from_sign_and_magnitude(negative: bool, magnitude: impl AsRef<[u8]>) -> Self {
        let bytes = magnitude.as_ref();
        let first = bytes
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(bytes.len());
        if first == bytes.len() {
            return Self::zero();
        }
        Self {
            negative,
            magnitude: bytes[first..].to_vec(),
        }
    }

    fn from_owned_sign_and_magnitude(negative: bool, mut magnitude: Vec<u8>) -> Self {
        let first = magnitude
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(magnitude.len());
        if first == magnitude.len() {
            return Self::zero();
        }
        if first != 0 {
            magnitude.drain(..first);
        }
        Self {
            negative,
            magnitude,
        }
    }

    /// Alias for [`Integer::from_sign_and_magnitude`].
    pub fn from_magnitude_be(sign: Sign, magnitude: impl AsRef<[u8]>) -> Self {
        Self::from_sign_and_magnitude(sign == Sign::Negative, magnitude)
    }

    /// Constructs an exact integer from an unsigned value.
    pub fn from_u128(value: u128) -> Self {
        if value == 0 {
            return Self::zero();
        }
        let bytes = value.to_be_bytes();
        Self::from_sign_and_magnitude(false, bytes)
    }

    /// Constructs an exact integer from a signed value.
    pub fn from_i128(value: i128) -> Self {
        if value >= 0 {
            return Self::from_u128(value as u128);
        }
        // `unsigned_abs` is defined for i128::MIN and avoids a signed
        // negation overflow.
        Self::from_sign_and_magnitude(true, value.unsigned_abs().to_be_bytes())
    }

    /// Parses an optional-sign decimal integer without rounding.
    pub fn parse_decimal(value: &str) -> Result<Self> {
        let bytes = value.as_bytes();
        if bytes.is_empty() {
            return Err(Error::InvalidInteger {
                offset: 0,
                reason: "empty decimal integer",
            });
        }
        let (negative, digits) = match bytes[0] {
            b'-' => (true, &bytes[1..]),
            b'+' => (false, &bytes[1..]),
            _ => (false, bytes),
        };
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return Err(Error::InvalidInteger {
                offset: 0,
                reason: "invalid decimal integer",
            });
        }
        let mut magnitude = Vec::new();
        for digit in digits {
            let mut carry = u16::from(*digit - b'0');
            for byte in magnitude.iter_mut().rev() {
                let value = u16::from(*byte) * 10 + carry;
                *byte = value as u8;
                carry = value >> 8;
            }
            if carry != 0 {
                magnitude.insert(0, carry as u8);
            }
        }
        Ok(Self::from_sign_and_magnitude(negative, magnitude))
    }

    /// Returns whether this integer is negative.
    pub const fn is_negative(&self) -> bool {
        self.negative
    }

    /// Returns whether this integer is zero.
    pub const fn is_zero(&self) -> bool {
        // `Vec::is_empty` is not const on the crate's 1.85 MSRV.
        self.magnitude.len() == 0
    }

    /// Returns the mathematical sign.
    pub const fn sign(&self) -> Sign {
        if self.negative {
            Sign::Negative
        } else {
            Sign::Positive
        }
    }

    /// Returns the minimal unsigned big-endian magnitude.
    pub fn magnitude_be(&self) -> &[u8] {
        &self.magnitude
    }

    /// Returns the value when it fits in `u128`.
    pub fn as_u128(&self) -> Option<u128> {
        if self.negative || self.magnitude.len() > 16 {
            return None;
        }
        let mut bytes = [0; 16];
        bytes[16 - self.magnitude.len()..].copy_from_slice(&self.magnitude);
        Some(u128::from_be_bytes(bytes))
    }

    /// Returns the value when it fits in `i128`.
    pub fn as_i128(&self) -> Option<i128> {
        let magnitude = self.as_u128_magnitude()?;
        if self.negative {
            if magnitude > (i128::MAX as u128) + 1 {
                None
            } else if magnitude == (i128::MAX as u128) + 1 {
                Some(i128::MIN)
            } else {
                Some(-(magnitude as i128))
            }
        } else {
            i128::try_from(magnitude).ok()
        }
    }

    fn as_u128_magnitude(&self) -> Option<u128> {
        if self.magnitude.len() > 16 {
            return None;
        }
        let mut bytes = [0; 16];
        bytes[16 - self.magnitude.len()..].copy_from_slice(&self.magnitude);
        Some(u128::from_be_bytes(bytes))
    }

    fn negative_cbor_magnitude(&self) -> Vec<u8> {
        let mut magnitude = self.magnitude.clone();
        if magnitude.is_empty() {
            return magnitude;
        }
        let mut index = magnitude.len();
        while index > 0 {
            index -= 1;
            if magnitude[index] != 0 {
                magnitude[index] -= 1;
                break;
            }
            magnitude[index] = 0xff;
        }
        let first = magnitude
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(magnitude.len());
        magnitude.drain(..first);
        magnitude
    }
}

impl Default for Integer {
    fn default() -> Self {
        Self::zero()
    }
}

impl fmt::Debug for Integer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Integer")
            .field("negative", &self.negative)
            .field("magnitude", &self.magnitude)
            .finish()
    }
}

impl fmt::Display for Integer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return formatter.write_str("0");
        }
        let mut magnitude = self.magnitude.clone();
        let mut digits = Vec::new();
        while !magnitude.is_empty() {
            let mut remainder = 0u16;
            for byte in &mut magnitude {
                let value = (remainder << 8) | u16::from(*byte);
                *byte = (value / 10) as u8;
                remainder = value % 10;
            }
            digits.push((b'0' + remainder as u8) as char);
            let first = magnitude
                .iter()
                .position(|byte| *byte != 0)
                .unwrap_or(magnitude.len());
            magnitude.drain(..first);
        }
        if self.negative {
            formatter.write_str("-")?;
        }
        for digit in digits.iter().rev() {
            formatter.write_str(&digit.to_string())?;
        }
        Ok(())
    }
}

impl FromStr for Integer {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse_decimal(value)
    }
}

impl From<i8> for Integer {
    fn from(value: i8) -> Self {
        Self::from_i128(value as i128)
    }
}

impl From<i16> for Integer {
    fn from(value: i16) -> Self {
        Self::from_i128(value as i128)
    }
}

impl From<i32> for Integer {
    fn from(value: i32) -> Self {
        Self::from_i128(value as i128)
    }
}

impl From<i64> for Integer {
    fn from(value: i64) -> Self {
        Self::from_i128(value as i128)
    }
}

impl From<i128> for Integer {
    fn from(value: i128) -> Self {
        Self::from_i128(value)
    }
}

impl From<u8> for Integer {
    fn from(value: u8) -> Self {
        Self::from_u128(value as u128)
    }
}

impl From<u16> for Integer {
    fn from(value: u16) -> Self {
        Self::from_u128(value as u128)
    }
}

impl From<u32> for Integer {
    fn from(value: u32) -> Self {
        Self::from_u128(value as u128)
    }
}

impl From<u64> for Integer {
    fn from(value: u64) -> Self {
        Self::from_u128(value as u128)
    }
}

impl From<u128> for Integer {
    fn from(value: u128) -> Self {
        Self::from_u128(value)
    }
}

/// IEEE-754 floating-point widths represented by [`Float`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FloatWidth {
    /// IEEE-754 binary16 (half precision).
    Bits16,
    /// IEEE-754 binary32 (single precision).
    Bits32,
    /// IEEE-754 binary64 (double precision).
    Bits64,
}

impl FloatWidth {
    const fn raw_bits_mask(self) -> u64 {
        match self {
            Self::Bits16 => u16::MAX as u64,
            Self::Bits32 => u32::MAX as u64,
            Self::Bits64 => u64::MAX,
        }
    }

    const fn byte_width(self) -> usize {
        match self {
            Self::Bits16 => 2,
            Self::Bits32 => 4,
            Self::Bits64 => 8,
        }
    }
}

/// An IEEE-754 value with its wire width and exact raw bits.
///
/// `raw_bits` stores the bit pattern in the least-significant bits. For
/// [`FloatWidth::Bits16`] and [`FloatWidth::Bits32`], the upper bits MUST be
/// zero. Constructors and decoders enforce this invariant; the public fields
/// remain available for direct model construction and are validated by every
/// encoder.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Float {
    /// IEEE-754 width of the value.
    pub width: FloatWidth,
    /// Raw IEEE-754 bits, right-aligned in this `u64`.
    pub raw_bits: u64,
}

impl Float {
    /// Constructs a float when `raw_bits` fits the selected width.
    pub const fn new(width: FloatWidth, raw_bits: u64) -> Option<Self> {
        if raw_bits & !width.raw_bits_mask() == 0 {
            Some(Self { width, raw_bits })
        } else {
            None
        }
    }

    /// Returns whether the raw bits fit the selected IEEE-754 width.
    pub const fn is_valid(self) -> bool {
        self.raw_bits & !self.width.raw_bits_mask() == 0
    }
}

/// A logical value in the cross-language model.
#[derive(Clone, Debug)]
pub enum Value {
    /// A language-level undefined value.
    Undefined,
    /// A null value.
    Null,
    /// A Boolean value.
    Boolean(bool),
    /// An exact arbitrary-precision integer.
    Integer(Integer),
    /// An IEEE-754 value with its width and exact raw bits.
    Float(Float),
    /// A well-formed UTF-8 string.
    TextString(String),
    /// An uninterpreted byte string.
    Bytes(Vec<u8>),
    /// An ordered sequence of values.
    Array(Vec<Value>),
    /// Ordered key/value entries. Keys must be scalar and unique by model equality.
    Map(Vec<(Value, Value)>),
}

impl Drop for Value {
    fn drop(&mut self) {
        // Move the root out of `self` and consume nested containers through an
        // explicit worklist. This preserves caller-selected depth limits
        // while preventing a deeply nested value from recursively unwinding
        // the native stack during destruction.
        let root = std::mem::replace(self, Self::Undefined);
        let mut pending = vec![root];
        while let Some(value) = pending.pop() {
            let mut value = std::mem::ManuallyDrop::new(value);
            // A type with a custom destructor cannot move fields out through
            // a by-value pattern. Borrow the owned container, take its
            // children, then explicitly drop scalar payloads. Forget the
            // now-empty shell so this destructor does not recursively invoke
            // itself.
            unsafe {
                match &mut *value {
                    Self::Array(values) => pending.extend(std::mem::take(values)),
                    Self::Map(entries) => {
                        for (key, value) in std::mem::take(entries) {
                            pending.push(key);
                            pending.push(value);
                        }
                    }
                    Self::Integer(integer) => std::ptr::drop_in_place(integer),
                    Self::TextString(text) => std::ptr::drop_in_place(text),
                    Self::Bytes(bytes) => std::ptr::drop_in_place(bytes),
                    Self::Undefined
                    | Self::Null
                    | Self::Boolean(_)
                    | Self::Float(_) => {}
                }
            }
        }
    }
}

impl Value {
    /// Constructs an exact arbitrary integer value.
    pub fn integer(value: impl Into<Integer>) -> Self {
        Self::Integer(value.into())
    }

    /// Constructs a binary16 value from raw IEEE-754 bits.
    pub const fn float16(raw_bits: u16) -> Self {
        Self::Float(Float {
            width: FloatWidth::Bits16,
            raw_bits: raw_bits as u64,
        })
    }

    /// Constructs a binary32 value from raw IEEE-754 bits.
    pub const fn float32(raw_bits: u32) -> Self {
        Self::Float(Float {
            width: FloatWidth::Bits32,
            raw_bits: raw_bits as u64,
        })
    }

    /// Constructs a binary64 value from raw IEEE-754 bits.
    pub const fn float64(raw_bits: u64) -> Self {
        Self::Float(Float {
            width: FloatWidth::Bits64,
            raw_bits,
        })
    }

    /// Constructs a UTF-8 text string.
    pub fn text(value: impl Into<String>) -> Self {
        Self::TextString(value.into())
    }

    /// Constructs an uninterpreted byte string.
    pub fn bytes(value: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(value.into())
    }

    /// Constructs an ordered array.
    pub fn array(values: Vec<Self>) -> Self {
        Self::Array(values)
    }

    /// Constructs a map after validating scalar keys and duplicate model keys.
    pub fn map(entries: Vec<(Value, Value)>) -> Result<Self> {
        validate_map_entries(&entries)?;
        Ok(Self::Map(entries))
    }

    /// Returns a borrowed view of map entries.
    pub fn as_map(&self) -> Option<&[(Value, Value)]> {
        match self {
            Self::Map(entries) => Some(entries),
            _ => None,
        }
    }

    /// Returns whether this value can be used as a map key.
    pub fn is_scalar_key(&self) -> bool {
        matches!(
            self,
            Self::Undefined
                | Self::Null
                | Self::Boolean(_)
                | Self::Integer(_)
                | Self::Float(_)
                | Self::TextString(_)
                | Self::Bytes(_)
        )
    }

    /// Returns the encoded StructuredValue-CBOR-v1 bytes.
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        encode(self)
    }

    /// Decodes one complete StructuredValue-CBOR-v1 value.
    pub fn from_cbor(bytes: &[u8]) -> Result<Self> {
        decode(bytes)
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        values_equal(self, other)
    }
}

impl Eq for Value {}

/// Stable high-level error category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    /// The supplied or produced bytes exceed a configured limit.
    ResourceLimit,
    /// The input ended before one complete item was available.
    Truncated,
    /// A complete value was followed by additional bytes.
    TrailingBytes,
    /// The CBOR byte grammar is malformed.
    InvalidEncoding,
    /// A valid CBOR type is outside this profile.
    UnsupportedType,
    /// Text bytes are not valid UTF-8.
    InvalidUtf8,
    /// A bignum violates its exact representation rules.
    InvalidInteger,
    /// A map key is not scalar.
    NonScalarKey,
    /// A map contains a duplicate logical key.
    DuplicateKey,
    /// An allocation could not be reserved.
    Allocation,
    /// A float contains bits outside its selected IEEE-754 width.
    InvalidFloat,
}

/// Resource that can be bounded by [`Limits`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Resource {
    /// Complete input or output bytes.
    Bytes,
    /// Nested container depth.
    Depth,
    /// Model node count.
    Items,
    /// Bignum magnitude bytes.
    IntegerBytes,
}

/// Errors produced by value validation and StructuredValue-CBOR-v1.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Input or output bytes exceed the configured limit.
    #[error("{resource:?} limit {limit} exceeded by {actual}")]
    ResourceLimit {
        /// Bounded resource.
        resource: Resource,
        /// Configured maximum.
        limit: usize,
        /// Observed amount.
        actual: usize,
    },
    /// A byte allocation could not be reserved.
    #[error("failed to allocate {size} bytes")]
    Allocation {
        /// Requested allocation.
        size: usize,
    },
    /// A float contains bits outside its selected IEEE-754 width.
    #[error("invalid {width:?} float raw bits {raw_bits:#018x}")]
    InvalidFloat {
        /// Selected IEEE-754 width.
        width: FloatWidth,
        /// Supplied right-aligned raw bits.
        raw_bits: u64,
    },
    /// No complete item begins at the reported byte.
    #[error("truncated CBOR at byte {offset}")]
    Truncated {
        /// Byte offset at which more input was required.
        offset: usize,
    },
    /// Extra bytes follow one complete item.
    #[error("trailing CBOR bytes at byte {offset}")]
    TrailingBytes {
        /// First trailing byte offset.
        offset: usize,
    },
    /// The CBOR grammar is malformed.
    #[error("invalid CBOR encoding at byte {offset}: {reason}")]
    InvalidEncoding {
        /// Byte offset of the malformed field.
        offset: usize,
        /// Stable reason identifier.
        reason: &'static str,
    },
    /// The CBOR item is not supported by this profile.
    #[error(
        "unsupported CBOR type at byte {offset}: major type {major}, additional information {additional}"
    )]
    UnsupportedType {
        /// Byte offset of the item.
        offset: usize,
        /// CBOR major type.
        major: u8,
        /// CBOR additional information.
        additional: u8,
    },
    /// An indefinite-length item is forbidden by this profile.
    #[error("indefinite-length CBOR item at byte {offset}")]
    IndefiniteLength {
        /// Byte offset of the item.
        offset: usize,
    },
    /// Text bytes are not valid UTF-8.
    #[error("invalid UTF-8 text string at byte {offset}")]
    InvalidUtf8 {
        /// Byte offset of the text item.
        offset: usize,
    },
    /// A CBOR bignum violates its minimal exact representation.
    #[error("invalid integer at byte {offset}: {reason}")]
    InvalidInteger {
        /// Byte offset of the integer item.
        offset: usize,
        /// Stable reason identifier.
        reason: &'static str,
    },
    /// A map key is not one of the profile's scalar values.
    #[error("non-scalar map key at entry {index}")]
    NonScalarKey {
        /// Zero-based map entry index.
        index: usize,
    },
    /// A map key compares equal to an earlier key under model equality.
    #[error("duplicate map key at entry {index}")]
    DuplicateKey {
        /// Zero-based duplicate entry index.
        index: usize,
    },
}

impl Error {
    /// Returns the stable category for this error.
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::ResourceLimit { .. } => ErrorKind::ResourceLimit,
            Self::Allocation { .. } => ErrorKind::Allocation,
            Self::InvalidFloat { .. } => ErrorKind::InvalidFloat,
            Self::Truncated { .. } => ErrorKind::Truncated,
            Self::TrailingBytes { .. } => ErrorKind::TrailingBytes,
            Self::InvalidEncoding { .. } | Self::IndefiniteLength { .. } => {
                ErrorKind::InvalidEncoding
            }
            Self::UnsupportedType { .. } => ErrorKind::UnsupportedType,
            Self::InvalidUtf8 { .. } => ErrorKind::InvalidUtf8,
            Self::InvalidInteger { .. } => ErrorKind::InvalidInteger,
            Self::NonScalarKey { .. } => ErrorKind::NonScalarKey,
            Self::DuplicateKey { .. } => ErrorKind::DuplicateKey,
        }
    }

    /// Alias for [`Error::kind`].
    pub const fn category(&self) -> ErrorKind {
        self.kind()
    }
}

/// Convenience result type for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Encodes one value with the default resource limits.
pub fn encode(value: &Value) -> Result<Vec<u8>> {
    encode_with_limits(value, Limits::default())
}

/// Encodes one value with explicit resource limits.
pub fn encode_with_limits(value: &Value, limits: Limits) -> Result<Vec<u8>> {
    encode_with_limits_and_budget(value, limits, usize::MAX, &mut |_| true)
}

/// Encodes one value while charging bounded allocations through a caller-owned
/// budget callback.
///
/// The callback is invoked before every bounded allocation. Returning `false`
/// rejects the operation before the allocation is attempted. The cumulative
/// amount charged to this operation is also checked against
/// `max_allocation_bytes`; callers that share a transport budget should retain
/// the permits acquired by the callback until the returned bytes are no longer
/// needed.
pub fn encode_with_limits_and_budget(
    value: &Value,
    limits: Limits,
    max_allocation_bytes: usize,
    reserve: &mut impl FnMut(usize) -> bool,
) -> Result<Vec<u8>> {
    validate_limits(limits)?;
    let mut allocation = AllocationState {
        used: 0,
        maximum: max_allocation_bytes,
        reserve,
    };
    let mut output = Vec::new();
    let mut tasks = Vec::new();
    let mut item_count = 0usize;
    let mut pending_items = 1usize;
    charge_allocation(&mut allocation, size_of::<(&Value, usize)>())?;
    tasks
        .try_reserve_exact(1)
        .map_err(|_| Error::Allocation { size: 1 })?;
    tasks.push((value, 0usize));
    while let Some((current, depth)) = tasks.pop() {
        item_count = item_count
            .checked_add(1)
            .ok_or_else(|| resource(Resource::Items, limits.max_items, usize::MAX))?;
        if item_count > limits.max_items {
            return Err(resource(Resource::Items, limits.max_items, item_count));
        }
        pending_items = pending_items
            .checked_sub(1)
            .expect("the root or a declared child is always pending");
        match current {
            Value::Undefined => push_bytes(&mut output, &[0xf7], limits, &mut allocation)?,
            Value::Null => push_bytes(&mut output, &[0xf6], limits, &mut allocation)?,
            Value::Boolean(false) => push_bytes(&mut output, &[0xf4], limits, &mut allocation)?,
            Value::Boolean(true) => push_bytes(&mut output, &[0xf5], limits, &mut allocation)?,
            Value::Integer(integer) => {
                encode_integer(integer, &mut output, limits, &mut allocation)?
            }
            Value::Float(float) => {
                if !float.is_valid() {
                    return Err(Error::InvalidFloat {
                        width: float.width,
                        raw_bits: float.raw_bits,
                    });
                }
                let (marker, width) = match float.width {
                    FloatWidth::Bits16 => (0xf9, FloatWidth::Bits16),
                    FloatWidth::Bits32 => (0xfa, FloatWidth::Bits32),
                    FloatWidth::Bits64 => (0xfb, FloatWidth::Bits64),
                };
                push_bytes(&mut output, &[marker], limits, &mut allocation)?;
                let bytes = float.raw_bits.to_be_bytes();
                push_bytes(
                    &mut output,
                    &bytes[8 - width.byte_width()..],
                    limits,
                    &mut allocation,
                )?;
            }
            Value::TextString(text) => {
                append_head(3, text.len(), &mut output, limits, &mut allocation)?;
                push_bytes(&mut output, text.as_bytes(), limits, &mut allocation)?;
            }
            Value::Bytes(bytes) => {
                append_head(2, bytes.len(), &mut output, limits, &mut allocation)?;
                push_bytes(&mut output, bytes, limits, &mut allocation)?;
            }
            Value::Array(values) => {
                if depth >= limits.max_depth {
                    return Err(resource(Resource::Depth, limits.max_depth, depth + 1));
                }
                validate_count(values.len(), limits.max_items)?;
                add_pending_items(
                    &mut pending_items,
                    values.len(),
                    item_count,
                    limits.max_items,
                )?;
                append_head(4, values.len(), &mut output, limits, &mut allocation)?;
                charge_allocation(
                    &mut allocation,
                    values
                        .len()
                        .checked_mul(size_of::<(&Value, usize)>())
                        .ok_or_else(|| {
                            resource(Resource::Bytes, max_allocation_bytes, usize::MAX)
                        })?,
                )?;
                tasks
                    .try_reserve_exact(values.len())
                    .map_err(|_| Error::Allocation {
                        size: tasks.len().saturating_add(values.len()),
                    })?;
                for child in values.iter().rev() {
                    tasks.push((child, depth + 1));
                }
            }
            Value::Map(entries) => {
                if depth >= limits.max_depth {
                    return Err(resource(Resource::Depth, limits.max_depth, depth + 1));
                }
                let task_count = entries
                    .len()
                    .checked_mul(2)
                    .ok_or_else(|| resource(Resource::Items, limits.max_items, usize::MAX))?;
                validate_count(task_count, limits.max_items)?;
                add_pending_items(&mut pending_items, task_count, item_count, limits.max_items)?;
                validate_map_entries_with_budget(entries, &mut allocation)?;
                append_head(5, entries.len(), &mut output, limits, &mut allocation)?;
                charge_allocation(
                    &mut allocation,
                    task_count
                        .checked_mul(size_of::<(&Value, usize)>())
                        .ok_or_else(|| {
                            resource(Resource::Bytes, max_allocation_bytes, usize::MAX)
                        })?,
                )?;
                tasks
                    .try_reserve_exact(task_count)
                    .map_err(|_| Error::Allocation {
                        size: tasks.len().saturating_add(task_count),
                    })?;
                for (key, child) in entries.iter().rev() {
                    tasks.push((child, depth + 1));
                    tasks.push((key, depth + 1));
                }
            }
        }
    }
    Ok(output)
}

/// Decodes one complete value with the default resource limits.
pub fn decode(bytes: &[u8]) -> Result<Value> {
    decode_with_limits(bytes, Limits::default())
}

/// Decodes one complete definite-length value with explicit resource limits.
pub fn decode_with_limits(bytes: &[u8], limits: Limits) -> Result<Value> {
    decode_with_limits_and_budget(bytes, limits, usize::MAX, &mut |_| true)
}

/// Decodes one complete value while charging bounded allocations through a
/// caller-owned budget callback.
///
/// The callback is invoked before every decoder frame, container-slot,
/// duplicate-key index, and owned byte/string/integer clone allocation.
pub fn decode_with_limits_and_budget(
    bytes: &[u8],
    limits: Limits,
    max_allocation_bytes: usize,
    reserve: &mut impl FnMut(usize) -> bool,
) -> Result<Value> {
    validate_limits(limits)?;
    if bytes.len() > limits.max_bytes {
        return Err(resource(Resource::Bytes, limits.max_bytes, bytes.len()));
    }
    if bytes.is_empty() {
        return Err(Error::Truncated { offset: 0 });
    }

    let mut cursor = 0usize;
    let mut frames = Vec::new();
    let mut root = None;
    let mut item_count = 0usize;
    let mut pending_items = 1usize;
    let mut allocation = AllocationState {
        used: 0,
        maximum: max_allocation_bytes,
        reserve,
    };
    loop {
        if let Some(value) = root {
            if cursor != bytes.len() {
                return Err(Error::TrailingBytes { offset: cursor });
            }
            return Ok(value);
        }

        item_count = item_count
            .checked_add(1)
            .ok_or_else(|| resource(Resource::Items, limits.max_items, usize::MAX))?;
        if item_count > limits.max_items {
            return Err(resource(Resource::Items, limits.max_items, item_count));
        }
        pending_items = pending_items
            .checked_sub(1)
            .expect("the root or a declared child is always pending");
        match parse_item(bytes, &mut cursor, limits, &mut allocation)? {
            ParsedItem::Value(value) => {
                accept_value(value, &mut frames, &mut root, &mut allocation)?
            }
            ParsedItem::Array(count) => {
                add_pending_items(&mut pending_items, count, item_count, limits.max_items)?;
                begin_frame(
                    Frame::Array {
                        remaining: count,
                        values: Vec::new(),
                    },
                    &mut frames,
                    limits,
                    &mut allocation,
                )?;
                if count == 0 {
                    accept_value(
                        Value::Array(Vec::new()),
                        &mut frames,
                        &mut root,
                        &mut allocation,
                    )?;
                }
            }
            ParsedItem::Map(count) => {
                let child_count = count
                    .checked_mul(2)
                    .ok_or_else(|| resource(Resource::Items, limits.max_items, usize::MAX))?;
                add_pending_items(
                    &mut pending_items,
                    child_count,
                    item_count,
                    limits.max_items,
                )?;
                begin_frame(
                    Frame::Map {
                        remaining: child_count,
                        entries: Vec::new(),
                        pending_key: None,
                        key_index: ScalarKeyIndex::new(),
                    },
                    &mut frames,
                    limits,
                    &mut allocation,
                )?;
                if count == 0 {
                    accept_value(
                        Value::Map(Vec::new()),
                        &mut frames,
                        &mut root,
                        &mut allocation,
                    )?;
                }
            }
        }
    }
}

fn add_pending_items(
    pending_items: &mut usize,
    child_count: usize,
    item_count: usize,
    maximum: usize,
) -> Result<()> {
    *pending_items = pending_items
        .checked_add(child_count)
        .ok_or_else(|| resource(Resource::Items, maximum, usize::MAX))?;
    let minimum_total = item_count
        .checked_add(*pending_items)
        .ok_or_else(|| resource(Resource::Items, maximum, usize::MAX))?;
    if minimum_total > maximum {
        return Err(resource(Resource::Items, maximum, minimum_total));
    }
    Ok(())
}

/// Alias for [`encode`].
pub fn to_cbor(value: &Value) -> Result<Vec<u8>> {
    encode(value)
}

/// Alias for [`decode`].
pub fn from_cbor(bytes: &[u8]) -> Result<Value> {
    decode(bytes)
}

/// Re-exports the codec under a profile-oriented module name.
pub mod cbor {
    pub use super::{
        Error, ErrorKind, Limits, Resource, Result, decode, decode_with_limits,
        decode_with_limits_and_budget, encode, encode_with_limits, encode_with_limits_and_budget,
        from_cbor, to_cbor,
    };
}

fn validate_limits(limits: Limits) -> Result<()> {
    if limits.max_bytes == 0 {
        return Err(resource(Resource::Bytes, 1, 0));
    }
    if limits.max_depth == 0 {
        return Err(resource(Resource::Depth, 1, 0));
    }
    if limits.max_depth > MAX_ALLOWED_DEPTH {
        return Err(resource(
            Resource::Depth,
            MAX_ALLOWED_DEPTH,
            limits.max_depth,
        ));
    }
    if limits.max_items == 0 {
        return Err(resource(Resource::Items, 1, 0));
    }
    Ok(())
}

struct AllocationState<'a> {
    used: usize,
    maximum: usize,
    reserve: &'a mut dyn FnMut(usize) -> bool,
}

fn charge_allocation(state: &mut AllocationState<'_>, size: usize) -> Result<()> {
    let next = state
        .used
        .checked_add(size)
        .ok_or_else(|| resource(Resource::Bytes, state.maximum, usize::MAX))?;
    if next > state.maximum || !(state.reserve)(size) {
        return Err(resource(Resource::Bytes, state.maximum, next));
    }
    state.used = next;
    Ok(())
}

fn resource(resource: Resource, limit: usize, actual: usize) -> Error {
    Error::ResourceLimit {
        resource,
        limit,
        actual,
    }
}

fn validate_count(count: usize, maximum: usize) -> Result<()> {
    if count > maximum {
        Err(resource(Resource::Items, maximum, count))
    } else {
        Ok(())
    }
}

fn push_bytes(
    output: &mut Vec<u8>,
    bytes: &[u8],
    limits: Limits,
    allocation: &mut AllocationState<'_>,
) -> Result<()> {
    let length = output
        .len()
        .checked_add(bytes.len())
        .ok_or_else(|| resource(Resource::Bytes, limits.max_bytes, usize::MAX))?;
    if length > limits.max_bytes {
        return Err(resource(Resource::Bytes, limits.max_bytes, length));
    }
    charge_allocation(allocation, bytes.len())?;
    output
        .try_reserve_exact(bytes.len())
        .map_err(|_| Error::Allocation { size: length })?;
    output.extend_from_slice(bytes);
    Ok(())
}

fn append_head(
    major: u8,
    length: usize,
    output: &mut Vec<u8>,
    limits: Limits,
    allocation: &mut AllocationState<'_>,
) -> Result<()> {
    let length = u64::try_from(length)
        .map_err(|_| resource(Resource::Bytes, limits.max_bytes, usize::MAX))?;
    let mut head = [0u8; 9];
    let size = if length < 24 {
        head[0] = (major << 5) | length as u8;
        1
    } else if length <= u8::MAX as u64 {
        head[0] = (major << 5) | 24;
        head[1] = length as u8;
        2
    } else if length <= u16::MAX as u64 {
        head[0] = (major << 5) | 25;
        head[1..3].copy_from_slice(&(length as u16).to_be_bytes());
        3
    } else if length <= u32::MAX as u64 {
        head[0] = (major << 5) | 26;
        head[1..5].copy_from_slice(&(length as u32).to_be_bytes());
        5
    } else {
        head[0] = (major << 5) | 27;
        head[1..9].copy_from_slice(&length.to_be_bytes());
        9
    };
    push_bytes(output, &head[..size], limits, allocation)
}

fn encode_integer(
    integer: &Integer,
    output: &mut Vec<u8>,
    limits: Limits,
    allocation: &mut AllocationState<'_>,
) -> Result<()> {
    validate_integer_magnitude(integer, limits)?;
    // Negative CBOR integers use `-1-n`; deriving that magnitude clones the
    // model bytes even when leading zeroes are removed from the temporary.
    // Charge the source-sized clone before constructing the derived value so
    // small values such as `-1` cannot bypass the aggregate budget.
    charge_allocation(allocation, integer.magnitude.len())?;
    let magnitude = if integer.negative {
        integer.negative_cbor_magnitude()
    } else {
        integer.magnitude.clone()
    };
    if magnitude.len() <= 8 {
        let mut bytes = [0u8; 8];
        bytes[8 - magnitude.len()..].copy_from_slice(&magnitude);
        let number = u64::from_be_bytes(bytes);
        let use_major_one = integer.negative;
        append_head(
            if use_major_one { 1 } else { 0 },
            usize::try_from(number)
                .map_err(|_| resource(Resource::Bytes, limits.max_bytes, usize::MAX))?,
            output,
            limits,
            allocation,
        )?;
    } else {
        append_head(
            6,
            if integer.negative { 3 } else { 2 },
            output,
            limits,
            allocation,
        )?;
        append_head(2, magnitude.len(), output, limits, allocation)?;
        push_bytes(output, &magnitude, limits, allocation)?;
    }
    Ok(())
}

fn validate_integer_magnitude(integer: &Integer, limits: Limits) -> Result<()> {
    if integer.magnitude.len() > limits.max_integer_bytes {
        return Err(resource(
            Resource::IntegerBytes,
            limits.max_integer_bytes,
            integer.magnitude.len(),
        ));
    }
    Ok(())
}

fn validate_map_entries(entries: &[(Value, Value)]) -> Result<()> {
    let mut key_index = ScalarKeyIndex::with_capacity(entries.len())?;
    for (index, (key, _)) in entries.iter().enumerate() {
        if !key.is_scalar_key() {
            return Err(Error::NonScalarKey { index });
        }
        if let Value::Float(float) = key {
            if !float.is_valid() {
                return Err(Error::InvalidFloat {
                    width: float.width,
                    raw_bits: float.raw_bits,
                });
            }
        }
        if key_index.contains(entries, key) {
            return Err(Error::DuplicateKey { index });
        }
        key_index.insert(key, index)?;
    }
    Ok(())
}

fn validate_map_entries_with_budget(
    entries: &[(Value, Value)],
    allocation: &mut AllocationState<'_>,
) -> Result<()> {
    let mut key_index = ScalarKeyIndex::with_capacity_and_budget(entries.len(), allocation)?;
    for (index, (key, _)) in entries.iter().enumerate() {
        if !key.is_scalar_key() {
            return Err(Error::NonScalarKey { index });
        }
        if let Value::Float(float) = key {
            if !float.is_valid() {
                return Err(Error::InvalidFloat {
                    width: float.width,
                    raw_bits: float.raw_bits,
                });
            }
        }
        if key_index.contains(entries, key) {
            return Err(Error::DuplicateKey { index });
        }
        key_index.insert_with_budget(key, index, allocation)?;
    }
    Ok(())
}

fn hash_map_allocation_size(capacity: usize) -> Result<usize> {
    capacity
        .checked_mul(size_of::<(u64, Vec<usize>)>())
        .ok_or_else(|| resource(Resource::Bytes, usize::MAX, usize::MAX))
}

struct ScalarKeyIndex {
    buckets: HashMap<u64, Vec<usize>>,
}

impl ScalarKeyIndex {
    fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    fn with_capacity(capacity: usize) -> Result<Self> {
        let mut index = Self::new();
        index.reserve(capacity)?;
        Ok(index)
    }

    fn with_capacity_and_budget(
        capacity: usize,
        allocation: &mut AllocationState<'_>,
    ) -> Result<Self> {
        let mut index = Self::new();
        index.reserve_with_budget(capacity, allocation)?;
        Ok(index)
    }

    fn reserve(&mut self, additional: usize) -> Result<()> {
        self.buckets
            .try_reserve(additional)
            .map_err(|_| Error::Allocation { size: additional })
    }

    fn reserve_with_budget(
        &mut self,
        additional: usize,
        allocation: &mut AllocationState<'_>,
    ) -> Result<()> {
        // HashMap's table keeps control slots in addition to entries and may
        // round a request up to its next growth class. Reserve twice the
        // logical entry count up front so subsequent inserts cannot grow the
        // table without a separately charged reservation.
        let table_capacity = additional
            .checked_mul(2)
            .ok_or_else(|| resource(Resource::Bytes, allocation.maximum, usize::MAX))?;
        charge_allocation(allocation, hash_map_allocation_size(table_capacity)?)?;
        self.reserve(table_capacity)
    }

    fn contains(&self, entries: &[(Value, Value)], key: &Value) -> bool {
        let hash = scalar_key_hash(key);
        self.buckets.get(&hash).is_some_and(|bucket| {
            bucket
                .iter()
                .any(|&index| scalar_equal(key, &entries[index].0))
        })
    }

    fn insert(&mut self, key: &Value, index: usize) -> Result<()> {
        let hash = scalar_key_hash(key);
        if let Some(bucket) = self.buckets.get_mut(&hash) {
            bucket
                .try_reserve(1)
                .map_err(|_| Error::Allocation { size: 1 })?;
            bucket.push(index);
            return Ok(());
        }

        self.buckets
            .try_reserve(1)
            .map_err(|_| Error::Allocation { size: 1 })?;
        let mut bucket = Vec::new();
        bucket
            .try_reserve_exact(1)
            .map_err(|_| Error::Allocation { size: 1 })?;
        bucket.push(index);
        self.buckets.insert(hash, bucket);
        Ok(())
    }

    fn insert_with_budget(
        &mut self,
        key: &Value,
        index: usize,
        allocation: &mut AllocationState<'_>,
    ) -> Result<()> {
        let hash = scalar_key_hash(key);
        if let Some(bucket) = self.buckets.get_mut(&hash) {
            charge_allocation(allocation, size_of::<usize>())?;
            bucket
                .try_reserve_exact(1)
                .map_err(|_| Error::Allocation { size: 1 })?;
            bucket.push(index);
            return Ok(());
        }

        // The table is reserved to the complete entry count up front, so
        // inserting a new hash cannot grow the table. Each bucket still owns
        // one index slot and must be charged before its allocation.
        charge_allocation(allocation, size_of::<usize>())?;
        if self.buckets.len() >= self.buckets.capacity() {
            charge_allocation(allocation, hash_map_allocation_size(1)?)?;
        }
        self.buckets
            .try_reserve(1)
            .map_err(|_| Error::Allocation { size: 1 })?;
        let mut bucket = Vec::new();
        bucket
            .try_reserve_exact(1)
            .map_err(|_| Error::Allocation { size: 1 })?;
        bucket.push(index);
        self.buckets.insert(hash, bucket);
        Ok(())
    }
}

fn scalar_key_hash(value: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    match value {
        Value::Undefined => 0u8.hash(&mut hasher),
        Value::Null => 1u8.hash(&mut hasher),
        Value::Boolean(value) => {
            2u8.hash(&mut hasher);
            value.hash(&mut hasher);
        }
        Value::Integer(value) => {
            3u8.hash(&mut hasher);
            value.hash(&mut hasher);
        }
        Value::Float(value) => {
            4u8.hash(&mut hasher);
            value.hash(&mut hasher);
        }
        Value::TextString(value) => {
            7u8.hash(&mut hasher);
            value.hash(&mut hasher);
        }
        Value::Bytes(value) => {
            8u8.hash(&mut hasher);
            value.hash(&mut hasher);
        }
        Value::Array(_) | Value::Map(_) => unreachable!("only scalar keys are hashed"),
    }
    hasher.finish()
}

fn scalar_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Boolean(left), Value::Boolean(right)) => left == right,
        (Value::Integer(left), Value::Integer(right)) => left == right,
        (Value::Float(left), Value::Float(right)) => left == right,
        (Value::TextString(left), Value::TextString(right)) => left == right,
        (Value::Bytes(left), Value::Bytes(right)) => left == right,
        _ => false,
    }
}

fn values_equal(left: &Value, right: &Value) -> bool {
    let mut work = vec![(left, right)];
    while let Some((left, right)) = work.pop() {
        match (left, right) {
            (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => {}
            (Value::Boolean(left), Value::Boolean(right)) if left == right => {}
            (Value::Integer(left), Value::Integer(right)) if left == right => {}
            (Value::Float(left), Value::Float(right)) if left == right => {}
            (Value::TextString(left), Value::TextString(right)) if left == right => {}
            (Value::Bytes(left), Value::Bytes(right)) if left == right => {}
            (Value::Array(left), Value::Array(right)) if left.len() == right.len() => {
                for (left, right) in left.iter().zip(right).rev() {
                    work.push((left, right));
                }
            }
            (Value::Map(left), Value::Map(right)) if left.len() == right.len() => {
                let mut matched = vec![false; right.len()];
                for (left_key, left_value) in left {
                    let Some(index) =
                        right
                            .iter()
                            .enumerate()
                            .find_map(|(index, (right_key, _))| {
                                (!matched[index] && scalar_equal(left_key, right_key))
                                    .then_some(index)
                            })
                    else {
                        return false;
                    };
                    matched[index] = true;
                    work.push((left_value, &right[index].1));
                }
            }
            _ => return false,
        }
    }
    true
}

enum ParsedItem {
    Value(Value),
    Array(usize),
    Map(usize),
}

enum Frame {
    Array {
        remaining: usize,
        values: Vec<Value>,
    },
    Map {
        remaining: usize,
        entries: Vec<(Value, Value)>,
        pending_key: Option<Value>,
        key_index: ScalarKeyIndex,
    },
}

fn begin_frame(
    mut frame: Frame,
    frames: &mut Vec<Frame>,
    limits: Limits,
    allocation: &mut AllocationState<'_>,
) -> Result<()> {
    if frames.len() >= limits.max_depth {
        return Err(resource(
            Resource::Depth,
            limits.max_depth,
            frames.len() + 1,
        ));
    }
    // Empty containers still consume a depth level, but do not need a frame or
    // slot allocation.
    if frame_remaining(&frame) == 0 {
        return Ok(());
    }
    charge_allocation(allocation, size_of::<Frame>())?;
    frames.try_reserve_exact(1).map_err(|_| Error::Allocation {
        size: frames.len() + 1,
    })?;
    match &mut frame {
        Frame::Array { remaining, values } => {
            validate_count(*remaining, limits.max_items)?;
            charge_allocation(
                allocation,
                remaining
                    .checked_mul(size_of::<Value>())
                    .ok_or_else(|| resource(Resource::Bytes, allocation.maximum, usize::MAX))?,
            )?;
            values
                .try_reserve_exact(*remaining)
                .map_err(|_| Error::Allocation { size: *remaining })?;
        }
        Frame::Map {
            remaining,
            entries,
            key_index,
            ..
        } => {
            validate_count(*remaining, limits.max_items)?;
            let entry_count = *remaining / 2;
            charge_allocation(
                allocation,
                entry_count
                    .checked_mul(size_of::<(Value, Value)>())
                    .ok_or_else(|| resource(Resource::Bytes, allocation.maximum, usize::MAX))?,
            )?;
            entries
                .try_reserve_exact(entry_count)
                .map_err(|_| Error::Allocation { size: entry_count })?;
            key_index.reserve_with_budget(entry_count, allocation)?;
        }
    }
    frames.push(frame);
    Ok(())
}

fn accept_value(
    mut value: Value,
    frames: &mut Vec<Frame>,
    root: &mut Option<Value>,
    allocation: &mut AllocationState<'_>,
) -> Result<()> {
    loop {
        let Some(frame) = frames.last_mut() else {
            *root = Some(value);
            return Ok(());
        };
        match frame {
            Frame::Array { remaining, values } => {
                values.push(value);
                *remaining -= 1;
            }
            Frame::Map {
                remaining,
                entries,
                pending_key,
                key_index,
            } => {
                if pending_key.is_none() {
                    let index = entries.len();
                    if !value.is_scalar_key() {
                        return Err(Error::NonScalarKey { index });
                    }
                    if key_index.contains(entries, &value) {
                        return Err(Error::DuplicateKey { index });
                    }
                    key_index.insert_with_budget(&value, index, allocation)?;
                    *pending_key = Some(value);
                } else {
                    let key = pending_key.take().expect("map key was checked above");
                    entries.push((key, value));
                }
                *remaining -= 1;
            }
        }
        if frame_remaining(frame) != 0 {
            return Ok(());
        }
        let completed = frames.pop().expect("last frame exists");
        value = match completed {
            Frame::Array { values, .. } => Value::Array(values),
            Frame::Map { entries, .. } => Value::Map(entries),
        };
    }
}

fn frame_remaining(frame: &Frame) -> usize {
    match frame {
        Frame::Array { remaining, .. } | Frame::Map { remaining, .. } => *remaining,
    }
}

fn parse_item(
    bytes: &[u8],
    cursor: &mut usize,
    limits: Limits,
    allocation: &mut AllocationState<'_>,
) -> Result<ParsedItem> {
    let offset = *cursor;
    let (major, additional) = read_head(bytes, cursor)?;
    match major {
        0 | 1 => {
            let argument = read_argument(bytes, cursor, additional, offset)?;
            let magnitude = if major == 0 {
                argument as u128
            } else {
                (argument as u128) + 1
            };
            charge_allocation(
                allocation,
                if magnitude == 0 {
                    0
                } else {
                    (u128::BITS as usize - magnitude.leading_zeros() as usize).div_ceil(8)
                },
            )?;
            let integer = if major == 0 {
                Integer::from_u128(argument as u128)
            } else {
                let magnitude = (argument as u128) + 1;
                Integer::from_sign_and_magnitude(true, magnitude.to_be_bytes())
            };
            validate_integer_magnitude(&integer, limits)?;
            Ok(ParsedItem::Value(Value::Integer(integer)))
        }
        2 | 3 => {
            let length = read_length(bytes, cursor, additional, offset, limits, Resource::Bytes)?;
            let end = cursor
                .checked_add(length)
                .ok_or_else(|| resource(Resource::Bytes, limits.max_bytes, usize::MAX))?;
            if end > bytes.len() {
                return Err(Error::Truncated { offset: *cursor });
            }
            let content = &bytes[*cursor..end];
            *cursor = end;
            if major == 2 {
                Ok(ParsedItem::Value(Value::Bytes(clone_bytes(
                    content, allocation,
                )?)))
            } else {
                let text =
                    std::str::from_utf8(content).map_err(|_| Error::InvalidUtf8 { offset })?;
                charge_allocation(allocation, content.len())?;
                let mut owned = String::new();
                owned
                    .try_reserve_exact(content.len())
                    .map_err(|_| Error::Allocation {
                        size: content.len(),
                    })?;
                owned.push_str(text);
                Ok(ParsedItem::Value(Value::TextString(owned)))
            }
        }
        4 => {
            let length = read_length(bytes, cursor, additional, offset, limits, Resource::Items)?;
            if length > limits.max_items {
                return Err(resource(Resource::Items, limits.max_items, length));
            }
            Ok(ParsedItem::Array(length))
        }
        5 => {
            let length = read_length(bytes, cursor, additional, offset, limits, Resource::Items)?;
            if length > limits.max_items {
                return Err(resource(Resource::Items, limits.max_items, length));
            }
            Ok(ParsedItem::Map(length))
        }
        6 => {
            let tag = read_argument(bytes, cursor, additional, offset)?;
            if tag != 2 && tag != 3 {
                return Err(Error::UnsupportedType {
                    offset,
                    major,
                    additional,
                });
            }
            let bignum_offset = *cursor;
            let (bignum_major, bignum_additional) = read_head(bytes, cursor)?;
            if bignum_major != 2 {
                return Err(Error::InvalidInteger {
                    offset: bignum_offset,
                    reason: "bignum tag must wrap a byte string",
                });
            }
            let length = read_length(
                bytes,
                cursor,
                bignum_additional,
                bignum_offset,
                limits,
                Resource::IntegerBytes,
            )?;
            if length == 0 {
                return Err(Error::InvalidInteger {
                    offset,
                    reason: "bignum magnitude must not be empty",
                });
            }
            if length > limits.max_integer_bytes {
                return Err(resource(
                    Resource::IntegerBytes,
                    limits.max_integer_bytes,
                    length,
                ));
            }
            let end = cursor.checked_add(length).ok_or_else(|| {
                resource(Resource::IntegerBytes, limits.max_integer_bytes, usize::MAX)
            })?;
            if end > bytes.len() {
                return Err(Error::Truncated { offset: *cursor });
            }
            let magnitude = &bytes[*cursor..end];
            *cursor = end;
            if magnitude[0] == 0 {
                return Err(Error::InvalidInteger {
                    offset,
                    reason: "bignum magnitude is not minimal",
                });
            }
            let model_magnitude = if tag == 3 {
                add_one_be(magnitude, limits.max_integer_bytes, allocation)?
            } else {
                clone_bytes(magnitude, allocation)?
            };
            Ok(ParsedItem::Value(Value::Integer(
                Integer::from_owned_sign_and_magnitude(tag == 3, model_magnitude),
            )))
        }
        7 => match additional {
            20 => Ok(ParsedItem::Value(Value::Boolean(false))),
            21 => Ok(ParsedItem::Value(Value::Boolean(true))),
            22 => Ok(ParsedItem::Value(Value::Null)),
            23 => Ok(ParsedItem::Value(Value::Undefined)),
            25 => {
                let raw = read_fixed(bytes, cursor, 2, offset)?;
                Ok(ParsedItem::Value(Value::Float(Float {
                    width: FloatWidth::Bits16,
                    raw_bits: u64::from(u16::from_be_bytes([raw[0], raw[1]])),
                })))
            }
            26 => {
                let raw = read_fixed(bytes, cursor, 4, offset)?;
                Ok(ParsedItem::Value(Value::Float(Float {
                    width: FloatWidth::Bits32,
                    raw_bits: u64::from(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]])),
                })))
            }
            27 => {
                let raw = read_fixed(bytes, cursor, 8, offset)?;
                Ok(ParsedItem::Value(Value::Float(Float {
                    width: FloatWidth::Bits64,
                    raw_bits: u64::from_be_bytes([
                        raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
                    ]),
                })))
            }
            31 => Err(Error::IndefiniteLength { offset }),
            _ => Err(Error::UnsupportedType {
                offset,
                major,
                additional,
            }),
        },
        _ => Err(Error::UnsupportedType {
            offset,
            major,
            additional,
        }),
    }
}

fn clone_bytes(bytes: &[u8], allocation: &mut AllocationState<'_>) -> Result<Vec<u8>> {
    charge_allocation(allocation, bytes.len())?;
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(bytes.len())
        .map_err(|_| Error::Allocation { size: bytes.len() })?;
    owned.extend_from_slice(bytes);
    Ok(owned)
}

fn add_one_be(
    bytes: &[u8],
    maximum: usize,
    allocation: &mut AllocationState<'_>,
) -> Result<Vec<u8>> {
    let result_length = if bytes.iter().all(|byte| *byte == u8::MAX) {
        bytes
            .len()
            .checked_add(1)
            .ok_or_else(|| resource(Resource::IntegerBytes, maximum, usize::MAX))?
    } else {
        bytes.len()
    };
    if result_length > maximum {
        return Err(resource(Resource::IntegerBytes, maximum, result_length));
    }
    charge_allocation(allocation, result_length)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(result_length)
        .map_err(|_| Error::Allocation {
            size: result_length,
        })?;
    result.extend_from_slice(bytes);
    let mut carry = 1u16;
    for byte in result.iter_mut().rev() {
        let value = u16::from(*byte) + carry;
        *byte = value as u8;
        carry = value >> 8;
    }
    if carry != 0 {
        debug_assert_eq!(result_length, bytes.len() + 1);
        result.insert(0, carry as u8);
    }
    Ok(result)
}

fn read_head(bytes: &[u8], cursor: &mut usize) -> Result<(u8, u8)> {
    let offset = *cursor;
    let byte = *bytes.get(*cursor).ok_or(Error::Truncated { offset })?;
    *cursor += 1;
    Ok((byte >> 5, byte & 0x1f))
}

fn read_argument(bytes: &[u8], cursor: &mut usize, additional: u8, offset: usize) -> Result<u64> {
    match additional {
        0..=23 => Ok(u64::from(additional)),
        24 => Ok(u64::from(read_fixed(bytes, cursor, 1, offset)?[0])),
        25 => {
            let raw = read_fixed(bytes, cursor, 2, offset)?;
            Ok(u64::from(u16::from_be_bytes([raw[0], raw[1]])))
        }
        26 => {
            let raw = read_fixed(bytes, cursor, 4, offset)?;
            Ok(u64::from(u32::from_be_bytes([
                raw[0], raw[1], raw[2], raw[3],
            ])))
        }
        27 => {
            let raw = read_fixed(bytes, cursor, 8, offset)?;
            Ok(u64::from_be_bytes([
                raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
            ]))
        }
        31 => Err(Error::IndefiniteLength { offset }),
        _ => Err(Error::InvalidEncoding {
            offset,
            reason: "invalid additional information",
        }),
    }
}

fn read_length(
    bytes: &[u8],
    cursor: &mut usize,
    additional: u8,
    offset: usize,
    limits: Limits,
    resource_kind: Resource,
) -> Result<usize> {
    let argument = read_argument(bytes, cursor, additional, offset)?;
    let maximum = match resource_kind {
        Resource::Bytes => limits.max_bytes,
        Resource::Depth => limits.max_depth,
        Resource::Items => limits.max_items,
        Resource::IntegerBytes => limits.max_integer_bytes,
    };
    let length =
        usize::try_from(argument).map_err(|_| resource(resource_kind, maximum, usize::MAX))?;
    if length > maximum {
        return Err(resource(resource_kind, maximum, length));
    }
    Ok(length)
}

fn read_fixed<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    length: usize,
    offset: usize,
) -> Result<&'a [u8]> {
    let end = cursor
        .checked_add(length)
        .ok_or(Error::Truncated { offset })?;
    if end > bytes.len() {
        return Err(Error::Truncated { offset: *cursor });
    }
    let result = &bytes[*cursor..end];
    *cursor = end;
    Ok(result)
}

#[allow(dead_code)]
fn compare_integers(left: &Integer, right: &Integer) -> Ordering {
    match (left.negative, right.negative) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (negative, _) => {
            let magnitude = left.magnitude.len().cmp(&right.magnitude.len());
            let magnitude = if magnitude == Ordering::Equal {
                left.magnitude.cmp(&right.magnitude)
            } else {
                magnitude
            };
            if negative {
                magnitude.reverse()
            } else {
                magnitude
            }
        }
    }
}
