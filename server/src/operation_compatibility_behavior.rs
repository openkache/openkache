//! Domain behavior implementations for modeled server operations.
//!
//! These handlers receive generated field views and storage capabilities, then
//! return transport-neutral outcomes. Wire responses, frame layouts, and client
//! result projections remain in the shared operation adapter.

use std::fmt::Write as _;
use std::future::Future;

use openkache_protocol::ResponseSegment;

use super::super::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, NamespaceDescriptor,
    NamespacePolicy, OverridePolicy, SetOptions,
};
use super::operation_compatibility_decode::{
    GetInput, NamespaceDeleteInput, NamespaceInput, NamespaceOpenInput, NamespaceRevisionInput,
    SetInput,
};
use super::operation_compatibility_services::{
    DeleteState, GetState, NamespaceDeleteState, NamespaceOpenState, NamespaceUpdateState,
    SetState, StatsState, SyncState, storage_write_options,
};
use super::operation_contract::{OperationId, OperationStatus, telemetry_operation_id};
use super::operation_outcome::{
    OperationBody, OperationError, OperationOutcome, OperationSuccessStatus,
};
use super::storage_port::{
    PreparedStorageAddress, StorageAdministrationPort, StorageDataPort, StorageError,
    StorageMutation, StorageScope, StorageValue, StorageWriteOutcome,
};
use super::{NamespaceError, NamespaceOpenResult};

fn descriptor_payload(descriptor: NamespaceDescriptor) -> ResponseSegment {
    descriptor
        .encode_inline()
        .expect("validated namespace policy remains encodable")
        .into()
}

fn resolve_set_options(
    policy: NamespacePolicy,
    options: SetOptions,
) -> std::result::Result<SetOptions, NamespaceError> {
    if options.expiration_mode != ExpirationMode::Inherit
        && policy.expiration_override == OverridePolicy::Disallowed
    {
        return Err(NamespaceError::PolicyConflict);
    }
    if options.eviction_mode != EvictionMode::Inherit
        && policy.eviction_override == OverridePolicy::Disallowed
    {
        return Err(NamespaceError::PolicyConflict);
    }
    let (expiration_mode, ttl_ms) = match options.expiration_mode {
        ExpirationMode::Inherit => match policy.default_expiration {
            ExpirationDefault::NoExpiry => (ExpirationMode::NoExpiry, None),
            ExpirationDefault::FixedTtl { ttl_ms } => (ExpirationMode::ExplicitTtl, Some(ttl_ms)),
        },
        ExpirationMode::NoExpiry => (ExpirationMode::NoExpiry, None),
        ExpirationMode::ExplicitTtl => (ExpirationMode::ExplicitTtl, options.ttl_ms),
    };
    let eviction_mode = match options.eviction_mode {
        EvictionMode::Inherit => match policy.default_eviction {
            EvictionDefault::Evictable => EvictionMode::Evictable,
            EvictionDefault::EvictionProtected => EvictionMode::EvictionProtected,
        },
        selected => selected,
    };
    Ok(SetOptions::with_policies(
        options.condition,
        expiration_mode,
        ttl_ms,
        eviction_mode,
    ))
}

fn domain_success(status: OperationSuccessStatus, body: OperationBody) -> OperationOutcome {
    OperationOutcome::success(status, body)
}

fn domain_error(status: OperationStatus, message: &'static [u8]) -> OperationOutcome {
    OperationOutcome::error(OperationError::status(status, message))
}

fn domain_storage(error: StorageError) -> OperationOutcome {
    OperationOutcome::error(storage_error(error))
}

fn item_address<S: StorageDataPort>(
    storage: &S,
    namespace_id: u64,
    item_id: &[u8],
) -> PreparedStorageAddress {
    let namespace_scope = namespace_id.to_be_bytes();
    storage.prepare_address(StorageScope::from_borrowed(&namespace_scope), item_id)
}

/// Converts the storage backend's error vocabulary at the API-owned boundary.
///
/// The generic operation outcome carries only a generated semantic status and bytes;
/// it does not depend on `KvError` or construct a wire response. A future
/// backend can provide the same boundary with its own mapping.
pub(super) fn storage_error(error: StorageError) -> OperationError {
    let status = match &error {
        StorageError::Timeout(_) => OperationStatus::Timeout,
        StorageError::NoCapacity(_) => OperationStatus::NoCapacity,
        StorageError::Overloaded(_) => OperationStatus::Overloaded,
        StorageError::TooLarge(_) => OperationStatus::TooLarge,
        StorageError::InvalidRequest(_) => OperationStatus::InvalidRequest,
        StorageError::Worker(_) | StorageError::Backend(_) => OperationStatus::InternalError,
    };
    OperationError::owned_status(status, error.into_message().into_bytes())
}

fn namespace_error(error: NamespaceError, message: &'static [u8]) -> OperationOutcome {
    let status = match error {
        NamespaceError::InvalidRequest => OperationStatus::InvalidRequest,
        NamespaceError::NotFound => OperationStatus::NamespaceNotFound,
        NamespaceError::Conflict => OperationStatus::Conflict,
        NamespaceError::PolicyConflict => OperationStatus::PolicyConflict,
        NamespaceError::NotEmpty => OperationStatus::NamespaceNotEmpty,
        NamespaceError::Internal => OperationStatus::InternalError,
    };
    domain_error(status, message)
}

pub(super) fn mutation_domain_error(is_set: bool, error: StorageError) -> OperationOutcome {
    let safe_before_mutation = matches!(
        &error,
        StorageError::InvalidRequest(_)
            | StorageError::NoCapacity(_)
            | StorageError::Overloaded(_)
            | StorageError::TooLarge(_)
    );
    if !safe_before_mutation {
        return OperationOutcome::abandoned();
    }
    if matches!(error, StorageError::NoCapacity(_)) && !is_set {
        domain_error(OperationStatus::Overloaded, b"storage has no capacity")
    } else {
        domain_storage(error)
    }
}

pub(super) fn get<'a, S: StorageDataPort>(
    state: &'a GetState<S>,
    decoded: GetInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let namespace_id = decoded.namespace_id;
        if !state.catalog.exists(namespace_id) {
            return domain_error(
                OperationStatus::NamespaceNotFound,
                b"namespace does not exist",
            );
        }
        let item_id = decoded.item_id;
        let address = item_address(&state.storage, namespace_id, item_id.as_bytes());
        match state
            .storage
            .get(telemetry_operation_id(OperationId::Get), address)
            .await
        {
            Ok(Some(value)) => OperationOutcome::opaque(OperationStatus::Ok, value),
            Ok(None) => {
                if state.membership.prune_item(namespace_id, item_id).is_err() {
                    return domain_error(
                        OperationStatus::InternalError,
                        b"namespace metadata is unavailable",
                    );
                }
                domain_success(OperationStatus::NotFound, OperationBody::opaque(Vec::new()))
            }
            Err(error) => domain_storage(error),
        }
    }
}

pub(super) fn namespace_open<'a>(
    state: &'a NamespaceOpenState,
    decoded: NamespaceOpenInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let catalog = state.catalog.as_ref();
        let result = catalog.open(decoded.name, decoded.create_if_missing, decoded.policy);
        match result {
            Ok((NamespaceOpenResult::Existing, descriptor)) => domain_success(
                OperationStatus::Ok,
                OperationBody::opaque(descriptor_payload(descriptor)),
            ),
            Ok((NamespaceOpenResult::Created, descriptor)) => domain_success(
                OperationStatus::Created,
                OperationBody::opaque(descriptor_payload(descriptor)),
            ),
            Err(error) => namespace_error(error, b"namespace operation rejected"),
        }
    }
}

pub(super) fn namespace_update_policy<'a>(
    state: &'a NamespaceUpdateState,
    decoded: NamespaceRevisionInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let catalog = state.catalog.as_ref();
        let result = catalog.update(
            decoded.namespace_id,
            decoded.expected_revision,
            decoded.policy,
        );
        match result {
            Ok(descriptor) => domain_success(
                OperationStatus::Ok,
                OperationBody::opaque(descriptor_payload(descriptor)),
            ),
            Err(error) => namespace_error(error, b"namespace policy update rejected"),
        }
    }
}

pub(super) fn namespace_delete<'a, S: StorageDataPort>(
    state: &'a NamespaceDeleteState<S>,
    decoded: NamespaceDeleteInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let cache = &state.storage;
        let namespace_id = decoded.namespace_id;
        let expected_revision = decoded.expected_revision;
        let tracked_items = match state.membership.tracked_items(namespace_id) {
            Some(items) => items,
            None => {
                return domain_error(
                    OperationStatus::InternalError,
                    b"namespace metadata is unavailable",
                );
            }
        };
        for item_id in tracked_items {
            let address = item_address(cache, namespace_id, item_id.as_bytes());
            match cache
                .get(telemetry_operation_id(OperationId::Get), address)
                .await
            {
                Ok(Some(_)) => {}
                Ok(None) => {
                    let pruned = state
                        .membership
                        .prune_item(namespace_id, item_id)
                        .map_err(|_| ());
                    if pruned.is_err() {
                        return domain_error(
                            OperationStatus::InternalError,
                            b"namespace metadata is unavailable",
                        );
                    }
                }
                Err(error) => return domain_storage(error),
            }
        }
        let result = state.catalog.delete(namespace_id, expected_revision);
        match result {
            Ok(()) => domain_success(OperationStatus::Deleted, OperationBody::Empty),
            Err(error) => namespace_error(error, b"namespace deletion rejected"),
        }
    }
}

pub(super) fn set<'a, S: StorageDataPort>(
    state: &'a SetState<S>,
    decoded: SetInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let cache = &state.storage;
        let catalog = state.catalog.as_ref();
        let namespace_id = decoded.namespace_id;
        let set_options = decoded.options;
        let policy = match catalog.policy(namespace_id) {
            Some(policy) => policy,
            None => {
                return domain_error(
                    OperationStatus::NamespaceNotFound,
                    b"namespace does not exist",
                );
            }
        };
        let effective_options = match resolve_set_options(policy, set_options) {
            Ok(options) => options,
            Err(error) => return namespace_error(error, b"SET policy is disallowed"),
        };
        let storage_options = match storage_write_options(effective_options) {
            Ok(options) => options,
            Err(message) => return domain_error(OperationStatus::InternalError, message),
        };
        let item_id = decoded.item_id;
        let value = decoded.value;
        let address = item_address(cache, namespace_id, item_id.as_bytes());
        let route = cache.route_for(&address);
        let reservation = match state.membership.reserve_item(namespace_id, item_id, route) {
            Ok(reservation) => reservation,
            Err(error) => {
                return namespace_error(error, b"namespace metadata is unavailable");
            }
        };
        let outcome = cache
            .set(
                telemetry_operation_id(OperationId::Set),
                address,
                StorageValue::from_owned_range(value),
                storage_options,
            )
            .await;
        match outcome {
            Ok(StorageWriteOutcome::Created) => {
                domain_success(OperationStatus::Created, OperationBody::Empty)
            }
            Ok(StorageWriteOutcome::Replaced) => {
                domain_success(OperationStatus::Replaced, OperationBody::Empty)
            }
            Ok(StorageWriteOutcome::Unchanged) => {
                let rollback = state.membership.rollback_set_reservation(
                    namespace_id,
                    item_id,
                    route,
                    reservation,
                );
                match rollback {
                    Ok(()) => domain_success(OperationStatus::NotStored, OperationBody::Empty),
                    Err(_) => OperationOutcome::abandoned(),
                }
            }
            Err(error) => {
                let response = mutation_domain_error(true, error);
                let rollback = state.membership.rollback_set_reservation(
                    namespace_id,
                    item_id,
                    route,
                    reservation,
                );
                if rollback.is_ok() {
                    response
                } else {
                    OperationOutcome::abandoned()
                }
            }
        }
    }
}

pub(super) fn delete<'a, S: StorageDataPort>(
    state: &'a DeleteState<S>,
    decoded: GetInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let cache = &state.storage;
        let catalog = state.catalog.as_ref();
        let namespace_id = decoded.namespace_id;
        if !catalog.exists(namespace_id) {
            return domain_error(
                OperationStatus::NamespaceNotFound,
                b"namespace does not exist",
            );
        }
        let item_id = decoded.item_id;
        let address = item_address(cache, namespace_id, item_id.as_bytes());
        let route = cache.route_for(&address);
        if let Err(status) = state.membership.reserve_worker(namespace_id, route) {
            return namespace_error(status, b"namespace metadata is unavailable");
        }
        let mutation = cache
            .delete(telemetry_operation_id(OperationId::Delete), address)
            .await;
        match mutation {
            Ok(mutation) => {
                let deleted = mutation == StorageMutation::Applied;
                if state
                    .membership
                    .mark_delete(namespace_id, item_id, deleted)
                    .is_err()
                {
                    return OperationOutcome::abandoned();
                }
                domain_success(
                    if deleted {
                        OperationStatus::Deleted
                    } else {
                        OperationStatus::NotFound
                    },
                    OperationBody::Empty,
                )
            }
            Err(error) => mutation_domain_error(false, error),
        }
    }
}

pub(super) fn stats<'a, S: StorageAdministrationPort>(
    state: &'a StatsState<S>,
    decoded: NamespaceInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let cache = &state.storage;
        let catalog = state.catalog.as_ref();
        let namespace_id = decoded.namespace_id;
        if !catalog.exists(namespace_id) {
            return domain_error(
                OperationStatus::NamespaceNotFound,
                b"namespace does not exist",
            );
        }
        match cache
            .stats(telemetry_operation_id(OperationId::Stats))
            .await
        {
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
                let stats = state.observability.stats_snapshot();
                write!(
                payload,
                r#""schema_version":1,"uptime_seconds":{:.3},"ready":{},"degraded":{},"active_connections":{},"active_streams":{},"requests_total":{}"#,
                stats.uptime_seconds,
                stats.ready,
                stats.degraded,
                stats.active_connections,
                stats.active_streams,
                stats.requests_total,
            )
            .expect("writing statistics to a String cannot fail");
                payload.push_str("}}");
                domain_success(
                    OperationStatus::Ok,
                    OperationBody::opaque(payload.into_bytes()),
                )
            }
            Err(error) => domain_storage(error),
        }
    }
}

pub(super) fn sync<'a, S: StorageAdministrationPort>(
    state: &'a SyncState<S>,
    decoded: NamespaceInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let cache = &state.storage;
        let catalog = state.catalog.as_ref();
        let namespace_id = decoded.namespace_id;
        if !catalog.exists(namespace_id) {
            return domain_error(
                OperationStatus::NamespaceNotFound,
                b"namespace does not exist",
            );
        }
        let dirty_workers = match state.membership.dirty_workers(namespace_id) {
            Some(workers) => workers,
            None => {
                return domain_error(
                    OperationStatus::NamespaceNotFound,
                    b"namespace does not exist",
                );
            }
        };
        match cache
            .sync_routes(
                &dirty_workers,
                telemetry_operation_id(OperationId::Sync),
            )
            .await
        {
            Ok(()) => {
                let clean = state.membership.mark_workers_clean(namespace_id);
                match clean {
                    Ok(()) => domain_success(OperationStatus::Ok, OperationBody::Empty),
                    Err(_) => OperationOutcome::abandoned(),
                }
            }
            Err(_) => OperationOutcome::abandoned(),
        }
    }
}
