//! Protocol-v1 compatibility projections.
//!
//! These route labels describe the historical compact request layouts. They
//! are intentionally kept outside the generic operation contract so a new
//! operation can use generic framing without adding a compatibility route variant.

use crate::Opcode;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/operation_compatibility.rs"));
}

/// Historical compact request route selected by a protocol-v1 adapter.
///
/// The route vocabulary and opcode projection are generated from explicit
/// compatibility metadata. This module only exposes the adapter-facing alias;
/// generic operation metadata never needs to import it.
pub use generated::{
    DELETE_FLAGS_BYTES, DELETE_IF_EMPTY, DELETE_MODE_MASK, DELETE_RESERVED_MASK,
    MAX_COMPATIBILITY_REQUEST_FRAME_BYTES, MAX_COMPATIBILITY_REQUEST_ITEM_IDS,
    NAMESPACE_NAME_MAX_BYTES, OPEN_CREATE_IF_MISSING, OPEN_FLAGS_BYTES, OPEN_RESERVED_MASK,
    OperationCompactV1Route, OperationFieldDirection, OperationFieldRole,
    POLICY_DEFAULT_EXPIRATION_MASK, POLICY_DEFAULT_EXPIRATION_RESERVED_BITS,
    POLICY_EVICTION_OVERRIDE, POLICY_EVICTION_PROTECTED, POLICY_EXPIRATION_OVERRIDE,
    POLICY_FIXED_TTL, POLICY_FLAGS_BYTES, POLICY_NO_EXPIRY, POLICY_RESERVED_MASK,
    SET_CONDITION_ANY_BITS, SET_CONDITION_MASK, SET_CONDITION_RESERVED_BITS, SET_EVICTABLE_BITS,
    SET_EVICTION_MASK, SET_EVICTION_PROTECTED_BITS, SET_EVICTION_RESERVED_BITS,
    SET_EXPIRATION_MASK, SET_EXPIRATION_RESERVED_BITS, SET_EXPLICIT_TTL_BITS, SET_FLAGS_BYTES,
    SET_IF_ABSENT_BITS, SET_IF_PRESENT_BITS, SET_INHERIT_EVICTION_BITS,
    SET_INHERIT_EXPIRATION_BITS, SET_NO_EXPIRY_BITS, SET_RESERVED_MASK, operation_field_count,
    operation_field_index, request_fields, response_fields,
};

/// Returns the historical route for an opcode.
///
/// This mapping is intentionally owned by the compatibility adapter. Generic
/// operation metadata contains only framing and field plans; adding a generic
/// operation therefore cannot add a compatibility route branch.
pub const fn route_for_opcode(opcode: crate::Opcode) -> Option<OperationCompactV1Route> {
    generated::route_for_opcode(opcode)
}
