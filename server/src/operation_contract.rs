//! Small typed facade over generated operation metadata.
//!
//! Generated contract types remain the source of truth, but operation behavior
//! and transport code should not each rediscover how to look them up. Keeping
//! these accessors here gives the composition boundary one place to evolve
//! when the generated descriptor changes.

use openkache_protocol::Opcode;

use crate::observability::Operation;

// Keep only the generic server-facing generated contract behind this facade.
// Compatibility adapters import `operation_compatibility_contract` instead,
// so generic handlers cannot accidentally acquire the compatibility role vocabulary
// or compact bit constants merely because the generated artifact contains
// them.
pub(super) use crate::contract::{
    MAX_OPERATION_REQUEST_FIELDS, MAX_REQUEST_FRAME_BYTES, OperationFieldPlan,
    OperationLayoutFraming, OperationStatus, OperationWireSpec, WIRE_CODEC_DESCRIPTORS,
    WIRE_CODEC_NAMES, WireCodecCardinality, WireCodecDescriptor, WireCodecKind,
    WireCodecLengthEncoding, WireCodecWidth, OperationId, operation_id_for_opcode,
    operation_registry, operation_wire_spec, wire_codec_kind, wire_request_layout,
};
// Future generated generic bindings consume field indexes through this neutral
// facade even when the currently registered generic API has no fields.
#[allow(unused_imports)]
pub(super) use crate::contract::request_fields;

pub(super) const MAX_FIELDS: usize = crate::contract::MAX_OPERATION_FIELDS;

const _: () = assert!(Opcode::COUNT <= u8::MAX as usize);

const fn operation_names() -> [&'static str; Opcode::COUNT + 1] {
    let mut names = ["unknown"; Opcode::COUNT + 1];
    let mut index = 0;
    while index < Opcode::COUNT {
        names[index] = Opcode::NAMES[index];
        index += 1;
    }
    names
}

pub(super) const OPERATION_NAMES: [&str; Opcode::COUNT + 1] = operation_names();

/// Projects one generated operation into the neutral telemetry catalog.
#[inline]
pub(super) const fn telemetry_operation(opcode: Opcode) -> Operation {
    Operation::from_generated_index(opcode.index() as u8)
}

#[inline]
pub(super) const fn spec(opcode: Opcode) -> OperationWireSpec {
    crate::contract::operation_wire_spec(opcode)
}

#[inline]
pub(super) const fn response_budget(opcode: Opcode) -> usize {
    spec(opcode).response.max_width
}
