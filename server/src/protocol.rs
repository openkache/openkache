//! Server-owned protocol composition and cache policy types.
//!
//! Request framing, transport ownership, and the historical compatibility
//! facade live in separate modules. This root keeps only their composition
//! boundary, shared errors, and small protocol-wide helpers.

#[allow(unused_imports)]
pub(crate) use crate::contract::{WireRequestLayout, WireRequestStep, wire_request_layout};
use openkache_protocol::{
    MAX_VALUE_BYTES, MAX_VARUINT_BYTES, NAMESPACE_ID_BYTES, REQUEST_FIXED_BYTES,
};
pub use openkache_protocol::{ItemId, Opcode, Response, Status};

type WireResult<T> = openkache_protocol::Result<T>;

#[path = "protocol_compat_v1.rs"]
mod compat_v1;
#[path = "protocol_generic.rs"]
mod generic;
#[path = "protocol_policy.rs"]
mod policy;
#[path = "protocol_header.rs"]
mod header;
#[path = "protocol_frame.rs"]
mod frame;
#[path = "protocol_facade.rs"]
mod facade;

pub use facade::Request;
pub use header::RequestHeader;
pub use frame::RequestFrame;
pub(crate) use frame::ServerRequest;
pub use policy::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, NamespaceDescriptor,
    NamespacePolicy, OverridePolicy, SetCondition, SetOptions,
};

/// Returns the namespace-name limit owned by the protocol-v1 compatibility
/// adapter. Generic operation infrastructure does not consume this value.
pub(crate) const fn compatibility_namespace_name_max_bytes() -> usize {
    compat_v1::namespace_name_max_bytes()
}

/// Returns the complete request-frame admission ceiling for the composed
/// server. Generic layouts contribute the normal bound; compatibility
/// adapters may contribute a larger historical prefix without making that
/// prefix part of the generic operation contract.
pub(crate) const fn max_request_frame_bytes() -> usize {
    let generic = REQUEST_FIXED_BYTES
        .saturating_add(MAX_VARUINT_BYTES)
        .saturating_add(crate::contract::MAX_GENERIC_REQUEST_PAYLOAD_BYTES);
    let exact = crate::contract::MAX_REQUEST_WIRE_FRAME_BYTES;
    let compatibility = openkache_protocol::compat_v1::MAX_COMPATIBILITY_REQUEST_FRAME_BYTES;
    let generic_or_exact = if generic > exact { generic } else { exact };
    if generic_or_exact > compatibility {
        generic_or_exact
    } else {
        compatibility
    }
}

/// Supplies operation-neutral request framing at the composition boundary.
///
/// Transport code receives only byte-consumption metadata. It never selects a
/// semantic adapter or learns whether an opcode has a historical public
/// convenience projection.
pub(crate) trait FrameLayoutProvider: Send + Sync {
    fn layout_for(&self, opcode: Opcode) -> WireResult<WireRequestLayout>;
}

/// Generated provider used by the public server protocol facade.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GeneratedFrameLayoutProvider;

impl FrameLayoutProvider for GeneratedFrameLayoutProvider {
    fn layout_for(&self, opcode: Opcode) -> WireResult<WireRequestLayout> {
        Ok(wire_request_layout(opcode))
    }
}

const DEFAULT_FRAME_LAYOUT_PROVIDER: GeneratedFrameLayoutProvider = GeneratedFrameLayoutProvider;

fn read_u64_be(input: &[u8]) -> Result<u64> {
    let bytes: [u8; NAMESPACE_ID_BYTES] = input
        .get(..NAMESPACE_ID_BYTES)
        .ok_or(ProtocolError::FrameTooShort {
            expected: NAMESPACE_ID_BYTES,
            actual: input.len(),
        })?
        .try_into()
        .expect("slice length checked");
    Ok(u64::from_be_bytes(bytes))
}

fn encode_varuint(value: u64) -> ([u8; MAX_VARUINT_BYTES], usize) {
    openkache_protocol::encode_varuint(value)
}

fn decode_varuint(input: &[u8], context: &'static str) -> Result<Option<(u64, usize)>> {
    openkache_protocol::decode_varuint(input, context).map_err(Into::into)
}

fn validate_value_length(value_len: usize) -> Result<()> {
    if value_len > MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: value_len,
            maximum: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

/// Protocol framing and validation errors.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("unknown opcode 0x{0:02x}")]
    UnknownOpcode(u8),
    #[error("unknown status 0x{0:02x}")]
    UnknownStatus(u8),
    #[error("request flags contain unknown bits 0x{0:02x}")]
    UnknownRequestFlags(u8),
    #[error("frame is too short: expected at least {expected} bytes, got {actual}")]
    FrameTooShort { expected: usize, actual: usize },
    #[error("frame length does not match header: expected {expected} bytes, got {actual}")]
    FrameLength { expected: usize, actual: usize },
    #[error("frame length overflow")]
    FrameLengthOverflow,
    #[error("{context} uses a non-canonical vu128 encoding")]
    NonCanonicalVaruint { context: &'static str },
    #[error("{context} exceeds the supported 64-bit vu128 range")]
    VaruintOverflow { context: &'static str },
    #[error("{opcode:?} requires a {expected}-byte item ID, received {actual} item ID bytes")]
    InvalidItemIdLength {
        opcode: Opcode,
        expected: usize,
        actual: usize,
    },
    #[error("value is too large: {size} bytes exceeds {maximum}")]
    ValueTooLarge { size: usize, maximum: usize },
    #[error("operation field sequence is invalid: {0}")]
    InvalidFieldSequence(&'static str),
    #[error("{opcode:?} requires a fixed item/value shape ({expected_item_id}, {expected_value})")]
    InvalidRequestShape {
        opcode: Opcode,
        expected_item_id: usize,
        expected_value: &'static str,
    },
    #[error("if-absent and if-present conditions cannot be combined")]
    ConflictingSetConditions,
    #[error("SET TTL must be greater than zero milliseconds")]
    InvalidSetTtl,
    #[error("SET TTL is required by ExplicitTtl")]
    MissingSetTtl,
    #[error("SET TTL is not allowed by this expiration mode")]
    UnexpectedSetTtl,
    #[error("SET options are not valid for {opcode:?}")]
    InvalidSetOptions { opcode: Opcode },
    #[error("namespace ID is missing")]
    MissingNamespaceId,
    #[error("namespace ID must be a positive non-zero u64")]
    InvalidNamespaceId,
    #[error("namespace name is invalid: {0}")]
    InvalidNamespaceName(&'static str),
    #[error("namespace policy is missing")]
    MissingNamespacePolicy,
    #[error("namespace policy is not allowed")]
    UnexpectedNamespacePolicy,
    #[error("namespace policy is invalid: {0}")]
    InvalidNamespacePolicy(&'static str),
    #[error("namespace revision must be positive")]
    InvalidRevision,
}

impl From<openkache_protocol::ProtocolError> for ProtocolError {
    fn from(error: openkache_protocol::ProtocolError) -> Self {
        match error {
            openkache_protocol::ProtocolError::UnknownOpcode(value) => Self::UnknownOpcode(value),
            openkache_protocol::ProtocolError::UnknownStatus(value) => Self::UnknownStatus(value),
            openkache_protocol::ProtocolError::FrameTooShort { expected, actual } => {
                Self::FrameTooShort { expected, actual }
            }
            openkache_protocol::ProtocolError::FrameLength { expected, actual } => {
                Self::FrameLength { expected, actual }
            }
            openkache_protocol::ProtocolError::FrameLengthOverflow => Self::FrameLengthOverflow,
            openkache_protocol::ProtocolError::NonCanonicalVaruint { context } => {
                Self::NonCanonicalVaruint { context }
            }
            openkache_protocol::ProtocolError::VaruintOverflow { context } => {
                Self::VaruintOverflow { context }
            }
            openkache_protocol::ProtocolError::ValueTooLarge { size, maximum } => {
                Self::ValueTooLarge { size, maximum }
            }
            openkache_protocol::ProtocolError::InvalidFieldSequence(message) => {
                Self::InvalidFieldSequence(message)
            }
        }
    }
}

/// Convenience result type for protocol operations.
pub type Result<T> = std::result::Result<T, ProtocolError>;
