//! Protocol-v1 request projections for the compact compatibility ABI.
//!
//! Generic operation fields deliberately do not contain namespace policy or
//! SET option enums. This adapter is the only server-side code that projects
//! compact protocol-v1 values into the generated field plan. Typed bindings
//! decode those field values at the behavior boundary.

use openkache_protocol::{ItemId, OwnedRange};
use smallvec::SmallVec;

use super::operation_handlers::{OperationFieldRecord, OperationFieldStorage, OperationInputView};
use super::super::operation_contract as generic_contract;
use crate::protocol::{NamespacePolicy, Request, RequestHeader, ServerRequest, SetOptions};

/// Verifies the compact compatibility projection independently of the generic
/// operation registry.
///
/// Generic operations only require a framing descriptor. A compact request is
/// the one place where the historical v1 route discriminator is meaningful, so
/// keep that invariant beside the adapter that consumes it.
pub(super) fn validate_compatibility_routes() -> Result<(), &'static str> {
    for entry in generic_contract::operation_registry() {
        let registered_as_compatibility =
            super::operation_compatibility_bindings::handles(entry.opcode);
        let has_route = openkache_protocol::compat_v1::route_for_opcode(entry.opcode).is_some();
        // Some v1 convenience projections (currently PING/PONG) use generic
        // empty framing and therefore have no compact request route. They are
        // still owned by the compatibility API module, but must not be treated
        // as malformed compact registrations.
        if has_route && !registered_as_compatibility {
            return Err("compact operation has no protocol-v1 compatibility route");
        }
    }
    Ok(())
}

/// Explicit registration metadata for the historical request projection.
impl OperationInputView {
    /// Decodes a fixed-width item identifier at a generated field index.
    pub(super) fn item_id_at_index(&self, index: usize) -> Option<ItemId> {
        match self.field_at_index(index) {
            Some(value) if value.len() == openkache_protocol::ITEM_ID_BYTES => Some(ItemId::new(
                value.try_into().expect("validated item ID width"),
            )),
            _ => None,
        }
    }

    /// Decodes a compatibility-owned unsigned integer field.
    ///
    /// Numeric interpretation is deliberately kept out of the generic input
    /// view. The v1 adapter knows which generated indexes represent protocol
    /// namespace/revision metadata; generic APIs consume codec envelopes
    /// instead.
    pub(super) fn unsigned_long_at_index(&self, index: usize) -> Option<u64> {
        self.encoded_field_at_index(index)
            .and_then(|field| field.decode_u64().ok())
    }

    /// Decodes an optional compatibility-owned unsigned integer field.
    pub(super) fn unsigned_long_at_index_result(
        &self,
        index: Option<usize>,
    ) -> Result<Option<u64>, &'static [u8]> {
        index
            .and_then(|index| self.encoded_field_at_index(index))
            .map(|field| field.decode_u64())
            .transpose()
    }

    /// Decodes a compatibility-owned canonical boolean field.
    pub(super) fn boolean_at_index(
        &self,
        index: Option<usize>,
    ) -> Result<Option<bool>, &'static [u8]> {
        index
            .and_then(|index| self.encoded_field_at_index(index))
            .map(|field| field.decode_bool())
            .transpose()
    }

    /// Test and compatibility entry point for constructing a view from the
    /// protocol-v1 request projection.
    ///
    /// The implementation lives in this adapter module so the generic handler
    /// module does not acquire namespace, SET, or wire-request parameters.
    #[allow(dead_code)]
    pub(super) fn from_v1_request(
        opcode: openkache_protocol::Opcode,
        namespace_id: Option<u64>,
        item_ids: &[ItemId],
        value: Vec<u8>,
        namespace_name: Option<&[u8]>,
        namespace_policy: Option<NamespacePolicy>,
        expected_revision: Option<u64>,
        create_if_missing: bool,
        set_options: SetOptions,
    ) -> OperationInputView {
        adapt_request(ServerRequest::from_request(Request {
            opcode,
            namespace_id,
            item_ids: item_ids.to_vec(),
            set_options,
            value,
            namespace_name: namespace_name.map(|name| name.to_vec()),
            namespace_policy,
            expected_revision,
            create_if_missing,
        }))
    }
}

/// Converts the protocol-v1 semantic request into the generic field view.
///
/// The server dispatcher sees only the returned view. Keeping this destructuring
/// here prevents compact namespace/SET members from becoming part of the
/// generic handler or context signature.
pub(super) fn adapt_request(request: ServerRequest) -> OperationInputView {
    debug_assert!(
        openkache_protocol::compat_v1::route_for_opcode(request.opcode()).is_some(),
        "v1 adapter registered for a non-compatibility operation"
    );
    match request {
        ServerRequest::Frame { frame, header } => adapt_compact_frame(frame, header),
        ServerRequest::Semantic(request) => adapt_compact_request(request),
    }
}

/// Projects an admitted compact item frame without materializing semantic
/// item/value buffers. All ranges remain relative to the retained frame owner.
fn adapt_compact_frame(frame: Vec<u8>, header: RequestHeader) -> OperationInputView {
    let opcode = header.opcode();
    let plan = generic_contract::operation_wire_spec(opcode).request.fields;
    let mut fields = SmallVec::<[Option<OperationFieldRecord>; 8]>::with_capacity(plan.len());
    fields.resize_with(plan.len(), || None);
    populate_frame_fields(&frame, plan, &mut fields, header);
    let mut input =
        OperationInputView::from_populated_projection(opcode, OwnedRange::whole(frame), fields);
    input.validate_populated_fields();
    input
}

fn populate_frame_fields(
    frame: &[u8],
    plan: &'static [generic_contract::OperationFieldPlan],
    fields: &mut [Option<OperationFieldRecord>],
    header: RequestHeader,
) {
    let mut item_index = 0_usize;
    let item_start = header.item_id_start();
    let set_options = header.set_options();
    let needs_policy = plan.iter().any(|field| {
        matches!(
            field.role,
            "default_expiration"
                | "default_ttl_milliseconds"
                | "expiration_override"
                | "default_eviction"
                | "eviction_override"
        )
    });
    let namespace_policy = needs_policy
        .then(|| crate::protocol::compatibility_namespace_policy(frame, header))
        .flatten();
    for (index, field_plan) in plan.iter().enumerate() {
        let Some(slot) = fields.get_mut(index) else {
            break;
        };
        let value = match field_plan.role {
            "namespace_id" => header
                .namespace_id_range()
                .map(|range| OperationFieldStorage::OwnerRange {
                    start: range.start,
                    end: range.end,
                }),
            "item_id" => item_start.and_then(|start| {
                let start = start.checked_add(
                    item_index.checked_mul(openkache_protocol::ITEM_ID_BYTES)?,
                )?;
                item_index += 1;
                let end = start.checked_add(openkache_protocol::ITEM_ID_BYTES)?;
                (item_index <= header.item_id_count())
                    .then_some(OperationFieldStorage::OwnerRange { start, end })
            }),
            "value" => header
                .encoded_len()
                .checked_add(header.value_len())
                .map(|end| OperationFieldStorage::OwnerRange {
                    start: header.encoded_len(),
                    end,
                }),
            "condition" => Some(OperationFieldStorage::StaticBytes(set_condition_token(
                set_options.condition,
            ))),
            "expiration_mode" => Some(OperationFieldStorage::StaticBytes(expiration_mode_token(
                set_options.expiration_mode,
            ))),
            "eviction_mode" => Some(OperationFieldStorage::StaticBytes(eviction_mode_token(
                set_options.eviction_mode,
            ))),
            "ttl_milliseconds" => set_options
                .ttl_ms
                .map(|value| OperationFieldStorage::inline(value.to_be_bytes())),
            "expected_revision" => header
                .expected_revision_range()
                .map(|range| OperationFieldStorage::OwnerRange {
                    start: range.start,
                    end: range.end,
                }),
            "policy" => None,
            "default_expiration" => namespace_policy.map(|policy| {
                OperationFieldStorage::StaticBytes(default_expiration_token(
                    policy.default_expiration,
                ))
            }),
            "default_ttl_milliseconds" => {
                namespace_policy.and_then(|policy| match policy.default_expiration {
                    crate::protocol::ExpirationDefault::FixedTtl { ttl_ms } => Some(
                        OperationFieldStorage::inline(ttl_ms.to_be_bytes()),
                    ),
                    crate::protocol::ExpirationDefault::NoExpiry => None,
                })
            }
            "expiration_override" => namespace_policy.map(|policy| {
                OperationFieldStorage::StaticBytes(override_policy_token(
                    policy.expiration_override,
                ))
            }),
            "default_eviction" => namespace_policy.map(|policy| {
                OperationFieldStorage::StaticBytes(default_eviction_token(policy.default_eviction))
            }),
            "eviction_override" => namespace_policy.map(|policy| {
                OperationFieldStorage::StaticBytes(override_policy_token(policy.eviction_override))
            }),
            _ => None,
        };
        *slot = Some(OperationFieldRecord {
            plan: field_plan,
            value,
        });
    }
}

/// Projects only the historical compact-v1 request family. Empty, opaque,
/// and ordered operations never destructure semantic request fields here.
fn adapt_compact_request(request: Request) -> OperationInputView {
    let Request {
        opcode,
        namespace_id,
        item_ids,
        set_options,
        value,
        namespace_name,
        namespace_policy,
        expected_revision,
        create_if_missing,
    } = request;
    let plan = generic_contract::operation_wire_spec(opcode).request.fields;
    let mut fields = SmallVec::<[Option<OperationFieldRecord>; 8]>::with_capacity(plan.len());
    fields.resize_with(plan.len(), || None);
    let mut value = Some(value);
    populate_request_fields(
        plan,
        &mut fields,
        namespace_id,
        item_ids,
        &mut value,
        namespace_name,
        namespace_policy,
        expected_revision,
        create_if_missing,
        set_options,
    );
    let mut input = OperationInputView::from_populated_parts(opcode, fields);
    input.validate_populated_fields();
    input
}

/// Populates the generated field records for a compact protocol-v1 request.
///
/// The generic operation view owns the records and their byte storage. Every
/// semantic value needed by a compatibility behavior is represented in that
/// generated plan; no typed sidecar is attached to the generic input.
pub(super) fn populate_request_fields<I>(
    plan: &'static [generic_contract::OperationFieldPlan],
    fields: &mut [Option<OperationFieldRecord>],
    namespace_id: Option<u64>,
    item_ids: I,
    value: &mut Option<Vec<u8>>,
    namespace_name: Option<Vec<u8>>,
    namespace_policy: Option<NamespacePolicy>,
    expected_revision: Option<u64>,
    create_if_missing: bool,
    set_options: SetOptions,
) where
    I: IntoIterator<Item = ItemId>,
{
    let mut item_ids = item_ids.into_iter();
    let mut namespace_name = namespace_name;
    for (index, field_plan) in plan.iter().enumerate() {
        // Generated metadata is validated at bind time, but keep this adapter
        // fail-closed if a stale contract is loaded by a private build or
        // another composition root. `OperationInputView` reports the bound
        // error instead of allowing a malformed contract to panic the server.
        let Some(slot) = fields.get_mut(index) else {
            break;
        };
        let field = match field_plan.role {
            "namespace_id" => namespace_id
                .map(|value| OperationFieldStorage::inline(value.to_be_bytes())),
            "item_id" => item_ids
                .next()
                .map(|item_id| OperationFieldStorage::OwnedBytes(item_id.into_bytes().to_vec())),
            "value" => value.take().map(OperationFieldStorage::OwnedBytes),
            "name" => namespace_name.take().map(OperationFieldStorage::OwnedBytes),
            "expected_revision" => expected_revision
                .map(|value| OperationFieldStorage::inline(value.to_be_bytes())),
            "condition" => Some(OperationFieldStorage::StaticBytes(set_condition_token(
                set_options.condition,
            ))),
            "expiration_mode" => Some(OperationFieldStorage::StaticBytes(expiration_mode_token(
                set_options.expiration_mode,
            ))),
            "eviction_mode" => Some(OperationFieldStorage::StaticBytes(eviction_mode_token(
                set_options.eviction_mode,
            ))),
            "ttl_milliseconds" => set_options
                .ttl_ms
                .map(|value| OperationFieldStorage::inline(value.to_be_bytes())),
            "create_if_missing" => Some(OperationFieldStorage::StaticBytes(if create_if_missing {
                b"\x01"
            } else {
                b"\x00"
            })),
            // The parent structure is represented by its generated nested
            // members below. Keeping only those members avoids a second owned
            // policy buffer in the operation input.
            "policy" => None,
            "default_expiration" => namespace_policy.map(|policy| {
                OperationFieldStorage::StaticBytes(default_expiration_token(
                    policy.default_expiration,
                ))
            }),
            "default_ttl_milliseconds" => {
                namespace_policy.and_then(|policy| match policy.default_expiration {
                    crate::protocol::ExpirationDefault::FixedTtl { ttl_ms } => Some(
                        OperationFieldStorage::inline(ttl_ms.to_be_bytes()),
                    ),
                    crate::protocol::ExpirationDefault::NoExpiry => None,
                })
            }
            "expiration_override" => namespace_policy.map(|policy| {
                OperationFieldStorage::StaticBytes(override_policy_token(
                    policy.expiration_override,
                ))
            }),
            "default_eviction" => namespace_policy.map(|policy| {
                OperationFieldStorage::StaticBytes(default_eviction_token(policy.default_eviction))
            }),
            "eviction_override" => namespace_policy.map(|policy| {
                OperationFieldStorage::StaticBytes(override_policy_token(policy.eviction_override))
            }),
            _ => None,
        };
        *slot = Some(OperationFieldRecord {
            plan: field_plan,
            value: field,
        });
    }
}

const fn set_condition_token(condition: crate::protocol::SetCondition) -> &'static [u8] {
    match condition {
        crate::protocol::SetCondition::Any => b"any",
        crate::protocol::SetCondition::IfAbsent => b"if_absent",
        crate::protocol::SetCondition::IfPresent => b"if_present",
    }
}

const fn expiration_mode_token(mode: crate::protocol::ExpirationMode) -> &'static [u8] {
    match mode {
        crate::protocol::ExpirationMode::Inherit => b"inherit",
        crate::protocol::ExpirationMode::NoExpiry => b"no_expiry",
        crate::protocol::ExpirationMode::ExplicitTtl => b"explicit_ttl",
    }
}

const fn eviction_mode_token(mode: crate::protocol::EvictionMode) -> &'static [u8] {
    match mode {
        crate::protocol::EvictionMode::Inherit => b"inherit",
        crate::protocol::EvictionMode::Evictable => b"evictable",
        crate::protocol::EvictionMode::EvictionProtected => b"eviction_protected",
    }
}

const fn default_expiration_token(expiration: crate::protocol::ExpirationDefault) -> &'static [u8] {
    match expiration {
        crate::protocol::ExpirationDefault::NoExpiry => b"no_expiry",
        crate::protocol::ExpirationDefault::FixedTtl { .. } => b"fixed_ttl",
    }
}

const fn default_eviction_token(eviction: crate::protocol::EvictionDefault) -> &'static [u8] {
    match eviction {
        crate::protocol::EvictionDefault::Evictable => b"evictable",
        crate::protocol::EvictionDefault::EvictionProtected => b"eviction_protected",
    }
}

const fn override_policy_token(policy: crate::protocol::OverridePolicy) -> &'static [u8] {
    match policy {
        crate::protocol::OverridePolicy::Allowed => b"allowed",
        crate::protocol::OverridePolicy::Disallowed => b"disallowed",
    }
}
