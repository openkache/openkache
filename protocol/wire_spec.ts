/** Renders and validates the generated protocol documentation sections. */

import {
  derive_wire_operation_descriptor,
  type Wire_Contract,
  type Wire_Operation,
  type Wire_Operation_Field_Plan,
  type Wire_Request_Step,
} from "./wire"

function wire_name(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase()
}

/**
 * Renders the protocol operation table used by `SPEC.md`.
 *
 * Keeping this table generator-owned gives documentation a stale-checkable
 * representation of opcode assignments and role-derived framing shape.
 */
export function render_protocol_spec_operation_table(contract: Wire_Contract): string {
  const modeled_operations = contract.operations
  if (modeled_operations === undefined) {
    throw new Error("protocol operation metadata is required for the specification table")
  }
  const operations = modeled_operations.filter(
    (operation) =>
      operation.contract.experimental !== true &&
      operation.contract.out_of_band !== true,
  )
  const request_layout = (operation: Wire_Operation): string => {
    const request_plan = operation.contract.request_plan ?? []
    const descriptor = derive_wire_operation_descriptor(operation.contract)
    const field_name = (index: number): string =>
      request_plan[index]?.path.join(".") ?? `field[${index}]`
    const step_layout = (step: Wire_Request_Step): string => {
      switch (step.kind) {
        case "fixed_field":
          return `${field_name(step.field)} (${step.bytes} bytes)`
        case "packed":
          return `packed(${step.fields.map((field) => field_name(field.field)).join(", ")})`
        case "byte_length_field":
          return `u8 length + ${field_name(step.field)}`
        case "byte_length_prefix_field":
          return `u8 length(${field_name(step.field)})`
        case "byte_field":
          return field_name(step.field)
        case "varuint_field":
          return `vu128(${field_name(step.field)})`
        case "value_length_field":
          return `vu128 length(${field_name(step.field)})`
        case "conditional":
          return `if ${field_name(step.field)}=${step.equals}: ${
            step.steps.map(step_layout).join(" + ")
          }`
        case "constant":
          return `constant 0x${
            step.bytes.map((byte) => byte.toString(16).padStart(2, "0")).join("")
          }`
        case "trailing_field":
          return `vu128 length + ${field_name(step.field)}`
      }
    }
    if (operation.contract.request_wire !== undefined) {
      return `opcode + request ID + ${
        operation.contract.request_wire.map(step_layout).join(" + ")
      }`
    }
    switch (descriptor.request_framing) {
      case "empty":
        return "opcode + request ID"
      case "opaque":
        return "opcode + request ID + value_len + value"
      case "ordered_fields":
        return descriptor.request_frame === "fixed_body"
          ? "opcode + request ID + fixed-width dense body"
          : "opcode + request ID + field_sequence_len + ordered field sequence"
    }
  }
  const response_payload = (operation: Wire_Operation): string => {
    const descriptor = derive_wire_operation_descriptor(operation.contract)
    // Generic ordered responses are defined by their complete generated plan,
    // not by a compatibility role count. This keeps documentation
    // correct for APIs whose fields are named `counter`, `enabled`, `status`,
    // or any other open semantic role.
    const value_count = operation.contract.response_plan?.length ?? 0
    switch (descriptor.response_framing) {
      case "empty":
        return "empty"
      case "opaque":
        return "opaque payload"
      case "optional_values":
        return value_count === 1
          ? "one ordered optional field"
          : `${value_count} ordered optional fields`
      case "field_sequence":
        return value_count === 1
          ? "one compact ordered field"
          : `${value_count} compact ordered fields`
    }
  }
  const field_codecs = (fields: readonly Wire_Operation_Field_Plan[]): string => {
    const codecs = fields.flatMap((field) => field.codecs ?? [])
    const unique = [...new Set(codecs)]
    return unique.length === 0 ? "N/A" : unique.map((codec) => `\`${codec}\``).join(", ")
  }
  const rows = operations
    .map((operation) => {
      const opcode = contract.opcodes.find((entry) => entry.name === operation.name)
      if (opcode === undefined) {
        throw new Error(`operation ${operation.name} has no opcode`)
      }
      return `| \`${opcode.value.toString(16).padStart(2, "0").toUpperCase()}\` | \`${wire_name(operation.name).toUpperCase()}\` | ${request_layout(operation)} | ${response_payload(operation)} | ${field_codecs(operation.contract.request_plan ?? [])} | ${field_codecs(operation.contract.response_plan ?? [])} |`
    })
    .join("\n")
  return `| Opcode | Name | Request layout | Response payload | Request codecs | Response codecs |
|---|---|---|---|---|---|
${rows}`
}

export const PROTOCOL_SPEC_OPERATION_TABLE_START =
  "<!-- openkache:generated-protocol-operation-table:start -->"
export const PROTOCOL_SPEC_OPERATION_TABLE_END =
  "<!-- openkache:generated-protocol-operation-table:end -->"

/** Returns the stale generated operation-table paths in a protocol spec. */
export function protocol_spec_operation_table_issues(
  spec: string,
  contract: Wire_Contract,
): readonly string[] {
  const start = spec.indexOf(PROTOCOL_SPEC_OPERATION_TABLE_START)
  const end = spec.indexOf(PROTOCOL_SPEC_OPERATION_TABLE_END)
  if (start < 0 || end < 0 || end < start) {
    return ["protocol/SPEC.md (generated operation table markers missing)"]
  }
  const actual = spec
    .slice(start + PROTOCOL_SPEC_OPERATION_TABLE_START.length, end)
    .trim()
  const expected = render_protocol_spec_operation_table(contract).trim()
  return actual === expected ? [] : ["protocol/SPEC.md (generated operation table stale)"]
}
