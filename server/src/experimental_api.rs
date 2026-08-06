//! Implementations for the experimental application and paired-read APIs.
//!
//! The operation contract and dispatch remain generated from Smithy. This
//! module owns only the behavior of the experimental operations themselves so
//! adding another generated operation does not add a branch to the transport
//! or shared wire crate.

use std::sync::Mutex;

use openkache_protocol::{ItemId, MAX_VALUE_BYTES, Opcode, Response, Status};

use super::operation_handlers::OperationContext;
use super::{
    NamespaceRegistry, NetworkWorkerCache, cache_error_response, namespace_exists, response,
    response_bytes,
};
use crate::contract::OperationValueTransform;

/// Executes an operation whose behavior belongs to an API-owned extension.
///
/// The protocol handler calls this hook for every decoded non-immediate
/// request. Keeping the opcode match here makes the shared dispatch path
/// stable while allowing the server to add behavior without teaching the
/// framing or client infrastructure about that behavior.
pub(super) async fn execute(context: &OperationContext<'_, '_>) -> Option<Response> {
    match context.opcode {
        Opcode::Get2 => {
            execute_get2(
                context.cache,
                context.namespace_id,
                context.item_ids,
                context.namespaces,
            )
            .await
        }
        _ => None,
    }
}

/// Applies an API-owned transform to an application-value request.
pub(super) fn application_value_response(
    transform: OperationValueTransform,
    value: Vec<u8>,
) -> Response {
    match transform_application_value(transform, value) {
        Ok(value) => response(Status::Ok, value),
        Err(message) => response_bytes(Status::InvalidRequest, message),
    }
}

fn transform_application_value(
    transform: OperationValueTransform,
    value: Vec<u8>,
) -> std::result::Result<Vec<u8>, &'static [u8]> {
    match transform {
        OperationValueTransform::Identity => Ok(value),
        OperationValueTransform::ReverseUtf8 => {
            let value = std::str::from_utf8(&value)
                .map_err(|_| b"application value must be valid UTF-8" as &'static [u8])?;
            Ok(value.chars().rev().collect::<String>().into_bytes())
        }
        OperationValueTransform::SquareArray => {
            const F64_BYTES: usize = std::mem::size_of::<f64>();
            if value.len() % F64_BYTES != 0 {
                return Err(b"square_array payload length must be a multiple of eight");
            }
            let mut squared = Vec::with_capacity(value.len());
            for chunk in value.chunks_exact(F64_BYTES) {
                let input = f64::from_be_bytes(
                    chunk
                        .try_into()
                        .expect("chunks_exact returned a fixed-width chunk"),
                );
                if !input.is_finite() {
                    return Err(b"square_array input must contain finite values");
                }
                let output = input * input;
                if !output.is_finite() {
                    return Err(b"square_array result must contain finite values");
                }
                squared.extend_from_slice(&output.to_be_bytes());
            }
            Ok(squared)
        }
    }
}

/// Executes the experimental two-item read without teaching the shared
/// protocol runtime about GET2.
pub(super) async fn execute_get2(
    cache: &NetworkWorkerCache<'_>,
    namespace_id: Option<u64>,
    item_ids: &[ItemId],
    namespaces: &Mutex<NamespaceRegistry>,
) -> Option<Response> {
    let namespace_id = namespace_id.expect("GET2 has a validated namespace ID");
    let [first_item, second_item] = item_ids else {
        return Some(response_bytes(
            Status::InvalidRequest,
            b"GET2 requires exactly two item IDs",
        ));
    };
    if !namespace_exists(namespaces, namespace_id) {
        return Some(response_bytes(
            Status::NamespaceNotFound,
            b"namespace does not exist",
        ));
    }

    let (first, second) = futures_util::future::join(
        cache.get_in_namespace(namespace_id, *first_item),
        cache.get_in_namespace(namespace_id, *second_item),
    )
    .await;
    let mut values = Vec::with_capacity(2);
    for (item_id, result) in [(*first_item, first), (*second_item, second)] {
        match result {
            Ok(value) => {
                if value.is_none() {
                    let pruned = namespaces.lock().map_err(|_| ()).and_then(|mut registry| {
                        registry.prune_item(namespace_id, item_id).map_err(|_| ())
                    });
                    if pruned.is_err() {
                        return Some(response_bytes(
                            Status::InternalError,
                            b"namespace metadata is unavailable",
                        ));
                    }
                }
                values.push(value);
            }
            Err(error) => return Some(cache_error_response(error)),
        }
    }

    let Some(payload) = encode_optional_values(&values) else {
        return Some(response_bytes(
            Status::TooLarge,
            b"GET2 response exceeds the protocol response limit",
        ));
    };
    Some(response(Status::Ok, payload))
}

/// Encodes the ordered optional values used only by the GET2 API contract.
fn encode_optional_values(
    values: &[Option<super::super::types::StoredItemValue>],
) -> Option<Vec<u8>> {
    const LENGTH_BYTES: usize = std::mem::size_of::<u32>();
    const MISSING: u32 = u32::MAX;
    let payload_len = values.iter().try_fold(0usize, |length, value| {
        let value_len = value.as_ref().map_or(0, |value| value.len());
        if value_len >= MISSING as usize {
            return None;
        }
        length.checked_add(LENGTH_BYTES)?.checked_add(value_len)
    })?;
    if payload_len > MAX_VALUE_BYTES {
        return None;
    }
    let mut payload = Vec::with_capacity(payload_len);
    for value in values {
        let length = value.as_ref().map_or(MISSING, |value| value.len() as u32);
        payload.extend_from_slice(&length.to_be_bytes());
        if let Some(value) = value {
            payload.extend_from_slice(value.as_ref());
        }
    }
    Some(payload)
}
