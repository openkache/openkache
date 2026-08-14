//! API-owned bindings for operations that use only generic field envelopes.
//!
//! These examples deliberately avoid namespace, item, and SET vocabulary.
//! Their request and response shapes come from the generated operation
//! contract; only application semantics live here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::lock::Mutex as AsyncMutex;
use openkache_protocol::{Opcode, OwnedRange};

use super::operation_api::{
    self, ApiModule, CapabilityKey, PrepareContext, PrepareError, PreparePlan, ResourceLock,
};
use super::operation_capabilities::CapabilityRegistry;
use super::operation_contract::{OperationStatus, request_fields};
use super::operation_handlers::{OperationContext, OperationInputView};
use super::operation_outcome::{
    OperationBody, OperationError, OperationOutcome, OperationValue,
};
use super::operation_registry::OperationFuture;
use super::storage_port::{
    STORAGE_PORT, StorageAddress, StorageError, StoragePortExt, StoragePortHandle,
};

/// Application behavior for the route-less example API.
///
/// This module deliberately has no generated-contract, wire-codec, transport,
/// client, or storage imports. The surrounding bindings translate between
/// typed values and field envelopes, just as an independently owned API module
/// would.
mod behavior {
    pub(super) struct DenseValue {
        pub(super) counter: u64,
        pub(super) enabled: bool,
    }

    pub(super) struct Page {
        pub(super) items: Vec<Vec<u8>>,
        pub(super) next_cursor: Option<Vec<u8>>,
    }

    pub(super) const fn ping() -> &'static [u8] {
        b"PONG"
    }

    pub(super) fn echo<T>(value: T) -> T {
        value
    }

    pub(super) fn acknowledge(_token: &str) {}

    pub(super) fn dense(value: DenseValue) -> DenseValue {
        value
    }

    pub(super) fn reverse(value: &str) -> String {
        value.chars().rev().collect()
    }

    pub(super) fn square(value: f64) -> Option<f64> {
        let squared = value * value;
        squared.is_finite().then_some(squared)
    }

    pub(super) fn page(cursor: Option<&[u8]>) -> Page {
        match cursor {
            None => Page {
                items: vec![b"first".to_vec(), b"second".to_vec()],
                next_cursor: Some(b"next".to_vec()),
            },
            Some([]) => Page {
                items: Vec::new(),
                next_cursor: Some(Vec::new()),
            },
            Some(_) => Page {
                items: vec![b"last".to_vec()],
                next_cursor: None,
            },
        }
    }
}

/// API-owned state used by the multi-resource example.
///
/// The executor sees only opaque [`ResourceLock`] handles. Resource identity,
/// storage, and mutation semantics remain local to this API module.
#[derive(Default)]
pub(super) struct ExperimentalResourceStore {
    locks: Mutex<HashMap<Vec<u8>, Arc<AsyncMutex<()>>>>,
    values: Mutex<HashMap<Vec<u8>, Vec<u8>>>,
}

impl ExperimentalResourceStore {
    fn resource_lock(&self, identity: &[u8]) -> Result<ResourceLock, &'static [u8]> {
        let mut locks = self
            .locks
            .lock()
            .map_err(|_| b"experimental resource lock registry is unavailable".as_slice())?;
        let lock = Arc::clone(
            locks
                .entry(identity.to_vec())
                .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
        );
        Ok(ResourceLock::unconditional(lock))
    }

    fn mutate(
        &self,
        source: &[u8],
        target: &[u8],
        payload: &[u8],
    ) -> Result<Vec<u8>, &'static [u8]> {
        let mut values = self
            .values
            .lock()
            .map_err(|_| b"experimental resource store is unavailable".as_slice())?;
        values.insert(source.to_vec(), payload.to_vec());
        values.insert(target.to_vec(), payload.to_vec());
        Ok(u64::try_from(values.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes()
            .to_vec())
    }
}

const EXPERIMENTAL_RESOURCE_STORE: CapabilityKey<ExperimentalResourceStore> =
    CapabilityKey::new("openkache.experimental.resource_store");

/// Installs the runtime-neutral storage port used by this API module.
///
/// The network loop only forwards the already-created port through the API
/// composition boundary. It does not need to know which generic operation
/// consumes storage, and a future API can expose a different capability
/// without changing the transport path.
pub(super) fn install_storage_port(
    registry: &mut CapabilityRegistry,
    storage: StoragePortHandle,
) {
    registry.insert(STORAGE_PORT, storage);
}

pub(super) fn install_resource_store(registry: &mut CapabilityRegistry) {
    registry.insert(
        EXPERIMENTAL_RESOURCE_STORE,
        ExperimentalResourceStore::default(),
    );
}

fn ping_handler(_input: OperationInputView) -> Result<OperationOutcome, OperationError> {
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

pub(super) fn echo_handler(input: OperationInputView) -> Result<OperationOutcome, OperationError> {
    Ok(OperationOutcome::opaque(
        OperationStatus::Ok,
        behavior::echo(take_single_field_range(input)?),
    ))
}

pub(super) fn acknowledge_handler(
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

pub(super) fn dense_handler(input: OperationInputView) -> Result<OperationOutcome, OperationError> {
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

pub(super) fn reverse_handler(
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

pub(super) fn square_array_handler(
    input: OperationInputView,
) -> Result<OperationOutcome, OperationError> {
    let field = input
        .required_encoded_field_at_index(
            request_fields::op_square_array::VALUES,
            b"SquareArray requires a values field",
        )
        .map_err(OperationError::InvalidRequest)?;
    let output = field
        .transform_packed_f64(behavior::square)
        .map_err(OperationError::InvalidRequest)?;
    Ok(OperationOutcome::opaque(OperationStatus::Ok, output))
}

pub(super) fn page_handler(input: OperationInputView) -> Result<OperationOutcome, OperationError> {
    let page = behavior::page(
        input.bytes_at_index(request_fields::op_experimental_page::CURSOR),
    );
    let item_refs = page
        .items
        .iter()
        .map(Vec::as_slice)
        .collect::<Vec<_>>();
    let items = openkache_protocol::codec::encode_list(&item_refs)
        .map_err(|error| OperationError::InvalidRequest(error.message()))?;
    Ok(OperationOutcome::field_sequence(
        OperationStatus::Ok,
        [Some(items), page.next_cursor],
    ))
}

pub(super) fn prepare_multi_resource_mutation(
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

pub(super) fn multi_resource_mutation_handler<'a>(
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
pub(super) fn storage_read_handler<'a>(context: OperationContext<'a>) -> OperationFuture<'a> {
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
        pub(super) fn $name<'a>(context: OperationContext<'a>) -> OperationFuture<'a> {
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

pub(super) const API: ApiModule = ApiModule::new(crate::protocol::generic_request_descriptor(), &[
    operation_api::RegistrationBuilder::generic(Opcode::Ping, ping_handler_async)
        .read_only()
        .build(),
    operation_api::RegistrationBuilder::generic(Opcode::ExperimentalEcho, echo_handler_async)
        .read_only()
        .build(),
    operation_api::RegistrationBuilder::generic(Opcode::ExperimentalReverse, reverse_handler_async)
        .read_only()
        .build(),
    operation_api::RegistrationBuilder::generic(Opcode::SquareArray, square_array_handler_async)
        .read_only()
        .build(),
    operation_api::RegistrationBuilder::generic(
        Opcode::ExperimentalAcknowledge,
        acknowledge_handler_async,
    )
    .read_only()
    .build(),
    operation_api::RegistrationBuilder::generic(Opcode::ExperimentalDense, dense_handler_async)
        .read_only()
        .build(),
    operation_api::RegistrationBuilder::generic(
        Opcode::ExperimentalStorageRead,
        storage_read_handler,
    )
    .read_only()
    .build(),
    operation_api::RegistrationBuilder::generic(Opcode::ExperimentalPage, page_handler_async)
        .read_only()
        .build(),
    operation_api::RegistrationBuilder::generic(
        Opcode::ExperimentalMultiResourceMutation,
        multi_resource_mutation_handler,
    )
    .prepare(prepare_multi_resource_mutation)
    .mutation()
    .build(),
]);
