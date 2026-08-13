//! Codec primitives shared by protocol adapters.
//!
//! This module knows only wire value shapes. It deliberately does not know
//! operation names, API roles, storage types, or client ABI discriminators.
//! Server and client adapters provide the generated codec-name lookup and map
//! [`CodecError`] to their local error boundary.

use crate::{decode_varuint, encode_varuint};

const INVALID_ENUM: CodecError = CodecError(b"enum value is not declared by its shape");
const INVALID_LIST: CodecError = CodecError(b"list payload is malformed");
const INVALID_MAP: CodecError = CodecError(b"map payload is malformed");
const INVALID_UNION: CodecError = CodecError(b"union payload is malformed");
const TOO_MANY_ENTRIES: CodecError = CodecError(b"container has too many entries");
const VALUE_TOO_LARGE: CodecError = CodecError(b"codec payload exceeds the protocol value limit");

/// Conservative default bound used when a generated shape has no tighter
/// container limit. API adapters can pass a smaller bound to cursor
/// constructors when their modeled operation declares one.
pub const DEFAULT_MAX_CONTAINER_ENTRIES: usize = 1_000_000;

/// Maximum recursive codec metadata depth accepted by the shared validator.
///
/// The generator rejects deeper plans as well. Keeping the runtime guard here
/// protects callers that construct descriptors manually or load an older
/// generated artifact.
pub const MAX_NESTED_CODEC_DEPTH: usize = 64;

/// A bounded semantic codec failure.
///
/// Frame parsing reports framing errors separately. This error describes only
/// a modeled value or reusable container primitive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodecError(&'static [u8]);

impl CodecError {
    /// Creates an adapter-facing error with a stable static diagnostic.
    pub const fn new(message: &'static [u8]) -> Self {
        Self(message)
    }

    /// Returns the stable diagnostic bytes for this codec failure.
    pub const fn message(self) -> &'static [u8] {
        self.0
    }
}

/// Generic wire-value codec capability.
///
/// This enum is owned by the protocol crate rather than generated from one
/// server contract. Generated server/client contracts map their codec IDs to
/// it at the adapter boundary, so this crate remains usable without Smithy
/// operation metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecKind {
    BoolU8,
    Enum,
    F64Be,
    I32Be,
    List,
    Map,
    PackedF64Be,
    RawBytes,
    U64Be,
    Union,
    Utf8,
}

/// Validates one payload against a generated codec kind.
pub fn validate_kind(
    kind: CodecKind,
    payload: &[u8],
    enum_values: &[&str],
    union_tags: &[u8],
) -> Result<(), CodecError> {
    if payload.len() > crate::MAX_PAYLOAD_BYTES {
        return Err(VALUE_TOO_LARGE);
    }
    match kind {
        CodecKind::Utf8 => decode_utf8(payload).map(|_| ()).map_err(CodecError),
        CodecKind::PackedF64Be => {
            if payload.len() % std::mem::size_of::<f64>() != 0 {
                return Err(CodecError(
                    b"packed_f64_be payload length must be a multiple of eight",
                ));
            }
            for chunk in payload.chunks_exact(std::mem::size_of::<f64>()) {
                let value = f64::from_be_bytes(
                    chunk
                        .try_into()
                        .expect("chunks_exact returned a fixed-width chunk"),
                );
                if !value.is_finite() {
                    return Err(CodecError(
                        b"packed_f64_be payload contains a non-finite value",
                    ));
                }
            }
            Ok(())
        }
        CodecKind::RawBytes => Ok(()),
        CodecKind::U64Be => decode_u64_be(payload).map(|_| ()).map_err(CodecError),
        CodecKind::I32Be => decode_i32_be(payload).map(|_| ()).map_err(CodecError),
        CodecKind::F64Be => decode_f64_be(payload)
            .and_then(|value| {
                value
                    .is_finite()
                    .then_some(value)
                    .ok_or(b"f64_be payload contains a non-finite value")
            })
            .map(|_| ())
            .map_err(CodecError),
        CodecKind::BoolU8 => decode_bool(payload).map(|_| ()).map_err(CodecError),
        CodecKind::Enum => validate_enum_bytes(payload, enum_values),
        CodecKind::List => validate_list(payload, DEFAULT_MAX_CONTAINER_ENTRIES).map(|_| ()),
        CodecKind::Map => validate_map(payload, DEFAULT_MAX_CONTAINER_ENTRIES).map(|_| ()),
        CodecKind::Union => validate_union(payload, union_tags).map(|_| ()),
    }
}

/// Validates a field's top-level and recursive codec metadata.
///
/// `resolve` is the adapter-owned mapping from generated codec IDs to this
/// module's open [`CodecKind`] vocabulary. The recursive traversal is
/// shared, so server and client cannot silently diverge on nested list, map,
/// or union validation.
/// Validates one field with the older metadata shape.
///
/// Older adapters did not carry nested width declarations. Preserve their
/// behavior by treating every nested width as unknown.
pub fn validate_field_codecs(
    payload: &[u8],
    codecs: &[&str],
    nested_codecs: &[&str],
    nested_enum_values: &[&[&str]],
    nested_union_tags: &[&[u8]],
    enum_values: &[&str],
    union_tags: &[u8],
    resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
) -> Result<(), CodecError> {
    validate_field_codecs_with_nested_widths(
        payload,
        codecs,
        nested_codecs,
        &[],
        nested_enum_values,
        nested_union_tags,
        enum_values,
        union_tags,
        resolve,
    )
}

/// Validates one field with generated nested codec widths.
///
/// The width vector is parallel to `nested_codecs`; zero means that the
/// nested value is variable-width or has not been proven fixed by generation.
pub fn validate_field_codecs_with_nested_widths(
    payload: &[u8],
    codecs: &[&str],
    nested_codecs: &[&str],
    nested_widths: &[usize],
    nested_enum_values: &[&[&str]],
    nested_union_tags: &[&[u8]],
    enum_values: &[&str],
    union_tags: &[u8],
    resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
) -> Result<(), CodecError> {
    if (!nested_widths.is_empty() && nested_codecs.len() != nested_widths.len())
        || nested_codecs.len() != nested_enum_values.len()
        || nested_codecs.len() != nested_union_tags.len()
    {
        return Err(CodecError(
            b"nested codec metadata does not have matching width/enum/tag entries",
        ));
    }
    // Validate the complete metadata path even when the container has zero
    // entries. Otherwise an empty list/map could make an unknown child codec
    // appear valid simply because no element happened to exercise it.
    for codec in nested_codecs {
        resolve(codec).ok_or(CodecError(
            b"operation contract names an unknown nested codec",
        ))?;
    }
    for codec in codecs {
        let kind =
            resolve(codec).ok_or(CodecError(b"operation contract names an unknown codec"))?;
        validate_kind(kind, payload, enum_values, union_tags)?;
    }
    let Some(codec) = codecs.first() else {
        // A structure can expose nested operation fields without having one
        // container envelope of its own. Those child fields are validated
        // when their own generated plan entries are visited.
        return Ok(());
    };
    if nested_codecs.is_empty() {
        return Ok(());
    }
    let (remaining_codecs, remaining_widths, remaining_enums, remaining_tags) =
        validate_nested_value(
            codec,
            payload,
            0,
            0,
            enum_values,
            union_tags,
            nested_codecs,
            nested_widths,
            nested_enum_values,
            nested_union_tags,
            resolve,
        )?;
    if remaining_codecs.is_empty()
        && remaining_widths.is_empty()
        && remaining_enums.is_empty()
        && remaining_tags.is_empty()
    {
        Ok(())
    } else {
        Err(CodecError(
            b"nested codec metadata does not match the encoded container shape",
        ))
    }
}

type NestedMetadata<'a> = (
    &'a [&'a str],
    &'a [usize],
    &'a [&'a [&'a str]],
    &'a [&'a [u8]],
);

fn validate_nested_value<'a>(
    codec: &str,
    payload: &[u8],
    width: usize,
    depth: usize,
    own_enum_values: &'a [&'a str],
    own_union_tags: &'a [u8],
    nested_codecs: &'a [&str],
    nested_widths: &'a [usize],
    nested_enum_values: &'a [&'a [&'a str]],
    nested_union_tags: &'a [&'a [u8]],
    resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
) -> Result<NestedMetadata<'a>, CodecError> {
    if depth > MAX_NESTED_CODEC_DEPTH {
        return Err(CodecError(
            b"nested codec metadata exceeds the supported recursion depth",
        ));
    }
    let kind = resolve(codec).ok_or(CodecError(b"operation contract names an unknown codec"))?;
    if width != 0 && payload.len() != width {
        return Err(CodecError(
            b"nested field does not match its declared fixed width",
        ));
    }
    match kind {
        CodecKind::List => {
            let mut values = ListCursor::new(payload, DEFAULT_MAX_CONTAINER_ENTRIES)?;
            let Some(next) = nested_codecs.first().copied() else {
                return Err(CodecError(b"nested list codec metadata is incomplete"));
            };
            let mut after_first: Option<NestedMetadata<'_>> = None;
            for value in &mut values {
                let after = validate_nested_value(
                    next,
                    value,
                    nested_widths.first().copied().unwrap_or(0),
                    depth + 1,
                    nested_enum_values.first().copied().unwrap_or(&[]),
                    nested_union_tags.first().copied().unwrap_or(&[]),
                    nested_codecs.get(1..).unwrap_or(&[]),
                    nested_widths.get(1..).unwrap_or(&[]),
                    nested_enum_values.get(1..).unwrap_or(&[]),
                    nested_union_tags.get(1..).unwrap_or(&[]),
                    resolve,
                )?;
                if let Some(previous) = after_first
                    && (previous.0.len() != after.0.len()
                        || previous.1.len() != after.1.len()
                        || previous.2.len() != after.2.len()
                        || previous.3.len() != after.3.len())
                {
                    return Err(CodecError(
                        b"nested list elements use inconsistent codec metadata",
                    ));
                }
                after_first = Some(after);
            }
            // Empty lists have no child bytes to inspect. The declared child
            // sequence is still structurally valid.
            Ok(after_first.unwrap_or((&[], &[], &[], &[])))
        }
        CodecKind::Map => {
            let mut entries = MapCursor::new(payload, DEFAULT_MAX_CONTAINER_ENTRIES)?;
            let key_codec = nested_codecs
                .first()
                .copied()
                .ok_or(CodecError(b"nested map key codec metadata is incomplete"))?;
            let mut after_first: Option<NestedMetadata<'_>> = None;
            for (key, value) in &mut entries {
                let after_key = validate_nested_value(
                    key_codec,
                    key,
                    nested_widths.first().copied().unwrap_or(0),
                    depth + 1,
                    nested_enum_values.first().copied().unwrap_or(&[]),
                    nested_union_tags.first().copied().unwrap_or(&[]),
                    nested_codecs.get(1..).unwrap_or(&[]),
                    nested_widths.get(1..).unwrap_or(&[]),
                    nested_enum_values.get(1..).unwrap_or(&[]),
                    nested_union_tags.get(1..).unwrap_or(&[]),
                    resolve,
                )?;
                let value_codec = after_key
                    .0
                    .first()
                    .copied()
                    .ok_or(CodecError(b"nested map value codec metadata is incomplete"))?;
                let after = validate_nested_value(
                    value_codec,
                    value,
                    after_key.1.first().copied().unwrap_or(0),
                    depth + 1,
                    after_key.2.first().copied().unwrap_or(&[]),
                    after_key.3.first().copied().unwrap_or(&[]),
                    after_key.0.get(1..).unwrap_or(&[]),
                    after_key.1.get(1..).unwrap_or(&[]),
                    after_key.2.get(1..).unwrap_or(&[]),
                    after_key.3.get(1..).unwrap_or(&[]),
                    resolve,
                )?;
                if let Some(previous) = after_first
                    && (previous.0.len() != after.0.len()
                        || previous.1.len() != after.1.len()
                        || previous.2.len() != after.2.len()
                        || previous.3.len() != after.3.len())
                {
                    return Err(CodecError(
                        b"nested map entries use inconsistent codec metadata",
                    ));
                }
                after_first = Some(after);
            }
            Ok(after_first.unwrap_or((&[], &[], &[], &[])))
        }
        CodecKind::Union => {
            validate_union(payload, own_union_tags)?;
            let union_value = payload.get(1..).ok_or(INVALID_UNION)?;
            let (value, cursor) = read_length_delimited(union_value, 0, INVALID_UNION)?;
            if cursor != union_value.len() {
                return Err(INVALID_UNION);
            }
            let child_codec = nested_codecs
                .first()
                .copied()
                .ok_or(CodecError(b"nested union codec metadata is incomplete"))?;
            validate_nested_value(
                child_codec,
                value,
                nested_widths.first().copied().unwrap_or(0),
                depth + 1,
                nested_enum_values.first().copied().unwrap_or(&[]),
                nested_union_tags.first().copied().unwrap_or(&[]),
                nested_codecs.get(1..).unwrap_or(&[]),
                nested_widths.get(1..).unwrap_or(&[]),
                nested_enum_values.get(1..).unwrap_or(&[]),
                nested_union_tags.get(1..).unwrap_or(&[]),
                resolve,
            )
        }
        _ => {
            validate_kind(kind, payload, own_enum_values, own_union_tags)?;
            Ok((
                nested_codecs,
                nested_widths,
                nested_enum_values,
                nested_union_tags,
            ))
        }
    }
}

/// Decodes one application payload as UTF-8 without allocating.
pub fn decode_utf8(payload: &[u8]) -> Result<&str, &'static [u8]> {
    if payload.len() > crate::MAX_PAYLOAD_BYTES {
        return Err(VALUE_TOO_LARGE.message());
    }
    std::str::from_utf8(payload).map_err(|_| b"application value must be valid UTF-8" as _)
}

/// Decodes one fixed-width big-endian unsigned integer.
pub fn decode_u64_be(payload: &[u8]) -> Result<u64, &'static [u8]> {
    let bytes: [u8; std::mem::size_of::<u64>()] = payload
        .try_into()
        .map_err(|_| b"u64_be field must contain exactly eight bytes" as &'static [u8])?;
    Ok(u64::from_be_bytes(bytes))
}

/// Decodes one fixed-width big-endian signed integer.
pub fn decode_i32_be(payload: &[u8]) -> Result<i32, &'static [u8]> {
    let bytes: [u8; std::mem::size_of::<i32>()] = payload
        .try_into()
        .map_err(|_| b"i32_be field must contain exactly four bytes" as &'static [u8])?;
    Ok(i32::from_be_bytes(bytes))
}

/// Decodes one fixed-width big-endian binary64 value.
pub fn decode_f64_be(payload: &[u8]) -> Result<f64, &'static [u8]> {
    let bytes: [u8; std::mem::size_of::<f64>()] = payload
        .try_into()
        .map_err(|_| b"f64_be field must contain exactly eight bytes" as &'static [u8])?;
    Ok(f64::from_be_bytes(bytes))
}

/// Decodes the canonical one-byte boolean used by generic field sequences.
pub fn decode_bool(payload: &[u8]) -> Result<bool, &'static [u8]> {
    match payload {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(b"boolean field must contain exactly one byte, either 0 or 1"),
    }
}

/// Encodes one fixed-width big-endian unsigned integer.
pub fn encode_u64_be(value: u64) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

/// Encodes one fixed-width big-endian signed integer.
pub fn encode_i32_be(value: i32) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

/// Encodes one fixed-width big-endian binary64 value.
pub fn encode_f64_be(value: f64) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

/// Encodes the canonical one-byte boolean used by generic field sequences.
pub fn encode_bool(value: bool) -> Vec<u8> {
    vec![u8::from(value)]
}

/// Encodes one UTF-8 application value.
pub fn encode_utf8(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}

/// Returns an owned copy for the raw byte codec.
pub fn decode_raw_bytes(payload: &[u8]) -> Vec<u8> {
    payload.to_vec()
}

/// Validates one enum value against generated string members.
pub fn validate_enum_bytes(payload: &[u8], allowed_values: &[&str]) -> Result<(), CodecError> {
    if payload.len() > crate::MAX_PAYLOAD_BYTES {
        return Err(VALUE_TOO_LARGE);
    }
    if allowed_values
        .iter()
        .any(|candidate| candidate.as_bytes() == payload)
    {
        Ok(())
    } else {
        Err(INVALID_ENUM)
    }
}

/// Validates one enum value represented as already encoded bytes.
pub fn validate_enum<'a>(payload: &[u8], allowed_values: &[&'a [u8]]) -> Result<(), CodecError> {
    if payload.len() > crate::MAX_PAYLOAD_BYTES {
        return Err(VALUE_TOO_LARGE);
    }
    if allowed_values.iter().any(|candidate| *candidate == payload) {
        Ok(())
    } else {
        Err(INVALID_ENUM)
    }
}

/// Builds one enum payload after validating it against generated values.
pub fn encode_enum<'a>(payload: &[u8], allowed_values: &[&'a [u8]]) -> Result<Vec<u8>, CodecError> {
    validate_enum(payload, allowed_values)?;
    Ok(payload.to_vec())
}

/// Encodes a list of required length-delimited elements.
pub fn encode_list(values: &[&[u8]]) -> Result<Vec<u8>, CodecError> {
    let count = u64::try_from(values.len()).map_err(|_| TOO_MANY_ENTRIES)?;
    let (count_bytes, count_len) = encode_varuint(count);
    let mut output = Vec::with_capacity(count_len);
    output.extend_from_slice(&count_bytes[..count_len]);
    for value in values {
        append_length_delimited(&mut output, value)?;
    }
    Ok(output)
}

/// Validates a list and returns its element count without allocating.
pub fn validate_list(payload: &[u8], max_entries: usize) -> Result<usize, CodecError> {
    Ok(ListCursor::new(payload, max_entries)?.len())
}

/// Borrowed iterator over a validated length-delimited list.
///
/// Construction validates the complete container and entry bound once. The
/// iterator then advances through the original payload without allocating a
/// vector of element slices.
#[derive(Clone, Copy, Debug)]
pub struct ListCursor<'a> {
    payload: &'a [u8],
    cursor: usize,
    remaining: usize,
}

impl<'a> ListCursor<'a> {
    /// Validates one list and returns a cursor over its borrowed elements.
    pub fn new(payload: &'a [u8], max_entries: usize) -> Result<Self, CodecError> {
        if payload.len() > crate::MAX_PAYLOAD_BYTES {
            return Err(VALUE_TOO_LARGE);
        }
        let (count, mut cursor) = decode_container_count(payload)?;
        let values_start = cursor;
        let count = usize::try_from(count).map_err(|_| TOO_MANY_ENTRIES)?;
        if count > max_entries {
            return Err(TOO_MANY_ENTRIES);
        }
        // Every element has at least one canonical vu128 length byte.
        if count > payload.len().saturating_sub(cursor) {
            return Err(INVALID_LIST);
        }
        for _ in 0..count {
            let (_, next) = read_length_delimited(payload, cursor, INVALID_LIST)?;
            cursor = next;
        }
        if cursor != payload.len() {
            return Err(INVALID_LIST);
        }
        Ok(Self {
            payload,
            cursor: values_start,
            remaining: count,
        })
    }

    /// Returns the validated element count.
    pub const fn len(self) -> usize {
        self.remaining
    }
}

impl<'a> Iterator for ListCursor<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let (value, next) = read_length_delimited(self.payload, self.cursor, INVALID_LIST).ok()?;
        self.cursor = next;
        self.remaining -= 1;
        Some(value)
    }
}

/// Borrows all list elements after validating the canonical container shape.
pub fn decode_list<'a>(payload: &'a [u8], max_entries: usize) -> Result<Vec<&'a [u8]>, CodecError> {
    Ok(ListCursor::new(payload, max_entries)?.collect())
}

/// Encodes a map as a count followed by length-delimited key/value pairs.
pub fn encode_map(entries: &[(&[u8], &[u8])]) -> Result<Vec<u8>, CodecError> {
    let count = u64::try_from(entries.len()).map_err(|_| TOO_MANY_ENTRIES)?;
    let (count_bytes, count_len) = encode_varuint(count);
    let mut output = Vec::new();
    output.extend_from_slice(&count_bytes[..count_len]);
    for (key, value) in entries {
        append_length_delimited(&mut output, key)?;
        append_length_delimited(&mut output, value)?;
    }
    Ok(output)
}

/// Validates a map and returns its entry count without allocating.
pub fn validate_map(payload: &[u8], max_entries: usize) -> Result<usize, CodecError> {
    Ok(MapCursor::new(payload, max_entries)?.len())
}

/// Borrowed iterator over a validated length-delimited map.
#[derive(Clone, Copy, Debug)]
pub struct MapCursor<'a> {
    payload: &'a [u8],
    cursor: usize,
    remaining: usize,
}

impl<'a> MapCursor<'a> {
    /// Validates one map and returns a cursor over borrowed key/value pairs.
    pub fn new(payload: &'a [u8], max_entries: usize) -> Result<Self, CodecError> {
        if payload.len() > crate::MAX_PAYLOAD_BYTES {
            return Err(VALUE_TOO_LARGE);
        }
        let (count, mut cursor) = decode_container_count(payload)?;
        let entries_start = cursor;
        let count = usize::try_from(count).map_err(|_| TOO_MANY_ENTRIES)?;
        if count > max_entries {
            return Err(TOO_MANY_ENTRIES);
        }
        // Every map entry has two canonical vu128 length prefixes.
        if count > payload.len().saturating_sub(cursor) / 2 {
            return Err(INVALID_MAP);
        }
        for _ in 0..count {
            let (_, next_key) = read_length_delimited(payload, cursor, INVALID_MAP)?;
            let (_, next_value) = read_length_delimited(payload, next_key, INVALID_MAP)?;
            cursor = next_value;
        }
        if cursor != payload.len() {
            return Err(INVALID_MAP);
        }
        Ok(Self {
            payload,
            cursor: entries_start,
            remaining: count,
        })
    }

    /// Returns the validated entry count.
    pub const fn len(self) -> usize {
        self.remaining
    }
}

impl<'a> Iterator for MapCursor<'a> {
    type Item = (&'a [u8], &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let (key, next_key) = read_length_delimited(self.payload, self.cursor, INVALID_MAP).ok()?;
        let (value, next_value) =
            read_length_delimited(self.payload, next_key, INVALID_MAP).ok()?;
        self.cursor = next_value;
        self.remaining -= 1;
        Some((key, value))
    }
}

/// Borrows map entries after validating key/value alignment.
pub fn decode_map<'a>(
    payload: &'a [u8],
    max_entries: usize,
) -> Result<Vec<(&'a [u8], &'a [u8])>, CodecError> {
    Ok(MapCursor::new(payload, max_entries)?.collect())
}

/// Encodes one tagged union member.
pub fn encode_union(tag: u8, payload: &[u8], allowed_tags: &[u8]) -> Result<Vec<u8>, CodecError> {
    if !allowed_tags.is_empty() && !allowed_tags.contains(&tag) {
        return Err(INVALID_UNION);
    }
    let mut output = Vec::with_capacity(
        1usize
            .saturating_add(crate::MAX_VARUINT_BYTES)
            .saturating_add(payload.len())
            .min(crate::MAX_PAYLOAD_BYTES),
    );
    output.push(tag);
    append_length_delimited(&mut output, payload)?;
    Ok(output)
}

/// Validates a tagged union and returns its active tag.
pub fn validate_union(payload: &[u8], allowed_tags: &[u8]) -> Result<u8, CodecError> {
    if payload.len() > crate::MAX_PAYLOAD_BYTES {
        return Err(VALUE_TOO_LARGE);
    }
    let Some((&tag, rest)) = payload.split_first() else {
        return Err(INVALID_UNION);
    };
    if !allowed_tags.is_empty() && !allowed_tags.contains(&tag) {
        return Err(INVALID_UNION);
    }
    let (_, cursor) = read_length_delimited(rest, 0, INVALID_UNION)?;
    if cursor != rest.len() {
        return Err(INVALID_UNION);
    }
    Ok(tag)
}

/// Applies a domain transform to a packed big-endian binary64 array.
pub fn transform_packed_f64_be(
    payload: &[u8],
    mut transform: impl FnMut(f64) -> Option<f64>,
) -> Result<Vec<u8>, &'static [u8]> {
    const F64_BYTES: usize = std::mem::size_of::<f64>();
    if payload.len() > crate::MAX_PAYLOAD_BYTES {
        return Err(VALUE_TOO_LARGE.message());
    }
    if payload.len() % F64_BYTES != 0 {
        return Err(b"packed_f64_be payload length must be a multiple of eight");
    }
    let mut output = Vec::with_capacity(payload.len());
    for chunk in payload.chunks_exact(F64_BYTES) {
        let input = f64::from_be_bytes(
            chunk
                .try_into()
                .expect("chunks_exact returned a fixed-width chunk"),
        );
        let value = transform(input)
            .filter(|value| value.is_finite())
            .ok_or(b"packed_f64_be transform produced a non-finite value" as &'static [u8])?;
        output.extend_from_slice(&value.to_be_bytes());
    }
    Ok(output)
}

fn append_length_delimited(output: &mut Vec<u8>, value: &[u8]) -> Result<(), CodecError> {
    let length = u64::try_from(value.len()).map_err(|_| VALUE_TOO_LARGE)?;
    let (encoded_length, encoded_length_len) = encode_varuint(length);
    let next_len = output
        .len()
        .checked_add(encoded_length_len)
        .and_then(|length| length.checked_add(value.len()))
        .ok_or(VALUE_TOO_LARGE)?;
    if next_len > crate::MAX_PAYLOAD_BYTES {
        return Err(VALUE_TOO_LARGE);
    }
    output.extend_from_slice(&encoded_length[..encoded_length_len]);
    output.extend_from_slice(value);
    Ok(())
}

fn decode_container_count(payload: &[u8]) -> Result<(u64, usize), CodecError> {
    decode_varuint(payload, "container element count")
        .map_err(|_| CodecError(b"container count is not canonical"))?
        .ok_or(CodecError(
            b"container payload is missing its element count",
        ))
}

fn read_length_delimited(
    payload: &[u8],
    cursor: usize,
    error: CodecError,
) -> Result<(&[u8], usize), CodecError> {
    let (length, encoded_length_len) = decode_varuint(
        payload.get(cursor..).unwrap_or_default(),
        "container value length",
    )
    .map_err(|_| error)?
    .ok_or(error)?;
    let value_start = cursor.checked_add(encoded_length_len).ok_or(error)?;
    let value_end = value_start
        .checked_add(usize::try_from(length).map_err(|_| error)?)
        .ok_or(error)?;
    let value = payload.get(value_start..value_end).ok_or(error)?;
    Ok((value, value_end))
}
