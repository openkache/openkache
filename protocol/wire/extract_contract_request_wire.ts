/** Parses and validates declarative request-wire steps from an operation trait. */

import {
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

/**
 * Converts a Smithy requestWire trait into the operation-neutral request plan.
 *
 * The parser only understands wire primitives (fixed fields, packed fields,
 * lengths, conditions, constants, and trailing values).  It deliberately
 * does not interpret field roles such as `namespace_id` or `value`; that
 * meaning belongs to a compatibility adapter or API-owned binding.
 */
export function request_wire_plan(
  contract: Json_Object,
  fields: readonly Wire_Operation_Field_Plan[],
  operation_location: string,
): readonly Wire_Request_Step[] | undefined {
  const raw = contract.requestWire
  if (raw === undefined) return undefined
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
  const validate_symbolic_value = (
    field: Wire_Operation_Field_Plan,
    value: string,
    location: string,
  ): void => {
    const allowed = field.shape === "Boolean"
      ? ["false", "true"]
      : field.enum_values
    if (allowed !== undefined && !allowed.includes(value)) {
      throw new Error(
        `${location} must be one of the modeled values: ${allowed.join(", ")}`,
      )
    }
  }
  const parse_steps = (
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
              `${step_location}.${kind}.bytes must match ${name}'s encoded width ` +
                `${field.encoded_width}`,
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
              const symbolic = string_member(mapping, "value", mapping_location)
              validate_symbolic_value(field, symbolic, `${mapping_location}.value`)
              const bits = integer_member(mapping, "bits", mapping_location, 0, 0xff)
              if ((bits & ~mask) !== 0) {
                throw new Error(`${mapping_location}.bits exceed the packed field mask`)
              }
              if (seen_values.has(symbolic) || seen_bits.has(bits)) {
                throw new Error(`${mapping_location} duplicates a packed value or bit pattern`)
              }
              seen_values.add(symbolic)
              seen_bits.add(bits)
              return { value: symbolic, bits }
            })
            return { field: field.index, mask, values }
          })
          const reserved_mask = optional_integer_member(
            value,
            "reservedMask",
            `${step_location}.${kind}`,
            0,
            0xff,
          ) ?? 0
          const constant_bits = optional_integer_member(
            value,
            "constantBits",
            `${step_location}.${kind}`,
            0,
            0xff,
          ) ?? 0
          if ((reserved_mask & occupied) !== 0 || (constant_bits & occupied) !== 0) {
            throw new Error(
              `${step_location}.${kind} reserved/constant bits overlap packed fields`,
            )
          }
          if ((reserved_mask & constant_bits) !== 0) {
            throw new Error(
              `${step_location}.${kind} constant bits overlap the reserved mask`,
            )
          }
          return {
            kind: "packed",
            fields: packed_fields,
            reserved_mask,
            constant_bits,
          }
        }
        case "byteLengthField":
        case "varuintField": {
          const name = string_member(value, "field", `${step_location}.${kind}`)
          const field = resolve_field(name, `${step_location}.${kind}.field`)
          return kind === "byteLengthField"
            ? { kind: "byte_length_field", field: field.index }
            : { kind: "varuint_field", field: field.index }
        }
        case "conditional": {
          const name = string_member(value, "field", `${step_location}.${kind}`)
          const field = resolve_field(name, `${step_location}.${kind}.field`)
          const equals = string_member(value, "equals", `${step_location}.${kind}`)
          validate_symbolic_value(field, equals, `${step_location}.${kind}.equals`)
          const nested = array_member(value, "steps", `${step_location}.${kind}`)
          if (nested.length === 0) {
            throw new Error(`${step_location}.${kind}.steps must not be empty`)
          }
          return {
            kind: "conditional",
            field: field.index,
            equals,
            steps: parse_steps(nested, `${step_location}.${kind}.steps`, depth + 1),
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
            bytes: Array.from(
              { length: hex.length / 2 },
              (_, byte) => Number.parseInt(hex.slice(byte * 2, byte * 2 + 2), 16),
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
  const request_wire = parse_steps(raw, `${operation_location}.requestWire`)
  const covered = new Set<number>()
  const validate_steps = (
    steps: readonly Wire_Request_Step[],
    location: string,
    assigned: ReadonlySet<number>,
    packed_values: ReadonlyMap<number, ReadonlySet<string>>,
    allow_trailing: boolean,
  ): void => {
    const available = new Set(assigned)
    const mappings = new Map(packed_values)
    for (const [index, step] of steps.entries()) {
      const step_location = `${location}[${index}]`
      const assign = (field: number): void => {
        if (available.has(field)) {
          throw new Error(
            `${step_location} assigns request field ${fields[field]?.path.join(".")} more than once`,
          )
        }
        available.add(field)
        covered.add(field)
      }
      switch (step.kind) {
        case "fixed_field":
        case "byte_length_field":
        case "varuint_field":
          assign(step.field)
          break
        case "packed":
          for (const field of step.fields) {
            assign(field.field)
            mappings.set(
              field.field,
              new Set(field.values.map((value) => value.value)),
            )
          }
          break
        case "conditional": {
          if (!mappings.get(step.field)?.has(step.equals)) {
            throw new Error(
              `${step_location} condition must reference a preceding packed field mapping`,
            )
          }
          validate_steps(
            step.steps,
            `${step_location}.conditional.steps`,
            available,
            mappings,
            false,
          )
          break
        }
        case "constant":
          break
        case "trailing_field":
          if (!allow_trailing || index + 1 !== steps.length) {
            throw new Error(
              `${step_location} trailing field must be the final top-level requestWire step`,
            )
          }
          assign(step.field)
          break
      }
    }
  }
  validate_steps(
    request_wire,
    `${operation_location}.requestWire`,
    new Set(),
    new Map(),
    true,
  )

  const leaf_fields = fields.filter((field) =>
    !fields.some((candidate) =>
      candidate.path.length > field.path.length &&
      field.path.every((part, index) => candidate.path[index] === part)
    )
  )
  const missing = leaf_fields.find((field) => !covered.has(field.index))
  if (missing !== undefined) {
    throw new Error(
      `${operation_location}.requestWire does not encode leaf request field ` +
        missing.path.join("."),
    )
  }
  return request_wire
}
