// Generated from the OpenKache Smithy wire contract. Do not edit.

/// QUIC application protocol identifier for wire protocol version 1.
pub const ALPN: &[u8] = b"openkache/1";
/// Bytes occupied by the request opcode.
pub const OPCODE_BYTES: usize = 1;
/// Bytes occupied by the response status.
pub const STATUS_BYTES: usize = 1;
/// Bytes before the variable-length request lengths.
pub const REQUEST_FIXED_BYTES: usize = 1;
/// Bytes occupied by the fixed response status before variable fields.
pub const RESPONSE_FIXED_BYTES: usize = 1;
/// Minimum bytes in one canonical unsigned `vu128`.
pub const MIN_VARUINT_BYTES: usize = 1;
/// Maximum bytes in one unsigned `vu128` accepted by this protocol.
pub const MAX_VARUINT_BYTES: usize = 9;
/// Maximum bytes in one opaque Item ID carried by the protocol.
pub const ITEM_ID_BYTES: usize = 32;
/// Absolute value or response payload ceiling representable by protocol v1.
pub const MAX_VALUE_BYTES: usize = 67_108_864;
/// Conservative maximum complete response frame size for protocol v1,
/// including the status, echoed request ID, payload length, and payload.
pub const MAX_RESPONSE_FRAME_BYTES: usize =
    67_108_883;
/// Bytes in every namespace ID and namespace revision.
pub const NAMESPACE_ID_BYTES: usize = 8;
pub const NAMESPACE_REVISION_BYTES: usize = 8;
/// Bytes in the fixed namespace name length field.
pub const NAMESPACE_NAME_LENGTH_BYTES: usize = 1;
/// Width and missing sentinel used by the generic optional-value codec.
pub const OPTIONAL_VALUE_LENGTH_BYTES: usize = 4;
pub const OPTIONAL_VALUE_MISSING: u32 = 4_294_967_295;

/// First assigned status value reserved for errors.
pub const ERROR_STATUS_MINIMUM: u8 = 0x80;

wire_enum! {
    /// Operations supported by protocol v1.
    pub enum Opcode {
        Ping = 0x01,
        Get = 0x02,
        Set = 0x03,
        Delete = 0x04,
        ExperimentalStats = 0x05,
        ExperimentalSync = 0x06,
        NamespaceOpen = 0x07,
        NamespaceUpdatePolicy = 0x08,
        NamespaceDelete = 0x09,
    }
    unknown => UnknownOpcode
}

impl Opcode {
    /// Number of values assigned by the Smithy Opcode contract.
    pub const COUNT: usize = 9;

    /// Every assigned Smithy Opcode value in wire-value order.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Ping,
        Self::Get,
        Self::Set,
        Self::Delete,
        Self::ExperimentalStats,
        Self::ExperimentalSync,
        Self::NamespaceOpen,
        Self::NamespaceUpdatePolicy,
        Self::NamespaceDelete,
    ];

    /// Stable lowercase Smithy names in wire-value order.
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

    /// Zero-based position in the Smithy value-order arrays.
    ///
    /// Wire values are intentionally allowed to be sparse. Callers that use
    /// an enum as an array index must use this generated position instead of
    /// the wire discriminant.
    pub const fn index(self) -> usize {
        match self {
        Self::Ping => 0,
        Self::Get => 1,
        Self::Set => 2,
        Self::Delete => 3,
        Self::ExperimentalStats => 4,
        Self::ExperimentalSync => 5,
        Self::NamespaceOpen => 6,
        Self::NamespaceUpdatePolicy => 7,
        Self::NamespaceDelete => 8,
        }
    }

    /// Stable lowercase Smithy name for this assigned value.
    pub const fn name(self) -> &'static str {
        match self {
        Self::Ping => "ping",
        Self::Get => "get",
        Self::Set => "set",
        Self::Delete => "delete",
        Self::ExperimentalStats => "experimental_stats",
        Self::ExperimentalSync => "experimental_sync",
        Self::NamespaceOpen => "namespace_open",
        Self::NamespaceUpdatePolicy => "namespace_update_policy",
        Self::NamespaceDelete => "namespace_delete",
        }
    }

    /// Resolves a generated Smithy name at an API adapter boundary.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "ping" => Some(Self::Ping),
            "get" => Some(Self::Get),
            "set" => Some(Self::Set),
            "delete" => Some(Self::Delete),
            "experimental_stats" => Some(Self::ExperimentalStats),
            "experimental_sync" => Some(Self::ExperimentalSync),
            "namespace_open" => Some(Self::NamespaceOpen),
            "namespace_update_policy" => Some(Self::NamespaceUpdatePolicy),
            "namespace_delete" => Some(Self::NamespaceDelete),
            _ => None,
        }
    }
}

wire_enum! {
    /// Status returned in every protocol response.
    pub enum Status {
        Ok = 0x00,
        NotFound = 0x01,
        Created = 0x02,
        Replaced = 0x03,
        Deleted = 0x04,
        NotStored = 0x05,
        Accepted = 0x06,
        InvalidRequest = 0x80,
        UnsupportedOpcode = 0x81,
        TooLarge = 0x82,
        Overloaded = 0x83,
        Timeout = 0x84,
        Forbidden = 0x85,
        InternalError = 0x86,
        NoCapacity = 0x87,
        PolicyConflict = 0x88,
        Conflict = 0x89,
        NamespaceNotFound = 0x8a,
        NamespaceNotEmpty = 0x8b,
    }
    unknown => UnknownStatus
}

impl Status {
    /// Number of values assigned by the Smithy Status contract.
    pub const COUNT: usize = 19;

    /// Every assigned Smithy Status value in wire-value order.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Ok,
        Self::NotFound,
        Self::Created,
        Self::Replaced,
        Self::Deleted,
        Self::NotStored,
        Self::Accepted,
        Self::InvalidRequest,
        Self::UnsupportedOpcode,
        Self::TooLarge,
        Self::Overloaded,
        Self::Timeout,
        Self::Forbidden,
        Self::InternalError,
        Self::NoCapacity,
        Self::PolicyConflict,
        Self::Conflict,
        Self::NamespaceNotFound,
        Self::NamespaceNotEmpty,
    ];

    /// Stable lowercase Smithy names in wire-value order.
    pub const NAMES: [&'static str; Self::COUNT] = [
        "ok",
        "not_found",
        "created",
        "replaced",
        "deleted",
        "not_stored",
        "accepted",
        "invalid_request",
        "unsupported_opcode",
        "too_large",
        "overloaded",
        "timeout",
        "forbidden",
        "internal_error",
        "no_capacity",
        "policy_conflict",
        "conflict",
        "namespace_not_found",
        "namespace_not_empty",
    ];

    /// Zero-based position in the Smithy value-order arrays.
    ///
    /// Wire values are intentionally allowed to be sparse. Callers that use
    /// an enum as an array index must use this generated position instead of
    /// the wire discriminant.
    pub const fn index(self) -> usize {
        match self {
        Self::Ok => 0,
        Self::NotFound => 1,
        Self::Created => 2,
        Self::Replaced => 3,
        Self::Deleted => 4,
        Self::NotStored => 5,
        Self::Accepted => 6,
        Self::InvalidRequest => 7,
        Self::UnsupportedOpcode => 8,
        Self::TooLarge => 9,
        Self::Overloaded => 10,
        Self::Timeout => 11,
        Self::Forbidden => 12,
        Self::InternalError => 13,
        Self::NoCapacity => 14,
        Self::PolicyConflict => 15,
        Self::Conflict => 16,
        Self::NamespaceNotFound => 17,
        Self::NamespaceNotEmpty => 18,
        }
    }

    /// Stable lowercase Smithy name for this assigned value.
    pub const fn name(self) -> &'static str {
        match self {
        Self::Ok => "ok",
        Self::NotFound => "not_found",
        Self::Created => "created",
        Self::Replaced => "replaced",
        Self::Deleted => "deleted",
        Self::NotStored => "not_stored",
        Self::Accepted => "accepted",
        Self::InvalidRequest => "invalid_request",
        Self::UnsupportedOpcode => "unsupported_opcode",
        Self::TooLarge => "too_large",
        Self::Overloaded => "overloaded",
        Self::Timeout => "timeout",
        Self::Forbidden => "forbidden",
        Self::InternalError => "internal_error",
        Self::NoCapacity => "no_capacity",
        Self::PolicyConflict => "policy_conflict",
        Self::Conflict => "conflict",
        Self::NamespaceNotFound => "namespace_not_found",
        Self::NamespaceNotEmpty => "namespace_not_empty",
        }
    }

    /// Resolves a generated Smithy name at an API adapter boundary.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "ok" => Some(Self::Ok),
            "not_found" => Some(Self::NotFound),
            "created" => Some(Self::Created),
            "replaced" => Some(Self::Replaced),
            "deleted" => Some(Self::Deleted),
            "not_stored" => Some(Self::NotStored),
            "accepted" => Some(Self::Accepted),
            "invalid_request" => Some(Self::InvalidRequest),
            "unsupported_opcode" => Some(Self::UnsupportedOpcode),
            "too_large" => Some(Self::TooLarge),
            "overloaded" => Some(Self::Overloaded),
            "timeout" => Some(Self::Timeout),
            "forbidden" => Some(Self::Forbidden),
            "internal_error" => Some(Self::InternalError),
            "no_capacity" => Some(Self::NoCapacity),
            "policy_conflict" => Some(Self::PolicyConflict),
            "conflict" => Some(Self::Conflict),
            "namespace_not_found" => Some(Self::NamespaceNotFound),
            "namespace_not_empty" => Some(Self::NamespaceNotEmpty),
            _ => None,
        }
    }
}
