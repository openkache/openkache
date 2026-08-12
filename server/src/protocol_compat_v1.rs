//! Compact protocol-v1 request projection.
//!
//! The request plan and frame layout are generated from Smithy metadata.  This
//! module is the semantic facade needed by the public `Request` convenience
//! type; it does not classify operations into an item/set/namespace route
//! enum.  Server execution consumes the same generated plan directly.

use std::borrow::Cow;

use openkache_protocol::{ItemId, Opcode, OwnedRange};
use smallvec::SmallVec;

use super::super::operation_compatibility_contract as compatibility_contract;
use super::super::operation_contract as contract;
use super::{NamespacePolicy, ProtocolError, RequestHeader, Result, SetOptions};

#[path = "protocol_compat_v1_fields.rs"]
mod fields;
#[path = "protocol_compat_v1_policy.rs"]
mod policy;
#[path = "protocol_compat_v1_validation.rs"]
mod validation;
#[path = "protocol_compat_v1_wire.rs"]
mod wire;
use fields::{field_bytes, field_count, field_u64, field_values};
pub(crate) use policy::decode_namespace_policy;
pub(super) use validation::{
    validate_namespace_id, validate_namespace_name, validate_request, validate_revision,
};
use wire::map_wire_decode_error;

pub(super) struct DecodedRequestMetadata {
    pub(super) namespace_id: Option<u64>,
    pub(super) item_ids: Vec<ItemId>,
    pub(super) set_options: SetOptions,
    pub(super) namespace_name: Option<Vec<u8>>,
    pub(super) namespace_policy: Option<NamespacePolicy>,
    pub(super) expected_revision: Option<u64>,
    pub(super) create_if_missing: bool,
}

pub(super) const fn namespace_name_max_bytes() -> usize {
    compatibility_contract::NAMESPACE_NAME_MAX_BYTES
}

/// Returns whether the public [`super::Request`] facade owns a typed
/// protocol-v1 projection for this opcode.
///
/// Exact request plans are also used by generic operations. The explicit
/// generated projection marker, rather than plan presence, is therefore the
/// only valid selector for this compatibility decoder.
pub(super) const fn is_compatibility_operation(opcode: Opcode) -> bool {
    openkache_protocol::compat_v1::request_projection(opcode).is_some()
}

/// Encodes a compact request through its generated declarative plan.
pub(super) fn encode_request(request: &super::Request) -> Result<Vec<u8>> {
    if !is_compatibility_operation(request.opcode) {
        return Err(ProtocolError::InvalidFieldSequence(
            "operation has no protocol-v1 compatibility projection",
        ));
    }
    let plan = contract::request_wire_plan(request.opcode).ok_or(
        ProtocolError::InvalidFieldSequence("operation has no compact request plan"),
    )?;
    let values = request_wire_values(request)?;
    let borrowed: SmallVec<[Option<&[u8]>; 8]> =
        values.iter().map(|value| value.as_deref()).collect();
    openkache_protocol::encode_request_wire_fields(request.opcode, &borrowed, plan)
        .map_err(Into::into)
}

/// Encodes only the compact metadata prefix.
pub(super) fn encode_request_prefix(
    request: &super::Request,
    output: &mut Vec<u8>,
) -> Result<bool> {
    if !is_compatibility_operation(request.opcode) {
        return Ok(false);
    }
    let Some(plan) = contract::request_wire_plan(request.opcode) else {
        return Ok(false);
    };
    let values = request_wire_values(request)?;
    let borrowed: SmallVec<[Option<&[u8]>; 8]> =
        values.iter().map(|value| value.as_deref()).collect();
    let prefix = openkache_protocol::encode_request_wire_prefix(request.opcode, &borrowed, plan)?;
    output.extend_from_slice(&prefix[openkache_protocol::OPCODE_BYTES..]);
    Ok(true)
}

fn request_wire_values(request: &super::Request) -> Result<SmallVec<[Option<Cow<'_, [u8]>>; 8]>> {
    let fields = contract::spec(request.opcode).request.fields;
    let mut item_ids = request.item_ids.iter();
    Ok(fields
        .iter()
        .map(|field| match field.role {
            "namespace_id" => request
                .namespace_id
                .map(|value| Cow::Owned(value.to_be_bytes().to_vec())),
            "item_id" => item_ids
                .next()
                .map(|item_id| Cow::Borrowed(item_id.as_bytes().as_slice())),
            "value" => Some(Cow::Borrowed(request.value.as_slice())),
            "name" => request.namespace_name.as_deref().map(Cow::Borrowed),
            "expected_revision" => request
                .expected_revision
                .map(|value| Cow::Owned(value.to_be_bytes().to_vec())),
            "condition" => Some(Cow::Borrowed(set_condition_value(
                request.set_options.condition,
            ))),
            "expiration_mode" => Some(Cow::Borrowed(expiration_mode_value(
                request.set_options.expiration_mode,
            ))),
            "eviction_mode" => Some(Cow::Borrowed(eviction_mode_value(
                request.set_options.eviction_mode,
            ))),
            "ttl_milliseconds" => request
                .set_options
                .ttl_ms
                .map(|value| Cow::Owned(value.to_be_bytes().to_vec())),
            "create_if_missing" => Some(Cow::Borrowed(boolean_value(request.create_if_missing))),
            "policy" => None,
            "default_expiration" => request
                .namespace_policy
                .map(|policy| Cow::Borrowed(default_expiration_value(policy.default_expiration))),
            "default_ttl_milliseconds" => request.namespace_policy.and_then(|policy| match policy
                .default_expiration
            {
                super::ExpirationDefault::FixedTtl { ttl_ms } => {
                    Some(Cow::Owned(ttl_ms.to_be_bytes().to_vec()))
                }
                super::ExpirationDefault::NoExpiry => None,
            }),
            "expiration_override" => request
                .namespace_policy
                .map(|policy| Cow::Borrowed(override_policy_value(policy.expiration_override))),
            "default_eviction" => request
                .namespace_policy
                .map(|policy| Cow::Borrowed(default_eviction_value(policy.default_eviction))),
            "eviction_override" => request
                .namespace_policy
                .map(|policy| Cow::Borrowed(override_policy_value(policy.eviction_override))),
            _ => None,
        })
        .collect())
}

const fn set_condition_value(value: super::SetCondition) -> &'static [u8] {
    match value {
        super::SetCondition::Any => b"any",
        super::SetCondition::IfAbsent => b"if_absent",
        super::SetCondition::IfPresent => b"if_present",
    }
}

const fn boolean_value(value: bool) -> &'static [u8] {
    if value { b"\x01" } else { b"\x00" }
}

const fn expiration_mode_value(value: super::ExpirationMode) -> &'static [u8] {
    match value {
        super::ExpirationMode::Inherit => b"inherit",
        super::ExpirationMode::NoExpiry => b"no_expiry",
        super::ExpirationMode::ExplicitTtl => b"explicit_ttl",
    }
}

const fn eviction_mode_value(value: super::EvictionMode) -> &'static [u8] {
    match value {
        super::EvictionMode::Inherit => b"inherit",
        super::EvictionMode::Evictable => b"evictable",
        super::EvictionMode::EvictionProtected => b"eviction_protected",
    }
}

const fn default_expiration_value(value: super::ExpirationDefault) -> &'static [u8] {
    match value {
        super::ExpirationDefault::NoExpiry => b"no_expiry",
        super::ExpirationDefault::FixedTtl { .. } => b"fixed_ttl",
    }
}

const fn override_policy_value(value: super::OverridePolicy) -> &'static [u8] {
    match value {
        super::OverridePolicy::Allowed => b"allowed",
        super::OverridePolicy::Disallowed => b"disallowed",
    }
}

const fn default_eviction_value(value: super::EvictionDefault) -> &'static [u8] {
    match value {
        super::EvictionDefault::Evictable => b"evictable",
        super::EvictionDefault::EvictionProtected => b"eviction_protected",
    }
}

/// Decodes one compact header using the generated byte-consumption layout.
pub(super) fn decode_header(prefix: &[u8], opcode: Opcode) -> Result<Option<RequestHeader>> {
    let plan = contract::request_wire_plan(opcode).ok_or(ProtocolError::InvalidFieldSequence(
        "operation has no compact request plan",
    ))?;
    let layout = super::wire_request_layout(opcode);
    // Report semantic packed-flag errors as soon as their byte is retained.
    // Header callers commonly pass a partial prefix and still expect a
    // malformed flag to be rejected before the value-length byte arrives.
    if let Some(error) = wire::classify_wire_prefix(opcode, prefix) {
        return Err(error);
    }
    let Some(header) = openkache_protocol::OpaqueRequestFrame::decode_header(prefix, layout)
        .map_err(|error| map_wire_decode_error(opcode, prefix, error))?
    else {
        return Ok(None);
    };
    let metadata_prefix =
        prefix
            .get(..header.encoded_len())
            .ok_or(ProtocolError::InvalidFieldSequence(
                "compact header exceeds retained prefix",
            ))?;
    let fields = openkache_protocol::decode_request_wire_prefix_fields(
        OwnedRange::whole(metadata_prefix.to_vec()),
        header.value_len(),
        plan,
    )
    .map_err(|error| map_wire_decode_error(opcode, metadata_prefix, error))?;
    let namespace_id = field_u64(opcode, &fields, "namespace_id", 0)?;
    if field_count(opcode, "namespace_id") != 0 {
        validate_namespace_id(namespace_id)?;
    }
    if let Some(name) = field_bytes(opcode, &fields, "name", 0) {
        validate_namespace_name(name)?;
    }
    let expected_revision = field_u64(opcode, &fields, "expected_revision", 0)?;
    if field_count(opcode, "expected_revision") != 0 {
        validate_revision(expected_revision)?;
    }
    // Header decoding is also the semantic admission point used by callers
    // that only retain a prefix. Validate compact policy fields here, while
    // leaving generic frame delimiting unaware of their meaning.
    decode_set_options(opcode, &fields)?;
    decode_policy(opcode, &fields)?;
    let item_count = field_count(opcode, "item_id");
    let has_ttl = field_bytes(opcode, &fields, "ttl_milliseconds", 0).is_some();
    Ok(Some(RequestHeader::compatibility(
        opcode,
        header.encoded_len(),
        header.value_len(),
        namespace_id,
        item_count,
        has_ttl,
    )))
}

pub(super) fn decode_request(frame: &[u8], header: RequestHeader) -> Result<super::Request> {
    let metadata = decode_request_metadata(frame, header)?;
    let value = frame
        .get(header.encoded_len()..)
        .ok_or(ProtocolError::FrameTooShort {
            expected: header.encoded_len(),
            actual: frame.len(),
        })?
        .to_vec();
    super::Request::from_decoded_parts(metadata, value, header.opcode())
}

pub(super) fn decode_owned_request(
    mut frame: Vec<u8>,
    header: RequestHeader,
) -> Result<super::Request> {
    let metadata = decode_request_metadata(&frame, header)?;
    let value = if header.value_len() == 0 {
        Vec::new()
    } else {
        frame.split_off(header.encoded_len())
    };
    super::Request::from_decoded_parts(metadata, value, header.opcode())
}

fn decode_request_metadata(frame: &[u8], header: RequestHeader) -> Result<DecodedRequestMetadata> {
    let opcode = header.opcode();
    let plan = contract::request_wire_plan(opcode).ok_or(ProtocolError::InvalidFieldSequence(
        "operation has no compact request plan",
    ))?;
    let prefix = frame
        .get(..header.encoded_len())
        .ok_or(ProtocolError::FrameTooShort {
            expected: header.encoded_len(),
            actual: frame.len(),
        })?;
    let payload = frame
        .get(header.encoded_len()..)
        .ok_or(ProtocolError::FrameTooShort {
            expected: header.encoded_len(),
            actual: frame.len(),
        })?;
    let fields = openkache_protocol::decode_request_wire_fields(
        OwnedRange::whole(prefix.to_vec()),
        OwnedRange::whole(payload.to_vec()),
        plan,
    )?;
    let namespace_id = field_u64(opcode, &fields, "namespace_id", 0)?;
    let item_ids = field_values(opcode, &fields, "item_id")
        .into_iter()
        .map(|value| {
            let bytes: [u8; openkache_protocol::ITEM_ID_BYTES] =
                value
                    .try_into()
                    .map_err(|_| ProtocolError::InvalidItemIdLength {
                        opcode,
                        expected: openkache_protocol::ITEM_ID_BYTES,
                        actual: value.len(),
                    })?;
            Ok(ItemId::new(bytes))
        })
        .collect::<Result<Vec<_>>>()?;
    let namespace_name = field_bytes(opcode, &fields, "name", 0).map(ToOwned::to_owned);
    let expected_revision = field_u64(opcode, &fields, "expected_revision", 0)?;
    let create_if_missing = field_bytes(opcode, &fields, "create_if_missing", 0)
        .map(decode_bool)
        .transpose()?
        .unwrap_or(false);
    let namespace_policy = decode_policy(opcode, &fields)?;
    let set_options = decode_set_options(opcode, &fields)?;
    Ok(DecodedRequestMetadata {
        namespace_id,
        item_ids,
        set_options,
        namespace_name,
        namespace_policy,
        expected_revision,
        create_if_missing,
    })
}

fn decode_bool(value: &[u8]) -> Result<bool> {
    match value {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(ProtocolError::InvalidFieldSequence(
            "compact boolean field is not canonical",
        )),
    }
}

fn decode_set_options(opcode: Opcode, fields: &[Option<OwnedRange>]) -> Result<SetOptions> {
    if field_count(opcode, "condition") == 0 {
        return Ok(SetOptions::NONE);
    }
    let condition = match field_bytes(opcode, fields, "condition", 0) {
        Some(b"any") => super::SetCondition::Any,
        Some(b"if_absent") => super::SetCondition::IfAbsent,
        Some(b"if_present") => super::SetCondition::IfPresent,
        _ => return Err(ProtocolError::ConflictingSetConditions),
    };
    let expiration_mode = match field_bytes(opcode, fields, "expiration_mode", 0) {
        Some(b"inherit") => super::ExpirationMode::Inherit,
        Some(b"no_expiry") => super::ExpirationMode::NoExpiry,
        Some(b"explicit_ttl") => super::ExpirationMode::ExplicitTtl,
        _ => return Err(ProtocolError::InvalidSetOptions { opcode }),
    };
    let eviction_mode = match field_bytes(opcode, fields, "eviction_mode", 0) {
        Some(b"inherit") => super::EvictionMode::Inherit,
        Some(b"evictable") => super::EvictionMode::Evictable,
        Some(b"eviction_protected") => super::EvictionMode::EvictionProtected,
        _ => return Err(ProtocolError::InvalidSetOptions { opcode }),
    };
    let ttl_ms = field_u64(opcode, fields, "ttl_milliseconds", 0)?;
    let options = SetOptions::with_policies(condition, expiration_mode, ttl_ms, eviction_mode);
    options.flags()?;
    Ok(options)
}

fn decode_policy(opcode: Opcode, fields: &[Option<OwnedRange>]) -> Result<Option<NamespacePolicy>> {
    if field_count(opcode, "policy") == 0 {
        return Ok(None);
    }
    let Some(default_expiration) = field_bytes(opcode, fields, "default_expiration", 0) else {
        return Ok(None);
    };
    let default_expiration = match default_expiration {
        b"no_expiry" => super::ExpirationDefault::NoExpiry,
        b"fixed_ttl" => super::ExpirationDefault::FixedTtl {
            ttl_ms: field_u64(opcode, fields, "default_ttl_milliseconds", 0)?
                .ok_or(ProtocolError::MissingNamespacePolicy)?,
        },
        _ => {
            return Err(ProtocolError::InvalidNamespacePolicy(
                "unknown default expiration",
            ));
        }
    };
    let expiration_override =
        decode_override(field_bytes(opcode, fields, "expiration_override", 0))?;
    let default_eviction = match field_bytes(opcode, fields, "default_eviction", 0) {
        Some(b"evictable") => super::EvictionDefault::Evictable,
        Some(b"eviction_protected") => super::EvictionDefault::EvictionProtected,
        _ => {
            return Err(ProtocolError::InvalidNamespacePolicy(
                "unknown default eviction",
            ));
        }
    };
    let eviction_override = decode_override(field_bytes(opcode, fields, "eviction_override", 0))?;
    let policy = NamespacePolicy {
        default_expiration,
        expiration_override,
        default_eviction,
        eviction_override,
    };
    policy.encode()?;
    Ok(Some(policy))
}

fn decode_override(value: Option<&[u8]>) -> Result<super::OverridePolicy> {
    match value {
        Some(b"allowed") => Ok(super::OverridePolicy::Allowed),
        Some(b"disallowed") => Ok(super::OverridePolicy::Disallowed),
        _ => Err(ProtocolError::InvalidNamespacePolicy(
            "unknown override policy",
        )),
    }
}
