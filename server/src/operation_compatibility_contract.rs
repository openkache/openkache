//! Generated contract projection used only by protocol-v1 compatibility code.
//!
//! The canonical operation descriptor is shared by every API. Draft-v1
//! behavior bindings and policy codecs additionally need historical bit
//! constants; keeping those imports here prevents generic handlers and the
//! transport-neutral executor from depending on that closed vocabulary.

pub(super) use crate::openkache_protocol::compat_v1::{
    POLICY_DEFAULT_EXPIRATION_MASK, POLICY_EVICTION_OVERRIDE, POLICY_EVICTION_PROTECTED,
    POLICY_EXPIRATION_OVERRIDE, POLICY_FIXED_TTL, POLICY_FLAGS_BYTES, POLICY_NO_EXPIRY,
    POLICY_RESERVED_MASK, SET_CONDITION_ANY_BITS, SET_CONDITION_MASK, SET_EVICTABLE_BITS,
    SET_EVICTION_MASK, SET_EVICTION_PROTECTED_BITS, SET_EXPIRATION_MASK, SET_EXPLICIT_TTL_BITS,
    SET_IF_ABSENT_BITS, SET_IF_PRESENT_BITS, SET_INHERIT_EVICTION_BITS,
    SET_INHERIT_EXPIRATION_BITS, SET_NO_EXPIRY_BITS, SET_RESERVED_MASK,
};
