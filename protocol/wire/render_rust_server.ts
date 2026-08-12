/** Rust rendering for the server-owned operation adapter. */

import {
  derive_wire_operation_descriptor,
  request_wire_frame_bound,
} from "../wire_descriptor"
import { fixed_plan_width } from "../wire_layout"
import type {
  Wire_Contract,
  Wire_Operation,
  Wire_Request_Step,
} from "../wire_types"

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function rust_request_layout(contract: Wire_Contract): string {
  const operations = contract.operations
  if (operations === undefined) return ""
  const rust_byte = (value: number): string =>
    `0x${value.toString(16).padStart(2, "0")}`
  const compact_steps = (operation: Wire_Operation): string[] => {
    const plan = operation.contract.request_wire
    if (plan === undefined) return []
    const mappings = new Map<number, Map<string, number>>()
    const collect_mappings = (steps: readonly Wire_Request_Step[]): void => {
      for (const step of steps) {
        if (step.kind === "packed") {
          for (const field of step.fields) {
            const values = mappings.get(field.field) ?? new Map<string, number>()
            for (const value of field.values) {
              const previous = values.get(value.value)
              if (previous !== undefined && previous !== value.bits) {
                throw new Error(
                  `operation ${operation.name} maps request field ${field.field} ` +
                    `value ${value.value} to inconsistent packed bits`,
                )
              }
              values.set(value.value, value.bits)
            }
            mappings.set(field.field, values)
          }
        } else if (step.kind === "conditional") {
          collect_mappings(step.steps)
        }
      }
    }
    collect_mappings(plan)
    const render_steps = (steps: readonly Wire_Request_Step[]): string[] =>
      steps.map((step): string => {
        switch (step.kind) {
          case "fixed_field":
            return `WireRequestStep::Fixed { bytes: ${formatted_decimal(step.bytes)} }`
          case "packed":
            return `WireRequestStep::Packed {
                fields: &[${step.fields.map((field) =>
                  `WireRequestPackedField {
                    field: ${field.field},
                    mask: ${rust_byte(field.mask)},
                }`
                ).join(", ")}],
            }`
          case "byte_length_field":
            return "WireRequestStep::ByteLength"
          case "varuint_field":
            return "WireRequestStep::VarUInt"
          case "conditional": {
            const expected = mappings.get(step.field)?.get(step.equals)
            if (expected === undefined) {
              throw new Error(
                `operation ${operation.name} condition references field ${step.field} ` +
                  `value ${step.equals} without a preceding packed mapping`,
              )
            }
            return `WireRequestStep::Conditional {
                field: ${step.field},
                expected: ${rust_byte(expected)},
                steps: &[${render_steps(step.steps).join(", ")}],
            }`
          }
          case "constant":
            return `WireRequestStep::Bytes { expected: &[${step.bytes
              .map(rust_byte)
              .join(", ")}] }`
          case "trailing_field":
            return "WireRequestStep::ValueLength"
        }
      })
    return render_steps(plan)
  }
  const step_expression = (operation: Wire_Operation): string => {
    if (operation.contract.request_wire !== undefined) {
      return `[${[
        `WireRequestStep::Fixed { bytes: OPCODE_BYTES }`,
        ...compact_steps(operation),
      ].join(", ")}]`
    }
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
    RequestPackedField as WireRequestPackedField,
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
  return `// Generated from the OpenKache Smithy operation contract. Do not edit.

use openkache_protocol::{OPCODE_BYTES, Opcode};
// The server consumes only the canonical wire projection.  Keep the
// client-result/retry and execution-scope metadata in their respective
// adapters; it is intentionally absent from this server contract surface.
pub use openkache_protocol::operation::{
    operation_registry, operation_wire_spec, wire_codec_kind,
    MAX_OPERATION_FIELDS, MAX_OPERATION_REQUEST_FIELDS,
    request_fields, request_wire_plan,
    OperationFieldLayout, OperationFieldPlan,
    OperationFramePolicy,
    OperationWireSpec, WireCodecCardinality, WireCodecDescriptor, WireCodecKind,
    WireCodecLengthEncoding, WireCodecWidth, WIRE_CODEC_DESCRIPTORS,
    WIRE_CODEC_NAMES,
};

/// Aggregate payload ceiling for generic request layouts.
///
/// Frame prefixes are added by the composed transport adapter, so this
/// constant does not encode a protocol-version-specific opcode or varuint
/// width.
pub const MAX_GENERIC_REQUEST_PAYLOAD_BYTES: usize =
    ${formatted_decimal(contract.max_value_bytes)};

/// Maximum complete frame size contributed by exact declarative request plans.
///
/// This is generic generated admission metadata. Compatibility projections are
/// combined separately by the server composition layer and do not change this
/// exact-plan calculation.
pub const MAX_REQUEST_WIRE_FRAME_BYTES: usize =
    ${formatted_decimal(
      contract.operations === undefined || contract.operations.length === 0
        ? 0
        : Math.max(
          ...contract.operations.map((operation) =>
            request_wire_frame_bound(contract, operation)
          ),
        ),
    )};

${rust_request_layout(contract)}
`
}
