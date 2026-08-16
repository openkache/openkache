//! Domain behavior implementations for modeled server operations.
//!
//! These handlers receive generated field views and storage capabilities, then
//! return transport-neutral outcomes. Wire responses, frame layouts, and client
//! result projections remain in the shared operation adapter.

use std::fmt::Write as _;
use std::future::Future;

use openkache_protocol::{ItemId, Opcode, ResponseSegment};

use super::super::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, NamespaceDescriptor,
    NamespacePolicy, OverridePolicy, SetOptions,
};
use super::operation_compatibility_decode::{
    GetInput, NamespaceDeleteInput, NamespaceInput, NamespaceOpenInput, NamespaceRevisionInput,
    SetInput,
};
use super::operation_compatibility_services::storage_write_options;
use super::operation_contract::{OperationStatus, telemetry_operation};
use super::operation_outcome::{
    OperationBody, OperationError, OperationOutcome, OperationSuccessStatus,
};
use super::operation_ports::{NamespaceCapability, ObservabilityCapability};
use super::storage_port::{
    CompatibilityStorageAddressPort, StorageAdministrationPort, StorageError, StorageMutation,
    StorageValue, StorageWriteOutcome,
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

pub(super) fn get<'a>(
    cache: &'a impl CompatibilityStorageAddressPort,
    namespaces: &'a dyn NamespaceCapability,
    decoded: GetInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move { execute_get(cache, decoded.namespace_id, decoded.item_id, namespaces).await }
}

pub(super) fn namespace_open<'a>(
    namespaces: &'a dyn NamespaceCapability,
    decoded: NamespaceOpenInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let result = namespaces.open(decoded.name, decoded.create_if_missing, decoded.policy);
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
    namespaces: &'a dyn NamespaceCapability,
    decoded: NamespaceRevisionInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let result = namespaces.update(
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

pub(super) fn namespace_delete<'a>(
    cache: &'a impl CompatibilityStorageAddressPort,
    namespaces: &'a dyn NamespaceCapability,
    decoded: NamespaceDeleteInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let namespace_id = decoded.namespace_id;
        let expected_revision = decoded.expected_revision;
        let tracked_items = match namespaces.tracked_items(namespace_id) {
            Some(items) => items,
            None => {
                return domain_error(
                    OperationStatus::InternalError,
                    b"namespace metadata is unavailable",
                );
            }
        };
        for item_id in tracked_items {
            let address =
                cache.prepare_compatibility_address(namespace_id, item_id.as_bytes());
            match cache.get(telemetry_operation(Opcode::Get), address).await {
                Ok(Some(_)) => {}
                Ok(None) => {
                    let pruned = namespaces.prune_item(namespace_id, item_id).map_err(|_| ());
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
        let result = namespaces.delete(namespace_id, expected_revision);
        match result {
            Ok(()) => domain_success(OperationStatus::Deleted, OperationBody::Empty),
            Err(error) => namespace_error(error, b"namespace deletion rejected"),
        }
    }
}

pub(super) fn set<'a>(
    cache: &'a impl CompatibilityStorageAddressPort,
    namespaces: &'a dyn NamespaceCapability,
    decoded: SetInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let namespace_id = decoded.namespace_id;
        let set_options = decoded.options;
        let policy = match namespaces.policy(namespace_id) {
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
        let address =
            cache.prepare_compatibility_address(namespace_id, item_id.as_bytes());
        let route = cache.route_for(&address);
        let reservation = match namespaces.reserve_item(namespace_id, item_id, route) {
            Ok(reservation) => reservation,
            Err(error) => {
                return namespace_error(error, b"namespace metadata is unavailable");
            }
        };
        let outcome = cache
            .set(
                telemetry_operation(Opcode::Set),
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
                let rollback =
                    namespaces.rollback_set_reservation(namespace_id, item_id, route, reservation);
                match rollback {
                    Ok(()) => domain_success(OperationStatus::NotStored, OperationBody::Empty),
                    Err(_) => OperationOutcome::abandoned(),
                }
            }
            Err(error) => {
                let response = mutation_domain_error(true, error);
                let rollback =
                    namespaces.rollback_set_reservation(namespace_id, item_id, route, reservation);
                if rollback.is_ok() {
                    response
                } else {
                    OperationOutcome::abandoned()
                }
            }
        }
    }
}

pub(super) fn delete<'a>(
    cache: &'a impl CompatibilityStorageAddressPort,
    namespaces: &'a dyn NamespaceCapability,
    decoded: GetInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let namespace_id = decoded.namespace_id;
        if !namespaces.exists(namespace_id) {
            return domain_error(
                OperationStatus::NamespaceNotFound,
                b"namespace does not exist",
            );
        }
        let item_id = decoded.item_id;
        let address =
            cache.prepare_compatibility_address(namespace_id, item_id.as_bytes());
        let route = cache.route_for(&address);
        if let Err(status) = namespaces.reserve_worker(namespace_id, route) {
            return namespace_error(status, b"namespace metadata is unavailable");
        }
        let mutation = cache
            .delete(telemetry_operation(Opcode::Delete), address)
            .await;
        match mutation {
            Ok(mutation) => {
                let deleted = mutation == StorageMutation::Applied;
                if namespaces
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

pub(super) fn stats<'a>(
    cache: &'a impl StorageAdministrationPort,
    namespaces: &'a dyn NamespaceCapability,
    observability: &'a dyn ObservabilityCapability,
    decoded: NamespaceInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let namespace_id = decoded.namespace_id;
        if !namespaces.exists(namespace_id) {
            return domain_error(
                OperationStatus::NamespaceNotFound,
                b"namespace does not exist",
            );
        }
        match cache.stats(telemetry_operation(Opcode::Stats)).await {
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
                let stats = observability.stats_snapshot();
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

pub(super) fn sync<'a>(
    cache: &'a impl StorageAdministrationPort,
    namespaces: &'a dyn NamespaceCapability,
    decoded: NamespaceInput,
) -> impl Future<Output = OperationOutcome> + 'a {
    async move {
        let namespace_id = decoded.namespace_id;
        if !namespaces.exists(namespace_id) {
            return domain_error(
                OperationStatus::NamespaceNotFound,
                b"namespace does not exist",
            );
        }
        let dirty_workers = match namespaces.dirty_workers(namespace_id) {
            Some(workers) => workers,
            None => {
                return domain_error(
                    OperationStatus::NamespaceNotFound,
                    b"namespace does not exist",
                );
            }
        };
        match cache
            .sync_routes(&dirty_workers, telemetry_operation(Opcode::Sync))
            .await
        {
            Ok(()) => {
                let clean = namespaces.mark_workers_clean(namespace_id);
                match clean {
                    Ok(()) => domain_success(OperationStatus::Ok, OperationBody::Empty),
                    Err(_) => OperationOutcome::abandoned(),
                }
            }
            Err(_) => OperationOutcome::abandoned(),
        }
    }
}

async fn execute_get(
    cache: &impl CompatibilityStorageAddressPort,
    namespace_id: u64,
    item_id: ItemId,
    namespaces: &dyn NamespaceCapability,
) -> OperationOutcome {
    if !namespaces.exists(namespace_id) {
        return domain_error(
            OperationStatus::NamespaceNotFound,
            b"namespace does not exist",
        );
    }
    let address = cache.prepare_compatibility_address(namespace_id, item_id.as_bytes());
    match cache.get(telemetry_operation(Opcode::Get), address).await {
        Ok(Some(value)) => OperationOutcome::opaque(OperationStatus::Ok, value),
        Ok(None) => {
            if namespaces.prune_item(namespace_id, item_id).is_err() {
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
