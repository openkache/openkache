//! Transport-neutral operation field envelopes.
//!
//! This module is independent from compatibility adapters. It exposes
//! generated field metadata, bytes, and reusable codec projections; domain
//! enums and response types remain in API-owned modules.

use super::operation_contract as contract;

/// Descriptor-backed raw field envelope for API-owned generic behavior.
///
/// Generic extensions consume this view when they need a codec or container
/// shape. The generated plan travels with the bytes, so a handler can dispatch
/// on codec IDs and requiredness without adding a domain variant to the server
/// foundation.
#[derive(Clone, Copy, Debug)]
pub(super) struct OperationFieldEnvelope<'a> {
    pub(super) bytes: &'a [u8],
    encoded_width: usize,
    codecs: &'static [&'static str],
    nested_codecs: &'static [&'static str],
    nested_widths: &'static [usize],
    nested_enum_values: &'static [&'static [&'static str]],
    nested_union_tags: &'static [&'static [u8]],
    enum_values: &'static [&'static str],
    union_tags: &'static [u8],
}

impl<'a> OperationFieldEnvelope<'a> {
    /// Creates an API-facing view without exposing the generated plan type.
    ///
    /// The operation decoder owns the plan and chooses which metadata is
    /// useful at this boundary. API behavior therefore depends on stable
    /// codec capabilities and borrowed bytes, not on generated field-layout
    /// structs.
    pub(super) fn from_plan(plan: &'static contract::OperationFieldPlan, bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            encoded_width: plan.encoded_width,
            codecs: plan.codecs,
            nested_codecs: plan.nested_codecs,
            nested_widths: plan.nested_widths,
            nested_enum_values: plan.nested_enum_values,
            nested_union_tags: plan.nested_union_tags,
            enum_values: plan.enum_values,
            union_tags: plan.union_tags,
        }
    }
}

#[allow(dead_code)]
impl OperationFieldEnvelope<'_> {
    /// Returns the encoded field bytes borrowed from the request allocation.
    pub(super) fn bytes(&self) -> &[u8] {
        self.bytes
    }

    pub(super) fn codecs(&self) -> &'static [&'static str] {
        self.codecs
    }

    /// Returns the generated child-codec path without exposing the field plan.
    ///
    /// Child metadata is parallel to the nested enum/tag slices. API-owned
    /// bindings can use it for diagnostics or select a reusable cursor while
    /// the shared validator still owns recursive traversal.
    pub(super) fn nested_codecs(&self) -> &'static [&'static str] {
        self.nested_codecs
    }

    /// Returns fixed child widths; zero denotes variable or unproven width.
    pub(super) fn nested_widths(&self) -> &'static [usize] {
        self.nested_widths
    }

    /// Returns whether the field declares a codec identifier.
    pub(super) fn has_codec(&self, codec: &str) -> bool {
        self.codecs.contains(&codec)
    }

    /// Validates this value against its complete generated codec path.
    ///
    /// The request view already performs this check before dispatch. Keeping
    /// the same operation on the envelope is useful for API-owned transforms
    /// that create or forward a value after borrowing its field bytes.
    pub(super) fn validate(&self) -> Result<(), &'static [u8]> {
        if self.encoded_width != 0 && self.bytes.len() != self.encoded_width {
            return Err(b"field does not match its declared fixed width");
        }
        openkache_protocol::codec::validate_field_codecs_with_nested_widths(
            self.bytes,
            self.codecs,
            self.nested_codecs,
            self.nested_widths,
            self.nested_enum_values,
            self.nested_union_tags,
            self.enum_values,
            self.union_tags,
            contract::wire_codec_kind,
        )
        .map_err(|error| error.message())
    }

    /// Decodes a UTF-8 field without allocating.
    pub(super) fn decode_utf8(&self) -> Result<&str, &'static [u8]> {
        openkache_protocol::codec::decode_utf8(self.bytes)
    }

    /// Returns the generated enum member spellings for an enum field.
    pub(super) fn enum_values(&self) -> &'static [&'static str] {
        self.enum_values
    }

    pub(super) fn decode_u64(&self) -> Result<u64, &'static [u8]> {
        openkache_protocol::codec::decode_u64_be(self.bytes)
    }

    pub(super) fn decode_i32(&self) -> Result<i32, &'static [u8]> {
        openkache_protocol::codec::decode_i32_be(self.bytes)
    }

    pub(super) fn decode_f64(&self) -> Result<f64, &'static [u8]> {
        openkache_protocol::codec::decode_f64_be(self.bytes)
    }

    pub(super) fn decode_bool(&self) -> Result<bool, &'static [u8]> {
        openkache_protocol::codec::decode_bool(self.bytes)
    }

    /// Applies one API-owned transform to a packed floating-point field.
    ///
    /// The envelope retains codec validation and byte traversal so behavior
    /// modules do not depend on protocol codec implementation details.
    pub(super) fn transform_packed_f64(
        &self,
        transform: impl FnMut(f64) -> Option<f64>,
    ) -> Result<Vec<u8>, &'static [u8]> {
        if !self.has_codec("packed_f64_be") {
            return Err(b"field does not declare packed_f64_be");
        }
        openkache_protocol::codec::transform_packed_f64_be(self.bytes, transform)
    }

    /// Decodes container values as borrowed slices. The shared codec has
    /// already validated bounds before an API binding asks for this view.
    pub(super) fn decode_list(&self) -> Result<Vec<&[u8]>, &'static [u8]> {
        openkache_protocol::codec::decode_list(
            self.bytes,
            openkache_protocol::codec::DEFAULT_MAX_CONTAINER_ENTRIES,
        )
        .map_err(|error| error.message())
    }

    /// Returns a borrowed list cursor for APIs that do not need a collected
    /// element vector.
    pub(super) fn list_cursor(
        &self,
        max_entries: usize,
    ) -> Result<openkache_protocol::codec::ListCursor<'_>, &'static [u8]> {
        openkache_protocol::codec::ListCursor::new(self.bytes, max_entries)
            .map_err(|error| error.message())
    }

    pub(super) fn decode_map(&self) -> Result<Vec<(&[u8], &[u8])>, &'static [u8]> {
        openkache_protocol::codec::decode_map(
            self.bytes,
            openkache_protocol::codec::DEFAULT_MAX_CONTAINER_ENTRIES,
        )
        .map_err(|error| error.message())
    }

    /// Returns a borrowed map cursor for APIs that do not need a collected
    /// key/value vector.
    pub(super) fn map_cursor(
        &self,
        max_entries: usize,
    ) -> Result<openkache_protocol::codec::MapCursor<'_>, &'static [u8]> {
        openkache_protocol::codec::MapCursor::new(self.bytes, max_entries)
            .map_err(|error| error.message())
    }

    pub(super) fn decode_union_tag(&self) -> Result<u8, &'static [u8]> {
        openkache_protocol::codec::validate_union(self.bytes, self.union_tags)
            .map_err(|error| error.message())
    }
}

/// Validates one generated field through the same envelope used by API
/// bindings. Request and response projection code therefore cannot drift on
/// nested codec or fixed-width checks.
pub(crate) fn validate_field_bytes(
    plan: &'static contract::OperationFieldPlan,
    bytes: &[u8],
) -> Result<(), &'static [u8]> {
    OperationFieldEnvelope::from_plan(plan, bytes).validate()
}
