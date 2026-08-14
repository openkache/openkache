//! API-owned bindings for operations that use only generic field envelopes.
//!
//! These examples deliberately avoid namespace, item, and SET vocabulary.
//! Their request and response shapes come from the generated operation
//! contract; only application semantics live here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::lock::Mutex as AsyncMutex;
use openkache_protocol::Opcode;

use super::operation_api::{
    self, ApiModule, CapabilityKey, PrepareContext, PrepareError, PreparePlan, ResourceLock,
};
use super::operation_capabilities::CapabilityRegistry;
use super::operation_contract::{OperationStatus, request_fields};
use super::operation_handlers::{OperationContext, OperationInputView};
use super::operation_outcome::{OperationError, OperationOutcome, OperationValue};
use super::operation_registry::OperationFuture;

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

pub(super) fn install_resource_store(registry: &mut CapabilityRegistry) {
    registry.insert(
        EXPERIMENTAL_RESOURCE_STORE,
        ExperimentalResourceStore::default(),
    );
}

fn ping_handler<'a>(_context: OperationContext<'a>) -> OperationFuture<'a> {
    OperationFuture::ready(OperationOutcome::opaque(
        OperationStatus::Ok,
        OperationValue::inline(b"PONG"),
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

pub(super) const API: ApiModule = ApiModule::new(crate::protocol::generic_request_descriptor(), &[
    operation_api::RegistrationBuilder::generic(Opcode::Ping, ping_handler)
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
