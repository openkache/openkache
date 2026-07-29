/**
 * Runtime-neutral codec registry for cross-language values.
 */

const ENCODING_PATTERN = /^[a-z][a-z0-9.-]{0,63}$/
const JSON_ENCODING = "json"
const TEXT_ENCODER = new TextEncoder()
const TEXT_DECODER = new TextDecoder("utf-8", { fatal: true })

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
   * @returns A regular JavaScript object.
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
    const codecs_by_encoding = new Map<string, Value_Codec>()
    for (const codec of codecs) {
      validate_encoding(codec.encoding)
      if (codec.encoding === JSON_ENCODING) {
        throw new Error(`encoding ${JSON_ENCODING} is reserved for the built-in codec`)
      }
      if (codecs_by_encoding.has(codec.encoding)) {
        throw new Error(`duplicate value codec encoding ${codec.encoding}`)
      }
      codecs_by_encoding.set(codec.encoding, codec)
    }
    this.#codecs = codecs.slice()
    this.#codecs_by_encoding = codecs_by_encoding
  }

  /**
   * Encodes a regular object using one custom codec or JSON fallback.
   *
   * @param value - Regular JavaScript object supplied to `set`.
   * @returns Codec metadata and payload for the Rust value-envelope encoder.
   * @throws {Error} When codec selection is ambiguous or encoding fails.
   */
  encode(value: object): Value_Envelope {
    if (!is_regular_object(value)) {
      throw new Error("value must be a regular JavaScript object")
    }
    const matching_codecs = this.#codecs.filter((codec): boolean =>
      codec.can_encode(value),
    )
    if (matching_codecs.length > 1) {
      throw new Error(
        `value matches multiple codecs: ${matching_codecs.map((codec): string => codec.encoding).join(", ")}`,
      )
    }
    const codec = matching_codecs[0]
    if (codec === undefined) {
      return {
        encoding: JSON_ENCODING,
        type_name: "",
        payload: encode_json_object(value),
      }
    }
    const encoded = codec.encode(value)
    if (!is_regular_object(encoded)) {
      throw new Error(`codec ${codec.encoding} returned an invalid encoded value`)
    }
    if (typeof encoded.type_name !== "string") {
      throw new Error(`codec ${codec.encoding} returned a non-string type name`)
    }
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
   * @returns A regular JavaScript object.
   * @throws {Error} When the metadata, selected codec, or payload is invalid.
   */
  decode(envelope: Value_Envelope): object {
    validate_envelope(envelope)
    if (envelope.encoding === JSON_ENCODING) {
      if (envelope.type_name.length !== 0) {
        throw new Error("JSON value envelope must not contain a type name")
      }
      return decode_json_object(envelope.payload)
    }
    const codec = this.#codecs_by_encoding.get(envelope.encoding)
    if (codec === undefined) {
      throw new Error(`no value codec is registered for ${envelope.encoding}`)
    }
    const value = codec.decode(envelope.type_name, envelope.payload)
    if (!is_regular_object(value)) {
      throw new Error(`codec ${codec.encoding} decoded a non-object value`)
    }
    return value
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
  if (!(envelope.payload instanceof Uint8Array)) {
    throw new Error("decoded value envelope has a non-binary payload")
  }
}

function encode_json_object(value: object): Uint8Array {
  validate_json_value(value, "$", new WeakSet())
  const text = JSON.stringify(value)
  if (text === undefined) {
    throw new Error("value cannot be represented as JSON")
  }
  return TEXT_ENCODER.encode(text)
}

function decode_json_object(bytes: Uint8Array): object {
  const value: unknown = JSON.parse(TEXT_DECODER.decode(bytes))
  if (!is_regular_object(value)) {
    throw new Error("decoded JSON value is not a regular JavaScript object")
  }
  return value
}

function validate_json_value(
  value: unknown,
  path: string,
  ancestors: WeakSet<object>,
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
    validate_json_container(value, path, ancestors, (): void => {
      for (let index = 0; index < value.length; index += 1) {
        if (!(index in value)) {
          throw new Error(`${path}[${index}] must not be a sparse array entry`)
        }
        validate_json_value(value[index], `${path}[${index}]`, ancestors)
      }
    })
    return
  }
  if (is_regular_object(value)) {
    validate_json_container(value, path, ancestors, (): void => {
      for (const [key, property_value] of Object.entries(value)) {
        if (property_value === undefined) continue
        validate_json_value(property_value, property_path(path, key), ancestors)
      }
      for (const symbol of Object.getOwnPropertySymbols(value)) {
        if (Object.prototype.propertyIsEnumerable.call(value, symbol)) {
          throw new Error(`${path} must not contain enumerable symbol properties`)
        }
      }
    })
    return
  }
  throw new Error(`${path} contains unsupported JSON value ${describe_value(value)}`)
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
  if (!ENCODING_PATTERN.test(encoding)) {
    throw new Error(`invalid value encoding ${JSON.stringify(encoding)}`)
  }
}

function is_regular_object(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
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
