//! Server-owned operation handlers.
//!
//! The protocol and client infrastructure only decode a request according to
//! its generated wire contract. This module is the server's decision point:
//! it receives a borrowed operation context and calls the concrete behavior
//! selected by the server. Adding a new operation therefore does not add an
//! operation-name branch to the transport, framing, or client infrastructure.

use std::fmt::Write as _;
use std::sync::Mutex;

use openkache_protocol::{Opcode, Status};

use super::{
    NamespaceRegistry, NetworkWorkerCache, ObservabilityState, SetOutcome, cache_error_response,
    descriptor_payload, mutation_cache_error_response, namespace_exists, resolve_set_options,
    response, response_bytes,
};
use crate::contract::{OperationRequestKind, OperationResponseKind};
use crate::protocol::{ItemId, NamespacePolicy, Response, SetOptions};

/// Borrowed context passed from the protocol server to one concrete handler.
///
/// The context deliberately contains storage primitives and decoded request
/// fields, rather than exposing transport or frame details to API handlers.
pub(super) struct OperationContext<'a, 'cache> {
    pub(super) cache: &'a NetworkWorkerCache<'cache>,
    pub(super) opcode: Opcode,
    pub(super) namespace_id: Option<u64>,
    pub(super) item_ids: &'a [ItemId],
    pub(super) set_options: SetOptions,
    pub(super) value: Vec<u8>,
    pub(super) namespace_name: Option<&'a [u8]>,
    pub(super) namespace_policy: Option<NamespacePolicy>,
    pub(super) expected_revision: Option<u64>,
    pub(super) create_if_missing: bool,
    pub(super) administrator: bool,
    pub(super) namespaces: &'a Mutex<NamespaceRegistry>,
    pub(super) observability: &'a ObservabilityState,
}

/// Returns whether an operation can be answered without touching storage.
pub(super) const fn is_immediate(opcode: Opcode) -> bool {
    let contract = crate::contract::operation_contract(opcode);
    matches!(
        (contract.request_kind, contract.response_kind),
        (OperationRequestKind::Empty, OperationResponseKind::Pong)
            | (
                OperationRequestKind::ApplicationValue,
                OperationResponseKind::ApplicationValue
            )
    )
}

/// Executes an already-classified immediate operation.
pub(super) fn immediate_response(opcode: Opcode, value: Vec<u8>) -> Response {
    let contract = crate::contract::operation_contract(opcode);
    match contract.response_kind {
        OperationResponseKind::Pong => response_bytes(Status::Ok, b"PONG"),
        OperationResponseKind::ApplicationValue => {
            super::experimental_api::application_value_response(contract.value_transform, value)
        }
        _ => response_bytes(
            Status::InternalError,
            b"invalid immediate operation contract",
        ),
    }
}

/// Dispatches a decoded, non-immediate request to server-owned behavior.
pub(super) async fn execute(context: OperationContext<'_, '_>) -> Option<Response> {
    if let Some(response) = super::experimental_api::execute(&context).await {
        return Some(response);
    }

    let OperationContext {
        cache,
        opcode,
        namespace_id,
        item_ids,
        set_options,
        value,
        namespace_name,
        namespace_policy,
        expected_revision,
        create_if_missing,
        administrator,
        namespaces,
        observability,
    } = context;

    match opcode {
        Opcode::Get => {
            let item_id = item_ids
                .first()
                .copied()
                .expect("GET requests have a validated item ID");
            execute_get(cache, namespace_id, item_id, namespaces).await
        }
        Opcode::NamespaceOpen => {
            let name = namespace_name.expect("namespace-open requests have a validated name");
            let result = namespaces
                .lock()
                .map_err(|_| Status::InternalError)
                .and_then(|mut registry| {
                    registry.open(name.to_vec(), create_if_missing, namespace_policy)
                });
            Some(match result {
                Ok((status, descriptor)) => response(status, descriptor_payload(descriptor)),
                Err(status) => response_bytes(status, b"namespace operation rejected"),
            })
        }
        Opcode::NamespaceUpdatePolicy => {
            let result = namespaces
                .lock()
                .map_err(|_| Status::InternalError)
                .and_then(|mut registry| {
                    registry.update(
                        namespace_id.expect("namespace update has a validated ID"),
                        expected_revision.expect("namespace update has a validated revision"),
                        namespace_policy.expect("namespace update has a validated policy"),
                    )
                });
            Some(match result {
                Ok(descriptor) => response(Status::Ok, descriptor_payload(descriptor)),
                Err(status) => response_bytes(status, b"namespace policy update rejected"),
            })
        }
        Opcode::NamespaceDelete => {
            let namespace_id = namespace_id.expect("namespace delete has a validated ID");
            let tracked_items = match namespaces.lock() {
                Ok(registry) => match registry.tracked_items(namespace_id) {
                    Some(items) => items,
                    None => {
                        return Some(response_bytes(
                            Status::NamespaceNotFound,
                            b"namespace does not exist",
                        ));
                    }
                },
                Err(_) => {
                    return Some(response_bytes(
                        Status::InternalError,
                        b"namespace metadata is unavailable",
                    ));
                }
            };
            // Expired items are logically absent even if their old storage records have not
            // been compacted yet. Prune them before the empty check so TTL does not prevent
            // namespace deletion.
            for item_id in tracked_items {
                match cache.get_in_namespace(namespace_id, item_id).await {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        if let Ok(mut registry) = namespaces.lock() {
                            if registry.prune_item(namespace_id, item_id).is_err() {
                                return Some(response_bytes(
                                    Status::InternalError,
                                    b"namespace metadata is unavailable",
                                ));
                            }
                        } else {
                            return Some(response_bytes(
                                Status::InternalError,
                                b"namespace metadata is unavailable",
                            ));
                        }
                    }
                    Err(error) => {
                        // Emptiness is a deletion precondition. If storage
                        // cannot answer the point lookup, do not remove the
                        // namespace metadata.
                        return Some(cache_error_response(error));
                    }
                }
            }
            let result = namespaces
                .lock()
                .map_err(|_| Status::InternalError)
                .and_then(|mut registry| {
                    registry.delete(
                        namespace_id,
                        expected_revision.expect("namespace delete has a validated revision"),
                    )
                });
            Some(match result {
                Ok(()) => response(Status::Deleted, Vec::new()),
                Err(status) => response_bytes(status, b"namespace deletion rejected"),
            })
        }
        Opcode::Set => {
            let namespace_id = namespace_id.expect("SET requests have a validated namespace ID");
            let policy = match namespaces
                .lock()
                .ok()
                .and_then(|registry| registry.policy(namespace_id))
            {
                Some(policy) => policy,
                None => {
                    return Some(response_bytes(
                        Status::NamespaceNotFound,
                        b"namespace does not exist",
                    ));
                }
            };
            let effective_options = match resolve_set_options(policy, set_options) {
                Ok(options) => options,
                Err(status) => {
                    return Some(response_bytes(status, b"SET policy is disallowed"));
                }
            };
            let item_id = item_ids
                .first()
                .copied()
                .expect("SET requests have a validated item ID");
            let worker = cache.namespace_item_worker(namespace_id, item_id);
            let reservation = match namespaces
                .lock()
                .map_err(|_| Status::InternalError)
                .and_then(|mut registry| registry.reserve_item(namespace_id, item_id, worker))
            {
                Ok(reservation) => reservation,
                Err(status) => {
                    return Some(response_bytes(status, b"namespace metadata is unavailable"));
                }
            };
            let outcome = cache
                .set_in_namespace(
                    namespace_id,
                    item_id,
                    super::super::types::StoredItemValue::new(value),
                    effective_options,
                )
                .await;
            match outcome {
                Ok(SetOutcome::Created) => Some(response(Status::Created, Vec::new())),
                Ok(SetOutcome::Replaced) => Some(response(Status::Replaced, Vec::new())),
                Ok(SetOutcome::NotStored) => {
                    let rollback = namespaces
                        .lock()
                        .map_err(|_| Status::InternalError)
                        .and_then(|mut registry| {
                            registry.rollback_set_reservation(
                                namespace_id,
                                item_id,
                                worker,
                                reservation,
                            )
                        });
                    match rollback {
                        Ok(()) => Some(response(Status::NotStored, Vec::new())),
                        Err(_) => None,
                    }
                }
                Err(error) => match mutation_cache_error_response(Opcode::Set, error) {
                    Some(response) => {
                        let rollback = namespaces
                            .lock()
                            .map_err(|_| Status::InternalError)
                            .and_then(|mut registry| {
                                registry.rollback_set_reservation(
                                    namespace_id,
                                    item_id,
                                    worker,
                                    reservation,
                                )
                            });
                        match rollback {
                            Ok(()) => Some(response),
                            Err(_) => None,
                        }
                    }
                    None => None,
                },
            }
        }
        Opcode::Delete => {
            let namespace_id = namespace_id.expect("DELETE requests have a validated namespace ID");
            if !namespace_exists(namespaces, namespace_id) {
                return Some(response_bytes(
                    Status::NamespaceNotFound,
                    b"namespace does not exist",
                ));
            }
            let item_id = item_ids
                .first()
                .copied()
                .expect("DELETE requests have a validated item ID");
            let worker = cache.namespace_item_worker(namespace_id, item_id);
            if let Err(status) = namespaces
                .lock()
                .map_err(|_| Status::InternalError)
                .and_then(|mut registry| registry.reserve_worker(namespace_id, worker))
            {
                return Some(response_bytes(status, b"namespace metadata is unavailable"));
            }
            let deleted = cache.delete_in_namespace(namespace_id, item_id).await;
            match deleted {
                Ok(deleted) => {
                    let Ok(mut registry) = namespaces.lock() else {
                        // The DELETE may already have taken effect. Closing
                        // the lane avoids claiming a reliable outcome while
                        // leaving the namespace tracker stale.
                        return None;
                    };
                    if registry
                        .mark_delete(namespace_id, item_id, deleted)
                        .is_err()
                    {
                        // The DELETE may already have taken effect, but the
                        // persisted tracker could not be updated.
                        return None;
                    }
                    Some(response(
                        if deleted {
                            Status::Deleted
                        } else {
                            Status::NotFound
                        },
                        Vec::new(),
                    ))
                }
                Err(error) => mutation_cache_error_response(Opcode::Delete, error),
            }
        }
        Opcode::Stats => {
            if !administrator {
                return Some(response_bytes(
                    Status::Forbidden,
                    b"STATS requires administrator authorization",
                ));
            }
            if !namespace_exists(
                namespaces,
                namespace_id.expect("STATS requests have a validated namespace ID"),
            ) {
                return Some(response_bytes(
                    Status::NamespaceNotFound,
                    b"namespace does not exist",
                ));
            }
            match cache.stats().await {
                Ok(workers) => {
                    let worker_bytes = workers.iter().map(String::len).sum::<usize>();
                    let mut payload = String::with_capacity(32 + worker_bytes);
                    payload.push_str(r#"{"storage":"ssd","workers":["#);
                    for (index, worker) in workers.into_iter().enumerate() {
                        if index > 0 {
                            payload.push(',');
                        }
                        write!(payload, "{worker:?}").expect("writing to a String cannot fail");
                    }
                    payload.push_str(r#"],"observability":{"#);
                    payload.push_str(&observability.stats_json_fields());
                    payload.push_str("}}");
                    Some(response(Status::Ok, payload.into_bytes()))
                }
                Err(error) => Some(cache_error_response(error)),
            }
        }
        Opcode::Sync => {
            if !administrator {
                return Some(response_bytes(
                    Status::Forbidden,
                    b"SYNC requires administrator authorization",
                ));
            }
            if !namespace_exists(
                namespaces,
                namespace_id.expect("SYNC requests have a validated namespace ID"),
            ) {
                return Some(response_bytes(
                    Status::NamespaceNotFound,
                    b"namespace does not exist",
                ));
            }
            let namespace_id = namespace_id.expect("SYNC requests have a validated namespace ID");
            let dirty_workers = match namespaces.lock() {
                Ok(registry) => match registry.dirty_workers(namespace_id) {
                    Some(workers) => workers,
                    None => {
                        return Some(response_bytes(
                            Status::NamespaceNotFound,
                            b"namespace does not exist",
                        ));
                    }
                },
                Err(_) => {
                    return Some(response_bytes(
                        Status::InternalError,
                        b"namespace metadata is unavailable",
                    ));
                }
            };
            match cache.sync_workers(&dirty_workers).await {
                Ok(()) => {
                    let clean = namespaces
                        .lock()
                        .map_err(|_| Status::InternalError)
                        .and_then(|mut registry| registry.mark_workers_clean(namespace_id));
                    match clean {
                        Ok(()) => Some(response(Status::Ok, Vec::new())),
                        Err(_) => {
                            // The worker barrier completed, but the metadata
                            // update did not. Keep the outcome ambiguous so
                            // the next SYNC retries the conservative barrier.
                            None
                        }
                    }
                }
                Err(_) => {
                    // SYNC is a persistence barrier. A worker may have
                    // completed its flush before another worker failed, so no
                    // error response can safely claim that the barrier did
                    // not take effect.
                    None
                }
            }
        }
        _ => unreachable!("immediate response operation escaped handler dispatch"),
    }
}

/// Executes the built-in single-item GET behavior.
///
/// The wire/runtime layers only deliver the decoded operation context. The
/// storage lookup and its domain response remain a server-owned decision.
async fn execute_get(
    cache: &NetworkWorkerCache<'_>,
    namespace_id: Option<u64>,
    item_id: ItemId,
    namespaces: &Mutex<NamespaceRegistry>,
) -> Option<Response> {
    let namespace_id = namespace_id.expect("scoped value requests have a validated ID");
    if !namespace_exists(namespaces, namespace_id) {
        return Some(response_bytes(
            Status::NamespaceNotFound,
            b"namespace does not exist",
        ));
    }
    match cache.get_in_namespace(namespace_id, item_id).await {
        Ok(Some(value)) => Some(response(Status::Ok, value.into_bytes())),
        Ok(None) => {
            if let Ok(mut registry) = namespaces.lock() {
                if registry.prune_item(namespace_id, item_id).is_err() {
                    return Some(response_bytes(
                        Status::InternalError,
                        b"namespace metadata is unavailable",
                    ));
                }
            } else {
                return Some(response_bytes(
                    Status::InternalError,
                    b"namespace metadata is unavailable",
                ));
            }
            Some(response(Status::NotFound, Vec::new()))
        }
        Err(error) => Some(cache_error_response(error)),
    }
}
