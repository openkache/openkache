//! Codec primitives shared by protocol adapters.
//!
//! This module knows only wire value shapes. It deliberately does not know
//! operation names, API roles, storage types, or client ABI discriminators.
//! Server and client adapters provide the generated codec-name lookup and map
//! [`CodecError`] to their local error boundary.

use smallvec::SmallVec;

use crate::{
    SegmentedPayload, SegmentedValue, decode_varuint, encode_varuint,
    response::SegmentedEncoding,
};

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
    if payload.len() > crate::MAX_VALUE_BYTES {
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
    let nested = NestedCodecPlan::new(
        nested_codecs,
        nested_widths,
        nested_enum_values,
        nested_union_tags,
        resolve,
    )?;
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
    let kind = resolve(codec).ok_or(CodecError(b"operation contract names an unknown codec"))?;
    if validate_contiguous_root(kind, payload, union_tags, &nested, resolve)?
        == nested_codecs.len()
    {
        Ok(())
    } else {
        Err(CodecError(
            b"nested codec metadata does not match the encoded container shape",
        ))
    }
}

/// Validates a segmented value against the same complete generated codec plan
/// used for contiguous values.
///
/// The composite tree is traversed directly, so nested values retain their
/// original allocations and no complete-value buffer is materialized merely
/// for validation.
pub fn validate_segmented_field_codecs_with_nested_widths(
    value: &SegmentedValue,
    codecs: &[&str],
    nested_codecs: &[&str],
    nested_widths: &[usize],
    nested_enum_values: &[&[&str]],
    nested_union_tags: &[&[u8]],
    _enum_values: &[&str],
    union_tags: &[u8],
    resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
) -> Result<(), CodecError> {
    let nested = NestedCodecPlan::new(
        nested_codecs,
        nested_widths,
        nested_enum_values,
        nested_union_tags,
        resolve,
    )?;
    for codec in codecs {
        let kind =
            resolve(codec).ok_or(CodecError(b"operation contract names an unknown codec"))?;
        match kind {
            CodecKind::RawBytes => {}
            CodecKind::List | CodecKind::Map | CodecKind::Union if kind == value.codec() => {}
            _ => {
                return Err(CodecError(
                    b"segmented value does not satisfy its declared top-level codec",
                ));
            }
        }
    }
    let Some(codec) = codecs.first() else {
        return Err(CodecError(
            b"segmented value requires a generated composite codec",
        ));
    };
    let kind = resolve(codec).ok_or(CodecError(b"operation contract names an unknown codec"))?;
    if value.codec() != kind {
        return Err(CodecError(
            b"segmented value does not match its generated composite codec",
        ));
    }
    let consumed = validate_segmented_root(value, kind, union_tags, &nested, resolve)?;
    if consumed == nested_codecs.len() {
        Ok(())
    } else {
        Err(CodecError(
            b"nested codec metadata does not match the segmented container shape",
        ))
    }
}

struct NestedCodecPlan<'a> {
    codecs: &'a [&'a str],
    widths: &'a [usize],
    enum_values: &'a [&'a [&'a str]],
    union_tags: &'a [&'a [u8]],
}

impl<'a> NestedCodecPlan<'a> {
    fn new(
        codecs: &'a [&'a str],
        widths: &'a [usize],
        enum_values: &'a [&'a [&'a str]],
        union_tags: &'a [&'a [u8]],
        resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
    ) -> Result<Self, CodecError> {
        if (!widths.is_empty() && codecs.len() != widths.len())
            || codecs.len() != enum_values.len()
            || codecs.len() != union_tags.len()
        {
            return Err(CodecError(
                b"nested codec metadata does not have matching width/enum/tag entries",
            ));
        }
        for codec in codecs {
            resolve(codec).ok_or(CodecError(
                b"operation contract names an unknown nested codec",
            ))?;
        }
        let plan = Self {
            codecs,
            widths,
            enum_values,
            union_tags,
        };
        if !codecs.is_empty() {
            let mut cursor = 0;
            while cursor < codecs.len() {
                cursor = plan.node_end(cursor, 0, resolve)?;
            }
        }
        Ok(plan)
    }

    fn kind(
        &self,
        index: usize,
        resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
    ) -> Result<CodecKind, CodecError> {
        let codec = self.codecs.get(index).copied().ok_or(CodecError(
            b"nested composite codec metadata is incomplete",
        ))?;
        resolve(codec).ok_or(CodecError(
            b"operation contract names an unknown nested codec",
        ))
    }

    fn width(&self, index: usize) -> usize {
        self.widths.get(index).copied().unwrap_or(0)
    }

    fn enum_values(&self, index: usize) -> &[&str] {
        self.enum_values.get(index).copied().unwrap_or(&[])
    }

    fn union_tags(&self, index: usize) -> &[u8] {
        self.union_tags.get(index).copied().unwrap_or(&[])
    }

    fn node_end(
        &self,
        index: usize,
        depth: usize,
        resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
    ) -> Result<usize, CodecError> {
        if depth >= MAX_NESTED_CODEC_DEPTH {
            return Err(CodecError(
                b"nested codec metadata exceeds the supported recursion depth",
            ));
        }
        let kind = self.kind(index, resolve)?;
        let mut cursor = index.checked_add(1).ok_or(CodecError(
            b"nested codec metadata exceeds the supported size",
        ))?;
        let child_count = match kind {
            CodecKind::List => 1,
            CodecKind::Map => 2,
            CodecKind::Union => self.union_tags(index).len(),
            _ => 0,
        };
        for _ in 0..child_count {
            cursor = self.node_end(cursor, depth + 1, resolve)?;
        }
        Ok(cursor)
    }

    fn union_variant_bounds(
        &self,
        first_variant: usize,
        depth: usize,
        tag: u8,
        tags: &[u8],
        resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
    ) -> Result<(usize, usize, usize), CodecError> {
        let selected = tags
            .iter()
            .position(|candidate| *candidate == tag)
            .ok_or(INVALID_UNION)?;
        let mut cursor = first_variant;
        let mut selected_bounds = None;
        for variant in 0..tags.len() {
            let end = self.node_end(cursor, depth, resolve)?;
            if variant == selected {
                selected_bounds = Some((cursor, end));
            }
            cursor = end;
        }
        let (start, end) = selected_bounds.ok_or(INVALID_UNION)?;
        Ok((start, end, cursor))
    }
}

fn validate_contiguous_root(
    kind: CodecKind,
    payload: &[u8],
    union_tags: &[u8],
    plan: &NestedCodecPlan<'_>,
    resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
) -> Result<usize, CodecError> {
    match kind {
        CodecKind::List => {
            let mut values = ListCursor::new(payload, DEFAULT_MAX_CONTAINER_ENTRIES)?;
            let end = plan.node_end(0, 1, resolve)?;
            for value in &mut values {
                if validate_contiguous_node(plan, 0, value, 1, resolve)? != end {
                    return Err(CodecError(
                        b"nested list elements use inconsistent codec metadata",
                    ));
                }
            }
            Ok(end)
        }
        CodecKind::Map => {
            let mut entries = MapCursor::new(payload, DEFAULT_MAX_CONTAINER_ENTRIES)?;
            let key_end = plan.node_end(0, 1, resolve)?;
            let value_end = plan.node_end(key_end, 1, resolve)?;
            for (key, value) in &mut entries {
                if validate_contiguous_node(plan, 0, key, 1, resolve)? != key_end
                    || validate_contiguous_node(plan, key_end, value, 1, resolve)? != value_end
                {
                    return Err(CodecError(
                        b"nested map entries use inconsistent codec metadata",
                    ));
                }
            }
            Ok(value_end)
        }
        CodecKind::Union => {
            let tag = validate_union(payload, union_tags)?;
            let union_value = payload.get(1..).ok_or(INVALID_UNION)?;
            let (value, cursor) = read_length_delimited(union_value, 0, INVALID_UNION)?;
            if cursor != union_value.len() {
                return Err(INVALID_UNION);
            }
            if plan.codecs.is_empty() {
                return Ok(0);
            }
            let (variant, variant_end, all_variants_end) =
                plan.union_variant_bounds(0, 1, tag, union_tags, resolve)?;
            if validate_contiguous_node(plan, variant, value, 1, resolve)? != variant_end {
                return Err(CodecError(
                    b"union payload does not match its selected variant codec",
                ));
            }
            Ok(all_variants_end)
        }
        _ => Err(CodecError(
            b"scalar codec cannot declare nested codec metadata",
        )),
    }
}

fn validate_contiguous_node(
    plan: &NestedCodecPlan<'_>,
    index: usize,
    payload: &[u8],
    depth: usize,
    resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
) -> Result<usize, CodecError> {
    if depth >= MAX_NESTED_CODEC_DEPTH {
        return Err(CodecError(
            b"nested codec metadata exceeds the supported recursion depth",
        ));
    }
    let kind = plan.kind(index, resolve)?;
    let width = plan.width(index);
    if width != 0 && payload.len() != width {
        return Err(CodecError(
            b"nested field does not match its declared fixed width",
        ));
    }
    match kind {
        CodecKind::List => {
            let child = index + 1;
            let end = plan.node_end(child, depth + 1, resolve)?;
            let mut values = ListCursor::new(payload, DEFAULT_MAX_CONTAINER_ENTRIES)?;
            for value in &mut values {
                if validate_contiguous_node(plan, child, value, depth + 1, resolve)? != end {
                    return Err(CodecError(
                        b"nested list elements use inconsistent codec metadata",
                    ));
                }
            }
            Ok(end)
        }
        CodecKind::Map => {
            let key = index + 1;
            let key_end = plan.node_end(key, depth + 1, resolve)?;
            let value = key_end;
            let value_end = plan.node_end(value, depth + 1, resolve)?;
            let mut entries = MapCursor::new(payload, DEFAULT_MAX_CONTAINER_ENTRIES)?;
            for (key_bytes, value_bytes) in &mut entries {
                if validate_contiguous_node(plan, key, key_bytes, depth + 1, resolve)? != key_end
                    || validate_contiguous_node(
                        plan,
                        value,
                        value_bytes,
                        depth + 1,
                        resolve,
                    )? != value_end
                {
                    return Err(CodecError(
                        b"nested map entries use inconsistent codec metadata",
                    ));
                }
            }
            Ok(value_end)
        }
        CodecKind::Union => {
            let tags = plan.union_tags(index);
            let tag = validate_union(payload, tags)?;
            let union_value = payload.get(1..).ok_or(INVALID_UNION)?;
            let (value, cursor) = read_length_delimited(union_value, 0, INVALID_UNION)?;
            if cursor != union_value.len() {
                return Err(INVALID_UNION);
            }
            let (variant, variant_end, all_variants_end) =
                plan.union_variant_bounds(index + 1, depth + 1, tag, tags, resolve)?;
            if validate_contiguous_node(plan, variant, value, depth + 1, resolve)? != variant_end {
                return Err(CodecError(
                    b"union payload does not match its selected variant codec",
                ));
            }
            Ok(all_variants_end)
        }
        _ => {
            validate_kind(
                kind,
                payload,
                plan.enum_values(index),
                plan.union_tags(index),
            )?;
            Ok(index + 1)
        }
    }
}

fn validate_segmented_root(
    value: &SegmentedValue,
    kind: CodecKind,
    union_tags: &[u8],
    plan: &NestedCodecPlan<'_>,
    resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
) -> Result<usize, CodecError> {
    match (kind, value.encoding()) {
        (CodecKind::List, SegmentedEncoding::List(values)) => {
            if plan.codecs.is_empty() {
                return Ok(0);
            }
            let end = plan.node_end(0, 1, resolve)?;
            for value in values {
                if validate_segmented_payload(plan, 0, value, 1, resolve)? != end {
                    return Err(CodecError(
                        b"nested list elements use inconsistent codec metadata",
                    ));
                }
            }
            Ok(end)
        }
        (CodecKind::Map, SegmentedEncoding::Map(entries)) => {
            if plan.codecs.is_empty() {
                return Ok(0);
            }
            let key_end = plan.node_end(0, 1, resolve)?;
            let value_end = plan.node_end(key_end, 1, resolve)?;
            for (key, value) in entries {
                if validate_segmented_payload(plan, 0, key, 1, resolve)? != key_end
                    || validate_segmented_payload(plan, key_end, value, 1, resolve)? != value_end
                {
                    return Err(CodecError(
                        b"nested map entries use inconsistent codec metadata",
                    ));
                }
            }
            Ok(value_end)
        }
        (CodecKind::Union, SegmentedEncoding::Union { tag, payload }) => {
            if !union_tags.is_empty() && !union_tags.contains(tag) {
                return Err(INVALID_UNION);
            }
            if plan.codecs.is_empty() {
                return Ok(0);
            }
            let (variant, variant_end, all_variants_end) =
                plan.union_variant_bounds(0, 1, *tag, union_tags, resolve)?;
            if validate_segmented_payload(plan, variant, payload, 1, resolve)? != variant_end {
                return Err(CodecError(
                    b"union payload does not match its selected variant codec",
                ));
            }
            Ok(all_variants_end)
        }
        _ => Err(CodecError(
            b"segmented value does not match its generated composite codec",
        )),
    }
}

fn validate_segmented_payload(
    plan: &NestedCodecPlan<'_>,
    index: usize,
    payload: &SegmentedPayload,
    depth: usize,
    resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
) -> Result<usize, CodecError> {
    if plan.width(index) != 0 && payload.len() != plan.width(index) {
        return Err(CodecError(
            b"nested field does not match its declared fixed width",
        ));
    }
    match payload {
        SegmentedPayload::Contiguous(payload) => {
            validate_contiguous_node(plan, index, payload.as_slice(), depth, resolve)
        }
        SegmentedPayload::Nested(payload) => {
            validate_segmented_node(plan, index, payload, depth, resolve)
        }
    }
}

fn validate_segmented_node(
    plan: &NestedCodecPlan<'_>,
    index: usize,
    value: &SegmentedValue,
    depth: usize,
    resolve: impl Fn(&str) -> Option<CodecKind> + Copy,
) -> Result<usize, CodecError> {
    if depth >= MAX_NESTED_CODEC_DEPTH {
        return Err(CodecError(
            b"nested codec metadata exceeds the supported recursion depth",
        ));
    }
    let kind = plan.kind(index, resolve)?;
    if value.codec() != kind {
        return Err(CodecError(
            b"nested segmented value does not match its generated codec",
        ));
    }
    match (kind, value.encoding()) {
        (CodecKind::List, SegmentedEncoding::List(values)) => {
            let child = index + 1;
            let end = plan.node_end(child, depth + 1, resolve)?;
            for value in values {
                if validate_segmented_payload(plan, child, value, depth + 1, resolve)? != end {
                    return Err(CodecError(
                        b"nested list elements use inconsistent codec metadata",
                    ));
                }
            }
            Ok(end)
        }
        (CodecKind::Map, SegmentedEncoding::Map(entries)) => {
            let key = index + 1;
            let key_end = plan.node_end(key, depth + 1, resolve)?;
            let value_index = key_end;
            let value_end = plan.node_end(value_index, depth + 1, resolve)?;
            for (key_value, value) in entries {
                if validate_segmented_payload(plan, key, key_value, depth + 1, resolve)? != key_end
                    || validate_segmented_payload(
                        plan,
                        value_index,
                        value,
                        depth + 1,
                        resolve,
                    )? != value_end
                {
                    return Err(CodecError(
                        b"nested map entries use inconsistent codec metadata",
                    ));
                }
            }
            Ok(value_end)
        }
        (CodecKind::Union, SegmentedEncoding::Union { tag, payload }) => {
            let tags = plan.union_tags(index);
            if !tags.is_empty() && !tags.contains(tag) {
                return Err(INVALID_UNION);
            }
            let (variant, variant_end, all_variants_end) =
                plan.union_variant_bounds(index + 1, depth + 1, *tag, tags, resolve)?;
            if validate_segmented_payload(plan, variant, payload, depth + 1, resolve)? != variant_end
            {
                return Err(CodecError(
                    b"union payload does not match its selected variant codec",
                ));
            }
            Ok(all_variants_end)
        }
        _ => Err(CodecError(
            b"scalar codec cannot be represented as a segmented composite",
        )),
    }
}

/// Decodes one application payload as UTF-8 without allocating.
pub fn decode_utf8(payload: &[u8]) -> Result<&str, &'static [u8]> {
    if payload.len() > crate::MAX_VALUE_BYTES {
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
    if payload.len() > crate::MAX_VALUE_BYTES {
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
    if payload.len() > crate::MAX_VALUE_BYTES {
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

/// Encodes a list while retaining every already-owned element allocation.
///
/// Count and element-length metadata stay inline. The returned value can be
/// nested directly in a segmented response, avoiding both an intermediate
/// slice collection and one complete-list allocation.
pub fn encode_list_segmented<I>(values: I) -> Result<SegmentedValue, CodecError>
where
    I: IntoIterator,
    I::Item: Into<SegmentedPayload>,
{
    let values = values
        .into_iter()
        .map(Into::into)
        .collect::<SmallVec<[SegmentedPayload; 8]>>();
    let count = u64::try_from(values.len()).map_err(|_| TOO_MANY_ENTRIES)?;
    let (_, count_len) = encode_varuint(count);
    let encoded_len = segmented_children_len(count_len, values.iter())?;
    SegmentedValue::new(encoded_len, SegmentedEncoding::List(values))
        .map_err(|_| VALUE_TOO_LARGE)
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
        if payload.len() > crate::MAX_VALUE_BYTES {
            return Err(VALUE_TOO_LARGE);
        }
        let (count, mut cursor) = decode_container_count(payload)?;
        let values_start = cursor;
        let count = usize::try_from(count).map_err(|_| TOO_MANY_ENTRIES)?;
        if count > max_entries {
            return Err(TOO_MANY_ENTRIES);
        }
        if count > payload.len().saturating_sub(cursor) / 4 {
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

/// Encodes a map without coalescing owned or nested key/value payloads.
pub fn encode_map_segmented<I, K, V>(entries: I) -> Result<SegmentedValue, CodecError>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<SegmentedPayload>,
    V: Into<SegmentedPayload>,
{
    let entries = entries
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect::<SmallVec<[(SegmentedPayload, SegmentedPayload); 4]>>();
    let count = u64::try_from(entries.len()).map_err(|_| TOO_MANY_ENTRIES)?;
    let (_, count_len) = encode_varuint(count);
    let encoded_len = segmented_children_len(
        count_len,
        entries.iter().flat_map(|(key, value)| [key, value]),
    )?;
    SegmentedValue::new(encoded_len, SegmentedEncoding::Map(entries))
        .map_err(|_| VALUE_TOO_LARGE)
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
        if payload.len() > crate::MAX_VALUE_BYTES {
            return Err(VALUE_TOO_LARGE);
        }
        let (count, mut cursor) = decode_container_count(payload)?;
        let entries_start = cursor;
        let count = usize::try_from(count).map_err(|_| TOO_MANY_ENTRIES)?;
        if count > max_entries {
            return Err(TOO_MANY_ENTRIES);
        }
        if count > payload.len().saturating_sub(cursor) / 8 {
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
            .saturating_add(std::mem::size_of::<u32>())
            .saturating_add(payload.len())
            .min(crate::MAX_VALUE_BYTES),
    );
    output.push(tag);
    append_length_delimited(&mut output, payload)?;
    Ok(output)
}

/// Encodes a tagged union while retaining its owned or nested member payload.
pub fn encode_union_segmented(
    tag: u8,
    payload: impl Into<SegmentedPayload>,
    allowed_tags: &[u8],
) -> Result<SegmentedValue, CodecError> {
    if !allowed_tags.is_empty() && !allowed_tags.contains(&tag) {
        return Err(INVALID_UNION);
    }
    let payload = payload.into();
    u32::try_from(payload.len()).map_err(|_| TOO_MANY_ENTRIES)?;
    let encoded_len = 1usize
        .checked_add(std::mem::size_of::<u32>())
        .and_then(|length| length.checked_add(payload.len()))
        .ok_or(VALUE_TOO_LARGE)?;
    SegmentedValue::new(encoded_len, SegmentedEncoding::Union { tag, payload })
        .map_err(|_| VALUE_TOO_LARGE)
}

/// Validates a tagged union and returns its active tag.
pub fn validate_union(payload: &[u8], allowed_tags: &[u8]) -> Result<u8, CodecError> {
    if payload.len() > crate::MAX_VALUE_BYTES {
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
    if payload.len() > crate::MAX_VALUE_BYTES {
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
    let length = u32::try_from(value.len()).map_err(|_| TOO_MANY_ENTRIES)?;
    let next_len = output
        .len()
        .checked_add(std::mem::size_of::<u32>())
        .and_then(|length| length.checked_add(value.len()))
        .ok_or(VALUE_TOO_LARGE)?;
    if next_len > crate::MAX_VALUE_BYTES {
        return Err(VALUE_TOO_LARGE);
    }
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

fn segmented_children_len<'a>(
    prefix_len: usize,
    values: impl IntoIterator<Item = &'a SegmentedPayload>,
) -> Result<usize, CodecError> {
    values.into_iter().try_fold(prefix_len, |total, value| {
        u32::try_from(value.len()).map_err(|_| TOO_MANY_ENTRIES)?;
        total
            .checked_add(std::mem::size_of::<u32>())
            .and_then(|total| total.checked_add(value.len()))
            .filter(|total| *total <= crate::MAX_VALUE_BYTES)
            .ok_or(VALUE_TOO_LARGE)
    })
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
    let length_end = cursor.checked_add(4).ok_or(error)?;
    let bytes = payload.get(cursor..length_end).ok_or(error)?;
    let length = u32::from_be_bytes(bytes.try_into().expect("length width is fixed"));
    let value_start = length_end;
    let value_end = value_start
        .checked_add(usize::try_from(length).map_err(|_| error)?)
        .ok_or(error)?;
    let value = payload.get(value_start..value_end).ok_or(error)?;
    Ok((value, value_end))
}
