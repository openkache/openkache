/**
 * Runtime-neutral codec registry for cross-language values.
 */

import {
  SMITHY_VALUE_ENVELOPE_JSON_ENCODING,
  SMITHY_VALUE_ENVELOPE_MAX_ENCODING_BYTES,
  SMITHY_VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES,
} from "./generated_local/smithy-value-envelope.js"

// Local early validation for the legacy metadata envelope; the Rust core remains authoritative.
const VALUE_ENVELOPE_ENCODING_PATTERN = new RegExp(
  `^[a-z][a-z0-9.-]{0,${SMITHY_VALUE_ENVELOPE_MAX_ENCODING_BYTES - 1}}$`,
)
const JSON_ENCODING = SMITHY_VALUE_ENVELOPE_JSON_ENCODING
const MAX_JSON_DEPTH = 128
const TEXT_ENCODER = new TextEncoder()
const TEXT_DECODER = new TextDecoder("utf-8", { fatal: true })

/**
 * JSON value accepted by the legacy envelope adapter and by the core-owned
 * canonical JSON API.
 */
export type Json_Value =
  | null
  | boolean
  | number
  | string
  | readonly Json_Value[]
  | Json_Object

/**
 * JSON object with string keys. `undefined` properties are omitted for
 * backwards compatibility with the original TypeScript envelope API.
 */
export interface Json_Object {
  readonly [key: string]: Json_Value | undefined
}

/**
 * Validates a value against the shared JSON value model.
 *
 * @param value - Candidate native value.
 * @throws {Error} When the value contains unsupported, cyclic, sparse, or
 * non-finite data.
 */
export function assert_json_value(value: unknown): asserts value is Json_Value {
  validate_json_value(value, "$", new WeakSet())
}

/**
 * Encoded payload and logical type returned by a custom value codec.
 */
export interface Encoded_Value {
  /** Cross-language type identifier, such as `acme.profile.v1`. */
  readonly type_name: string
  /** Codec-specific bytes stored inside the OpenKache value envelope. */
  readonly payload: Uint8Array
}

/**
 * Codec metadata and payload passed to the Rust value-envelope implementation.
 */
export interface Value_Envelope {
  /** Stable codec identifier, such as `json`, `protobuf`, or `flatbuffers`. */
  readonly encoding: string
  /** Codec-defined logical type stored with the payload. */
  readonly type_name: string
  /** Exact codec-specific payload bytes. */
  readonly payload: Uint8Array
}

/**
 * Pluggable cross-language object codec.
 *
 * A Protobuf or FlatBuffers integration can own a schema registry internally.
 * The stored envelope carries `encoding` and `type_name`, so cache operations
 * do not need positional schema arguments.
 */
export interface Value_Codec {
  /** Stable cross-language encoding identifier, such as `protobuf`. */
  readonly encoding: string

  /**
   * Reports whether this codec owns a value.
   *
   * @param value - Regular JavaScript object supplied to `set`.
   * @returns Whether `encode` should serialize this value.
   */
  can_encode(value: object): boolean

  /**
   * Serializes an owned value.
   *
   * @param value - Value accepted by `can_encode`.
   * @returns Logical type metadata and encoded payload bytes.
   * @throws {Error} When the value cannot be encoded.
   */
  encode(value: object): Encoded_Value

  /**
   * Deserializes bytes selected by the stored envelope.
   *
   * @param type_name - Cross-language logical type stored with the payload.
   * @param payload - Exact codec-specific payload bytes.
   * @returns A regular JavaScript object returned by a custom codec.
   * @throws {Error} When the type is unknown or the payload is invalid.
   */
  decode(type_name: string, payload: Uint8Array): object
}

/**
 * Selects codecs for writes and routes stored envelopes for reads.
 */
export class Value_Codec_Registry {
  readonly #codecs: readonly Value_Codec[]
  readonly #codecs_by_encoding: ReadonlyMap<string, Value_Codec>

  /**
   * Creates a registry with built-in JSON fallback.
   *
   * @param codecs - Optional Protobuf, FlatBuffers, or application codecs.
   * @throws {Error} When encoding identifiers are invalid or duplicated.
   */
  constructor(codecs: readonly Value_Codec[] = []) {
    if (!Array.isArray(codecs)) {
      throw new Error("value codecs must be an array")
    }
    const codecs_by_encoding = new Map<string, Value_Codec>()
    for (const codec of codecs) {
      if (!is_regular_object(codec)) {
        throw new Error("value codec must be a regular object")
      }
      const valid_codec = codec as unknown as Value_Codec
      if (
        typeof valid_codec.encoding !== "string" ||
        typeof valid_codec.can_encode !== "function" ||
        typeof valid_codec.encode !== "function" ||
        typeof valid_codec.decode !== "function"
      ) {
        throw new Error(
          "value codec must define an encoding, can_encode, encode, and decode",
        )
      }
      validate_encoding(valid_codec.encoding)
      if (valid_codec.encoding === JSON_ENCODING) {
        throw new Error(`encoding ${JSON_ENCODING} is reserved for the built-in codec`)
      }
      if (codecs_by_encoding.has(valid_codec.encoding)) {
        throw new Error(`duplicate value codec encoding ${valid_codec.encoding}`)
      }
      codecs_by_encoding.set(valid_codec.encoding, valid_codec)
    }
    this.#codecs = codecs.slice()
    this.#codecs_by_encoding = codecs_by_encoding
  }

  /**
   * Encodes a JSON value using one custom object codec or the legacy JSON
   * envelope fallback.
   *
   * @param value - JSON value supplied to `set`.
   * @returns Codec metadata and payload for the Rust value-envelope encoder.
   * @throws {Error} When codec selection is ambiguous or encoding fails.
   */
  encode(value: unknown): Value_Envelope {
    const matching_codecs = is_object_value(value)
      ? this.#codecs.filter((codec): boolean => codec.can_encode(value))
      : []
    if (matching_codecs.length > 1) {
      throw new Error(
        `value matches multiple codecs: ${matching_codecs.map((codec): string => codec.encoding).join(", ")}`,
      )
    }
    const codec = matching_codecs[0]
    if (codec === undefined) {
      assert_legacy_json_value(value)
      return {
        encoding: JSON_ENCODING,
        type_name: "",
        payload: encode_json_value(value),
      }
    }
    const encoded = codec.encode(value as object)
    if (!is_regular_object(encoded)) {
      throw new Error(`codec ${codec.encoding} returned an invalid encoded value`)
    }
    if (typeof encoded.type_name !== "string") {
      throw new Error(`codec ${codec.encoding} returned a non-string type name`)
    }
    validate_type_name(encoded.type_name)
    if (!(encoded.payload instanceof Uint8Array)) {
      throw new Error(`codec ${codec.encoding} returned a non-binary payload`)
    }
    return {
      encoding: codec.encoding,
      type_name: encoded.type_name,
      payload: encoded.payload.slice(),
    }
  }

  /**
   * Decodes value-envelope components through the registered codec.
   *
   * @param envelope - Metadata and payload decoded by the Rust value-envelope implementation.
   * @returns A JSON value or a regular object returned by a custom codec.
   * @throws {Error} When the metadata, selected codec, or payload is invalid.
   */
  decode(envelope: Value_Envelope): Json_Value {
    validate_envelope(envelope)
    if (envelope.encoding === JSON_ENCODING) {
      if (envelope.type_name.length !== 0) {
        throw new Error("JSON value envelope must not contain a type name")
      }
      return decode_json_value(envelope.payload)
    }
    const codec = this.#codecs_by_encoding.get(envelope.encoding)
    if (codec === undefined) {
      throw new Error(`no value codec is registered for ${envelope.encoding}`)
    }
    const value = codec.decode(envelope.type_name, envelope.payload)
    if (!is_regular_object(value)) {
      throw new Error(`codec ${codec.encoding} decoded a non-object value`)
    }
    return value as Json_Object
  }
}

function validate_envelope(envelope: Value_Envelope): void {
  if (!is_regular_object(envelope)) {
    throw new Error("decoded value envelope is not an object")
  }
  if (typeof envelope.encoding !== "string") {
    throw new Error("decoded value envelope has a non-string encoding")
  }
  validate_encoding(envelope.encoding)
  if (typeof envelope.type_name !== "string") {
    throw new Error("decoded value envelope has a non-string type name")
  }
  validate_type_name(envelope.type_name)
  if (!(envelope.payload instanceof Uint8Array)) {
    throw new Error("decoded value envelope has a non-binary payload")
  }
}

function encode_json_value(value: Json_Value): Uint8Array {
  validate_json_value(value, "$", new WeakSet(), true)
  const text = JSON.stringify(value)
  if (text === undefined) {
    throw new Error("value cannot be represented as JSON")
  }
  return TEXT_ENCODER.encode(text)
}

function decode_json_value(bytes: Uint8Array): Json_Value {
  const value: unknown = JSON.parse(TEXT_DECODER.decode(bytes))
  validate_json_value(value, "$", new WeakSet())
  return value as Json_Value
}

function validate_json_value(
  value: unknown,
  path: string,
  ancestors: WeakSet<object>,
  omit_undefined_properties = false,
  depth = 0,
): void {
  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "boolean"
  ) {
    return
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw new Error(`${path} must contain a finite JSON number`)
    }
    return
  }
  if (Array.isArray(value)) {
    if (depth >= MAX_JSON_DEPTH) {
      throw new Error(`${path} exceeds the maximum JSON depth of ${MAX_JSON_DEPTH}`)
    }
    validate_json_container(value, path, ancestors, (): void => {
      for (let index = 0; index < value.length; index += 1) {
        if (!(index in value)) {
          throw new Error(`${path}[${index}] must not be a sparse array entry`)
        }
        validate_json_value(value[index], `${path}[${index}]`, ancestors, false, depth + 1)
      }
      for (const key of Object.keys(value)) {
        if (
          !/^(?:0|[1-9][0-9]*)$/.test(key) ||
          Number(key) >= value.length
        ) {
          throw new Error(`${path} must not contain enumerable property ${JSON.stringify(key)}`)
        }
      }
      for (const symbol of Object.getOwnPropertySymbols(value)) {
        if (Object.prototype.propertyIsEnumerable.call(value, symbol)) {
          throw new Error(`${path} must not contain enumerable symbol properties`)
        }
      }
    })
    return
  }
  if (is_regular_object(value)) {
    if (depth >= MAX_JSON_DEPTH) {
      throw new Error(`${path} exceeds the maximum JSON depth of ${MAX_JSON_DEPTH}`)
    }
    validate_json_container(value, path, ancestors, (): void => {
      for (const [key, property_value] of Object.entries(value)) {
        if (property_value === undefined) {
          if (omit_undefined_properties) continue
          throw new Error(`${property_path(path, key)} contains unsupported JSON value undefined`)
        }
        validate_json_value(
          property_value,
          property_path(path, key),
          ancestors,
          omit_undefined_properties,
          depth + 1,
        )
      }
      for (const symbol of Object.getOwnPropertySymbols(value)) {
        if (Object.prototype.propertyIsEnumerable.call(value, symbol)) {
          throw new Error(`${path} must not contain enumerable symbol properties`)
        }
      }
    })
    return
  }
  if (typeof value === "undefined") {
    throw new Error(`${path} contains unsupported JSON value undefined`)
  }
  throw new Error(`${path} contains unsupported JSON value ${describe_value(value)}`)
}

function assert_legacy_json_value(value: unknown): asserts value is Json_Value {
  validate_json_value(value, "$", new WeakSet(), true)
}

function validate_json_container(
  value: object,
  path: string,
  ancestors: WeakSet<object>,
  validate_children: () => void,
): void {
  if (ancestors.has(value)) {
    throw new Error(`${path} contains a cyclic reference`)
  }
  ancestors.add(value)
  try {
    validate_children()
  } finally {
    ancestors.delete(value)
  }
}

function validate_encoding(encoding: string): void {
  if (!VALUE_ENVELOPE_ENCODING_PATTERN.test(encoding)) {
    throw new Error(`invalid value encoding ${JSON.stringify(encoding)}`)
  }
}

function validate_type_name(type_name: string): void {
  const byte_length = TEXT_ENCODER.encode(type_name).byteLength
  if (byte_length > SMITHY_VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES) {
    throw new Error(
      `value type name contains ${byte_length} bytes, maximum is ${SMITHY_VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES}`,
    )
  }
}

function is_regular_object(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

function is_object_value(value: unknown): value is object {
  return value !== null && typeof value === "object"
}

function property_path(path: string, key: string): string {
  return /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(key)
    ? `${path}.${key}`
    : `${path}[${JSON.stringify(key)}]`
}

function describe_value(value: unknown): string {
  if (typeof value === "bigint") return "bigint"
  if (typeof value === "undefined") return "undefined"
  if (typeof value === "function") return "function"
  if (typeof value === "symbol") return "symbol"
  return Object.prototype.toString.call(value)
}

// ---------------------------------------------------------------------------
// StructuredValue-CBOR-v1
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

/** Error raised by native conversion or StructuredValue-CBOR-v1 parsing. */
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
const MISSING_VALUE = Symbol("missing structured value root")

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
    this.values = values.map((value): Structured_Value => to_value(value))
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
    entries.forEach((entry, index): void => {
      if (!Array.isArray(entry) || entry.length !== 2) {
        throw new Structured_Value_Error(
          `map entry ${index} must be a two-item pair`,
        )
      }
      const key = to_value(entry[0])
      const value = to_value(entry[1])
      validate_map_key(key, index, converted)
      converted.push([key, value])
    })
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

/** Singleton helper for constructing the model's undefined value. */
export const UNDEFINED_VALUE = new Undefined_Value()

/** Converts a JavaScript native value into the lossless model. */
export function to_value(value: unknown): Structured_Value {
  return convert_value(value, new Set<object>())
}

function convert_value(value: unknown, ancestors: Set<object>): Structured_Value {
  if (
    value instanceof Undefined_Value ||
    value instanceof Integer_Value ||
    value instanceof Float_Value ||
    value instanceof ByteString_Value ||
    value instanceof TextString_Value ||
    value instanceof Array_Value ||
    value instanceof Map_Value
  ) {
    return value
  }
  if (typeof value === "undefined") return UNDEFINED_VALUE
  if (value === null || typeof value === "boolean") return value
  if (typeof value === "number") {
    const bits = new ArrayBuffer(8)
    new DataView(bits).setFloat64(0, value, false)
    return new Float_Value(64, new DataView(bits).getBigUint64(0, false))
  }
  if (typeof value === "bigint") return new Integer_Value(value)
  if (typeof value === "string") return new TextString_Value(value)
  if (value instanceof Uint8Array) return new ByteString_Value(value)
  if (Array.isArray(value)) {
    assert_acyclic(value, ancestors)
    try {
      const children: Structured_Value[] = []
      for (let index = 0; index < value.length; index += 1) {
        if (!(index in value)) {
          throw new Structured_Value_Error(
            `array entry ${index} is sparse`,
            "conversion",
          )
        }
        children.push(convert_value(value[index], ancestors))
      }
      return new Array_Value(children)
    } finally {
      ancestors.delete(value)
    }
  }
  if (value instanceof Map) {
    assert_acyclic(value, ancestors)
    try {
      return new Map_Value(
        [...value.entries()].map(
          ([key, child]): readonly [Structured_Value, Structured_Value] => [
            convert_value(key, ancestors),
            convert_value(child, ancestors),
          ],
        ),
      )
    } finally {
      ancestors.delete(value)
    }
  }
  if (is_regular_object(value)) {
    assert_acyclic(value, ancestors)
    try {
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
            new TextString_Value(key),
            convert_value(value[key], ancestors),
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
  entries: readonly (readonly [Structured_Value, Structured_Value])[],
): void {
  if (!is_scalar_key(key)) {
    throw new Structured_Value_Error(
      `map key at entry ${index} is not scalar`,
      "non_scalar_key",
    )
  }
  if (entries.some(([candidate]): boolean => model_equal(candidate, key))) {
    throw new Structured_Value_Error(
      `duplicate map key at entry ${index}`,
      "duplicate_key",
    )
  }
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

/** Encodes one complete native/model value as StructuredValue-CBOR-v1. */
export function encode_structured_value(
  value: unknown,
  limits: Value_Limits = {},
): Uint8Array {
  const budget = checked_limits(limits)
  const model = to_value(value)
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
  output.push(...bytes)
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
  const transformed = negative ? -value - 1n : value
  if (transformed <= 0xffff_ffff_ffff_ffffn) {
    append_head(negative ? 1 : 0, transformed, output, budget)
    return
  }
  const magnitude = bigint_bytes(transformed, Math.ceil(transformed.toString(2).length / 8))
  if (magnitude.length > budget.max_integer_bytes) {
    resource("integer bytes", budget.max_integer_bytes, magnitude.length)
  }
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

/** Decodes exactly one StructuredValue-CBOR-v1 value to its lossless model. */
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
    | { kind: "map"; remaining: number; entries: [Structured_Value, Structured_Value][]; pending?: Structured_Value }
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
        validate_map_key(current, frame.entries.length, frame.entries)
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
      if (length === 0) accept(new Array_Value([]))
      else {
        if (frames.length >= budget.max_depth) resource("depth", budget.max_depth, frames.length + 1)
        frames.push({ kind: "array", remaining: length, values: [] })
      }
    } else if (head.major === 5) {
      const length = safe_length(head.value, budget)
      if (length * 2 > budget.max_items) resource("items", budget.max_items, length * 2)
      if (length === 0) accept(new Map_Value([]))
      else {
        if (frames.length >= budget.max_depth) resource("depth", budget.max_depth, frames.length + 1)
        frames.push({ kind: "map", remaining: length * 2, entries: [] })
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
  if (width === 0 || cursor + width > input.length) {
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

/** Strict native projection: Integer -> bigint and Map -> Map. */
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
  if (value instanceof Float_Value) return decode_float(value)
  if (value instanceof ByteString_Value) return value.value.slice()
  if (value instanceof TextString_Value) return value.value
  if (value instanceof Array_Value) return value.values.map((child): unknown => to_native(child, options))
  if (value instanceof Map_Value) {
    const result = new Map<unknown, unknown>()
    const projected_keys: unknown[] = []
    for (const [key, child] of value.entries) {
      const native_key = to_native(key, options)
      if (projected_keys.some((previous): boolean => native_map_key_equal(previous, native_key))) {
        throw new Structured_Value_Error(
          "map keys cannot be represented by a JavaScript Map without loss",
          "conversion",
        )
      }
      projected_keys.push(native_key)
      result.set(native_key, to_native(child, options))
    }
    return result
  }
  throw new Structured_Value_Error("unsupported model value")
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

/** Decodes one payload and applies the strict native projection. */
export function decode_native_value(
  input: Uint8Array,
  options: { readonly safe_integer?: boolean } = {},
): unknown {
  return to_native(decode_structured_value(input), options)
}

/** Safely projects a text-keyed lossless map to a null-prototype object. */
export function to_plain_object(value: Map_Value): Record<string, unknown> {
  const result = Object.create(null) as Record<string, unknown>
  for (const [key, child] of value.entries) {
    if (!(key instanceof TextString_Value)) {
      throw new Structured_Value_Error("map contains a non-text key")
    }
    Object.defineProperty(result, key.value, {
      configurable: true,
      enumerable: true,
      writable: true,
      value: to_native(child),
    })
  }
  return result
}

function decode_float(value: Float_Value): number {
  if (value.width === 64) {
    const buffer = new ArrayBuffer(8)
    new DataView(buffer).setBigUint64(0, value.raw_bits, false)
    return new DataView(buffer).getFloat64(0, false)
  }
  if (value.width === 32) {
    const buffer = new ArrayBuffer(4)
    new DataView(buffer).setUint32(0, Number(value.raw_bits), false)
    return new DataView(buffer).getFloat32(0, false)
  }
  const bits = Number(value.raw_bits)
  const sign = (bits & 0x8000) === 0 ? 1 : -1
  const exponent = (bits >>> 10) & 0x1f
  const fraction = bits & 0x3ff
  if (exponent === 0) return sign * 2 ** -14 * (fraction / 2 ** 10)
  if (exponent === 0x1f) return fraction === 0 ? sign * Infinity : NaN
  return sign * 2 ** (exponent - 15) * (1 + fraction / 2 ** 10)
}

function assert_valid_unicode_string(value: string): void {
  for (let index = 0; index < value.length; index += 1) {
    const code_unit = value.charCodeAt(index)
    if (code_unit >= 0xd800 && code_unit <= 0xdbff) {
      const next_code_unit = value.charCodeAt(index + 1)
      if (next_code_unit < 0xdc00 || next_code_unit > 0xdfff) {
        throw new Structured_Value_Error("text contains unpaired surrogates", "invalid_utf8")
      }
      index += 1
    } else if (code_unit >= 0xdc00 && code_unit <= 0xdfff) {
      throw new Structured_Value_Error("text contains unpaired surrogates", "invalid_utf8")
    }
  }
}
