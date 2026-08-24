import {
  load_native_module,
  type Native_Client,
  type Native_Client_Options,
} from "./native-binding.js"
import {
  assert_json_value,
  decode_structured_value,
  encode_structured_value,
  to_native,
  type Json_Value,
  type Structured_Value,
} from "./value-codec.js"

export type {
  Json_Value,
  Structured_Value,
  Structured_Value_Error_Kind,
  Value_Limits,
} from "./value-codec.js"
export type {
  Json_Object,
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
export * from "./generated_local/smithy-api.js"
export * from "./generated_local/smithy-value-format.js"
export * from "./generated_local/smithy-value-envelope.js"
import {
  SMITHY_DEFAULT_MAX_IN_FLIGHT,
  SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS,
  SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS,
  SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS,
  SMITHY_DEFAULT_ZSTANDARD_LEVEL,
  SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX,
  SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN,
  SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES,
  SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES,
  SMITHY_CLIENT_DEFAULT_SERVER_NAME,
  SMITHY_ITEM_ID_BYTES,
  SMITHY_MAX_VALUE_BYTES,
  SMITHY_EVICTION_MODE_EVICTABLE,
  SMITHY_EVICTION_MODE_EVICTION_PROTECTED,
  SMITHY_EVICTION_MODE_INHERIT,
  SMITHY_EXPIRATION_MODE_EXPLICIT_TTL,
  SMITHY_EXPIRATION_MODE_INHERIT,
  SMITHY_EXPIRATION_MODE_NO_EXPIRY,
  SMITHY_FFI_CONNECTION_STATE_CLOSED_NAME,
  SMITHY_FFI_CONNECTION_STATE_CONNECTED_NAME,
  SMITHY_FFI_CONNECTION_STATE_DISCONNECTED_NAME,
  SMITHY_FFI_CONNECTION_STATE_RECONNECTING_NAME,
  SMITHY_FFI_CONNECTION_STATE_UNKNOWN_NAME,
  SMITHY_SET_CONDITION_ANY,
  SMITHY_SET_CONDITION_IF_ABSENT,
  SMITHY_SET_CONDITION_IF_PRESENT,
  SMITHY_SET_OUTCOME_CREATED,
  SMITHY_SET_OUTCOME_NOT_STORED,
  SMITHY_SET_OUTCOME_REPLACED,
  type Smithy_Delete_Input,
  type Smithy_Delete_Output,
  type Smithy_Get_Input,
  type Smithy_Get_Output,
  type Smithy_Eviction_Mode,
  type Smithy_Expiration_Mode,
  type Smithy_Namespace_Delete_Input,
  type Smithy_Namespace_Delete_Output,
  type Smithy_Namespace_Descriptor,
  type Smithy_Namespace_Open_Input,
  type Smithy_Namespace_Open_Output,
  type Smithy_Namespace_Policy,
  type Smithy_Namespace_Update_Policy_Input,
  type Smithy_Namespace_Update_Policy_Output,
  type Smithy_OpenKache_Api,
  type Smithy_Ping_Input,
  type Smithy_Ping_Output,
  type Smithy_Set_Condition,
  type Smithy_Set_Input,
  type Smithy_Set_Outcome,
  type Smithy_Set_Output,
  type Smithy_Stats_Input,
  type Smithy_Stats_Output,
  type Smithy_Sync_Input,
  type Smithy_Sync_Output,
} from "./generated_local/smithy-api.js"
import { SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES } from "./generated_local/smithy-value-format.js"

const TEXT_ENCODER = new TextEncoder()
export const MAX_CANONICAL_KEY_BYTES = 1_048_576

export type Key_Spec = "integer" | "text" | "bytes"
export type Client_Key = string | Uint8Array | number | bigint

/**
 * Native JavaScript projection returned by the primary `get` method.
 *
 * Plain objects supplied to `set` are represented as `Map` values so text keys,
 * scalar keys, and entry order remain unambiguous across language bindings.
 */
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
 * Zstandard compression settings applied by the Rust value codec.
 */
export interface Zstandard_Options {
  /** Enables automatic level-1 Zstandard compression before encryption. */
  readonly enabled?: boolean
  /** Zstandard compression level from the shared contract range. */
  readonly level?: number
  /** Optional input-size threshold; the maintained default is zero. */
  readonly minimum_input_size?: number
  /** Optional savings threshold; the maintained default is zero. */
  readonly minimum_savings?: number
}

/**
 * Certificate identity presented to production servers that require mutual TLS.
 */
export interface Client_Identity {
  /** Client leaf certificate followed by intermediates, each encoded as DER or PEM. */
  readonly certificate_chain: readonly Uint8Array[]
  /** PKCS#1, SEC1, or PKCS#8 private key encoded as DER or PEM. */
  readonly private_key: Uint8Array
}

/**
 * Native connection and complete request/response deadlines.
 */
export interface Client_Timeouts {
  /** Maximum duration for connection setup and the TLS handshake. */
  readonly connect_ms?: number
  /** Maximum duration for one complete cache operation. */
  readonly request_ms?: number
}

/**
 * Retry settings for response-safe operations.
 */
export interface Retry_Options {
  /** Maximum total attempts, including the initial request. */
  readonly max_attempts?: number
}

/**
 * Connection settings for the Rust-backed Node.js, Bun, and Deno client.
 */
export interface Client_Options {
  /** Server transport address, such as `127.0.0.1:4433`. */
  readonly address: string
  /** Server or CA certificate trusted for the selected TLS transport, encoded as DER or PEM. */
  readonly certificate: Uint8Array
  /** Optional exact 32-byte root secret; omitted means unprotected values. */
  readonly data_protection_key?: Uint8Array
  /** Deprecated compatibility selector; mapped operations infer the key type per call. */
  readonly key_spec?: Key_Spec
  /** TLS server name. Defaults to the shared contract value. */
  readonly server_name?: string
  /** Client certificate and private key required by production mutual TLS. */
  readonly identity?: Client_Identity
  /** Client-side compression settings. */
  readonly compression?: Zstandard_Options
  /** Bounded connection and operation durations. */
  readonly timeouts?: Client_Timeouts
  /** Automatic retry policy for response-safe operations. */
  readonly retry?: Retry_Options
  /** Maximum concurrent request lanes on one connection (TCP uses one lane). */
  readonly max_in_flight?: number
  /** Aggregate bytes retained across transport and value-protection work. */
  readonly max_in_flight_bytes?: number
  /** Authenticated value-encryption profile; requires `data_protection_key`. */
  readonly encryption?: "compact" | "robust"
  /** Explicit Node-API adapter path, primarily for custom packaging. */
  readonly native_path?: string
  /** Verified QUIC (default), TLS-over-TCP, or explicit TLS-preserving insecure variants. */
  readonly transport?: "quic" | "tls_tcp" | "quic_insecure" | "tls_tcp_insecure"
}

/**
 * Outcome of a successful `set` operation.
 */
export type Set_Outcome = Smithy_Set_Outcome

/**
 * Optional TTL and atomic existence condition for `set`.
 */
export interface Set_Options {
  /** Store only when the key is absent (`if_absent`) or present (`if_present`). */
  readonly condition?: Smithy_Set_Condition
  /** Item expiration selection. */
  readonly expiration_mode?: Smithy_Expiration_Mode
  /** Item capacity-eviction selection. */
  readonly eviction_mode?: Smithy_Eviction_Mode
  /** Positive relative lifetime in milliseconds. */
  readonly ttl_ms?: number
}

/** Structured-value projection requested by a read operation. */
export type Value_Representation = "lossless" | "native"

/**
 * Structured statistics returned by an administrator-authorized server.
 */
export interface Server_Stats {
  /** Storage implementation reported by the server. */
  readonly storage: string
  /** Per-worker statistics encoded by the server. */
  readonly workers: readonly string[]
}

/**
 * Best-effort lifecycle state reported by the shared Rust core.
 */
export type Connection_State =
  | typeof SMITHY_FFI_CONNECTION_STATE_CONNECTED_NAME
  | typeof SMITHY_FFI_CONNECTION_STATE_RECONNECTING_NAME
  | typeof SMITHY_FFI_CONNECTION_STATE_DISCONNECTED_NAME
  | typeof SMITHY_FFI_CONNECTION_STATE_CLOSED_NAME
  | typeof SMITHY_FFI_CONNECTION_STATE_UNKNOWN_NAME

export type OpenKache_Error_Kind =
  | "openkache_error"
  | "unknown_mutation"
  | "cancelled"

/**
 * Error returned by client validation, structured-value conversion, native
 * transport, or server failures.
 */
export class OpenKache_Error extends Error {
  readonly kind: OpenKache_Error_Kind

  /**
   * Creates a stable client error.
   *
   * @param message - Human-readable failure description.
   * @param cause - Optional underlying failure.
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

/** A mutation may have reached the server but its outcome is unknown. */
export class OpenKache_Unknown_Mutation_Error extends OpenKache_Error {
  constructor(message: string, cause?: unknown) {
    super(message, cause, "unknown_mutation")
    this.name = "OpenKache_Unknown_Mutation_Error"
  }
}

/** The native boundary cancelled an operation before a definitive result. */
export class OpenKache_Cancelled_Error extends OpenKache_Error {
  constructor(message: string, cause?: unknown) {
    super(message, cause, "cancelled")
    this.name = "OpenKache_Cancelled_Error"
  }
}

/**
 * Promise-based Node.js, Bun, and Deno client backed by Rust through Node-API.
 */
export class OpenKache_Client {
  readonly #native_client: Native_Client
  readonly #raw_client: OpenKache_Raw_Client
  readonly #lifecycle: Client_Lifecycle
  readonly #key_spec: Key_Spec | undefined

  private constructor(
    native_client: Native_Client,
    key_spec: Key_Spec | undefined,
    lifecycle: Client_Lifecycle,
  ) {
    this.#native_client = native_client
    this.#key_spec = key_spec
    this.#lifecycle = lifecycle
    this.#raw_client = new Raw_Client(native_client, lifecycle)
    CLIENT_FINALIZER.register(this.#raw_client, native_client, this.#raw_client)
  }

  /**
   * Connects Node.js, Bun, or Deno through the packaged asynchronous Node-API adapter.
   *
   * @param options - Address, trust, mTLS identity, encryption, and compression settings.
   * @returns A connected client that reuses the selected transport connection.
   * @throws {OpenKache_Error} When configuration, native loading, or transport setup fails.
   */
  static async connect(options: Client_Options): Promise<OpenKache_Client> {
    validate_options(options)
    const compression = options.compression ?? {}
    const timeouts = options.timeouts ?? {}
    const retry = options.retry ?? {}
    const native_options: Native_Client_Options = {
      address: options.address,
      server_name: options.server_name ?? SMITHY_CLIENT_DEFAULT_SERVER_NAME,
      certificate: options.certificate.slice(),
      identity: owned_identity(options.identity),
      data_protection_key: options.data_protection_key?.slice(),
      compression_enabled: compression.enabled !== false,
      compression_level: compression.level ?? SMITHY_DEFAULT_ZSTANDARD_LEVEL,
      minimum_input_size:
        compression.minimum_input_size ?? SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES,
      minimum_savings:
        compression.minimum_savings ?? SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES,
      connect_timeout_ms:
        timeouts.connect_ms ?? SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS,
      request_timeout_ms:
        timeouts.request_ms ?? SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS,
      retry_max_attempts:
        retry.max_attempts ?? SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS,
      max_in_flight: options.max_in_flight ?? SMITHY_DEFAULT_MAX_IN_FLIGHT,
      max_in_flight_bytes: options.max_in_flight_bytes,
      encryption: options.encryption,
      key_spec: options.key_spec,
      transport: options.transport ?? "quic",
    }
    try {
      const native_module = load_native_module(options.native_path)
      const native_client = await native_module.connect(native_options)
      const client = new OpenKache_Client(
        native_client,
        options.key_spec,
        { closed: false },
      )
      return client
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Verifies that the server is reachable and speaks the expected protocol.
   *
   * @returns A promise that resolves after a valid `PONG`.
   * @throws {OpenKache_Error} When the client is closed or the operation fails.
   */
  async ping(): Promise<void> {
    this.#assert_open()
    try {
      await this.#native_client.ping()
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Returns the raw Smithy operation client sharing this connection.
   *
   * The returned client accepts exact protocol item IDs and opaque bytes. It
   * does not derive IDs or apply the JavaScript value-codec registry.
   *
   * @returns A raw client view over this connection.
   */
  raw(): OpenKache_Raw_Client {
    return this.#raw_client
  }

  /**
   * Returns the shared core's best-effort lifecycle state.
   *
   * @returns The latest connection state snapshot.
   * @throws {OpenKache_Error} When the native state cannot be read.
   */
  connection_state(): Connection_State {
    try {
      return parse_connection_state(this.#native_client.connection_state())
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Reconnects without replaying an operation.
   *
   * @returns A promise resolved after a replacement connection is ready.
   * @throws {OpenKache_Error} When the client is closed or reconnection fails.
   */
  async reconnect(): Promise<void> {
    this.#assert_open()
    try {
      await this.#native_client.reconnect()
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Retrieves and decodes one value from the common structured value model.
   *
   * This is the primary mapped-value API. The shared core owns the wire
   * representation; the result uses the documented native JavaScript
   * projection (`bigint`, `number`, `Uint8Array`, arrays, and maps).
   *
   * @typeParam Value - Optional caller-provided result shape.
   * @param key - A UTF-8 string, Uint8Array, or signed-i64 integer inferred per operation.
   * @returns The decoded value, or `undefined` when the key does not exist.
   * @throws {OpenKache_Error} When the client is closed, the key is invalid,
   * or transport, decryption, or decoding fails.
   */
  async get<Value = Native_Value>(
    key: Client_Key,
  ): Promise<Value | undefined> {
    this.#assert_open()
    let payload: Uint8Array | null
    try {
      payload = await this.#native_client.get_structured(
        owned_key_bytes(key, this.#key_spec),
      )
    } catch (error) {
      throw as_openkache_error(error)
    }
    if (payload === null) return undefined
    try {
      const model = decode_structured_value(payload)
      return to_native(model) as Value
    } catch (error) {
      throw new OpenKache_Error(`structured value decoding failed: ${error_message(error)}`, error)
    }
  }

  /**
   * Encodes and stores one value from the common structured value model.
   *
   * This is the primary mapped-value API. It accepts the native values defined
   * by the common value model; it does not use JSON, Raw, or the former
   * package-local metadata envelope.
   *
   * @param key - A UTF-8 string, Uint8Array, or signed-i64 integer inferred per operation.
   * @param value - Runtime value accepted by `encode_structured_value`.
   * @param options - Optional TTL and `if_absent` or `if_present` condition.
   * @returns Whether the operation created, replaced, or did not store the key.
   * @throws {OpenKache_Error} When the client is closed or validation, encoding,
   * transport, or storage fails.
   */
  async set(
    key: Client_Key,
    value: unknown,
    options: Set_Options = {},
  ): Promise<Set_Outcome> {
    this.#assert_open()
    validate_set_options(options)
    let payload: Uint8Array
    try {
      payload = encode_structured_value(value)
    } catch (error) {
      throw new OpenKache_Error(`structured value encoding failed: ${error_message(error)}`, error)
    }
    try {
      const outcome = await this.#native_client.set_structured(
        owned_key_bytes(key, this.#key_spec),
        payload,
        options.condition,
        options.expiration_mode,
        options.eviction_mode,
        options.ttl_ms,
      )
      return parse_set_outcome(outcome)
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Retrieves one value from the common structured value model.
   *
   * The default lossless representation retains integer/float distinctions,
   * undefined, scalar map-key identity, and map order.  Native projection
   * returns bigint/number/Uint8Array/Map and rejects values that cannot be
   * represented without loss.
   *
   * @param key - A UTF-8 string, Uint8Array, or signed-i64 integer inferred per operation.
   * @param representation - Lossless model wrappers by default, or a strict native projection.
   * @returns The decoded value, or `undefined` when the key does not exist.
   * @throws {OpenKache_Error} When the client is closed, the key or representation is invalid,
   * or transport or decoding fails.
   */
  async get_structured(
    key: Client_Key,
    representation?: "lossless",
  ): Promise<Structured_Value | undefined>
  async get_structured(
    key: Client_Key,
    representation: "native",
  ): Promise<unknown | undefined>
  async get_structured(
    key: Client_Key,
    representation: Value_Representation = "lossless",
  ): Promise<unknown | undefined> {
    this.#assert_open()
    if (representation !== "lossless" && representation !== "native") {
      throw new OpenKache_Error(
        "representation must be lossless or native",
      )
    }
    let payload: Uint8Array | null
    try {
      payload = await this.#native_client.get_structured(
        owned_key_bytes(key, this.#key_spec),
      )
    } catch (error) {
      throw as_openkache_error(error)
    }
    if (payload === null) return undefined
    try {
      const value = decode_structured_value(payload)
      return representation === "native" ? to_native(value) : value
    } catch (error) {
      throw new OpenKache_Error(
        `structured value decoding failed: ${error_message(error)}`,
        error,
      )
    }
  }

  /**
   * Encodes and stores one value from the common structured value model.
   *
   * This method never routes through legacy JSON or Raw operations; the
   * generated native adapter owns the structured representation directly.
   *
   * @param key - A UTF-8 string, Uint8Array, or signed-i64 integer inferred per operation.
   * @param value - A runtime value accepted by `encode_structured_value`.
   * @param options - Optional TTL and atomic existence condition.
   * @returns Whether the operation created, replaced, or did not store the key.
   * @throws {OpenKache_Error} When the client is closed or encoding, validation,
   * transport, or storage fails.
   */
  async set_structured(
    key: Client_Key,
    value: unknown,
    options: Set_Options = {},
  ): Promise<Set_Outcome> {
    this.#assert_open()
    validate_set_options(options)
    let payload: Uint8Array
    try {
      payload = encode_structured_value(value)
    } catch (error) {
      throw new OpenKache_Error(
        `structured value encoding failed: ${error_message(error)}`,
        error,
      )
    }
    try {
      const outcome = await this.#native_client.set_structured(
        owned_key_bytes(key, this.#key_spec),
        payload,
        options.condition,
        options.expiration_mode,
        options.eviction_mode,
        options.ttl_ms,
      )
      return parse_set_outcome(outcome)
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Retrieves a caller-owned version-0 envelope without interpreting or
   * rewriting its body.
   *
   * @param key - A UTF-8 string, Uint8Array, or signed-i64 integer inferred per operation.
   * @returns The complete stored version-0 bytes, or `undefined` when absent.
   * @throws {OpenKache_Error} When the client is closed or the operation fails.
   */
  async get_v0(key: Client_Key): Promise<Uint8Array | undefined> {
    this.#assert_open()
    try {
      const value = await this.#native_client.get_v0(
        owned_key_bytes(key, this.#key_spec),
      )
      return value === null ? undefined : value
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Stores a complete caller-owned version-0 envelope unchanged.
   *
   * The shared core validates only the canonical leading version field and
   * the protocol value-size limit; all remaining version-0 interpretation
   * belongs to the caller.
   */
  async set_v0(
    key: Client_Key,
    value: Uint8Array,
    options: Set_Options = {},
  ): Promise<Set_Outcome> {
    this.#assert_open()
    validate_set_options(options)
    if (!(value instanceof Uint8Array)) {
      throw new OpenKache_Error("value must be a Uint8Array")
    }
    try {
      const outcome = await this.#native_client.set_v0(
        owned_key_bytes(key, this.#key_spec),
        owned_raw_value(value),
        options.condition,
        options.expiration_mode,
        options.eviction_mode,
        options.ttl_ms,
      )
      return parse_set_outcome(outcome)
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Retrieves exact decrypted and decompressed bytes without envelope decoding.
   *
   * @param key - A UTF-8 string, Uint8Array, or signed-i64 integer inferred per operation.
   * @returns Stored bytes, or `undefined` when the key does not exist.
   * @throws {OpenKache_Error} When the client is closed or the operation fails.
   */
  async get_raw(key: Client_Key): Promise<Uint8Array | undefined> {
    this.#assert_open()
    try {
      const value = await this.#native_client.get(
        owned_key_bytes(key, this.#key_spec),
      )
      return value === null ? undefined : value
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Stores exact bytes without value-envelope encoding.
   *
   * @param key - A UTF-8 string, Uint8Array, or signed-i64 integer inferred per operation.
   * @param value - Bytes to compress, encrypt, and store; empty values are supported.
   * @param options - Optional TTL and `if_absent` or `if_present` condition.
   * @returns Whether the operation created, replaced, or did not store the key.
   * @throws {OpenKache_Error} When validation, transport, or storage fails.
   */
  async set_raw(
    key: Client_Key,
    value: Uint8Array,
    options: Set_Options = {},
  ): Promise<Set_Outcome> {
    this.#assert_open()
    validate_set_options(options)
    if (!(value instanceof Uint8Array)) {
      throw new OpenKache_Error("value must be a Uint8Array")
    }
    return this.#set_owned_bytes(key, owned_raw_value(value), options)
  }

  /**
   * Retrieves a value encoded by the shared core's canonical JSON format.
   *
   * This is an advanced JSON compatibility helper. The primary `get` method
   * reads the common structured value instead.
   *
   * @param key - A UTF-8 string, Uint8Array, or signed-i64 integer inferred per operation.
   * @returns The canonical JSON value, or `undefined` when absent.
   * @throws {OpenKache_Error} When transport, value validation, or decoding fails.
   */
  async get_json(key: Client_Key): Promise<Json_Value | undefined> {
    this.#assert_open()
    let result: string | null
    try {
      result = await this.#native_client.get_json(
        owned_key_bytes(key, this.#key_spec),
      )
    } catch (error) {
      throw as_openkache_error(error)
    }
    if (result === null) return undefined
    try {
      return parse_json_value(JSON.parse(result) as unknown)
    } catch (error) {
      throw new OpenKache_Error(`canonical JSON decoding failed: ${error_message(error)}`, error)
    }
  }

  /**
   * Stores a value through the shared core's canonical JSON format.
   *
   * This is an advanced JSON compatibility helper. The primary `set` method
   * writes the common structured value instead.
   *
   * @param key - A UTF-8 string, Uint8Array, or signed-i64 integer inferred per operation.
   * @param value - Dense, finite JSON value.
   * @param options - Optional TTL and atomic existence condition.
   * @returns Whether the operation created, replaced, or did not store the key.
   * @throws {OpenKache_Error} When validation, canonicalization, transport, or storage fails.
   */
  async set_json(
    key: Client_Key,
    value: Json_Value,
    options: Set_Options = {},
  ): Promise<Set_Outcome> {
    this.#assert_open()
    validate_set_options(options)
    try {
      assert_json_value(value)
      const encoded = JSON.stringify(value)
      if (encoded === undefined) {
        throw new Error("value cannot be represented as JSON")
      }
      const outcome = await this.#native_client.set_json(
        owned_key_bytes(key, this.#key_spec),
        encoded,
        options.condition,
        options.expiration_mode,
        options.eviction_mode,
        options.ttl_ms,
      )
      return parse_set_outcome(outcome)
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Deletes a key.
   *
   * @param key - A UTF-8 string, Uint8Array, or signed-i64 integer inferred per operation.
   * @returns `true` when the key existed and was deleted.
   * @throws {OpenKache_Error} When the client is closed or the operation fails.
   */
  async delete(key: Client_Key): Promise<boolean> {
    this.#assert_open()
    try {
      return await this.#native_client.delete(
        owned_key_bytes(key, this.#key_spec),
      )
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Retrieves structured server statistics.
   *
   * @returns Validated storage and per-worker statistics.
   * @throws {OpenKache_Error} When authorization, transport, or response validation fails.
   */
  async stats(): Promise<Server_Stats> {
    this.#assert_open()
    let text: string
    try {
      text = await this.#native_client.stats()
    } catch (error) {
      throw as_openkache_error(error)
    }
    try {
      return parse_stats(text)
    } catch (error) {
      throw new OpenKache_Error(`STATS decoding failed: ${error_message(error)}`, error)
    }
  }

  /**
   * Requests a server durability barrier.
   *
   * @returns A promise that resolves after every SSD worker flushes.
   * @throws {OpenKache_Error} When authorization, transport, or synchronization fails.
   */
  async sync(): Promise<void> {
    this.#assert_open()
    try {
      await this.#native_client.sync()
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Closes the native connection. Repeated calls are safe.
   *
   * @returns A shared promise for native resource release.
   */
  close(): Promise<void> {
    return this.#close_once()
  }

  async #set_owned_bytes(
    key: Client_Key,
    bytes: Uint8Array,
    options: Set_Options,
  ): Promise<Set_Outcome> {
    this.#assert_open()
    try {
      const outcome = await this.#native_client.set(
        owned_key_bytes(key, this.#key_spec),
        bytes,
        options.condition,
        options.expiration_mode,
        options.eviction_mode,
        options.ttl_ms,
      )
      return parse_set_outcome(outcome)
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  #assert_open(): void {
    assert_lifecycle_open(this.#lifecycle)
  }

  async #close_once(): Promise<void> {
    try {
      await close_native_client(this.#native_client, this.#lifecycle)
    } catch (error) {
      throw as_openkache_error(error)
    } finally {
      CLIENT_FINALIZER.unregister(this.#raw_client)
    }
  }
}

function assert_lifecycle_open(lifecycle: Client_Lifecycle): void {
  if (lifecycle.closed || lifecycle.close_promise !== undefined) {
    throw new OpenKache_Error("client is closed")
  }
}

function close_native_client(
  native_client: Native_Client,
  lifecycle: Client_Lifecycle,
): Promise<void> {
  lifecycle.close_promise ??= (async (): Promise<void> => {
    try {
      await native_client.close()
    } finally {
      lifecycle.closed = true
    }
  })()
  return lifecycle.close_promise
}

function parse_json_value(value: unknown): Json_Value {
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return value
  }
  if (typeof value === "number" && Number.isFinite(value)) {
    return value
  }
  if (Array.isArray(value)) {
    return value.map((child, index): Json_Value => {
      try {
        return parse_json_value(child)
      } catch (error) {
        throw new Error(`$[${index}] is invalid: ${error_message(error)}`)
      }
    })
  }
  if (is_regular_object(value)) {
    const result: Record<string, Json_Value> = {}
    for (const [key, child] of Object.entries(value)) {
      Object.defineProperty(result, key, {
        configurable: true,
        enumerable: true,
        value: parse_json_value(child),
        writable: true,
      })
    }
    return result
  }
  throw new Error("response is not a finite JSON value")
}

/**
 * Exact-item-ID client implementing the Smithy-generated service contract.
 *
 * This view shares the protected connection owned by `OpenKache_Client`; it
 * does not open a second transport connection. Use it when an application already
 * owns protocol item IDs and formatted value bytes.
 */
export interface OpenKache_Raw_Client extends Smithy_OpenKache_Api {
  /** Reads canonical JSON UTF-8 bytes at an exact Item ID. */
  get_json(input: Smithy_Get_Input): Promise<Smithy_Get_Output>
  /** Stores canonical JSON UTF-8 bytes at an exact Item ID. */
  set_json(input: Smithy_Set_Input): Promise<Smithy_Set_Output>
  /** Reads structured-value bytes at an exact Item ID. */
  get_structured(input: Smithy_Get_Input): Promise<Smithy_Get_Output>
  /** Stores structured-value bytes at an exact Item ID. */
  set_structured(input: Smithy_Set_Input): Promise<Smithy_Set_Output>
  /** Reads a caller-owned version-0 envelope at an exact Item ID. */
  get_v0(input: Smithy_Get_Input): Promise<Smithy_Get_Output>
  /** Stores a caller-owned version-0 envelope at an exact Item ID. */
  set_v0(input: Smithy_Set_Input): Promise<Smithy_Set_Output>
  /**
   * Reconnects the shared core client without replaying an operation.
   *
   * @returns A promise resolved after reconnection.
   * @throws {OpenKache_Error} When reconnection fails.
   */
  reconnect(): Promise<void>

  /**
   * Closes the shared native connection.
   *
   * @returns A promise resolved after native resource release.
   * @throws {OpenKache_Error} When native shutdown fails.
   */
  close(): Promise<void>

  /**
   * Returns the shared core's best-effort lifecycle state.
   *
   * @returns The latest connection state snapshot, including `closed`.
   * @throws {OpenKache_Error} When native state cannot be read.
   */
  connection_state(): Connection_State
}

class Raw_Client implements OpenKache_Raw_Client {
  readonly #native_client: Native_Client
  readonly #lifecycle: Client_Lifecycle

  constructor(
    native_client: Native_Client,
    lifecycle: Client_Lifecycle = { closed: false },
  ) {
    this.#native_client = native_client
    this.#lifecycle = lifecycle
  }

  /**
   * Invokes the Smithy PING operation.
   *
   * @param _input - Empty Smithy operation input.
   * @returns The Smithy operation output with its empty payload.
   * @throws {OpenKache_Error} When the operation fails.
   */
  async ping(_input: Smithy_Ping_Input): Promise<Smithy_Ping_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      await this.#native_client.ping()
      return { payload: new Uint8Array() }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Invokes the Smithy GET operation for an exact item ID.
   *
   * @param input - Exact protocol item ID.
   * @returns Opaque stored bytes, or an absent value.
   * @throws {OpenKache_Error} When the operation fails.
   */
  async get(input: Smithy_Get_Input): Promise<Smithy_Get_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      const value = await this.#native_client.raw_get_in_namespace(
        input.namespace_id,
        owned_item_id(input.item_id),
      )
      return value === null ? {} : { value }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  async get_json(input: Smithy_Get_Input): Promise<Smithy_Get_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      const value = await this.#native_client.get_json_in_namespace(
        input.namespace_id,
        owned_item_id(input.item_id),
      )
      return value === null
        ? {}
        : { value: new TextEncoder().encode(value) }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Invokes the Smithy SET operation for an exact item ID.
   *
   * @param input - Exact item ID, opaque bytes, and optional set behavior.
   * @returns The Smithy set outcome.
   * @throws {OpenKache_Error} When validation or the operation fails.
   */
  async set(input: Smithy_Set_Input): Promise<Smithy_Set_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      const outcome = await this.#native_client.raw_set_in_namespace(
        input.namespace_id,
        owned_item_id(input.item_id),
        owned_raw_value(input.value),
        input.condition,
        input.expiration_mode,
        input.eviction_mode,
        input.ttl_milliseconds,
      )
      return { outcome: parse_set_outcome(outcome) }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  async set_json(input: Smithy_Set_Input): Promise<Smithy_Set_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      const outcome = await this.#native_client.set_json_in_namespace(
        input.namespace_id,
        owned_item_id(input.item_id),
        new TextDecoder("utf-8", { fatal: true }).decode(input.value),
        input.condition,
        input.expiration_mode,
        input.eviction_mode,
        input.ttl_milliseconds,
      )
      return { outcome: parse_set_outcome(outcome) }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  async get_structured(input: Smithy_Get_Input): Promise<Smithy_Get_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      const value = await this.#native_client.get_structured_in_namespace(
        input.namespace_id,
        owned_item_id(input.item_id),
      )
      return value === null ? {} : { value }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  async set_structured(input: Smithy_Set_Input): Promise<Smithy_Set_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      const outcome = await this.#native_client.set_structured_in_namespace(
        input.namespace_id,
        owned_item_id(input.item_id),
        owned_raw_value(input.value),
        input.condition,
        input.expiration_mode,
        input.eviction_mode,
        input.ttl_milliseconds,
      )
      return { outcome: parse_set_outcome(outcome) }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  async get_v0(input: Smithy_Get_Input): Promise<Smithy_Get_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      const value = await this.#native_client.get_v0_in_namespace(
        input.namespace_id,
        owned_item_id(input.item_id),
      )
      return value === null ? {} : { value }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  async set_v0(input: Smithy_Set_Input): Promise<Smithy_Set_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      const outcome = await this.#native_client.set_v0_in_namespace(
        input.namespace_id,
        owned_item_id(input.item_id),
        owned_raw_value(input.value),
        input.condition,
        input.expiration_mode,
        input.eviction_mode,
        input.ttl_milliseconds,
      )
      return { outcome: parse_set_outcome(outcome) }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Invokes the Smithy DELETE operation for an exact item ID.
   *
   * @param input - Exact protocol item ID.
   * @returns Whether the item was deleted.
   * @throws {OpenKache_Error} When the operation fails.
   */
  async delete(input: Smithy_Delete_Input): Promise<Smithy_Delete_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      return {
        deleted: await this.#native_client.raw_delete_in_namespace(
          input.namespace_id,
          owned_item_id(input.item_id),
        ),
      }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Invokes the Smithy STATS operation.
   *
   * @param _input - Empty Smithy operation input.
   * @returns The server's JSON statistics string.
   * @throws {OpenKache_Error} When authorization or transport fails.
   */
  async stats(input: Smithy_Stats_Input): Promise<Smithy_Stats_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      return { json: await this.#native_client.stats_in_namespace(input.namespace_id) }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Invokes the Smithy SYNC operation.
   *
   * @param _input - Empty Smithy operation input.
   * @returns An empty Smithy operation output.
   * @throws {OpenKache_Error} When authorization or synchronization fails.
   */
  async sync(input: Smithy_Sync_Input): Promise<Smithy_Sync_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      await this.#native_client.sync_in_namespace(input.namespace_id)
      return {}
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Invokes the Smithy NAMESPACE_OPEN operation.
   *
   * @param input - Namespace name, creation flag, and optional policy.
   * @returns The server-assigned descriptor and creation result.
   */
  async namespace_open(
    input: Smithy_Namespace_Open_Input,
  ): Promise<Smithy_Namespace_Open_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      return await this.#native_client.namespace_open(
        input.name,
        input.create_if_missing,
        input.policy,
      )
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Invokes the Smithy NAMESPACE_UPDATE_POLICY operation.
   */
  async namespace_update_policy(
    input: Smithy_Namespace_Update_Policy_Input,
  ): Promise<Smithy_Namespace_Update_Policy_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      return {
        descriptor: await this.#native_client.namespace_update_policy(
          input.namespace_id,
          input.expected_revision,
          input.policy,
        ),
      }
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Invokes the Smithy NAMESPACE_DELETE operation.
   */
  async namespace_delete(
    input: Smithy_Namespace_Delete_Input,
  ): Promise<Smithy_Namespace_Delete_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      await this.#native_client.namespace_delete(
        input.namespace_id,
        input.expected_revision,
      )
      return {}
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Reconnects the shared core client without replaying an operation.
   *
   * @returns A promise resolved after reconnection.
   * @throws {OpenKache_Error} When reconnection fails.
   */
  async reconnect(): Promise<void> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      await this.#native_client.reconnect()
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Closes the shared native connection.
   *
   * @returns A promise resolved after native resource release.
   * @throws {OpenKache_Error} When native shutdown fails.
   */
  async close(): Promise<void> {
    try {
      await close_native_client(this.#native_client, this.#lifecycle)
    } catch (error) {
      throw as_openkache_error(error)
    } finally {
      CLIENT_FINALIZER.unregister(this)
    }
  }

  /**
   * Returns the shared core's best-effort lifecycle state.
   *
   * @returns The latest connection state snapshot, including `closed`.
   * @throws {OpenKache_Error} When native state cannot be read.
   */
  connection_state(): Connection_State {
    try {
      return parse_connection_state(this.#native_client.connection_state())
    } catch (error) {
      throw as_openkache_error(error)
    }
  }
}

function owned_key_bytes(
  key: Client_Key,
  _key_spec?: Key_Spec,
): Uint8Array {
  if (typeof key === "string") {
    assert_valid_unicode_string(key)
    const bytes = TEXT_ENCODER.encode(key)
    return encode_cbor_bytes_or_text(3, bytes)
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
      throw new OpenKache_Error(
        "integer keys must fit the signed 64-bit range",
      )
    }
    return encode_cbor_integer(key)
  }
  throw new OpenKache_Error(
    "key must be a UTF-8 string, Uint8Array, or signed-i64 integer",
  )
}

function encode_cbor_bytes_or_text(
  major: 2 | 3,
  bytes: Uint8Array,
): Uint8Array {
  const header = encode_cbor_argument(major, bytes.byteLength)
  const total_length = header.byteLength + bytes.byteLength
  if (total_length > MAX_CANONICAL_KEY_BYTES) {
    throw new OpenKache_Error(
      `canonical key exceeds ${MAX_CANONICAL_KEY_BYTES} bytes`,
    )
  }
  const output = new Uint8Array(total_length)
  output.set(header)
  output.set(bytes, header.byteLength)
  return output
}

function encode_cbor_integer(value: bigint): Uint8Array {
  const minimum = -(1n << 63n)
  const maximum = (1n << 63n) - 1n
  if (value < minimum || value > maximum) {
    throw new OpenKache_Error("integer keys must fit signed i64")
  }
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
  const output = new Uint8Array(value <= 0xffff_ffffn ? 5 : 9)
  if (value <= 23n) {
    return Uint8Array.of((major << 5) | Number(value))
  }
  if (value <= 0xffn) {
    return Uint8Array.of((major << 5) | 24, Number(value))
  }
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
      if (next_code_unit < 0xdc00 || next_code_unit > 0xdfff) {
        throw new OpenKache_Error("text keys must not contain unpaired surrogates")
      }
      index += 1
    } else if (code_unit >= 0xdc00 && code_unit <= 0xdfff) {
      throw new OpenKache_Error("text keys must not contain unpaired surrogates")
    }
  }
}

function owned_item_id(item_id: Uint8Array): Uint8Array {
  if (!(item_id instanceof Uint8Array)) {
    throw new OpenKache_Error("item_id must be a Uint8Array")
  }
  const bytes = item_id.slice()
  if (bytes.byteLength > SMITHY_ITEM_ID_BYTES) {
    throw new OpenKache_Error(
      `item_id must contain at most ${SMITHY_ITEM_ID_BYTES} bytes, got ${bytes.byteLength}`,
    )
  }
  return bytes
}

function owned_raw_value(value: Uint8Array): Uint8Array {
  if (!(value instanceof Uint8Array)) {
    throw new OpenKache_Error("value must be a Uint8Array")
  }
  if (value.byteLength > SMITHY_MAX_VALUE_BYTES) {
    throw new OpenKache_Error(
      `value exceeds the protocol maximum of ${SMITHY_MAX_VALUE_BYTES} bytes`,
    )
  }
  return value.slice()
}

function owned_identity(identity: Client_Identity | undefined): Client_Identity | undefined {
  if (identity === undefined) return undefined
  return {
    certificate_chain: identity.certificate_chain.map(
      (certificate): Uint8Array => certificate.slice(),
    ),
    private_key: identity.private_key.slice(),
  }
}

function validate_options(options: Client_Options): void {
  if (!is_regular_object(options)) {
    throw new OpenKache_Error("client options must be a regular object")
  }
  if (typeof options.address !== "string" || options.address.length === 0) {
    throw new OpenKache_Error("address must be a non-empty string")
  }
  const insecure_transport =
    options.transport === "quic_insecure" || options.transport === "tls_tcp_insecure"
  if (
    (!(options.certificate instanceof Uint8Array) || options.certificate.byteLength === 0) &&
    !insecure_transport
  ) {
    throw new OpenKache_Error("certificate must be a non-empty Uint8Array")
  }
  if (
    options.transport !== undefined &&
    options.transport !== "quic" &&
    options.transport !== "tls_tcp" &&
    options.transport !== "quic_insecure" &&
    options.transport !== "tls_tcp_insecure"
  ) {
    throw new OpenKache_Error("transport must be quic, tls_tcp, quic_insecure, or tls_tcp_insecure")
  }
  if (
    options.data_protection_key !== undefined &&
    (!(options.data_protection_key instanceof Uint8Array) ||
      options.data_protection_key.byteLength !== SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES)
  ) {
    throw new OpenKache_Error(
      `data_protection_key must contain exactly ${SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES} bytes when supplied`,
    )
  }
  if (
    options.key_spec !== undefined &&
    options.key_spec !== "integer" &&
    options.key_spec !== "text" &&
    options.key_spec !== "bytes"
  ) {
    throw new OpenKache_Error(
      `key_spec must be integer, text, or bytes, got ${String(options.key_spec)}`,
    )
  }
  if (
    options.server_name !== undefined &&
    (typeof options.server_name !== "string" || options.server_name.length === 0)
  ) {
    throw new OpenKache_Error("server_name must be a non-empty string")
  }
  if (
    options.native_path !== undefined &&
    (typeof options.native_path !== "string" || options.native_path.length === 0)
  ) {
    throw new OpenKache_Error("native_path must be a non-empty string")
  }
  if (options.identity !== undefined && !is_regular_object(options.identity)) {
    throw new OpenKache_Error("identity must be a regular object")
  }
  if (options.compression !== undefined && !is_regular_object(options.compression)) {
    throw new OpenKache_Error("compression must be a regular object")
  }
  if (options.timeouts !== undefined && !is_regular_object(options.timeouts)) {
    throw new OpenKache_Error("timeouts must be a regular object")
  }
  if (options.retry !== undefined && !is_regular_object(options.retry)) {
    throw new OpenKache_Error("retry must be a regular object")
  }
  validate_compression(options.compression)
  validate_timeout(options.timeouts?.connect_ms, "timeouts.connect_ms")
  validate_timeout(options.timeouts?.request_ms, "timeouts.request_ms")
  validate_positive_integer(options.retry?.max_attempts, "retry.max_attempts")
  validate_positive_integer(options.max_in_flight, "max_in_flight")
  validate_positive_integer(options.max_in_flight_bytes, "max_in_flight_bytes")
  if (
    options.encryption !== undefined &&
    options.encryption !== "compact" &&
    options.encryption !== "robust"
  ) {
    throw new OpenKache_Error(
      `encryption must be compact or robust, got ${String(options.encryption)}`,
    )
  }
  if (options.identity !== undefined) {
    if (
      !Array.isArray(options.identity.certificate_chain) ||
      options.identity.certificate_chain.length === 0
    ) {
      throw new OpenKache_Error("identity.certificate_chain must not be empty")
    }
    for (const certificate of options.identity.certificate_chain) {
      if (!(certificate instanceof Uint8Array) || certificate.byteLength === 0) {
        throw new OpenKache_Error(
          "identity.certificate_chain entries must be non-empty Uint8Arrays",
        )
      }
    }
    if (
      !(options.identity.private_key instanceof Uint8Array) ||
      options.identity.private_key.byteLength === 0
    ) {
      throw new OpenKache_Error("identity.private_key must be a non-empty Uint8Array")
    }
  }
}

function validate_compression(options: Zstandard_Options | undefined): void {
  if (options === undefined) return
  if (options.enabled !== undefined && typeof options.enabled !== "boolean") {
    throw new OpenKache_Error("compression.enabled must be a boolean")
  }
  if (
    options.level !== undefined &&
    (!Number.isSafeInteger(options.level) ||
      options.level < SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN ||
      options.level > SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX)
  ) {
    throw new OpenKache_Error(
      "compression.level must be an integer from "
        + `${SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN} through ${SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX}`,
    )
  }
  validate_non_negative_integer(options.minimum_input_size, "compression.minimum_input_size")
  validate_non_negative_integer(options.minimum_savings, "compression.minimum_savings")
}

function validate_timeout(timeout_ms: number | undefined, name: string): void {
  if (timeout_ms !== undefined && (!Number.isSafeInteger(timeout_ms) || timeout_ms <= 0)) {
    throw new OpenKache_Error(`${name} must be a positive safe integer`)
  }
}

function validate_positive_integer(value: number | undefined, name: string): void {
  if (value !== undefined && (!Number.isSafeInteger(value) || value <= 0)) {
    throw new OpenKache_Error(`${name} must be a positive safe integer`)
  }
}

function validate_non_negative_integer(value: number | undefined, name: string): void {
  if (value !== undefined && (!Number.isSafeInteger(value) || value < 0)) {
    throw new OpenKache_Error(`${name} must be a non-negative safe integer`)
  }
}

function validate_set_options(options: Set_Options): void {
  if (!is_regular_object(options)) {
    throw new OpenKache_Error("set options must be a regular object")
  }
  if (
    options.condition !== undefined &&
    options.condition !== SMITHY_SET_CONDITION_ANY &&
    options.condition !== SMITHY_SET_CONDITION_IF_ABSENT &&
    options.condition !== SMITHY_SET_CONDITION_IF_PRESENT
  ) {
    throw new OpenKache_Error(
      `condition must be ${SMITHY_SET_CONDITION_ANY}, ${SMITHY_SET_CONDITION_IF_ABSENT}, or ${SMITHY_SET_CONDITION_IF_PRESENT}, got ${String(options.condition)}`,
    )
  }
  if (
    options.expiration_mode !== undefined &&
    options.expiration_mode !== SMITHY_EXPIRATION_MODE_INHERIT &&
    options.expiration_mode !== SMITHY_EXPIRATION_MODE_NO_EXPIRY &&
    options.expiration_mode !== SMITHY_EXPIRATION_MODE_EXPLICIT_TTL
  ) {
    throw new OpenKache_Error(
      `expiration_mode must be ${SMITHY_EXPIRATION_MODE_INHERIT}, ${SMITHY_EXPIRATION_MODE_NO_EXPIRY}, or ${SMITHY_EXPIRATION_MODE_EXPLICIT_TTL}, got ${String(options.expiration_mode)}`,
    )
  }
  if (
    options.eviction_mode !== undefined &&
    options.eviction_mode !== SMITHY_EVICTION_MODE_INHERIT &&
    options.eviction_mode !== SMITHY_EVICTION_MODE_EVICTABLE &&
    options.eviction_mode !== SMITHY_EVICTION_MODE_EVICTION_PROTECTED
  ) {
    throw new OpenKache_Error(
      `eviction_mode must be ${SMITHY_EVICTION_MODE_INHERIT}, ${SMITHY_EVICTION_MODE_EVICTABLE}, or ${SMITHY_EVICTION_MODE_EVICTION_PROTECTED}, got ${String(options.eviction_mode)}`,
    )
  }
  if (
    options.expiration_mode === SMITHY_EXPIRATION_MODE_EXPLICIT_TTL &&
    options.ttl_ms === undefined
  ) {
    throw new OpenKache_Error(
      `ttl_ms is required with ${SMITHY_EXPIRATION_MODE_EXPLICIT_TTL} expiration_mode`,
    )
  }
  if (
    options.expiration_mode !== undefined &&
    options.expiration_mode !== SMITHY_EXPIRATION_MODE_EXPLICIT_TTL &&
    options.ttl_ms !== undefined
  ) {
    throw new OpenKache_Error(
      `ttl_ms is only valid with ${SMITHY_EXPIRATION_MODE_EXPLICIT_TTL} expiration_mode`,
    )
  }
  validate_positive_integer(options.ttl_ms, "ttl_ms")
}

function is_regular_object(value: unknown): value is object {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false
  if (value instanceof Uint8Array) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

function parse_stats(text: string): Server_Stats {
  const value: unknown = JSON.parse(text)
  if (!is_regular_object(value)) {
    throw new Error("response is not an object")
  }
  const candidate = value as Record<string, unknown>
  if (typeof candidate.storage !== "string") {
    throw new Error("response.storage is not a string")
  }
  if (
    !Array.isArray(candidate.workers) ||
    !candidate.workers.every((worker): worker is string => typeof worker === "string")
  ) {
    throw new Error("response.workers is not a string array")
  }
  return {
    storage: candidate.storage,
    workers: candidate.workers,
  }
}

function parse_set_outcome(value: string): Set_Outcome {
  switch (value) {
    case SMITHY_SET_OUTCOME_CREATED:
    case SMITHY_SET_OUTCOME_REPLACED:
    case SMITHY_SET_OUTCOME_NOT_STORED:
      return value
    default:
      throw new OpenKache_Error(`SET returned unexpected native outcome ${value}`)
  }
}

function parse_connection_state(value: string): Connection_State {
  switch (value) {
    case SMITHY_FFI_CONNECTION_STATE_CONNECTED_NAME:
    case SMITHY_FFI_CONNECTION_STATE_RECONNECTING_NAME:
    case SMITHY_FFI_CONNECTION_STATE_DISCONNECTED_NAME:
    case SMITHY_FFI_CONNECTION_STATE_CLOSED_NAME:
    case SMITHY_FFI_CONNECTION_STATE_UNKNOWN_NAME:
      return value
    default:
      throw new OpenKache_Error(
        `native client returned unexpected connection state ${value}`,
      )
  }
}

function as_openkache_error(error: unknown): OpenKache_Error {
  return error instanceof OpenKache_Error
    ? error
    : native_category_error(error)
}

function error_message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function native_category_error(error: unknown): OpenKache_Error {
  const message = error_message(error)
  const categories: readonly [string, (message: string, cause: unknown) => OpenKache_Error][] = [
    [
      "openkache:error:unknown_mutation:",
      (detail, cause) => new OpenKache_Unknown_Mutation_Error(detail, cause),
    ],
    [
      "openkache:error:cancelled:",
      (detail, cause) => new OpenKache_Cancelled_Error(detail, cause),
    ],
  ]
  for (const [prefix, create] of categories) {
    if (message.startsWith(prefix)) return create(message.slice(prefix.length), error)
  }
  return new OpenKache_Error(message, error)
}
