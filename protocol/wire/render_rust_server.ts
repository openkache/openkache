/** Rust rendering for the server-owned operation adapter. */

import type { Wire_Contract } from "../wire_types"

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function rust_string_literal(value: string): string {
  return JSON.stringify(value)
}

function wire_name(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase()
}

/** Renders generic operation metadata for the server-owned adapter. */
export function render_rust_server_contract(contract: Wire_Contract): string {
  const model_opcodes = contract.model_opcodes ?? contract.opcodes
  const operation_id_variants = model_opcodes
    .map((opcode) => `    ${opcode.name},`)
    .join("\n")
  const operation_id_names = model_opcodes
    .map(
      (opcode) =>
        `        ${rust_string_literal(opcode.text ?? wire_name(opcode.name))},`,
    )
    .join("\n")
  const operation_id_from_opcode_arms = contract.opcodes
    .map(
      (opcode) =>
        `        Opcode::${opcode.name} => OperationId::${opcode.name},`,
    )
    .join("\n")
  const opcode_from_operation_id_arms = contract.opcodes
    .map(
      (opcode) =>
        `        OperationId::${opcode.name} => Opcode::${opcode.name},`,
    )
    .join("\n")
  const operation_status_variants = contract.statuses
    .map((status) => `    ${status.name},`)
    .join("\n")
  const operation_status_wire_arms = contract.statuses
    .map((status) => `            Self::${status.name} => Status::${status.name},`)
    .join("\n")
  return `// Generated from the OpenKache Smithy operation contract. Do not edit.

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
${operation_id_variants}
}

impl OperationId {
    pub const COUNT: usize = ${model_opcodes.length};

    /// Stable modeled names in runtime operation-ID order.
    pub const NAMES: [&'static str; Self::COUNT] = [
${operation_id_names}
    ];

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

#[inline]
pub const fn operation_id_for_opcode(opcode: Opcode) -> OperationId {
    match opcode {
${operation_id_from_opcode_arms}
    }
}

#[inline]
pub const fn opcode_for_operation_id(operation: OperationId) -> Opcode {
    match operation {
${opcode_from_operation_id_arms}
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
${operation_status_variants}
}

impl OperationStatus {
    /// Projects one semantic status at the wire-adapter boundary.
    pub const fn wire_status(self) -> Status {
        match self {
${operation_status_wire_arms}
        }
    }
}

/// Aggregate payload ceiling for generic request layouts.
///
/// Frame prefixes are added by the composed transport adapter, so this
/// constant does not encode a protocol-version-specific opcode or varuint
/// width.
pub const MAX_GENERIC_REQUEST_PAYLOAD_BYTES: usize =
    ${formatted_decimal(contract.max_value_bytes)};
`
}
