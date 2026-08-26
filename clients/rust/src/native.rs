//! Native adapters for the lossless OpenKache [`Value`] model.
//!
//! Native values deliberately pass through the same structured-value model as
//! the existing `get` and `set` APIs. This keeps the wire format language
//! neutral while preserving integer widths, floating-point bits, byte strings,
//! and the distinction between a missing item and a stored null. Serde has a
//! built-in adapter; other structured serializers use [`ValueCodec`].

use std::fmt;
use std::mem::ManuallyDrop;
use std::ptr;

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess,
    SeqAccess, VariantAccess, Visitor,
};
use serde::ser::{
    self, Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
    SerializeTuple, SerializeTupleStruct, SerializeTupleVariant, Serializer,
};

use super::internal_value::{Float, FloatWidth, Integer, Value};

/// Converts one Serde value into the OpenKache structured-value model.
pub(crate) fn to_value<T>(value: &T) -> Result<Value, NativeError>
where
    T: Serialize + ?Sized,
{
    value.serialize(ValueSerializer)
}

/// Converts one OpenKache structured value into a native Serde value.
pub(crate) fn from_value<T>(value: Value) -> Result<T, NativeError>
where
    T: DeserializeOwned,
{
    T::deserialize(ValueDeserializer::new(value))
}

/// Converts a native type to and from the public [`Value`] model.
///
/// Implement this trait when an application uses a serializer other than
/// Serde. A self-describing, cross-language format can map directly to
/// `Value`; an opaque format such as bincode should use `Value::Bytes` and
/// keep its codec choice alongside the application schema.
pub trait ValueCodec<T> {
    /// Codec-specific encoding failure type.
    type EncodeError: fmt::Display;
    /// Codec-specific decoding failure type.
    type DecodeError: fmt::Display;

    /// Encodes one native value before a write is admitted.
    fn encode(&self, value: &T) -> Result<Value, Self::EncodeError>;

    /// Decodes one stored value after it has been retrieved.
    fn decode(&self, value: Value) -> Result<T, Self::DecodeError>;
}

/// The built-in [`ValueCodec`] backed by Serde.
#[derive(Clone, Copy, Debug, Default)]
pub struct SerdeCodec;

impl<T> ValueCodec<T> for SerdeCodec
where
    T: Serialize + DeserializeOwned,
{
    type EncodeError = String;
    type DecodeError = String;

    fn encode(&self, value: &T) -> Result<Value, Self::EncodeError> {
        to_value(value).map_err(|error| error.to_string())
    }

    fn decode(&self, value: Value) -> Result<T, Self::DecodeError> {
        from_value(value).map_err(|error| error.to_string())
    }
}

/// A codec assembled from application-provided encode and decode functions.
///
/// This is the adapter for serializers that do not use Serde. The functions
/// construct a structured [`Value`] for a format that should remain
/// cross-language. Opaque byte formats should use the ordinary `set` and
/// `get` methods with `Value::Bytes` directly.
pub struct FunctionCodec<Encode, Decode> {
    encode: Encode,
    decode: Decode,
}

impl<Encode, Decode> FunctionCodec<Encode, Decode> {
    /// Creates a codec from one encoder and one decoder.
    pub const fn new(encode: Encode, decode: Decode) -> Self {
        Self { encode, decode }
    }
}

impl<T, Encode, Decode, EncodeError, DecodeError> ValueCodec<T> for FunctionCodec<Encode, Decode>
where
    Encode: Fn(&T) -> Result<Value, EncodeError>,
    Decode: Fn(Value) -> Result<T, DecodeError>,
    EncodeError: fmt::Display,
    DecodeError: fmt::Display,
{
    type EncodeError = EncodeError;
    type DecodeError = DecodeError;

    fn encode(&self, value: &T) -> Result<Value, Self::EncodeError> {
        (self.encode)(value)
    }

    fn decode(&self, value: Value) -> Result<T, Self::DecodeError> {
        (self.decode)(value)
    }
}

/// Error shared by the native serializer and deserializer.
#[derive(Debug)]
pub(crate) struct NativeError(String);

impl NativeError {
    fn message(message: impl fmt::Display) -> Self {
        Self(message.to_string())
    }

    fn expected_owned(expected: &str, actual: &OwnedValue) -> Self {
        Self::message(format!(
            "expected {expected}, found {}",
            owned_value_type_name(actual)
        ))
    }
}

impl fmt::Display for NativeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for NativeError {}

impl ser::Error for NativeError {
    fn custom<T>(message: T) -> Self
    where
        T: fmt::Display,
    {
        Self::message(message)
    }
}

impl de::Error for NativeError {
    fn custom<T>(message: T) -> Self
    where
        T: fmt::Display,
    {
        Self::message(message)
    }
}

struct ValueSerializer;

impl Serializer for ValueSerializer {
    type Ok = Value;
    type Error = NativeError;
    type SerializeSeq = Compound;
    type SerializeTuple = Compound;
    type SerializeTupleStruct = Compound;
    type SerializeTupleVariant = Compound;
    type SerializeMap = Compound;
    type SerializeStruct = Compound;
    type SerializeStructVariant = Compound;

    fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Boolean(value))
    }

    fn serialize_i8(self, value: i8) -> Result<Self::Ok, Self::Error> {
        Ok(Value::integer(value))
    }

    fn serialize_i16(self, value: i16) -> Result<Self::Ok, Self::Error> {
        Ok(Value::integer(value))
    }

    fn serialize_i32(self, value: i32) -> Result<Self::Ok, Self::Error> {
        Ok(Value::integer(value))
    }

    fn serialize_i64(self, value: i64) -> Result<Self::Ok, Self::Error> {
        Ok(Value::integer(value))
    }

    fn serialize_i128(self, value: i128) -> Result<Self::Ok, Self::Error> {
        Ok(Value::integer(value))
    }

    fn serialize_u8(self, value: u8) -> Result<Self::Ok, Self::Error> {
        Ok(Value::integer(value))
    }

    fn serialize_u16(self, value: u16) -> Result<Self::Ok, Self::Error> {
        Ok(Value::integer(value))
    }

    fn serialize_u32(self, value: u32) -> Result<Self::Ok, Self::Error> {
        Ok(Value::integer(value))
    }

    fn serialize_u64(self, value: u64) -> Result<Self::Ok, Self::Error> {
        Ok(Value::integer(value))
    }

    fn serialize_u128(self, value: u128) -> Result<Self::Ok, Self::Error> {
        Ok(Value::integer(value))
    }

    fn serialize_f32(self, value: f32) -> Result<Self::Ok, Self::Error> {
        Ok(Value::float32(value.to_bits()))
    }

    fn serialize_f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
        Ok(Value::float64(value.to_bits()))
    }

    fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
        Ok(Value::text(value.to_string()))
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        Ok(Value::text(value))
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<Self::Ok, Self::Error> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(value.len())
            .map_err(|_| NativeError::message("failed to allocate native byte string"))?;
        bytes.extend_from_slice(value);
        Ok(Value::bytes(bytes))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Null)
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Null)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        variant_value(variant, Value::Null)
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        variant_value(variant, value.serialize(self)?)
    }

    fn serialize_seq(self, length: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(Compound::sequence(length)?)
    }

    fn serialize_tuple(self, length: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(Compound::sequence(Some(length))?)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        length: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(Compound::sequence(Some(length))?)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        length: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(Compound::tuple_variant(variant, length)?)
    }

    fn serialize_map(self, length: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(Compound::map(length)?)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        length: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(Compound::map(Some(length))?)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        length: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(Compound::struct_variant(variant, length)?)
    }
}

enum CompoundState {
    Sequence(Vec<Value>),
    Map {
        entries: Vec<(Value, Value)>,
        pending_key: Option<Value>,
    },
    TupleVariant {
        name: String,
        values: Vec<Value>,
    },
    StructVariant {
        name: String,
        entries: Vec<(Value, Value)>,
        pending_key: Option<Value>,
    },
}

struct Compound {
    state: CompoundState,
}

impl Compound {
    fn sequence(length: Option<usize>) -> Result<Self, NativeError> {
        Ok(Self {
            state: CompoundState::Sequence(reserve_vec(length)?),
        })
    }

    fn map(length: Option<usize>) -> Result<Self, NativeError> {
        Ok(Self {
            state: CompoundState::Map {
                entries: reserve_vec(length)?,
                pending_key: None,
            },
        })
    }

    fn tuple_variant(name: &'static str, length: usize) -> Result<Self, NativeError> {
        Ok(Self {
            state: CompoundState::TupleVariant {
                name: name.to_owned(),
                values: reserve_vec(Some(length))?,
            },
        })
    }

    fn struct_variant(name: &'static str, length: usize) -> Result<Self, NativeError> {
        Ok(Self {
            state: CompoundState::StructVariant {
                name: name.to_owned(),
                entries: reserve_vec(Some(length))?,
                pending_key: None,
            },
        })
    }

    fn push_value(&mut self, value: Value) -> Result<(), NativeError> {
        match &mut self.state {
            CompoundState::Sequence(values) | CompoundState::TupleVariant { values, .. } => {
                values.push(value);
                Ok(())
            }
            CompoundState::Map { .. } | CompoundState::StructVariant { .. } => Err(
                NativeError::message("a sequence element was supplied to a map serializer"),
            ),
        }
    }

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), NativeError>
    where
        T: ?Sized + Serialize,
    {
        let key = key.serialize(ValueSerializer)?;
        match &mut self.state {
            CompoundState::Map { pending_key, .. }
            | CompoundState::StructVariant { pending_key, .. } => {
                if pending_key.is_some() {
                    return Err(NativeError::message(
                        "map serializer received a key before its previous value",
                    ));
                }
                *pending_key = Some(key);
                Ok(())
            }
            CompoundState::Sequence { .. } | CompoundState::TupleVariant { .. } => Err(
                NativeError::message("a map key was supplied to a sequence serializer"),
            ),
        }
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), NativeError>
    where
        T: ?Sized + Serialize,
    {
        let value = value.serialize(ValueSerializer)?;
        match &mut self.state {
            CompoundState::Map {
                entries,
                pending_key,
            }
            | CompoundState::StructVariant {
                entries,
                pending_key,
                ..
            } => {
                let key = pending_key.take().ok_or_else(|| {
                    NativeError::message("map serializer received a value without a key")
                })?;
                entries.push((key, value));
                Ok(())
            }
            CompoundState::Sequence { .. } | CompoundState::TupleVariant { .. } => Err(
                NativeError::message("a map value was supplied to a sequence serializer"),
            ),
        }
    }

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), NativeError>
    where
        T: ?Sized + Serialize,
    {
        self.serialize_key(&key)?;
        self.serialize_value(value)
    }
}

// `CompoundState::finish` needs to distinguish the two sequence variants
// after moving the state. Keep this small helper separate so all compound
// implementations use the same finalization path.
fn finish_sequence_or_variant(state: CompoundState) -> Result<Value, NativeError> {
    match state {
        CompoundState::Sequence(values) => Ok(Value::array(values)),
        CompoundState::TupleVariant { name, values } => variant_value(&name, Value::array(values)),
        CompoundState::Map {
            entries,
            pending_key,
        } => {
            if pending_key.is_some() {
                return Err(NativeError::message(
                    "map serializer ended with a key without a value",
                ));
            }
            checked_map(entries)
        }
        CompoundState::StructVariant {
            name,
            entries,
            pending_key,
        } => {
            if pending_key.is_some() {
                return Err(NativeError::message(
                    "struct variant serializer ended with a key without a value",
                ));
            }
            variant_value(&name, checked_map(entries)?)
        }
    }
}

impl SerializeSeq for Compound {
    type Ok = Value;
    type Error = NativeError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.push_value(value.serialize(ValueSerializer)?)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        finish_sequence_or_variant(self.state)
    }
}

impl SerializeTuple for Compound {
    type Ok = Value;
    type Error = NativeError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.push_value(value.serialize(ValueSerializer)?)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        finish_sequence_or_variant(self.state)
    }
}

impl SerializeTupleStruct for Compound {
    type Ok = Value;
    type Error = NativeError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.push_value(value.serialize(ValueSerializer)?)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        finish_sequence_or_variant(self.state)
    }
}

impl SerializeTupleVariant for Compound {
    type Ok = Value;
    type Error = NativeError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.push_value(value.serialize(ValueSerializer)?)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        finish_sequence_or_variant(self.state)
    }
}

impl SerializeMap for Compound {
    type Ok = Value;
    type Error = NativeError;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        Compound::serialize_key(self, key)
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        Compound::serialize_value(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        finish_sequence_or_variant(self.state)
    }
}

impl SerializeStruct for Compound {
    type Ok = Value;
    type Error = NativeError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        Compound::serialize_field(self, key, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        finish_sequence_or_variant(self.state)
    }
}

impl SerializeStructVariant for Compound {
    type Ok = Value;
    type Error = NativeError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        Compound::serialize_field(self, key, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        finish_sequence_or_variant(self.state)
    }
}

fn reserve_vec<T>(length: Option<usize>) -> Result<Vec<T>, NativeError> {
    let mut values = Vec::new();
    if let Some(length) = length {
        values
            .try_reserve_exact(length)
            .map_err(|_| NativeError::message("failed to allocate native compound value"))?;
    }
    Ok(values)
}

fn checked_map(entries: Vec<(Value, Value)>) -> Result<Value, NativeError> {
    Value::map(entries)
        .map_err(|error| NativeError::message(format!("invalid native map: {error}")))
}

fn variant_value(name: &str, value: Value) -> Result<Value, NativeError> {
    checked_map(vec![(Value::text(name), value)])
}

// `Value` owns a custom non-recursive destructor, so Rust intentionally does
// not allow moving one of its fields through pattern matching. The native
// decoder consumes values as it walks them; this detached representation lets
// it move owned strings, bytes, and child containers without reintroducing
// recursive drops.
enum OwnedValue {
    Undefined,
    Null,
    Boolean(bool),
    Integer(Integer),
    Float(Float),
    TextString(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
}

fn into_owned(value: Value) -> OwnedValue {
    let value = ManuallyDrop::new(value);
    // SAFETY: `value` is never dropped after its owned fields are read. Every
    // field that is moved below is read exactly once, and scalar variants do
    // not own heap data.
    unsafe {
        match &*value {
            Value::Undefined => OwnedValue::Undefined,
            Value::Null => OwnedValue::Null,
            Value::Boolean(value) => OwnedValue::Boolean(*value),
            Value::Integer(value) => OwnedValue::Integer(ptr::read(value)),
            Value::Float(value) => OwnedValue::Float(*value),
            Value::TextString(value) => OwnedValue::TextString(ptr::read(value)),
            Value::Bytes(value) => OwnedValue::Bytes(ptr::read(value)),
            Value::Array(value) => OwnedValue::Array(ptr::read(value)),
            Value::Map(value) => OwnedValue::Map(ptr::read(value)),
        }
    }
}

/// A deserializer over one owned structured value.
struct ValueDeserializer {
    value: OwnedValue,
}

impl ValueDeserializer {
    fn new(value: Value) -> Self {
        Self {
            value: into_owned(value),
        }
    }

    fn from_owned(value: OwnedValue) -> Self {
        Self { value }
    }

    fn type_error<T>(expected: &str, actual: &OwnedValue) -> Result<T, NativeError> {
        Err(NativeError::expected_owned(expected, actual))
    }

    fn integer<T>(
        self,
        expected: &str,
        convert: impl FnOnce(&Integer) -> Option<T>,
    ) -> Result<T, NativeError> {
        match self.value {
            OwnedValue::Integer(value) => convert(&value)
                .ok_or_else(|| NativeError::message(format!("{expected} integer is out of range"))),
            value => Self::type_error(expected, &value),
        }
    }

    fn float(self, expected: &str) -> Result<Float, NativeError> {
        match self.value {
            OwnedValue::Float(value) => Ok(value),
            value => Self::type_error(expected, &value),
        }
    }
}

impl<'de> Deserializer<'de> for ValueDeserializer {
    type Error = NativeError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Undefined => Err(NativeError::message(
                "undefined has no native Serde representation",
            )),
            OwnedValue::Null => visitor.visit_unit(),
            OwnedValue::Boolean(value) => visitor.visit_bool(value),
            OwnedValue::Integer(value) => {
                if value.is_negative() {
                    value
                        .as_i128()
                        .map(|value| visitor.visit_i128(value))
                        .unwrap_or_else(|| {
                            Err(NativeError::message(
                                "negative integer does not fit in native i128",
                            ))
                        })
                } else {
                    value
                        .as_u128()
                        .map(|value| visitor.visit_u128(value))
                        .unwrap_or_else(|| {
                            Err(NativeError::message("integer does not fit in native u128"))
                        })
                }
            }
            OwnedValue::Float(value) => visitor.visit_f64(float_to_f64(value)?),
            OwnedValue::TextString(value) => visitor.visit_string(value),
            OwnedValue::Bytes(value) => visitor.visit_byte_buf(value),
            OwnedValue::Array(value) => visitor.visit_seq(ValueSeqAccess::new(value)),
            OwnedValue::Map(value) => visitor.visit_map(ValueMapAccess::new(value)),
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Boolean(value) => visitor.visit_bool(value),
            value => Self::type_error("a boolean", &value),
        }
    }

    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.integer("an i8", |value| {
            value.as_i128().and_then(|v| i8::try_from(v).ok())
        })
        .and_then(|value| visitor.visit_i8(value))
    }

    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.integer("an i16", |value| {
            value.as_i128().and_then(|v| i16::try_from(v).ok())
        })
        .and_then(|value| visitor.visit_i16(value))
    }

    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.integer("an i32", |value| {
            value.as_i128().and_then(|v| i32::try_from(v).ok())
        })
        .and_then(|value| visitor.visit_i32(value))
    }

    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.integer("an i64", |value| {
            value.as_i128().and_then(|v| i64::try_from(v).ok())
        })
        .and_then(|value| visitor.visit_i64(value))
    }

    fn deserialize_i128<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.integer("an i128", Integer::as_i128)
            .and_then(|value| visitor.visit_i128(value))
    }

    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.integer("a u8", |value| {
            value.as_u128().and_then(|v| u8::try_from(v).ok())
        })
        .and_then(|value| visitor.visit_u8(value))
    }

    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.integer("a u16", |value| {
            value.as_u128().and_then(|v| u16::try_from(v).ok())
        })
        .and_then(|value| visitor.visit_u16(value))
    }

    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.integer("a u32", |value| {
            value.as_u128().and_then(|v| u32::try_from(v).ok())
        })
        .and_then(|value| visitor.visit_u32(value))
    }

    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.integer("a u64", |value| {
            value.as_u128().and_then(|v| u64::try_from(v).ok())
        })
        .and_then(|value| visitor.visit_u64(value))
    }

    fn deserialize_u128<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.integer("a u128", Integer::as_u128)
            .and_then(|value| visitor.visit_u128(value))
    }

    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let source = float_to_f64(self.float("an f32")?)?;
        let value = source as f32;
        if source.is_finite() && !value.is_finite() {
            return Err(NativeError::message("float is out of f32 range"));
        }
        visitor.visit_f32(value)
    }

    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_f64(float_to_f64(self.float("an f64")?)?)
    }

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::TextString(value) => {
                let mut chars = value.chars();
                let character = chars
                    .next()
                    .ok_or_else(|| NativeError::message("expected one Unicode scalar value"))?;
                if chars.next().is_some() {
                    return Err(NativeError::message(
                        "expected one Unicode scalar value, found a string",
                    ));
                }
                visitor.visit_char(character)
            }
            value => Self::type_error("a character", &value),
        }
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::TextString(value) => visitor.visit_string(value),
            value => Self::type_error("a string", &value),
        }
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Bytes(value) => visitor.visit_byte_buf(value),
            value => Self::type_error("a byte string", &value),
        }
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Null => visitor.visit_none(),
            value => visitor.visit_some(ValueDeserializer::from_owned(value)),
        }
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Null => visitor.visit_unit(),
            value => Self::type_error("null", &value),
        }
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Array(value) => visitor.visit_seq(ValueSeqAccess::new(value)),
            value => Self::type_error("a sequence", &value),
        }
    }

    fn deserialize_tuple<V>(self, length: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Array(value) if value.len() == length => {
                visitor.visit_seq(ValueSeqAccess::new(value))
            }
            OwnedValue::Array(value) => Err(NativeError::message(format!(
                "expected tuple of length {length}, found {}",
                value.len()
            ))),
            value => Self::type_error("a tuple", &value),
        }
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        length: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_tuple(length, visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            OwnedValue::Map(value) => visitor.visit_map(ValueMapAccess::new(value)),
            value => Self::type_error("a map", &value),
        }
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (variant, value) = match self.value {
            OwnedValue::TextString(variant) => (variant, Value::Null),
            OwnedValue::Map(mut entries) if entries.len() == 1 => {
                let (key, value) = entries.pop().expect("a one-entry map contains one entry");
                let OwnedValue::TextString(variant) = into_owned(key) else {
                    return Err(NativeError::message(
                        "externally tagged enum key must be a string",
                    ));
                };
                (variant, value)
            }
            OwnedValue::Map(entries) => {
                return Err(NativeError::message(format!(
                    "externally tagged enum requires one variant entry, found {}",
                    entries.len()
                )));
            }
            value => return Self::type_error("an externally tagged enum", &value),
        };
        visitor.visit_enum(ValueEnumAccess { variant, value })
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }
}

struct ValueSeqAccess {
    values: std::vec::IntoIter<Value>,
}

impl ValueSeqAccess {
    fn new(values: Vec<Value>) -> Self {
        Self {
            values: values.into_iter(),
        }
    }
}

impl<'de> SeqAccess<'de> for ValueSeqAccess {
    type Error = NativeError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        self.values
            .next()
            .map(|value| seed.deserialize(ValueDeserializer::new(value)))
            .transpose()
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.values.len())
    }
}

struct ValueMapAccess {
    entries: std::vec::IntoIter<(Value, Value)>,
    pending_value: Option<Value>,
}

impl ValueMapAccess {
    fn new(entries: Vec<(Value, Value)>) -> Self {
        Self {
            entries: entries.into_iter(),
            pending_value: None,
        }
    }
}

impl<'de> MapAccess<'de> for ValueMapAccess {
    type Error = NativeError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        if self.pending_value.is_some() {
            return Err(NativeError::message(
                "map value was not consumed before requesting the next key",
            ));
        }
        let Some((key, value)) = self.entries.next() else {
            return Ok(None);
        };
        self.pending_value = Some(value);
        seed.deserialize(ValueDeserializer::new(key)).map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let value = self
            .pending_value
            .take()
            .ok_or_else(|| NativeError::message("map value requested before its key"))?;
        seed.deserialize(ValueDeserializer::new(value))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len() + usize::from(self.pending_value.is_some()))
    }
}

struct ValueEnumAccess {
    variant: String,
    value: Value,
}

impl<'de> EnumAccess<'de> for ValueEnumAccess {
    type Error = NativeError;
    type Variant = ValueVariantAccess;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let variant = seed.deserialize(self.variant.into_deserializer())?;
        Ok((variant, ValueVariantAccess { value: self.value }))
    }
}

struct ValueVariantAccess {
    value: Value,
}

impl<'de> VariantAccess<'de> for ValueVariantAccess {
    type Error = NativeError;

    fn unit_variant(self) -> Result<(), Self::Error> {
        match into_owned(self.value) {
            OwnedValue::Null => Ok(()),
            value => Err(NativeError::expected_owned("null enum payload", &value)),
        }
    }

    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        seed.deserialize(ValueDeserializer::new(self.value))
    }

    fn tuple_variant<V>(self, length: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        ValueDeserializer::new(self.value).deserialize_tuple(length, visitor)
    }

    fn struct_variant<V>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        ValueDeserializer::new(self.value).deserialize_struct("", fields, visitor)
    }
}

fn float_to_f64(value: Float) -> Result<f64, NativeError> {
    if !value.is_valid() {
        return Err(NativeError::message(format!(
            "invalid {:?} float raw bits {:#018x}",
            value.width, value.raw_bits
        )));
    }
    Ok(match value.width {
        FloatWidth::Bits16 => half_to_f64(value.raw_bits as u16),
        FloatWidth::Bits32 => f32::from_bits(value.raw_bits as u32) as f64,
        FloatWidth::Bits64 => f64::from_bits(value.raw_bits),
    })
}

fn half_to_f64(bits: u16) -> f64 {
    let sign = u64::from(bits & 0x8000) << 48;
    let exponent = (bits >> 10) & 0x1f;
    let fraction = u64::from(bits & 0x03ff);
    let raw = match exponent {
        0 => {
            if fraction == 0 {
                sign
            } else {
                let mut value = fraction as f64;
                value *= 2_f64.powi(-24);
                return if sign == 0 { value } else { -value };
            }
        }
        0x1f => sign | 0x7ff0_0000_0000_0000 | (fraction << 42),
        _ => {
            let exponent = u64::from(exponent) + (1023 - 15);
            sign | (exponent << 52) | (fraction << 42)
        }
    };
    f64::from_bits(raw)
}

fn owned_value_type_name(value: &OwnedValue) -> &'static str {
    match value {
        OwnedValue::Undefined => "undefined",
        OwnedValue::Null => "null",
        OwnedValue::Boolean(_) => "a boolean",
        OwnedValue::Integer(_) => "an integer",
        OwnedValue::Float(_) => "a float",
        OwnedValue::TextString(_) => "a string",
        OwnedValue::Bytes(_) => "a byte string",
        OwnedValue::Array(_) => "an array",
        OwnedValue::Map(_) => "a map",
    }
}
