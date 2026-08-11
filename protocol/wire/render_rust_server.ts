/** Rust rendering for the server-owned operation adapter. */

import {
  derive_wire_operation_descriptor,
} from "../wire_descriptor"
import { fixed_plan_width } from "../wire_layout"
import type {
  Wire_Contract,
  Wire_Operation,
} from "../wire_types"

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function rust_request_layout(contract: Wire_Contract): string {
  const operations = contract.operations
  if (operations === undefined) return ""
  const step_expression = (operation: Wire_Operation): string => {
    const descriptor = derive_wire_operation_descriptor(operation.contract)
    const fixed = (bytes: string): string =>
      `WireRequestStep::Fixed { bytes: ${bytes} }`
    switch (descriptor.request_framing) {
      case "empty":
        return `[${fixed("OPCODE_BYTES")}]`
      case "opaque":
        return `[${fixed("OPCODE_BYTES")}, WireRequestStep::ValueLength]`
      case "ordered_fields":
        if (descriptor.request_frame === "fixed_body") {
          const width = fixed_plan_width(operation.contract.request_plan)
          if (width === undefined) {
            throw new Error(
              `operation ${operation.name} selected fixed request framing without an exact width`,
            )
          }
          return `[${fixed("OPCODE_BYTES")}, WireRequestStep::FixedBody { bytes: ${formatted_decimal(width)} }]`
        }
        return `[${fixed("OPCODE_BYTES")}, WireRequestStep::ValueLength]`
      default:
        return `[]`
    }
  }
  const metadata = operations
    .map(
      (operation) => `    WireRequestLayout {
            steps: &${step_expression(operation)},
        }`,
    )
    .join(",\n")
  return `/// The server adapter reuses the operation-neutral request parser's
/// byte-consumption metadata.
///
/// These aliases preserve the generated server contract vocabulary without
/// making the shared protocol parser depend on operation names or compact
/// protocol-v1 meanings.
pub use openkache_protocol::{
    RequestFrameLayout as WireRequestLayout,
    RequestFrameStep as WireRequestStep,
};

/// Returns the wire-level request layout for one assigned opcode.
pub const WIRE_REQUEST_LAYOUTS: [WireRequestLayout; Opcode::COUNT] = [
${metadata}
];

/// Returns the wire-level request layout for one assigned opcode.
pub const fn wire_request_layout(opcode: Opcode) -> WireRequestLayout {
    WIRE_REQUEST_LAYOUTS[opcode.index()]
}
`
}

/** Renders generic operation metadata for the server-owned adapter. */
export function render_rust_server_contract(contract: Wire_Contract): string {
  const operation_status_variants = contract.statuses
    .map((status) => `    ${status.name},`)
    .join("\n")
  const operation_status_wire_arms = contract.statuses
    .map((status) => `            Self::${status.name} => Status::${status.name},`)
    .join("\n")
  return `// Generated from the OpenKache Smithy operation contract. Do not edit.

use openkache_protocol::{OPCODE_BYTES, Opcode, Status};
// The server consumes only the canonical wire projection.  Keep the
// Client-result/retry and execution-scope metadata belongs to the respective
// adapters; it is intentionally absent from this server contract surface.
pub use openkache_protocol::operation::{
    operation_registry, operation_wire_spec, wire_codec_kind,
    MAX_OPERATION_FIELDS, MAX_OPERATION_REQUEST_FIELDS,
    request_fields,
    OperationFieldLayout, OperationFieldPlan,
    OperationFramePolicy, OperationLayoutFraming,
    OperationWireSpec, WireCodecCardinality, WireCodecDescriptor, WireCodecKind,
    WireCodecLengthEncoding, WireCodecWidth, WIRE_CODEC_DESCRIPTORS,
    WIRE_CODEC_NAMES,
};

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

${rust_request_layout(contract)}
`
}
