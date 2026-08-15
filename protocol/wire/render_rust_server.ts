/** Rust rendering for the server-owned operation adapter. */

import {
  derive_wire_operation_descriptor,
} from "../wire_descriptor"
import { effective_field_codec } from "../wire_types"
import { fixed_plan_width } from "../wire_layout"
import type {
  Wire_Contract,
  Wire_Operation,
  Wire_Request_Step,
} from "../wire_types"
import { MAX_GENERATED_REQUEST_FRAME_STATE_SLOTS } from "../wire_types"

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function formatted_byte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`
}

function packed_value_bytes(
  operation: Wire_Operation,
  field_index: number,
  value: string,
): readonly number[] {
  const field = operation.contract.request_plan?.find(
    (candidate) => candidate.index === field_index,
  )
  if (field === undefined) {
    throw new Error(
      `operation ${operation.name} has no request field ${field_index} for a packed mapping`,
    )
  }
  const codec = effective_field_codec(field)
  if (codec === "bool_u8") {
    if (value === "false") return [0]
    if (value === "true") return [1]
    throw new Error(
      `operation ${operation.name} packed Boolean field ${field_index} has invalid value ${JSON.stringify(value)}`,
    )
  }
  if (codec !== "enum") {
    throw new Error(
      `operation ${operation.name} packed field ${field_index} does not use the canonical enum codec`,
    )
  }
  return Array.from(new TextEncoder().encode(value))
}

type Packed_Selector = {
  readonly slot: number
  readonly values: ReadonlyMap<string, number>
}

type Request_Wire_Render_State = {
  readonly byte_length_slots: Map<number, number>
  readonly packed_selectors: Map<number, Packed_Selector>
  next_byte_length_slot: number
  next_packed_selector: number
  value_length_declared: boolean
}

function reserve_state_slot(next: number, operation: Wire_Operation, kind: string): number {
  if (next >= MAX_GENERATED_REQUEST_FRAME_STATE_SLOTS) {
    throw new Error(
      `operation ${operation.name} requires more than ` +
        `${MAX_GENERATED_REQUEST_FRAME_STATE_SLOTS} ${kind} slots`,
    )
  }
  return next
}

function request_wire_steps(
  operation: Wire_Operation,
  steps: readonly Wire_Request_Step[],
  state: Request_Wire_Render_State,
): readonly string[] {
  return steps.map((step): string => {
    switch (step.kind) {
      case "fixed_field":
        return `WireRequestStep::FixedField {
                field: ${step.field},
                bytes: ${formatted_decimal(step.bytes)},
            }`
      case "packed": {
        const fields = step.fields.map((field): string => {
          if (state.packed_selectors.has(field.field)) {
            throw new Error(
              `operation ${operation.name} assigns request field ${field.field} to more than one packed byte`,
            )
          }
          const slot = reserve_state_slot(
            state.next_packed_selector,
            operation,
            "packed-selector",
          )
          state.next_packed_selector += 1
          state.packed_selectors.set(field.field, {
            slot,
            values: new Map(field.values.map(({ value, bits }) => [value, bits])),
          })
          const values = field.values.map(({ value, bits }) => {
            const bytes = packed_value_bytes(operation, field.field, value)
            return `WireRequestPackedValue {
                    bits: ${formatted_byte(bits)},
                    bytes: &[${Array.from(bytes).map(formatted_byte).join(", ")}],
                }`
          })
          return `WireRequestPackedField {
                    slot: ${slot},
                    field: ${field.field},
                    mask: ${formatted_byte(field.mask)},
                    values: &[${values.join(", ")}],
                }`
        })
        return `WireRequestStep::Packed {
                fields: &[${fields.join(", ")}],
                reserved_mask: ${formatted_byte(step.reserved_mask)},
                constant_bits: ${formatted_byte(step.constant_bits)},
            }`
      }
      case "byte_length_field":
        return `WireRequestStep::ByteLengthField { field: ${step.field} }`
      case "byte_length_prefix_field": {
        if (state.byte_length_slots.has(step.field)) {
          throw new Error(
            `operation ${operation.name} assigns request field ${step.field} more than one byte-length prefix`,
          )
        }
        const slot = reserve_state_slot(
          state.next_byte_length_slot,
          operation,
          "deferred byte-length",
        )
        state.next_byte_length_slot += 1
        state.byte_length_slots.set(step.field, slot)
        return `WireRequestStep::ByteLengthPrefix { slot: ${slot} }`
      }
      case "byte_field": {
        const slot = state.byte_length_slots.get(step.field)
        if (slot === undefined) {
          throw new Error(
            `operation ${operation.name} request field ${step.field} has no preceding byte-length prefix`,
          )
        }
        state.byte_length_slots.delete(step.field)
        return `WireRequestStep::ByteLengthBodyField {
                slot: ${slot},
                field: ${step.field},
            }`
      }
      case "varuint_field":
        return `WireRequestStep::VarUIntField { field: ${step.field} }`
      case "value_length_field":
        if (state.value_length_declared) {
          throw new Error(`operation ${operation.name} declares more than one request value body`)
        }
        state.value_length_declared = true
        return `WireRequestStep::ValueLengthPrefixField { field: ${step.field} }`
      case "conditional": {
        const selector = state.packed_selectors.get(step.field)
        const expected = selector?.values.get(step.equals)
        if (selector === undefined || expected === undefined) {
          throw new Error(
            `operation ${operation.name} conditional field ${step.field} has no preceding packed mapping for ${step.equals}`,
          )
        }
        const nested = request_wire_steps(operation, step.steps, state)
        return `WireRequestStep::Conditional {
                selector: ${selector.slot},
                expected: ${formatted_byte(expected)},
                steps: &[${nested.join(", ")}],
            }`
      }
      case "constant":
        return `WireRequestStep::Constant { bytes: &[${step.bytes
          .map(formatted_byte)
          .join(", ")}] }`
      case "trailing_field":
        if (state.value_length_declared) {
          throw new Error(`operation ${operation.name} declares more than one request value body`)
        }
        state.value_length_declared = true
        return `WireRequestStep::TrailingField { field: ${step.field} }`
    }
  })
}

function explicit_request_step_expression(operation: Wire_Operation): string | undefined {
  const plan = operation.contract.request_wire
  if (plan === undefined) return undefined
  const state: Request_Wire_Render_State = {
    byte_length_slots: new Map(),
    packed_selectors: new Map(),
    next_byte_length_slot: 0,
    next_packed_selector: 0,
    value_length_declared: false,
  }
  const steps = request_wire_steps(operation, plan, state)
  if (state.byte_length_slots.size !== 0) {
    throw new Error(
      `operation ${operation.name} leaves a byte-length prefix without its body`,
    )
  }
  return `[${steps.join(", ")}]`
}

function request_wire_uses_packed(steps: readonly Wire_Request_Step[]): boolean {
  return steps.some((step) =>
    step.kind === "packed" ||
    (step.kind === "conditional" && request_wire_uses_packed(step.steps))
  )
}

function rust_request_layout(contract: Wire_Contract): string {
  const operations = contract.operations
  if (operations === undefined) return ""
  const uses_packed = operations.some((operation) =>
    operation.contract.request_wire !== undefined &&
    request_wire_uses_packed(operation.contract.request_wire)
  )
  const step_expression = (operation: Wire_Operation): string => {
    const explicit = explicit_request_step_expression(operation)
    if (explicit !== undefined) return explicit
    const descriptor = derive_wire_operation_descriptor(operation.contract)
    switch (descriptor.request_framing) {
      case "empty":
        return "[]"
      case "opaque":
        return "[WireRequestStep::ValueLength]"
      case "ordered_fields":
        if (descriptor.request_frame === "fixed_body") {
          const width = fixed_plan_width(operation.contract.request_plan)
          if (width === undefined) {
            throw new Error(
              `operation ${operation.name} selected fixed request framing without an exact width`,
            )
          }
          return `[WireRequestStep::FixedBody { bytes: ${formatted_decimal(width)} }]`
        }
        return "[WireRequestStep::ValueLength]"
      default:
        return `[]`
    }
  }
  const metadata = operations
    .map(
      (operation) => `    WireRequestLayout {
            steps: &${step_expression(operation)},
            field_count: ${formatted_decimal(
              operation.contract.request_plan?.length ?? 0,
            )},
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
${uses_packed ? "    RequestFramePackedField as WireRequestPackedField,\n" : ""}\
${uses_packed ? "    RequestFramePackedValue as WireRequestPackedValue,\n" : ""}\
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

use openkache_protocol::{Opcode, Status};
// The server consumes only the canonical wire projection.  Keep the
// Client-result/retry and execution-scope metadata belongs to the respective
// adapters; it is intentionally absent from this server contract surface.
pub use openkache_protocol::operation::{
    operation_registry, operation_wire_spec, wire_codec_kind,
    MAX_OPERATION_FIELDS, MAX_OPERATION_REQUEST_FIELDS,
    request_fields,
    OperationFieldPlan, OperationFramePolicy, OperationLayoutFraming,
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
