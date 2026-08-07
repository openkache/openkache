//! Implementations for the experimental application and paired-read APIs.
//!
//! The operation contract and dispatch remain generated from Smithy. This
//! module owns only the behavior of the experimental operations themselves so
//! adding another generated operation does not add a branch to the transport
//! or shared wire crate.

use std::sync::Mutex;

use openkache_protocol::{Opcode, Status};

use super::super::types::StoredItemValue;
use super::operation_handlers::OperationContext;
use super::protocol::Response;
use super::{
    NamespaceRegistry, NetworkWorkerCache, cache_error_response, namespace_exists,
    operation_codecs, response_bytes,
};

/// Domain-level result returned by a server operation extension.
///
/// The shared handler owns protocol framing for successful application and
/// ordered optional values.
pub(super) enum ExtensionResponse {
    Response(Response),
    ApplicationValue(Vec<u8>),
    OptionalValues(Vec<Option<StoredItemValue>>),
}

/// Executes an operation whose behavior belongs to an API-owned extension.
///
/// The protocol handler calls this hook for every decoded non-immediate
/// request. Keeping the opcode match here makes the shared dispatch path
/// stable while allowing the server to add behavior without teaching the
/// framing or client infrastructure about that behavior.
pub(super) async fn execute(context: &OperationContext<'_, '_>) -> Option<ExtensionResponse> {
    match context.opcode {
        Opcode::Get2 => execute_get2(context.cache, &context.input, context.namespaces).await,
        _ => None,
    }
}

type ApplicationValueHandler = fn(Vec<u8>) -> std::result::Result<Vec<u8>, &'static [u8]>;

struct ApplicationValueExtension {
    opcode: Opcode,
    handler: ApplicationValueHandler,
}

/// Server-owned implementations for application-value operations.
///
/// This registry is deliberately outside the generated protocol contract:
/// Smithy describes the payload shape and codec, while the server chooses the
/// behavior attached to each operation. The transport only needs to pass the
/// decoded bytes through this extension point.
const APPLICATION_VALUE_EXTENSIONS: &[ApplicationValueExtension] = &[
    ApplicationValueExtension {
        opcode: Opcode::ExperimentalEcho,
        handler: echo_application_value,
    },
    ApplicationValueExtension {
        opcode: Opcode::ExperimentalReverse,
        handler: reverse_utf8_application_value,
    },
    ApplicationValueExtension {
        opcode: Opcode::SquareArray,
        handler: square_array_application_value,
    },
];

/// Applies the server-owned implementation for an application-value opcode.
pub(super) fn application_value(opcode: Opcode, value: Vec<u8>) -> Option<ExtensionResponse> {
    let Some(extension) = APPLICATION_VALUE_EXTENSIONS
        .iter()
        .find(|extension| extension.opcode == opcode)
    else {
        return None;
    };
    match (extension.handler)(value) {
        Ok(value) => Some(ExtensionResponse::ApplicationValue(value)),
        Err(message) => Some(ExtensionResponse::Response(response_bytes(
            Status::InvalidRequest,
            message,
        ))),
    }
}

/// Reports whether this extension owns an operation outside the built-in
/// server handler match.
pub(super) fn handles(opcode: Opcode) -> bool {
    opcode == Opcode::Get2
        || APPLICATION_VALUE_EXTENSIONS
            .iter()
            .any(|extension| extension.opcode == opcode)
}

fn echo_application_value(value: Vec<u8>) -> std::result::Result<Vec<u8>, &'static [u8]> {
    operation_codecs::decode_utf8(&value)?;
    Ok(value)
}

fn reverse_utf8_application_value(value: Vec<u8>) -> std::result::Result<Vec<u8>, &'static [u8]> {
    let value = operation_codecs::decode_utf8(&value)?;
    let reversed = value.chars().rev().collect::<String>();
    Ok(operation_codecs::encode_utf8(&reversed))
}

fn square_array_application_value(value: Vec<u8>) -> std::result::Result<Vec<u8>, &'static [u8]> {
    operation_codecs::transform_packed_f64_be(&value, |input| {
        input.is_finite().then_some(input * input)
    })
}

/// Executes the experimental two-item read without teaching the shared
/// protocol runtime about GET2.
pub(super) async fn execute_get2(
    cache: &NetworkWorkerCache<'_>,
    input: &super::operation_handlers::OperationInputView<'_>,
    namespaces: &Mutex<NamespaceRegistry>,
) -> Option<ExtensionResponse> {
    let namespace_id = match input.field("namespace_id") {
        Some(super::operation_handlers::OperationFieldValue::UnsignedLong(namespace_id)) => {
            namespace_id
        }
        _ => {
            return Some(ExtensionResponse::Response(response_bytes(
                Status::InvalidRequest,
                b"GET2 requires a namespace ID",
            )));
        }
    };
    let item_ids = match input.field("item_id") {
        Some(super::operation_handlers::OperationFieldValue::ItemIds(item_ids)) => item_ids,
        _ => {
            return Some(ExtensionResponse::Response(response_bytes(
                Status::InvalidRequest,
                b"GET2 requires item IDs",
            )));
        }
    };
    let [first_item, second_item] = item_ids else {
        return Some(ExtensionResponse::Response(response_bytes(
            Status::InvalidRequest,
            b"GET2 requires exactly two item IDs",
        )));
    };
    if !namespace_exists(namespaces, namespace_id) {
        return Some(ExtensionResponse::Response(response_bytes(
            Status::NamespaceNotFound,
            b"namespace does not exist",
        )));
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
                        return Some(ExtensionResponse::Response(response_bytes(
                            Status::InternalError,
                            b"namespace metadata is unavailable",
                        )));
                    }
                }
                values.push(value);
            }
            Err(error) => return Some(ExtensionResponse::Response(cache_error_response(error))),
        }
    }

    Some(ExtensionResponse::OptionalValues(values))
}
