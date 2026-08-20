//! API-owned admission and resource preparation for compatibility operations.

use super::operation_compatibility_decode as decode;
use super::operation_compatibility_services::{
    DeleteState, GetState, SetState, StatsState, SyncState,
};
use super::operation_contract::{OperationStatus, request_fields};
use super::operation_handlers::OperationInputView;
use super::operation_ports::NamespaceCoordinationCapability;
use super::operation_preparation::{
    HeaderAdmissionContext, HeaderAdmissionError, OperationHeaderView, PrepareContext,
    PrepareError, PreparePlan, ResourceLock,
};

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
    coordination: &dyn NamespaceCoordinationCapability,
) -> Result<ResourceLock, PrepareError> {
    let resource = coordination.operation_lock(namespace_id).ok_or_else(|| {
        PrepareError::resource_unavailable(
            OperationStatus::NamespaceNotFound,
            b"namespace does not exist",
        )
    })?;
    let (lock, active) = resource.into_parts();
    Ok(ResourceLock::new(
        lock,
        active,
        PrepareError::resource_unavailable(
            OperationStatus::NamespaceNotFound,
            b"namespace does not exist",
        ),
    ))
}

fn operation_state<'a, T: 'static>(context: PrepareContext<'a>) -> Result<&'a T, PrepareError> {
    context
        .state::<T>()
        .ok_or(PrepareError::resource_unavailable(
            OperationStatus::InternalError,
            b"compatibility module state is unavailable",
        ))
}

fn prepare_namespace_at(
    input: &OperationInputView,
    coordination: &dyn NamespaceCoordinationCapability,
    field_index: usize,
) -> Result<PreparePlan, PrepareError> {
    let namespace_id =
        decode::required_namespace_id(input, field_index).map_err(PrepareError::invalid_request)?;
    let resource = namespace_resource(namespace_id, coordination)?;
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
        state.coordination.as_ref(),
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
        state.coordination.as_ref(),
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
        state.coordination.as_ref(),
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
        state.coordination.as_ref(),
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
        state.coordination.as_ref(),
    )?))
}
