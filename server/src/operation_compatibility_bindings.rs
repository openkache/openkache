//! API-owned decode/encode trampolines for namespace/item compatibility behavior.
//!
//! The shared dispatcher supplies a generated field view. This module turns
//! protocol-v1 namespace and SET projections into the typed values expected by
//! the storage behavior. Generic field-envelope examples live in
//! [`super::operation_generic_bindings`] and do not depend on these adapters.

use openkache_protocol::{ItemId, Opcode, OwnedRange};

use super::operation_api::{
    ApiModule, PrepareContext, PrepareError, PreparePlan, RegistrationBuilder, ResourceLock,
};
use super::operation_compatibility_behavior as compatibility_behavior;
use super::operation_compatibility_services::{
    COMPATIBILITY_RESOURCE_RESOLVER, COMPATIBILITY_SERVICES, CompatibilityResourceResolver,
    CompatibilityServices,
};
use super::operation_contract::OperationStatus;
use super::operation_handlers::{self, OperationContext, OperationInputView};
use super::operation_outcome::OperationOutcome;
use super::operation_registry::OperationFuture;
// This file is included below `server`, so the contract facade lives two
// module levels above it. Keeping that boundary explicit also lets the
// private source-copy test crate use the same path without a composition-root
// re-export.
use super::super::operation_compatibility_contract as contract;
use crate::protocol::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, NamespacePolicy,
    OverridePolicy, SetCondition, SetOptions,
};
use contract::request_fields;

const INVALID_NAMESPACE_ID: &[u8] = b"namespace identity must be nonzero";
const INVALID_EXPECTED_REVISION: &[u8] = b"expected revision must be nonzero";
const INVALID_SET_TTL: &[u8] = b"SET explicit TTL must be positive";
const INVALID_NAMESPACE_NAME: &[u8] = b"namespace-open name is not UTF-8";

/// Installs the compatibility adapter's concrete service bundle at the API
/// composition boundary. The network loop only calls the aggregate operation
/// registration installer and never reaches into this compatibility surface.
pub(super) use super::operation_compatibility_services::install_compatibility_services;

impl OperationInputView {
    fn item_id_at_index(&self, index: usize) -> Option<ItemId> {
        match self.field_at_index(index) {
            Some(value) if value.len() == openkache_protocol::ITEM_ID_BYTES => Some(ItemId::new(
                value.try_into().expect("validated item ID width"),
            )),
            _ => None,
        }
    }

    fn unsigned_long_at_index(&self, index: usize) -> Option<u64> {
        self.encoded_field_at_index(index)
            .and_then(|field| field.decode_u64().ok())
    }

    fn unsigned_long_at_index_result(
        &self,
        index: Option<usize>,
    ) -> Result<Option<u64>, &'static [u8]> {
        index
            .and_then(|index| self.encoded_field_at_index(index))
            .map(|field| field.decode_u64())
            .transpose()
    }

    fn boolean_at_index(&self, index: Option<usize>) -> Result<Option<bool>, &'static [u8]> {
        index
            .and_then(|index| self.encoded_field_at_index(index))
            .map(|field| field.decode_bool())
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

/// Typed input for namespace-open.
pub(super) struct NamespaceOpenInput {
    pub(super) name: OwnedRange,
    pub(super) create_if_missing: bool,
    pub(super) policy: Option<NamespacePolicy>,
}

/// Typed input for namespace policy updates and deletes.
pub(super) struct NamespaceRevisionInput {
    pub(super) namespace_id: u64,
    pub(super) expected_revision: u64,
    pub(super) policy: NamespacePolicy,
}

/// Typed input for namespace deletion. Deletion uses the revision as a
/// concurrency check but does not carry a replacement policy.
pub(super) struct NamespaceDeleteInput {
    pub(super) namespace_id: u64,
    pub(super) expected_revision: u64,
}

struct SetFields {
    namespace_id: u64,
    item_id: ItemId,
    options: SetOptions,
}

struct NamespaceOpenFields {
    create_if_missing: bool,
    policy: Option<NamespacePolicy>,
}

fn required_namespace_id(
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

fn required_expected_revision(
    input: &OperationInputView,
    field_index: usize,
) -> Result<u64, &'static [u8]> {
    required_positive_unsigned(
        input,
        field_index,
        b"operation requires an expected revision",
        INVALID_EXPECTED_REVISION,
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
        namespace_id: required_namespace_id(input, request_fields::GET_NAMESPACE_ID_0)?,
        item_id: required_item_id_at(input, request_fields::GET_ITEM_ID_0)?,
    })
}

fn decode_set_fields(input: &OperationInputView) -> Result<SetFields, &'static [u8]> {
    let condition = decode_set_condition(input, request_fields::SET_CONDITION_0)?;
    let expiration_mode = decode_expiration_mode(input, request_fields::SET_EXPIRATION_MODE_0)?;
    let eviction_mode = decode_eviction_mode(input, request_fields::SET_EVICTION_MODE_0)?;
    let ttl_ms = input
        .unsigned_long_at_index_result(Some(request_fields::SET_TTL_MILLISECONDS_0))
        .map_err(|_| &b"SET TTL is malformed"[..])?;
    validate_set_ttl_value(expiration_mode == ExpirationMode::ExplicitTtl, ttl_ms)?;
    let namespace_id = required_namespace_id(input, request_fields::SET_NAMESPACE_ID_0)?;
    let item_id = required_item_id_at(input, request_fields::SET_ITEM_ID_0)?;
    input
        .bytes_at_index(request_fields::SET_VALUE_0)
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
        .take_owned_bytes_range_at_index(request_fields::SET_VALUE_0)
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
        namespace_id: required_namespace_id(input, request_fields::DELETE_NAMESPACE_ID_0)?,
        item_id: required_item_id_at(input, request_fields::DELETE_ITEM_ID_0)?,
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

fn decode_stats(input: &OperationInputView) -> Result<NamespaceInput, &'static [u8]> {
    decode_namespace(input, request_fields::STATS_NAMESPACE_ID_0)
}

fn decode_sync(input: &OperationInputView) -> Result<NamespaceInput, &'static [u8]> {
    decode_namespace(input, request_fields::SYNC_NAMESPACE_ID_0)
}

pub(super) fn decode_namespace_open(
    input: &mut OperationInputView,
) -> Result<NamespaceOpenInput, &'static [u8]> {
    let NamespaceOpenFields {
        create_if_missing,
        policy,
    } = decode_namespace_open_fields(input)?;
    let name = input
        .take_owned_bytes_range_at_index(request_fields::NAMESPACE_OPEN_NAME_0)
        .ok_or(&b"namespace-open requires a name"[..])?;
    Ok(NamespaceOpenInput {
        name,
        create_if_missing,
        policy,
    })
}

fn decode_namespace_open_fields(
    input: &OperationInputView,
) -> Result<NamespaceOpenFields, &'static [u8]> {
    validate_namespace_open_name(input)?;
    let create_if_missing = input
        .boolean_at_index(Some(request_fields::NAMESPACE_OPEN_CREATE_IF_MISSING_0))
        .map_err(|_| &b"namespace-open create flag is malformed"[..])?
        .unwrap_or(false);
    let policy = decode_namespace_open_policy(input)?;
    Ok(NamespaceOpenFields {
        create_if_missing,
        policy,
    })
}

pub(super) fn decode_namespace_revision(
    input: &OperationInputView,
) -> Result<NamespaceRevisionInput, &'static [u8]> {
    Ok(NamespaceRevisionInput {
        namespace_id: required_namespace_id(
            input,
            request_fields::NAMESPACE_UPDATE_POLICY_NAMESPACE_ID_0,
        )?,
        expected_revision: required_expected_revision(
            input,
            request_fields::NAMESPACE_UPDATE_POLICY_EXPECTED_REVISION_0,
        )?,
        policy: decode_namespace_update_policy(input)?
            .ok_or(&b"namespace policy is required"[..])?,
    })
}

pub(super) fn decode_namespace_delete(
    input: &OperationInputView,
) -> Result<NamespaceDeleteInput, &'static [u8]> {
    Ok(NamespaceDeleteInput {
        namespace_id: required_namespace_id(
            input,
            request_fields::NAMESPACE_DELETE_NAMESPACE_ID_0,
        )?,
        expected_revision: required_expected_revision(
            input,
            request_fields::NAMESPACE_DELETE_EXPECTED_REVISION_0,
        )?,
    })
}

fn namespace_resource(
    namespace_id: u64,
    context: PrepareContext<'_>,
) -> std::result::Result<ResourceLock, PrepareError> {
    compatibility_resolver(context)?.resolve_namespace(&namespace_id.to_be_bytes())
}

fn compatibility_resolver<'a>(
    context: PrepareContext<'a>,
) -> std::result::Result<&'a CompatibilityResourceResolver, PrepareError> {
    context
        .capability::<CompatibilityResourceResolver>(COMPATIBILITY_RESOURCE_RESOLVER)
        .ok_or(PrepareError::resource_unavailable(
            OperationStatus::InternalError,
            b"compatibility resource resolver is unavailable",
        ))
}

fn compatibility_services<'a>(
    context: &OperationContext<'a>,
) -> Option<&'a (dyn CompatibilityServices + Send + Sync)> {
    context
        .capability(COMPATIBILITY_SERVICES)
        .map(std::sync::Arc::as_ref)
}

/// Computes an opaque resource handle from the typed namespace identity used
/// by the API binding. The dispatcher never infers this identity from fields.
pub(super) fn prepare_namespace(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> std::result::Result<PreparePlan, PrepareError> {
    let namespace_field_index = namespace_field_index(input.opcode)
        .ok_or_else(|| PrepareError::invalid_request(b"operation has no namespace identity"))?;
    let namespace_id = required_namespace_id(input, namespace_field_index)
        .map_err(PrepareError::invalid_request)?;
    let resource = namespace_resource(namespace_id, context)?;
    Ok(PreparePlan::resource(resource))
}

pub(super) fn prepare_set(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> std::result::Result<PreparePlan, PrepareError> {
    let namespace_id = required_namespace_id(input, request_fields::SET_NAMESPACE_ID_0)
        .map_err(PrepareError::invalid_request)?;
    validate_set_ttl(input).map_err(PrepareError::invalid_request)?;
    Ok(PreparePlan::resource(namespace_resource(
        namespace_id,
        context,
    )?))
}

fn namespace_field_index(opcode: openkache_protocol::Opcode) -> Option<usize> {
    contract::operation_field_index(
        opcode,
        contract::OperationFieldDirection::Request,
        contract::OperationFieldRole::NamespaceId,
        0,
    )
}

pub(super) fn prepare_lifecycle(
    _input: &OperationInputView,
    context: PrepareContext<'_>,
) -> std::result::Result<PreparePlan, PrepareError> {
    Ok(PreparePlan::resource(
        compatibility_resolver(context)?.resolve_global()?,
    ))
}

pub(super) fn prepare_namespace_open(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> std::result::Result<PreparePlan, PrepareError> {
    validate_namespace_open_name(input).map_err(PrepareError::invalid_request)?;
    validate_namespace_policy_ttl(input, NAMESPACE_OPEN_POLICY_FIELDS)
        .map_err(PrepareError::invalid_request)?;
    prepare_lifecycle(input, context)
}

pub(super) fn prepare_namespace_update(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> std::result::Result<PreparePlan, PrepareError> {
    let namespace_id = required_namespace_id(
        input,
        request_fields::NAMESPACE_UPDATE_POLICY_NAMESPACE_ID_0,
    )
    .map_err(PrepareError::invalid_request)?;
    required_expected_revision(
        input,
        request_fields::NAMESPACE_UPDATE_POLICY_EXPECTED_REVISION_0,
    )
    .map_err(PrepareError::invalid_request)?;
    validate_namespace_policy_ttl(input, NAMESPACE_UPDATE_POLICY_FIELDS)
        .map_err(PrepareError::invalid_request)?;
    Ok(PreparePlan::resource(namespace_resource(
        namespace_id,
        context,
    )?))
}

pub(super) fn prepare_namespace_delete(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> std::result::Result<PreparePlan, PrepareError> {
    let namespace_id =
        required_namespace_id(input, request_fields::NAMESPACE_DELETE_NAMESPACE_ID_0)
            .map_err(PrepareError::invalid_request)?;
    required_expected_revision(
        input,
        request_fields::NAMESPACE_DELETE_EXPECTED_REVISION_0,
    )
    .map_err(PrepareError::invalid_request)?;
    let resolver = compatibility_resolver(context)?;
    let resource = resolver.resolve_namespace(&namespace_id.to_be_bytes())?;
    Ok(PreparePlan::from_resources([
        resolver.resolve_global()?,
        resource,
    ]))
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

#[derive(Clone, Copy)]
struct NamespacePolicyFieldIndexes {
    default_expiration: usize,
    default_ttl_milliseconds: usize,
    expiration_override: usize,
    default_eviction: usize,
    eviction_override: usize,
}

const NAMESPACE_OPEN_POLICY_FIELDS: NamespacePolicyFieldIndexes = NamespacePolicyFieldIndexes {
    default_expiration: request_fields::NAMESPACE_OPEN_DEFAULT_EXPIRATION_0,
    default_ttl_milliseconds: request_fields::NAMESPACE_OPEN_DEFAULT_TTL_MILLISECONDS_0,
    expiration_override: request_fields::NAMESPACE_OPEN_EXPIRATION_OVERRIDE_0,
    default_eviction: request_fields::NAMESPACE_OPEN_DEFAULT_EVICTION_0,
    eviction_override: request_fields::NAMESPACE_OPEN_EVICTION_OVERRIDE_0,
};

const NAMESPACE_UPDATE_POLICY_FIELDS: NamespacePolicyFieldIndexes = NamespacePolicyFieldIndexes {
    default_expiration: request_fields::NAMESPACE_UPDATE_POLICY_DEFAULT_EXPIRATION_0,
    default_ttl_milliseconds: request_fields::NAMESPACE_UPDATE_POLICY_DEFAULT_TTL_MILLISECONDS_0,
    expiration_override: request_fields::NAMESPACE_UPDATE_POLICY_EXPIRATION_OVERRIDE_0,
    default_eviction: request_fields::NAMESPACE_UPDATE_POLICY_DEFAULT_EVICTION_0,
    eviction_override: request_fields::NAMESPACE_UPDATE_POLICY_EVICTION_OVERRIDE_0,
};

fn decode_namespace_open_policy(
    input: &OperationInputView,
) -> Result<Option<NamespacePolicy>, &'static [u8]> {
    decode_namespace_policy(input, NAMESPACE_OPEN_POLICY_FIELDS)
}

fn decode_namespace_update_policy(
    input: &OperationInputView,
) -> Result<Option<NamespacePolicy>, &'static [u8]> {
    decode_namespace_policy(input, NAMESPACE_UPDATE_POLICY_FIELDS)
}

fn validate_set_ttl(input: &OperationInputView) -> Result<(), &'static [u8]> {
    let expiration_mode = required_token(
        input,
        request_fields::SET_EXPIRATION_MODE_0,
        b"SET expiration mode is missing",
    )?;
    let ttl_ms = input
        .unsigned_long_at_index_result(Some(request_fields::SET_TTL_MILLISECONDS_0))
        .map_err(|_| &b"SET TTL is malformed"[..])?;
    validate_set_ttl_value(expiration_mode == b"explicit_ttl", ttl_ms)
}

fn validate_namespace_open_name(input: &OperationInputView) -> Result<(), &'static [u8]> {
    let name = input
        .bytes_at_index(request_fields::NAMESPACE_OPEN_NAME_0)
        .ok_or(&b"namespace-open requires a name"[..])?;
    std::str::from_utf8(name)
        .map(|_| ())
        .map_err(|_| INVALID_NAMESPACE_NAME)
}

fn validate_set_ttl_value(
    explicit_ttl: bool,
    ttl_ms: Option<u64>,
) -> Result<(), &'static [u8]> {
    match (explicit_ttl, ttl_ms) {
        (true, Some(0)) => Err(INVALID_SET_TTL),
        (true, None) => Err(b"SET explicit TTL is missing"),
        (true, Some(_)) | (false, None) => Ok(()),
        (false, Some(_)) => Err(b"SET TTL is only valid with explicit expiration"),
    }
}

fn validate_namespace_policy_ttl(
    input: &OperationInputView,
    fields: NamespacePolicyFieldIndexes,
) -> Result<(), &'static [u8]> {
    let default_expiration = input.bytes_at_index(fields.default_expiration);
    let ttl_ms = input
        .unsigned_long_at_index_result(Some(fields.default_ttl_milliseconds))
        .map_err(|_| &b"namespace TTL is malformed"[..])?;
    validate_namespace_policy_ttl_value(default_expiration, ttl_ms)
}

fn validate_namespace_policy_ttl_value(
    default_expiration: Option<&[u8]>,
    ttl_ms: Option<u64>,
) -> Result<(), &'static [u8]> {
    match (default_expiration, ttl_ms) {
        (Some(b"fixed_ttl"), Some(0)) => Err(b"fixed namespace TTL must be positive"),
        (Some(b"fixed_ttl"), None) => Err(b"fixed namespace TTL is missing"),
        (Some(b"fixed_ttl"), Some(_)) | (_, None) => Ok(()),
        (Some(b"no_expiry"), Some(_)) => Err(b"namespace TTL is invalid for no-expiry policy"),
        (None, Some(_)) => Err(b"namespace policy is incomplete"),
        (Some(_), Some(_)) => Ok(()),
    }
}

fn decode_namespace_policy(
    input: &OperationInputView,
    fields: NamespacePolicyFieldIndexes,
) -> Result<Option<NamespacePolicy>, &'static [u8]> {
    let has_nested_value = input.bytes_at_index(fields.default_expiration).is_some()
        || input.bytes_at_index(fields.expiration_override).is_some()
        || input.bytes_at_index(fields.default_eviction).is_some()
        || input.bytes_at_index(fields.eviction_override).is_some()
        || input
            .unsigned_long_at_index(fields.default_ttl_milliseconds)
            .is_some();
    let Some(default_expiration) = input.bytes_at_index(fields.default_expiration) else {
        if has_nested_value {
            return Err(b"namespace policy is incomplete");
        }
        return Ok(None);
    };
    let default_ttl_milliseconds =
        input.unsigned_long_at_index(fields.default_ttl_milliseconds);
    validate_namespace_policy_ttl_value(Some(default_expiration), default_ttl_milliseconds)?;
    let default_expiration = match default_expiration {
        b"no_expiry" => ExpirationDefault::NoExpiry,
        b"fixed_ttl" => {
            let ttl_ms = default_ttl_milliseconds.expect("validated fixed namespace TTL");
            ExpirationDefault::FixedTtl { ttl_ms }
        }
        _ => return Err(b"namespace expiration default is malformed"),
    };
    let expiration_override = decode_override_policy(input, fields.expiration_override)?;
    let default_eviction = match required_token(
        input,
        fields.default_eviction,
        b"namespace eviction default is missing",
    )? {
        b"evictable" => EvictionDefault::Evictable,
        b"eviction_protected" => EvictionDefault::EvictionProtected,
        _ => return Err(b"namespace eviction default is malformed"),
    };
    let eviction_override = decode_override_policy(input, fields.eviction_override)?;
    Ok(Some(NamespacePolicy {
        default_expiration,
        expiration_override,
        default_eviction,
        eviction_override,
    }))
}

fn decode_override_policy(
    input: &OperationInputView,
    field_index: usize,
) -> Result<OverridePolicy, &'static [u8]> {
    match required_token(input, field_index, b"namespace override policy is missing")? {
        b"allowed" => Ok(OverridePolicy::Allowed),
        b"disallowed" => Ok(OverridePolicy::Disallowed),
        _ => Err(b"namespace override policy is malformed"),
    }
}

fn invalid_input<'a>(message: &'static [u8]) -> OperationFuture<'a> {
    OperationFuture::ready(OperationOutcome::invalid_request(message))
}

/// Builds a typed API binding from a generated field decoder and an
/// API-owned behavior function.
///
/// Every compatibility operation follows the same boundary: decode the
/// generated view, map malformed domain input to a transport-neutral error,
/// then hand the typed value to behavior. The macro keeps that plumbing in one
/// place while leaving the decoder and behavior names API-owned.
macro_rules! typed_handler {
    ($name:ident, mut $decode:ident, $behavior:path) => {
        pub(super) fn $name<'a>(context: OperationContext<'a>) -> OperationFuture<'a> {
            let Some(services) = compatibility_services(&context) else {
                return invalid_input(b"compatibility services are unavailable");
            };
            let OperationContext { mut input, .. } = context;
            let decoded = match $decode(&mut input) {
                Ok(input) => input,
                Err(message) => return invalid_input(message),
            };
            OperationFuture::pending($behavior(Some(services), decoded))
        }
    };
    ($name:ident, $decode:path, $behavior:path) => {
        pub(super) fn $name<'a>(context: OperationContext<'a>) -> OperationFuture<'a> {
            let Some(services) = compatibility_services(&context) else {
                return invalid_input(b"compatibility services are unavailable");
            };
            let OperationContext { input, .. } = context;
            let decoded = match $decode(&input) {
                Ok(input) => input,
                Err(message) => return invalid_input(message),
            };
            OperationFuture::pending($behavior(Some(services), decoded))
        }
    };
}

typed_handler!(get_handler, decode_get, compatibility_behavior::get);
typed_handler!(
    namespace_open_handler,
    mut decode_namespace_open,
    compatibility_behavior::namespace_open
);
typed_handler!(
    namespace_update_policy_handler,
    decode_namespace_revision,
    compatibility_behavior::namespace_update_policy
);
typed_handler!(
    namespace_delete_handler,
    decode_namespace_delete,
    compatibility_behavior::namespace_delete
);
typed_handler!(set_handler, mut decode_set, compatibility_behavior::set);
typed_handler!(
    delete_handler,
    decode_delete,
    compatibility_behavior::delete
);
typed_handler!(stats_handler, decode_stats, compatibility_behavior::stats);
typed_handler!(sync_handler, decode_sync, compatibility_behavior::sync);

fn install_capabilities(
    registry: &mut super::operation_capabilities::CapabilityRegistry,
    bootstrap: &dyn super::operation_capabilities::CapabilityCatalog,
) -> Result<(), &'static str> {
    let resources = super::operation_api::downcast_capability(
        bootstrap,
        super::operation_runtime_capabilities::SERVER_RUNTIME_RESOURCES,
    )
    .ok_or("server runtime bootstrap capability is unavailable")?;
    install_compatibility_services(
        registry,
        resources.cache.clone(),
        resources.namespaces.clone(),
        resources.observability.clone(),
    );
    Ok(())
}

pub(super) const API: ApiModule = ApiModule::new(
    crate::protocol::compatibility_request_descriptor(),
    &[
        RegistrationBuilder::new(Opcode::Get, get_handler)
        .prepare(prepare_namespace)
        .authorize(operation_handlers::authorization_none)
        .read_only()
        .build(),
        RegistrationBuilder::new(Opcode::Set, set_handler)
        .prepare(prepare_set)
        .authorize(operation_handlers::authorization_none)
        .mutation()
        .build(),
        RegistrationBuilder::new(Opcode::Delete, delete_handler)
        .prepare(prepare_namespace)
        .authorize(operation_handlers::authorization_none)
        .mutation()
        .build(),
        RegistrationBuilder::new(Opcode::Stats, stats_handler)
        .prepare(prepare_namespace)
        .authorize(operation_handlers::authorization_administrator)
        .read_only()
        .build(),
        RegistrationBuilder::new(Opcode::Sync, sync_handler)
        .prepare(prepare_namespace)
        .authorize(operation_handlers::authorization_administrator)
        .mutation()
        .build(),
        RegistrationBuilder::new(Opcode::NamespaceOpen, namespace_open_handler)
        .prepare(prepare_namespace_open)
        .authorize(operation_handlers::authorization_none)
        .mutation()
        .build(),
        RegistrationBuilder::new(Opcode::NamespaceUpdatePolicy, namespace_update_policy_handler)
        .prepare(prepare_namespace_update)
        .authorize(operation_handlers::authorization_none)
        .mutation()
        .build(),
        RegistrationBuilder::new(Opcode::NamespaceDelete, namespace_delete_handler)
        .prepare(prepare_namespace_delete)
        .authorize(operation_handlers::authorization_none)
        .mutation()
        .build(),
    ],
)
.install_capabilities(install_capabilities);
