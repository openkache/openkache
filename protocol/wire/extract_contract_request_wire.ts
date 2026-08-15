/** Parses declarative operation request-wire primitives. */

import {
  effective_field_codec,
  MAX_GENERATED_NESTED_CODEC_DEPTH,
  type Wire_Operation_Field_Plan,
  type Wire_Request_Step,
} from "../wire_types"
import {
  array_member,
  integer_member,
  object_value,
  optional_integer_member,
  string_member,
  type Json_Object,
} from "./validate_contract"

export function request_wire_plan(
  contract: Json_Object,
  fields: readonly Wire_Operation_Field_Plan[],
  operation_location: string,
): readonly Wire_Request_Step[] | undefined {
  const raw = contract.requestWire
  if (raw === undefined) {
    if (fields.length === 0) return undefined
    throw new Error(
      `${operation_location}.requestWire is required for a non-empty modeled request`,
    )
  }
  if (!Array.isArray(raw) || raw.length === 0) {
    throw new Error(`${operation_location}.requestWire must be a non-empty array`)
  }
  const by_path = new Map(fields.map((field) => [field.path.join("."), field]))
  const resolve_field = (name: string, location: string): Wire_Operation_Field_Plan => {
    const field = by_path.get(name)
    if (field === undefined) {
      throw new Error(`${location} references unknown request field ${JSON.stringify(name)}`)
    }
    return field
  }
  const symbolic = (
    field: Wire_Operation_Field_Plan,
    value: string,
    location: string,
  ): void => {
    const allowed = effective_field_codec(field) === "bool_u8"
      ? ["false", "true"]
      : field.enum_values
    if (allowed !== undefined && !allowed.includes(value)) {
      throw new Error(`${location} must be one of the modeled values: ${allowed.join(", ")}`)
    }
  }
  const parse = (
    values: readonly unknown[],
    location: string,
    depth = 0,
  ): readonly Wire_Request_Step[] => {
    if (depth > MAX_GENERATED_NESTED_CODEC_DEPTH) {
      throw new Error(`${location} exceeds the generated request-wire nesting bound`)
    }
    return values.map((raw_step, index) => {
      const step_location = `${location}[${index}]`
      const step = object_value(raw_step, step_location)
      const entries = Object.entries(step)
      if (entries.length !== 1) {
        throw new Error(`${step_location} must select exactly one request-wire primitive`)
      }
      const [kind, raw_value] = entries[0]!
      const value = object_value(raw_value, `${step_location}.${kind}`)
      switch (kind) {
        case "fixedField": {
          const name = string_member(value, "field", `${step_location}.${kind}`)
          const field = resolve_field(name, `${step_location}.${kind}.field`)
          const bytes = integer_member(value, "bytes", `${step_location}.${kind}`, 1)
          if (field.encoded_width !== undefined && field.encoded_width !== bytes) {
            throw new Error(
              `${step_location}.${kind}.bytes must match ${name}'s encoded width ${field.encoded_width}`,
            )
          }
          return { kind: "fixed_field", field: field.index, bytes }
        }
        case "packed": {
          const raw_fields = array_member(value, "fields", `${step_location}.${kind}`)
          if (raw_fields.length === 0) {
            throw new Error(`${step_location}.${kind}.fields must not be empty`)
          }
          let occupied = 0
          const packed_fields = raw_fields.map((raw_field, field_index) => {
            const field_location = `${step_location}.${kind}.fields[${field_index}]`
            const packed = object_value(raw_field, field_location)
            const name = string_member(packed, "field", field_location)
            const field = resolve_field(name, `${field_location}.field`)
            if (!["bool_u8", "enum"].includes(effective_field_codec(field) ?? "")) {
              throw new Error(
                `${field_location}.field must use the canonical bool_u8 or enum codec`,
              )
            }
            const mask = integer_member(packed, "mask", field_location, 1, 0xff)
            if ((occupied & mask) !== 0) {
              throw new Error(`${field_location}.mask overlaps another packed field`)
            }
            occupied |= mask
            const raw_values = array_member(packed, "values", field_location)
            if (raw_values.length === 0) {
              throw new Error(`${field_location}.values must not be empty`)
            }
            const seen_values = new Set<string>()
            const seen_bits = new Set<number>()
            const values = raw_values.map((raw_mapping, mapping_index) => {
              const mapping_location = `${field_location}.values[${mapping_index}]`
              const mapping = object_value(raw_mapping, mapping_location)
              const value_name = string_member(mapping, "value", mapping_location)
              symbolic(field, value_name, `${mapping_location}.value`)
              const bits = integer_member(mapping, "bits", mapping_location, 0, 0xff)
              if (
                (bits & ~mask) !== 0 ||
                seen_values.has(value_name) ||
                seen_bits.has(bits)
              ) {
                throw new Error(
                  `${mapping_location} contains an invalid or duplicate packed mapping`,
                )
              }
              seen_values.add(value_name)
              seen_bits.add(bits)
              return { value: value_name, bits }
            })
            return { field: field.index, mask, values }
          })
          const reserved_mask =
            optional_integer_member(value, "reservedMask", `${step_location}.${kind}`, 0, 0xff) ?? 0
          const constant_bits =
            optional_integer_member(value, "constantBits", `${step_location}.${kind}`, 0, 0xff) ?? 0
          if (
            (reserved_mask & occupied) !== 0 ||
            (constant_bits & occupied) !== 0 ||
            (reserved_mask & constant_bits) !== 0
          ) {
            throw new Error(
              `${step_location}.${kind} reserved/constant bits overlap packed fields`,
            )
          }
          return { kind: "packed", fields: packed_fields, reserved_mask, constant_bits }
        }
        case "byteLengthField":
        case "varuintField":
        case "byteLengthPrefixField":
        case "byteField": {
          const name = string_member(value, "field", `${step_location}.${kind}`)
          const field = resolve_field(name, `${step_location}.${kind}.field`)
          if (
            kind === "varuintField" &&
            effective_field_codec(field) !== "u64_be"
          ) {
            throw new Error(
              `${step_location}.${kind}.field must use the canonical u64_be codec`,
            )
          }
          return kind === "byteLengthField"
            ? { kind: "byte_length_field", field: field.index }
            : kind === "varuintField"
              ? { kind: "varuint_field", field: field.index }
              : kind === "byteLengthPrefixField"
                ? { kind: "byte_length_prefix_field", field: field.index }
                : { kind: "byte_field", field: field.index }
        }
        case "valueLengthField": {
          const name = string_member(value, "field", `${step_location}.${kind}`)
          const field = resolve_field(name, `${step_location}.${kind}.field`)
          const length = string_member(value, "length", `${step_location}.${kind}`)
          if (length !== "varuint") {
            throw new Error(`${step_location}.${kind}.length must be varuint`)
          }
          return { kind: "value_length_field", field: field.index, length }
        }
        case "conditional": {
          const name = string_member(value, "field", `${step_location}.${kind}`)
          const field = resolve_field(name, `${step_location}.${kind}.field`)
          const equals = string_member(value, "equals", `${step_location}.${kind}`)
          symbolic(field, equals, `${step_location}.${kind}.equals`)
          const nested = array_member(value, "steps", `${step_location}.${kind}`)
          if (nested.length === 0) {
            throw new Error(`${step_location}.${kind}.steps must not be empty`)
          }
          return {
            kind: "conditional",
            field: field.index,
            equals,
            steps: parse(nested, `${step_location}.${kind}.steps`, depth + 1),
          }
        }
        case "constant": {
          const hex = string_member(value, "hex", `${step_location}.${kind}`)
          if (!/^(?:[0-9a-f]{2})+$/.test(hex)) {
            throw new Error(
              `${step_location}.${kind}.hex must be non-empty lowercase hexadecimal bytes`,
            )
          }
          return {
            kind: "constant",
            bytes: Array.from({ length: hex.length / 2 }, (_, byte) =>
              Number.parseInt(hex.slice(byte * 2, byte * 2 + 2), 16),
            ),
          }
        }
        case "trailingField": {
          const name = string_member(value, "field", `${step_location}.${kind}`)
          const field = resolve_field(name, `${step_location}.${kind}.field`)
          const length = string_member(value, "length", `${step_location}.${kind}`)
          if (length !== "varuint") {
            throw new Error(`${step_location}.${kind}.length must be varuint`)
          }
          return { kind: "trailing_field", field: field.index, length }
        }
        default:
          throw new Error(`${step_location} selects unknown request-wire primitive ${kind}`)
      }
    })
  }
  const plan = parse(raw, `${operation_location}.requestWire`)
  // Coverage is path-insensitive so mutually exclusive conditionals may
  // project the same field. Runtime projection still rejects duplicate writes
  // along any decoded path.
  const projected = new Set<number>()
  const collect_projected_fields = (steps: readonly Wire_Request_Step[]): void => {
    for (const step of steps) {
      switch (step.kind) {
        case "fixed_field":
        case "byte_length_field":
        case "byte_field":
        case "varuint_field":
        case "value_length_field":
        case "trailing_field":
          projected.add(step.field)
          break
        case "packed":
          for (const field of step.fields) projected.add(field.field)
          break
        case "conditional":
          collect_projected_fields(step.steps)
          break
        case "byte_length_prefix_field":
        case "constant":
          break
        default: {
          const exhaustive_step: never = step
          throw new Error(`unsupported request wire step ${exhaustive_step}`)
        }
      }
    }
  }
  collect_projected_fields(plan)
  const missing = fields.filter((field) => {
    // Aggregate parents carry presence metadata; their projected children own
    // the actual compact bytes.
    const aggregate_parent = fields.some(
      (candidate) =>
        candidate.path.length > field.path.length &&
        field.path.every((part, index) => candidate.path[index] === part),
    )
    return !aggregate_parent && !projected.has(field.index)
  })
  if (missing.length !== 0) {
    throw new Error(
      `${operation_location}.requestWire must project every modeled request leaf field; ` +
        `missing ${missing.map((field) => field.path.join(".")).join(", ")}`,
    )
  }
  return plan
}
