import {
  load_native_module,
  type Native_Client,
  type Native_Client_Options,
} from "./native-binding.js"
import {
  decode_structured_value,
  encode_structured_value,
  to_native,
  type Structured_Value,
  type Structured_Value_Error_Kind,
  type Value_Limits,
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
  Array_Value as ArrayValue,
  ByteString_Value,
  ByteString_Value as ByteStringValue,
  Float_Value,
  Float_Value as FloatValue,
  Integer_Value,
  Integer_Value as IntegerValue,
  Map_Value,
  Map_Value as MapValue,
  Structured_Value_Error,
  Structured_Value_Error as StructuredValueError,
  TextString_Value,
  TextString_Value as TextStringValue,
  UNDEFINED_VALUE,
  Undefined_Value,
  Undefined_Value as UndefinedValue,
  decode_native_value,
  decode_native_value as decodeNativeValue,
  decode_structured_value,
  decode_structured_value as decodeStructuredValue,
  encode_structured_value,
  encode_structured_value as encodeStructuredValue,
  model_equal,
  model_equal as modelEqual,
  to_native,
  to_native as toNative,
  to_plain_object,
  to_plain_object as toPlainObject,
  to_value,
  to_value as toValue,
} from "./value-codec.js"
export type StructuredValue = Structured_Value
export type StructuredValueErrorKind = Structured_Value_Error_Kind
export type ValueLimits = Value_Limits

const TEXT_ENCODER = new TextEncoder()
const INCOMPATIBLE_OUTCOME_PREFIX =
  "openkache:error:incompatible_server_outcome:"

/**
 * A mapped cache key: UTF-8 text, exact bytes, a safe integer number, or a
 * signed i64 bigint.
 */
export type ClientKey = string | Uint8Array | number | bigint

/** Compatibility spelling retained for existing callers. */
export type Client_Key = ClientKey

/** Connection options accepted by the client. */
export interface ClientOptions {
  /** Server endpoint, for example `127.0.0.1:4433`. */
  readonly address: string
}

/** Compatibility spelling retained for existing callers. */
export type Client_Options = ClientOptions

/** Native values accepted by `set` after conversion to the lossless model. */
export type NativeValue =
  | undefined
  | null
  | boolean
  | bigint
  | number
  | string
  | Uint8Array
  | readonly NativeValue[]
  | ReadonlyMap<NativeValue, NativeValue>
  | { readonly [key: string]: NativeValue }

/** Compatibility spelling retained for existing callers. */
export type Native_Value = NativeValue

/** Selects the value view returned by `get`. */
export type ValueRepresentation = "native" | "lossless"

/** Compatibility spelling retained for existing callers. */
export type Value_Representation = ValueRepresentation

/** Optional value-view settings accepted by `get`. */
export interface GetOptions {
  readonly representation?: ValueRepresentation
}

/** Compatibility spelling retained for existing callers. */
export type Get_Options = GetOptions

/** Public set outcomes for unconditional writes. */
export type SetOutcome = Gate0_Set_Outcome

/** Compatibility spelling retained for existing callers. */
export type Set_Outcome = SetOutcome

/** Stable categories for failures surfaced by the client. */
export type OpenKacheErrorKind =
  | "openkache_error"
  | "unknown_mutation"
  | "incompatible_server_outcome"

/** Compatibility spelling retained for existing callers. */
export type OpenKache_Error_Kind = OpenKacheErrorKind

/** Error raised by validation, transport, server, or value conversion. */
export class OpenKacheError extends Error {
  readonly kind: OpenKacheErrorKind

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
    kind: OpenKacheErrorKind = "openkache_error",
  ) {
    super(message, cause === undefined ? undefined : { cause })
    this.name = "OpenKacheError"
    this.kind = kind
  }
}

/** Compatibility spelling retained for existing callers. */
export { OpenKacheError as OpenKache_Error }

/** A mutation may have crossed admission but did not return a definitive result. */
export class OpenKacheUnknownMutationError extends OpenKacheError {
  constructor(message: string, cause?: unknown) {
    super(message, cause, "unknown_mutation")
    this.name = "OpenKacheUnknownMutationError"
  }
}

/** Compatibility spelling retained for existing callers. */
export { OpenKacheUnknownMutationError as OpenKache_Unknown_Mutation_Error }

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
 * Promise-based OpenKache client.
 *
 * The public cache surface is deliberately limited to `connect`, `get`, `set`,
 * `delete`, and `close`. The development profile uses TLS 1.3 with server
 * certificate verification disabled; this is development only — do not use
 * this trust profile in production.
 */
export class OpenKacheClient {
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
   * Opens a connection using the local development profile.
   *
   * The server certificate is deliberately not verified. TLS 1.3 still
   * encrypts traffic, but this profile has no active MITM protection and is
   * development only — do not use this trust profile in production.
   *
   * @param endpoint - Server endpoint string or the `{ address }` shape.
   * @returns A connected client.
   * @throws {OpenKacheError} When the endpoint or native connection is invalid.
   */
  static async connect(
    endpoint: string | ClientOptions,
  ): Promise<OpenKacheClient> {
    const address = parse_endpoint(endpoint)
    const native_options: Native_Client_Options = { address }
    try {
      const native_client = await load_native_module().connect(native_options)
      return new OpenKacheClient(native_client, { closed: false })
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Retrieves one value using the native JavaScript representation by default.
   *
   * `undefined` is returned when no live item exists. A stored `undefined` has
   * the same result in the native view, just as with `Map.get`; pass
   * `{ representation: "lossless" }` to retain the model's `Undefined` value,
   * integer/float distinctions, raw float bits, and model map keys.
   *
   * @param key - UTF-8 text, exact bytes, a safe integer number, or a
   * signed-i64 bigint key.
   * @param options - Optional native or lossless value representation.
   * @returns The decoded value, or `undefined` when the key is absent.
   * @throws {OpenKacheError} When validation, transport, decoding, or native
   * projection fails.
   */
  async get(
    key: ClientKey,
    options?: GetOptions & { readonly representation?: "native" },
  ): Promise<NativeValue | undefined>
  async get(
    key: ClientKey,
    options: GetOptions & { readonly representation: "lossless" },
  ): Promise<StructuredValue | undefined>
  async get(
    key: ClientKey,
    options: GetOptions = {},
  ): Promise<NativeValue | StructuredValue | undefined> {
    this.#assert_open()
    if (
      options.representation !== undefined &&
      options.representation !== "native" &&
      options.representation !== "lossless"
    ) {
      throw new OpenKacheError(
        'representation must be "native" or "lossless"',
      )
    }
    let payload: Uint8Array | null
    try {
      payload = await this.#native_client.get(owned_key_bytes(key))
    } catch (error) {
      throw as_openkache_error(error)
    }
    if (payload === null) return undefined
    let model: StructuredValue
    try {
      model = decode_structured_value(payload)
    } catch (error) {
      throw new OpenKacheError(
        `structured value decoding failed: ${error_message(error)}`,
        error,
      )
    }
    if (options.representation === "lossless") {
      return model
    }
    try {
      return to_native(model) as NativeValue
    } catch (error) {
      throw new OpenKacheError(
        `native value projection failed: ${error_message(error)}`,
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
   * @throws {OpenKacheError} When validation, encoding, transport, or storage fails.
   */
  async set(
    key: ClientKey,
    value: NativeValue | StructuredValue,
  ): Promise<SetOutcome> {
    this.#assert_open()
    let payload: Uint8Array
    try {
      payload = encode_structured_value(value)
    } catch (error) {
      throw new OpenKacheError(
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
        throw new OpenKacheError(
          "server returned unsupported conditional SET outcome not_stored",
          undefined,
          "incompatible_server_outcome",
        )
      default:
        throw new OpenKacheError(
          `SET returned unexpected native outcome ${outcome}`,
        )
    }
  }

  /**
   * Deletes one mapped key.
   *
   * @param key - UTF-8 text, exact bytes, a safe integer number, or a
   * signed-i64 bigint key.
   * @returns `true` when an item existed, otherwise `false`.
   * @throws {OpenKacheError} When validation, transport, or storage fails.
   */
  async delete(key: ClientKey): Promise<boolean> {
    this.#assert_open()
    try {
      return await this.#native_client.delete(owned_key_bytes(key))
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Closes the connection. Repeated calls complete successfully.
   *
   * If native shutdown rejects, the rejected promise is discarded so a later
   * call retries shutdown. Operations remain closed after the first attempt.
   *
   * @returns A promise resolved after native resource release.
   */
  close(): Promise<void> {
    if (this.#lifecycle.close_promise !== undefined) {
      return this.#lifecycle.close_promise
    }
    const close_promise = Promise.resolve()
      .then(() => this.#native_client.close())
      .then(
        (): void => {
          this.#lifecycle.closed = true
          CLIENT_FINALIZER.unregister(this)
        },
        (error: unknown): never => {
          this.#lifecycle.closed = true
          this.#lifecycle.close_promise = undefined
          throw as_openkache_error(error)
        },
      )
    this.#lifecycle.close_promise = close_promise
    return close_promise
  }

  #assert_open(): void {
    if (this.#lifecycle.closed || this.#lifecycle.close_promise !== undefined) {
      throw new OpenKacheError("client is closed")
    }
  }
}

/** Compatibility spelling retained for existing callers. */
export { OpenKacheClient as OpenKache_Client }
/** Short primary spelling for the package-level client. */
export { OpenKacheClient as Client }

function parse_endpoint(endpoint: string | ClientOptions): string {
  if (typeof endpoint === "string") {
    if (endpoint.length === 0) {
      throw new OpenKacheError("endpoint must be a non-empty string")
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
      throw new OpenKacheError("connect expects an endpoint string or { address }")
    }
    const keys = Reflect.ownKeys(endpoint)
    if (keys.length !== 1 || keys[0] !== "address") {
      throw new OpenKacheError(
        "connect accepts only the address field; trust and certificate options are unsupported",
      )
    }
    const address = endpoint.address
    if (typeof address !== "string" || address.length === 0) {
      throw new OpenKacheError("address must be a non-empty string")
    }
    return address
  } catch (error) {
    if (error instanceof OpenKacheError) throw error
    throw new OpenKacheError(
      "connect expects an endpoint string or { address }",
      error,
    )
  }
}

function owned_key_bytes(key: ClientKey): Uint8Array {
  if (typeof key === "string") {
    assert_valid_unicode_string(key)
    return encode_cbor_bytes_or_text(3, TEXT_ENCODER.encode(key))
  }
  if (key instanceof Uint8Array) {
    return encode_cbor_bytes_or_text(2, key)
  }
  if (typeof key === "number") {
    if (!Number.isSafeInteger(key) || Object.is(key, -0)) {
      throw new OpenKacheError(
        "number keys must be finite safe integers and must not be negative zero",
      )
    }
    return encode_cbor_integer(BigInt(key))
  }
  if (typeof key === "bigint") {
    if (key < -(1n << 63n) || key > (1n << 63n) - 1n) {
      throw new OpenKacheError("integer keys must fit the signed 64-bit range")
    }
    return encode_cbor_integer(key)
  }
  throw new OpenKacheError(
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
    throw new OpenKacheError(
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
    throw new OpenKacheError("CBOR argument is outside the supported range")
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
    throw new OpenKacheError("CBOR integer argument is outside the supported range")
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
        throw new OpenKacheError("text keys must not contain unpaired surrogates")
      }
      index += 1
    } else if (code_unit >= 0xdc00 && code_unit <= 0xdfff) {
      throw new OpenKacheError("text keys must not contain unpaired surrogates")
    }
  }
}

function as_openkache_error(error: unknown): OpenKacheError {
  if (error instanceof OpenKacheError) return error
  const message = error_message(error)
  if (message.startsWith(INCOMPATIBLE_OUTCOME_PREFIX)) {
    return new OpenKacheError(
      message.slice(INCOMPATIBLE_OUTCOME_PREFIX.length),
      error,
      "incompatible_server_outcome",
    )
  }
  if (message.startsWith("openkache:error:unknown_mutation:")) {
    return new OpenKacheUnknownMutationError(
      message.slice("openkache:error:unknown_mutation:".length),
      error,
    )
  }
  return new OpenKacheError(message, error)
}

function error_message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
