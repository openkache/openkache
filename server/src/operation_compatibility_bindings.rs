//! API-owned decode/encode trampolines for namespace/item compatibility behavior.
//!
//! The shared dispatcher supplies a generated field view. This module turns
//! protocol-v1 namespace and SET projections into the typed values expected by
//! the storage behavior. Generic field-envelope examples live in
//! [`super::operation_generic_bindings`] and do not depend on these adapters.

use openkache_protocol::{ItemId, Opcode};

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

/// Installs the compatibility adapter's concrete service bundle at the API
/// composition boundary. The network loop only calls the aggregate operation
/// registration installer and never reaches into this compatibility surface.
pub(super) use super::operation_compatibility_services::install_compatibility_services;

/// Typed input for the namespace/item compatibility adapter.
pub(super) struct GetInput {
    pub(super) namespace_id: u64,
    pub(super) item_id: ItemId,
}

/// Typed input for the namespace/item mutation adapter.
pub(super) struct SetInput {
    pub(super) namespace_id: u64,
    pub(super) item_id: ItemId,
    pub(super) value: Vec<u8>,
    pub(super) options: SetOptions,
}

/// Typed input for namespace-scoped operations.
pub(super) struct NamespaceInput {
    pub(super) namespace_id: u64,
}

/// Typed input for namespace-open.
pub(super) struct NamespaceOpenInput {
    pub(super) name: Vec<u8>,
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

fn required_namespace_id(
    input: &OperationInputView,
    field_index: usize,
) -> Result<u64, &'static [u8]> {
    input
        .unsigned_long_at_index(field_index)
        .ok_or(b"operation requires namespace identity")
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

pub(super) fn decode_set(input: &mut OperationInputView) -> Result<SetInput, &'static [u8]> {
    let condition = decode_set_condition(input, request_fields::SET_CONDITION_0)?;
    let expiration_mode = decode_expiration_mode(input, request_fields::SET_EXPIRATION_MODE_0)?;
    let eviction_mode = decode_eviction_mode(input, request_fields::SET_EVICTION_MODE_0)?;
    let ttl_ms = input
        .unsigned_long_at_index_result(Some(request_fields::SET_TTL_MILLISECONDS_0))
        .map_err(|_| &b"SET TTL is malformed"[..])?;
    if expiration_mode == ExpirationMode::ExplicitTtl {
        if ttl_ms.is_none() {
            return Err(b"SET explicit TTL is missing");
        }
    } else if ttl_ms.is_some() {
        return Err(b"SET TTL is only valid with explicit expiration");
    }
    Ok(SetInput {
        namespace_id: required_namespace_id(input, request_fields::SET_NAMESPACE_ID_0)?,
        item_id: required_item_id_at(input, request_fields::SET_ITEM_ID_0)?,
        value: input
            .take_owned_bytes_at_index(request_fields::SET_VALUE_0)
            .ok_or(&b"operation requires a value"[..])?,
        options: SetOptions::with_policies(condition, expiration_mode, ttl_ms, eviction_mode),
    })
}

pub(super) fn decode_delete(input: &OperationInputView) -> Result<GetInput, &'static [u8]> {
    decode_get(input)
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
    Ok(NamespaceOpenInput {
        name: input
            .take_owned_bytes_at_index(request_fields::NAMESPACE_OPEN_NAME_0)
            .ok_or(&b"namespace-open requires a name"[..])?,
        create_if_missing: input
            .boolean_at_index(Some(request_fields::NAMESPACE_OPEN_CREATE_IF_MISSING_0))
            .map_err(|_| &b"namespace-open create flag is malformed"[..])?
            .unwrap_or(false),
        policy: decode_namespace_open_policy(input)?,
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
        expected_revision: input
            .unsigned_long_at_index(request_fields::NAMESPACE_UPDATE_POLICY_EXPECTED_REVISION_0)
            .ok_or(&b"operation requires an expected revision"[..])?,
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
        expected_revision: input
            .unsigned_long_at_index(request_fields::NAMESPACE_DELETE_EXPECTED_REVISION_0)
            .ok_or(&b"operation requires an expected revision"[..])?,
    })
}

fn required_resource(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> std::result::Result<ResourceLock, PrepareError> {
    let namespace_field_index = namespace_field_index(input.opcode)
        .ok_or_else(|| PrepareError::invalid_request(b"operation has no namespace identity"))?;
    let namespace_id = required_namespace_id(input, namespace_field_index)
        .map_err(PrepareError::invalid_request)?;
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
    let resource = required_resource(input, context)?;
    Ok(PreparePlan::resource(resource))
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

pub(super) fn prepare_lifecycle_and_namespace(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> std::result::Result<PreparePlan, PrepareError> {
    let namespace_field_index = namespace_field_index(input.opcode)
        .ok_or_else(|| PrepareError::invalid_request(b"operation has no namespace identity"))?;
    let namespace_id = required_namespace_id(input, namespace_field_index)
        .map_err(PrepareError::invalid_request)?;
    let resource =
        compatibility_resolver(context)?.resolve_namespace(&namespace_id.to_be_bytes())?;
    Ok(PreparePlan::from_resources([
        compatibility_resolver(context)?.resolve_global()?,
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
    let default_expiration = match default_expiration {
        b"no_expiry" => {
            if input
                .unsigned_long_at_index(fields.default_ttl_milliseconds)
                .is_some()
            {
                return Err(b"namespace TTL is invalid for no-expiry policy");
            }
            ExpirationDefault::NoExpiry
        }
        b"fixed_ttl" => {
            let Some(ttl_ms) = input.unsigned_long_at_index(fields.default_ttl_milliseconds) else {
                return Err(b"fixed namespace TTL is missing");
            };
            if ttl_ms == 0 {
                return Err(b"fixed namespace TTL must be positive");
            }
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

/// Returns whether this API module owns the historical v1 projection for an
/// opcode. The compatibility validator uses this module-local catalog rather
/// than placing a compatibility marker in every generic server registration.
pub(super) fn handles(opcode: Opcode) -> bool {
    API.operations()
        .iter()
        .any(|registration| registration.opcode == opcode)
}

pub(super) const API: ApiModule = ApiModule::new(
    crate::protocol::compatibility_request_descriptor(),
    &[
    RegistrationBuilder::with_decoder(
        Opcode::Get,
        super::v1_adapter::adapt_request,
        get_handler,
    )
    .prepare(prepare_namespace)
    .authorize(operation_handlers::authorization_none)
    .read_only()
    .build(),
    RegistrationBuilder::with_decoder(
        Opcode::Set,
        super::v1_adapter::adapt_request,
        set_handler,
    )
    .prepare(prepare_namespace)
    .authorize(operation_handlers::authorization_none)
    .mutation()
    .build(),
    RegistrationBuilder::with_decoder(
        Opcode::Delete,
        super::v1_adapter::adapt_request,
        delete_handler,
    )
    .prepare(prepare_namespace)
    .authorize(operation_handlers::authorization_none)
    .mutation()
    .build(),
    RegistrationBuilder::with_decoder(
        Opcode::Stats,
        super::v1_adapter::adapt_request,
        stats_handler,
    )
    .prepare(prepare_namespace)
    .authorize(operation_handlers::authorization_administrator)
    .read_only()
    .build(),
    RegistrationBuilder::with_decoder(
        Opcode::Sync,
        super::v1_adapter::adapt_request,
        sync_handler,
    )
    .prepare(prepare_namespace)
    .authorize(operation_handlers::authorization_administrator)
    .mutation()
    .build(),
    RegistrationBuilder::with_decoder(
        Opcode::NamespaceOpen,
        super::v1_adapter::adapt_request,
        namespace_open_handler,
    )
    .prepare(prepare_lifecycle)
    .authorize(operation_handlers::authorization_none)
    .mutation()
    .build(),
    RegistrationBuilder::with_decoder(
        Opcode::NamespaceUpdatePolicy,
        super::v1_adapter::adapt_request,
        namespace_update_policy_handler,
    )
    .prepare(prepare_namespace)
    .authorize(operation_handlers::authorization_none)
    .mutation()
    .build(),
    RegistrationBuilder::with_decoder(
        Opcode::NamespaceDelete,
        super::v1_adapter::adapt_request,
        namespace_delete_handler,
    )
    .prepare(prepare_lifecycle_and_namespace)
    .authorize(operation_handlers::authorization_none)
    .mutation()
    .build(),
    ],
);
