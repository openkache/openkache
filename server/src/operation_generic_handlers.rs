//! Generated-field bindings for the route-less example operations.
//!
//! These functions are API-owned adapters. They consume the generic
//! [`OperationInputView`] and return transport-neutral outcomes; framing,
//! status projection, and client representation stay outside this module.

use openkache_protocol::OwnedRange;

use super::operation_api::{PrepareContext, PrepareError, PreparePlan};
use super::operation_contract::request_fields;
use super::operation_fields::OperationFieldEnvelope;
use super::operation_generic_behavior as behavior;
use super::operation_generic_resources::EXPERIMENTAL_RESOURCE_STORE;
use super::operation_handlers::{OperationContext, OperationInputView};
use super::operation_outcome::{
    OperationBody, OperationError, OperationOutcome, OperationValue,
};
use super::operation_generic_status as status;
use super::operation_registry::OperationFuture;
use super::storage_port::{
    StorageAddress, StorageError, StoragePortExt,
};

pub(crate) fn ping_handler(_input: OperationInputView) -> Result<OperationOutcome, OperationError> {
    Ok(OperationOutcome::opaque(
        status::OK,
        OperationValue::inline(behavior::ping()),
    ))
}

fn transform_packed_f64(
    field: OperationFieldEnvelope<'_>,
) -> Result<Vec<u8>, &'static [u8]> {
    if !field.has_codec("packed_f64_be") {
        return Err(b"field does not declare packed_f64_be");
    }
    openkache_protocol::codec::transform_packed_f64_be(field.bytes(), behavior::square)
}

pub(crate) fn echo_handler(
    mut input: OperationInputView,
) -> Result<OperationOutcome, OperationError> {
    Ok(OperationOutcome::opaque(
        status::OK,
        behavior::echo(
            input
                .take_single_field_bytes_range()
                .ok_or(OperationError::InvalidRequest(
                    b"operation requires one request field",
                ))?,
        ),
    ))
}

pub(crate) fn acknowledge_handler(
    input: OperationInputView,
) -> Result<OperationOutcome, OperationError> {
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
        status::ACCEPTED,
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
    Ok(OperationOutcome::fields(
        status::OK,
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
        status::OK,
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
    Ok(OperationOutcome::opaque(status::OK, output))
}

pub(crate) fn page_handler(input: OperationInputView) -> Result<OperationOutcome, OperationError> {
    let page = behavior::page(
        input.bytes_at_index(request_fields::op_experimental_page::CURSOR),
    );
    let items = openkache_protocol::codec::encode_list_segmented(
        page.items.into_iter().map(OwnedRange::whole),
    )
    .map_err(|error| OperationError::InvalidRequest(error.message()))?;
    Ok(OperationOutcome::fields(
        status::OK,
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
                status::INTERNAL_ERROR,
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
        PrepareError::resource_unavailable(status::INTERNAL_ERROR, message)
    })?;
    let target_lock = store.resource_lock(target).map_err(|message| {
        PrepareError::resource_unavailable(status::INTERNAL_ERROR, message)
    })?;
    Ok(PreparePlan::from_resources([source_lock, target_lock]))
}

pub(crate) fn multi_resource_mutation_handler<'a>(
    context: OperationContext<'a>,
) -> OperationFuture<'a> {
    let Some(store) = context.capability(EXPERIMENTAL_RESOURCE_STORE) else {
        return OperationFuture::ready(OperationOutcome::error(OperationError::status(
            status::INTERNAL_ERROR,
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
            OperationOutcome::fields(status::OK, [Some(receipt)])
        }
        Err(message) => OperationOutcome::error(OperationError::status(
            status::INTERNAL_ERROR,
            message,
        )),
    };
    OperationFuture::ready(outcome)
}

fn storage_failure(error: StorageError) -> OperationOutcome {
    let status = match &error {
        StorageError::InvalidRequest(_) => status::INVALID_REQUEST,
        StorageError::Unavailable(_) => status::OVERLOADED,
        StorageError::Timeout(_) => status::TIMEOUT,
        StorageError::Worker(_) | StorageError::Backend(_) => status::INTERNAL_ERROR,
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
            status::INTERNAL_ERROR,
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
            Ok(Some(value)) => OperationOutcome::opaque(status::OK, value),
            Ok(None) => OperationOutcome::opaque(
                status::NOT_FOUND,
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
