//! Generated-field bindings for the route-less example operations.
//!
//! These functions are API-owned adapters. They consume the generic
//! [`OperationInputView`] and return transport-neutral outcomes; framing,
//! status projection, and client representation stay outside this module.

use openkache_protocol::OwnedRange;

use super::operation_api::{PrepareContext, PrepareError, PreparePlan};
use super::operation_contract::{OperationStatus, request_fields};
use super::operation_fields::OperationFieldEnvelope;
use super::operation_generic_behavior as behavior;
use super::operation_generic_resources::EXPERIMENTAL_RESOURCE_STORE;
use super::operation_handlers::{OperationContext, OperationInputView};
use super::operation_outcome::{
    OperationBody, OperationError, OperationOutcome, OperationValue,
};
use super::operation_registry::OperationFuture;
use super::storage_port::{
    StorageAddress, StorageError, StoragePortExt,
};

pub(crate) fn ping_handler(_input: OperationInputView) -> Result<OperationOutcome, OperationError> {
    Ok(OperationOutcome::opaque(
        OperationStatus::Ok,
        OperationValue::inline(behavior::ping()),
    ))
}

fn require_single_field(input: &OperationInputView) -> Result<(), OperationError> {
    if input.field_count() != 1 || input.field_at_index(0).is_none() {
        return Err(OperationError::InvalidRequest(
            b"operation requires one request field",
        ));
    }
    Ok(())
}

fn transform_packed_f64(
    field: OperationFieldEnvelope<'_>,
) -> Result<Vec<u8>, &'static [u8]> {
    if !field.has_codec("packed_f64_be") {
        return Err(b"field does not declare packed_f64_be");
    }
    openkache_protocol::codec::transform_packed_f64_be(field.bytes(), behavior::square)
}

fn take_single_field_range(
    mut input: OperationInputView,
) -> Result<OwnedRange, OperationError> {
    require_single_field(&input)?;
    input
        .take_single_field_bytes_range()
        .ok_or(OperationError::InvalidRequest(
            b"operation requires one request field",
        ))
}

pub(crate) fn echo_handler(input: OperationInputView) -> Result<OperationOutcome, OperationError> {
    Ok(OperationOutcome::opaque(
        OperationStatus::Ok,
        behavior::echo(take_single_field_range(input)?),
    ))
}

pub(crate) fn acknowledge_handler(
    input: OperationInputView,
) -> Result<OperationOutcome, OperationError> {
    require_single_field(&input)?;
    let token_field = input
        .required_encoded_field_at_index(
            request_fields::op_experimental_acknowledge::TOKEN,
            b"operation requires a token field",
        )
        .map_err(OperationError::InvalidRequest)?;
    let token = token_field
        .decode_utf8()
        .map_err(OperationError::InvalidRequest)?;
    behavior::acknowledge(token);
    Ok(OperationOutcome::success(
        OperationStatus::Accepted,
        OperationBody::Empty,
    ))
}

pub(crate) fn dense_handler(input: OperationInputView) -> Result<OperationOutcome, OperationError> {
    let counter = input
        .required_encoded_field_at_index(
            request_fields::op_experimental_dense::COUNTER,
            b"operation requires an unsigned counter field",
        )
        .map_err(OperationError::InvalidRequest)?
        .decode_u64()
        .map_err(OperationError::InvalidRequest)?;
    let enabled = input
        .required_encoded_field_at_index(
            request_fields::op_experimental_dense::ENABLED,
            b"operation requires a boolean field",
        )
        .map_err(OperationError::InvalidRequest)?
        .decode_bool()
        .map_err(OperationError::InvalidRequest)?;
    let output = behavior::dense(behavior::DenseValue { counter, enabled });
    let counter = output.counter.to_be_bytes();
    let enabled = [u8::from(output.enabled)];
    Ok(OperationOutcome::field_sequence(
        OperationStatus::Ok,
        [
            Some(OperationValue::inline(&counter)),
            Some(OperationValue::inline(&enabled)),
        ],
    ))
}

pub(crate) fn reverse_handler(
    input: OperationInputView,
) -> Result<OperationOutcome, OperationError> {
    let field = input
        .required_encoded_field_at_index(
            request_fields::op_experimental_reverse::MESSAGE,
            b"reverse requires one message field",
        )
        .map_err(OperationError::InvalidRequest)?;
    let value = field
        .decode_utf8()
        .map_err(OperationError::InvalidRequest)?;
    let value = behavior::reverse(value);
    Ok(OperationOutcome::opaque(
        OperationStatus::Ok,
        value.into_bytes(),
    ))
}

pub(crate) fn square_array_handler(
    input: OperationInputView,
) -> Result<OperationOutcome, OperationError> {
    let field = input
        .required_encoded_field_at_index(
            request_fields::op_square_array::VALUES,
            b"SquareArray requires a values field",
        )
        .map_err(OperationError::InvalidRequest)?;
    let output = transform_packed_f64(field).map_err(OperationError::InvalidRequest)?;
    Ok(OperationOutcome::opaque(OperationStatus::Ok, output))
}

pub(crate) fn page_handler(input: OperationInputView) -> Result<OperationOutcome, OperationError> {
    let page = behavior::page(
        input.bytes_at_index(request_fields::op_experimental_page::CURSOR),
    );
    let items = openkache_protocol::codec::encode_list_segmented(
        page.items.into_iter().map(OwnedRange::whole),
    )
    .map_err(|error| OperationError::InvalidRequest(error.message()))?;
    Ok(OperationOutcome::field_sequence(
        OperationStatus::Ok,
        [
            Some(OperationValue::from(items)),
            page.next_cursor.map(OperationValue::from),
        ],
    ))
}

pub(crate) fn prepare_multi_resource_mutation(
    input: &OperationInputView,
    context: PrepareContext<'_>,
) -> Result<PreparePlan, PrepareError> {
    let store = context
        .capability(EXPERIMENTAL_RESOURCE_STORE)
        .ok_or_else(|| {
            PrepareError::resource_unavailable(
                OperationStatus::InternalError,
                b"experimental resource store is not installed",
            )
        })?;
    let source = input
        .bytes_at_index(
            request_fields::op_experimental_multi_resource_mutation::SOURCE_RESOURCE,
        )
        .ok_or_else(|| PrepareError::invalid_request(b"source resource is missing"))?;
    let target = input
        .bytes_at_index(
            request_fields::op_experimental_multi_resource_mutation::TARGET_RESOURCE,
        )
        .ok_or_else(|| PrepareError::invalid_request(b"target resource is missing"))?;
    let source_lock = store.resource_lock(source).map_err(|message| {
        PrepareError::resource_unavailable(OperationStatus::InternalError, message)
    })?;
    let target_lock = store.resource_lock(target).map_err(|message| {
        PrepareError::resource_unavailable(OperationStatus::InternalError, message)
    })?;
    Ok(PreparePlan::from_resources([source_lock, target_lock]))
}

pub(crate) fn multi_resource_mutation_handler<'a>(
    context: OperationContext<'a>,
) -> OperationFuture<'a> {
    let Some(store) = context.capability(EXPERIMENTAL_RESOURCE_STORE) else {
        return OperationFuture::ready(OperationOutcome::error(OperationError::status(
            OperationStatus::InternalError,
            b"experimental resource store is not installed",
        )));
    };
    let input = context.input;
    let Some(source) = input.bytes_at_index(
        request_fields::op_experimental_multi_resource_mutation::SOURCE_RESOURCE,
    ) else {
        return OperationFuture::ready(OperationOutcome::invalid_request(
            b"source resource is missing",
        ));
    };
    let Some(target) = input.bytes_at_index(
        request_fields::op_experimental_multi_resource_mutation::TARGET_RESOURCE,
    ) else {
        return OperationFuture::ready(OperationOutcome::invalid_request(
            b"target resource is missing",
        ));
    };
    let Some(payload) =
        input.bytes_at_index(request_fields::op_experimental_multi_resource_mutation::PAYLOAD)
    else {
        return OperationFuture::ready(OperationOutcome::invalid_request(
            b"mutation payload is missing",
        ));
    };
    let outcome = match store.mutate(source, target, payload) {
        Ok(receipt) => {
            OperationOutcome::field_sequence(OperationStatus::Ok, [Some(receipt)])
        }
        Err(message) => OperationOutcome::error(OperationError::status(
            OperationStatus::InternalError,
            message,
        )),
    };
    OperationFuture::ready(outcome)
}

fn storage_failure(error: StorageError) -> OperationOutcome {
    let status = match &error {
        StorageError::InvalidRequest(_) => OperationStatus::InvalidRequest,
        StorageError::Unavailable(_) => OperationStatus::Overloaded,
        StorageError::Timeout(_) => OperationStatus::Timeout,
        StorageError::Worker(_) | StorageError::Backend(_) => OperationStatus::InternalError,
    };
    OperationOutcome::error(OperationError::owned_status(
        status,
        error.message().as_bytes().to_vec(),
    ))
}

/// A small storage-backed example that exercises the neutral storage port.
///
/// The API owns only the key/value meaning. Address normalization, worker
/// affinity, and backend error translation stay below this binding.
pub(crate) fn storage_read_handler<'a>(context: OperationContext<'a>) -> OperationFuture<'a> {
    let storage = context
        .capability(super::storage_port::STORAGE_PORT)
        .map(std::sync::Arc::as_ref);
    let mut input = context.input;
    let Some(key) = input.take_single_field_bytes_range() else {
        return OperationFuture::ready(OperationOutcome::invalid_request(
            b"storage read requires one key field",
        ));
    };
    let Some(storage) = storage else {
        return OperationFuture::ready(OperationOutcome::error(OperationError::status(
            OperationStatus::InternalError,
            b"storage capability is not installed",
        )));
    };
    let address = StorageAddress::from_owned_range(key);
    let task_address = address.clone();
    OperationFuture::pending(Box::pin(async move {
        match storage
            .execute_typed_for_key(address, move |worker| Box::pin(worker.get(task_address)))
            .await
        {
            Ok(Some(value)) => OperationOutcome::opaque(OperationStatus::Ok, value),
            Ok(None) => OperationOutcome::opaque(
                OperationStatus::NotFound,
                OperationValue::inline(b""),
            ),
            Err(error) => storage_failure(error),
        }
    }))
}

fn immediate_as_future(
    input: OperationInputView,
    handler: fn(OperationInputView) -> Result<OperationOutcome, OperationError>,
) -> OperationFuture<'static> {
    OperationFuture::ready(handler(input).unwrap_or_else(OperationOutcome::error))
}

/// Adapts a storage-free binding to the same future boundary as every other
/// operation. The dispatcher has no synchronous execution family.
macro_rules! immediate_handler {
    ($name:ident, $binding:path) => {
        pub fn $name<'a>(context: OperationContext<'a>) -> OperationFuture<'a> {
            immediate_as_future(context.input, $binding)
        }
    };
}

immediate_handler!(ping_handler_async, ping_handler);
immediate_handler!(echo_handler_async, echo_handler);
immediate_handler!(acknowledge_handler_async, acknowledge_handler);
immediate_handler!(dense_handler_async, dense_handler);
immediate_handler!(reverse_handler_async, reverse_handler);
immediate_handler!(square_array_handler_async, square_array_handler);
immediate_handler!(page_handler_async, page_handler);
