//! Domain behavior implementations for modeled server operations.
//!
//! These handlers receive generated field views and storage capabilities, then
//! return transport-neutral outcomes. Wire responses, frame layouts, and client
//! result projections remain in the shared operation adapter.

use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;

use openkache_protocol::ItemId;

use super::super::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, NamespaceDescriptor,
    NamespacePolicy, OverridePolicy, SetOptions, SetOutcome,
};
use super::operation_compatibility_bindings::{
    GetInput, NamespaceDeleteInput, NamespaceInput, NamespaceOpenInput, NamespaceRevisionInput,
    SetInput,
};
use super::operation_compatibility_services::{
    CompatibilityServices, NamespaceCapability, StorageCapability, storage_write_options,
};
use super::operation_contract::OperationStatus;
use super::operation_outcome::{
    OperationBody, OperationError, OperationOutcome, OperationSuccessStatus,
};
use super::{KvError, NamespaceError, NamespaceOpenResult};

fn descriptor_payload(descriptor: NamespaceDescriptor) -> Vec<u8> {
    descriptor
        .encode()
        .expect("validated namespace policy remains encodable")
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

fn domain_storage(error: KvError) -> OperationOutcome {
    OperationOutcome::error(storage_error(error))
}

/// Converts the storage backend's error vocabulary at the API-owned boundary.
///
/// The generic operation outcome carries only a generated semantic status and bytes;
/// it does not depend on `KvError` or construct a wire response. A future
/// backend can provide the same boundary with its own mapping.
pub(super) fn storage_error(error: KvError) -> OperationError {
    let status = match &error {
        KvError::Timeout(_) => OperationStatus::Timeout,
        KvError::NoCapacity => OperationStatus::NoCapacity,
        KvError::TableFull | KvError::CapacityExhausted { .. } => OperationStatus::Overloaded,
        KvError::ItemTooLarge { .. } | KvError::BlobSegmentFull { .. } => {
            OperationStatus::TooLarge
        }
        KvError::InvalidRequest(_) => OperationStatus::InvalidRequest,
        KvError::Io(_) | KvError::InvalidConfig(_) | KvError::Worker(_) | KvError::Usage(_) => {
            OperationStatus::InternalError
        }
    };
    OperationError::owned_status(status, error.to_string().into_bytes())
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

fn missing_services() -> OperationOutcome {
    domain_error(
        OperationStatus::InternalError,
        b"operation requires an unavailable capability",
    )
}

fn compatibility_services<'a>(
    services: Option<&'a dyn CompatibilityServices>,
) -> Result<&'a dyn CompatibilityServices, OperationOutcome> {
    services.ok_or_else(missing_services)
}

pub(super) fn mutation_domain_error(is_set: bool, error: KvError) -> OperationOutcome {
    let safe_before_mutation = matches!(
        &error,
        KvError::InvalidRequest(_)
            | KvError::TableFull
            | KvError::ItemTooLarge { .. }
            | KvError::BlobSegmentFull { .. }
            | KvError::CapacityExhausted { .. }
            | KvError::NoCapacity
    );
    if !safe_before_mutation {
        return OperationOutcome::abandoned();
    }
    if matches!(error, KvError::NoCapacity) && !is_set {
        domain_error(OperationStatus::Overloaded, b"storage has no capacity")
    } else {
        domain_storage(error)
    }
}

pub(super) fn get<'a>(
    server: Option<&'a dyn CompatibilityServices>,
    decoded: GetInput,
) -> Pin<Box<dyn Future<Output = OperationOutcome> + 'a>> {
    Box::pin(async move {
        let services = match compatibility_services(server) {
            Ok(services) => services,
            Err(error) => return error,
        };
        let cache = services.storage();
        let namespaces = services.namespaces();
        execute_get(cache, decoded.namespace_id, decoded.item_id, namespaces).await
    })
}

pub(super) fn namespace_open<'a>(
    server: Option<&'a dyn CompatibilityServices>,
    decoded: NamespaceOpenInput,
) -> Pin<Box<dyn Future<Output = OperationOutcome> + 'a>> {
    Box::pin(async move {
        let services = match compatibility_services(server) {
            Ok(services) => services,
            Err(error) => return error,
        };
        let namespaces = services.namespaces();
        let result = namespaces.open(decoded.name, decoded.create_if_missing, decoded.policy);
        match result {
            Ok((NamespaceOpenResult::Existing, descriptor)) => {
                domain_success(
                    OperationStatus::Ok,
                    OperationBody::opaque(descriptor_payload(descriptor)),
                )
            }
            Ok((NamespaceOpenResult::Created, descriptor)) => domain_success(
                OperationStatus::Created,
                OperationBody::opaque(descriptor_payload(descriptor)),
            ),
            Err(error) => namespace_error(error, b"namespace operation rejected"),
        }
    })
}

pub(super) fn namespace_update_policy<'a>(
    server: Option<&'a dyn CompatibilityServices>,
    decoded: NamespaceRevisionInput,
) -> Pin<Box<dyn Future<Output = OperationOutcome> + 'a>> {
    Box::pin(async move {
        let services = match compatibility_services(server) {
            Ok(services) => services,
            Err(error) => return error,
        };
        let namespaces = services.namespaces();
        let result = namespaces.update(
            decoded.namespace_id,
            decoded.expected_revision,
            decoded.policy,
        );
        match result {
            Ok(descriptor) => {
                domain_success(
                    OperationStatus::Ok,
                    OperationBody::opaque(descriptor_payload(descriptor)),
                )
            }
            Err(error) => namespace_error(error, b"namespace policy update rejected"),
        }
    })
}

pub(super) fn namespace_delete<'a>(
    server: Option<&'a dyn CompatibilityServices>,
    decoded: NamespaceDeleteInput,
) -> Pin<Box<dyn Future<Output = OperationOutcome> + 'a>> {
    Box::pin(async move {
        let services = match compatibility_services(server) {
            Ok(services) => services,
            Err(error) => return error,
        };
        let cache = services.storage();
        let namespaces = services.namespaces();
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
            match cache.get_in_namespace(namespace_id, item_id).await {
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
    })
}

pub(super) fn set<'a>(
    server: Option<&'a dyn CompatibilityServices>,
    decoded: SetInput,
) -> Pin<Box<dyn Future<Output = OperationOutcome> + 'a>> {
    Box::pin(async move {
        let services = match compatibility_services(server) {
            Ok(services) => services,
            Err(error) => return error,
        };
        let cache = services.storage();
        let namespaces = services.namespaces();
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
        let worker = cache.namespace_item_worker(namespace_id, item_id);
        let reservation = match namespaces.reserve_item(namespace_id, item_id, worker) {
            Ok(reservation) => reservation,
            Err(error) => {
                return namespace_error(error, b"namespace metadata is unavailable");
            }
        };
        let outcome = cache
            .set_in_namespace(
                namespace_id,
                item_id,
                super::super::types::StoredItemValue::from_owned_range(value),
                storage_options,
            )
            .await;
        match outcome {
            Ok(SetOutcome::Created) => {
                domain_success(OperationStatus::Created, OperationBody::Empty)
            }
            Ok(SetOutcome::Replaced) => {
                domain_success(OperationStatus::Replaced, OperationBody::Empty)
            }
            Ok(SetOutcome::NotStored) => {
                let rollback =
                    namespaces.rollback_set_reservation(namespace_id, item_id, worker, reservation);
                match rollback {
                    Ok(()) => domain_success(OperationStatus::NotStored, OperationBody::Empty),
                    Err(_) => OperationOutcome::abandoned(),
                }
            }
            Err(error) => {
                let response = mutation_domain_error(true, error);
                let rollback =
                    namespaces.rollback_set_reservation(namespace_id, item_id, worker, reservation);
                if rollback.is_ok() {
                    response
                } else {
                    OperationOutcome::abandoned()
                }
            }
        }
    })
}

pub(super) fn delete<'a>(
    server: Option<&'a dyn CompatibilityServices>,
    decoded: GetInput,
) -> Pin<Box<dyn Future<Output = OperationOutcome> + 'a>> {
    Box::pin(async move {
        let services = match compatibility_services(server) {
            Ok(services) => services,
            Err(error) => return error,
        };
        let cache = services.storage();
        let namespaces = services.namespaces();
        let namespace_id = decoded.namespace_id;
        if !namespaces.exists(namespace_id) {
            return domain_error(
                OperationStatus::NamespaceNotFound,
                b"namespace does not exist",
            );
        }
        let item_id = decoded.item_id;
        let worker = cache.namespace_item_worker(namespace_id, item_id);
        if let Err(status) = namespaces.reserve_worker(namespace_id, worker) {
            return namespace_error(status, b"namespace metadata is unavailable");
        }
        let deleted = cache.delete_in_namespace(namespace_id, item_id).await;
        match deleted {
            Ok(deleted) => {
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
    })
}

pub(super) fn stats<'a>(
    server: Option<&'a dyn CompatibilityServices>,
    decoded: NamespaceInput,
) -> Pin<Box<dyn Future<Output = OperationOutcome> + 'a>> {
    Box::pin(async move {
        let services = match compatibility_services(server) {
            Ok(services) => services,
            Err(error) => return error,
        };
        let cache = services.storage();
        let namespaces = services.namespaces();
        let observability = services.observability();
        let namespace_id = decoded.namespace_id;
        if !namespaces.exists(namespace_id) {
            return domain_error(
                OperationStatus::NamespaceNotFound,
                b"namespace does not exist",
            );
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
                domain_success(
                    OperationStatus::Ok,
                    OperationBody::opaque(payload.into_bytes()),
                )
            }
            Err(error) => domain_storage(error),
        }
    })
}

pub(super) fn sync<'a>(
    server: Option<&'a dyn CompatibilityServices>,
    decoded: NamespaceInput,
) -> Pin<Box<dyn Future<Output = OperationOutcome> + 'a>> {
    Box::pin(async move {
        let services = match compatibility_services(server) {
            Ok(services) => services,
            Err(error) => return error,
        };
        let cache = services.storage();
        let namespaces = services.namespaces();
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
        match cache.sync_workers(&dirty_workers).await {
            Ok(()) => {
                let clean = namespaces.mark_workers_clean(namespace_id);
                match clean {
                    Ok(()) => domain_success(OperationStatus::Ok, OperationBody::Empty),
                    Err(_) => OperationOutcome::abandoned(),
                }
            }
            Err(_) => OperationOutcome::abandoned(),
        }
    })
}

async fn execute_get(
    cache: &dyn StorageCapability,
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
    match cache.get_in_namespace(namespace_id, item_id).await {
        Ok(Some(value)) => OperationOutcome::opaque(OperationStatus::Ok, value.into_bytes()),
        Ok(None) => {
            if namespaces.prune_item(namespace_id, item_id).is_err() {
                return domain_error(
                    OperationStatus::InternalError,
                    b"namespace metadata is unavailable",
                );
            }
            domain_success(
                OperationStatus::NotFound,
                OperationBody::opaque(Vec::new()),
            )
        }
        Err(error) => domain_storage(error),
    }
}
