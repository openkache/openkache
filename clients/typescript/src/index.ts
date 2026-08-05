import {
  load_native_module,
  type Native_Client,
  type Native_Client_Options,
} from "./native-binding.js"
import {
  assert_json_value,
  Value_Codec_Registry,
  type Json_Value,
  type Value_Codec,
  type Value_Envelope,
} from "./value-codec.js"
import {
  Smithy_Generated_Operations,
  type Smithy_Operation_Transport,
} from "./generated_local/smithy-operations.js"

export type {
  Encoded_Value,
  Json_Object,
  Json_Value,
  Value_Codec,
  Value_Envelope,
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
  type Smithy_Echo_Input,
  type Smithy_Echo_Output,
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
  /** Enables Zstandard compression before encryption. */
  readonly enabled?: boolean
  /** Zstandard compression level from the shared contract range. */
  readonly level?: number
  /** Values below this byte length bypass compression. */
  readonly minimum_input_size?: number
  /** Compressed values must save at least this many bytes. */
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
  /** Maximum duration for connection setup and the QUIC/TLS handshake. */
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
  /** Server UDP socket address, such as `127.0.0.1:4433`. */
  readonly address: string
  /** Server or CA certificate trusted for the QUIC connection, encoded as DER or PEM. */
  readonly certificate: Uint8Array
  /** Exact 32-byte master secret used to derive key-hiding and value-encryption subkeys. */
  readonly data_protection_key: Uint8Array
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
  /** Maximum concurrent request lanes on one connection. */
  readonly max_in_flight?: number
  /** Authenticated value-encryption profile. Defaults to `robust`. */
  readonly encryption?: "compact" | "robust"
  /** Optional Protobuf, FlatBuffers, or application value codecs. */
  readonly value_codecs?: readonly Value_Codec[]
  /** Explicit Node-API adapter path, primarily for custom packaging. */
  readonly native_path?: string
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

/**
 * Error returned by client validation, value codecs, native transport, or server failures.
 */
export class OpenKache_Error extends Error {
  readonly kind = "openkache_error" as const

  /**
   * Creates a stable client error.
   *
   * @param message - Human-readable failure description.
   * @param cause - Optional underlying failure.
   */
  constructor(message: string, cause?: unknown) {
    super(message, cause === undefined ? undefined : { cause })
    this.name = "OpenKache_Error"
  }
}

/**
 * Promise-based Node.js, Bun, and Deno client backed by Rust through Node-API.
 */
export class OpenKache_Client {
  readonly #native_client: Native_Client
  readonly #value_codecs: Value_Codec_Registry
  readonly #raw_client: OpenKache_Raw_Client
  readonly #lifecycle: Client_Lifecycle

  private constructor(
    native_client: Native_Client,
    value_codecs: Value_Codec_Registry,
    lifecycle: Client_Lifecycle,
  ) {
    this.#native_client = native_client
    this.#value_codecs = value_codecs
    this.#lifecycle = lifecycle
    this.#raw_client = new Raw_Client(native_client, lifecycle)
    CLIENT_FINALIZER.register(this.#raw_client, native_client, this.#raw_client)
  }

  /**
   * Connects Node.js, Bun, or Deno through the packaged asynchronous Node-API adapter.
   *
   * @param options - Address, trust, mTLS identity, encryption, and compression settings.
   * @returns A connected client that reuses one QUIC connection.
   * @throws {OpenKache_Error} When configuration, native loading, TLS, or QUIC fails.
   */
  static async connect(options: Client_Options): Promise<OpenKache_Client> {
    validate_options(options)
    let value_codecs: Value_Codec_Registry
    try {
      value_codecs = new Value_Codec_Registry(options.value_codecs ?? [])
    } catch (error) {
      throw new OpenKache_Error(
        `value codec configuration failed: ${error_message(error)}`,
        error,
      )
    }
    const compression = options.compression ?? {}
    const timeouts = options.timeouts ?? {}
    const retry = options.retry ?? {}
    const native_options: Native_Client_Options = {
      address: options.address,
      server_name: options.server_name ?? SMITHY_CLIENT_DEFAULT_SERVER_NAME,
      certificate: options.certificate.slice(),
      identity: owned_identity(options.identity),
      data_protection_key: options.data_protection_key.slice(),
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
      encryption: options.encryption,
    }
    try {
      const native_module = load_native_module(options.native_path)
      const native_client = await native_module.connect(native_options)
      const client = new OpenKache_Client(native_client, value_codecs, {
        closed: false,
      })
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
   * Sends an experimental UTF-8 message and returns the echoed message.
   *
   * @param message - UTF-8 text carried by the experimental ECHO operation.
   * @returns The server's UTF-8 response.
   * @throws {OpenKache_Error} When the client is closed or the response is not UTF-8.
   */
  async echo(message: string): Promise<string> {
    this.#assert_open()
    if (typeof message !== "string") {
      throw new OpenKache_Error("message must be a string")
    }
    try {
      const value = await this.#native_client.echo(TEXT_ENCODER.encode(message))
      return new TextDecoder("utf-8", { fatal: true }).decode(value)
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
   * Retrieves and codec-decodes a JSON value or custom codec object.
   *
   * @typeParam Value - Expected object shape selected by the caller.
   * @param key - Exact non-empty string or binary cache key.
   * @returns The decoded value, or `undefined` when the key does not exist.
   * @throws {OpenKache_Error} When transport, decryption, or decoding fails.
   */
  async get<Value = Json_Value>(
    key: string | Uint8Array,
  ): Promise<Value | undefined> {
    this.#assert_open()
    let envelope: Value_Envelope | null
    try {
      envelope = await this.#native_client.get_value(owned_key_bytes(key))
    } catch (error) {
      throw as_openkache_error(error)
    }
    if (envelope === null) return undefined
    try {
      return this.#value_codecs.decode(envelope) as Value
    } catch (error) {
      throw new OpenKache_Error(`value decoding failed: ${error_message(error)}`, error)
    }
  }

  /**
   * Codec-encodes and stores a JSON value.
   *
   * @typeParam Value - JSON value shape to store.
   * @param key - Exact non-empty string or binary cache key.
   * @param value - JSON value accepted by the built-in envelope or a registered
   * custom object codec.
   * @param options - Optional TTL and `if_absent` or `if_present` condition.
   * @returns Whether the operation created, replaced, or did not store the key.
   * @throws {OpenKache_Error} When validation, encoding, transport, or storage fails.
   */
  async set<Value>(
    key: string | Uint8Array,
    value: Value,
    options: Set_Options = {},
  ): Promise<Set_Outcome> {
    this.#assert_open()
    validate_set_options(options)
    let envelope: Value_Envelope
    try {
      envelope = this.#value_codecs.encode(value)
    } catch (error) {
      throw new OpenKache_Error(`value encoding failed: ${error_message(error)}`, error)
    }
    try {
      const outcome = await this.#native_client.set_value(
        owned_key_bytes(key),
        envelope.encoding,
        envelope.type_name,
        envelope.payload,
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
   * @param key - Exact non-empty string or binary cache key.
   * @returns Stored bytes, or `undefined` when the key does not exist.
   * @throws {OpenKache_Error} When the client is closed or the operation fails.
   */
  async get_raw(key: string | Uint8Array): Promise<Uint8Array | undefined> {
    this.#assert_open()
    try {
      const value = await this.#native_client.get(owned_key_bytes(key))
      return value === null ? undefined : value
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Stores exact bytes without value-envelope encoding.
   *
   * @param key - Exact non-empty string or binary cache key.
   * @param value - Bytes to compress, encrypt, and store; empty values are supported.
   * @param options - Optional TTL and `if_absent` or `if_present` condition.
   * @returns Whether the operation created, replaced, or did not store the key.
   * @throws {OpenKache_Error} When validation, transport, or storage fails.
   */
  async set_raw(
    key: string | Uint8Array,
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
   * This method is the cross-language value API. Use `get` when reading the
   * backwards-compatible TypeScript metadata envelope or a custom codec.
   *
   * @param key - Exact non-empty string or binary cache key.
   * @returns The canonical JSON value, or `undefined` when absent.
   * @throws {OpenKache_Error} When transport, value validation, or decoding fails.
   */
  async get_json(key: string | Uint8Array): Promise<Json_Value | undefined> {
    this.#assert_open()
    let result: string | null
    try {
      result = await this.#native_client.get_json(owned_key_bytes(key))
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
   * @param key - Exact non-empty string or binary cache key.
   * @param value - Dense, finite JSON value.
   * @param options - Optional TTL and atomic existence condition.
   * @returns Whether the operation created, replaced, or did not store the key.
   * @throws {OpenKache_Error} When validation, canonicalization, transport, or storage fails.
   */
  async set_json(
    key: string | Uint8Array,
    value: Json_Value,
    options: Set_Options = {},
  ): Promise<Set_Outcome> {
    this.#assert_open()
    validate_set_options(options)
    try {
      assert_json_value(value)
      const outcome = await this.#native_client.set_json(
        owned_key_bytes(key),
        value,
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
   * @param key - Exact non-empty string or binary cache key.
   * @returns `true` when the key existed and was deleted.
   * @throws {OpenKache_Error} When the client is closed or the operation fails.
   */
  async delete(key: string | Uint8Array): Promise<boolean> {
    this.#assert_open()
    try {
      return await this.#native_client.delete(owned_key_bytes(key))
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
    key: string | Uint8Array,
    bytes: Uint8Array,
    options: Set_Options,
  ): Promise<Set_Outcome> {
    this.#assert_open()
    try {
      const outcome = await this.#native_client.set(
        owned_key_bytes(key),
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
 * does not open a second QUIC connection. Use it when an application already
 * owns protocol item IDs and formatted value bytes.
 */
export interface OpenKache_Raw_Client extends Smithy_OpenKache_Api {
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

class Raw_Client extends Smithy_Generated_Operations implements OpenKache_Raw_Client {
  readonly #native_client: Native_Client
  readonly #lifecycle: Client_Lifecycle

  constructor(
    native_client: Native_Client,
    lifecycle: Client_Lifecycle = { closed: false },
  ) {
    super(raw_operation_transport(native_client, lifecycle))
    this.#native_client = native_client
    this.#lifecycle = lifecycle
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

function raw_operation_transport(
  native_client: Native_Client,
  lifecycle: Client_Lifecycle,
): Smithy_Operation_Transport {
  return {
    assert_open: (): void => assert_lifecycle_open(lifecycle),
    ping: async (): Promise<void> => {
      try {
        await native_client.ping()
      } catch (error) {
        throw as_openkache_error(error)
      }
    },
    echo: async (input: Smithy_Echo_Input): Promise<string> => {
      try {
        const value = await native_client.echo(TEXT_ENCODER.encode(input.message))
        return new TextDecoder("utf-8", { fatal: true }).decode(value)
      } catch (error) {
        throw as_openkache_error(error)
      }
    },
    get: async (input: Smithy_Get_Input): Promise<Uint8Array | null> => {
      try {
        return await native_client.raw_get_in_namespace(
          input.namespace_id,
          owned_item_id(input.item_id),
        )
      } catch (error) {
        throw as_openkache_error(error)
      }
    },
    set: async (input: Smithy_Set_Input): Promise<Set_Outcome> => {
      try {
        const outcome = await native_client.raw_set_in_namespace(
          input.namespace_id,
          owned_item_id(input.item_id),
          owned_raw_value(input.value),
          input.condition,
          input.expiration_mode,
          input.eviction_mode,
          input.ttl_milliseconds,
        )
        return parse_set_outcome(outcome)
      } catch (error) {
        throw as_openkache_error(error)
      }
    },
    delete: async (input: Smithy_Delete_Input): Promise<boolean> => {
      try {
        return await native_client.raw_delete_in_namespace(
          input.namespace_id,
          owned_item_id(input.item_id),
        )
      } catch (error) {
        throw as_openkache_error(error)
      }
    },
    stats: async (input: Smithy_Stats_Input): Promise<string> => {
      try {
        return await native_client.stats_in_namespace(input.namespace_id)
      } catch (error) {
        throw as_openkache_error(error)
      }
    },
    sync: async (input: Smithy_Sync_Input): Promise<void> => {
      try {
        await native_client.sync_in_namespace(input.namespace_id)
      } catch (error) {
        throw as_openkache_error(error)
      }
    },
    namespace_open: async (
      input: Smithy_Namespace_Open_Input,
    ): Promise<Smithy_Namespace_Open_Output> => {
      try {
        return await native_client.namespace_open(
          input.name,
          input.create_if_missing,
          input.policy,
        )
      } catch (error) {
        throw as_openkache_error(error)
      }
    },
    namespace_update_policy: async (
      input: Smithy_Namespace_Update_Policy_Input,
    ): Promise<Smithy_Namespace_Descriptor> => {
      try {
        return await native_client.namespace_update_policy(
          input.namespace_id,
          input.expected_revision,
          input.policy,
        )
      } catch (error) {
        throw as_openkache_error(error)
      }
    },
    namespace_delete: async (
      input: Smithy_Namespace_Delete_Input,
    ): Promise<void> => {
      try {
        await native_client.namespace_delete(
          input.namespace_id,
          input.expected_revision,
        )
      } catch (error) {
        throw as_openkache_error(error)
      }
    },
  }
}

function owned_key_bytes(key: string | Uint8Array): Uint8Array {
  if (typeof key !== "string" && !(key instanceof Uint8Array)) {
    throw new OpenKache_Error("key must be a string or Uint8Array")
  }
  const bytes = typeof key === "string" ? TEXT_ENCODER.encode(key) : key.slice()
  if (bytes.byteLength === 0) {
    throw new OpenKache_Error("key must not be empty")
  }
  return bytes
}

function owned_item_id(item_id: Uint8Array): Uint8Array {
  if (!(item_id instanceof Uint8Array)) {
    throw new OpenKache_Error("item_id must be a Uint8Array")
  }
  const bytes = item_id.slice()
  if (bytes.byteLength !== SMITHY_ITEM_ID_BYTES) {
    throw new OpenKache_Error(
      `item_id must contain exactly ${SMITHY_ITEM_ID_BYTES} bytes, got ${bytes.byteLength}`,
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
  if (!(options.certificate instanceof Uint8Array) || options.certificate.byteLength === 0) {
    throw new OpenKache_Error("certificate must be a non-empty Uint8Array")
  }
  if (
    !(options.data_protection_key instanceof Uint8Array) ||
    options.data_protection_key.byteLength !== SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES
  ) {
    throw new OpenKache_Error(
      `data_protection_key must contain exactly ${SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES} bytes`,
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
  if (options.value_codecs !== undefined && !Array.isArray(options.value_codecs)) {
    throw new OpenKache_Error("value_codecs must be an array")
  }
  validate_compression(options.compression)
  validate_timeout(options.timeouts?.connect_ms, "timeouts.connect_ms")
  validate_timeout(options.timeouts?.request_ms, "timeouts.request_ms")
  validate_positive_integer(options.retry?.max_attempts, "retry.max_attempts")
  validate_positive_integer(options.max_in_flight, "max_in_flight")
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
    : new OpenKache_Error(error_message(error), error)
}

function error_message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
