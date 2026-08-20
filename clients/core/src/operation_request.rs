//! Client API bindings for generated request layouts.

use openkache_protocol::request_fields::{
    op_delete, op_get, op_namespace_delete, op_namespace_open, op_namespace_update_policy, op_set,
    op_stats, op_sync,
};
use openkache_protocol::{
    ItemId, MAX_OPERATION_REQUEST_FIELDS, MAX_VALUE_BYTES, Opcode, OwnedRequestFrame, WireSegment,
    encode_request_frame_with_id, wire_request_layout,
};

use crate::Operation;
use crate::request::{RequestBuilder, RequestContext};

use super::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, NamespacePolicy,
    OverridePolicy, ProtocolError, Result, SetCondition, SetWireOptions, generated_retry_policy,
};

/// One client request expressed only as generated numeric fields.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct OperationRequest {
    context: RequestContext,
    fields: [Option<WireSegment>; MAX_OPERATION_REQUEST_FIELDS],
}

impl OperationRequest {
    pub(crate) fn ping() -> Self {
        Self::new(Opcode::Ping, Operation::Ping, false, false)
    }

    pub(crate) fn get(namespace_id: u64, item_id: ItemId) -> Result<Self> {
        let mut request = Self::new(Opcode::Get, Operation::Get, false, false);
        request.insert(
            op_get::NAMESPACE_ID,
            WireSegment::inline(&namespace_id_bytes(namespace_id)?),
        );
        request.insert(op_get::ITEM_ID, WireSegment::inline(item_id.as_bytes()));
        Ok(request)
    }

    pub(crate) fn set(
        namespace_id: u64,
        item_id: ItemId,
        options: SetWireOptions,
        value: Vec<u8>,
    ) -> Result<Self> {
        validate_value_length(value.len())?;
        let namespace_id = namespace_id_bytes(namespace_id)?;
        let fields = SetFields::from_options(options)?;
        let mut request = Self::new(Opcode::Set, Operation::Set, true, false);
        request.insert(op_set::NAMESPACE_ID, WireSegment::inline(&namespace_id));
        request.insert(op_set::ITEM_ID, WireSegment::inline(item_id.as_bytes()));
        request.insert(op_set::VALUE, WireSegment::owned(value));
        request.insert(op_set::CONDITION, WireSegment::inline(fields.condition));
        request.insert(
            op_set::EXPIRATION_MODE,
            WireSegment::inline(fields.expiration_mode),
        );
        request.insert(
            op_set::EVICTION_MODE,
            WireSegment::inline(fields.eviction_mode),
        );
        if let Some(ttl_ms) = fields.ttl_ms {
            request.insert(
                op_set::TTL_MILLISECONDS,
                WireSegment::inline(&ttl_ms.to_be_bytes()),
            );
        }
        Ok(request)
    }

    pub(crate) fn delete(namespace_id: u64, item_id: ItemId) -> Result<Self> {
        let mut request = Self::new(Opcode::Delete, Operation::Delete, true, false);
        request.insert(
            op_delete::NAMESPACE_ID,
            WireSegment::inline(&namespace_id_bytes(namespace_id)?),
        );
        request.insert(op_delete::ITEM_ID, WireSegment::inline(item_id.as_bytes()));
        Ok(request)
    }

    pub(crate) fn stats(namespace_id: u64) -> Result<Self> {
        let mut request = Self::new(Opcode::Stats, Operation::Stats, false, false);
        request.insert(
            op_stats::NAMESPACE_ID,
            WireSegment::inline(&namespace_id_bytes(namespace_id)?),
        );
        Ok(request)
    }

    pub(crate) fn sync(namespace_id: u64) -> Result<Self> {
        let mut request = Self::new(Opcode::Sync, Operation::Sync, false, false);
        request.insert(
            op_sync::NAMESPACE_ID,
            WireSegment::inline(&namespace_id_bytes(namespace_id)?),
        );
        Ok(request)
    }

    pub(crate) fn namespace_open(
        name: Vec<u8>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> Result<Self> {
        validate_namespace_name(&name)?;
        let policy = match (create_if_missing, policy) {
            (true, Some(policy)) => Some(NamespacePolicyFields::new(policy)?),
            (true, None) => return Err(ProtocolError::MissingNamespacePolicy),
            (false, Some(_)) => return Err(ProtocolError::UnexpectedNamespacePolicy),
            (false, None) => None,
        };
        let mut request = Self::new(
            Opcode::NamespaceOpen,
            Operation::NamespaceOpen,
            create_if_missing,
            create_if_missing,
        );
        request.insert(op_namespace_open::NAME, WireSegment::owned(name));
        request.insert(
            op_namespace_open::CREATE_IF_MISSING,
            WireSegment::inline(&[u8::from(create_if_missing)]),
        );
        if let Some(policy) = policy {
            request.insert_namespace_policy(OPEN_POLICY_FIELDS, policy);
        }
        Ok(request)
    }

    pub(crate) fn namespace_update_policy(
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> Result<Self> {
        let namespace_id = namespace_id_bytes(namespace_id)?;
        let expected_revision = revision_bytes(expected_revision)?;
        let policy = NamespacePolicyFields::new(policy)?;
        let mut request = Self::new(
            Opcode::NamespaceUpdatePolicy,
            Operation::NamespaceUpdatePolicy,
            true,
            false,
        );
        request.insert(
            op_namespace_update_policy::NAMESPACE_ID,
            WireSegment::inline(&namespace_id),
        );
        request.insert(
            op_namespace_update_policy::EXPECTED_REVISION,
            WireSegment::inline(&expected_revision),
        );
        request.insert_namespace_policy(UPDATE_POLICY_FIELDS, policy);
        Ok(request)
    }

    pub(crate) fn namespace_delete(namespace_id: u64, expected_revision: u64) -> Result<Self> {
        let mut request =
            Self::new(Opcode::NamespaceDelete, Operation::NamespaceDelete, true, false);
        request.insert(
            op_namespace_delete::NAMESPACE_ID,
            WireSegment::inline(&namespace_id_bytes(namespace_id)?),
        );
        request.insert(
            op_namespace_delete::EXPECTED_REVISION,
            WireSegment::inline(&revision_bytes(expected_revision)?),
        );
        Ok(request)
    }

    fn new(
        opcode: Opcode,
        operation: Operation,
        mutation: bool,
        creates_resource: bool,
    ) -> Self {
        Self {
            context: RequestContext {
                opcode,
                operation,
                mutation,
                retry_policy: generated_retry_policy(opcode, creates_resource),
            },
            fields: std::array::from_fn(|_| None),
        }
    }

    fn insert(&mut self, index: usize, field: WireSegment) {
        assert!(
            index < wire_request_layout(self.context.opcode).field_count,
            "generated request field index belongs to this operation"
        );
        let slot = self
            .fields
            .get_mut(index)
            .expect("generated request field index is within the shared bound");
        assert!(
            slot.is_none(),
            "generated request field index is assigned only once"
        );
        *slot = Some(field);
    }

    fn insert_namespace_policy(
        &mut self,
        indexes: NamespacePolicyFieldIndexes,
        fields: NamespacePolicyFields,
    ) {
        self.insert(
            indexes.default_expiration,
            WireSegment::inline(fields.default_expiration),
        );
        if let Some(ttl_ms) = fields.default_ttl_ms {
            self.insert(
                indexes.default_ttl_ms,
                WireSegment::inline(&ttl_ms.to_be_bytes()),
            );
        }
        self.insert(
            indexes.expiration_override,
            WireSegment::inline(fields.expiration_override),
        );
        self.insert(
            indexes.default_eviction,
            WireSegment::inline(fields.default_eviction),
        );
        self.insert(
            indexes.eviction_override,
            WireSegment::inline(fields.eviction_override),
        );
    }
}

impl RequestBuilder for OperationRequest {
    fn context(&self) -> RequestContext {
        self.context
    }

    fn into_frame(self, request_id: u64) -> crate::Result<OwnedRequestFrame> {
        let layout = wire_request_layout(self.context.opcode);
        if self
            .fields
            .get(layout.field_count..)
            .is_some_and(|fields| fields.iter().any(Option::is_some))
        {
            return Err(crate::Error::protocol(
                openkache_protocol::ProtocolError::InvalidFieldSequence(
                    "request field index exceeds the generated layout",
                ),
            ));
        }
        encode_request_frame_with_id(
            request_id,
            self.context.opcode,
            layout,
            self.fields.into_iter().take(layout.field_count),
        )
        .map_err(crate::Error::protocol)
    }
}

struct SetFields {
    condition: &'static [u8],
    expiration_mode: &'static [u8],
    eviction_mode: &'static [u8],
    ttl_ms: Option<u64>,
}

impl SetFields {
    fn from_options(options: SetWireOptions) -> Result<Self> {
        if options.ttl_ms == Some(0) {
            return Err(ProtocolError::InvalidSetTtl);
        }
        let condition = match options.condition {
            SetCondition::Any => b"any".as_slice(),
            SetCondition::IfAbsent => b"if_absent".as_slice(),
            SetCondition::IfPresent => b"if_present".as_slice(),
        };
        let expiration_mode = match options.expiration_mode {
            ExpirationMode::Inherit if options.ttl_ms.is_none() => b"inherit".as_slice(),
            ExpirationMode::NoExpiry if options.ttl_ms.is_none() => b"no_expiry".as_slice(),
            ExpirationMode::ExplicitTtl if options.ttl_ms.is_some() => b"explicit_ttl".as_slice(),
            ExpirationMode::ExplicitTtl => return Err(ProtocolError::MissingSetTtl),
            ExpirationMode::Inherit | ExpirationMode::NoExpiry => {
                return Err(ProtocolError::UnexpectedSetTtl);
            }
        };
        let eviction_mode = match options.eviction_mode {
            EvictionMode::Inherit => b"inherit".as_slice(),
            EvictionMode::Evictable => b"evictable".as_slice(),
            EvictionMode::EvictionProtected => b"eviction_protected".as_slice(),
        };
        Ok(Self {
            condition,
            expiration_mode,
            eviction_mode,
            ttl_ms: options.ttl_ms,
        })
    }
}

#[derive(Clone, Copy)]
struct NamespacePolicyFieldIndexes {
    default_expiration: usize,
    default_ttl_ms: usize,
    expiration_override: usize,
    default_eviction: usize,
    eviction_override: usize,
}

const OPEN_POLICY_FIELDS: NamespacePolicyFieldIndexes = NamespacePolicyFieldIndexes {
    default_expiration: op_namespace_open::POLICY_DEFAULT_EXPIRATION,
    default_ttl_ms: op_namespace_open::POLICY_DEFAULT_TTL_MILLISECONDS,
    expiration_override: op_namespace_open::POLICY_EXPIRATION_OVERRIDE,
    default_eviction: op_namespace_open::POLICY_DEFAULT_EVICTION,
    eviction_override: op_namespace_open::POLICY_EVICTION_OVERRIDE,
};

const UPDATE_POLICY_FIELDS: NamespacePolicyFieldIndexes = NamespacePolicyFieldIndexes {
    default_expiration: op_namespace_update_policy::POLICY_DEFAULT_EXPIRATION,
    default_ttl_ms: op_namespace_update_policy::POLICY_DEFAULT_TTL_MILLISECONDS,
    expiration_override: op_namespace_update_policy::POLICY_EXPIRATION_OVERRIDE,
    default_eviction: op_namespace_update_policy::POLICY_DEFAULT_EVICTION,
    eviction_override: op_namespace_update_policy::POLICY_EVICTION_OVERRIDE,
};

struct NamespacePolicyFields {
    default_expiration: &'static [u8],
    default_ttl_ms: Option<u64>,
    expiration_override: &'static [u8],
    default_eviction: &'static [u8],
    eviction_override: &'static [u8],
}

impl NamespacePolicyFields {
    fn new(policy: NamespacePolicy) -> Result<Self> {
        let (default_expiration, default_ttl_ms) = match policy.default_expiration {
            ExpirationDefault::NoExpiry => (b"no_expiry".as_slice(), None),
            ExpirationDefault::FixedTtl { ttl_ms: 0 } => {
                return Err(ProtocolError::InvalidNamespacePolicy(
                    "fixed namespace TTL must be positive",
                ));
            }
            ExpirationDefault::FixedTtl { ttl_ms } => (b"fixed_ttl".as_slice(), Some(ttl_ms)),
        };
        Ok(Self {
            default_expiration,
            default_ttl_ms,
            expiration_override: match policy.expiration_override {
                OverridePolicy::Allowed => b"allowed",
                OverridePolicy::Disallowed => b"disallowed",
            },
            default_eviction: match policy.default_eviction {
                EvictionDefault::Evictable => b"evictable",
                EvictionDefault::EvictionProtected => b"eviction_protected",
            },
            eviction_override: match policy.eviction_override {
                OverridePolicy::Allowed => b"allowed",
                OverridePolicy::Disallowed => b"disallowed",
            },
        })
    }
}

fn namespace_id_bytes(namespace_id: u64) -> Result<[u8; 8]> {
    if namespace_id == 0 {
        return Err(ProtocolError::InvalidNamespaceId);
    }
    Ok(namespace_id.to_be_bytes())
}

fn revision_bytes(revision: u64) -> Result<[u8; 8]> {
    if revision == 0 {
        return Err(ProtocolError::InvalidRevision);
    }
    Ok(revision.to_be_bytes())
}

fn validate_namespace_name(name: &[u8]) -> Result<()> {
    if name.len() > usize::from(u8::MAX) {
        return Err(ProtocolError::InvalidNamespaceName(
            "namespace name exceeds 255 octets",
        ));
    }
    if std::str::from_utf8(name).is_err() {
        return Err(ProtocolError::InvalidNamespaceName(
            "namespace name is not UTF-8",
        ));
    }
    Ok(())
}

fn validate_value_length(value_len: usize) -> Result<()> {
    if value_len > MAX_VALUE_BYTES {
        return Err(openkache_protocol::ProtocolError::ValueTooLarge {
            size: value_len,
            maximum: MAX_VALUE_BYTES,
        }
        .into());
    }
    Ok(())
}
