/** Generic wire-layout selection and allocation-free size planning. */

import {
  DEFAULT_SHAPE_CODECS,
  WIRE_CODEC_DESCRIPTORS,
  WIRE_CODEC_NAMES,
  type Wire_Codec_Name,
  type Wire_Operation_Field_Layout,
  type Wire_Operation_Field_Plan,
  type Wire_Request_Framing,
  type Wire_Response_Framing,
} from "./wire_types"

/** Returns the exact encoded width of a field when its codec is fixed-width. */
export function fixed_field_width(
  field: Wire_Operation_Field_Plan,
): number | undefined {
  // The plan is flattened before layout selection. A nested fixed record is
  // therefore just a sequence of required fixed-width leaves; path depth must
  // not force it onto the variable sequence layout. Containers and unions
  // remain variable unless their codec descriptor explicitly supplies a
  // fixed width.
  if (!field.required) return undefined
  const codec = field.codecs?.[0] ??
    DEFAULT_SHAPE_CODECS[field.shape]
  const codec_index = field.codecs?.indexOf(codec) ?? -1
  const declared_width = codec_index >= 0
    ? field.codec_widths?.[codec_index]
    : undefined
  if (codec === undefined) return undefined
  const descriptor = WIRE_CODEC_NAMES.includes(codec as Wire_Codec_Name)
    ? WIRE_CODEC_DESCRIPTORS[codec as Wire_Codec_Name]
    : undefined
  if (descriptor === undefined) return undefined
  if (
    descriptor.container ||
    descriptor.recursive ||
    descriptor.cardinality !== "scalar"
  ) {
    return undefined
  }
  if (declared_width !== undefined) return declared_width
  return descriptor.width === "fixed"
    ? descriptor.min_width
    : undefined
}

/**
 * Selects the compact layout for a flattened field plan.
 *
 * Dense encoding is safe only when the flattened plan is all-required and
 * every leaf has an exact width. Optional, repeated, and variable-width
 * shapes use the general presence-mask sequence instead; nested fixed
 * records can remain dense after flattening.
 */
export function field_layout(
  plan: readonly Wire_Operation_Field_Plan[] | undefined,
  framing: Wire_Request_Framing | Wire_Response_Framing,
): Wire_Operation_Field_Layout {
  if (framing === "empty") return "empty"
  if (framing === "opaque") return "opaque"
  // Optional-values framing carries one fixed length/sentinel prefix per
  // field even when every underlying codec happens to be fixed-width. Keep
  // this explicit in the generated layout so adapters cannot accidentally
  // decode the fixed optional-field table with the generic presence-mask
  // sequence codec.
  if (framing === "optional_values") return "optional_values"
  return plan !== undefined &&
      plan.length > 0 &&
      plan.every((field) => fixed_field_width(field) !== undefined)
    ? "dense"
    : "sequence"
}

/** Returns the exact body width of a dense fixed-width field plan. */
export function fixed_plan_width(
  plan: readonly Wire_Operation_Field_Plan[] | undefined,
): number | undefined {
  if (plan === undefined || plan.length === 0) return undefined
  let width = 0
  for (const field of plan) {
    const field_width = fixed_field_width(field)
    if (field_width === undefined) return undefined
    if (width > Number.MAX_SAFE_INTEGER - field_width) {
      throw new Error("dense field plan width exceeds the safe integer range")
    }
    width += field_width
  }
  return width
}

function vu128_width(value: number): number {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`cannot calculate vu128 width for invalid length ${value}`)
  }
  let width = 1
  let limit = 0x80
  while (value >= limit && width < 9) {
    width += 1
    limit *= 0x80
  }
  if (value >= limit) {
    throw new Error(`length ${value} exceeds the supported vu128 range`)
  }
  return width
}

function bounded_add(value: number, increment: number, ceiling: number): number {
  if (
    !Number.isSafeInteger(value) ||
    value < 0 ||
    !Number.isSafeInteger(increment) ||
    increment < 0 ||
    !Number.isSafeInteger(ceiling) ||
    ceiling < 0
  ) {
    throw new Error("layout size arithmetic requires non-negative safe integers")
  }
  if (value >= ceiling || increment >= ceiling - value) return ceiling
  return value + increment
}

/**
 * Computes a generic presence-mask field-sequence payload size without
 * allocating field bytes. `undefined` is missing; `0` is a present-empty
 * field. Every present field before the final present field carries a
 * canonical length prefix; the final present field consumes the remainder.
 *
 * This is intentionally a wire primitive rather than an operation-family
 * helper, so generated planners can compare layouts before choosing a buffer
 * or response projection.
 */
export function field_sequence_encoded_len_from_lengths(
  lengths: readonly (number | undefined)[],
): number {
  const mask_bytes = Math.ceil(lengths.length / 8)
  let encoded_length = mask_bytes
  let last_present = -1
  for (const [index, length] of lengths.entries()) {
    if (
      length !== undefined &&
      (!Number.isSafeInteger(length) || length < 0)
    ) {
      throw new Error(`field length must be a non-negative safe integer: ${length}`)
    }
    if (length !== undefined) last_present = index
  }
  for (const [index, length] of lengths.entries()) {
    if (length === undefined) continue
    const field_cost = bounded_add(
      index === last_present ? 0 : vu128_width(length),
      length,
      Number.MAX_SAFE_INTEGER,
    )
    encoded_length = bounded_add(
      encoded_length,
      field_cost,
      Number.MAX_SAFE_INTEGER,
    )
  }
  return encoded_length
}

/**
 * Computes the fixed optional-value table size from lengths.
 *
 * The table is an explicit reusable layout primitive. A compatibility adapter
 * may select it to preserve an existing byte contract, while a future API may
 * use the same presence-preserving representation without adding a new
 * operation family.
 */
export function optional_values_encoded_len_from_lengths(
  lengths: readonly (number | undefined)[],
): number {
  let encoded_length = 0
  for (const length of lengths) {
    if (
      length !== undefined &&
      (!Number.isSafeInteger(length) || length < 0 || length >= 0xffff_ffff)
    ) {
      throw new Error(`optional value length is outside the u32 range: ${length}`)
    }
    const field_cost = 4 + (length ?? 0)
    encoded_length = bounded_add(encoded_length, field_cost, Number.MAX_SAFE_INTEGER)
  }
  return encoded_length
}

/**
 * Computes the payload cost for a generated field layout without allocating
 * field bytes. `undefined` means a missing optional field. Dense and opaque
 * layouts reject missing or extra entries because their shape is fixed.
 *
 * This is the single planner primitive used by operation IR consumers. It
 * keeps layout selection descriptor-driven while retaining the fixed
 * optional-value table as an explicit calculation.
 */
export function layout_encoded_len_from_lengths(
  layout: Wire_Operation_Field_Layout,
  lengths: readonly (number | undefined)[],
): number {
  switch (layout) {
    case "empty":
      if (lengths.length !== 0) {
        throw new Error("empty layout cannot carry field lengths")
      }
      return 0
    case "opaque": {
      const length = lengths[0]
      if (lengths.length !== 1 || length === undefined) {
        throw new Error("opaque layout requires one present field length")
      }
      return length
    }
    case "dense": {
      let total = 0
      for (const length of lengths) {
        if (length === undefined) {
          throw new Error("dense layout requires every field length")
        }
        total = bounded_add(total, length, Number.MAX_SAFE_INTEGER)
      }
      return total
    }
    case "sequence":
      return field_sequence_encoded_len_from_lengths(lengths)
    case "optional_values":
      return optional_values_encoded_len_from_lengths(lengths)
  }
}

/**
 * Computes an admission bound from the selected layout and codec widths.
 *
 * The protocol still enforces one aggregate value ceiling. This tighter
 * shape-derived bound prevents a fixed tuple or a small field sequence from
 * reserving the maximum value buffer for every in-flight request.
 */
export function layout_payload_bound(
  max_value_bytes: number,
  max_varuint_bytes: number,
  optional_length_bytes: number,
  framing: Wire_Request_Framing | Wire_Response_Framing,
  layout: Wire_Operation_Field_Layout,
  plan: readonly Wire_Operation_Field_Plan[],
): number {
  if (framing === "empty" || layout === "empty") return 0
  if (layout === "dense") {
    // A fixed tuple can still be wider than the aggregate protocol ceiling
    // when a model supplies an explicit codec width. Keep the generated
    // admission budget bounded by the same ceiling enforced by frame
    // validation; generation/runtime validation will reject the oversized
    // payload itself.
    return Math.min(max_value_bytes, fixed_plan_width(plan) ?? 0)
  }
  if (layout === "opaque") {
    const width = plan.length === 1 ? fixed_field_width(plan[0]!) : undefined
    return Math.min(max_value_bytes, width ?? max_value_bytes)
  }
  // Generic field sequences use one shared presence-mask prefix followed by
  // a canonical length for every present field except the final present
  // field, which consumes the remaining bytes. The explicit optional-value
  // table keeps its fixed-width prefix.
  const prefix_bytes = framing === "optional_values"
    ? optional_length_bytes > 0 &&
        plan.length > Math.floor(max_value_bytes / optional_length_bytes)
      ? max_value_bytes
      : Math.min(max_value_bytes, plan.length * optional_length_bytes)
    : Math.min(max_value_bytes, Math.ceil(plan.length / 8))
  let payload_bound = prefix_bytes
  for (const [index, field] of plan.entries()) {
    if (payload_bound >= max_value_bytes) return max_value_bytes
    const width = fixed_field_width(field) ?? max_value_bytes
    const length_bytes = framing === "optional_values" || index + 1 === plan.length
      ? 0
      : max_varuint_bytes
    payload_bound = bounded_add(
      bounded_add(payload_bound, length_bytes, max_value_bytes),
      width,
      max_value_bytes,
    )
  }
  return Math.min(max_value_bytes, payload_bound)
}
