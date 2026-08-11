/** Renders and validates the generated protocol documentation sections. */

import {
  derive_wire_operation_descriptor,
  type Wire_Contract,
  type Wire_Entry,
  type Wire_Operation,
  type Wire_Operation_Field_Plan,
} from "./wire"
import {
  derive_wire_compatibility_response_semantics,
  derive_wire_compatibility_retry_mode,
  derive_wire_compatibility_scope,
} from "./compatibility_v1"

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
  const operations = contract.operations
  if (operations === undefined) {
    throw new Error("protocol operation metadata is required for the specification table")
  }
  const request_layout = (operation: Wire_Operation): string => {
    const descriptor = derive_wire_operation_descriptor(operation.contract)
    if (operation.contract.request_wire !== undefined) {
      return "opcode + generated exact field plan"
    }
    switch (descriptor.request_framing) {
      case "empty":
        return "opcode only"
      case "opaque":
        return "opcode + value_len + value"
      case "ordered_fields":
        return descriptor.request_frame === "fixed_body"
          ? "opcode + fixed-width dense body"
          : "opcode + field_sequence_len + ordered field sequence"
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
    return unique.length === 0 ? "—" : unique.map((codec) => `\`${codec}\``).join(", ")
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

export const PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START =
  "<!-- openkache:generated-protocol-contract-snapshot:start -->"
export const PROTOCOL_SPEC_CONTRACT_SNAPSHOT_END =
  "<!-- openkache:generated-protocol-contract-snapshot:end -->"

/**
 * Renders the shape-level contract snapshot used by `SPEC.md`.
 *
 * The operation table is intentionally concise for readers. This second,
 * generated block records requiredness, nested paths, codecs, and declared
 * statuses so a model change cannot silently leave the normative prose
 * describing a different contract. It is still documentation output, not a
 * second input source.
 */
export function render_protocol_spec_contract_snapshot(
  contract: Wire_Contract,
): string {
  const operations = contract.operations
  if (operations === undefined) {
    throw new Error("protocol operation metadata is required for the contract snapshot")
  }
  const opcode_for = (operation: Wire_Operation): Wire_Entry => {
    const opcode = contract.opcodes.find((entry) => entry.name === operation.name)
    if (opcode === undefined) {
      throw new Error(`operation ${operation.name} has no opcode`)
    }
    return opcode
  }
  const plan = (fields: readonly Wire_Operation_Field_Plan[]): string =>
    fields.length === 0
      ? "—"
      : fields
        .map((field) => {
          const required = field.required ? "!" : "?"
          const codecs = field.codecs === undefined || field.codecs.length === 0
            ? ""
            : `<${[...new Set(field.codecs)].join(",")}>`
          const enum_values = field.enum_values === undefined ||
              field.enum_values.length === 0
            ? ""
            : `{${field.enum_values.join("|")}}`
          return `${required}${field.role}@${field.path.join(".")}:${field.shape}${codecs}${enum_values}`
        })
        .join("; ")
  const operation_scope = (operation: Wire_Operation): string =>
    derive_wire_compatibility_scope(operation.contract)
  const operation_retry_mode = (operation: Wire_Operation): string =>
    derive_wire_compatibility_retry_mode(operation.contract)
  const operation_semantics = (operation: Wire_Operation): string =>
    derive_wire_compatibility_response_semantics(operation.contract) ?? "-"
  const rows = operations
    .map((operation) => {
      const operation_contract = operation.contract
      const opcode = opcode_for(operation)
      return `| \`${opcode.value.toString(16).padStart(2, "0").toUpperCase()}\` | \`${wire_name(operation.name).toUpperCase()}\` | \`${operation_scope(operation)}\` | \`${operation_retry_mode(operation)}\` | \`${operation_semantics(operation)}\` | \`${operation_contract.success_statuses.join(",")}\` | \`${operation_contract.error_statuses.join(",")}\` | \`${plan(operation_contract.request_plan ?? [])}\` | \`${plan(operation_contract.response_plan ?? [])}\` |`
    })
    .join("\n")
  return `| Opcode | Name | Scope | Retry | Semantics | Success statuses | Error statuses | Request plan | Response plan |
|---|---|---|---|---|---|---|---|---|
${rows}`
}

/** Returns stale or missing generated contract-snapshot markers in `SPEC.md`. */
export function protocol_spec_contract_snapshot_issues(
  spec: string,
  contract: Wire_Contract,
): readonly string[] {
  const start = spec.indexOf(PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START)
  const end = spec.indexOf(PROTOCOL_SPEC_CONTRACT_SNAPSHOT_END)
  if (start < 0 || end < 0 || end < start) {
    return ["protocol/SPEC.md (generated operation contract snapshot markers missing)"]
  }
  const actual = spec
    .slice(start + PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START.length, end)
    .trim()
  const expected = render_protocol_spec_contract_snapshot(contract).trim()
  return actual === expected
    ? []
    : ["protocol/SPEC.md (generated operation contract snapshot stale)"]
}
