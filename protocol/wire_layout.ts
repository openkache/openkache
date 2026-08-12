/** Generic wire-layout selection and allocation-free size planning. */

import {
  DEFAULT_SHAPE_CODECS,
  OPTIONAL_VALUE_LENGTH_BYTES,
  OPTIONAL_VALUE_MISSING,
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
  if (framing === "optional_values") return "optional_values"
  if (
    framing !== "ordered_fields" &&
    framing !== "field_sequence"
  ) {
    // Unknown response framings belong to the adapter that declared them.
    // Keeping this marker opaque prevents a future adapter from adding a
    // branch to the generic layout planner.
    return "adapter_owned"
  }
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
 * Computes the payload cost for a generated field layout without allocating
 * field bytes. `undefined` means a missing optional field. Dense and opaque
 * layouts reject missing or extra entries because their shape is fixed.
 * Adapter-owned layouts are deliberately not interpreted here.
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
    case "optional_values": {
      let total = 0
      for (const length of lengths) {
        if (
          length !== undefined &&
          (!Number.isSafeInteger(length) ||
            length < 0 ||
            length >= OPTIONAL_VALUE_MISSING)
        ) {
          throw new Error(`optional value length is outside the u32 range: ${length}`)
        }
        total = bounded_add(
          total,
          OPTIONAL_VALUE_LENGTH_BYTES + (length ?? 0),
          Number.MAX_SAFE_INTEGER,
        )
      }
      return total
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
    case "adapter_owned":
      throw new Error("adapter-owned layout requires an adapter size planner")
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
  framing: Wire_Request_Framing | Wire_Response_Framing,
  layout: Wire_Operation_Field_Layout,
  plan: readonly Wire_Operation_Field_Plan[],
): number {
  if (framing === "empty" || layout === "empty") return 0
  if (layout === "adapter_owned") {
    // The adapter owns any prefix, sentinel, or aggregate shape. Generic
    // admission still enforces the protocol-wide value ceiling.
    return max_value_bytes
  }
  if (layout === "optional_values") {
    const prefix_bytes = Math.min(
      max_value_bytes,
      plan.length * OPTIONAL_VALUE_LENGTH_BYTES,
    )
    let payload_bound = prefix_bytes
    for (const field of plan) {
      if (payload_bound >= max_value_bytes) return max_value_bytes
      const width = fixed_field_width(field) ?? max_value_bytes
      payload_bound = bounded_add(payload_bound, width, max_value_bytes)
    }
    return Math.min(max_value_bytes, payload_bound)
  }
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
  // canonical lengths.
  const prefix_bytes = Math.min(max_value_bytes, Math.ceil(plan.length / 8))
  let payload_bound = prefix_bytes
  for (const [index, field] of plan.entries()) {
    if (payload_bound >= max_value_bytes) return max_value_bytes
    const width = fixed_field_width(field) ?? max_value_bytes
    const length_bytes = index + 1 === plan.length
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
