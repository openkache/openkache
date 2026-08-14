//! Small typed facade over generated operation metadata.
//!
//! Generated contract types remain the source of truth, but operation behavior
//! and transport code should not each rediscover how to look them up. Keeping
//! these accessors here gives the composition boundary one place to evolve
//! when the generated descriptor changes.

use openkache_protocol::Opcode;

// Keep only the generic server-facing generated contract behind this facade.
// Compatibility adapters import `operation_compatibility_contract` instead,
// so generic handlers cannot accidentally acquire the compatibility role vocabulary
// or compact bit constants merely because the generated artifact contains
// them.
pub(super) use crate::contract::{
    MAX_OPERATION_REQUEST_FIELDS, OperationFieldPlan, OperationFramePolicy, OperationLayoutFraming,
    OperationStatus, OperationWireSpec, WIRE_CODEC_DESCRIPTORS, WIRE_CODEC_NAMES,
    WireCodecCardinality, WireCodecDescriptor, WireCodecKind, WireCodecLengthEncoding,
    WireCodecWidth, operation_registry, operation_wire_spec, wire_codec_kind,
    wire_request_layout,
};
// Future generated generic bindings consume field indexes through this neutral
// facade even when the currently registered generic API has no fields.
#[allow(unused_imports)]
pub(super) use crate::contract::request_fields;

pub(super) const MAX_FIELDS: usize = crate::contract::MAX_OPERATION_FIELDS;

#[inline]
pub(super) const fn spec(opcode: Opcode) -> OperationWireSpec {
    crate::contract::operation_wire_spec(opcode)
}

#[inline]
pub(super) const fn response_budget(opcode: Opcode) -> usize {
    spec(opcode).response.max_width
}
