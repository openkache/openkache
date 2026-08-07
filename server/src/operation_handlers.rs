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

use super::operation_extensions::ExtensionResponse;
use super::{
    NamespaceRegistry, NetworkWorkerCache, ObservabilityState, SetOutcome, cache_error_response,
    descriptor_payload, mutation_cache_error_response, namespace_exists, resolve_set_options,
    response, response_bytes,
};
use crate::protocol::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, ItemId, NamespacePolicy,
    OverridePolicy, Response, SetCondition, SetOptions, field_values_response,
};

/// Borrowed context passed from the protocol server to one concrete handler.
///
/// The context deliberately contains storage primitives and decoded request
/// fields, rather than exposing transport or frame details to API handlers.
pub(super) struct OperationInputView<'a> {
    pub(super) opcode: Opcode,
    fields: [Option<OperationFieldRecord<'a>>; crate::contract::MAX_OPERATION_REQUEST_FIELDS],
    field_len: usize,
}

/// One generated plan entry and its decoded semantic value.
///
/// Records are ordered exactly like the Smithy request plan. Optional fields
/// remain present with `None`, so a server extension can distinguish a missing
/// optional member from a zero/empty value without an operation-specific
/// context union.
pub(super) struct OperationFieldRecord<'a> {
    pub(super) plan: &'static crate::contract::OperationFieldPlan,
    value: Option<OperationFieldStorage<'a>>,
}

/// Storage owned or borrowed by one operation-field record.
///
/// Application/SET payloads stay owned so dispatch can move the existing
/// request allocation into storage without copying. All other values are
/// compact scalar or borrowed views over the decoded request.
enum OperationFieldStorage<'a> {
    UnsignedLong(u64),
    ItemId(ItemId),
    Bytes(&'a [u8]),
    OwnedBytes(Vec<u8>),
    Policy(NamespacePolicy),
    ExpirationDefault(ExpirationDefault),
    OverridePolicy(OverridePolicy),
    EvictionDefault(EvictionDefault),
    SetCondition(SetCondition),
    ExpirationMode(ExpirationMode),
    EvictionMode(EvictionMode),
    Boolean(bool),
}

/// A typed, borrowed operation field exposed to server-owned behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OperationFieldValue<'a> {
    UnsignedLong(u64),
    ItemId(ItemId),
    Bytes(&'a [u8]),
    Policy(NamespacePolicy),
    ExpirationDefault(ExpirationDefault),
    OverridePolicy(OverridePolicy),
    EvictionDefault(EvictionDefault),
    SetCondition(SetCondition),
    ExpirationMode(ExpirationMode),
    EvictionMode(EvictionMode),
    Boolean(bool),
}

impl<'a> OperationInputView<'a> {
    /// Builds a generated field view from the decoded v1 request primitives.
    ///
    /// The v1 parser still owns its compact transport layouts, but the
    /// handler boundary no longer mirrors those layouts with a fixed Rust
    /// struct. Every modeled plan entry gets one record; adding a role that
    /// reuses an existing v1 primitive therefore changes only the generated
    /// plan and the API-owned behavior.
    pub(super) fn from_request(
        opcode: Opcode,
        namespace_id: Option<u64>,
        item_ids: &'a [ItemId],
        value: Vec<u8>,
        namespace_name: Option<&'a [u8]>,
        namespace_policy: Option<NamespacePolicy>,
        expected_revision: Option<u64>,
        create_if_missing: bool,
        set_options: SetOptions,
    ) -> OperationInputView<'a> {
        let mut item_index = 0;
        let mut value = Some(value);
        let plan = crate::contract::operation_contract(opcode).request_plan;
        let mut fields = std::array::from_fn(|_| None);
        for (index, plan) in plan.iter().enumerate() {
            assert!(
                index < fields.len(),
                "generated operation request field bound is stale"
            );
            let field = match plan.role {
                "namespace_id" => namespace_id.map(OperationFieldStorage::UnsignedLong),
                "item_id" => {
                    let value = item_ids
                        .get(item_index)
                        .copied()
                        .map(OperationFieldStorage::ItemId);
                    item_index += 1;
                    value
                }
                "value" | "payload" => value.take().map(OperationFieldStorage::OwnedBytes),
                "name" => namespace_name.map(OperationFieldStorage::Bytes),
                "policy" => namespace_policy.map(OperationFieldStorage::Policy),
                "default_expiration" => namespace_policy.map(|policy| {
                    OperationFieldStorage::ExpirationDefault(policy.default_expiration)
                }),
                "default_ttl_milliseconds" => {
                    namespace_policy.and_then(|policy| match policy.default_expiration {
                        ExpirationDefault::FixedTtl { ttl_ms } => {
                            Some(OperationFieldStorage::UnsignedLong(ttl_ms))
                        }
                        ExpirationDefault::NoExpiry => None,
                    })
                }
                "expiration_override" => namespace_policy.map(|policy| {
                    OperationFieldStorage::OverridePolicy(policy.expiration_override)
                }),
                "default_eviction" => namespace_policy
                    .map(|policy| OperationFieldStorage::EvictionDefault(policy.default_eviction)),
                "eviction_override" => namespace_policy
                    .map(|policy| OperationFieldStorage::OverridePolicy(policy.eviction_override)),
                "expected_revision" => expected_revision.map(OperationFieldStorage::UnsignedLong),
                "condition" => Some(OperationFieldStorage::SetCondition(set_options.condition)),
                "expiration_mode" => Some(OperationFieldStorage::ExpirationMode(
                    set_options.expiration_mode,
                )),
                "eviction_mode" => Some(OperationFieldStorage::EvictionMode(
                    set_options.eviction_mode,
                )),
                "ttl_milliseconds" => set_options.ttl_ms.map(OperationFieldStorage::UnsignedLong),
                "create_if_missing" => Some(OperationFieldStorage::Boolean(create_if_missing)),
                _ => None,
            };
            fields[index] = Some(OperationFieldRecord { plan, value: field });
        }
        let field_len = plan.len();
        Self {
            opcode,
            fields,
            field_len,
        }
    }

    /// Returns the generated role metadata for this operation.
    ///
    /// Server extensions can inspect this open role list without adding a
    /// field to the shared operation context. Repeated roles remain a
    /// cardinality in the generated contract rather than an operation-name
    /// special case.
    pub(super) fn fields(&self) -> &'static [crate::contract::OperationFieldPlan] {
        crate::contract::operation_contract(self.opcode).request_plan
    }

    /// Returns the decoded ordered records, including missing optional fields.
    #[allow(dead_code)]
    pub(super) fn records(&self) -> &[Option<OperationFieldRecord<'_>>] {
        &self.fields[..self.field_len]
    }

    /// Returns the modeled cardinality for one semantic role.
    pub(super) fn field_count(&self, role: &str) -> usize {
        self.fields()
            .iter()
            .filter(|field| field.role == role)
            .count()
    }

    /// Returns one ordered generated plan entry for a semantic role.
    ///
    /// The ordinal is based on the Smithy field order, so repeated roles do
    /// not require an operation-specific accessor or a second central enum.
    pub(super) fn field_plan_at(
        &self,
        role: &str,
        index: usize,
    ) -> Option<&'static crate::contract::OperationFieldPlan> {
        self.fields()
            .iter()
            .filter(|field| field.role == role)
            .nth(index)
    }

    /// Returns the modeled value for a role without requiring a wire-family
    /// or operation-name match in the extension.
    pub(super) fn field(&self, role: &str) -> Option<OperationFieldValue<'_>> {
        self.field_at(role, 0)
    }

    /// Returns one ordered occurrence of a modeled role.
    ///
    /// Repeated fields are addressed by ordinal rather than by an
    /// operation-specific accessor. Existing storage fields retain their
    /// zero-copy slices; callers that need a new wire primitive can add a
    /// codec-backed `OperationFieldValue` without changing the dispatch path.
    pub(super) fn field_at(&self, role: &str, index: usize) -> Option<OperationFieldValue<'_>> {
        self.field_plan_at(role, index)?;
        self.fields
            .iter()
            .take(self.field_len)
            .filter_map(Option::as_ref)
            .filter(|field| field.plan.role == role)
            .nth(index)?
            .value
            .as_ref()
            .map(OperationFieldStorage::as_value)
    }

    /// Returns one namespace/item-style unsigned long field.
    pub(super) fn unsigned_long(&self, role: &str) -> Option<u64> {
        match self.field(role) {
            Some(OperationFieldValue::UnsignedLong(value)) => Some(value),
            _ => None,
        }
    }

    /// Returns one item ID field by generated role ordinal.
    pub(super) fn item_id_at(&self, index: usize) -> Option<ItemId> {
        match self.field_at("item_id", index) {
            Some(OperationFieldValue::ItemId(value)) => Some(value),
            _ => None,
        }
    }

    /// Returns one borrowed byte field by generated role.
    pub(super) fn bytes(&self, role: &str) -> Option<&[u8]> {
        match self.field(role) {
            Some(OperationFieldValue::Bytes(value)) => Some(value),
            _ => None,
        }
    }

    /// Returns the modeled namespace policy field.
    pub(super) fn policy(&self) -> Option<NamespacePolicy> {
        match self.field("policy") {
            Some(OperationFieldValue::Policy(value)) => Some(value),
            _ => None,
        }
    }

    /// Returns the modeled boolean field.
    pub(super) fn boolean(&self, role: &str) -> Option<bool> {
        match self.field(role) {
            Some(OperationFieldValue::Boolean(value)) => Some(value),
            _ => None,
        }
    }

    /// Reconstructs the existing SET semantic options from independent fields.
    pub(super) fn set_options(&self) -> SetOptions {
        let condition = match self.field("condition") {
            Some(OperationFieldValue::SetCondition(value)) => value,
            _ => SetCondition::Any,
        };
        let expiration_mode = match self.field("expiration_mode") {
            Some(OperationFieldValue::ExpirationMode(value)) => value,
            _ => ExpirationMode::Inherit,
        };
        let ttl_ms = self.unsigned_long("ttl_milliseconds");
        let eviction_mode = match self.field("eviction_mode") {
            Some(OperationFieldValue::EvictionMode(value)) => value,
            _ => EvictionMode::Inherit,
        };
        SetOptions::with_policies(condition, expiration_mode, ttl_ms, eviction_mode)
    }

    /// Moves an owned payload out of one generated value role without copying.
    pub(super) fn take_owned_bytes(&mut self, role: &str) -> Option<Vec<u8>> {
        let record = self
            .fields
            .iter_mut()
            .take(self.field_len)
            .filter_map(Option::as_mut)
            .find(|field| field.plan.role == role)?;
        let value = record.value.take()?;
        match value {
            OperationFieldStorage::OwnedBytes(value) => Some(value),
            other => {
                record.value = Some(other);
                None
            }
        }
    }
}

impl OperationFieldStorage<'_> {
    fn as_value(&self) -> OperationFieldValue<'_> {
        match self {
            Self::UnsignedLong(value) => OperationFieldValue::UnsignedLong(*value),
            Self::ItemId(value) => OperationFieldValue::ItemId(*value),
            Self::Bytes(value) => OperationFieldValue::Bytes(value),
            Self::OwnedBytes(value) => OperationFieldValue::Bytes(value),
            Self::Policy(value) => OperationFieldValue::Policy(*value),
            Self::ExpirationDefault(value) => OperationFieldValue::ExpirationDefault(*value),
            Self::OverridePolicy(value) => OperationFieldValue::OverridePolicy(*value),
            Self::EvictionDefault(value) => OperationFieldValue::EvictionDefault(*value),
            Self::SetCondition(value) => OperationFieldValue::SetCondition(*value),
            Self::ExpirationMode(value) => OperationFieldValue::ExpirationMode(*value),
            Self::EvictionMode(value) => OperationFieldValue::EvictionMode(*value),
            Self::Boolean(value) => OperationFieldValue::Boolean(*value),
        }
    }
}

pub(super) struct OperationContext<'a, 'cache> {
    pub(super) cache: &'a NetworkWorkerCache<'cache>,
    pub(super) opcode: Opcode,
    pub(super) input: OperationInputView<'a>,
    pub(super) administrator: bool,
    pub(super) namespaces: &'a Mutex<NamespaceRegistry>,
    pub(super) observability: &'a ObservabilityState,
}

/// Returns whether an operation can be answered without touching storage.
pub(super) fn is_immediate(opcode: Opcode) -> bool {
    let contract = crate::contract::operation_contract(opcode);
    (contract.request_kind == "empty" && contract.response_kind == "pong")
        || (contract.request_kind == "application_value"
            && contract.response_kind == "application_value")
}

fn is_builtin(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Ping
            | Opcode::Get
            | Opcode::Set
            | Opcode::Delete
            | Opcode::Stats
            | Opcode::Sync
            | Opcode::NamespaceOpen
            | Opcode::NamespaceUpdatePolicy
            | Opcode::NamespaceDelete
    )
}

/// Verifies that every modeled opcode has a server-owned execution path.
///
/// This runs during server bind rather than allowing an omitted extension to
/// reach a panic or an accidental fallback response.
pub(super) fn validate_handler_registry() -> Result<(), &'static str> {
    for value in u8::MIN..=u8::MAX {
        let Ok(opcode) = Opcode::try_from(value) else {
            continue;
        };
        if is_builtin(opcode) || super::operation_extensions::handles(opcode) {
            continue;
        }
        return Err("modeled operation has no registered server handler");
    }
    Ok(())
}

/// Executes an already-classified immediate operation.
pub(super) fn immediate_response(opcode: Opcode, value: Vec<u8>) -> Response {
    let contract = crate::contract::operation_contract(opcode);
    if contract.response_kind == "pong" {
        return response_bytes(Status::Ok, b"PONG");
    }
    if contract.response_kind == "application_value" {
        return match super::operation_extensions::application_value(opcode, value) {
            Some(extension) => encode_extension_response(opcode, extension),
            None => response_bytes(
                Status::InternalError,
                b"application-value operation has no server implementation",
            ),
        };
    }
    response_bytes(
        Status::InternalError,
        b"invalid immediate operation contract",
    )
}

fn encode_extension_response(opcode: Opcode, extension: ExtensionResponse) -> Response {
    match extension {
        ExtensionResponse::Response(response) => response,
        ExtensionResponse::ApplicationValue(value) => response(Status::Ok, value),
        ExtensionResponse::FieldValues(values) => field_values_response(opcode, &values),
    }
}

/// Dispatches a decoded, non-immediate request to server-owned behavior.
pub(super) async fn execute(context: OperationContext<'_, '_>) -> Option<Response> {
    if let Some(extension) = super::operation_extensions::execute(&context).await {
        return Some(encode_extension_response(context.opcode, extension));
    }
    let OperationContext {
        cache,
        opcode,
        input,
        administrator,
        namespaces,
        observability,
    } = context;
    let mut input = input;
    let namespace_id = input.unsigned_long("namespace_id");
    let expected_revision = input.unsigned_long("expected_revision");
    let namespace_name = input.bytes("name");
    let namespace_policy = input.policy();
    let create_if_missing = input.boolean("create_if_missing").unwrap_or(false);
    let set_options = input.set_options();

    match opcode {
        Opcode::Get => {
            let item_id = input
                .item_id_at(0)
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
            let item_id = input
                .item_id_at(0)
                .expect("SET requests have a validated item ID");
            let value = input
                .take_owned_bytes("value")
                .expect("SET requests have a validated value");
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
            let item_id = input
                .item_id_at(0)
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
        _ => Some(response_bytes(
            Status::InternalError,
            b"operation has no server implementation",
        )),
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
