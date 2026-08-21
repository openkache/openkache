//! Versioned client value envelopes and value-key protection.
//!
//! The server receives [`ItemValue`] as an opaque byte string.  This module
//! owns the client-only boundary described by `VALUE_FORMAT.md` and
//! `VALUE_SECURITY.md`: version and selector dispatch, exact opaque bytes,
//! StructuredValue-CBOR-v1, bounded Zstandard, and authenticated protection.
//! Raw protocol operations never call this module and therefore never sniff or
//! reinterpret stored bytes.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::io::{self, Write};
use std::mem::{ManuallyDrop, size_of};
use std::sync::{Arc, OnceLock};

use aes_gcm_siv::aead::{AeadInOut, KeyInit as _};
use aes_gcm_siv::{Aes256GcmSiv, Nonce, Tag};
use aes_siv::siv::Aes256Siv;
use openkache_protocol::ITEM_ID_BYTES;
use openkache_value::{
    Limits as StructuredLimits, Value as StructuredValue, decode_with_limits_and_budget,
    encode_with_limits_and_budget,
};
use serde::de::{Deserialize, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use zeroize::{Zeroize, Zeroizing};
use zstd_pure_rs::prelude::{
    ERR_getErrorName, ERR_isError, ZSTD_CONTENTSIZE_UNKNOWN, ZSTD_FrameHeader, ZSTD_FrameType_e,
    ZSTD_compress, ZSTD_compressBound, ZSTD_decompress, ZSTD_findFrameCompressedSize,
    ZSTD_getFrameHeader,
};

use crate::contract::{
    DEFAULT_ZSTANDARD_LEVEL, DEFAULT_ZSTANDARD_LEVEL_MAX, DEFAULT_ZSTANDARD_LEVEL_MIN,
};
use crate::transport::RequestBudget;
use crate::{DATA_PROTECTION_KEY_BYTES, DataProtectionKey, ItemId, ValueBytePermit as BytePermit};

/// The one OpenKache-defined envelope grammar implemented by this module.
pub const VERSION: u128 = 1;
/// Exact bytes of the canonical version field.
pub const VERSION_BYTES: &[u8] = &[1];
/// Maximum complete envelope size (64 MiB).
pub const MAX_VALUE_ENVELOPE_BYTES: usize = 67_108_864;
/// Maximum expanded payload size (64 MiB).
pub const MAX_EXPANDED_PAYLOAD_BYTES: usize = 67_108_864;
/// Maximum Zstandard decoder window (64 MiB).
pub const MAX_ZSTD_WINDOW_BYTES: usize = 67_108_864;
/// Maximum compatibility-value nesting depth.
///
/// JSON parsing and compatibility conversion use bounded recursive adapters;
/// keep their caller-selected depth below the public structured codec's much
/// larger iterative policy so a configuration value cannot reintroduce native
/// stack growth.
pub const MAX_VALUE_DEPTH: usize = openkache_value::DEFAULT_MAX_DEPTH;
/// Maximum canonical unsigned-64-bit `vu128` width used by this profile.
pub const MAX_VU128_BYTES: usize = 9;
/// Number of bytes in an AES-256-GCM-SIV nonce.
pub const GCM_SIV_NONCE_BYTES: usize = 12;
/// Number of bytes in an AEAD authentication tag.
pub const AUTH_TAG_BYTES: usize = 16;
/// Number of bytes in an AES-SIV synthetic IV.
pub const SIV_SYNTHETIC_IV_BYTES: usize = 16;
/// The value-key size required by both authenticated profiles.
pub const VALUE_KEY_BYTES: usize = DATA_PROTECTION_KEY_BYTES;
/// Compatibility spelling retained for existing adapters.
pub const ENCRYPTION_KEY_BYTES: usize = VALUE_KEY_BYTES;

const AAD_DOMAIN: &[u8] = b"openkache/value-format/aad/v1";
const VALUE_ROOT_CONTEXT: &str = "OpenKache value format v1 root key";
const SIV_MAC_CONTEXT: &str = "OpenKache value format v1 AES-256-SIV-CMAC MAC key";
const SIV_ENCRYPTION_CONTEXT: &str = "OpenKache value format v1 AES-256-SIV-CMAC encryption key";
const GCM_SIV_CONTEXT: &str = "OpenKache value format v1 AES-256-GCM-SIV key";
const BINARY64_SIGNIFICAND_BITS: u32 = 53;

const PROTECTION_UNPROTECTED: u8 = 0;
const PROTECTION_GCM_SIV: u8 = 1;
const PROTECTION_SIV_CMAC: u8 = 2;
const COMPRESSION_NONE: u8 = 0;
const COMPRESSION_ZSTANDARD: u8 = 1;
const PAYLOAD_OPAQUE_BYTES: u8 = 0;
const PAYLOAD_STRUCTURED_CBOR_V1: u8 = 1;

const PROTECTION_MASK: u8 = 0b0000_0011;
const COMPRESSION_MASK: u8 = 0b0000_1100;
const PAYLOAD_MASK: u8 = 0b0011_0000;
const RESERVED_SELECTOR_MASK: u8 = 0b1100_0000;

/// Client-owned encoded bytes stored opaquely by the server.
#[derive(Clone)]
pub struct ItemValue {
    bytes: Vec<u8>,
    /// Transport response lease retained while a protected client decodes
    /// this value. Raw callers never observe or construct this field.
    response_permit: Option<Arc<BytePermit>>,
}

impl ItemValue {
    /// Wrap exact bytes returned by or sent to the raw protocol API.
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            response_permit: None,
        }
    }

    /// Wrap exact plaintext bytes for raw protocol operations.
    pub const fn plaintext(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }

    /// Borrow the complete opaque byte string.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consume the wrapper and return the complete opaque byte string.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Return whether the opaque byte string is empty.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return the complete opaque byte length.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn with_response_permit(bytes: Vec<u8>, permit: Option<BytePermit>) -> Self {
        Self {
            bytes,
            response_permit: permit.map(Arc::new),
        }
    }

    fn into_budgeted_parts(self) -> (Vec<u8>, Option<Arc<BytePermit>>) {
        (self.bytes, self.response_permit)
    }
}

impl fmt::Debug for ItemValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ItemValue")
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl PartialEq for ItemValue {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl Eq for ItemValue {}

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

/// Common logical JSON value retained as a compatibility adapter.
#[derive(Debug, PartialEq)]
pub enum JsonValue {
    /// JSON `null`.
    Null,
    /// JSON Boolean.
    Boolean(bool),
    /// Finite IEEE-754 binary64 number.
    Number(f64),
    /// Unicode string.
    String(String),
    /// Ordered array.
    Array(Vec<Self>),
    /// Object entries with unique names.
    Object(Vec<(String, Self)>),
}

impl Clone for JsonValue {
    fn clone(&self) -> Self {
        enum Task<'a> {
            Visit(&'a JsonValue),
            Array(usize),
            Object(&'a [(String, JsonValue)]),
        }

        let mut tasks = Vec::new();
        let mut values = Vec::new();
        tasks.push(Task::Visit(self));
        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(value) => match value {
                    JsonValue::Null => values.push(JsonValue::Null),
                    JsonValue::Boolean(value) => values.push(JsonValue::Boolean(*value)),
                    JsonValue::Number(value) => values.push(JsonValue::Number(*value)),
                    JsonValue::String(value) => values.push(JsonValue::String(value.clone())),
                    JsonValue::Array(children) => {
                        tasks.push(Task::Array(children.len()));
                        for child in children.iter().rev() {
                            tasks.push(Task::Visit(child));
                        }
                    }
                    JsonValue::Object(entries) => {
                        tasks.push(Task::Object(entries));
                        for (_, child) in entries.iter().rev() {
                            tasks.push(Task::Visit(child));
                        }
                    }
                },
                Task::Array(length) => {
                    let start = values
                        .len()
                        .checked_sub(length)
                        .expect("every array child was cloned before its parent");
                    let children: Vec<_> = values.drain(start..).collect();
                    values.push(JsonValue::Array(children));
                }
                Task::Object(entries) => {
                    let start = values
                        .len()
                        .checked_sub(entries.len())
                        .expect("every object child was cloned before its parent");
                    let children: Vec<_> = values.drain(start..).collect();
                    let entries = entries
                        .iter()
                        .zip(children)
                        .map(|((key, _), value)| (key.clone(), value))
                        .collect();
                    values.push(JsonValue::Object(entries));
                }
            }
        }
        values
            .pop()
            .expect("the root JSON value always produces one clone")
    }
}

impl Drop for JsonValue {
    fn drop(&mut self) {
        let root = std::mem::replace(self, Self::Null);
        let mut pending = vec![root];
        while let Some(value) = pending.pop() {
            let mut value = ManuallyDrop::new(value);
            // SAFETY: each container is emptied before its shell is forgotten,
            // so no nested JsonValue destructor can recurse on this stack.
            unsafe {
                match &mut *value {
                    Self::Array(children) => pending.extend(std::mem::take(children)),
                    Self::Object(entries) => {
                        for (key, child) in std::mem::take(entries) {
                            drop(key);
                            pending.push(child);
                        }
                    }
                    Self::String(value) => std::ptr::drop_in_place(value),
                    Self::Null | Self::Boolean(_) | Self::Number(_) => {}
                }
                std::mem::forget(value);
            }
        }
    }
}

impl JsonValue {
    /// Construct a finite JSON number.
    pub fn number(value: f64) -> Result<Self> {
        if value.is_finite() {
            Ok(Self::Number(value))
        } else {
            Err(Error::InvalidJson(
                "JSON numbers must be finite IEEE-754 values".into(),
            ))
        }
    }

    /// Construct an object after checking duplicate property names.
    pub fn object(entries: Vec<(String, Self)>) -> Result<Self> {
        validate_json_object(&entries)?;
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
        formatter.write_str("a finite JSON value")
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

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let mut owned = String::new();
        owned
            .try_reserve_exact(value.len())
            .map_err(|_| E::custom("failed to allocate JSON string"))?;
        owned.push_str(value);
        Ok(JsonValue::String(owned))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(JsonValue::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        if let Some(length) = sequence.size_hint() {
            values
                .try_reserve_exact(length)
                .map_err(|_| serde::de::Error::custom("failed to allocate JSON array"))?;
        }
        while let Some(value) = sequence.next_element()? {
            values
                .try_reserve(1)
                .map_err(|_| serde::de::Error::custom("failed to grow JSON array"))?;
            values.push(value);
        }
        Ok(JsonValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::new();
        let mut keys = HashSet::<String>::new();
        if let Some(length) = map.size_hint() {
            entries
                .try_reserve_exact(length)
                .map_err(|_| serde::de::Error::custom("failed to allocate JSON object"))?;
            keys.try_reserve(length)
                .map_err(|_| serde::de::Error::custom("failed to allocate JSON keys"))?;
        }
        while let Some(key) = map.next_key::<String>()? {
            keys.try_reserve(1)
                .map_err(|_| serde::de::Error::custom("failed to grow JSON keys"))?;
            let mut key_copy = String::new();
            key_copy
                .try_reserve_exact(key.len())
                .map_err(|_| serde::de::Error::custom("failed to allocate JSON key"))?;
            key_copy.push_str(&key);
            if !keys.insert(key_copy) {
                return Err(serde::de::Error::custom("duplicate JSON object property"));
            }
            entries
                .try_reserve(1)
                .map_err(|_| serde::de::Error::custom("failed to grow JSON object"))?;
            entries.push((key, map.next_value()?));
        }
        Ok(JsonValue::Object(entries))
    }
}

/// State retained while a compatibility JSON document is decoded.
///
/// `serde_json` normally allocates the complete `JsonValue` recursively before
/// the value codec can apply its aggregate budget.  The FFI adapters cannot
/// afford that unbounded admission path: native callers can supply arbitrary
/// documents and the parser is itself part of the request.  This state tracks
/// the caller-selected structural limits and retains permits for every
/// allocation made by the parser until the caller has finished with the
/// resulting model.
struct JsonParseState<'a> {
    budget: &'a RequestBudget,
    limits: ValueLimits,
    permits: Vec<BytePermit>,
    item_count: usize,
    failure: Option<Error>,
}

impl JsonParseState<'_> {
    fn new(budget: &RequestBudget, limits: ValueLimits) -> JsonParseState<'_> {
        JsonParseState {
            budget,
            limits,
            permits: Vec::new(),
            item_count: 0,
            failure: None,
        }
    }

    fn reserve(&mut self, size: usize) -> Result<()> {
        let permit = reserve_budget(self.budget, size, &self.limits, Resource::StructuredValue)?;
        self.permits
            .try_reserve(1)
            .map_err(|_| Error::Allocation { size: 1 })?;
        self.permits.push(permit);
        Ok(())
    }

    fn reserve_vec<T>(&mut self, length: usize) -> Result<Vec<T>> {
        let bytes = length
            .checked_mul(size_of::<T>())
            .ok_or_else(|| structured_resource(self.limits.max_in_flight_bytes, usize::MAX))?;
        self.reserve(bytes)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(length)
            .map_err(|_| Error::Allocation { size: bytes })?;
        Ok(values)
    }

    fn reserve_hash_set<T: Eq + std::hash::Hash>(&mut self, length: usize) -> Result<HashSet<T>> {
        let bytes = hash_set_allocation_bytes::<T>(length)?;
        self.reserve(bytes)?;
        let mut values = HashSet::new();
        values
            .try_reserve(length)
            .map_err(|_| Error::Allocation { size: bytes })?;
        Ok(values)
    }

    fn clone_string(&mut self, value: &str) -> Result<String> {
        self.reserve(value.len())?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(value.len())
            .map_err(|_| Error::Allocation { size: value.len() })?;
        owned.push_str(value);
        Ok(owned)
    }

    fn admit_item(&mut self) -> Result<()> {
        self.item_count = self
            .item_count
            .checked_add(1)
            .ok_or_else(|| structured_resource(self.limits.max_items, usize::MAX))?;
        if self.item_count > self.limits.max_items {
            return Err(structured_resource(self.limits.max_items, self.item_count));
        }
        Ok(())
    }

    fn admit_children(&mut self, count: usize) -> Result<()> {
        let total = self
            .item_count
            .checked_add(count)
            .ok_or_else(|| structured_resource(self.limits.max_items, usize::MAX))?;
        if total > self.limits.max_items {
            return Err(structured_resource(self.limits.max_items, total));
        }
        Ok(())
    }

    fn reject<E: serde::de::Error>(&mut self, error: Error) -> E {
        if self.failure.is_none() {
            self.failure = Some(error);
        }
        E::custom("bounded JSON parser rejected input")
    }

    fn finish_error(&mut self, error: impl fmt::Display) -> Error {
        self.failure
            .take()
            .unwrap_or_else(|| Error::InvalidJson(error.to_string()))
    }
}

struct JsonBudgetSeed<'a, 'b> {
    state: &'a mut JsonParseState<'b>,
    depth: usize,
}

impl<'de, 'a, 'b> DeserializeSeed<'de> for JsonBudgetSeed<'a, 'b> {
    type Value = JsonValue;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        self.state
            .admit_item()
            .map_err(|error| self.state.reject(error))?;
        self.state
            .reserve(size_of::<JsonValue>())
            .map_err(|error| self.state.reject(error))?;
        deserializer.deserialize_any(JsonBudgetVisitor {
            state: self.state,
            depth: self.depth,
        })
    }
}

struct JsonStringSeed<'a, 'b> {
    state: &'a mut JsonParseState<'b>,
}

impl<'de, 'a, 'b> DeserializeSeed<'de> for JsonStringSeed<'a, 'b> {
    type Value = String;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        self.state
            .admit_item()
            .map_err(|error| self.state.reject(error))?;
        self.state
            .reserve(size_of::<(String, JsonValue)>())
            .and_then(|()| self.state.reserve(size_of::<&str>()))
            .map_err(|error| self.state.reject(error))?;
        deserializer.deserialize_string(JsonStringVisitor { state: self.state })
    }
}

struct JsonStringVisitor<'a, 'b> {
    state: &'a mut JsonParseState<'b>,
}

impl<'de> Visitor<'de> for JsonStringVisitor<'_, '_> {
    type Value = String;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object property name")
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.state
            .clone_string(value)
            .map_err(|error| self.state.reject(error))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(value)
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.state
            .reserve(value.len())
            .map(|()| value)
            .map_err(|error| self.state.reject(error))
    }
}

struct JsonBudgetVisitor<'a, 'b> {
    state: &'a mut JsonParseState<'b>,
    depth: usize,
}

impl<'de> Visitor<'de> for JsonBudgetVisitor<'_, '_> {
    type Value = JsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a finite JSON value")
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
        visit_json_integer(value.unsigned_abs() as u128, value as f64)
            .map_err(|error| self.state.reject(error))
    }

    fn visit_i128<E>(self, value: i128) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        visit_json_integer(value.unsigned_abs(), value as f64)
            .map_err(|error| self.state.reject(error))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        visit_json_integer(value as u128, value as f64).map_err(|error| self.state.reject(error))
    }

    fn visit_u128<E>(self, value: u128) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        visit_json_integer(value, value as f64).map_err(|error| self.state.reject(error))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        JsonValue::number(value).map_err(|error| self.state.reject(error))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.state
            .clone_string(value)
            .map(JsonValue::String)
            .map_err(|error| self.state.reject(error))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(value)
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.state
            .reserve(value.len())
            .map(|()| JsonValue::String(value))
            .map_err(|error| self.state.reject(error))
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if self.depth >= self.state.limits.max_depth {
            return Err(self.state.reject(structured_depth(
                self.state.limits.max_depth,
                self.depth + 1,
            )));
        }
        let length = sequence.size_hint().unwrap_or(0);
        if let Err(error) = self.state.admit_children(length) {
            return Err(self.state.reject(error));
        }
        let mut values = self
            .state
            .reserve_vec::<JsonValue>(length)
            .map_err(|error| self.state.reject(error))?;
        while let Some(value) = {
            let seed = JsonBudgetSeed {
                state: &mut *self.state,
                depth: self.depth + 1,
            };
            sequence.next_element_seed(seed)
        }? {
            if values.len() == values.capacity() {
                self.state
                    .reserve(size_of::<JsonValue>())
                    .map_err(|error| self.state.reject(error))?;
                values.try_reserve_exact(1).map_err(|_| {
                    self.state.reject(Error::Allocation {
                        size: size_of::<JsonValue>(),
                    })
                })?;
            }
            values.push(value);
        }
        Ok(JsonValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        if self.depth >= self.state.limits.max_depth {
            return Err(self.state.reject(structured_depth(
                self.state.limits.max_depth,
                self.depth + 1,
            )));
        }
        let length = map.size_hint().unwrap_or(0);
        let child_count = length
            .checked_mul(2)
            .ok_or_else(|| structured_resource(self.state.limits.max_items, usize::MAX))
            .map_err(|error| self.state.reject(error))?;
        if let Err(error) = self.state.admit_children(child_count) {
            return Err(self.state.reject(error));
        }
        let mut entries = self
            .state
            .reserve_vec::<(String, JsonValue)>(length)
            .map_err(|error| self.state.reject(error))?;
        while let Some(key) = {
            let seed = JsonStringSeed {
                state: &mut *self.state,
            };
            map.next_key_seed(seed)
        }? {
            let value = {
                let seed = JsonBudgetSeed {
                    state: &mut *self.state,
                    depth: self.depth + 1,
                };
                map.next_value_seed(seed)
            }?;
            if entries.len() == entries.capacity() {
                self.state
                    .reserve(size_of::<(String, JsonValue)>())
                    .map_err(|error| self.state.reject(error))?;
                entries.try_reserve_exact(1).map_err(|_| {
                    self.state.reject(Error::Allocation {
                        size: size_of::<(String, JsonValue)>(),
                    })
                })?;
            }
            entries.push((key, value));
        }
        let mut keys = self
            .state
            .reserve_hash_set::<&str>(entries.len())
            .map_err(|error| self.state.reject(error))?;
        for (key, _) in &entries {
            if !keys.insert(key.as_str()) {
                return Err(self
                    .state
                    .reject(Error::InvalidJson("duplicate JSON object property".into())));
            }
        }
        Ok(JsonValue::Object(entries))
    }
}

/// Compatibility logical value. `Raw` is always `OpaqueBytes`; `Json` uses
/// the explicit StructuredValue-CBOR-v1 selector.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Exact application bytes.
    Raw(Vec<u8>),
    /// Compatibility JSON view encoded as StructuredValue-CBOR-v1.
    Json(JsonValue),
}

/// Zstandard write policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ZstandardOptions {
    /// Compression level.
    pub level: i32,
    /// Optional input-size threshold; the maintained default is zero.
    pub minimum_input_size: usize,
    /// Optional minimum savings threshold; the maintained default is zero.
    pub minimum_savings: usize,
}

impl Default for ZstandardOptions {
    fn default() -> Self {
        Self {
            level: DEFAULT_ZSTANDARD_LEVEL,
            minimum_input_size: 0,
            minimum_savings: 0,
        }
    }
}

/// Compression policy for formatted v1 writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Compression {
    /// Emit the payload without compression. Use this for an explicit opt-out
    /// from the maintained automatic policy.
    Disabled,
    /// Try one declared-size Zstandard frame and retain it only when the
    /// completed frame is smaller.
    Zstandard(ZstandardOptions),
}

impl Default for Compression {
    fn default() -> Self {
        Self::Zstandard(ZstandardOptions::default())
    }
}

/// Protection profile selected by the v1 selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encryption {
    /// No confidentiality or authentication.
    Unprotected,
    /// Deterministic AES-SIV-CMAC.
    Compact,
    /// Randomized AES-256-GCM-SIV.
    Robust,
}

impl Encryption {
    const fn selector_id(self) -> u8 {
        match self {
            Self::Unprotected => PROTECTION_UNPROTECTED,
            Self::Robust => PROTECTION_GCM_SIV,
            Self::Compact => PROTECTION_SIV_CMAC,
        }
    }

    const fn from_selector(id: u8) -> Option<Self> {
        match id {
            PROTECTION_UNPROTECTED => Some(Self::Unprotected),
            PROTECTION_GCM_SIV => Some(Self::Robust),
            PROTECTION_SIV_CMAC => Some(Self::Compact),
            _ => None,
        }
    }
}

/// Per-value and decoder resource limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValueLimits {
    /// Complete envelope byte limit.
    pub max_envelope_bytes: usize,
    /// Decrypted/decompressed payload byte limit.
    pub max_expanded_payload_bytes: usize,
    /// Declared Zstandard window limit.
    pub max_zstd_window_bytes: usize,
    /// Structured-value nesting limit.
    pub max_depth: usize,
    /// Structured-value item limit.
    pub max_items: usize,
    /// Structured-value integer magnitude limit.
    pub max_integer_bytes: usize,
    /// Maximum bytes one operation may reserve at a time.
    pub max_in_flight_bytes: usize,
}

impl Default for ValueLimits {
    fn default() -> Self {
        Self {
            max_envelope_bytes: MAX_VALUE_ENVELOPE_BYTES,
            max_expanded_payload_bytes: MAX_EXPANDED_PAYLOAD_BYTES,
            max_zstd_window_bytes: MAX_ZSTD_WINDOW_BYTES,
            max_depth: openkache_value::DEFAULT_MAX_DEPTH,
            max_items: openkache_value::DEFAULT_MAX_ITEMS,
            max_integer_bytes: openkache_value::DEFAULT_MAX_INTEGER_BYTES,
            max_in_flight_bytes: MAX_VALUE_ENVELOPE_BYTES,
        }
    }
}

/// An immutable positive-ID value-key mapping.
pub struct ValueKeyring {
    keys: BTreeMap<u64, Zeroizing<[u8; VALUE_KEY_BYTES]>>,
    active_id: Option<u64>,
}

impl fmt::Debug for ValueKeyring {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValueKeyring")
            .field("key_ids", &self.keys.keys().collect::<Vec<_>>())
            .field("active_id", &self.active_id)
            .finish()
    }
}

impl ValueKeyring {
    /// Creates an empty read-only keyring.
    pub fn new() -> Self {
        Self {
            keys: BTreeMap::new(),
            active_id: None,
        }
    }

    /// Creates a keyring containing one active key.
    pub fn single(id: u64, key: [u8; VALUE_KEY_BYTES]) -> Result<Self> {
        let mut ring = Self::new();
        ring.insert(id, key)?;
        ring.active_id = Some(id);
        Ok(ring)
    }

    /// Creates the compatibility keyring derived from a client root key.
    pub(crate) fn from_root_key(key: &DataProtectionKey) -> Result<Self> {
        Self::single(1, *key.master_key())
    }

    /// Adds an immutable positive key ID.
    pub fn insert(&mut self, id: u64, key: [u8; VALUE_KEY_BYTES]) -> Result<()> {
        if id == 0 {
            return Err(Error::InvalidValueKeyId(id));
        }
        if key.iter().all(|byte| *byte == 0) {
            return Err(Error::InvalidValueKey);
        }
        if let Some(previous) = self.keys.get(&id) {
            if previous.as_slice() != key {
                return Err(Error::ValueKeyIdRebound(id));
            }
            return Ok(());
        }
        self.keys.insert(id, Zeroizing::new(key));
        Ok(())
    }

    /// Select the active write key ID.
    pub fn set_active_id(&mut self, id: Option<u64>) -> Result<()> {
        if let Some(id) = id {
            if id == 0 {
                return Err(Error::InvalidValueKeyId(id));
            }
            if !self.keys.contains_key(&id) {
                return Err(Error::KeyUnavailable(id));
            }
        }
        self.active_id = id;
        Ok(())
    }

    /// Return the active write key ID, when configured.
    pub const fn active_id(&self) -> Option<u64> {
        self.active_id
    }

    /// Return a key by its exact positive ID.
    pub fn get(&self, id: u64) -> Option<&[u8; VALUE_KEY_BYTES]> {
        self.keys.get(&id).map(|key| &**key)
    }
}

impl Default for ValueKeyring {
    fn default() -> Self {
        Self::new()
    }
}

/// Reusable value encoder/decoder with explicit key and budget boundaries.
pub struct ValueCodec {
    compression: Compression,
    encryption: Encryption,
    keyring: Option<ValueKeyring>,
    read_profiles: u8,
    limits: ValueLimits,
    budget: Option<RequestBudget>,
    default_budget: OnceLock<RequestBudget>,
}

impl Default for ValueCodec {
    fn default() -> Self {
        Self::plaintext()
    }
}

impl ValueCodec {
    /// Create an unprotected, uncompressed codec.
    pub const fn plaintext() -> Self {
        Self {
            compression: Compression::Disabled,
            encryption: Encryption::Unprotected,
            keyring: None,
            read_profiles: 1 << PROTECTION_UNPROTECTED,
            limits: ValueLimits {
                max_envelope_bytes: MAX_VALUE_ENVELOPE_BYTES,
                max_expanded_payload_bytes: MAX_EXPANDED_PAYLOAD_BYTES,
                max_zstd_window_bytes: MAX_ZSTD_WINDOW_BYTES,
                max_depth: openkache_value::DEFAULT_MAX_DEPTH,
                max_items: openkache_value::DEFAULT_MAX_ITEMS,
                max_integer_bytes: openkache_value::DEFAULT_MAX_INTEGER_BYTES,
                max_in_flight_bytes: MAX_VALUE_ENVELOPE_BYTES,
            },
            budget: None,
            default_budget: OnceLock::new(),
        }
    }

    /// Create an unprotected codec with a compression policy.
    pub fn compressed(compression: Compression) -> Result<Self> {
        validate_compression(compression)?;
        Ok(Self {
            compression,
            ..Self::plaintext()
        })
    }

    /// Create the default GCM-SIV codec from a client root key.
    pub fn protected(key: &DataProtectionKey, compression: Compression) -> Result<Self> {
        Self::protected_with_profile(key, compression, Encryption::Robust)
    }

    /// Create an authenticated codec from a client root key.
    pub fn protected_with_profile(
        key: &DataProtectionKey,
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        if encryption == Encryption::Unprotected {
            return Err(Error::InvalidEncryptionConfiguration);
        }
        Self::with_keyring(ValueKeyring::from_root_key(key)?, compression, encryption)
    }

    /// Create an authenticated codec from exact key bytes.
    pub fn encrypted(key: [u8; ENCRYPTION_KEY_BYTES], compression: Compression) -> Result<Self> {
        Self::encrypted_with_profile(key, compression, Encryption::Robust)
    }

    /// Create an authenticated codec from exact key bytes and profile.
    pub fn encrypted_with_profile(
        key: [u8; ENCRYPTION_KEY_BYTES],
        compression: Compression,
        encryption: Encryption,
    ) -> Result<Self> {
        let ring = ValueKeyring::single(1, key)?;
        Self::with_keyring(ring, compression, encryption)
    }

    /// Create a codec with an explicit immutable keyring.
    pub fn with_keyring(
        keyring: ValueKeyring,
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
            keyring: Some(keyring),
            read_profiles: 1 << encryption.selector_id(),
            limits: ValueLimits::default(),
            budget: None,
            default_budget: OnceLock::new(),
        })
    }

    /// Allow an additional protection profile for reads without changing the
    /// write selector or active key.
    pub fn allow_read_profile(mut self, encryption: Encryption) -> Self {
        self.read_profiles |= 1 << encryption.selector_id();
        self
    }

    /// Override all local resource limits.
    pub fn with_limits(mut self, limits: ValueLimits) -> Result<Self> {
        validate_limits(limits)?;
        self.limits = limits;
        Ok(self)
    }

    /// Applies one aggregate byte budget to this codec's bounded work.
    ///
    /// The same budget may be shared with a transport client so network
    /// response bodies remain accounted while a value is authenticated,
    /// decompressed, or parsed.
    pub fn with_budget(mut self, budget: RequestBudget) -> Self {
        self.budget = Some(budget);
        self.default_budget = OnceLock::new();
        self
    }

    /// Borrow the configured limits.
    pub const fn limits(&self) -> ValueLimits {
        self.limits
    }

    /// Encode a compatibility logical value.
    pub fn encode(&self, item_id: ItemId, value: Value) -> Result<ItemValue> {
        self.encode_in_namespace(1, item_id, value)
    }

    /// Encode a compatibility logical value bound to a namespace and Item ID.
    pub fn encode_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: Value,
    ) -> Result<ItemValue> {
        match value {
            Value::Raw(bytes) => self.seal_opaque_in_namespace(namespace_id, item_id, &bytes),
            Value::Json(value) => {
                validate_json_limits(&value, self.limits, self.budget())?;
                let (structured, _json_permits) =
                    json_to_structured(&value, self.limits, self.budget())?;
                self.seal_structured_in_namespace(namespace_id, item_id, &structured)
            }
        }
    }

    /// Encode one canonical JSON document as an `OpaqueBytes` payload.
    ///
    /// JSON helpers are a convenience representation, not a value-format
    /// selector. The complete canonical UTF-8 document is stored unchanged
    /// as the payload of selector `0`; structured-value callers must use the
    /// dedicated [`Self::seal_structured_in_namespace`] API.
    pub fn seal_json_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        mut value: JsonValue,
    ) -> Result<ItemValue> {
        validate_json_limits(&value, self.limits, self.budget())?;
        let (payload, permits) =
            encode_json_output_with_budget(&mut value, self.limits, self.budget())?;
        let result = self.encode_payload(
            namespace_id,
            item_id.as_bytes(),
            PAYLOAD_OPAQUE_BYTES,
            &payload,
        );
        drop(permits);
        result
    }

    /// Encode canonical JSON using the compatibility namespace `1`.
    pub fn seal_json(&self, item_id: ItemId, value: JsonValue) -> Result<ItemValue> {
        self.seal_json_in_namespace(1, item_id, value)
    }

    /// Decode and validate one canonical JSON document stored as
    /// `OpaqueBytes`.
    ///
    /// A raw value is not silently reinterpreted: the payload must be valid
    /// JSON and its bytes must already be the canonical UTF-8 spelling emitted
    /// by the shared helper.
    pub fn open_json_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<JsonValue> {
        let decoded = self.decode_payload(namespace_id, item_id.as_bytes(), encoded)?;
        if decoded.format != PAYLOAD_OPAQUE_BYTES {
            return Err(Error::ExpectedOpaqueBytes);
        }
        let (mut value, parse_permits) =
            parse_json_input_with_budget(&decoded.payload, self.limits, self.budget())?;
        let (canonical, canonical_permits) =
            encode_json_output_with_budget(&mut value, self.limits, self.budget())?;
        let result = if canonical == decoded.payload {
            Ok(value)
        } else {
            Err(Error::NonCanonicalJson)
        };
        drop(canonical_permits);
        drop(parse_permits);
        result
    }

    /// Decode canonical JSON using the compatibility namespace `1`.
    pub fn open_json(&self, item_id: ItemId, encoded: ItemValue) -> Result<JsonValue> {
        self.open_json_in_namespace(1, item_id, encoded)
    }

    /// Encode one exact OpaqueBytes payload.
    pub fn seal(&self, item_id: ItemId, plaintext: &[u8]) -> Result<ItemValue> {
        self.seal_opaque_in_namespace(1, item_id, plaintext)
    }

    /// Encode exact OpaqueBytes while binding namespace and Item ID.
    pub fn seal_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        plaintext: &[u8],
    ) -> Result<ItemValue> {
        self.seal_opaque_in_namespace(namespace_id, item_id, plaintext)
    }

    /// Encode owned exact OpaqueBytes.
    pub fn seal_owned(&self, item_id: ItemId, plaintext: Vec<u8>) -> Result<ItemValue> {
        self.seal_owned_in_namespace(1, item_id, plaintext)
    }

    /// Encode owned exact OpaqueBytes while binding namespace and Item ID.
    pub fn seal_owned_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        plaintext: Vec<u8>,
    ) -> Result<ItemValue> {
        self.seal_opaque_in_namespace(namespace_id, item_id, &plaintext)
    }

    /// Encode OpaqueBytes without a format-specific interpretation.
    pub fn seal_opaque(&self, item_id: ItemId, payload: &[u8]) -> Result<ItemValue> {
        self.seal_opaque_in_namespace(1, item_id, payload)
    }

    /// Encode OpaqueBytes bound to a namespace and Item ID.
    pub fn seal_opaque_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        payload: &[u8],
    ) -> Result<ItemValue> {
        self.encode_payload(
            namespace_id,
            item_id.as_bytes(),
            PAYLOAD_OPAQUE_BYTES,
            payload,
        )
    }

    /// Encode OpaqueBytes for an exact wire Item ID byte slice.
    ///
    /// The low-level protocol permits the complete `0..=32` byte Item ID
    /// range, including an empty identity. Mapped high-level APIs continue to
    /// use the fixed-size [`ItemId`] type.
    pub fn seal_opaque_with_item_id_bytes(
        &self,
        namespace_id: u64,
        item_id: &[u8],
        payload: &[u8],
    ) -> Result<ItemValue> {
        self.encode_payload(namespace_id, item_id, PAYLOAD_OPAQUE_BYTES, payload)
    }

    /// Encode one StructuredValue-CBOR-v1 payload.
    pub fn seal_structured(&self, item_id: ItemId, value: &StructuredValue) -> Result<ItemValue> {
        self.seal_structured_in_namespace(1, item_id, value)
    }

    /// Encode StructuredValue-CBOR-v1 bound to a namespace and Item ID.
    pub fn seal_structured_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: &StructuredValue,
    ) -> Result<ItemValue> {
        let (payload, payload_permit) = self.encode_structured_payload(value)?;
        self.encode_payload_owned(
            namespace_id,
            item_id.as_bytes(),
            PAYLOAD_STRUCTURED_CBOR_V1,
            payload,
            payload_permit,
        )
    }

    /// Encode StructuredValue-CBOR-v1 for an exact wire Item ID byte slice.
    pub fn seal_structured_with_item_id_bytes(
        &self,
        namespace_id: u64,
        item_id: &[u8],
        value: &StructuredValue,
    ) -> Result<ItemValue> {
        let (payload, payload_permit) = self.encode_structured_payload(value)?;
        self.encode_payload_owned(
            namespace_id,
            item_id,
            PAYLOAD_STRUCTURED_CBOR_V1,
            payload,
            payload_permit,
        )
    }

    /// Decode a compatibility logical value.
    pub fn decode(&self, item_id: ItemId, encoded: ItemValue) -> Result<Value> {
        self.decode_in_namespace(1, item_id, encoded)
    }

    /// Decode a compatibility logical value bound to a namespace and Item ID.
    pub fn decode_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<Value> {
        let decoded = self.decode_payload(namespace_id, item_id.as_bytes(), encoded)?;
        match decoded.format {
            PAYLOAD_OPAQUE_BYTES => Ok(Value::Raw(decoded.payload)),
            PAYLOAD_STRUCTURED_CBOR_V1 => {
                let (structured, _structured_permits) =
                    self.decode_structured_payload(&decoded.payload)?;
                let (json, _json_permits) =
                    structured_to_json(&structured, self.limits, self.budget())?;
                json.map(Value::Json)
                    .ok_or_else(|| Error::UnsupportedStructuredValue)
            }
            _ => Err(Error::UnsupportedPayloadFormat(decoded.format)),
        }
    }

    /// Decode an exact OpaqueBytes payload.
    pub fn open(&self, item_id: ItemId, encoded: ItemValue) -> Result<Vec<u8>> {
        self.open_opaque_in_namespace(1, item_id, encoded)
    }

    /// Decode exact OpaqueBytes bound to a namespace and Item ID.
    pub fn open_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<Vec<u8>> {
        self.open_opaque_in_namespace(namespace_id, item_id, encoded)
    }

    /// Decode exact OpaqueBytes without applying another payload format.
    pub fn open_opaque(&self, item_id: ItemId, encoded: ItemValue) -> Result<Vec<u8>> {
        self.open_opaque_in_namespace(1, item_id, encoded)
    }

    /// Decode exact OpaqueBytes bound to a namespace and Item ID.
    pub fn open_opaque_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<Vec<u8>> {
        let decoded = self.decode_payload(namespace_id, item_id.as_bytes(), encoded)?;
        if decoded.format != PAYLOAD_OPAQUE_BYTES {
            return Err(Error::ExpectedOpaqueBytes);
        }
        Ok(decoded.payload)
    }

    /// Decode OpaqueBytes for an exact wire Item ID byte slice.
    pub fn open_opaque_with_item_id_bytes(
        &self,
        namespace_id: u64,
        item_id: &[u8],
        encoded: ItemValue,
    ) -> Result<Vec<u8>> {
        let decoded = self.decode_payload(namespace_id, item_id, encoded)?;
        if decoded.format != PAYLOAD_OPAQUE_BYTES {
            return Err(Error::ExpectedOpaqueBytes);
        }
        Ok(decoded.payload)
    }

    /// Decode one StructuredValue-CBOR-v1 payload.
    pub fn open_structured(&self, item_id: ItemId, encoded: ItemValue) -> Result<StructuredValue> {
        self.open_structured_in_namespace(1, item_id, encoded)
    }

    /// Decode StructuredValue-CBOR-v1 bound to a namespace and Item ID.
    pub fn open_structured_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        encoded: ItemValue,
    ) -> Result<StructuredValue> {
        let decoded = self.decode_payload(namespace_id, item_id.as_bytes(), encoded)?;
        if decoded.format != PAYLOAD_STRUCTURED_CBOR_V1 {
            return Err(Error::ExpectedStructuredValue);
        }
        let (structured, _structured_permits) = self.decode_structured_payload(&decoded.payload)?;
        Ok(structured)
    }

    /// Decode StructuredValue-CBOR-v1 for an exact wire Item ID byte slice.
    pub fn open_structured_with_item_id_bytes(
        &self,
        namespace_id: u64,
        item_id: &[u8],
        encoded: ItemValue,
    ) -> Result<StructuredValue> {
        let decoded = self.decode_payload(namespace_id, item_id, encoded)?;
        if decoded.format != PAYLOAD_STRUCTURED_CBOR_V1 {
            return Err(Error::ExpectedStructuredValue);
        }
        let (structured, _structured_permits) = self.decode_structured_payload(&decoded.payload)?;
        Ok(structured)
    }

    fn encode_structured_payload(&self, value: &StructuredValue) -> Result<(Vec<u8>, BytePermit)> {
        let mut temporary_permits = Vec::new();
        let mut reserve = |size: usize| {
            let Ok(permit) = self.reserve(size, Resource::StructuredValue) else {
                return false;
            };
            temporary_permits.push(permit);
            true
        };
        let payload = encode_with_limits_and_budget(
            value,
            structured_limits(self.limits),
            self.limits.max_in_flight_bytes,
            &mut reserve,
        )
        .map_err(Error::Structured)?;
        // The callback accounts all temporary encoder work cumulatively. Once
        // encoding has completed only the returned CBOR bytes remain live;
        // retain one exact permit for that output rather than carrying every
        // historical callback reservation into envelope encoding.
        drop(temporary_permits);
        let payload_permit = self.reserve(payload.len(), Resource::StructuredValue)?;
        Ok((payload, payload_permit))
    }

    /// Encode one StructuredValue-CBOR-v1 payload for a language adapter.
    ///
    /// The returned bytes are produced with the same depth, item, integer, and
    /// aggregate-budget limits as envelope encoding. The temporary output
    /// permit is released after the caller receives the owned bytes.
    pub fn encode_structured_cbor(&self, value: &StructuredValue) -> Result<Vec<u8>> {
        let (payload, permit) = self.encode_structured_payload(value)?;
        drop(permit);
        Ok(payload)
    }

    fn decode_structured_payload(
        &self,
        payload: &[u8],
    ) -> Result<(StructuredValue, Vec<BytePermit>)> {
        let mut permits = Vec::new();
        let mut reserve = |size: usize| {
            let Ok(permit) = self.reserve(size, Resource::StructuredValue) else {
                return false;
            };
            permits.push(permit);
            true
        };
        let structured = decode_with_limits_and_budget(
            payload,
            structured_limits(self.limits),
            self.limits.max_in_flight_bytes,
            &mut reserve,
        )
        .map_err(Error::Structured)?;
        Ok((structured, permits))
    }

    /// Decode and seal one complete StructuredValue-CBOR-v1 payload while
    /// retaining parser permits through envelope encoding.
    pub fn seal_structured_cbor_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        payload: &[u8],
    ) -> Result<ItemValue> {
        let (value, permits) = self.decode_structured_payload(payload)?;
        let result = self.seal_structured_in_namespace(namespace_id, item_id, &value);
        drop(permits);
        result
    }

    /// Validate and pass through a caller-owned version-0 envelope.
    ///
    /// Version 0 is deliberately not parsed or transformed. This method only
    /// checks the first canonical `vu128` field and the outer byte limit.
    pub fn pass_through_v0(&self, bytes: Vec<u8>) -> Result<ItemValue> {
        if bytes.len() > self.limits.max_envelope_bytes {
            return Err(Error::ResourceLimit {
                resource: Resource::EnvelopeBytes,
                limit: self.limits.max_envelope_bytes,
                actual: bytes.len(),
            });
        }
        let (version, _) = decode_vu128(&bytes, "value envelope version")?;
        if version != 0 {
            return Err(Error::UnsupportedVersion(u128::from(version)));
        }
        Ok(ItemValue::new(bytes))
    }

    /// Return a caller-owned version-0 envelope unchanged after validating its
    /// canonical leading version field and complete byte limit.
    pub fn open_v0(&self, encoded: ItemValue) -> Result<Vec<u8>> {
        let bytes = encoded.into_bytes();
        if bytes.len() > self.limits.max_envelope_bytes {
            return Err(Error::ResourceLimit {
                resource: Resource::EnvelopeBytes,
                limit: self.limits.max_envelope_bytes,
                actual: bytes.len(),
            });
        }
        let (version, _) = decode_vu128(&bytes, "value envelope version")?;
        if version != 0 {
            return Err(Error::UnsupportedVersion(u128::from(version)));
        }
        Ok(bytes)
    }

    fn encode_payload(
        &self,
        namespace_id: u64,
        item_id: &[u8],
        payload_format: u8,
        payload: &[u8],
    ) -> Result<ItemValue> {
        self.encode_payload_input(
            namespace_id,
            item_id,
            payload_format,
            PayloadInput::Borrowed(payload),
        )
    }

    fn encode_payload_owned(
        &self,
        namespace_id: u64,
        item_id: &[u8],
        payload_format: u8,
        payload: Vec<u8>,
        payload_permit: BytePermit,
    ) -> Result<ItemValue> {
        self.encode_payload_input(
            namespace_id,
            item_id,
            payload_format,
            PayloadInput::Owned {
                bytes: payload,
                permit: payload_permit,
            },
        )
    }

    fn encode_payload_input(
        &self,
        namespace_id: u64,
        item_id: &[u8],
        payload_format: u8,
        input: PayloadInput<'_>,
    ) -> Result<ItemValue> {
        validate_namespace(namespace_id)?;
        validate_item_id(item_id)?;
        let payload_len = match &input {
            PayloadInput::Borrowed(payload) => payload.len(),
            PayloadInput::Owned { bytes, .. } => bytes.len(),
        };
        if payload_len > self.limits.max_expanded_payload_bytes {
            return Err(Error::ResourceLimit {
                resource: Resource::ExpandedPayloadBytes,
                limit: self.limits.max_expanded_payload_bytes,
                actual: payload_len,
            });
        }
        let key_id = if self.encryption == Encryption::Unprotected {
            None
        } else {
            Some(
                self.keyring
                    .as_ref()
                    .and_then(ValueKeyring::active_id)
                    .ok_or(Error::KeyUnavailable(0))?,
            )
        };
        let key_id_bytes = key_id.map(encode_vu128).transpose()?.unwrap_or_default();
        let protection_overhead = match self.encryption {
            Encryption::Unprotected => 0,
            Encryption::Compact => SIV_SYNTHETIC_IV_BYTES,
            Encryption::Robust => GCM_SIV_NONCE_BYTES + AUTH_TAG_BYTES,
        };
        let envelope_prefix = VERSION_BYTES
            .len()
            .checked_add(1)
            .and_then(|length| length.checked_add(key_id_bytes.len()))
            .and_then(|length| length.checked_add(protection_overhead))
            .ok_or(Error::ResourceLimit {
                resource: Resource::EnvelopeBytes,
                limit: self.limits.max_envelope_bytes,
                actual: usize::MAX,
            })?;
        let max_body_bytes = self
            .limits
            .max_envelope_bytes
            .checked_sub(envelope_prefix)
            .ok_or(Error::EncodedValueTooLarge {
                size: envelope_prefix,
                maximum: self.limits.max_envelope_bytes,
            })?;
        let (transformed, compression_id, _transformed_permit) = match input {
            PayloadInput::Borrowed(payload) => compress_if_beneficial(
                payload,
                self.compression,
                &self.limits,
                self.budget(),
                max_body_bytes,
            )?,
            PayloadInput::Owned { bytes, permit } => compress_owned(
                bytes,
                permit,
                self.compression,
                &self.limits,
                self.budget(),
                max_body_bytes,
            )?,
        };
        let selector = make_selector(
            self.encryption.selector_id(),
            compression_id,
            payload_format,
        )?;
        let aad = make_aad(namespace_id, item_id, selector, &key_id_bytes);
        let encoded_length = VERSION_BYTES
            .len()
            .checked_add(envelope_prefix - VERSION_BYTES.len())
            .and_then(|length| length.checked_add(transformed.len()))
            .ok_or(Error::ResourceLimit {
                resource: Resource::EnvelopeBytes,
                limit: self.limits.max_envelope_bytes,
                actual: usize::MAX,
            })?;
        if encoded_length > self.limits.max_envelope_bytes {
            return Err(Error::EncodedValueTooLarge {
                size: encoded_length,
                maximum: self.limits.max_envelope_bytes,
            });
        }
        let _encoded_permit = self.reserve(encoded_length, Resource::EnvelopeBytes)?;
        let body = match self.encryption {
            Encryption::Unprotected => transformed,
            Encryption::Compact => self.encrypt_compact(
                key_id.expect("protected profile has active key"),
                namespace_id,
                item_id,
                &aad,
                transformed,
            )?,
            Encryption::Robust => self.encrypt_robust(
                key_id.expect("protected profile has active key"),
                namespace_id,
                item_id,
                &aad,
                transformed,
            )?,
        };
        let mut encoded = Vec::new();
        encoded
            .try_reserve_exact(encoded_length)
            .map_err(|_| Error::Allocation {
                size: encoded_length,
            })?;
        encoded.extend_from_slice(VERSION_BYTES);
        encoded.push(selector);
        encoded.extend_from_slice(&key_id_bytes);
        encoded.extend_from_slice(&body);
        Ok(ItemValue::new(encoded))
    }

    fn decode_payload(
        &self,
        namespace_id: u64,
        item_id: &[u8],
        encoded: ItemValue,
    ) -> Result<DecodedPayload> {
        validate_namespace(namespace_id)?;
        validate_item_id(item_id)?;
        let (mut encoded, response_permit) = encoded.into_budgeted_parts();
        if encoded.len() > self.limits.max_envelope_bytes {
            return Err(Error::EncodedValueTooLarge {
                size: encoded.len(),
                maximum: self.limits.max_envelope_bytes,
            });
        }
        if encoded.len() > self.limits.max_in_flight_bytes {
            return Err(Error::ResourceLimit {
                resource: Resource::EnvelopeBytes,
                limit: self.limits.max_in_flight_bytes,
                actual: encoded.len(),
            });
        }
        let envelope_permit = if response_permit.is_some() {
            None
        } else {
            Some(self.reserve(encoded.len(), Resource::EnvelopeBytes)?)
        };
        let (version, version_length) = decode_vu128(&encoded, "value envelope version")?;
        if version == 0 {
            return Err(Error::CallerOwnedV0Required);
        }
        if u128::from(version) != VERSION {
            return Err(Error::UnsupportedVersion(u128::from(version)));
        }
        let selector = *encoded
            .get(version_length)
            .ok_or(Error::TruncatedEnvelope)?;
        let (protection, compression, format) = parse_selector(selector)?;
        if self.read_profiles & (1 << protection.selector_id()) == 0 {
            return Err(match (self.encryption, protection) {
                (Encryption::Unprotected, _) => Error::KeyUnavailable(0),
                (_, Encryption::Unprotected) => Error::ProtectionRequired,
                _ => Error::ProtectionProfileMismatch {
                    expected: self.encryption,
                    actual: protection,
                },
            });
        }
        let mut offset = version_length + 1;
        let key_id_bytes = if protection == Encryption::Unprotected {
            &[][..]
        } else {
            let (_, length) = decode_vu128(
                encoded.get(offset..).ok_or(Error::TruncatedEnvelope)?,
                "value key ID",
            )?;
            let bytes = encoded
                .get(offset..offset + length)
                .ok_or(Error::TruncatedEnvelope)?;
            offset += length;
            let (key_id, _) = decode_vu128(bytes, "value key ID")?;
            if key_id == 0 {
                return Err(Error::InvalidValueKeyId(key_id));
            }
            bytes
        };
        let key_id = if key_id_bytes.is_empty() {
            None
        } else {
            Some(decode_vu128(key_id_bytes, "value key ID")?.0)
        };
        if let Some(key_id) = key_id {
            // Resolve the exact advertised key before inspecting protected-body
            // details. Unknown or retired IDs have one distinct outcome and
            // never fall through to authentication or decompression errors.
            self.key(key_id)?;
        }
        let aad = make_aad(namespace_id, item_id, selector, key_id_bytes);
        if offset > encoded.len() {
            return Err(Error::TruncatedEnvelope);
        }
        encoded.drain(..offset);
        let body = encoded;
        let transformed = match protection {
            Encryption::Unprotected => body,
            Encryption::Compact => {
                if body.len() < SIV_SYNTHETIC_IV_BYTES {
                    return Err(Error::TruncatedProtectedBody);
                }
                self.decrypt_compact(
                    key_id.ok_or(Error::KeyUnavailable(0))?,
                    namespace_id,
                    item_id,
                    &aad,
                    body,
                )?
            }
            Encryption::Robust => {
                if body.len() < GCM_SIV_NONCE_BYTES + AUTH_TAG_BYTES {
                    return Err(Error::TruncatedProtectedBody);
                }
                self.decrypt_robust(
                    key_id.ok_or(Error::KeyUnavailable(0))?,
                    namespace_id,
                    item_id,
                    &aad,
                    body,
                )?
            }
        };
        let (payload, payload_permit) = if compression == COMPRESSION_ZSTANDARD {
            let (payload, permit) =
                decompress_zstandard(&transformed, &self.limits, self.budget())?;
            (payload, Some(permit))
        } else {
            (transformed, None)
        };
        if payload.len() > self.limits.max_expanded_payload_bytes {
            return Err(Error::ResourceLimit {
                resource: Resource::ExpandedPayloadBytes,
                limit: self.limits.max_expanded_payload_bytes,
                actual: payload.len(),
            });
        }
        Ok(DecodedPayload {
            format,
            payload,
            _response_permit: response_permit,
            _envelope_permit: envelope_permit,
            _payload_permit: payload_permit,
        })
    }

    fn key(&self, key_id: u64) -> Result<&[u8; VALUE_KEY_BYTES]> {
        self.keyring
            .as_ref()
            .and_then(|ring| ring.get(key_id))
            .ok_or(Error::KeyUnavailable(key_id))
    }

    fn encrypt_compact(
        &self,
        key_id: u64,
        namespace_id: u64,
        item_id: &[u8],
        aad: &[u8],
        mut plaintext: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let material = item_material(self.key(key_id)?, key_id, namespace_id, item_id);
        let mac_key = Zeroizing::new(blake3::derive_key(SIV_MAC_CONTEXT, &material));
        let encryption_key = Zeroizing::new(blake3::derive_key(SIV_ENCRYPTION_CONTEXT, &material));
        let mut combined = Zeroizing::new([0_u8; VALUE_KEY_BYTES * 2]);
        combined[..VALUE_KEY_BYTES].copy_from_slice(&mac_key[..]);
        combined[VALUE_KEY_BYTES..].copy_from_slice(&encryption_key[..]);
        Aes256Siv::new((&*combined).into())
            .encrypt_in_place([aad], &mut plaintext)
            .map_err(|_| Error::Encryption)?;
        Ok(plaintext)
    }

    fn decrypt_compact(
        &self,
        key_id: u64,
        namespace_id: u64,
        item_id: &[u8],
        aad: &[u8],
        mut ciphertext: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let material = item_material(self.key(key_id)?, key_id, namespace_id, item_id);
        let mac_key = Zeroizing::new(blake3::derive_key(SIV_MAC_CONTEXT, &material));
        let encryption_key = Zeroizing::new(blake3::derive_key(SIV_ENCRYPTION_CONTEXT, &material));
        let mut combined = Zeroizing::new([0_u8; VALUE_KEY_BYTES * 2]);
        combined[..VALUE_KEY_BYTES].copy_from_slice(&mac_key[..]);
        combined[VALUE_KEY_BYTES..].copy_from_slice(&encryption_key[..]);
        if Aes256Siv::new((&*combined).into())
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
        key_id: u64,
        namespace_id: u64,
        item_id: &[u8],
        aad: &[u8],
        mut plaintext: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let material = item_material(self.key(key_id)?, key_id, namespace_id, item_id);
        let key = Zeroizing::new(blake3::derive_key(GCM_SIV_CONTEXT, &material));
        let cipher = Aes256GcmSiv::new((&*key).into());
        let mut nonce_bytes = [0_u8; GCM_SIV_NONCE_BYTES];
        getrandom::fill(&mut nonce_bytes).map_err(|error| Error::Entropy(error.to_string()))?;
        let nonce = Nonce::from(nonce_bytes);
        let plaintext_length = plaintext.len();
        let body_length = GCM_SIV_NONCE_BYTES
            .checked_add(plaintext_length)
            .and_then(|length| length.checked_add(AUTH_TAG_BYTES))
            .ok_or(Error::ResourceLimit {
                resource: Resource::EnvelopeBytes,
                limit: self.limits.max_envelope_bytes,
                actual: usize::MAX,
            })?;
        plaintext.resize(body_length, 0);
        plaintext.copy_within(0..plaintext_length, GCM_SIV_NONCE_BYTES);
        let tag = cipher
            .encrypt_inout_detached(
                &nonce,
                aad,
                plaintext[GCM_SIV_NONCE_BYTES..GCM_SIV_NONCE_BYTES + plaintext_length]
                    .as_mut()
                    .into(),
            )
            .map_err(|_| Error::Encryption)?;
        plaintext[..GCM_SIV_NONCE_BYTES].copy_from_slice(&nonce_bytes);
        plaintext[GCM_SIV_NONCE_BYTES + plaintext_length..].copy_from_slice(&tag);
        Ok(plaintext)
    }

    fn decrypt_robust(
        &self,
        key_id: u64,
        namespace_id: u64,
        item_id: &[u8],
        aad: &[u8],
        mut body: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let tag_offset = body.len() - AUTH_TAG_BYTES;
        let nonce_bytes: [u8; GCM_SIV_NONCE_BYTES] = body
            .get(..GCM_SIV_NONCE_BYTES)
            .ok_or(Error::TruncatedProtectedBody)?
            .try_into()
            .map_err(|_| Error::TruncatedProtectedBody)?;
        let tag_bytes: [u8; AUTH_TAG_BYTES] = body
            .get(tag_offset..)
            .ok_or(Error::TruncatedProtectedBody)?
            .try_into()
            .map_err(|_| Error::TruncatedProtectedBody)?;
        let ciphertext_length = tag_offset
            .checked_sub(GCM_SIV_NONCE_BYTES)
            .ok_or(Error::TruncatedProtectedBody)?;
        body.copy_within(GCM_SIV_NONCE_BYTES..tag_offset, 0);
        body.truncate(ciphertext_length);
        let material = item_material(self.key(key_id)?, key_id, namespace_id, item_id);
        let key = Zeroizing::new(blake3::derive_key(GCM_SIV_CONTEXT, &material));
        let cipher = Aes256GcmSiv::new((&*key).into());
        let nonce = Nonce::from(nonce_bytes);
        let tag = Tag::from(tag_bytes);
        if cipher
            .decrypt_inout_detached(&nonce, aad, body.as_mut_slice().into(), &tag)
            .is_err()
        {
            body.zeroize();
            return Err(Error::Authentication);
        }
        Ok(body)
    }

    fn reserve(&self, size: usize, resource: Resource) -> Result<BytePermit> {
        reserve_budget(self.budget(), size, &self.limits, resource)
    }

    pub(crate) fn budget(&self) -> &RequestBudget {
        self.budget.as_ref().unwrap_or_else(|| {
            self.default_budget
                .get_or_init(|| RequestBudget::new(MAX_VALUE_ENVELOPE_BYTES))
        })
    }
}

pub(crate) fn reserve_budget(
    budget: &RequestBudget,
    size: usize,
    limits: &ValueLimits,
    resource: Resource,
) -> Result<BytePermit> {
    if size > limits.max_in_flight_bytes {
        return Err(Error::ResourceLimit {
            resource,
            limit: limits.max_in_flight_bytes,
            actual: size,
        });
    }
    budget.try_reserve(size).map_err(|_| Error::ResourceLimit {
        resource,
        limit: budget.capacity(),
        actual: size,
    })
}

struct DecodedPayload {
    format: u8,
    payload: Vec<u8>,
    _response_permit: Option<Arc<BytePermit>>,
    _envelope_permit: Option<BytePermit>,
    _payload_permit: Option<BytePermit>,
}

enum PayloadInput<'a> {
    Borrowed(&'a [u8]),
    Owned { bytes: Vec<u8>, permit: BytePermit },
}

/// Resource dimensions exposed by value errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Resource {
    /// Complete stored envelope.
    EnvelopeBytes,
    /// Expanded plaintext payload.
    ExpandedPayloadBytes,
    /// Zstandard declared window.
    ZstdWindowBytes,
    /// Structured-value model depth/items/integers.
    StructuredValue,
}

/// Client-side value-format errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Invalid compression level.
    #[error("Zstandard level {0} is outside the supported range")]
    InvalidCompressionLevel(i32),
    /// Namespace zero is reserved.
    #[error("namespace ID must be a positive server-assigned identity")]
    InvalidNamespace,
    /// Item IDs are bounded by the wire protocol's 32-byte identity field.
    #[error("Item ID contains {0} bytes; maximum is {ITEM_ID_BYTES}")]
    InvalidItemIdLength(usize),
    /// A protected constructor was given an unprotected profile.
    #[error("protected value codecs require Compact or Robust encryption")]
    InvalidEncryptionConfiguration,
    /// A key ID was zero or outside the unsigned 64-bit profile.
    #[error("invalid value key ID {0}")]
    InvalidValueKeyId(u64),
    /// A key was all zero.
    #[error("value keys must not be all zero")]
    InvalidValueKey,
    /// A key ID was rebound to different key material.
    #[error("value key ID {0} was already assigned different key material")]
    ValueKeyIdRebound(u64),
    /// The exact key ID in a protected value is not available.
    #[error("value key unavailable: {0}")]
    KeyUnavailable(u64),
    /// Authentication failed; details are intentionally generic.
    #[error("value authentication failed")]
    Authentication,
    /// Encryption failed before an envelope was emitted.
    #[error("value encryption failed")]
    Encryption,
    /// OS randomness failed.
    #[error("operating-system entropy failed: {0}")]
    Entropy(String),
    /// Stored data requires a protected profile.
    #[error("client policy requires protected values")]
    ProtectionRequired,
    /// Legacy spelling retained for adapters during selector migration.
    #[error("client policy requires encrypted values")]
    EncryptionRequired,
    /// Legacy spelling retained for adapters during selector migration.
    #[error("encrypted value requires a data protection key")]
    EncryptionKeyRequired,
    /// Stored data used another allowed profile.
    #[error("value protection profile mismatch: expected {expected:?}, got {actual:?}")]
    ProtectionProfileMismatch {
        /// Configured profile.
        expected: Encryption,
        /// Stored profile.
        actual: Encryption,
    },
    /// Legacy spelling retained for adapters during selector migration.
    #[error("value encryption profile mismatch: expected {expected:?}, got {actual:?}")]
    EncryptionProfileMismatch {
        /// Configured profile.
        expected: Encryption,
        /// Stored profile.
        actual: Encryption,
    },
    /// Envelope version is unknown.
    #[error("unsupported value-format version {0}")]
    UnsupportedVersion(u128),
    /// Version-0 data requires an explicit caller-owned path.
    #[error("caller-owned version-0 value requires the explicit pass-through API")]
    CallerOwnedV0Required,
    /// The version or selector field ended early.
    #[error("value envelope is truncated")]
    TruncatedEnvelope,
    /// Legacy structural error spelling retained for adapters.
    #[error("invalid encoded value: {0}")]
    InvalidEncodedValue(&'static str),
    /// Protected body ended before its minimum profile overhead.
    #[error("protected value body is truncated")]
    TruncatedProtectedBody,
    /// Selector has reserved bits or unknown assignments.
    #[error("unsupported value selector {0:#04x}")]
    UnsupportedSelector(u8),
    /// Compression profile is unknown.
    #[error("unsupported value compression identifier {0}")]
    UnsupportedCompression(u8),
    /// Legacy selector spelling retained for adapters.
    #[error("unsupported value encryption identifier {0}")]
    UnsupportedEncryption(u8),
    /// Payload profile is unknown.
    #[error("unsupported value payload format identifier {0}")]
    UnsupportedPayloadFormat(u8),
    /// Legacy payload discriminator spelling retained for adapters.
    #[error("unsupported value serialization identifier {0}")]
    UnsupportedSerialization(u128),
    /// Expected OpaqueBytes but received a structured payload.
    #[error("formatted value is not OpaqueBytes")]
    ExpectedOpaqueBytes,
    /// Expected StructuredValue-CBOR-v1 but received opaque bytes.
    #[error("formatted value is not StructuredValue-CBOR-v1")]
    ExpectedStructuredValue,
    /// Compatibility JSON conversion cannot represent the complete model.
    #[error("structured value cannot be represented as the compatibility JSON view")]
    UnsupportedStructuredValue,
    /// JSON compatibility conversion failed.
    #[error("invalid canonical JSON: {0}")]
    InvalidJson(String),
    /// Legacy canonical JSON mismatch category.
    #[error("JSON payload is not canonical RFC 8785 JSON")]
    NonCanonicalJson,
    /// StructuredValue-CBOR-v1 validation failed.
    #[error("structured value failed validation: {0}")]
    Structured(#[source] openkache_value::Error),
    /// Resource limit was exceeded before allocation or traversal.
    #[error("{resource:?} limit {limit} exceeded by {actual}")]
    ResourceLimit {
        /// Bounded resource.
        resource: Resource,
        /// Configured maximum.
        limit: usize,
        /// Observed amount.
        actual: usize,
    },
    /// Complete encoded bytes exceeded the configured envelope limit.
    ///
    /// This spelling remains available to callers migrating from the
    /// pre-selector codec. New code should prefer [`Error::ResourceLimit`]
    /// with [`Resource::EnvelopeBytes`].
    #[error("encoded value is too large: {size} bytes exceeds {maximum}")]
    EncodedValueTooLarge {
        /// Actual encoded size.
        size: usize,
        /// Maximum accepted encoded size.
        maximum: usize,
    },
    /// Legacy expanded-payload limit category.
    #[error("decoded value is too large: {size} bytes exceeds {maximum}")]
    DecodedValueTooLarge {
        /// Actual expanded bytes.
        size: usize,
        /// Configured expanded limit.
        maximum: usize,
    },
    /// Allocation failed.
    #[error("failed to allocate {size} bytes")]
    Allocation {
        /// Requested size.
        size: usize,
    },
    /// Malformed vu128.
    #[error("invalid {field}: {reason}")]
    InvalidVu128 {
        /// Stable field name.
        field: &'static str,
        /// Stable validation detail.
        reason: &'static str,
    },
    /// Zstandard framing or decoding failed.
    #[error("Zstandard {operation} failed: {message}")]
    Zstandard {
        /// Operation name.
        operation: &'static str,
        /// Codec diagnostic.
        message: String,
    },
    /// Zstandard output did not match its declared content size.
    #[error("decompressed length mismatch: expected {expected}, got {actual}")]
    DecompressedLength {
        /// Declared content size.
        expected: usize,
        /// Produced bytes.
        actual: usize,
    },
    /// Kept for callers migrating from the legacy JSON/raw codec.
    #[error("formatted value is not Raw serialization")]
    ExpectedRawValue,
}

/// Convenience result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Parse one complete JSON input for compatibility adapters.
#[allow(dead_code)]
pub(crate) fn parse_json_input(payload: &[u8]) -> Result<JsonValue> {
    let budget = RequestBudget::new(MAX_VALUE_ENVELOPE_BYTES);
    let (value, _permits) = parse_json_input_with_budget(payload, ValueLimits::default(), &budget)?;
    Ok(value)
}

/// Parse one complete JSON input while charging parser allocations to the
/// caller's aggregate request budget. Temporary parser permits are released
/// when this regression helper returns.
#[doc(hidden)]
pub fn parse_json_input_for_test(
    payload: &[u8],
    limits: ValueLimits,
    budget: &RequestBudget,
) -> Result<JsonValue> {
    let (value, _permits) = parse_json_input_with_budget(payload, limits, budget)?;
    Ok(value)
}

/// Parse one complete JSON input while retaining permits for the caller's
/// subsequent bounded conversion.
pub fn parse_json_input_with_budget(
    payload: &[u8],
    limits: ValueLimits,
    budget: &RequestBudget,
) -> Result<(JsonValue, Vec<BytePermit>)> {
    if payload.len() > limits.max_expanded_payload_bytes {
        return Err(Error::ResourceLimit {
            resource: Resource::ExpandedPayloadBytes,
            limit: limits.max_expanded_payload_bytes,
            actual: payload.len(),
        });
    }
    validate_json_integer_tokens(payload)?;
    let mut deserializer = serde_json::Deserializer::from_slice(payload);
    let mut state = JsonParseState::new(budget, limits);
    let value = JsonBudgetSeed {
        state: &mut state,
        depth: 0,
    }
    .deserialize(&mut deserializer)
    .map_err(|error| state.finish_error(error))?;
    deserializer
        .end()
        .map_err(|error| state.finish_error(error))?;
    Ok((value, state.permits))
}

/// Canonically serializes one logical JSON value while charging output
/// backing storage and emitted bytes to the caller's aggregate budget.
///
/// The returned permits must remain live while the output buffer is exposed to
/// another API boundary. Dropping them releases every reservation made by the
/// writer.
pub fn encode_json_output_with_budget(
    value: &mut JsonValue,
    limits: ValueLimits,
    budget: &RequestBudget,
) -> Result<(Vec<u8>, Vec<BytePermit>)> {
    fn write_value(
        value: &mut JsonValue,
        depth: usize,
        limits: ValueLimits,
        writer: &mut JsonBudgetWriter<'_>,
    ) -> Result<()> {
        if depth > limits.max_depth {
            return Err(structured_depth(limits.max_depth, depth));
        }
        match value {
            JsonValue::Null
            | JsonValue::Boolean(_)
            | JsonValue::Number(_)
            | JsonValue::String(_) => {
                serde_json_canonicalizer::to_writer(value, writer).map_err(|error| {
                    writer
                        .failure
                        .take()
                        .unwrap_or_else(|| Error::InvalidJson(error.to_string()))
                })?;
            }
            JsonValue::Array(values) => {
                if depth >= limits.max_depth {
                    return Err(structured_depth(limits.max_depth, depth + 1));
                }
                writer
                    .write_all(b"[")
                    .map_err(|error| writer.io_error(error))?;
                for (index, value) in values.iter_mut().enumerate() {
                    if index != 0 {
                        writer
                            .write_all(b",")
                            .map_err(|error| writer.io_error(error))?;
                    }
                    write_value(value, depth + 1, limits, writer)?;
                }
                writer
                    .write_all(b"]")
                    .map_err(|error| writer.io_error(error))?;
            }
            JsonValue::Object(entries) => {
                if depth >= limits.max_depth {
                    return Err(structured_depth(limits.max_depth, depth + 1));
                }
                entries.sort_unstable_by(|(left, _), (right, _)| {
                    left.encode_utf16().cmp(right.encode_utf16())
                });
                writer
                    .write_all(b"{")
                    .map_err(|error| writer.io_error(error))?;
                for (index, (key, value)) in entries.iter_mut().enumerate() {
                    if index != 0 {
                        writer
                            .write_all(b",")
                            .map_err(|error| writer.io_error(error))?;
                    }
                    serde_json_canonicalizer::to_writer(key, writer)
                        .map_err(|error| writer.io_error(io::Error::other(error)))?;
                    writer
                        .write_all(b":")
                        .map_err(|error| writer.io_error(error))?;
                    write_value(value, depth + 1, limits, writer)?;
                }
                writer
                    .write_all(b"}")
                    .map_err(|error| writer.io_error(error))?;
            }
        }
        Ok(())
    }

    struct JsonBudgetWriter<'a> {
        budget: &'a RequestBudget,
        limits: ValueLimits,
        payload: Vec<u8>,
        permits: Vec<BytePermit>,
        failure: Option<Error>,
    }

    impl JsonBudgetWriter<'_> {
        fn io_error(&mut self, error: io::Error) -> Error {
            self.failure
                .take()
                .unwrap_or_else(|| Error::InvalidJson(error.to_string()))
        }
    }

    impl Write for JsonBudgetWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let next = self
                .written()
                .checked_add(bytes.len())
                .ok_or_else(|| io::Error::other("canonical JSON output length overflow"))?;
            if next > self.limits.max_expanded_payload_bytes {
                let error = Error::ResourceLimit {
                    resource: Resource::ExpandedPayloadBytes,
                    limit: self.limits.max_expanded_payload_bytes,
                    actual: next,
                };
                if self.failure.is_none() {
                    self.failure = Some(error);
                }
                return Err(io::Error::other("canonical JSON output exceeds its limit"));
            }
            let permit = reserve_budget(
                self.budget,
                bytes.len(),
                &self.limits,
                Resource::ExpandedPayloadBytes,
            )
            .map_err(|error| {
                if self.failure.is_none() {
                    self.failure = Some(error);
                }
                io::Error::other("canonical JSON output exceeds its budget")
            })?;
            self.payload.try_reserve_exact(bytes.len()).map_err(|_| {
                if self.failure.is_none() {
                    self.failure = Some(Error::Allocation { size: bytes.len() });
                }
                io::Error::other("failed to allocate canonical JSON output")
            })?;
            self.payload.extend_from_slice(bytes);
            self.permits.push(permit);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl JsonBudgetWriter<'_> {
        fn written(&self) -> usize {
            self.payload.len()
        }
    }

    let mut writer = JsonBudgetWriter {
        budget,
        limits,
        payload: Vec::new(),
        permits: Vec::new(),
        failure: None,
    };
    if let Err(error) = write_value(value, 0, limits, &mut writer) {
        return Err(writer.failure.take().unwrap_or(error));
    }
    Ok((writer.payload, writer.permits))
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
    if !same_integer_value(token, &format!("{value:.0}")) {
        return Err(Error::InvalidJson(
            "JSON integers must be exactly representable as IEEE-754 binary64 values".into(),
        ));
    }
    Ok(())
}

fn same_integer_value(input: &str, rendered: &str) -> bool {
    let (input_negative, input_digits) = input
        .strip_prefix('-')
        .map_or((false, input), |digits| (true, digits));
    let (rendered_negative, rendered_digits) = rendered
        .strip_prefix('-')
        .map_or((false, rendered), |digits| (true, digits));
    let input_digits = input_digits.trim_start_matches('0');
    let rendered_digits = rendered_digits.trim_start_matches('0');
    let input_zero = input_digits.is_empty();
    let rendered_zero = rendered_digits.is_empty();
    (input_zero && rendered_zero || input_negative == rendered_negative)
        && if input_zero {
            rendered_zero
        } else {
            input_digits == rendered_digits
        }
}

#[allow(dead_code)]
fn validate_json(value: &JsonValue) -> Result<()> {
    let mut work = vec![value];
    while let Some(value) = work.pop() {
        match value {
            JsonValue::Number(number) if !number.is_finite() => {
                return Err(Error::InvalidJson(
                    "JSON numbers must be finite IEEE-754 values".into(),
                ));
            }
            JsonValue::Array(values) => {
                work.extend(values.iter().rev());
            }
            JsonValue::Object(entries) => {
                validate_json_object(entries)?;
                work.extend(entries.iter().rev().map(|(_, value)| value));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_json_object(entries: &[(String, JsonValue)]) -> Result<()> {
    let mut keys = HashSet::new();
    keys.try_reserve(entries.len())
        .map_err(|_| Error::Allocation {
            size: entries.len(),
        })?;
    for (key, _) in entries {
        if !keys.insert(key) {
            return Err(Error::InvalidJson(
                "JSON object property names must be unique".into(),
            ));
        }
    }
    Ok(())
}

fn validate_json_object_with_budget(
    entries: &[(String, JsonValue)],
    limits: ValueLimits,
    budget: &RequestBudget,
) -> Result<()> {
    let bytes = hash_set_allocation_bytes::<&str>(entries.len())?;
    let _permit = reserve_budget(budget, bytes, &limits, Resource::StructuredValue)?;
    let mut keys = HashSet::new();
    keys.try_reserve(entries.len())
        .map_err(|_| Error::Allocation { size: bytes })?;
    for (key, _) in entries {
        if !keys.insert(key.as_str()) {
            return Err(Error::InvalidJson(
                "JSON object property names must be unique".into(),
            ));
        }
    }
    Ok(())
}

fn validate_json_limits(
    value: &JsonValue,
    limits: ValueLimits,
    budget: &RequestBudget,
) -> Result<()> {
    fn visit(
        value: &JsonValue,
        depth: usize,
        item_count: &mut usize,
        pending_items: &mut usize,
        limits: ValueLimits,
        budget: &RequestBudget,
    ) -> Result<()> {
        *pending_items = pending_items
            .checked_sub(1)
            .expect("the root or a declared child is always pending");
        *item_count = item_count
            .checked_add(1)
            .ok_or_else(|| structured_resource(limits.max_items, usize::MAX))?;
        if *item_count > limits.max_items {
            return Err(structured_resource(limits.max_items, *item_count));
        }
        match value {
            JsonValue::Array(values) => {
                if depth >= limits.max_depth {
                    return Err(structured_depth(limits.max_depth, depth + 1));
                }
                add_json_pending_items(pending_items, values.len(), *item_count, limits.max_items)?;
                for value in values {
                    visit(value, depth + 1, item_count, pending_items, limits, budget)?;
                }
            }
            JsonValue::Object(entries) => {
                if depth >= limits.max_depth {
                    return Err(structured_depth(limits.max_depth, depth + 1));
                }
                validate_json_object_with_budget(entries, limits, budget)?;
                let child_count = entries
                    .len()
                    .checked_mul(2)
                    .ok_or_else(|| structured_resource(limits.max_items, usize::MAX))?;
                add_json_pending_items(pending_items, child_count, *item_count, limits.max_items)?;
                for (_, value) in entries {
                    *pending_items = pending_items
                        .checked_sub(1)
                        .expect("the declared object key is always pending");
                    *item_count = item_count
                        .checked_add(1)
                        .ok_or_else(|| structured_resource(limits.max_items, usize::MAX))?;
                    if *item_count > limits.max_items {
                        return Err(structured_resource(limits.max_items, *item_count));
                    }
                    visit(value, depth + 1, item_count, pending_items, limits, budget)?;
                }
            }
            JsonValue::Number(number) if !number.is_finite() => {
                return Err(Error::InvalidJson(
                    "JSON numbers must be finite IEEE-754 values".into(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    let mut item_count = 0usize;
    let mut pending_items = 1usize;
    visit(
        value,
        0,
        &mut item_count,
        &mut pending_items,
        limits,
        budget,
    )
}

fn add_json_pending_items(
    pending_items: &mut usize,
    child_count: usize,
    item_count: usize,
    maximum: usize,
) -> Result<()> {
    *pending_items = pending_items
        .checked_add(child_count)
        .ok_or_else(|| structured_resource(maximum, usize::MAX))?;
    let minimum_total = item_count
        .checked_add(*pending_items)
        .ok_or_else(|| structured_resource(maximum, usize::MAX))?;
    if minimum_total > maximum {
        return Err(structured_resource(maximum, minimum_total));
    }
    Ok(())
}

struct JsonAllocation<'a> {
    budget: &'a RequestBudget,
    limits: ValueLimits,
    permits: Vec<BytePermit>,
}

impl JsonAllocation<'_> {
    fn reserve(&mut self, size: usize) -> Result<()> {
        self.permits
            .try_reserve(1)
            .map_err(|_| Error::Allocation { size: 1 })?;
        let permit = reserve_budget(self.budget, size, &self.limits, Resource::StructuredValue)?;
        self.permits.push(permit);
        Ok(())
    }

    fn reserve_vec<T>(&mut self, length: usize) -> Result<Vec<T>> {
        let bytes = length
            .checked_mul(size_of::<T>())
            .ok_or_else(|| structured_resource(self.limits.max_in_flight_bytes, usize::MAX))?;
        self.reserve(bytes)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(length)
            .map_err(|_| Error::Allocation { size: bytes })?;
        Ok(values)
    }

    fn clone_string(&mut self, value: &str) -> Result<String> {
        self.reserve(value.len())?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(value.len())
            .map_err(|_| Error::Allocation { size: value.len() })?;
        owned.push_str(value);
        Ok(owned)
    }
}

fn json_to_structured(
    value: &JsonValue,
    limits: ValueLimits,
    budget: &RequestBudget,
) -> Result<(StructuredValue, Vec<BytePermit>)> {
    fn convert(
        value: &JsonValue,
        depth: usize,
        item_count: &mut usize,
        limits: ValueLimits,
        allocation: &mut JsonAllocation<'_>,
    ) -> Result<StructuredValue> {
        *item_count = item_count
            .checked_add(1)
            .ok_or_else(|| structured_resource(limits.max_items, usize::MAX))?;
        if *item_count > limits.max_items {
            return Err(structured_resource(limits.max_items, *item_count));
        }
        Ok(match value {
            JsonValue::Null => StructuredValue::Null,
            JsonValue::Boolean(value) => StructuredValue::Boolean(*value),
            JsonValue::Number(value) => StructuredValue::Float64(value.to_bits()),
            JsonValue::String(value) => {
                StructuredValue::TextString(allocation.clone_string(value)?)
            }
            JsonValue::Array(values) => {
                if depth >= limits.max_depth {
                    return Err(structured_depth(limits.max_depth, depth + 1));
                }
                let mut converted = allocation.reserve_vec(values.len())?;
                for value in values {
                    converted.push(convert(value, depth + 1, item_count, limits, allocation)?);
                }
                StructuredValue::Array(converted)
            }
            JsonValue::Object(entries) => {
                if depth >= limits.max_depth {
                    return Err(structured_depth(limits.max_depth, depth + 1));
                }
                let mut converted = allocation.reserve_vec(entries.len())?;
                for (key, value) in entries {
                    let key = allocation.clone_string(key)?;
                    let value = convert(value, depth + 1, item_count, limits, allocation)?;
                    converted.push((StructuredValue::TextString(key), value));
                }
                StructuredValue::Map(converted)
            }
        })
    }

    let mut allocation = JsonAllocation {
        budget,
        limits,
        permits: Vec::new(),
    };
    let mut item_count = 0usize;
    let value = convert(value, 0, &mut item_count, limits, &mut allocation)?;
    Ok((value, allocation.permits))
}

fn structured_to_json(
    value: &StructuredValue,
    limits: ValueLimits,
    budget: &RequestBudget,
) -> Result<(Option<JsonValue>, Vec<BytePermit>)> {
    fn convert(
        value: &StructuredValue,
        depth: usize,
        limits: ValueLimits,
        allocation: &mut JsonAllocation<'_>,
    ) -> Result<Option<JsonValue>> {
        Ok(match value {
            StructuredValue::Undefined => None,
            StructuredValue::Null => Some(JsonValue::Null),
            StructuredValue::Boolean(value) => Some(JsonValue::Boolean(*value)),
            StructuredValue::Float16(bits) => Some(JsonValue::number(f16_to_f64(*bits))?),
            StructuredValue::Float32(bits) => {
                Some(JsonValue::number(f32::from_bits(*bits) as f64)?)
            }
            StructuredValue::Float64(bits) => Some(JsonValue::number(f64::from_bits(*bits))?),
            StructuredValue::Integer(integer) => {
                Some(JsonValue::number(integer_to_binary64(integer)?)?)
            }
            StructuredValue::TextString(value) => {
                Some(JsonValue::String(allocation.clone_string(value)?))
            }
            StructuredValue::Bytes(_) => None,
            StructuredValue::Array(values) => {
                if depth >= limits.max_depth {
                    return Err(structured_depth(limits.max_depth, depth + 1));
                }
                let mut converted = allocation.reserve_vec(values.len())?;
                for value in values {
                    converted.push(
                        convert(value, depth + 1, limits, allocation)?
                            .ok_or(Error::UnsupportedStructuredValue)?,
                    );
                }
                Some(JsonValue::Array(converted))
            }
            StructuredValue::Map(entries) => {
                if depth >= limits.max_depth {
                    return Err(structured_depth(limits.max_depth, depth + 1));
                }
                let mut converted = allocation.reserve_vec(entries.len())?;
                for (key, value) in entries {
                    let StructuredValue::TextString(key) = key else {
                        return Ok(None);
                    };
                    let key = allocation.clone_string(key)?;
                    let value = convert(value, depth + 1, limits, allocation)?
                        .ok_or(Error::UnsupportedStructuredValue)?;
                    converted.push((key, value));
                }
                Some(JsonValue::Object(converted))
            }
        })
    }

    let mut allocation = JsonAllocation {
        budget,
        limits,
        permits: Vec::new(),
    };
    let value = convert(value, 0, limits, &mut allocation)?;
    Ok((value, allocation.permits))
}

fn structured_resource(limit: usize, actual: usize) -> Error {
    Error::ResourceLimit {
        resource: Resource::StructuredValue,
        limit,
        actual,
    }
}

fn hash_set_allocation_bytes<T>(length: usize) -> Result<usize> {
    if length == 0 {
        return Ok(0);
    }
    // `HashSet` keeps a spare bucket for an empty slot and grows at roughly a
    // 7/8 load factor. Charge the next power-of-two bucket table plus one
    // control byte per bucket before asking the allocator for it.
    let buckets = length
        .checked_mul(2)
        .and_then(|length| length.checked_next_power_of_two())
        .ok_or_else(|| structured_resource(usize::MAX, usize::MAX))?;
    buckets
        .checked_mul(size_of::<T>().saturating_add(1))
        .ok_or_else(|| structured_resource(usize::MAX, usize::MAX))
}

fn structured_depth(limit: usize, actual: usize) -> Error {
    structured_resource(limit, actual)
}

/// Converts an arbitrary-precision integer only when its mathematical value
/// is exactly representable as a finite IEEE-754 binary64 integer.
fn integer_to_binary64(integer: &openkache_value::Integer) -> Result<f64> {
    let magnitude = integer.magnitude_be();
    if magnitude.is_empty() {
        return Ok(0.0);
    }
    let leading = magnitude[0].leading_zeros() as usize;
    let bit_length = magnitude
        .len()
        .checked_mul(8)
        .and_then(|bits| bits.checked_sub(leading))
        .ok_or(Error::UnsupportedStructuredValue)?;
    // Binary64's largest finite integer has a 1024-bit magnitude. A larger
    // integer would overflow even if all discarded bits were zero.
    if bit_length > 1024 {
        return Err(Error::UnsupportedStructuredValue);
    }

    let discarded = bit_length.saturating_sub(BINARY64_SIGNIFICAND_BITS as usize);
    if discarded != 0 {
        let whole_bytes = discarded / 8;
        if magnitude
            .get(magnitude.len().saturating_sub(whole_bytes)..)
            .is_some_and(|bytes| bytes.iter().any(|byte| *byte != 0))
        {
            return Err(Error::UnsupportedStructuredValue);
        }
        let remaining_bits = discarded % 8;
        if remaining_bits != 0 {
            let index = magnitude
                .len()
                .checked_sub(whole_bytes + 1)
                .ok_or(Error::UnsupportedStructuredValue)?;
            let mask = (1_u8 << remaining_bits) - 1;
            if magnitude[index] & mask != 0 {
                return Err(Error::UnsupportedStructuredValue);
            }
        }
    }

    let significant_bits = bit_length.min(BINARY64_SIGNIFICAND_BITS as usize);
    let mut significand = 0_u64;
    for bit in 0..significant_bits {
        let position = leading + bit;
        let byte = magnitude[position / 8];
        significand = (significand << 1) | u64::from((byte >> (7 - (position % 8))) & 1);
    }
    let shift = discarded as i32;
    let value = (significand as f64) * 2f64.powi(shift);
    if !value.is_finite() {
        return Err(Error::UnsupportedStructuredValue);
    }
    Ok(if integer.is_negative() { -value } else { value })
}

fn f16_to_f64(bits: u16) -> f64 {
    let sign = f64::from((bits >> 15) & 1);
    let exponent = (bits >> 10) & 0x1f;
    let fraction = bits & 0x03ff;
    if exponent == 0 {
        sign * f64::from(fraction) * 2f64.powi(-24)
    } else if exponent == 0x1f {
        if fraction == 0 {
            if sign == 0.0 {
                f64::INFINITY
            } else {
                f64::NEG_INFINITY
            }
        } else {
            f64::NAN
        }
    } else {
        let significand = 1.0 + f64::from(fraction) / 1024.0;
        if sign == 0.0 {
            significand * 2f64.powi(i32::from(exponent) - 15)
        } else {
            -significand * 2f64.powi(i32::from(exponent) - 15)
        }
    }
}

fn structured_limits(limits: ValueLimits) -> StructuredLimits {
    StructuredLimits {
        max_bytes: limits
            .max_expanded_payload_bytes
            .min(limits.max_in_flight_bytes),
        max_depth: limits.max_depth,
        max_items: limits.max_items,
        max_integer_bytes: limits.max_integer_bytes,
    }
}

fn validate_namespace(namespace_id: u64) -> Result<()> {
    if namespace_id == 0 {
        Err(Error::InvalidNamespace)
    } else {
        Ok(())
    }
}

fn validate_item_id(item_id: &[u8]) -> Result<()> {
    if item_id.len() > ITEM_ID_BYTES {
        Err(Error::InvalidItemIdLength(item_id.len()))
    } else {
        Ok(())
    }
}

fn validate_limits(limits: ValueLimits) -> Result<()> {
    if limits.max_depth > MAX_VALUE_DEPTH {
        return Err(Error::ResourceLimit {
            resource: Resource::StructuredValue,
            limit: MAX_VALUE_DEPTH,
            actual: limits.max_depth,
        });
    }
    if limits.max_envelope_bytes == 0
        || limits.max_expanded_payload_bytes == 0
        || limits.max_zstd_window_bytes == 0
        || limits.max_depth == 0
        || limits.max_items == 0
        || limits.max_integer_bytes == 0
        || limits.max_in_flight_bytes == 0
        || limits.max_envelope_bytes > MAX_VALUE_ENVELOPE_BYTES
        || limits.max_expanded_payload_bytes > MAX_EXPANDED_PAYLOAD_BYTES
        || limits.max_zstd_window_bytes > MAX_ZSTD_WINDOW_BYTES
    {
        return Err(Error::ResourceLimit {
            resource: Resource::EnvelopeBytes,
            limit: 1,
            actual: 0,
        });
    }
    Ok(())
}

fn validate_compression(compression: Compression) -> Result<()> {
    if let Compression::Zstandard(options) = compression {
        if !(DEFAULT_ZSTANDARD_LEVEL_MIN..=DEFAULT_ZSTANDARD_LEVEL_MAX).contains(&options.level) {
            return Err(Error::InvalidCompressionLevel(options.level));
        }
    }
    Ok(())
}

fn make_selector(protection: u8, compression: u8, format: u8) -> Result<u8> {
    if protection > 3 || compression > 3 || format > 3 {
        return Err(Error::UnsupportedSelector(0xff));
    }
    let selector = protection | (compression << 2) | (format << 4);
    if selector & RESERVED_SELECTOR_MASK != 0 {
        return Err(Error::UnsupportedSelector(selector));
    }
    Ok(selector)
}

fn parse_selector(selector: u8) -> Result<(Encryption, u8, u8)> {
    if selector & RESERVED_SELECTOR_MASK != 0 {
        return Err(Error::UnsupportedSelector(selector));
    }
    let protection_id = selector & PROTECTION_MASK;
    let compression_id = (selector & COMPRESSION_MASK) >> 2;
    let format = (selector & PAYLOAD_MASK) >> 4;
    let protection =
        Encryption::from_selector(protection_id).ok_or(Error::UnsupportedSelector(selector))?;
    if compression_id > COMPRESSION_ZSTANDARD {
        return Err(Error::UnsupportedCompression(compression_id));
    }
    if format > PAYLOAD_STRUCTURED_CBOR_V1 {
        return Err(Error::UnsupportedPayloadFormat(format));
    }
    Ok((protection, compression_id, format))
}

fn compress_if_beneficial(
    payload: &[u8],
    compression: Compression,
    limits: &ValueLimits,
    budget: &RequestBudget,
    max_body_bytes: usize,
) -> Result<(Vec<u8>, u8, Option<BytePermit>)> {
    let Compression::Zstandard(options) = compression else {
        return raw_payload(payload, limits, budget, max_body_bytes);
    };
    if payload.len() < options.minimum_input_size {
        return raw_payload(payload, limits, budget, max_body_bytes);
    }
    let bound = ZSTD_compressBound(payload.len());
    if bound > limits.max_in_flight_bytes {
        // A compression bound is a temporary allocation requirement. If it
        // cannot fit the operation's in-flight limit, retain the v1 raw
        // envelope whenever its complete body still fits.  Do not apply the
        // envelope-body limit to the bound itself: a payload may be too large
        // to store raw while its actual compressed frame still fits.
        return raw_payload(payload, limits, budget, max_body_bytes);
    }
    let compression_permit = match reserve_budget(budget, bound, limits, Resource::EnvelopeBytes) {
        Ok(permit) => permit,
        Err(_error) if payload.len() <= max_body_bytes => {
            // A competing operation may consume the temporary compression
            // capacity even though the raw envelope remains valid.
            return raw_payload(payload, limits, budget, max_body_bytes);
        }
        Err(error) => return Err(error),
    };
    let mut compressed = Vec::new();
    compressed
        .try_reserve_exact(bound)
        .map_err(|_| Error::Allocation { size: bound })?;
    compressed.resize(bound, 0);
    let compressed_length = ZSTD_compress(&mut compressed, payload, options.level);
    if let Err(error) = check_zstandard("compression", compressed_length) {
        drop(compression_permit);
        return Err(error);
    }
    if compressed_length >= payload.len()
        || payload.len() - compressed_length < options.minimum_savings
    {
        drop(compression_permit);
        return raw_payload(payload, limits, budget, max_body_bytes);
    }
    if compressed_length > max_body_bytes {
        drop(compression_permit);
        if payload.len() <= max_body_bytes {
            return raw_payload(payload, limits, budget, max_body_bytes);
        }
        let size = limits
            .max_envelope_bytes
            .saturating_sub(max_body_bytes)
            .saturating_add(compressed_length);
        return Err(Error::EncodedValueTooLarge {
            size,
            maximum: limits.max_envelope_bytes,
        });
    }
    compressed.truncate(compressed_length);
    // The bound is only a scratch allocation. Copy the frame into an exact
    // output-sized buffer so the permit retained by the caller reflects the
    // bytes that remain live after compression.
    let output_permit = reserve_budget(budget, compressed_length, limits, Resource::EnvelopeBytes)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(compressed_length)
        .map_err(|_| Error::Allocation {
            size: compressed_length,
        })?;
    output.extend_from_slice(&compressed);
    drop(compressed);
    drop(compression_permit);
    Ok((output, COMPRESSION_ZSTANDARD, Some(output_permit)))
}

fn compress_owned(
    payload: Vec<u8>,
    payload_permit: BytePermit,
    compression: Compression,
    limits: &ValueLimits,
    budget: &RequestBudget,
    max_body_bytes: usize,
) -> Result<(Vec<u8>, u8, Option<BytePermit>)> {
    let Compression::Zstandard(options) = compression else {
        return Ok((payload, COMPRESSION_NONE, Some(payload_permit)));
    };
    if payload.len() < options.minimum_input_size {
        return Ok((payload, COMPRESSION_NONE, Some(payload_permit)));
    }
    let bound = ZSTD_compressBound(payload.len());
    if bound > limits.max_in_flight_bytes {
        if payload.len() <= max_body_bytes {
            return Ok((payload, COMPRESSION_NONE, Some(payload_permit)));
        }
        return Err(encoded_body_too_large(
            payload.len(),
            limits.max_envelope_bytes,
            max_body_bytes,
        ));
    }
    let compression_permit = match reserve_budget(budget, bound, limits, Resource::EnvelopeBytes) {
        Ok(permit) => permit,
        Err(_error) if payload.len() <= max_body_bytes => {
            // Existing structured payload or request permits may leave less
            // capacity than the temporary compression bound. Compression is
            // an optimization; retry the raw form while it still fits.
            return Ok((payload, COMPRESSION_NONE, Some(payload_permit)));
        }
        Err(error) => return Err(error),
    };
    let mut compressed = Vec::new();
    compressed
        .try_reserve_exact(bound)
        .map_err(|_| Error::Allocation { size: bound })?;
    compressed.resize(bound, 0);
    let compressed_length = ZSTD_compress(&mut compressed, &payload, options.level);
    if let Err(error) = check_zstandard("compression", compressed_length) {
        drop(compression_permit);
        return Err(error);
    }
    if compressed_length >= payload.len()
        || payload.len() - compressed_length < options.minimum_savings
    {
        drop(compression_permit);
        return Ok((payload, COMPRESSION_NONE, Some(payload_permit)));
    }
    if compressed_length > max_body_bytes {
        drop(compression_permit);
        if payload.len() <= max_body_bytes {
            return Ok((payload, COMPRESSION_NONE, Some(payload_permit)));
        }
        return Err(encoded_body_too_large(
            compressed_length,
            limits.max_envelope_bytes,
            max_body_bytes,
        ));
    }
    compressed.truncate(compressed_length);
    let output_permit = reserve_budget(budget, compressed_length, limits, Resource::EnvelopeBytes)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(compressed_length)
        .map_err(|_| Error::Allocation {
            size: compressed_length,
        })?;
    output.extend_from_slice(&compressed);
    drop(compressed);
    drop(compression_permit);
    drop(payload);
    drop(payload_permit);
    Ok((output, COMPRESSION_ZSTANDARD, Some(output_permit)))
}

fn encoded_body_too_large(
    body_length: usize,
    max_envelope_bytes: usize,
    max_body_bytes: usize,
) -> Error {
    let size = max_envelope_bytes
        .saturating_sub(max_body_bytes)
        .saturating_add(body_length);
    Error::EncodedValueTooLarge {
        size,
        maximum: max_envelope_bytes,
    }
}

fn raw_payload(
    payload: &[u8],
    limits: &ValueLimits,
    budget: &RequestBudget,
    max_body_bytes: usize,
) -> Result<(Vec<u8>, u8, Option<BytePermit>)> {
    if payload.len() > max_body_bytes {
        let prefix = limits.max_envelope_bytes.saturating_sub(max_body_bytes);
        let size = payload.len().saturating_add(prefix);
        return Err(Error::EncodedValueTooLarge {
            size,
            maximum: limits.max_envelope_bytes,
        });
    }
    let permit = reserve_budget(
        budget,
        payload.len(),
        limits,
        Resource::ExpandedPayloadBytes,
    )?;
    let mut raw = Vec::new();
    raw.try_reserve_exact(payload.len())
        .map_err(|_| Error::Allocation {
            size: payload.len(),
        })?;
    raw.extend_from_slice(payload);
    Ok((raw, COMPRESSION_NONE, Some(permit)))
}

fn decompress_zstandard(
    compressed: &[u8],
    limits: &ValueLimits,
    budget: &RequestBudget,
) -> Result<(Vec<u8>, BytePermit)> {
    let mut header = ZSTD_FrameHeader::default();
    let header_result = ZSTD_getFrameHeader(&mut header, compressed);
    check_zstandard("frame-header decoding", header_result)?;
    if header_result != 0 {
        return Err(Error::TruncatedEnvelope);
    }
    if header.frameType != ZSTD_FrameType_e::ZSTD_frame {
        return Err(Error::Zstandard {
            operation: "frame validation",
            message: "skippable frames are not supported".into(),
        });
    }
    if header.frameContentSize == ZSTD_CONTENTSIZE_UNKNOWN {
        return Err(Error::Zstandard {
            operation: "frame validation",
            message: "frame must declare content size".into(),
        });
    }
    if header.dictID != 0 {
        return Err(Error::Zstandard {
            operation: "frame validation",
            message: "external dictionaries are not supported".into(),
        });
    }
    let window = usize::try_from(header.windowSize).map_err(|_| Error::ResourceLimit {
        resource: Resource::ZstdWindowBytes,
        limit: limits.max_zstd_window_bytes,
        actual: usize::MAX,
    })?;
    if window > limits.max_zstd_window_bytes {
        return Err(Error::ResourceLimit {
            resource: Resource::ZstdWindowBytes,
            limit: limits.max_zstd_window_bytes,
            actual: window,
        });
    }
    let original_length =
        usize::try_from(header.frameContentSize).map_err(|_| Error::ResourceLimit {
            resource: Resource::ExpandedPayloadBytes,
            limit: limits.max_expanded_payload_bytes,
            actual: usize::MAX,
        })?;
    if original_length > limits.max_expanded_payload_bytes {
        return Err(Error::ResourceLimit {
            resource: Resource::ExpandedPayloadBytes,
            limit: limits.max_expanded_payload_bytes,
            actual: original_length,
        });
    }
    if original_length > limits.max_in_flight_bytes {
        return Err(Error::ResourceLimit {
            resource: Resource::ExpandedPayloadBytes,
            limit: limits.max_in_flight_bytes,
            actual: original_length,
        });
    }
    let frame_length = ZSTD_findFrameCompressedSize(compressed);
    check_zstandard("frame-size validation", frame_length)?;
    if frame_length != compressed.len() {
        return Err(Error::Zstandard {
            operation: "frame validation",
            message: "multiple frames or trailing bytes are not supported".into(),
        });
    }
    let output_permit = budget
        .try_reserve(original_length)
        .map_err(|_| Error::ResourceLimit {
            resource: Resource::ExpandedPayloadBytes,
            limit: budget.capacity(),
            actual: original_length,
        })?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(original_length)
        .map_err(|_| Error::Allocation {
            size: original_length,
        })?;
    output.resize(original_length, 0);
    let decoded = ZSTD_decompress(&mut output, compressed);
    check_zstandard("decompression", decoded)?;
    if decoded != original_length {
        return Err(Error::DecompressedLength {
            expected: original_length,
            actual: decoded,
        });
    }
    Ok((output, output_permit))
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

fn make_aad(namespace_id: u64, item_id: &[u8], selector: u8, key_id: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(
        AAD_DOMAIN.len() + 8 + 1 + item_id.len() + VERSION_BYTES.len() + 1 + key_id.len(),
    );
    aad.extend_from_slice(AAD_DOMAIN);
    aad.extend_from_slice(&namespace_id.to_be_bytes());
    aad.push(item_id.len() as u8);
    aad.extend_from_slice(item_id);
    aad.extend_from_slice(VERSION_BYTES);
    aad.push(selector);
    aad.extend_from_slice(key_id);
    aad
}

fn item_material(
    value_key: &[u8; VALUE_KEY_BYTES],
    key_id: u64,
    namespace_id: u64,
    item_id: &[u8],
) -> Zeroizing<Vec<u8>> {
    let value_derivation_key = Zeroizing::new(blake3::derive_key(VALUE_ROOT_CONTEXT, value_key));
    let mut material = Zeroizing::new(Vec::with_capacity(
        VALUE_KEY_BYTES + 8 + 8 + 1 + item_id.len(),
    ));
    material.extend_from_slice(&value_derivation_key[..]);
    material.extend_from_slice(&key_id.to_be_bytes());
    material.extend_from_slice(&namespace_id.to_be_bytes());
    material.push(item_id.len() as u8);
    material.extend_from_slice(item_id);
    material
}

fn encode_vu128(value: u64) -> Result<Vec<u8>> {
    let mut bytes = [0_u8; MAX_VU128_BYTES];
    let length = vu128::encode_u64(&mut bytes, value);
    Ok(bytes[..length].to_vec())
}

fn decode_vu128(input: &[u8], field: &'static str) -> Result<(u64, usize)> {
    let Some(&first) = input.first() else {
        return Err(Error::InvalidVu128 {
            field,
            reason: "field is truncated",
        });
    };
    let length = vu128::encoded_len(first);
    if length > MAX_VU128_BYTES || input.len() < length {
        return Err(Error::InvalidVu128 {
            field,
            reason: "field is truncated or overlong",
        });
    }
    let mut bytes = [0_u8; MAX_VU128_BYTES];
    bytes[..length].copy_from_slice(&input[..length]);
    let (value, decoded_length) = vu128::decode_u64(&bytes);
    if decoded_length != length {
        return Err(Error::InvalidVu128 {
            field,
            reason: "decoder returned an invalid length",
        });
    }
    let mut canonical = [0_u8; MAX_VU128_BYTES];
    let canonical_length = vu128::encode_u64(&mut canonical, value);
    if canonical_length != length || canonical[..length] != input[..length] {
        return Err(Error::InvalidVu128 {
            field,
            reason: "field is not canonical",
        });
    }
    Ok((value, length))
}
