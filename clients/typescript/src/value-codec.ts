/** Runtime-neutral StructuredValue-CBOR-v1 codec and lossless model. */

const TEXT_ENCODER = new TextEncoder()
const TEXT_DECODER = new TextDecoder("utf-8", { fatal: true })

function describe_value(value: unknown): string {
  if (typeof value === "bigint") return "bigint"
  if (typeof value === "undefined") return "undefined"
  if (typeof value === "function") return "function"
  if (typeof value === "symbol") return "symbol"
  return Object.prototype.toString.call(value)
}

function is_regular_object(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

// ---------------------------------------------------------------------------
// Structured value model
// ---------------------------------------------------------------------------

/** Stable local value-codec error categories. */
export type Structured_Value_Error_Kind =
  | "conversion"
  | "resource_limit"
  | "truncated"
  | "trailing_bytes"
  | "invalid_encoding"
  | "unsupported_type"
  | "invalid_utf8"
  | "invalid_integer"
  | "non_scalar_key"
  | "duplicate_key"

/** Error raised by native conversion or structured-value parsing. */
export class Structured_Value_Error extends Error {
  readonly kind: Structured_Value_Error_Kind

  constructor(
    message: string,
    kind: Structured_Value_Error_Kind = "conversion",
  ) {
    super(message)
    this.name = "Structured_Value_Error"
    this.kind = kind
  }
}

/** One shared structural budget for structured value operations. */
export interface Value_Limits {
  readonly max_bytes?: number
  readonly max_depth?: number
  readonly max_items?: number
  readonly max_integer_bytes?: number
}

const DEFAULT_VALUE_LIMITS: Required<Value_Limits> = {
  max_bytes: 67_108_864,
  max_depth: 128,
  max_items: 1_000_000,
  max_integer_bytes: 1 << 20,
}
const MAX_ALLOWED_VALUE_DEPTH = 1_024
const MISSING_VALUE = Symbol("missing structured value root")

interface Conversion_State {
  items: number
  bytes: number
}

/** Lossless model value for JavaScript ``undefined``. */
export class Undefined_Value {
  readonly kind = "undefined" as const
}

/** Lossless model arbitrary-precision integer. */
export class Integer_Value {
  readonly kind = "integer" as const
  readonly value: bigint

  constructor(value: bigint) {
    if (typeof value !== "bigint") {
      throw new Structured_Value_Error("Integer_Value requires a bigint")
    }
    this.value = value
  }
}

/** Lossless model float retaining IEEE width and raw bits. */
export class Float_Value {
  readonly kind = "float" as const
  readonly width: 16 | 32 | 64
  readonly raw_bits: bigint

  constructor(width: 16 | 32 | 64, raw_bits: bigint) {
    if (
      (width !== 16 && width !== 32 && width !== 64) ||
      typeof raw_bits !== "bigint" ||
      raw_bits < 0n ||
      raw_bits >= 1n << BigInt(width)
    ) {
      throw new Structured_Value_Error("invalid Float_Value width or raw bits")
    }
    this.width = width
    this.raw_bits = raw_bits
  }
}

/** Lossless model uninterpreted bytes. */
export class ByteString_Value {
  readonly kind = "bytes" as const
  readonly value: Uint8Array

  constructor(value: Uint8Array) {
    if (!(value instanceof Uint8Array)) {
      throw new Structured_Value_Error("ByteString_Value requires Uint8Array")
    }
    this.value = value.slice()
  }
}

/** Lossless model UTF-8 text. */
export class TextString_Value {
  readonly kind = "text" as const
  readonly value: string

  constructor(value: string) {
    if (typeof value !== "string") {
      throw new Structured_Value_Error("TextString_Value requires string")
    }
    assert_valid_unicode_string(value)
    this.value = value
  }
}

/** Lossless model ordered array. */
export class Array_Value {
  readonly kind = "array" as const
  readonly values: readonly Structured_Value[]

  constructor(values: readonly unknown[]) {
    if (!Array.isArray(values)) {
      throw new Structured_Value_Error("Array_Value requires an array")
    }
    const converted: Structured_Value[] = []
    for (let index = 0; index < values.length; index += 1) {
      if (!Object.prototype.hasOwnProperty.call(values, index)) {
        throw new Structured_Value_Error(
          `array entry ${index} is sparse`,
          "conversion",
        )
      }
      const value = values[index]
      converted.push(is_model_value(value) ? value : to_value(value))
    }
    this.values = converted
  }

  get length(): number {
    return this.values.length
  }

  at(index: number): Structured_Value | undefined {
    return this.values[index]
  }

  [Symbol.iterator](): Iterator<Structured_Value> {
    return this.values[Symbol.iterator]()
  }
}

/** Lossless model ordered map with scalar, structurally unique keys. */
export class Map_Value {
  readonly kind = "map" as const
  readonly entries: readonly (readonly [Structured_Value, Structured_Value])[]

  constructor(entries: readonly (readonly [unknown, unknown])[]) {
    if (!Array.isArray(entries)) {
      throw new Structured_Value_Error("Map_Value requires entry pairs")
    }
    const converted: [Structured_Value, Structured_Value][] = []
    const key_identities = new Set<string>()
    for (let index = 0; index < entries.length; index += 1) {
      if (!Object.prototype.hasOwnProperty.call(entries, index)) {
        throw new Structured_Value_Error(
          `map entry ${index} is sparse`,
          "conversion",
        )
      }
      const entry = entries[index]
      if (!Array.isArray(entry) || entry.length !== 2) {
        throw new Structured_Value_Error(
          `map entry ${index} must be a two-item pair`,
        )
      }
      if (
        !Object.prototype.hasOwnProperty.call(entry, 0) ||
        !Object.prototype.hasOwnProperty.call(entry, 1)
      ) {
        throw new Structured_Value_Error(
          `map entry ${index} is sparse`,
          "conversion",
        )
      }
      const key = is_model_value(entry[0]) ? entry[0] : to_value(entry[0])
      const value = is_model_value(entry[1]) ? entry[1] : to_value(entry[1])
      validate_map_key(key, index, key_identities)
      converted.push([key, value])
    }
    this.entries = converted
  }

  get size(): number {
    return this.entries.length
  }

  get(key: unknown): Structured_Value | undefined {
    const sought = to_value(key)
    const matches = this.entries
      .filter(([candidate]): boolean => model_equal(candidate, sought))
      .map(([, value]): Structured_Value => value)
    if (matches.length > 1) {
      throw new Structured_Value_Error(
        "map lookup is ambiguous",
        "duplicate_key",
      )
    }
    return matches[0]
  }

  has(key: unknown): boolean {
    const sought = to_value(key)
    return this.entries.some(([candidate]): boolean => model_equal(candidate, sought))
  }

  keys(): IterableIterator<Structured_Value> {
    return this.entries.map(([key]): Structured_Value => key)[Symbol.iterator]()
  }

  values(): IterableIterator<Structured_Value> {
    return this.entries.map(([, value]): Structured_Value => value)[Symbol.iterator]()
  }

  [Symbol.iterator](): Iterator<readonly [Structured_Value, Structured_Value]> {
    return this.entries[Symbol.iterator]()
  }
}

/** Complete lossless StructuredValue model. */
export type Structured_Value =
  | null
  | boolean
  | Undefined_Value
  | Integer_Value
  | Float_Value
  | ByteString_Value
  | TextString_Value
  | Array_Value
  | Map_Value

function is_model_value(value: unknown): value is Structured_Value {
  return (
    value === null ||
    typeof value === "boolean" ||
    value instanceof Undefined_Value ||
    value instanceof Integer_Value ||
    value instanceof Float_Value ||
    value instanceof ByteString_Value ||
    value instanceof TextString_Value ||
    value instanceof Array_Value ||
    value instanceof Map_Value
  )
}

/** Singleton helper for constructing the model's undefined value. */
export const UNDEFINED_VALUE = new Undefined_Value()

/**
 * Converts a JavaScript native value into the lossless model.
 *
 * @param value - Native or already-modelled structured value.
 * @param limits - Structural resource limits used while converting native
 *   containers.
 * @returns The lossless structured-value model.
 * @throws {Structured_Value_Error} When conversion exceeds a resource limit
 *   or the native value cannot be represented losslessly.
 */
export function to_value(
  value: unknown,
  limits: Value_Limits = {},
): Structured_Value {
  return convert_value(
    value,
    new Set<object>(),
    0,
    checked_limits(limits),
    { items: 0, bytes: 0 },
  )
}

function convert_value(
  value: unknown,
  ancestors: Set<object>,
  depth: number,
  budget: Required<Value_Limits>,
  state: Conversion_State,
): Structured_Value {
  if (value instanceof Undefined_Value) {
    consume_items(state, budget, 1)
    consume_bytes(state, budget, 1)
    return value
  }
  if (value instanceof Integer_Value) {
    consume_items(state, budget, 1)
    consume_integer_bytes(value.value, budget)
    consume_bytes(state, budget, encoded_integer_size(value.value))
    return value
  }
  if (value instanceof Float_Value) {
    consume_items(state, budget, 1)
    consume_bytes(state, budget, 1 + value.width / 8)
    return value
  }
  if (value instanceof ByteString_Value) {
    consume_items(state, budget, 1)
    consume_bytes(
      state,
      budget,
      cbor_head_size(BigInt(value.value.byteLength)) + value.value.byteLength,
    )
    return value
  }
  if (value instanceof TextString_Value) {
    consume_items(state, budget, 1)
    const byte_length = TEXT_ENCODER.encode(value.value).byteLength
    consume_bytes(
      state,
      budget,
      cbor_head_size(BigInt(byte_length)) + byte_length,
    )
    return value
  }
  if (value instanceof Array_Value) {
    assert_acyclic(value, ancestors)
    try {
      if (depth >= budget.max_depth) resource("depth", budget.max_depth, depth + 1)
      consume_items(state, budget, 1)
      consume_bytes(state, budget, cbor_head_size(BigInt(value.values.length)))
      for (const child of value.values) {
        convert_value(child, ancestors, depth + 1, budget, state)
      }
      return value
    } finally {
      ancestors.delete(value)
    }
  }
  if (value instanceof Map_Value) {
    assert_acyclic(value, ancestors)
    try {
      if (depth >= budget.max_depth) resource("depth", budget.max_depth, depth + 1)
      consume_items(state, budget, 1)
      consume_bytes(state, budget, cbor_head_size(BigInt(value.entries.length)))
      const key_identities = new Set<string>()
      for (let index = 0; index < value.entries.length; index += 1) {
        const [key, child] = value.entries[index]!
        const converted_key = convert_value(
          key,
          ancestors,
          depth + 1,
          budget,
          state,
        )
        validate_map_key(converted_key, index, key_identities)
        convert_value(child, ancestors, depth + 1, budget, state)
      }
      return value
    } finally {
      ancestors.delete(value)
    }
  }
  if (typeof value === "undefined") {
    consume_items(state, budget, 1)
    consume_bytes(state, budget, 1)
    return UNDEFINED_VALUE
  }
  if (value === null || typeof value === "boolean") {
    consume_items(state, budget, 1)
    consume_bytes(state, budget, 1)
    return value
  }
  if (typeof value === "number") {
    consume_items(state, budget, 1)
    consume_bytes(state, budget, 9)
    const bits = new ArrayBuffer(8)
    new DataView(bits).setFloat64(0, value, false)
    return new Float_Value(64, new DataView(bits).getBigUint64(0, false))
  }
  if (typeof value === "bigint") {
    consume_items(state, budget, 1)
    consume_integer_bytes(value, budget)
    consume_bytes(state, budget, encoded_integer_size(value))
    return new Integer_Value(value)
  }
  if (typeof value === "string") {
    consume_items(state, budget, 1)
    const byte_length = TEXT_ENCODER.encode(value).byteLength
    consume_bytes(
      state,
      budget,
      cbor_head_size(BigInt(byte_length)) + byte_length,
    )
    return new TextString_Value(value)
  }
  if (value instanceof Uint8Array) {
    consume_items(state, budget, 1)
    consume_bytes(
      state,
      budget,
      cbor_head_size(BigInt(value.byteLength)) + value.byteLength,
    )
    return new ByteString_Value(value)
  }
  if (Array.isArray(value)) {
    assert_acyclic(value, ancestors)
    try {
      if (depth >= budget.max_depth) resource("depth", budget.max_depth, depth + 1)
      consume_items(state, budget, 1)
      consume_bytes(state, budget, cbor_head_size(BigInt(value.length)))
      const children: Structured_Value[] = []
      for (let index = 0; index < value.length; index += 1) {
        if (!Object.prototype.hasOwnProperty.call(value, index)) {
          throw new Structured_Value_Error(
            `array entry ${index} is sparse`,
            "conversion",
          )
        }
        children.push(
          convert_value(value[index], ancestors, depth + 1, budget, state),
        )
      }
      return new Array_Value(children)
    } finally {
      ancestors.delete(value)
    }
  }
  if (value instanceof Map) {
    assert_acyclic(value, ancestors)
    try {
      if (depth >= budget.max_depth) resource("depth", budget.max_depth, depth + 1)
      consume_items(state, budget, 1)
      consume_bytes(state, budget, cbor_head_size(BigInt(value.size)))
      const entries: [Structured_Value, Structured_Value][] = []
      const key_identities = new Set<string>()
      let index = 0
      for (const [key, child] of value.entries()) {
        const converted_key = convert_value(
          key,
          ancestors,
          depth + 1,
          budget,
          state,
        )
        validate_map_key(converted_key, index, key_identities)
        entries.push([
          converted_key,
          convert_value(child, ancestors, depth + 1, budget, state),
        ])
        index += 1
      }
      return new Map_Value(entries)
    } finally {
      ancestors.delete(value)
    }
  }
  if (is_regular_object(value)) {
    assert_acyclic(value, ancestors)
    try {
      if (depth >= budget.max_depth) resource("depth", budget.max_depth, depth + 1)
      consume_items(state, budget, 1)
      consume_bytes(state, budget, cbor_head_size(BigInt(Object.keys(value).length)))
      for (const symbol of Object.getOwnPropertySymbols(value)) {
        if (Object.prototype.propertyIsEnumerable.call(value, symbol)) {
          throw new Structured_Value_Error(
            "plain object contains enumerable symbol properties",
          )
        }
      }
      return new Map_Value(
        Object.keys(value).map(
          (key): readonly [Structured_Value, Structured_Value] => [
            convert_value(key, ancestors, depth + 1, budget, state),
            convert_value(value[key], ancestors, depth + 1, budget, state),
          ],
        ),
      )
    } finally {
      ancestors.delete(value)
    }
  }
  throw new Structured_Value_Error(
    `unsupported JavaScript value ${describe_value(value)}`,
  )
}

function consume_items(
  state: Conversion_State,
  budget: Required<Value_Limits>,
  count: number,
): void {
  if (
    !Number.isSafeInteger(count) ||
    count < 0 ||
    count > Number.MAX_SAFE_INTEGER - state.items
  ) {
    resource("items", budget.max_items, Number.MAX_SAFE_INTEGER)
  }
  const actual = state.items + count
  if (actual > budget.max_items) resource("items", budget.max_items, actual)
  state.items = actual
}

function consume_bytes(
  state: Conversion_State,
  budget: Required<Value_Limits>,
  count: number,
): void {
  if (
    !Number.isSafeInteger(count) ||
    count < 0 ||
    count > Number.MAX_SAFE_INTEGER - state.bytes
  ) {
    resource("bytes", budget.max_bytes, Number.MAX_SAFE_INTEGER)
  }
  const actual = state.bytes + count
  if (actual > budget.max_bytes) resource("bytes", budget.max_bytes, actual)
  state.bytes = actual
}

function consume_integer_bytes(
  value: bigint,
  budget: Required<Value_Limits>,
): void {
  const magnitude = value < 0n ? -value : value
  const count = magnitude === 0n ? 0 : Math.ceil(magnitude.toString(2).length / 8)
  if (count > budget.max_integer_bytes) {
    resource("integer bytes", budget.max_integer_bytes, count)
  }
}

function encoded_integer_size(value: bigint): number {
  const negative = value < 0n
  const transformed = negative ? -value - 1n : value
  if (transformed <= 0xffff_ffff_ffff_ffffn) {
    return cbor_head_size(transformed)
  }
  const magnitude_length = Math.ceil(transformed.toString(2).length / 8)
  return (
    cbor_head_size(2n) +
    cbor_head_size(BigInt(magnitude_length)) +
    magnitude_length
  )
}

function cbor_head_size(length: bigint): number {
  if (length < 24n) return 1
  if (length <= 0xffn) return 2
  if (length <= 0xffffn) return 3
  if (length <= 0xffff_ffffn) return 5
  if (length <= 0xffff_ffff_ffff_ffffn) return 9
  return Number.MAX_SAFE_INTEGER
}

function assert_acyclic(value: object, ancestors: Set<object>): void {
  if (ancestors.has(value)) {
    throw new Structured_Value_Error("value contains a cyclic reference")
  }
  ancestors.add(value)
}

function is_scalar_key(value: Structured_Value): boolean {
  return !(value instanceof Array_Value || value instanceof Map_Value)
}

function validate_map_key(
  key: Structured_Value,
  index: number,
  key_identities: Set<string>,
): void {
  if (!is_scalar_key(key)) {
    throw new Structured_Value_Error(
      `map key at entry ${index} is not scalar`,
      "non_scalar_key",
    )
  }
  const identity = scalar_key_identity(key)
  if (key_identities.has(identity)) {
    throw new Structured_Value_Error(
      `duplicate map key at entry ${index}`,
      "duplicate_key",
    )
  }
  key_identities.add(identity)
}

function scalar_key_identity(value: Structured_Value): string {
  if (value === null) return "null"
  if (typeof value === "boolean") return value ? "boolean:true" : "boolean:false"
  if (value instanceof Undefined_Value) return "undefined"
  if (value instanceof Integer_Value) return `integer:${value.value}`
  if (value instanceof Float_Value) {
    return `float:${value.width}:${value.raw_bits}`
  }
  if (value instanceof ByteString_Value) {
    return `bytes:${bytes_to_hex(value.value)}`
  }
  if (value instanceof TextString_Value) {
    return `text:${bytes_to_hex(TEXT_ENCODER.encode(value.value))}`
  }
  throw new Structured_Value_Error("map key is not scalar", "non_scalar_key")
}

function bytes_to_hex(value: Uint8Array): string {
  const chunks: string[] = []
  for (const byte of value) chunks.push(byte.toString(16).padStart(2, "0"))
  return chunks.join("")
}

/** Model structural equality, including float width/raw bits and key kinds. */
export function model_equal(
  left: Structured_Value,
  right: Structured_Value,
): boolean {
  if (left === null || right === null) return left === right
  if (typeof left === "boolean" || typeof right === "boolean") {
    return typeof left === "boolean" && typeof right === "boolean" && left === right
  }
  if (left instanceof Undefined_Value || right instanceof Undefined_Value) {
    return left instanceof Undefined_Value && right instanceof Undefined_Value
  }
  if (left instanceof Integer_Value || right instanceof Integer_Value) {
    return left instanceof Integer_Value && right instanceof Integer_Value && left.value === right.value
  }
  if (left instanceof Float_Value || right instanceof Float_Value) {
    return (
      left instanceof Float_Value &&
      right instanceof Float_Value &&
      left.width === right.width &&
      left.raw_bits === right.raw_bits
    )
  }
  if (left instanceof ByteString_Value || right instanceof ByteString_Value) {
    if (!(left instanceof ByteString_Value) || !(right instanceof ByteString_Value)) return false
    return bytes_equal(left.value, right.value)
  }
  if (left instanceof TextString_Value || right instanceof TextString_Value) {
    return left instanceof TextString_Value && right instanceof TextString_Value && left.value === right.value
  }
  if (left instanceof Array_Value || right instanceof Array_Value) {
    return (
      left instanceof Array_Value &&
      right instanceof Array_Value &&
      left.values.length === right.values.length &&
      left.values.every((value, index): boolean => model_equal(value, right.values[index]!))
    )
  }
  if (left instanceof Map_Value || right instanceof Map_Value) {
    if (!(left instanceof Map_Value) || !(right instanceof Map_Value) || left.size !== right.size) return false
    const unmatched = [...right.entries]
    for (const [key, value] of left.entries) {
      const index = unmatched.findIndex(([other_key, other_value]): boolean => model_equal(key, other_key) && model_equal(value, other_value))
      if (index < 0) return false
      unmatched.splice(index, 1)
    }
    return unmatched.length === 0
  }
  return false
}

function bytes_equal(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((byte, index): boolean => byte === right[index])
}

/** Encodes one complete native/model value as a structured-value payload. */
export function encode_structured_value(
  value: unknown,
  limits: Value_Limits = {},
): Uint8Array {
  const budget = checked_limits(limits)
  const model = to_value(value, budget)
  const output: number[] = []
  const tasks: [Structured_Value, number][] = [[model, 0]]
  let item_count = 0
  while (tasks.length > 0) {
    const task = tasks.pop()!
    const current = task[0]
    const depth = task[1]
    item_count += 1
    if (item_count > budget.max_items) resource("items", budget.max_items, item_count)
    if (current instanceof Undefined_Value) append(output, [0xf7], budget)
    else if (current === null) append(output, [0xf6], budget)
    else if (typeof current === "boolean") append(output, [current ? 0xf5 : 0xf4], budget)
    else if (current instanceof Integer_Value) encode_integer(current.value, output, budget)
    else if (current instanceof Float_Value) {
      const ai = current.width === 16 ? 25 : current.width === 32 ? 26 : 27
      append(output, [(7 << 5) | ai], budget)
      append(output, bigint_bytes(current.raw_bits, current.width / 8), budget)
    } else if (current instanceof ByteString_Value) {
      append_head(2, BigInt(current.value.length), output, budget)
      append(output, [...current.value], budget)
    } else if (current instanceof TextString_Value) {
      const bytes = [...TEXT_ENCODER.encode(current.value)]
      append_head(3, BigInt(bytes.length), output, budget)
      append(output, bytes, budget)
    } else if (current instanceof Array_Value) {
      if (depth >= budget.max_depth) resource("depth", budget.max_depth, depth + 1)
      append_head(4, BigInt(current.values.length), output, budget)
      if (item_count + current.values.length > budget.max_items) {
        resource("items", budget.max_items, item_count + current.values.length)
      }
      for (let index = current.values.length - 1; index >= 0; index -= 1) {
        tasks.push([current.values[index]!, depth + 1])
      }
    } else if (current instanceof Map_Value) {
      if (depth >= budget.max_depth) resource("depth", budget.max_depth, depth + 1)
      append_head(5, BigInt(current.entries.length), output, budget)
      if (item_count + 2 * current.entries.length > budget.max_items) {
        resource("items", budget.max_items, item_count + 2 * current.entries.length)
      }
      for (let index = current.entries.length - 1; index >= 0; index -= 1) {
        const [key, child] = current.entries[index]!
        tasks.push([child, depth + 1])
        tasks.push([key, depth + 1])
      }
    }
  }
  return Uint8Array.from(output)
}

function checked_limits(limits: Value_Limits): Required<Value_Limits> {
  const budget = { ...DEFAULT_VALUE_LIMITS, ...limits }
  for (const [name, value] of Object.entries(budget)) {
    if (!Number.isSafeInteger(value) || value <= 0) {
      throw new Structured_Value_Error(`${name} must be a positive safe integer`, "resource_limit")
    }
  }
  if (budget.max_depth > MAX_ALLOWED_VALUE_DEPTH) {
    resource("depth", MAX_ALLOWED_VALUE_DEPTH, budget.max_depth)
  }
  return budget
}

function resource(name: string, limit: number, actual: number): never {
  throw new Structured_Value_Error(
    `${name} limit ${limit} exceeded by ${actual}`,
    "resource_limit",
  )
}

function append(output: number[], bytes: readonly number[], budget: Required<Value_Limits>): void {
  if (output.length + bytes.length > budget.max_bytes) {
    resource("bytes", budget.max_bytes, output.length + bytes.length)
  }
  const chunk_size = 8_192
  for (let offset = 0; offset < bytes.length; offset += chunk_size) {
    output.push(...bytes.slice(offset, offset + chunk_size))
  }
}

function append_head(
  major: number,
  length: bigint,
  output: number[],
  budget: Required<Value_Limits>,
): void {
  if (length < 24n) append(output, [(major << 5) | Number(length)], budget)
  else if (length <= 0xffn) append(output, [(major << 5) | 24, Number(length)], budget)
  else if (length <= 0xffffn) append(output, [(major << 5) | 25, Number(length >> 8n), Number(length & 0xffn)], budget)
  else if (length <= 0xffff_ffffn) append(output, [(major << 5) | 26, ...bigint_bytes(length, 4)], budget)
  else if (length <= 0xffff_ffff_ffff_ffffn) append(output, [(major << 5) | 27, ...bigint_bytes(length, 8)], budget)
  else resource("bytes", budget.max_bytes, Number.MAX_SAFE_INTEGER)
}

function encode_integer(value: bigint, output: number[], budget: Required<Value_Limits>): void {
  const negative = value < 0n
  const model_magnitude = negative ? -value : value
  const model_magnitude_length =
    model_magnitude === 0n ? 0 : Math.ceil(model_magnitude.toString(2).length / 8)
  // The budget covers the mathematical magnitude, not the transformed
  // `-1-n` argument used by CBOR major type 1. A negative power of two can
  // therefore have one more model byte than its native CBOR argument.
  if (model_magnitude_length > budget.max_integer_bytes) {
    resource("integer bytes", budget.max_integer_bytes, model_magnitude_length)
  }
  const transformed = negative ? -value - 1n : value
  const magnitude_length =
    transformed === 0n ? 0 : Math.ceil(transformed.toString(2).length / 8)
  if (transformed <= 0xffff_ffff_ffff_ffffn) {
    append_head(negative ? 1 : 0, transformed, output, budget)
    return
  }
  const magnitude = bigint_bytes(transformed, magnitude_length)
  append_head(6, negative ? 3n : 2n, output, budget)
  append_head(2, BigInt(magnitude.length), output, budget)
  append(output, magnitude, budget)
}

function bigint_bytes(value: bigint, width: number): number[] {
  const bytes = new Array<number>(width)
  let remaining = value
  for (let index = width - 1; index >= 0; index -= 1) {
    bytes[index] = Number(remaining & 0xffn)
    remaining >>= 8n
  }
  return bytes
}

/** Decodes exactly one structured-value payload to its lossless model. */
export function decode_structured_value(
  input: Uint8Array,
  limits: Value_Limits = {},
): Structured_Value {
  if (!(input instanceof Uint8Array)) {
    throw new Structured_Value_Error("structured value must be a Uint8Array")
  }
  const budget = checked_limits(limits)
  if (input.length > budget.max_bytes) resource("bytes", budget.max_bytes, input.length)
  if (input.length === 0) throw new Structured_Value_Error("value is truncated", "truncated")
  let cursor = 0
  let root: Structured_Value | typeof MISSING_VALUE = MISSING_VALUE
  let item_count = 0
  type Frame =
    | { kind: "array"; remaining: number; values: Structured_Value[] }
    | {
        kind: "map"
        remaining: number
        entries: [Structured_Value, Structured_Value][]
        key_identities: Set<string>
        pending?: Structured_Value
      }
  const frames: Frame[] = []

  const accept = (value: Structured_Value): void => {
    let current = value
    while (true) {
      const frame = frames.at(-1)
      if (frame === undefined) {
        root = current
        return
      }
      if (frame.kind === "array") frame.values.push(current)
      else if (frame.pending === undefined) {
        validate_map_key(current, frame.entries.length, frame.key_identities)
        frame.pending = current
      } else {
        frame.entries.push([frame.pending, current])
        delete frame.pending
      }
      frame.remaining -= 1
      if (frame.remaining !== 0) return
      frames.pop()
      current =
        frame.kind === "array"
          ? new Array_Value(frame.values)
          : new Map_Value(frame.entries)
    }
  }

  while (root === MISSING_VALUE) {
    item_count += 1
    if (item_count > budget.max_items) resource("items", budget.max_items, item_count)
    const head = read_head(input, cursor)
    cursor = head.next
    if (head.major === 0 || head.major === 1) {
      const number = head.value
      const magnitude = head.major === 0 ? number : number + 1n
      const magnitude_bytes =
        magnitude === 0n ? 0 : Math.ceil(magnitude.toString(2).length / 8)
      if (magnitude_bytes > budget.max_integer_bytes) {
        resource("integer bytes", budget.max_integer_bytes, magnitude_bytes)
      }
      const integer = new Integer_Value(head.major === 0 ? number : -number - 1n)
      accept(integer)
    } else if (head.major === 2 || head.major === 3) {
      const length = safe_length(head.value, budget)
      if (cursor + length > input.length) throw new Structured_Value_Error("value is truncated", "truncated")
      const bytes = input.slice(cursor, cursor + length)
      cursor += length
      if (head.major === 2) accept(new ByteString_Value(bytes))
      else {
        let text: string
        try {
          text = TEXT_DECODER.decode(bytes)
        } catch (error) {
          throw new Structured_Value_Error("text is not valid UTF-8", "invalid_utf8")
        }
        accept(new TextString_Value(text))
      }
    } else if (head.major === 4) {
      const length = safe_length(head.value, budget)
      if (length > budget.max_items) resource("items", budget.max_items, length)
      if (frames.length >= budget.max_depth) resource("depth", budget.max_depth, frames.length + 1)
      if (length === 0) accept(new Array_Value([]))
      else {
        frames.push({ kind: "array", remaining: length, values: [] })
      }
    } else if (head.major === 5) {
      const length = safe_length(head.value, budget)
      if (length * 2 > budget.max_items) resource("items", budget.max_items, length * 2)
      if (frames.length >= budget.max_depth) resource("depth", budget.max_depth, frames.length + 1)
      if (length === 0) accept(new Map_Value([]))
      else {
        frames.push({
          kind: "map",
          remaining: length * 2,
          entries: [],
          key_identities: new Set<string>(),
        })
      }
    } else if (head.major === 6) {
      if (head.value !== 2n && head.value !== 3n) {
        throw new Structured_Value_Error("unsupported CBOR tag", "unsupported_type")
      }
      const wrapped = read_head(input, cursor)
      cursor = wrapped.next
      if (wrapped.major !== 2) throw new Structured_Value_Error("bignum tag must wrap bytes", "invalid_integer")
      const length = safe_length(wrapped.value, budget)
      if (length === 0) throw new Structured_Value_Error("bignum magnitude must not be empty", "invalid_integer")
      if (length > budget.max_integer_bytes) resource("integer bytes", budget.max_integer_bytes, length)
      if (cursor + length > input.length) throw new Structured_Value_Error("bignum magnitude is truncated", "truncated")
      const magnitude = input.slice(cursor, cursor + length)
      cursor += length
      if (magnitude[0] === 0) throw new Structured_Value_Error("bignum magnitude is not minimal", "invalid_integer")
      if (
        head.value === 3n &&
        magnitude.every((byte) => byte === 0xff) &&
        length === budget.max_integer_bytes
      ) {
        resource("integer bytes", budget.max_integer_bytes, length + 1)
      }
      let number = 0n
      for (const byte of magnitude) number = (number << 8n) | BigInt(byte)
      accept(new Integer_Value(head.value === 2n ? number : -number - 1n))
    } else if (head.major === 7) {
      if (head.ai === 20) accept(false)
      else if (head.ai === 21) accept(true)
      else if (head.ai === 22) accept(null)
      else if (head.ai === 23) accept(UNDEFINED_VALUE)
      else if (head.ai === 25) accept(new Float_Value(16, head.value))
      else if (head.ai === 26) accept(new Float_Value(32, head.value))
      else if (head.ai === 27) accept(new Float_Value(64, head.value))
      else throw new Structured_Value_Error("unsupported CBOR simple value", "unsupported_type")
    } else {
      throw new Structured_Value_Error("unsupported CBOR major type", "unsupported_type")
    }
  }
  if (frames.length !== 0) throw new Structured_Value_Error("value is truncated", "truncated")
  if (cursor !== input.length) throw new Structured_Value_Error("trailing CBOR bytes", "trailing_bytes")
  return root as Structured_Value
}

interface Cbor_Head {
  readonly major: number
  readonly ai: number
  readonly value: bigint
  readonly next: number
}

function read_head(input: Uint8Array, cursor: number): Cbor_Head {
  if (cursor >= input.length) throw new Structured_Value_Error("value is truncated", "truncated")
  const first = input[cursor]!
  cursor += 1
  const major = first >> 5
  const ai = first & 0x1f
  if (ai < 24) return { major, ai, value: BigInt(ai), next: cursor }
  if (ai === 31) throw new Structured_Value_Error("indefinite-length item", "invalid_encoding")
  const width = ai === 24 ? 1 : ai === 25 ? 2 : ai === 26 ? 4 : ai === 27 ? 8 : 0
  if (width === 0) {
    throw new Structured_Value_Error(
      "invalid CBOR additional information",
      "invalid_encoding",
    )
  }
  if (cursor + width > input.length) {
    throw new Structured_Value_Error("CBOR head is truncated", "truncated")
  }
  let value = 0n
  for (const byte of input.slice(cursor, cursor + width)) value = (value << 8n) | BigInt(byte)
  return { major, ai, value, next: cursor + width }
}

function safe_length(value: bigint, budget: Required<Value_Limits>): number {
  if (value > BigInt(Number.MAX_SAFE_INTEGER)) resource("bytes", budget.max_bytes, Number.MAX_SAFE_INTEGER)
  const length = Number(value)
  if (length > budget.max_bytes) resource("bytes", budget.max_bytes, length)
  return length
}

/**
 * Projects one lossless value into ordinary JavaScript values.
 *
 * JavaScript `undefined` is safe here because client lookups wrap every found
 * value in `FoundResult`; a missing lookup uses the separate `MISSING` result.
 * Float width and raw bits are reduced to JavaScript's observable `number`
 * value. Callers that need those distinctions must keep the lossless model
 * returned by `decode_structured_value`.
 *
 * @param value - Lossless structured value to project.
 * @param options - Optional checked integer convenience conversion.
 * @returns A native JavaScript value.
 * @throws {Structured_Value_Error} If a map cannot be represented without
 * semantic loss.
 */
export function to_native(
  value: Structured_Value,
  options: { readonly safe_integer?: boolean } = {},
): unknown {
  if (value instanceof Undefined_Value) return undefined
  if (value === null || typeof value === "boolean") return value
  if (value instanceof Integer_Value) {
    if (options.safe_integer && (value.value < BigInt(Number.MIN_SAFE_INTEGER) || value.value > BigInt(Number.MAX_SAFE_INTEGER))) {
      throw new Structured_Value_Error("integer is outside JavaScript safe-integer range")
    }
    return options.safe_integer ? Number(value.value) : value.value
  }
  if (value instanceof Float_Value) return float_to_native(value)
  if (value instanceof ByteString_Value) return value.value.slice()
  if (value instanceof TextString_Value) return value.value
  if (value instanceof Array_Value) return value.values.map((child): unknown => to_native(child, options))
  if (value instanceof Map_Value) return project_native_map(value, options)
  throw new Structured_Value_Error("unsupported model value")
}

function float_to_native(value: Float_Value): number {
  if (value.width === 16) {
    const sign = (value.raw_bits >> 15n) === 0n ? 1 : -1
    const exponent = Number((value.raw_bits >> 10n) & 0x1fn)
    const fraction = Number(value.raw_bits & 0x3ffn)
    if (exponent === 0) return sign * fraction * 2 ** -24
    if (exponent === 0x1f) return fraction === 0 ? sign * Infinity : Number.NaN
    return sign * (1 + fraction / 1024) * 2 ** (exponent - 15)
  }
  if (value.width === 32) {
    const buffer = new ArrayBuffer(4)
    const view = new DataView(buffer)
    view.setUint32(0, Number(value.raw_bits), false)
    return view.getFloat32(0, false)
  }
  const buffer = new ArrayBuffer(8)
  const view = new DataView(buffer)
  view.setBigUint64(0, value.raw_bits, false)
  return view.getFloat64(0, false)
}

function project_native_map(
  value: Map_Value,
  options: { readonly safe_integer?: boolean },
): unknown {
  const result = new Map<unknown, unknown>()
  const projected_keys: unknown[] = []
  const text_entries: [string, unknown][] = []
  let all_text_keys = true
  for (const [key, child] of value.entries) {
    const native_key = to_native(key, options)
    if (
      projected_keys.some((previous): boolean =>
        native_map_key_equal(previous, native_key),
      )
    ) {
      throw new Structured_Value_Error(
        "map keys cannot be represented by a JavaScript Map without loss",
        "conversion",
      )
    }
    projected_keys.push(native_key)
    const native_value = to_native(child, options)
    result.set(native_key, native_value)
    if (typeof native_key === "string") {
      text_entries.push([native_key, native_value])
    } else {
      all_text_keys = false
    }
  }
  if (!all_text_keys) return result

  const plain_object = Object.create(null) as Record<string, unknown>
  for (const [key, child] of text_entries) {
    Object.defineProperty(plain_object, key, {
      configurable: true,
      enumerable: true,
      writable: true,
      value: child,
    })
  }
  const expected_keys = text_entries.map(([key]): string => key)
  if (
    Object.keys(plain_object).every(
      (key, index): boolean => key === expected_keys[index],
    )
  ) {
    return plain_object
  }
  return result
}

/**
 * JavaScript Map uses SameValueZero for key identity.  Check that projection
 * does not collapse distinct model keys before mutating the native map.
 */
function native_map_key_equal(left: unknown, right: unknown): boolean {
  if (typeof left === "number" && typeof right === "number") {
    return left === right || (Number.isNaN(left) && Number.isNaN(right))
  }
  return left === right
}

/**
 * Decodes one payload and applies the native projection.
 *
 * @param input - One complete StructuredValue-CBOR-v1 payload.
 * @param options - Optional checked integer convenience conversion.
 * @returns A native JavaScript value.
 * @throws {Structured_Value_Error} If decoding fails or a map projection
 * would collapse distinct keys.
 */
export function decode_native_value(
  input: Uint8Array,
  options: { readonly safe_integer?: boolean } = {},
): unknown {
  return to_native(decode_structured_value(input), options)
}

/**
 * Projects a text-keyed lossless map to a null-prototype object.
 *
 * The projection uses the same native rules as `to_native` and rejects
 * text-keyed maps whose JavaScript object property order would differ from
 * the model entry order.
 *
 * @param value - Lossless map with text keys.
 * @returns A null-prototype object containing the map entries.
 * @throws {Structured_Value_Error} If a key or value cannot be represented
 * without semantic loss.
 */
export function to_plain_object(
  value: Map_Value,
  options: { readonly safe_integer?: boolean } = {},
): Record<string, unknown> {
  const result = Object.create(null) as Record<string, unknown>
  const expected_keys: string[] = []
  for (const [key, child] of value.entries) {
    if (!(key instanceof TextString_Value)) {
      throw new Structured_Value_Error("map contains a non-text key")
    }
    expected_keys.push(key.value)
    Object.defineProperty(result, key.value, {
      configurable: true,
      enumerable: true,
      writable: true,
      value: to_native(child, options),
    })
  }
  if (
    !Object.keys(result).every(
      (key, index): boolean => key === expected_keys[index],
    )
  ) {
    throw new Structured_Value_Error(
      "map keys cannot be represented by a JavaScript object without changing entry order",
    )
  }
  return result
}

function assert_valid_unicode_string(value: string): void {
  for (let index = 0; index < value.length; index += 1) {
    const code_unit = value.charCodeAt(index)
    if (code_unit >= 0xd800 && code_unit <= 0xdbff) {
      const next_code_unit = value.charCodeAt(index + 1)
      if (
        Number.isNaN(next_code_unit) ||
        next_code_unit < 0xdc00 ||
        next_code_unit > 0xdfff
      ) {
        throw new Structured_Value_Error("text contains unpaired surrogates", "invalid_utf8")
      }
      index += 1
    } else if (code_unit >= 0xdc00 && code_unit <= 0xdfff) {
      throw new Structured_Value_Error("text contains unpaired surrogates", "invalid_utf8")
    }
  }
}

// Idiomatic TypeScript spellings. The underscored exports remain available
// for compatibility with the first package release.
export {
  Array_Value as ArrayValue,
  ByteString_Value as ByteStringValue,
  Float_Value as FloatValue,
  Integer_Value as IntegerValue,
  Map_Value as MapValue,
  Structured_Value_Error as StructuredValueError,
  TextString_Value as TextStringValue,
  Undefined_Value as UndefinedValue,
  decode_native_value as decodeNativeValue,
  decode_structured_value as decodeStructuredValue,
  encode_structured_value as encodeStructuredValue,
  model_equal as modelEqual,
  to_native as toNative,
  to_plain_object as toPlainObject,
  to_value as toValue,
}

export type {
  Structured_Value as StructuredValue,
  Structured_Value_Error_Kind as StructuredValueErrorKind,
  Value_Limits as ValueLimits,
}
