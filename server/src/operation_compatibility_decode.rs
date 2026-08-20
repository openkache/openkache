//! API-owned field decoding for namespace/item compatibility behavior.
//!
//! The shared dispatcher supplies a generated field view. This module turns
//! protocol-v1 namespace and SET projections into the typed values expected by
//! the storage behavior. Generic field-envelope examples live in
//! [`super::operation_generic_bindings`] and do not depend on these adapters.

use openkache_protocol::{ItemId, OwnedRange};

use super::operation_contract::request_fields;
use super::operation_handlers::OperationInputView;
use crate::protocol::{EvictionMode, ExpirationMode, SetCondition, SetOptions};

const INVALID_NAMESPACE_ID: &[u8] = b"namespace identity must be nonzero";
const INVALID_SET_TTL: &[u8] = b"SET explicit TTL must be positive";

impl OperationInputView {
    fn item_id_at_index(&self, index: usize) -> Option<ItemId> {
        self.field_at_index(index)
            .and_then(|value| ItemId::from_slice(value).ok())
    }

    fn unsigned_long_at_index(&self, index: usize) -> Option<u64> {
        self.encoded_field_at_index(index)
            .and_then(|field| openkache_protocol::codec::decode_u64_be(field.bytes()).ok())
    }

    fn unsigned_long_at_index_result(
        &self,
        index: Option<usize>,
    ) -> Result<Option<u64>, &'static [u8]> {
        index
            .and_then(|index| self.encoded_field_at_index(index))
            .map(|field| openkache_protocol::codec::decode_u64_be(field.bytes()))
            .transpose()
    }
}

/// Typed input for the namespace/item compatibility adapter.
pub(super) struct GetInput {
    pub(super) namespace_id: u64,
    pub(super) item_id: ItemId,
}

/// Typed input for the namespace/item mutation adapter.
pub(super) struct SetInput {
    pub(super) namespace_id: u64,
    pub(super) item_id: ItemId,
    pub(super) value: OwnedRange,
    pub(super) options: SetOptions,
}

/// Typed input for namespace-scoped operations.
pub(super) struct NamespaceInput {
    pub(super) namespace_id: u64,
}

struct SetFields {
    namespace_id: u64,
    item_id: ItemId,
    options: SetOptions,
}

pub(super) fn required_namespace_id(
    input: &OperationInputView,
    field_index: usize,
) -> Result<u64, &'static [u8]> {
    required_positive_unsigned(
        input,
        field_index,
        b"operation requires namespace identity",
        INVALID_NAMESPACE_ID,
    )
}

fn required_positive_unsigned(
    input: &OperationInputView,
    field_index: usize,
    missing: &'static [u8],
    zero: &'static [u8],
) -> Result<u64, &'static [u8]> {
    match input.unsigned_long_at_index(field_index) {
        Some(0) => Err(zero),
        Some(value) => Ok(value),
        None => Err(missing),
    }
}

fn required_item_id_at(
    input: &OperationInputView,
    field_index: usize,
) -> Result<ItemId, &'static [u8]> {
    input
        .item_id_at_index(field_index)
        .ok_or(b"operation requires a valid item ID")
}

pub(super) fn decode_get(input: &OperationInputView) -> Result<GetInput, &'static [u8]> {
    Ok(GetInput {
        namespace_id: required_namespace_id(input, request_fields::op_get::NAMESPACE_ID)?,
        item_id: required_item_id_at(input, request_fields::op_get::ITEM_ID)?,
    })
}

fn decode_set_fields(input: &OperationInputView) -> Result<SetFields, &'static [u8]> {
    let condition = decode_set_condition(input, request_fields::op_set::CONDITION)?;
    let expiration_mode = decode_expiration_mode(input, request_fields::op_set::EXPIRATION_MODE)?;
    let eviction_mode = decode_eviction_mode(input, request_fields::op_set::EVICTION_MODE)?;
    let ttl_ms = input
        .unsigned_long_at_index_result(Some(request_fields::op_set::TTL_MILLISECONDS))
        .map_err(|_| &b"SET TTL is malformed"[..])?;
    validate_set_ttl_value(expiration_mode == ExpirationMode::ExplicitTtl, ttl_ms)?;
    let namespace_id = required_namespace_id(input, request_fields::op_set::NAMESPACE_ID)?;
    let item_id = required_item_id_at(input, request_fields::op_set::ITEM_ID)?;
    input
        .bytes_at_index(request_fields::op_set::VALUE)
        .ok_or(&b"operation requires a value"[..])?;
    Ok(SetFields {
        namespace_id,
        item_id,
        options: SetOptions::with_policies(condition, expiration_mode, ttl_ms, eviction_mode),
    })
}

pub(super) fn decode_set(input: &mut OperationInputView) -> Result<SetInput, &'static [u8]> {
    let SetFields {
        namespace_id,
        item_id,
        options,
    } = decode_set_fields(input)?;
    let value = input
        .take_owned_bytes_range_at_index(request_fields::op_set::VALUE)
        .ok_or(&b"operation requires a value"[..])?;
    // An empty value has no payload allocation to preserve. Release the
    // admitted frame instead of retaining its prefix and spare capacity.
    let value = if value.is_empty() {
        OwnedRange::whole(Vec::new())
    } else {
        value
    };
    Ok(SetInput {
        namespace_id,
        item_id,
        value,
        options,
    })
}

pub(super) fn decode_delete(input: &OperationInputView) -> Result<GetInput, &'static [u8]> {
    Ok(GetInput {
        namespace_id: required_namespace_id(input, request_fields::op_delete::NAMESPACE_ID)?,
        item_id: required_item_id_at(input, request_fields::op_delete::ITEM_ID)?,
    })
}

fn decode_namespace(
    input: &OperationInputView,
    field_index: usize,
) -> Result<NamespaceInput, &'static [u8]> {
    Ok(NamespaceInput {
        namespace_id: required_namespace_id(input, field_index)?,
    })
}

pub(super) fn decode_stats(input: &OperationInputView) -> Result<NamespaceInput, &'static [u8]> {
    decode_namespace(input, request_fields::op_stats::NAMESPACE_ID)
}

pub(super) fn decode_sync(input: &OperationInputView) -> Result<NamespaceInput, &'static [u8]> {
    decode_namespace(input, request_fields::op_sync::NAMESPACE_ID)
}

fn required_token<'a>(
    input: &'a OperationInputView,
    field_index: usize,
    message: &'static [u8],
) -> Result<&'a [u8], &'static [u8]> {
    input.bytes_at_index(field_index).ok_or(message)
}

fn decode_set_condition(
    input: &OperationInputView,
    field_index: usize,
) -> Result<SetCondition, &'static [u8]> {
    match required_token(input, field_index, b"SET condition is missing")? {
        b"any" => Ok(SetCondition::Any),
        b"if_absent" => Ok(SetCondition::IfAbsent),
        b"if_present" => Ok(SetCondition::IfPresent),
        _ => Err(b"SET condition is malformed"),
    }
}

fn decode_expiration_mode(
    input: &OperationInputView,
    field_index: usize,
) -> Result<ExpirationMode, &'static [u8]> {
    match required_token(input, field_index, b"SET expiration mode is missing")? {
        b"inherit" => Ok(ExpirationMode::Inherit),
        b"no_expiry" => Ok(ExpirationMode::NoExpiry),
        b"explicit_ttl" => Ok(ExpirationMode::ExplicitTtl),
        _ => Err(b"SET expiration mode is malformed"),
    }
}

fn decode_eviction_mode(
    input: &OperationInputView,
    field_index: usize,
) -> Result<EvictionMode, &'static [u8]> {
    match required_token(input, field_index, b"SET eviction mode is missing")? {
        b"inherit" => Ok(EvictionMode::Inherit),
        b"evictable" => Ok(EvictionMode::Evictable),
        b"eviction_protected" => Ok(EvictionMode::EvictionProtected),
        _ => Err(b"SET eviction mode is malformed"),
    }
}

pub(super) fn validate_set_ttl(input: &OperationInputView) -> Result<(), &'static [u8]> {
    let expiration_mode = required_token(
        input,
        request_fields::op_set::EXPIRATION_MODE,
        b"SET expiration mode is missing",
    )?;
    let ttl_ms = input
        .unsigned_long_at_index_result(Some(request_fields::op_set::TTL_MILLISECONDS))
        .map_err(|_| &b"SET TTL is malformed"[..])?;
    validate_set_ttl_value(expiration_mode == b"explicit_ttl", ttl_ms)
}

fn validate_set_ttl_value(explicit_ttl: bool, ttl_ms: Option<u64>) -> Result<(), &'static [u8]> {
    match (explicit_ttl, ttl_ms) {
        (true, Some(0)) => Err(INVALID_SET_TTL),
        (true, None) => Err(b"SET explicit TTL is missing"),
        (true, Some(_)) | (false, None) => Ok(()),
        (false, Some(_)) => Err(b"SET TTL is only valid with explicit expiration"),
    }
}
