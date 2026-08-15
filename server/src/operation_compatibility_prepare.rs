//! API-owned admission and resource preparation for compatibility operations.

use super::operation_api::{
    HeaderAdmissionContext, HeaderAdmissionError, OperationHeaderView, PrepareContext,
    PrepareError, PreparePlan, ResourceLock,
};
use super::operation_compatibility_decode as decode;
use super::operation_compatibility_services::{
    DeleteState, GetState, NamespaceCapability, NamespaceDeleteState, NamespaceOpenState,
    NamespaceUpdateState, SetState, StatsState, SyncState,
};
use super::operation_contract::{OperationStatus, request_fields};
use super::operation_handlers::OperationInputView;

pub(super) fn admit_set_header(
    input: &OperationHeaderView<'_>,
    context: HeaderAdmissionContext<'_>,
) -> Result<(), HeaderAdmissionError> {
    let limits = context.state::<SetState>().ok_or_else(|| {
        HeaderAdmissionError::new(
            OperationStatus::InternalError,
            b"compatibility module state is unavailable",
        )
    })?;
    let value_len = input
        .declared_body_len(request_fields::op_set::VALUE)
        .ok_or_else(|| {
            HeaderAdmissionError::new(
                OperationStatus::InvalidRequest,
                b"SET value declaration is unavailable",
            )
        })?;
    if value_len > limits.max_item_bytes {
        return Err(HeaderAdmissionError::new(
            OperationStatus::TooLarge,
            b"SET value exceeds the configured item limit",
        ));
    }
    Ok(())
}

fn namespace_resource(
    namespace_id: u64,
    namespaces: &dyn NamespaceCapability,
) -> Result<ResourceLock, PrepareError> {
    namespaces.operation_lock(namespace_id).ok_or_else(|| {
        PrepareError::resource_unavailable(
            OperationStatus::NamespaceNotFound,
            b"namespace does not exist",
        )
    })
}

fn operation_state<'a, T: 'static>(context: PrepareContext<'a>) -> Result<&'a T, PrepareError> {
    context
        .state::<T>()
        .ok_or(PrepareError::resource_unavailable(
            OperationStatus::InternalError,
            b"compatibility module state is unavailable",
        ))
}

fn global_resource(namespaces: &dyn NamespaceCapability) -> Result<ResourceLock, PrepareError> {
    let shared = namespaces.lifecycle_lock().map_err(|_| {
        PrepareError::resource_unavailable(
            OperationStatus::InternalError,
            b"namespace metadata is unavailable",
        )
    })?;
    Ok(ResourceLock::unconditional(shared))
}

fn prepare_namespace_at(
    input: &OperationInputView,
    namespaces: &dyn NamespaceCapability,
    field_index: usize,
) -> Result<PreparePlan, PrepareError> {
    let namespace_id =
        decode::required_namespace_id(input, field_index).map_err(PrepareError::invalid_request)?;
    let resource = namespace_resource(namespace_id, namespaces)?;
    Ok(PreparePlan::resource(resource))
}

/// Computes an opaque resource handle from the generated GET namespace field.
pub(super) fn prepare_get_namespace(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> Result<PreparePlan, PrepareError> {
    let state = operation_state::<GetState>(context)?;
    prepare_namespace_at(
        input,
        state.namespaces.as_ref(),
        request_fields::op_get::NAMESPACE_ID,
    )
}

pub(super) fn prepare_delete_namespace(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> Result<PreparePlan, PrepareError> {
    let state = operation_state::<DeleteState>(context)?;
    prepare_namespace_at(
        input,
        state.namespaces.as_ref(),
        request_fields::op_delete::NAMESPACE_ID,
    )
}

pub(super) fn prepare_stats_namespace(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> Result<PreparePlan, PrepareError> {
    let state = operation_state::<StatsState>(context)?;
    prepare_namespace_at(
        input,
        state.namespaces.as_ref(),
        request_fields::op_stats::NAMESPACE_ID,
    )
}

pub(super) fn prepare_sync_namespace(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> Result<PreparePlan, PrepareError> {
    let state = operation_state::<SyncState>(context)?;
    prepare_namespace_at(
        input,
        state.namespaces.as_ref(),
        request_fields::op_sync::NAMESPACE_ID,
    )
}

pub(super) fn prepare_set(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> Result<PreparePlan, PrepareError> {
    let namespace_id = decode::required_namespace_id(input, request_fields::op_set::NAMESPACE_ID)
        .map_err(PrepareError::invalid_request)?;
    decode::validate_set_ttl(input).map_err(PrepareError::invalid_request)?;
    let state = operation_state::<SetState>(context)?;
    Ok(PreparePlan::resource(namespace_resource(
        namespace_id,
        state.namespaces.as_ref(),
    )?))
}

fn prepare_lifecycle(namespaces: &dyn NamespaceCapability) -> Result<PreparePlan, PrepareError> {
    Ok(PreparePlan::resource(global_resource(namespaces)?))
}

pub(super) fn prepare_namespace_open(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> Result<PreparePlan, PrepareError> {
    decode::validate_namespace_open_name(input).map_err(PrepareError::invalid_request)?;
    decode::validate_namespace_open_policy_ttl(input).map_err(PrepareError::invalid_request)?;
    let state = operation_state::<NamespaceOpenState>(context)?;
    prepare_lifecycle(state.namespaces.as_ref())
}

pub(super) fn prepare_namespace_update(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> Result<PreparePlan, PrepareError> {
    let namespace_id = decode::required_namespace_id(
        input,
        request_fields::op_namespace_update_policy::NAMESPACE_ID,
    )
    .map_err(PrepareError::invalid_request)?;
    decode::required_expected_revision(
        input,
        request_fields::op_namespace_update_policy::EXPECTED_REVISION,
    )
    .map_err(PrepareError::invalid_request)?;
    decode::validate_namespace_update_policy_ttl(input).map_err(PrepareError::invalid_request)?;
    let state = operation_state::<NamespaceUpdateState>(context)?;
    Ok(PreparePlan::resource(namespace_resource(
        namespace_id,
        state.namespaces.as_ref(),
    )?))
}

pub(super) fn prepare_namespace_delete(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> Result<PreparePlan, PrepareError> {
    let namespace_id =
        decode::required_namespace_id(input, request_fields::op_namespace_delete::NAMESPACE_ID)
            .map_err(PrepareError::invalid_request)?;
    decode::required_expected_revision(
        input,
        request_fields::op_namespace_delete::EXPECTED_REVISION,
    )
    .map_err(PrepareError::invalid_request)?;
    let state = operation_state::<NamespaceDeleteState>(context)?;
    let namespaces = state.namespaces.as_ref();
    let resource = namespace_resource(namespace_id, namespaces)?;
    Ok(PreparePlan::from_resources([
        global_resource(namespaces)?,
        resource,
    ]))
}
