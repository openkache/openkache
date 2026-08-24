import {
  load_native_module,
  type Native_Client,
  type Native_Client_Options,
} from "./native-binding.js"
import {
  decode_structured_value,
  encode_structured_value,
  type Structured_Value,
} from "./value-codec.js"
import {
  SET_OUTCOME_CREATED,
  SET_OUTCOME_NOT_STORED,
  SET_OUTCOME_REPLACED,
  type Gate0_Set_Outcome,
} from "./gate0-contract.js"
import { SMITHY_MAX_CANONICAL_KEY_BYTES } from "./generated_local/smithy-api.js"

export type {
  Structured_Value,
  Structured_Value_Error_Kind,
  Value_Limits,
} from "./value-codec.js"
export {
  Array_Value,
  ByteString_Value,
  Float_Value,
  Integer_Value,
  Map_Value,
  Structured_Value_Error,
  TextString_Value,
  UNDEFINED_VALUE,
  Undefined_Value,
  decode_native_value,
  decode_structured_value,
  encode_structured_value,
  model_equal,
  to_native,
  to_plain_object,
  to_value,
} from "./value-codec.js"

const TEXT_ENCODER = new TextEncoder()
const INCOMPATIBLE_OUTCOME_PREFIX =
  "openkache:error:incompatible_server_outcome:"

/**
 * A Gate 0 mapped key: UTF-8 text, exact bytes, a safe integer number, or a
 * signed i64 bigint.
 */
export type Client_Key = string | Uint8Array | number | bigint

/** The only accepted Gate 0 connection shape. */
export interface Client_Options {
  /** Server endpoint, for example `127.0.0.1:4433`. */
  readonly address: string
}

/** Native values accepted by `set` after conversion to the lossless model. */
export type Native_Value =
  | undefined
  | null
  | boolean
  | bigint
  | number
  | string
  | Uint8Array
  | readonly Native_Value[]
  | ReadonlyMap<Native_Value, Native_Value>
  | { readonly [key: string]: Native_Value }

/** A tagged result for a lookup that may be absent. */
export type Get_Result<Value> = Missing_Result | Found_Result<Value>

/** Explicit missing lookup result; it is distinct from a stored `Undefined`. */
export class Missing_Result {
  readonly kind = "missing" as const
}

/** Explicit found lookup result, including a stored `Undefined` value. */
export class Found_Result<Value> {
  readonly kind = "found" as const

  constructor(readonly value: Value) {}
}

/** Stable singleton for callers that need to compare missing results by identity. */
export const MISSING = new Missing_Result()

/** Public set outcomes for unconditional Gate 0 writes. */
export type Set_Outcome = Gate0_Set_Outcome

/** Public delete outcomes for Gate 0 deletes. */
export type Delete_Outcome = "deleted" | "not_found"

/** Stable categories for failures surfaced by the maintained facade. */
export type OpenKache_Error_Kind =
  | "openkache_error"
  | "unknown_mutation"
  | "incompatible_server_outcome"

/** Error raised by validation, transport, server, or value conversion. */
export class OpenKache_Error extends Error {
  readonly kind: OpenKache_Error_Kind

  /**
   * Creates a stable client error.
   *
   * @param message - Human-readable failure description.
   * @param cause - Optional underlying failure.
   * @param kind - Stable error category.
   */
  constructor(
    message: string,
    cause?: unknown,
    kind: OpenKache_Error_Kind = "openkache_error",
  ) {
    super(message, cause === undefined ? undefined : { cause })
    this.name = "OpenKache_Error"
    this.kind = kind
  }
}

/** A mutation may have crossed admission but did not return a definitive result. */
export class OpenKache_Unknown_Mutation_Error extends OpenKache_Error {
  constructor(message: string, cause?: unknown) {
    super(message, cause, "unknown_mutation")
    this.name = "OpenKache_Unknown_Mutation_Error"
  }
}

interface Client_Lifecycle {
  closed: boolean
  close_promise?: Promise<void>
}

const CLIENT_FINALIZER = new FinalizationRegistry<Native_Client>(
  (native_client): void => {
    try {
      native_client.close_now()
    } catch {
      // Finalization is best effort and has no caller that can observe errors.
    }
  },
)

/**
 * Promise-based OpenKache Gate 0 client.
 *
 * The public cache surface is deliberately limited to `connect`, `get`, `set`,
 * `delete`, and `close`. The development profile uses TLS 1.3 with server
 * certificate verification disabled; this is development only — do not use
 * this trust profile in production.
 */
export class OpenKache_Client {
  readonly #native_client: Native_Client
  readonly #lifecycle: Client_Lifecycle

  private constructor(
    native_client: Native_Client,
    lifecycle: Client_Lifecycle,
  ) {
    this.#native_client = native_client
    this.#lifecycle = lifecycle
    CLIENT_FINALIZER.register(this, native_client, this)
  }

  /**
   * Opens a connection using the fixed Gate 0 development profile.
   *
   * The server certificate is deliberately not verified. TLS 1.3 still
   * encrypts traffic, but this profile has no active MITM protection and is
   * development only — do not use this trust profile in production.
   *
   * @param endpoint - Server endpoint string or the `{ address }` shape.
   * @returns A connected client.
   * @throws {OpenKache_Error} When the endpoint or native connection is invalid.
   */
  static async connect(
    endpoint: string | Client_Options,
  ): Promise<OpenKache_Client> {
    const address = parse_endpoint(endpoint)
    const native_options: Native_Client_Options = { address }
    try {
      const native_client = await load_native_module().connect(native_options)
      return new OpenKache_Client(native_client, { closed: false })
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Retrieves one lossless StructuredValue-CBOR-v1 value.
   *
   * `Missing_Result` is returned when no live item exists. A stored `Null` or
   * `Undefined_Value` is always wrapped in `Found_Result`, so JavaScript
   * `undefined` is never used as the missing marker.
   *
   * @param key - UTF-8 text, exact bytes, a safe integer number, or a
   * signed-i64 bigint key.
   * @returns A tagged missing/found result.
   * @throws {OpenKache_Error} When validation, transport, or decoding fails.
   */
  async get(key: Client_Key): Promise<Get_Result<Structured_Value>> {
    this.#assert_open()
    let payload: Uint8Array | null
    try {
      payload = await this.#native_client.get(owned_key_bytes(key))
    } catch (error) {
      throw as_openkache_error(error)
    }
    if (payload === null) return MISSING
    try {
      return new Found_Result(decode_structured_value(payload))
    } catch (error) {
      throw new OpenKache_Error(
        `structured value decoding failed: ${error_message(error)}`,
        error,
      )
    }
  }

  /**
   * Encodes and stores one lossless StructuredValue-CBOR-v1 value.
   *
   * The write is unconditional. The result is `created` when the item was
   * absent and `replaced` when a live item was overwritten.
   *
   * @param key - UTF-8 text, exact bytes, a safe integer number, or a
   * signed-i64 bigint key.
   * @param value - Native or lossless structured value.
   * @returns The created/replaced outcome.
   * @throws {OpenKache_Error} When validation, encoding, transport, or storage fails.
   */
  async set(
    key: Client_Key,
    value: Native_Value | Structured_Value,
  ): Promise<Set_Outcome> {
    this.#assert_open()
    let payload: Uint8Array
    try {
      payload = encode_structured_value(value)
    } catch (error) {
      throw new OpenKache_Error(
        `structured value encoding failed: ${error_message(error)}`,
        error,
      )
    }
    let outcome: string
    try {
      outcome = await this.#native_client.set(
        owned_key_bytes(key),
        payload,
      )
    } catch (error) {
      throw as_openkache_error(error)
    }
    switch (outcome) {
      case SET_OUTCOME_CREATED:
      case SET_OUTCOME_REPLACED:
        return outcome
      case SET_OUTCOME_NOT_STORED:
        throw new OpenKache_Error(
          "server returned unsupported conditional SET outcome not_stored",
          undefined,
          "incompatible_server_outcome",
        )
      default:
        throw new OpenKache_Error(
          `SET returned unexpected native outcome ${outcome}`,
        )
    }
  }

  /**
   * Deletes one mapped key.
   *
   * @param key - UTF-8 text, exact bytes, a safe integer number, or a
   * signed-i64 bigint key.
   * @returns `deleted` when an item existed, otherwise `not_found`.
   * @throws {OpenKache_Error} When validation, transport, or storage fails.
   */
  async delete(key: Client_Key): Promise<Delete_Outcome> {
    this.#assert_open()
    try {
      return (await this.#native_client.delete(owned_key_bytes(key)))
        ? "deleted"
        : "not_found"
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Closes the connection. Repeated calls complete successfully.
   *
   * @returns A promise resolved after native resource release.
   */
  close(): Promise<void> {
    this.#lifecycle.close_promise ??= (async (): Promise<void> => {
      try {
        await this.#native_client.close()
      } catch (error) {
        throw as_openkache_error(error)
      } finally {
        this.#lifecycle.closed = true
        CLIENT_FINALIZER.unregister(this)
      }
    })()
    return this.#lifecycle.close_promise
  }

  #assert_open(): void {
    if (this.#lifecycle.closed || this.#lifecycle.close_promise !== undefined) {
      throw new OpenKache_Error("client is closed")
    }
  }
}

function parse_endpoint(endpoint: string | Client_Options): string {
  if (typeof endpoint === "string") {
    if (endpoint.length === 0) {
      throw new OpenKache_Error("endpoint must be a non-empty string")
    }
    return endpoint
  }
  try {
    if (
      endpoint === null ||
      typeof endpoint !== "object" ||
      Array.isArray(endpoint) ||
      (Object.getPrototypeOf(endpoint) !== Object.prototype &&
        Object.getPrototypeOf(endpoint) !== null)
    ) {
      throw new OpenKache_Error("connect expects an endpoint string or { address }")
    }
    const keys = Reflect.ownKeys(endpoint)
    if (keys.length !== 1 || keys[0] !== "address") {
      throw new OpenKache_Error(
        "Gate 0 connect accepts only the address field; trust and certificate options are unsupported",
      )
    }
    const address = endpoint.address
    if (typeof address !== "string" || address.length === 0) {
      throw new OpenKache_Error("address must be a non-empty string")
    }
    return address
  } catch (error) {
    if (error instanceof OpenKache_Error) throw error
    throw new OpenKache_Error(
      "connect expects an endpoint string or { address }",
      error,
    )
  }
}

function owned_key_bytes(key: Client_Key): Uint8Array {
  if (typeof key === "string") {
    assert_valid_unicode_string(key)
    return encode_cbor_bytes_or_text(3, TEXT_ENCODER.encode(key))
  }
  if (key instanceof Uint8Array) {
    return encode_cbor_bytes_or_text(2, key)
  }
  if (typeof key === "number") {
    if (!Number.isSafeInteger(key) || Object.is(key, -0)) {
      throw new OpenKache_Error(
        "number keys must be finite safe integers and must not be negative zero",
      )
    }
    return encode_cbor_integer(BigInt(key))
  }
  if (typeof key === "bigint") {
    if (key < -(1n << 63n) || key > (1n << 63n) - 1n) {
      throw new OpenKache_Error("integer keys must fit the signed 64-bit range")
    }
    return encode_cbor_integer(key)
  }
  throw new OpenKache_Error(
    "key must be a UTF-8 string, Uint8Array, a safe integer number, or " +
      "signed-i64 bigint",
  )
}

function encode_cbor_bytes_or_text(
  major: 2 | 3,
  bytes: Uint8Array,
): Uint8Array {
  const header = encode_cbor_argument(major, bytes.byteLength)
  const total_length = header.byteLength + bytes.byteLength
  if (total_length > SMITHY_MAX_CANONICAL_KEY_BYTES) {
    throw new OpenKache_Error(
      `canonical key exceeds ${SMITHY_MAX_CANONICAL_KEY_BYTES} bytes`,
    )
  }
  const output = new Uint8Array(total_length)
  output.set(header)
  output.set(bytes, header.byteLength)
  return output
}

function encode_cbor_integer(value: bigint): Uint8Array {
  const negative = value < 0n
  const transformed = negative ? -value - 1n : value
  return encode_cbor_bigint_argument(negative ? 1 : 0, transformed)
}

function encode_cbor_argument(major: number, value: number): Uint8Array {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new OpenKache_Error("CBOR argument is outside the supported range")
  }
  const prefix = major << 5
  if (value <= 23) return Uint8Array.of(prefix | value)
  if (value <= 0xff) return Uint8Array.of(prefix | 24, value)
  if (value <= 0xffff) {
    return Uint8Array.of(prefix | 25, value >>> 8, value & 0xff)
  }
  if (value <= 0xffff_ffff) {
    return Uint8Array.of(
      prefix | 26,
      (value >>> 24) & 0xff,
      (value >>> 16) & 0xff,
      (value >>> 8) & 0xff,
      value & 0xff,
    )
  }
  const output = new Uint8Array(9)
  output[0] = prefix | 27
  let remaining = BigInt(value)
  for (let index = 8; index >= 1; index -= 1) {
    output[index] = Number(remaining & 0xffn)
    remaining >>= 8n
  }
  return output
}

function encode_cbor_bigint_argument(major: number, value: bigint): Uint8Array {
  if (value < 0n || value > 0xffff_ffff_ffff_ffffn) {
    throw new OpenKache_Error("CBOR integer argument is outside the supported range")
  }
  if (value <= 23n) return Uint8Array.of((major << 5) | Number(value))
  if (value <= 0xffn) return Uint8Array.of((major << 5) | 24, Number(value))
  if (value <= 0xffffn) {
    return Uint8Array.of(
      (major << 5) | 25,
      Number((value >> 8n) & 0xffn),
      Number(value & 0xffn),
    )
  }
  if (value <= 0xffff_ffffn) {
    return Uint8Array.of(
      (major << 5) | 26,
      Number((value >> 24n) & 0xffn),
      Number((value >> 16n) & 0xffn),
      Number((value >> 8n) & 0xffn),
      Number(value & 0xffn),
    )
  }
  const output = new Uint8Array(9)
  output[0] = (major << 5) | 27
  let remaining = value
  for (let index = 8; index >= 1; index -= 1) {
    output[index] = Number(remaining & 0xffn)
    remaining >>= 8n
  }
  return output
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
        throw new OpenKache_Error("text keys must not contain unpaired surrogates")
      }
      index += 1
    } else if (code_unit >= 0xdc00 && code_unit <= 0xdfff) {
      throw new OpenKache_Error("text keys must not contain unpaired surrogates")
    }
  }
}

function as_openkache_error(error: unknown): OpenKache_Error {
  if (error instanceof OpenKache_Error) return error
  const message = error_message(error)
  if (message.startsWith(INCOMPATIBLE_OUTCOME_PREFIX)) {
    return new OpenKache_Error(
      message.slice(INCOMPATIBLE_OUTCOME_PREFIX.length),
      error,
      "incompatible_server_outcome",
    )
  }
  if (message.startsWith("openkache:error:unknown_mutation:")) {
    return new OpenKache_Unknown_Mutation_Error(
      message.slice("openkache:error:unknown_mutation:".length),
      error,
    )
  }
  return new OpenKache_Error(message, error)
}

function error_message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
