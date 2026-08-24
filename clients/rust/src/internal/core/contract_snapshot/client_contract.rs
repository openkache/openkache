// Generated from the OpenKache client Smithy contract. Do not edit.

/// Maximum UTF-8 octets accepted in a namespace name.
pub const NAMESPACE_NAME_MAX_BYTES: usize = 255;

/// Width of the SET flags field.
pub const SET_FLAGS_BYTES: usize = 1;
pub const SET_CONDITION_MASK: u8 = 0x03;
pub const SET_CONDITION_ANY_BITS: u8 = 0x00;
pub const SET_IF_ABSENT_BITS: u8 = 0x01;
pub const SET_IF_PRESENT_BITS: u8 = 0x02;
pub const SET_CONDITION_RESERVED_BITS: u8 = 0x03;
pub const SET_EXPIRATION_MASK: u8 = 0x0c;
pub const SET_INHERIT_EXPIRATION_BITS: u8 = 0x00;
pub const SET_NO_EXPIRY_BITS: u8 = 0x04;
pub const SET_EXPLICIT_TTL_BITS: u8 = 0x08;
pub const SET_EXPIRATION_RESERVED_BITS: u8 = 0x0c;
pub const SET_EVICTION_MASK: u8 = 0x30;
pub const SET_INHERIT_EVICTION_BITS: u8 = 0x00;
pub const SET_EVICTABLE_BITS: u8 = 0x10;
pub const SET_EVICTION_PROTECTED_BITS: u8 = 0x20;
pub const SET_EVICTION_RESERVED_BITS: u8 = 0x30;
pub const SET_RESERVED_MASK: u8 = 0xc0;

/// Namespace-open flag fields.
pub const OPEN_FLAGS_BYTES: usize = 1;
pub const OPEN_CREATE_IF_MISSING: u8 = 0x01;
pub const OPEN_RESERVED_MASK: u8 = 0xfe;

/// Namespace-delete flag fields.
pub const DELETE_FLAGS_BYTES: usize = 1;
pub const DELETE_IF_EMPTY: u8 = 0x00;
pub const DELETE_MODE_MASK: u8 = 0x03;
pub const DELETE_RESERVED_MASK: u8 = 0xfc;

/// Namespace-policy flag fields.
pub const POLICY_FLAGS_BYTES: usize = 1;
pub const POLICY_DEFAULT_EXPIRATION_MASK: u8 = 0x03;
pub const POLICY_NO_EXPIRY: u8 = 0x00;
pub const POLICY_FIXED_TTL: u8 = 0x01;
pub const POLICY_DEFAULT_EXPIRATION_RESERVED_BITS: u8 = 0x03;
pub const POLICY_EXPIRATION_OVERRIDE: u8 = 0x04;
pub const POLICY_EVICTION_PROTECTED: u8 = 0x08;
pub const POLICY_EVICTION_OVERRIDE: u8 = 0x10;
pub const POLICY_RESERVED_MASK: u8 = 0xe0;


/// Default maximum number of concurrent request lanes.
pub const DEFAULT_MAX_IN_FLIGHT: usize = 256;
/// Default connection-establishment timeout in milliseconds.
pub const DEFAULT_CONNECT_TIMEOUT_MILLISECONDS: u64 = 5_000;
/// Default complete-request timeout in milliseconds.
pub const DEFAULT_REQUEST_TIMEOUT_MILLISECONDS: u64 = 2_000;
/// Default maximum total attempts for response-safe operations.
pub const DEFAULT_RETRY_MAX_ATTEMPTS: usize = 2;
/// Default Zstandard compression level.
pub const DEFAULT_ZSTANDARD_LEVEL: i32 = 1;
/// Default minimum serialized input size considered for Zstandard compression.
pub const DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES: usize = 0;
/// Default minimum Zstandard savings required to retain compression.
pub const DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES: usize = 0;
/// Inclusive minimum supported Zstandard compression level.
pub const DEFAULT_ZSTANDARD_LEVEL_MIN: i32 = 1;
/// Inclusive maximum supported Zstandard compression level.
pub const DEFAULT_ZSTANDARD_LEVEL_MAX: i32 = 22;
/// Default TLS server name used when an adapter does not provide one.
pub const CLIENT_DEFAULT_SERVER_NAME: &str = "localhost";
/// PEM label used for adapter-assembled certificate chains.
pub const CLIENT_CERTIFICATE_PEM_TYPE: &str = "CERTIFICATE";
/// Minimum positive setting value when zero selects a default.
pub const CLIENT_MINIMUM_POSITIVE_VALUE: usize = 1;

/// Version of the native client FFI contract.
pub const FFI_ABI_VERSION: u32 = 1;
/// Native FFI operation identifier for GetJson.
pub const FFI_OPERATION_GET_JSON: u32 = 16;
/// Native FFI operation identifier for SetJson.
pub const FFI_OPERATION_SET_JSON: u32 = 17;
/// Native FFI operation identifier for GetStructured.
pub const FFI_OPERATION_GET_STRUCTURED: u32 = 18;
/// Native FFI operation identifier for SetStructured.
pub const FFI_OPERATION_SET_STRUCTURED: u32 = 19;
/// Native FFI operation identifier for GetV0.
pub const FFI_OPERATION_GET_V0: u32 = 20;
/// Native FFI operation identifier for SetV0.
pub const FFI_OPERATION_SET_V0: u32 = 21;
/// Native FFI operation identifier for Reconnect.
pub const FFI_OPERATION_RECONNECT: u32 = 4_294_967_041;
/// Native FFI result-kind identifier for Error.
pub const FFI_RESULT_ERROR: u32 = 0;
/// Native FFI result-kind identifier for Ok.
pub const FFI_RESULT_OK: u32 = 1;
/// Native FFI result-kind identifier for Value.
pub const FFI_RESULT_VALUE: u32 = 2;
/// Native FFI result-kind identifier for NotFound.
pub const FFI_RESULT_NOT_FOUND: u32 = 3;
/// Native FFI result-kind identifier for Created.
pub const FFI_RESULT_CREATED: u32 = 4;
/// Native FFI result-kind identifier for Replaced.
pub const FFI_RESULT_REPLACED: u32 = 5;
/// Native FFI result-kind identifier for Deleted.
pub const FFI_RESULT_DELETED: u32 = 6;
/// Native FFI result-kind identifier for NotDeleted.
pub const FFI_RESULT_NOT_DELETED: u32 = 7;
/// Native FFI result-kind identifier for Connected.
pub const FFI_RESULT_CONNECTED: u32 = 8;
/// Native FFI result-kind identifier for NotStored.
pub const FFI_RESULT_NOT_STORED: u32 = 9;
/// Native FFI result-kind identifier for Raw.
pub const FFI_RESULT_RAW: u32 = 10;
/// Native FFI result-kind identifier for Canceled.
pub const FFI_RESULT_CANCELED: u32 = 11;
/// Native FFI result-kind identifier for UnknownMutation.
pub const FFI_RESULT_UNKNOWN_MUTATION: u32 = 12;
/// Native FFI result-kind identifier for ResourceExhausted.
pub const FFI_RESULT_RESOURCE_EXHAUSTED: u32 = 13;
/// Native FFI status-category identifier for Success.
pub const FFI_STATUS_CATEGORY_SUCCESS: u32 = 0;
/// Native FFI status-category identifier for NotFound.
pub const FFI_STATUS_CATEGORY_NOT_FOUND: u32 = 1;
/// Native FFI status-category identifier for Mutation.
pub const FFI_STATUS_CATEGORY_MUTATION: u32 = 2;
/// Native FFI status-category identifier for Error.
pub const FFI_STATUS_CATEGORY_ERROR: u32 = 3;
/// Native FFI status-category identifier for Canceled.
pub const FFI_STATUS_CATEGORY_CANCELED: u32 = 4;
/// Native FFI status-category identifier for UnknownMutation.
pub const FFI_STATUS_CATEGORY_UNKNOWN_MUTATION: u32 = 5;
/// Native FFI status-category identifier for ResourceExhausted.
pub const FFI_STATUS_CATEGORY_RESOURCE_EXHAUSTED: u32 = 6;
/// Native FFI error-category identifier for None.
pub const FFI_ERROR_CATEGORY_NONE: u32 = 0;
/// Native FFI error-category identifier for InvalidInput.
pub const FFI_ERROR_CATEGORY_INVALID_INPUT: u32 = 1;
/// Native FFI error-category identifier for Configuration.
pub const FFI_ERROR_CATEGORY_CONFIGURATION: u32 = 2;
/// Native FFI error-category identifier for Timeout.
pub const FFI_ERROR_CATEGORY_TIMEOUT: u32 = 3;
/// Native FFI error-category identifier for Transport.
pub const FFI_ERROR_CATEGORY_TRANSPORT: u32 = 4;
/// Native FFI error-category identifier for Server.
pub const FFI_ERROR_CATEGORY_SERVER: u32 = 5;
/// Native FFI error-category identifier for Protocol.
pub const FFI_ERROR_CATEGORY_PROTOCOL: u32 = 6;
/// Native FFI error-category identifier for Value.
pub const FFI_ERROR_CATEGORY_VALUE: u32 = 7;
/// Native FFI error-category identifier for Key.
pub const FFI_ERROR_CATEGORY_KEY: u32 = 8;
/// Native FFI error-category identifier for Canceled.
pub const FFI_ERROR_CATEGORY_CANCELED: u32 = 9;
/// Native FFI error-category identifier for UnknownMutation.
pub const FFI_ERROR_CATEGORY_UNKNOWN_MUTATION: u32 = 10;
/// Native FFI error-category identifier for ResourceExhausted.
pub const FFI_ERROR_CATEGORY_RESOURCE_EXHAUSTED: u32 = 11;
/// Native FFI error-category identifier for Closed.
pub const FFI_ERROR_CATEGORY_CLOSED: u32 = 12;
/// Native FFI error-category identifier for Internal.
pub const FFI_ERROR_CATEGORY_INTERNAL: u32 = 13;
/// Native FFI request-state identifier for Pending.
pub const FFI_REQUEST_STATE_PENDING: u32 = 0;
/// Native FFI request-state identifier for Ready.
pub const FFI_REQUEST_STATE_READY: u32 = 1;
/// Native FFI request-state identifier for Canceled.
pub const FFI_REQUEST_STATE_CANCELED: u32 = 2;
/// Native FFI request-state identifier for Consumed.
pub const FFI_REQUEST_STATE_CONSUMED: u32 = 3;
/// Native FFI request-state identifier for Freed.
pub const FFI_REQUEST_STATE_FREED: u32 = 4;
/// Native FFI value-representation identifier for Lossless.
pub const FFI_VALUE_REPRESENTATION_LOSSLESS: u32 = 0;
/// Native FFI value-representation identifier for Native.
pub const FFI_VALUE_REPRESENTATION_NATIVE: u32 = 1;
/// Native FFI value-mode identifier for FormattedV1.
pub const FFI_VALUE_MODE_FORMATTED_V1: u32 = 0;
/// Native FFI value-mode identifier for Raw.
pub const FFI_VALUE_MODE_RAW: u32 = 1;
/// Native FFI value-mode identifier for CallerOwnedV0.
pub const FFI_VALUE_MODE_CALLER_OWNED_V0: u32 = 2;
/// Native FFI connection-state identifier for Connected.
pub const FFI_CONNECTION_STATE_CONNECTED: u32 = 0;
/// Native FFI connection-state identifier for Reconnecting.
pub const FFI_CONNECTION_STATE_RECONNECTING: u32 = 1;
/// Native FFI connection-state identifier for Disconnected.
pub const FFI_CONNECTION_STATE_DISCONNECTED: u32 = 2;
/// Native FFI connection-state identifier for Closed.
pub const FFI_CONNECTION_STATE_CLOSED: u32 = 3;
/// Native FFI connection-state identifier for Unknown.
pub const FFI_CONNECTION_STATE_UNKNOWN: u32 = 4;
/// Native FFI transport selector for Quic.
pub const FFI_TRANSPORT_QUIC: u32 = 0;
/// Native FFI transport selector for TlsTcp.
pub const FFI_TRANSPORT_TLS_TCP: u32 = 1;
/// Native FFI transport selector for QuicInsecure.
pub const FFI_TRANSPORT_QUIC_INSECURE: u32 = 2;
/// Native FFI transport selector for TlsTcpInsecure.
pub const FFI_TRANSPORT_TLS_TCP_INSECURE: u32 = 3;
/// Native FFI SET-condition identifier for Any.
pub const FFI_SET_CONDITION_ANY: u32 = 0;
/// Native FFI SET-condition identifier for IfAbsent.
pub const FFI_SET_CONDITION_IF_ABSENT: u32 = 1;
/// Native FFI SET-condition identifier for IfPresent.
pub const FFI_SET_CONDITION_IF_PRESENT: u32 = 2;
/// Native FFI logical-key specification identifier for Text.
pub const FFI_KEY_SPEC_TEXT: u32 = 0;
/// Native FFI logical-key specification identifier for Bytes.
pub const FFI_KEY_SPEC_BYTES: u32 = 1;
/// Native FFI logical-key specification identifier for Integer.
pub const FFI_KEY_SPEC_INTEGER: u32 = 2;
/// Native namespace-descriptor decode status for Ok.
pub const FFI_NAMESPACE_DESCRIPTOR_DECODE_OK: u32 = 0;
/// Native namespace-descriptor decode status for Invalid.
pub const FFI_NAMESPACE_DESCRIPTOR_DECODE_INVALID: u32 = 1;
/// Native namespace default-expiration value for NoExpiry.
pub const FFI_NAMESPACE_DEFAULT_EXPIRATION_NO_EXPIRY: u32 = 0;
/// Native namespace default-expiration value for FixedTtl.
pub const FFI_NAMESPACE_DEFAULT_EXPIRATION_FIXED_TTL: u32 = 1;
/// Native namespace default-eviction value for Evictable.
pub const FFI_NAMESPACE_DEFAULT_EVICTION_EVICTABLE: u32 = 0;
/// Native namespace default-eviction value for Protected.
pub const FFI_NAMESPACE_DEFAULT_EVICTION_PROTECTED: u32 = 1;
/// Native namespace override-policy value for Disallowed.
pub const FFI_NAMESPACE_OVERRIDE_DISALLOWED: u32 = 0;
/// Native namespace override-policy value for Allowed.
pub const FFI_NAMESPACE_OVERRIDE_ALLOWED: u32 = 1;
/// Size of the C-compatible native namespace descriptor.
pub const FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES: usize = 40;
/// Native namespace descriptor field offsets.
pub const FFI_NAMESPACE_DESCRIPTOR_NAMESPACE_ID_OFFSET: usize = 0;
pub const FFI_NAMESPACE_DESCRIPTOR_REVISION_OFFSET: usize = 8;
pub const FFI_NAMESPACE_DESCRIPTOR_DEFAULT_TTL_MS_OFFSET: usize = 16;
pub const FFI_NAMESPACE_DESCRIPTOR_DEFAULT_EXPIRATION_OFFSET: usize = 24;
pub const FFI_NAMESPACE_DESCRIPTOR_EXPIRATION_OVERRIDE_OFFSET: usize = 28;
pub const FFI_NAMESPACE_DESCRIPTOR_DEFAULT_EVICTION_OFFSET: usize = 32;
pub const FFI_NAMESPACE_DESCRIPTOR_EVICTION_OVERRIDE_OFFSET: usize = 36;

/// C-compatible namespace descriptor returned by the native ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FfiNamespaceDescriptor {
    pub namespace_id: u64,
    pub revision: u64,
    pub default_ttl_ms: u64,
    pub default_expiration: u32,
    pub expiration_override: u32,
    pub default_eviction: u32,
    pub eviction_override: u32,
}

const _: () = {
    assert!(
        core::mem::size_of::<FfiNamespaceDescriptor>()
            == FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES
    );
    assert!(
        core::mem::offset_of!(FfiNamespaceDescriptor, namespace_id)
            == FFI_NAMESPACE_DESCRIPTOR_NAMESPACE_ID_OFFSET
    );
    assert!(
        core::mem::offset_of!(FfiNamespaceDescriptor, revision)
            == FFI_NAMESPACE_DESCRIPTOR_REVISION_OFFSET
    );
    assert!(
        core::mem::offset_of!(FfiNamespaceDescriptor, default_ttl_ms)
            == FFI_NAMESPACE_DESCRIPTOR_DEFAULT_TTL_MS_OFFSET
    );
    assert!(
        core::mem::offset_of!(FfiNamespaceDescriptor, default_expiration)
            == FFI_NAMESPACE_DESCRIPTOR_DEFAULT_EXPIRATION_OFFSET
    );
    assert!(
        core::mem::offset_of!(FfiNamespaceDescriptor, expiration_override)
            == FFI_NAMESPACE_DESCRIPTOR_EXPIRATION_OVERRIDE_OFFSET
    );
    assert!(
        core::mem::offset_of!(FfiNamespaceDescriptor, default_eviction)
            == FFI_NAMESPACE_DESCRIPTOR_DEFAULT_EVICTION_OFFSET
    );
    assert!(
        core::mem::offset_of!(FfiNamespaceDescriptor, eviction_override)
            == FFI_NAMESPACE_DESCRIPTOR_EVICTION_OVERRIDE_OFFSET
    );
};

/// Borrowed field passed through a generated structured native operation.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FfiOperationField {
    pub data: *const u8,
    pub length: usize,
    pub present: u8,
}

/// Smithy EvictionDefault value Evictable.
pub const SMITHY_EVICTION_DEFAULT_EVICTABLE: &str = "evictable";
/// Smithy EvictionDefault value EvictionProtected.
pub const SMITHY_EVICTION_DEFAULT_EVICTION_PROTECTED: &str = "eviction_protected";
/// Smithy EvictionMode value Inherit.
pub const SMITHY_EVICTION_MODE_INHERIT: &str = "inherit";
/// Smithy EvictionMode value Evictable.
pub const SMITHY_EVICTION_MODE_EVICTABLE: &str = "evictable";
/// Smithy EvictionMode value EvictionProtected.
pub const SMITHY_EVICTION_MODE_EVICTION_PROTECTED: &str = "eviction_protected";
/// Smithy ExpirationDefault value NoExpiry.
pub const SMITHY_EXPIRATION_DEFAULT_NO_EXPIRY: &str = "no_expiry";
/// Smithy ExpirationDefault value FixedTtl.
pub const SMITHY_EXPIRATION_DEFAULT_FIXED_TTL: &str = "fixed_ttl";
/// Smithy ExpirationMode value Inherit.
pub const SMITHY_EXPIRATION_MODE_INHERIT: &str = "inherit";
/// Smithy ExpirationMode value NoExpiry.
pub const SMITHY_EXPIRATION_MODE_NO_EXPIRY: &str = "no_expiry";
/// Smithy ExpirationMode value ExplicitTtl.
pub const SMITHY_EXPIRATION_MODE_EXPLICIT_TTL: &str = "explicit_ttl";
/// Smithy OverridePolicy value Allowed.
pub const SMITHY_OVERRIDE_POLICY_ALLOWED: &str = "allowed";
/// Smithy OverridePolicy value Disallowed.
pub const SMITHY_OVERRIDE_POLICY_DISALLOWED: &str = "disallowed";
/// Smithy SetCondition value Any.
pub const SMITHY_SET_CONDITION_ANY: &str = "any";
/// Smithy SetCondition value IfAbsent.
pub const SMITHY_SET_CONDITION_IF_ABSENT: &str = "if_absent";
/// Smithy SetCondition value IfPresent.
pub const SMITHY_SET_CONDITION_IF_PRESENT: &str = "if_present";
/// Smithy SetOutcome value Created.
pub const SMITHY_SET_OUTCOME_CREATED: &str = "created";
/// Smithy SetOutcome value Replaced.
pub const SMITHY_SET_OUTCOME_REPLACED: &str = "replaced";
/// Smithy SetOutcome value NotStored.
pub const SMITHY_SET_OUTCOME_NOT_STORED: &str = "not_stored";
// Operation framing, field plans, codec descriptors, and response
// layouts are generated once by protocol/wire.ts. Client retry/result policy
// is rendered here, at the client adapter boundary, so the protocol crate
// remains free of client execution metadata.
pub use crate::internal_protocol::operation::{
    OperationFieldLayout,
    OperationFramePolicy,
    OperationFieldPlan,
    OperationLayoutFraming,
    OperationLayoutPlan,
    OperationRequestFraming,
    OperationResponseFraming,
    OperationWireSpec,
    WireCodecDescriptor,
    WireCodecKind,
    MAX_OPERATION_FIELDS,
    MAX_OPERATION_REQUEST_FIELDS,
    OPERATION_CODEC_NAMES,
    WIRE_CODEC_DESCRIPTORS,
    WIRE_CODEC_NAMES,
};
/// Generated replay policy owned by the client adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationRetryMode {
    Always,
    Never,
    WhenNotCreating,
}

/// Open response semantic label owned by a client result adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationResultSpec {
    pub response_semantics: &'static str,
}

/// Client-only projection derived from the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationClientProjection {
    pub retry_mode: OperationRetryMode,
    pub result: OperationResultSpec,
}

/// Generated client projections in opcode order.
pub const OPERATION_CLIENT_PROJECTIONS: [OperationClientProjection; crate::internal_protocol::Opcode::COUNT] = [
    OperationClientProjection {
        retry_mode: OperationRetryMode::Always,
        result: OperationResultSpec {
            response_semantics: "pong",
        },
    },
    OperationClientProjection {
        retry_mode: OperationRetryMode::Always,
        result: OperationResultSpec {
            response_semantics: "value",
        },
    },
    OperationClientProjection {
        retry_mode: OperationRetryMode::Never,
        result: OperationResultSpec {
            response_semantics: "set_outcome",
        },
    },
    OperationClientProjection {
        retry_mode: OperationRetryMode::Never,
        result: OperationResultSpec {
            response_semantics: "delete_outcome",
        },
    },
    OperationClientProjection {
        retry_mode: OperationRetryMode::Always,
        result: OperationResultSpec {
            response_semantics: "stats_json",
        },
    },
    OperationClientProjection {
        retry_mode: OperationRetryMode::Never,
        result: OperationResultSpec {
            response_semantics: "empty",
        },
    },
    OperationClientProjection {
        retry_mode: OperationRetryMode::WhenNotCreating,
        result: OperationResultSpec {
            response_semantics: "namespace_descriptor",
        },
    },
    OperationClientProjection {
        retry_mode: OperationRetryMode::Never,
        result: OperationResultSpec {
            response_semantics: "namespace_descriptor",
        },
    },
    OperationClientProjection {
        retry_mode: OperationRetryMode::Never,
        result: OperationResultSpec {
            response_semantics: "delete_outcome",
        },
    }
];

/// Returns the canonical wire-only operation spec.
pub const fn operation_wire_spec(
    opcode: crate::internal_protocol::Opcode,
) -> OperationWireSpec {
    crate::internal_protocol::operation::operation_wire_spec(opcode)
}

/// Returns the client-only projection for one operation.
pub const fn operation_client_projection(
    opcode: crate::internal_protocol::Opcode,
) -> Option<OperationClientProjection> {
    Some(OPERATION_CLIENT_PROJECTIONS[opcode.index()])
}

/// Resolves a canonical generated codec identifier.
pub fn wire_codec_kind(
    name: &str,
) -> Option<crate::internal_protocol::codec::CodecKind> {
    crate::internal_protocol::wire_codec_kind(name)
}

/// Maps a contract-approved response status to the native result discriminator
/// consumed by generated language adapters. The mapping is generated from the
/// operation's semantic result plan; the transport executor does not maintain
/// an operation-name table.
pub const fn operation_result_kind(
    opcode: crate::internal_protocol::Opcode,
    status: crate::internal_protocol::Status,
) -> Option<FfiResultKind> {
    match opcode {
        crate::internal_protocol::Opcode::Ping => match status {
            crate::internal_protocol::Status::Ok => Some(FfiResultKind::Ok),
            _ => None,
        },
        crate::internal_protocol::Opcode::Get => match status {
            crate::internal_protocol::Status::Ok => Some(FfiResultKind::Value),
            crate::internal_protocol::Status::NotFound => Some(FfiResultKind::NotFound),
            _ => None,
        },
        crate::internal_protocol::Opcode::Set => match status {
            crate::internal_protocol::Status::Created => Some(FfiResultKind::Created),
            crate::internal_protocol::Status::Replaced => Some(FfiResultKind::Replaced),
            crate::internal_protocol::Status::NotStored => Some(FfiResultKind::NotStored),
            _ => None,
        },
        crate::internal_protocol::Opcode::Delete => match status {
            crate::internal_protocol::Status::Deleted => Some(FfiResultKind::Deleted),
            crate::internal_protocol::Status::NotFound => Some(FfiResultKind::NotDeleted),
            _ => None,
        },
        crate::internal_protocol::Opcode::ExperimentalStats => match status {
            crate::internal_protocol::Status::Ok => Some(FfiResultKind::Value),
            _ => None,
        },
        crate::internal_protocol::Opcode::ExperimentalSync => match status {
            crate::internal_protocol::Status::Ok => Some(FfiResultKind::Ok),
            _ => None,
        },
        crate::internal_protocol::Opcode::NamespaceOpen => match status {
            crate::internal_protocol::Status::Ok => Some(FfiResultKind::Ok),
            crate::internal_protocol::Status::Created => Some(FfiResultKind::Created),
            _ => None,
        },
        crate::internal_protocol::Opcode::NamespaceUpdatePolicy => match status {
            crate::internal_protocol::Status::Ok => Some(FfiResultKind::Value),
            _ => None,
        },
        crate::internal_protocol::Opcode::NamespaceDelete => match status {
            crate::internal_protocol::Status::Deleted => Some(FfiResultKind::Ok),
            _ => None,
        },
    }
}

/// Native FFI operation identifiers shared by every language adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum FfiOperation {
    /// Native FFI operation identifier for Ping.
    Ping = 1,
    /// Native FFI operation identifier for Get.
    Get = 2,
    /// Native FFI operation identifier for Set.
    Set = 3,
    /// Native FFI operation identifier for Delete.
    Delete = 4,
    /// Native FFI operation identifier for ExperimentalStats.
    ExperimentalStats = 5,
    /// Native FFI operation identifier for ExperimentalSync.
    ExperimentalSync = 6,
    /// Native FFI operation identifier for NamespaceOpen.
    NamespaceOpen = 7,
    /// Native FFI operation identifier for NamespaceUpdatePolicy.
    NamespaceUpdatePolicy = 8,
    /// Native FFI operation identifier for NamespaceDelete.
    NamespaceDelete = 9,
    /// Native FFI operation identifier for GetJson.
    GetJson = 16,
    /// Native FFI operation identifier for SetJson.
    SetJson = 17,
    /// Native FFI operation identifier for GetStructured.
    GetStructured = 18,
    /// Native FFI operation identifier for SetStructured.
    SetStructured = 19,
    /// Native FFI operation identifier for GetV0.
    GetV0 = 20,
    /// Native FFI operation identifier for SetV0.
    SetV0 = 21,
    /// Native FFI operation identifier for Reconnect.
    Reconnect = 4_294_967_041,
}

impl FfiOperation {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for FfiOperation {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
            value if value == Self::Ping.code() => Ok(Self::Ping),
            value if value == Self::Get.code() => Ok(Self::Get),
            value if value == Self::Set.code() => Ok(Self::Set),
            value if value == Self::Delete.code() => Ok(Self::Delete),
            value if value == Self::ExperimentalStats.code() => Ok(Self::ExperimentalStats),
            value if value == Self::ExperimentalSync.code() => Ok(Self::ExperimentalSync),
            value if value == Self::NamespaceOpen.code() => Ok(Self::NamespaceOpen),
            value if value == Self::NamespaceUpdatePolicy.code() => Ok(Self::NamespaceUpdatePolicy),
            value if value == Self::NamespaceDelete.code() => Ok(Self::NamespaceDelete),
            value if value == Self::GetJson.code() => Ok(Self::GetJson),
            value if value == Self::SetJson.code() => Ok(Self::SetJson),
            value if value == Self::GetStructured.code() => Ok(Self::GetStructured),
            value if value == Self::SetStructured.code() => Ok(Self::SetStructured),
            value if value == Self::GetV0.code() => Ok(Self::GetV0),
            value if value == Self::SetV0.code() => Ok(Self::SetV0),
            value if value == Self::Reconnect.code() => Ok(Self::Reconnect),
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for FfiOperation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Ping => "ping",
            Self::Get => "get",
            Self::Set => "set",
            Self::Delete => "delete",
            Self::ExperimentalStats => "experimental_stats",
            Self::ExperimentalSync => "experimental_sync",
            Self::NamespaceOpen => "namespace_open",
            Self::NamespaceUpdatePolicy => "namespace_update_policy",
            Self::NamespaceDelete => "namespace_delete",
            Self::GetJson => "get_json",
            Self::SetJson => "set_json",
            Self::GetStructured => "get_structured",
            Self::SetStructured => "set_structured",
            Self::GetV0 => "get_v0",
            Self::SetV0 => "set_v0",
            Self::Reconnect => "reconnect",
        })
    }
}

/// Input buffer kind declared by the native FFI contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FfiInputKind {
    None,
    ApplicationKey,
    ItemId,
}

/// Native dispatch and buffer contract for one FFI operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiOperationContract {
    pub input_kind: FfiInputKind,
    pub request_item_count: usize,
    pub accepts_value: bool,
    pub accepts_set_options: bool,
    pub supports_protected: bool,
    pub supports_raw: bool,
    pub supports_scoped: bool,
    pub dedicated_abi: bool,
}

/// Returns the generated native contract for one FFI operation.
pub const fn ffi_operation_contract(
    operation: FfiOperation,
) -> FfiOperationContract {
    match operation {
        FfiOperation::Ping => FfiOperationContract {
            input_kind: FfiInputKind::None,
            request_item_count: 0,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: false,
            dedicated_abi: false,
        },
        FfiOperation::Get => FfiOperationContract {
            input_kind: FfiInputKind::ItemId,
            request_item_count: 1,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: true,
            dedicated_abi: false,
        },
        FfiOperation::Set => FfiOperationContract {
            input_kind: FfiInputKind::ItemId,
            request_item_count: 1,
            accepts_value: true,
            accepts_set_options: true,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: true,
            dedicated_abi: false,
        },
        FfiOperation::Delete => FfiOperationContract {
            input_kind: FfiInputKind::ItemId,
            request_item_count: 1,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: true,
            dedicated_abi: false,
        },
        FfiOperation::ExperimentalStats => FfiOperationContract {
            input_kind: FfiInputKind::None,
            request_item_count: 0,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: true,
            dedicated_abi: false,
        },
        FfiOperation::ExperimentalSync => FfiOperationContract {
            input_kind: FfiInputKind::None,
            request_item_count: 0,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: true,
            dedicated_abi: false,
        },
        FfiOperation::NamespaceOpen => FfiOperationContract {
            input_kind: FfiInputKind::None,
            request_item_count: 0,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: false,
            supports_raw: false,
            supports_scoped: false,
            dedicated_abi: true,
        },
        FfiOperation::NamespaceUpdatePolicy => FfiOperationContract {
            input_kind: FfiInputKind::None,
            request_item_count: 0,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: false,
            supports_raw: false,
            supports_scoped: false,
            dedicated_abi: true,
        },
        FfiOperation::NamespaceDelete => FfiOperationContract {
            input_kind: FfiInputKind::None,
            request_item_count: 0,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: false,
            supports_raw: false,
            supports_scoped: false,
            dedicated_abi: true,
        },
        FfiOperation::GetJson => FfiOperationContract {
            input_kind: FfiInputKind::ApplicationKey,
            request_item_count: 0,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: true,
            dedicated_abi: false,
        },
        FfiOperation::SetJson => FfiOperationContract {
            input_kind: FfiInputKind::ApplicationKey,
            request_item_count: 0,
            accepts_value: true,
            accepts_set_options: true,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: true,
            dedicated_abi: false,
        },
        FfiOperation::GetStructured => FfiOperationContract {
            input_kind: FfiInputKind::ApplicationKey,
            request_item_count: 0,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: true,
            dedicated_abi: false,
        },
        FfiOperation::SetStructured => FfiOperationContract {
            input_kind: FfiInputKind::ApplicationKey,
            request_item_count: 0,
            accepts_value: true,
            accepts_set_options: true,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: true,
            dedicated_abi: false,
        },
        FfiOperation::GetV0 => FfiOperationContract {
            input_kind: FfiInputKind::ApplicationKey,
            request_item_count: 0,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: true,
            dedicated_abi: false,
        },
        FfiOperation::SetV0 => FfiOperationContract {
            input_kind: FfiInputKind::ApplicationKey,
            request_item_count: 0,
            accepts_value: true,
            accepts_set_options: true,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: true,
            dedicated_abi: false,
        },
        FfiOperation::Reconnect => FfiOperationContract {
            input_kind: FfiInputKind::None,
            request_item_count: 0,
            accepts_value: false,
            accepts_set_options: false,
            supports_protected: true,
            supports_raw: true,
            supports_scoped: false,
            dedicated_abi: false,
        },
    }
}

/// Resolves a protocol opcode from the shared native operation enum.
pub const fn protocol_opcode(
    operation: FfiOperation,
) -> Option<crate::internal_protocol::Opcode> {
    match operation {
        FfiOperation::Ping => Some(crate::internal_protocol::Opcode::Ping),
        FfiOperation::Get => Some(crate::internal_protocol::Opcode::Get),
        FfiOperation::Set => Some(crate::internal_protocol::Opcode::Set),
        FfiOperation::Delete => Some(crate::internal_protocol::Opcode::Delete),
        FfiOperation::ExperimentalStats => Some(crate::internal_protocol::Opcode::ExperimentalStats),
        FfiOperation::ExperimentalSync => Some(crate::internal_protocol::Opcode::ExperimentalSync),
        FfiOperation::NamespaceOpen => Some(crate::internal_protocol::Opcode::NamespaceOpen),
        FfiOperation::NamespaceUpdatePolicy => Some(crate::internal_protocol::Opcode::NamespaceUpdatePolicy),
        FfiOperation::NamespaceDelete => Some(crate::internal_protocol::Opcode::NamespaceDelete),
        _ => None,
    }
}


/// Native FFI result-kind identifiers shared by every language adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum FfiResultKind {
    /// Native FFI result-kind identifier for Error.
    Error = 0,
    /// Native FFI result-kind identifier for Ok.
    Ok = 1,
    /// Native FFI result-kind identifier for Value.
    Value = 2,
    /// Native FFI result-kind identifier for NotFound.
    NotFound = 3,
    /// Native FFI result-kind identifier for Created.
    Created = 4,
    /// Native FFI result-kind identifier for Replaced.
    Replaced = 5,
    /// Native FFI result-kind identifier for Deleted.
    Deleted = 6,
    /// Native FFI result-kind identifier for NotDeleted.
    NotDeleted = 7,
    /// Native FFI result-kind identifier for Connected.
    Connected = 8,
    /// Native FFI result-kind identifier for NotStored.
    NotStored = 9,
    /// Native FFI result-kind identifier for Raw.
    Raw = 10,
    /// Native FFI result-kind identifier for Canceled.
    Canceled = 11,
    /// Native FFI result-kind identifier for UnknownMutation.
    UnknownMutation = 12,
    /// Native FFI result-kind identifier for ResourceExhausted.
    ResourceExhausted = 13,
}

impl FfiResultKind {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for FfiResultKind {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
            value if value == Self::Error.code() => Ok(Self::Error),
            value if value == Self::Ok.code() => Ok(Self::Ok),
            value if value == Self::Value.code() => Ok(Self::Value),
            value if value == Self::NotFound.code() => Ok(Self::NotFound),
            value if value == Self::Created.code() => Ok(Self::Created),
            value if value == Self::Replaced.code() => Ok(Self::Replaced),
            value if value == Self::Deleted.code() => Ok(Self::Deleted),
            value if value == Self::NotDeleted.code() => Ok(Self::NotDeleted),
            value if value == Self::Connected.code() => Ok(Self::Connected),
            value if value == Self::NotStored.code() => Ok(Self::NotStored),
            value if value == Self::Raw.code() => Ok(Self::Raw),
            value if value == Self::Canceled.code() => Ok(Self::Canceled),
            value if value == Self::UnknownMutation.code() => Ok(Self::UnknownMutation),
            value if value == Self::ResourceExhausted.code() => Ok(Self::ResourceExhausted),
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for FfiResultKind {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Error => "error",
            Self::Ok => "ok",
            Self::Value => "value",
            Self::NotFound => "not_found",
            Self::Created => "created",
            Self::Replaced => "replaced",
            Self::Deleted => "deleted",
            Self::NotDeleted => "not_deleted",
            Self::Connected => "connected",
            Self::NotStored => "not_stored",
            Self::Raw => "raw",
            Self::Canceled => "canceled",
            Self::UnknownMutation => "unknown_mutation",
            Self::ResourceExhausted => "resource_exhausted",
        })
    }
}

/// Native FFI completion-status categories shared by every language adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum FfiStatusCategory {
    /// Native FFI status category identifier for Success.
    Success = 0,
    /// Native FFI status category identifier for NotFound.
    NotFound = 1,
    /// Native FFI status category identifier for Mutation.
    Mutation = 2,
    /// Native FFI status category identifier for Error.
    Error = 3,
    /// Native FFI status category identifier for Canceled.
    Canceled = 4,
    /// Native FFI status category identifier for UnknownMutation.
    UnknownMutation = 5,
    /// Native FFI status category identifier for ResourceExhausted.
    ResourceExhausted = 6,
}

impl FfiStatusCategory {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for FfiStatusCategory {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
            value if value == Self::Success.code() => Ok(Self::Success),
            value if value == Self::NotFound.code() => Ok(Self::NotFound),
            value if value == Self::Mutation.code() => Ok(Self::Mutation),
            value if value == Self::Error.code() => Ok(Self::Error),
            value if value == Self::Canceled.code() => Ok(Self::Canceled),
            value if value == Self::UnknownMutation.code() => Ok(Self::UnknownMutation),
            value if value == Self::ResourceExhausted.code() => Ok(Self::ResourceExhausted),
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for FfiStatusCategory {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Success => "success",
            Self::NotFound => "not_found",
            Self::Mutation => "mutation",
            Self::Error => "error",
            Self::Canceled => "canceled",
            Self::UnknownMutation => "unknown_mutation",
            Self::ResourceExhausted => "resource_exhausted",
        })
    }
}

/// Native FFI transport selectors shared by every language adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum FfiTransport {
    /// Native FFI transport identifier for Quic.
    Quic = 0,
    /// Native FFI transport identifier for TlsTcp.
    TlsTcp = 1,
    /// Native FFI transport identifier for QuicInsecure.
    QuicInsecure = 2,
    /// Native FFI transport identifier for TlsTcpInsecure.
    TlsTcpInsecure = 3,
}

impl FfiTransport {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for FfiTransport {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
            value if value == Self::Quic.code() => Ok(Self::Quic),
            value if value == Self::TlsTcp.code() => Ok(Self::TlsTcp),
            value if value == Self::QuicInsecure.code() => Ok(Self::QuicInsecure),
            value if value == Self::TlsTcpInsecure.code() => Ok(Self::TlsTcpInsecure),
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for FfiTransport {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Quic => "quic",
            Self::TlsTcp => "tls_tcp",
            Self::QuicInsecure => "quic_insecure",
            Self::TlsTcpInsecure => "tls_tcp_insecure",
        })
    }
}

/// Native FFI structured error categories shared by every language adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum FfiErrorCategory {
    /// Native FFI error category identifier for None.
    None = 0,
    /// Native FFI error category identifier for InvalidInput.
    InvalidInput = 1,
    /// Native FFI error category identifier for Configuration.
    Configuration = 2,
    /// Native FFI error category identifier for Timeout.
    Timeout = 3,
    /// Native FFI error category identifier for Transport.
    Transport = 4,
    /// Native FFI error category identifier for Server.
    Server = 5,
    /// Native FFI error category identifier for Protocol.
    Protocol = 6,
    /// Native FFI error category identifier for Value.
    Value = 7,
    /// Native FFI error category identifier for Key.
    Key = 8,
    /// Native FFI error category identifier for Canceled.
    Canceled = 9,
    /// Native FFI error category identifier for UnknownMutation.
    UnknownMutation = 10,
    /// Native FFI error category identifier for ResourceExhausted.
    ResourceExhausted = 11,
    /// Native FFI error category identifier for Closed.
    Closed = 12,
    /// Native FFI error category identifier for Internal.
    Internal = 13,
}

impl FfiErrorCategory {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for FfiErrorCategory {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
            value if value == Self::None.code() => Ok(Self::None),
            value if value == Self::InvalidInput.code() => Ok(Self::InvalidInput),
            value if value == Self::Configuration.code() => Ok(Self::Configuration),
            value if value == Self::Timeout.code() => Ok(Self::Timeout),
            value if value == Self::Transport.code() => Ok(Self::Transport),
            value if value == Self::Server.code() => Ok(Self::Server),
            value if value == Self::Protocol.code() => Ok(Self::Protocol),
            value if value == Self::Value.code() => Ok(Self::Value),
            value if value == Self::Key.code() => Ok(Self::Key),
            value if value == Self::Canceled.code() => Ok(Self::Canceled),
            value if value == Self::UnknownMutation.code() => Ok(Self::UnknownMutation),
            value if value == Self::ResourceExhausted.code() => Ok(Self::ResourceExhausted),
            value if value == Self::Closed.code() => Ok(Self::Closed),
            value if value == Self::Internal.code() => Ok(Self::Internal),
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for FfiErrorCategory {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::None => "none",
            Self::InvalidInput => "invalid_input",
            Self::Configuration => "configuration",
            Self::Timeout => "timeout",
            Self::Transport => "transport",
            Self::Server => "server",
            Self::Protocol => "protocol",
            Self::Value => "value",
            Self::Key => "key",
            Self::Canceled => "canceled",
            Self::UnknownMutation => "unknown_mutation",
            Self::ResourceExhausted => "resource_exhausted",
            Self::Closed => "closed",
            Self::Internal => "internal",
        })
    }
}

/// Native FFI asynchronous request lifecycle states shared by every language adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum FfiRequestState {
    /// Native FFI request state identifier for Pending.
    Pending = 0,
    /// Native FFI request state identifier for Ready.
    Ready = 1,
    /// Native FFI request state identifier for Canceled.
    Canceled = 2,
    /// Native FFI request state identifier for Consumed.
    Consumed = 3,
    /// Native FFI request state identifier for Freed.
    Freed = 4,
}

impl FfiRequestState {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for FfiRequestState {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
            value if value == Self::Pending.code() => Ok(Self::Pending),
            value if value == Self::Ready.code() => Ok(Self::Ready),
            value if value == Self::Canceled.code() => Ok(Self::Canceled),
            value if value == Self::Consumed.code() => Ok(Self::Consumed),
            value if value == Self::Freed.code() => Ok(Self::Freed),
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for FfiRequestState {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Canceled => "canceled",
            Self::Consumed => "consumed",
            Self::Freed => "freed",
        })
    }
}

/// Native FFI value-representation options shared by every language adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum FfiValueRepresentation {
    /// Native FFI value representation identifier for Lossless.
    Lossless = 0,
    /// Native FFI value representation identifier for Native.
    Native = 1,
}

impl FfiValueRepresentation {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for FfiValueRepresentation {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
            value if value == Self::Lossless.code() => Ok(Self::Lossless),
            value if value == Self::Native.code() => Ok(Self::Native),
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for FfiValueRepresentation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Lossless => "lossless",
            Self::Native => "native",
        })
    }
}

/// Native FFI value-mode options shared by every language adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum FfiValueMode {
    /// Native FFI value mode identifier for FormattedV1.
    FormattedV1 = 0,
    /// Native FFI value mode identifier for Raw.
    Raw = 1,
    /// Native FFI value mode identifier for CallerOwnedV0.
    CallerOwnedV0 = 2,
}

impl FfiValueMode {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for FfiValueMode {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
            value if value == Self::FormattedV1.code() => Ok(Self::FormattedV1),
            value if value == Self::Raw.code() => Ok(Self::Raw),
            value if value == Self::CallerOwnedV0.code() => Ok(Self::CallerOwnedV0),
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for FfiValueMode {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::FormattedV1 => "formatted_v1",
            Self::Raw => "raw",
            Self::CallerOwnedV0 => "caller_owned_v0",
        })
    }
}

/// Native FFI connection-state identifiers shared by every language adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum ConnectionState {
    /// Native FFI connection-state identifier for Connected.
    Connected = 0,
    /// Native FFI connection-state identifier for Reconnecting.
    Reconnecting = 1,
    /// Native FFI connection-state identifier for Disconnected.
    Disconnected = 2,
    /// Native FFI connection-state identifier for Closed.
    Closed = 3,
    /// Native FFI connection-state identifier for Unknown.
    Unknown = 4,
}

impl ConnectionState {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for ConnectionState {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
            value if value == Self::Connected.code() => Ok(Self::Connected),
            value if value == Self::Reconnecting.code() => Ok(Self::Reconnecting),
            value if value == Self::Disconnected.code() => Ok(Self::Disconnected),
            value if value == Self::Closed.code() => Ok(Self::Closed),
            value if value == Self::Unknown.code() => Ok(Self::Unknown),
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for ConnectionState {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Disconnected => "disconnected",
            Self::Closed => "closed",
            Self::Unknown => "unknown",
        })
    }
}

/// Native FFI SET-condition identifiers shared by every language adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum FfiSetCondition {
    /// Native FFI SET-condition identifier for Any.
    Any = 0,
    /// Native FFI SET-condition identifier for IfAbsent.
    IfAbsent = 1,
    /// Native FFI SET-condition identifier for IfPresent.
    IfPresent = 2,
}

impl FfiSetCondition {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for FfiSetCondition {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
            value if value == Self::Any.code() => Ok(Self::Any),
            value if value == Self::IfAbsent.code() => Ok(Self::IfAbsent),
            value if value == Self::IfPresent.code() => Ok(Self::IfPresent),
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for FfiSetCondition {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Any => "any",
            Self::IfAbsent => "if_absent",
            Self::IfPresent => "if_present",
        })
    }
}

/// Native FFI logical-key specification identifiers shared by every language adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum FfiKeySpec {
    /// Native FFI key spec identifier for Text.
    Text = 0,
    /// Native FFI key spec identifier for Bytes.
    Bytes = 1,
    /// Native FFI key spec identifier for Integer.
    Integer = 2,
}

impl FfiKeySpec {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for FfiKeySpec {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
            value if value == Self::Text.code() => Ok(Self::Text),
            value if value == Self::Bytes.code() => Ok(Self::Bytes),
            value if value == Self::Integer.code() => Ok(Self::Integer),
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for FfiKeySpec {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Text => "text",
            Self::Bytes => "bytes",
            Self::Integer => "integer",
        })
    }
}

/// Current client-owned value-format version.
pub const VALUE_FORMAT_VERSION: u128 = 1;
/// Canonical VU128 bytes for the current value-format version.
pub const VALUE_FORMAT_VERSION_BYTES: &[u8] = &[0x01];
/// Maximum bytes accepted for a canonical value-format VU128.
pub const VALUE_FORMAT_MAX_VU128_BYTES: usize = 9;
/// Bytes occupied by the value-format transform byte.
pub const VALUE_FORMAT_FORMAT_BYTE_BYTES: usize = 1;
/// Low-nibble mask for the value-format compression identifier.
pub const VALUE_FORMAT_COMPRESSION_MASK: u8 = 0x0f;
/// Number of bits to shift the value-format encryption identifier.
pub const VALUE_FORMAT_ENCRYPTION_SHIFT: u8 = 0x04;
/// Raw serialized-value identifier.
pub const VALUE_FORMAT_SERIALIZATION_RAW: u8 = 0x00;
/// Legacy metadata identifier; JSON helpers use OpaqueBytes selector 0.
pub const VALUE_FORMAT_SERIALIZATION_JSON: u8 = 0x01;
/// StructuredValue-CBOR-v1 payload-format selector.
pub const VALUE_FORMAT_SERIALIZATION_STRUCTURED: u8 = 0x01;
/// Uncompressed value-format identifier.
pub const VALUE_FORMAT_COMPRESSION_NONE: u8 = 0x00;
/// Zstandard value-format identifier.
pub const VALUE_FORMAT_COMPRESSION_ZSTANDARD: u8 = 0x01;
/// Unencrypted value-format identifier.
pub const VALUE_FORMAT_ENCRYPTION_NONE: u8 = 0x00;
/// Compact AES-SIV value-format identifier.
pub const VALUE_FORMAT_ENCRYPTION_COMPACT: u8 = 0x02;
/// Robust AES-GCM-SIV value-format identifier.
pub const VALUE_FORMAT_ENCRYPTION_ROBUST: u8 = 0x01;
/// Compact AES-SIV synthetic-IV and authentication-tag size.
pub const VALUE_FORMAT_COMPACT_SYNTHETIC_IV_BYTES: usize = 16;
/// Robust AES-GCM-SIV nonce size.
pub const VALUE_FORMAT_ROBUST_NONCE_BYTES: usize = 12;
/// Robust AES-GCM-SIV authentication-tag size.
pub const VALUE_FORMAT_ROBUST_TAG_BYTES: usize = 16;
/// Application-managed data-protection key size.
pub const VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES: usize = 32;
/// BLAKE3 protected-item-ID root derivation context.
pub const VALUE_FORMAT_ITEM_ID_ROOT_CONTEXT: &str = "OpenKache item ID derivation root v1";
/// Gate 0's fixed development namespace.
pub const GATE0_NAMESPACE_ID: u64 = 1;
/// Gate 0's public development Item-ID root fixture.
pub const GATE0_ITEM_ID_ROOT: [u8; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
    0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
    0x1e, 0x1f,
];
/// Gate 0's fixed selector: unprotected, uncompressed StructuredValue-CBOR-v1.
pub const GATE0_VALUE_FORMAT_SELECTOR: u8 =
    VALUE_FORMAT_ENCRYPTION_NONE
        | (VALUE_FORMAT_COMPRESSION_NONE << 2)
        | (VALUE_FORMAT_SERIALIZATION_STRUCTURED << 4);
/// Associated-data domain separator.
pub const VALUE_FORMAT_AAD_DOMAIN: &[u8] = b"openkache/value-format/aad/v1";
/// BLAKE3 value-root derivation context.
pub const VALUE_FORMAT_VALUE_ROOT_CONTEXT: &str = "OpenKache value format v1 root key";
/// BLAKE3 Compact AES-SIV MAC-key derivation context.
pub const VALUE_FORMAT_COMPACT_MAC_CONTEXT: &str = "OpenKache value format v1 AES-256-SIV-CMAC MAC key";
/// BLAKE3 Compact AES-SIV encryption-key derivation context.
pub const VALUE_FORMAT_COMPACT_ENCRYPTION_CONTEXT: &str = "OpenKache value format v1 AES-256-SIV-CMAC encryption key";
/// BLAKE3 Robust AES-GCM-SIV key derivation context.
pub const VALUE_FORMAT_ROBUST_CONTEXT: &str = "OpenKache value format v1 AES-256-GCM-SIV key";

/// Legacy metadata-envelope magic and version.
pub const VALUE_ENVELOPE_MAGIC_AND_VERSION: [u8; 4] = [0x4f, 0x4b, 0x56, 0x01];
/// Maximum UTF-8 byte length of a legacy metadata-envelope encoding identifier.
pub const VALUE_ENVELOPE_MAX_ENCODING_BYTES: usize = 64;
/// Maximum UTF-8 byte length of a legacy metadata-envelope logical type name.
pub const VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES: usize = 65_535;
/// Built-in canonical JSON codec identifier used by the legacy envelope adapter.
pub const VALUE_ENVELOPE_JSON_ENCODING: &str = "json";
