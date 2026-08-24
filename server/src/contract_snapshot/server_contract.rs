// Generated from the OpenKache Smithy operation contract. Do not edit.

use openkache_protocol::{Opcode, Status};
// The server consumes only the canonical wire projection. Client-result/retry
// and execution-scope metadata belongs to the respective
// adapters; it is intentionally absent from this server contract surface.
pub use openkache_protocol::operation::{
    operation_registry, operation_wire_spec, wire_codec_kind,
    MAX_OPERATION_FIELDS, MAX_OPERATION_REQUEST_FIELDS,
    request_fields,
    OperationFieldPlan, OperationLayoutFraming,
    OperationWireSpec, WireCodecCardinality, WireCodecDescriptor, WireCodecKind,
    WireCodecLengthEncoding, WireCodecWidth, WIRE_CODEC_DESCRIPTORS,
    WIRE_CODEC_NAMES,
};
pub use openkache_protocol::{
    MAX_REQUEST_FRAME_BYTES, RequestFrameLayout as WireRequestLayout,
    RequestFrameStep as WireRequestStep, wire_request_layout,
};

/// Runtime operation identity generated from the modeled operation table.
///
/// This ordinal is deliberately independent of the wire discriminant. API
/// modules and runtime catalogs use it as their dense key; only the generated
/// adapter below translates to/from the wire Opcode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum OperationId {
    Ping,
    Get,
    Set,
    Delete,
    ExperimentalStats,
    ExperimentalSync,
    NamespaceOpen,
    NamespaceUpdatePolicy,
    NamespaceDelete,
}

impl OperationId {
    pub const COUNT: usize = 9;

    /// Stable modeled names in runtime operation-ID order.
    pub const NAMES: [&'static str; Self::COUNT] = [
        "ping",
        "get",
        "set",
        "delete",
        "experimental_stats",
        "experimental_sync",
        "namespace_open",
        "namespace_update_policy",
        "namespace_delete",
    ];

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

#[inline]
pub const fn operation_id_for_opcode(opcode: Opcode) -> OperationId {
    match opcode {
        Opcode::Ping => OperationId::Ping,
        Opcode::Get => OperationId::Get,
        Opcode::Set => OperationId::Set,
        Opcode::Delete => OperationId::Delete,
        Opcode::ExperimentalStats => OperationId::ExperimentalStats,
        Opcode::ExperimentalSync => OperationId::ExperimentalSync,
        Opcode::NamespaceOpen => OperationId::NamespaceOpen,
        Opcode::NamespaceUpdatePolicy => OperationId::NamespaceUpdatePolicy,
        Opcode::NamespaceDelete => OperationId::NamespaceDelete,
    }
}

#[inline]
pub const fn opcode_for_operation_id(operation: OperationId) -> Opcode {
    match operation {
        OperationId::Ping => Opcode::Ping,
        OperationId::Get => Opcode::Get,
        OperationId::Set => Opcode::Set,
        OperationId::Delete => Opcode::Delete,
        OperationId::ExperimentalStats => Opcode::ExperimentalStats,
        OperationId::ExperimentalSync => Opcode::ExperimentalSync,
        OperationId::NamespaceOpen => Opcode::NamespaceOpen,
        OperationId::NamespaceUpdatePolicy => Opcode::NamespaceUpdatePolicy,
        OperationId::NamespaceDelete => Opcode::NamespaceDelete,
    }
}

/// Resolves generated wire metadata after runtime code has selected an
/// operation by its neutral dense identity.
#[inline]
pub const fn operation_wire_spec_for_id(operation: OperationId) -> OperationWireSpec {
    operation_wire_spec(opcode_for_operation_id(operation))
}

/// Resolves the compact request layout at the wire-adapter boundary.
#[inline]
pub const fn wire_request_layout_for_id(operation: OperationId) -> WireRequestLayout {
    wire_request_layout(opcode_for_operation_id(operation))
}

/// Transport-neutral semantic status generated from the modeled vocabulary.
///
/// API behavior returns this enum. Only the response adapter projects it to
/// the protocol wire value, so request execution performs no string lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationStatus {
    Ok,
    NotFound,
    Created,
    Replaced,
    Deleted,
    NotStored,
    Accepted,
    InvalidRequest,
    UnsupportedOpcode,
    TooLarge,
    Overloaded,
    Timeout,
    Forbidden,
    InternalError,
    NoCapacity,
    PolicyConflict,
    Conflict,
    NamespaceNotFound,
    NamespaceNotEmpty,
}

impl OperationStatus {
    /// Projects one semantic status at the wire-adapter boundary.
    pub const fn wire_status(self) -> Status {
        match self {
            Self::Ok => Status::Ok,
            Self::NotFound => Status::NotFound,
            Self::Created => Status::Created,
            Self::Replaced => Status::Replaced,
            Self::Deleted => Status::Deleted,
            Self::NotStored => Status::NotStored,
            Self::Accepted => Status::Accepted,
            Self::InvalidRequest => Status::InvalidRequest,
            Self::UnsupportedOpcode => Status::UnsupportedOpcode,
            Self::TooLarge => Status::TooLarge,
            Self::Overloaded => Status::Overloaded,
            Self::Timeout => Status::Timeout,
            Self::Forbidden => Status::Forbidden,
            Self::InternalError => Status::InternalError,
            Self::NoCapacity => Status::NoCapacity,
            Self::PolicyConflict => Status::PolicyConflict,
            Self::Conflict => Status::Conflict,
            Self::NamespaceNotFound => Status::NamespaceNotFound,
            Self::NamespaceNotEmpty => Status::NamespaceNotEmpty,
        }
    }
}

/// Aggregate payload ceiling for generic request layouts.
///
/// Frame prefixes are added by the composed transport adapter, so this
/// constant does not encode a protocol-version-specific opcode or varuint
/// width.
pub const MAX_GENERIC_REQUEST_PAYLOAD_BYTES: usize =
    67_108_864;
