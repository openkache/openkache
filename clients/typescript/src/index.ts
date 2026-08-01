import {
  load_native_module,
  type Native_Client,
  type Native_Metrics_Snapshot,
  type Native_Client_Options,
} from "./native-binding.js"
import { assert_json_value, type Json_Object, type Json_Value } from "./json.js"

export { assert_json_value } from "./json.js"
export type {
  Json_Object,
  Json_Value,
} from "./json.js"
export * from "./generated_local/smithy-api.js"
export * from "./generated_local/smithy-value-format.js"
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
  SMITHY_FFI_ERROR_CANCELLED,
  SMITHY_FFI_BACKEND_NONE,
  SMITHY_FFI_OPERATION_GET_JSON,
  SMITHY_FFI_OPERATION_RECONNECT,
  SMITHY_FFI_OPERATION_SET_JSON,
  SMITHY_FFI_PHASE_UNKNOWN,
  SMITHY_OPCODE_DELETE,
  SMITHY_OPCODE_GET,
  SMITHY_OPCODE_PING,
  SMITHY_OPCODE_SET,
  SMITHY_OPCODE_STATS,
  SMITHY_OPCODE_SYNC,
  SMITHY_ITEM_ID_BYTES,
  SMITHY_MAX_PREVIOUS_DATA_PROTECTION_KEYS,
  SMITHY_MUTATION_ID_BYTES,
  SMITHY_MAX_VALUE_BYTES,
  type Smithy_Delete_Input,
  type Smithy_Delete_Output,
  type Smithy_Get_Input,
  type Smithy_Get_Output,
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
  readonly data_protection_key?: Uint8Array
  /** Active and retired data-protection keys used during key rotation. */
  readonly key_ring?: Data_Protection_Key_Ring
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
  /** Explicit Node-API adapter path, primarily for custom packaging. */
  readonly native_path?: string
}

/**
 * Active data-protection key plus a bounded read/delete rotation window.
 */
export interface Data_Protection_Key_Ring {
  readonly active: Uint8Array
  readonly previous?: readonly Uint8Array[]
}

/** Optional AbortSignal used to stop one native operation. */
export interface Request_Options {
  readonly signal?: AbortSignal
}

/**
 * Outcome of a successful `set` operation.
 */
export type Set_Outcome = Smithy_Set_Outcome

/**
 * Optional TTL and atomic existence condition for `set`.
 */
export interface Set_Options extends Request_Options {
  /** Store only when the key is absent (`if_absent`) or present (`if_present`). */
  readonly condition?: Smithy_Set_Condition
  /** Positive relative lifetime in milliseconds. */
  readonly ttl_ms?: number
  /** Fixed-width idempotency token reused when a mutation is retried. */
  readonly mutation_id?: Uint8Array
}

/** Optional idempotency token for a DELETE mutation. */
export interface Delete_Options extends Request_Options {
  readonly mutation_id?: Uint8Array
}

/** Point-in-time native request, retry, error, and lane counters. */
export interface Metrics_Snapshot {
  readonly requests: number
  readonly hits: number
  readonly misses: number
  readonly retries: number
  readonly reconnects: number
  readonly cancellations: number
  readonly transport_errors: number
  readonly protocol_errors: number
  readonly bytes_sent: number
  readonly bytes_received: number
  readonly active_lanes: number
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
  | "connected"
  | "reconnecting"
  | "disconnected"
  | "closed"
  | "unknown"

/**
 * Error returned by client validation, value codecs, native transport, or server failures.
 */
export class OpenKache_Error extends Error {
  readonly kind = "openkache_error" as const
  readonly metadata?: Error_Metadata

  /**
   * Creates a stable client error.
   *
   * @param message - Human-readable failure description.
   * @param cause - Optional underlying failure.
   */
  constructor(message: string, cause?: unknown, metadata?: Error_Metadata) {
    super(message, cause === undefined ? undefined : { cause })
    this.name = "OpenKache_Error"
    this.metadata = metadata
  }
}

/** Structured metadata attached to native operation failures when available. */
export interface Error_Metadata {
  readonly code: number
  readonly operation: number
  readonly phase: number
  readonly backend: number
  readonly retryable: boolean
  readonly ambiguous: boolean
  readonly mutation_id?: Uint8Array
}

/**
 * Promise-based Node.js, Bun, and Deno client backed by Rust through Node-API.
 */
export class OpenKache_Client {
  readonly #native_client: Native_Client
  readonly #raw_client: OpenKache_Raw_Client
  readonly #lifecycle: Client_Lifecycle

  private constructor(
    native_client: Native_Client,
    lifecycle: Client_Lifecycle,
  ) {
    this.#native_client = native_client
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
    const compression = options.compression ?? {}
    const timeouts = options.timeouts ?? {}
    const retry = options.retry ?? {}
    const native_options: Native_Client_Options = {
      address: options.address,
      server_name: options.server_name ?? SMITHY_CLIENT_DEFAULT_SERVER_NAME,
      certificate: options.certificate.slice(),
      identity: owned_identity(options.identity),
      data_protection_key: owned_active_key(options),
      previous_data_protection_keys: owned_previous_keys(options),
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
      const client = new OpenKache_Client(native_client, {
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
  async ping(options: Request_Options = {}): Promise<void> {
    this.#assert_open()
    try {
      await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_OPCODE_PING,
        undefined,
        (request_id): Promise<void> => this.#native_client.ping(request_id),
      )
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Returns the raw Smithy operation client sharing this connection.
   *
   * The returned client accepts exact protocol item IDs and opaque bytes. It
   * does not derive IDs or apply application-specific value codecs.
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
  async reconnect(options: Request_Options = {}): Promise<void> {
    this.#assert_open()
    try {
      await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_FFI_OPERATION_RECONNECT,
        undefined,
        (request_id): Promise<void> => this.#native_client.reconnect(request_id),
      )
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /** Retrieves a canonical JSON value. */
  async get<Value = Json_Value>(
    key: string | Uint8Array,
    options: Request_Options = {},
  ): Promise<Value | undefined> {
    this.#assert_open()
    try {
      const value = await this.get_json(key, options)
      return value as Value | undefined
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /** Stores a canonical JSON value. */
  async set<Value>(
    key: string | Uint8Array,
    value: Value,
    options: Set_Options = {},
  ): Promise<Set_Outcome> {
    this.#assert_open()
    validate_set_options(options)
    try {
      assert_json_value(value)
      const mutation_id = owned_mutation_id(options.mutation_id)
      const outcome = await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_FFI_OPERATION_SET_JSON,
        mutation_id,
        (request_id): Promise<string> =>
          this.#native_client.set_json(
            owned_key_bytes(key),
            value,
            options.condition,
            options.ttl_ms,
            mutation_id,
            request_id,
          ),
      )
      return parse_set_outcome(outcome)
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Retrieves exact decrypted and decompressed opaque bytes.
   *
   * @param key - Exact non-empty string or binary cache key.
   * @returns Stored bytes, or `undefined` when the key does not exist.
   * @throws {OpenKache_Error} When the client is closed or the operation fails.
   */
  async get_raw(
    key: string | Uint8Array,
    options: Request_Options = {},
  ): Promise<Uint8Array | undefined> {
    this.#assert_open()
    try {
      const value = await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_OPCODE_GET,
        undefined,
        (request_id): Promise<Uint8Array | null> =>
          this.#native_client.get(owned_key_bytes(key), request_id),
      )
      return value === null ? undefined : value
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Stores exact opaque bytes without JSON conversion.
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
   * This method is an explicit alias for the canonical JSON API.
   *
   * @param key - Exact non-empty string or binary cache key.
   * @returns The canonical JSON value, or `undefined` when absent.
   * @throws {OpenKache_Error} When transport, value validation, or decoding fails.
   */
  async get_json(
    key: string | Uint8Array,
    options: Request_Options = {},
  ): Promise<Json_Value | undefined> {
    this.#assert_open()
    let result: string | null
    try {
      result = await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_FFI_OPERATION_GET_JSON,
        undefined,
        (request_id): Promise<string | null> =>
          this.#native_client.get_json(owned_key_bytes(key), request_id),
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
      const mutation_id = owned_mutation_id(options.mutation_id)
      const outcome = await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_FFI_OPERATION_SET_JSON,
        mutation_id,
        (request_id): Promise<string> =>
          this.#native_client.set_json(
            owned_key_bytes(key),
            value,
            options.condition,
            options.ttl_ms,
            mutation_id,
            request_id,
          ),
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
  async delete(
    key: string | Uint8Array,
    options: Delete_Options = {},
  ): Promise<boolean> {
    this.#assert_open()
    validate_delete_options(options)
    try {
      const mutation_id = owned_mutation_id(options.mutation_id)
      return await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_OPCODE_DELETE,
        mutation_id,
        (request_id): Promise<boolean> =>
          this.#native_client.delete(
            owned_key_bytes(key),
            mutation_id,
            request_id,
          ),
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
  async stats(options: Request_Options = {}): Promise<Server_Stats> {
    this.#assert_open()
    let text: string
    try {
      text = await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_OPCODE_STATS,
        undefined,
        (request_id): Promise<string> => this.#native_client.stats(request_id),
      )
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
  async sync(options: Request_Options = {}): Promise<void> {
    this.#assert_open()
    try {
      await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_OPCODE_SYNC,
        undefined,
        (request_id): Promise<void> => this.#native_client.sync(request_id),
      )
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  /**
   * Returns a point-in-time native metrics snapshot.
   *
   * @returns Request, hit/miss, retry, cancellation, byte, and lane counters.
   */
  metrics_snapshot(): Metrics_Snapshot {
    this.#assert_open()
    try {
      return normalize_metrics_snapshot(this.#native_client.metrics_snapshot())
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
      const mutation_id = owned_mutation_id(options.mutation_id)
      const outcome = await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_OPCODE_SET,
        mutation_id,
        (request_id): Promise<string> =>
          this.#native_client.set(
            owned_key_bytes(key),
            bytes,
            options.condition,
            options.ttl_ms,
            mutation_id,
            request_id,
          ),
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

async function run_with_signal<T>(
  native_client: Native_Client,
  signal: AbortSignal | undefined,
  operation: number,
  mutation_id: Uint8Array | undefined,
  invoke: (request_id: number) => Promise<T>,
): Promise<T> {
  const request_id = native_client.next_request_id()
  if (signal?.aborted) {
    throw cancellation_error(undefined, undefined, operation, mutation_id)
  }
  let cancelled = false
  const on_abort = (): void => {
    cancelled = true
    try {
      native_client.cancel(request_id)
    } catch {
      // The operation will still resolve through its native deadline.
    }
  }
  signal?.addEventListener("abort", on_abort, { once: true })
  try {
    const result = await invoke(request_id)
    if (cancelled || signal?.aborted) {
      throw cancellation_error(undefined, undefined, operation, mutation_id)
    }
    return result
  } catch (error) {
    if (cancelled || signal?.aborted) {
      const normalized_error = as_openkache_error(error)
      throw cancellation_error(error, normalized_error.metadata, operation, mutation_id)
    }
    throw as_openkache_error(error)
  } finally {
    signal?.removeEventListener("abort", on_abort)
  }
}

function cancellation_error(
  cause?: unknown,
  metadata?: Error_Metadata,
  operation = 0,
  mutation_id?: Uint8Array,
): OpenKache_Error {
  const cancellation_metadata: Error_Metadata = {
    code: SMITHY_FFI_ERROR_CANCELLED,
    operation,
    phase: metadata?.phase ?? SMITHY_FFI_PHASE_UNKNOWN,
    backend: metadata?.backend ?? SMITHY_FFI_BACKEND_NONE,
    retryable: metadata?.retryable ?? mutation_id !== undefined,
    ambiguous: metadata?.ambiguous ?? mutation_id !== undefined,
    mutation_id: mutation_id?.slice() ?? metadata?.mutation_id?.slice(),
  }
  return new OpenKache_Error(
    "client operation canceled",
    cause,
    cancellation_metadata,
  )
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
  ping(input: Smithy_Ping_Input, options?: Request_Options): Promise<Smithy_Ping_Output>
  get(input: Smithy_Get_Input, options?: Request_Options): Promise<Smithy_Get_Output>
  set(input: Smithy_Set_Input, options?: Request_Options): Promise<Smithy_Set_Output>
  delete(input: Smithy_Delete_Input, options?: Request_Options): Promise<Smithy_Delete_Output>
  stats(input: Smithy_Stats_Input, options?: Request_Options): Promise<Smithy_Stats_Output>
  sync(input: Smithy_Sync_Input, options?: Request_Options): Promise<Smithy_Sync_Output>

  /**
   * Reconnects the shared core client without replaying an operation.
   *
   * @returns A promise resolved after reconnection.
   * @throws {OpenKache_Error} When reconnection fails.
   */
  reconnect(options?: Request_Options): Promise<void>

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

  /** Returns a point-in-time native metrics snapshot. */
  metrics_snapshot(): Metrics_Snapshot
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
   * @returns An empty Smithy operation output.
   * @throws {OpenKache_Error} When the operation fails.
   */
  async ping(
    _input: Smithy_Ping_Input,
    options: Request_Options = {},
  ): Promise<Smithy_Ping_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_OPCODE_PING,
        undefined,
        (request_id): Promise<void> => this.#native_client.ping(request_id),
      )
      return {}
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
  async get(
    input: Smithy_Get_Input,
    options: Request_Options = {},
  ): Promise<Smithy_Get_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      const value = await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_OPCODE_GET,
        undefined,
        (request_id): Promise<Uint8Array | null> =>
          this.#native_client.raw_get(owned_item_id(input.item_id), request_id),
      )
      return value === null ? {} : { value }
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
  async set(
    input: Smithy_Set_Input,
    options: Request_Options = {},
  ): Promise<Smithy_Set_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      validate_set_options({
        condition: input.condition,
        ttl_ms: input.ttl_milliseconds,
        mutation_id: input.mutation_id,
      })
      const mutation_id = owned_mutation_id(input.mutation_id)
      const outcome = await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_OPCODE_SET,
        mutation_id,
        (request_id): Promise<string> =>
          this.#native_client.raw_set(
            owned_item_id(input.item_id),
            owned_raw_value(input.value),
            input.condition,
            input.ttl_milliseconds,
            mutation_id,
            request_id,
          ),
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
  async delete(
    input: Smithy_Delete_Input,
    options: Request_Options = {},
  ): Promise<Smithy_Delete_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      const mutation_id = owned_mutation_id(input.mutation_id)
      const deleted = await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_OPCODE_DELETE,
        mutation_id,
        (request_id): Promise<boolean> =>
          this.#native_client.raw_delete(
            owned_item_id(input.item_id),
            mutation_id,
            request_id,
          ),
      )
      return { deleted }
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
  async stats(
    _input: Smithy_Stats_Input,
    options: Request_Options = {},
  ): Promise<Smithy_Stats_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      return {
        json: await run_with_signal(
          this.#native_client,
          options.signal,
          SMITHY_OPCODE_STATS,
          undefined,
          (request_id): Promise<string> => this.#native_client.stats(request_id),
        ),
      }
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
  async sync(
    _input: Smithy_Sync_Input,
    options: Request_Options = {},
  ): Promise<Smithy_Sync_Output> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_OPCODE_SYNC,
        undefined,
        (request_id): Promise<void> => this.#native_client.sync(request_id),
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
  async reconnect(options: Request_Options = {}): Promise<void> {
    assert_lifecycle_open(this.#lifecycle)
    try {
      await run_with_signal(
        this.#native_client,
        options.signal,
        SMITHY_FFI_OPERATION_RECONNECT,
        undefined,
        (request_id): Promise<void> => this.#native_client.reconnect(request_id),
      )
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

  metrics_snapshot(): Metrics_Snapshot {
    assert_lifecycle_open(this.#lifecycle)
    try {
      return normalize_metrics_snapshot(this.#native_client.metrics_snapshot())
    } catch (error) {
      throw as_openkache_error(error)
    }
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
  const has_legacy_key = options.data_protection_key !== undefined
  const has_key_ring = options.key_ring !== undefined
  if (has_legacy_key === has_key_ring) {
    throw new OpenKache_Error(
      "provide exactly one of data_protection_key or key_ring",
    )
  }
  if (
    has_legacy_key &&
    (!(options.data_protection_key instanceof Uint8Array) ||
      options.data_protection_key.byteLength !== SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES)
  ) {
    throw new OpenKache_Error(
      `data_protection_key must contain exactly ${SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES} bytes`,
    )
  }
  if (has_key_ring) validate_key_ring(options.key_ring)
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
    options.condition !== "if_absent" &&
    options.condition !== "if_present"
  ) {
    throw new OpenKache_Error(
      `condition must be if_absent or if_present, got ${String(options.condition)}`,
    )
  }
  validate_positive_integer(options.ttl_ms, "ttl_ms")
  validate_mutation_id(options.mutation_id, "mutation_id")
}

function validate_delete_options(options: Delete_Options): void {
  if (!is_regular_object(options)) {
    throw new OpenKache_Error("delete options must be a regular object")
  }
  validate_mutation_id(options.mutation_id, "mutation_id")
}

function validate_mutation_id(value: Uint8Array | undefined, name: string): void {
  if (
    value !== undefined &&
    (!(value instanceof Uint8Array) || value.byteLength !== SMITHY_MUTATION_ID_BYTES)
  ) {
    throw new OpenKache_Error(
      `${name} must contain exactly ${SMITHY_MUTATION_ID_BYTES} bytes`,
    )
  }
}

function validate_key_ring(key_ring: Data_Protection_Key_Ring | undefined): void {
  if (key_ring === undefined || !is_regular_object(key_ring)) {
    throw new OpenKache_Error("key_ring must be a regular object")
  }
  if (
    !(key_ring.active instanceof Uint8Array) ||
    key_ring.active.byteLength !== SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES
  ) {
    throw new OpenKache_Error(
      `key_ring.active must contain exactly ${SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES} bytes`,
    )
  }
  const previous = key_ring.previous ?? []
  if (
    !Array.isArray(previous) ||
    previous.length > SMITHY_MAX_PREVIOUS_DATA_PROTECTION_KEYS
  ) {
    throw new OpenKache_Error(
      `key_ring.previous may contain at most ${SMITHY_MAX_PREVIOUS_DATA_PROTECTION_KEYS} keys`,
    )
  }
  for (const key of previous) {
    if (
      !(key instanceof Uint8Array) ||
      key.byteLength !== SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES
    ) {
      throw new OpenKache_Error(
        `key_ring.previous entries must contain exactly ${SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES} bytes`,
      )
    }
  }
}

function owned_active_key(options: Client_Options): Uint8Array {
  if (options.key_ring !== undefined) return options.key_ring.active.slice()
  if (options.data_protection_key !== undefined) return options.data_protection_key.slice()
  throw new OpenKache_Error("data protection key is missing")
}

function owned_previous_keys(options: Client_Options): readonly Uint8Array[] | undefined {
  return options.key_ring?.previous?.map((key): Uint8Array => key.slice())
}

function owned_mutation_id(value: Uint8Array | undefined): Uint8Array {
  if (value !== undefined) {
    validate_mutation_id(value, "mutation_id")
    return value.slice()
  }
  const mutation_id = new Uint8Array(SMITHY_MUTATION_ID_BYTES)
  const crypto = globalThis.crypto
  if (crypto === undefined || typeof crypto.getRandomValues !== "function") {
    throw new OpenKache_Error("secure random number generation is unavailable")
  }
  crypto.getRandomValues(mutation_id)
  return mutation_id
}

function normalize_metrics_snapshot(snapshot: Native_Metrics_Snapshot): Metrics_Snapshot {
  const fields: (keyof Metrics_Snapshot)[] = [
    "requests",
    "hits",
    "misses",
    "retries",
    "reconnects",
    "cancellations",
    "transport_errors",
    "protocol_errors",
    "bytes_sent",
    "bytes_received",
    "active_lanes",
  ]
  for (const field of fields) {
    const value = snapshot[field]
    if (!Number.isSafeInteger(value) || value < 0) {
      throw new OpenKache_Error(`native metrics field ${field} is invalid`)
    }
  }
  return { ...snapshot }
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
    case "created":
    case "replaced":
    case "not_stored":
      return value
    default:
      throw new OpenKache_Error(`SET returned unexpected native outcome ${value}`)
  }
}

function parse_connection_state(value: string): Connection_State {
  switch (value) {
    case "connected":
    case "reconnecting":
    case "disconnected":
    case "closed":
    case "unknown":
      return value
    default:
      throw new OpenKache_Error(
        `native client returned unexpected connection state ${value}`,
      )
  }
}

function as_openkache_error(error: unknown): OpenKache_Error {
  if (error instanceof OpenKache_Error) return error
  const native_envelope = parse_native_error(error)
  if (native_envelope !== undefined) {
    return new OpenKache_Error(
      native_envelope.message,
      error,
      native_envelope.metadata,
    )
  }
  const native_error = is_regular_object(error)
    ? (error as { readonly code?: unknown; readonly status?: unknown })
    : undefined
  if (native_error?.code === "Cancelled" || native_error?.status === "Cancelled") {
    return cancellation_error(error)
  }
  return new OpenKache_Error(error_message(error), error)
}

interface Native_Error_Envelope {
  readonly message: string
  readonly metadata?: Error_Metadata
}

function parse_native_error(error: unknown): Native_Error_Envelope | undefined {
  const message = error_message(error)
  let value: unknown
  try {
    value = JSON.parse(message) as unknown
  } catch {
    return undefined
  }
  if (!is_regular_object(value)) return undefined
  const candidate = value as Record<string, unknown>
  if (
    candidate.__openkache_native_error !== true ||
    typeof candidate.message !== "string"
  ) {
    return undefined
  }
  return {
    message: candidate.message,
    metadata: parse_error_metadata(candidate.metadata),
  }
}

function parse_error_metadata(value: unknown): Error_Metadata | undefined {
  if (!is_regular_object(value)) return undefined
  const candidate = value as Record<string, unknown>
  const code = candidate.code
  const operation = candidate.operation
  const phase = candidate.phase
  const backend = candidate.backend
  const retryable = candidate.retryable
  const ambiguous = candidate.ambiguous
  if (
    !is_safe_non_negative_integer(code) ||
    !is_safe_non_negative_integer(operation) ||
    !is_safe_non_negative_integer(phase) ||
    !is_safe_non_negative_integer(backend) ||
    typeof retryable !== "boolean" ||
    typeof ambiguous !== "boolean"
  ) {
    return undefined
  }
  let mutation_id: Uint8Array | undefined
  if (candidate.mutation_id !== null && candidate.mutation_id !== undefined) {
    if (
      !Array.isArray(candidate.mutation_id) ||
      !candidate.mutation_id.every(
        (byte): byte is number => is_safe_non_negative_integer(byte) && byte <= 255,
      )
    ) {
      return undefined
    }
    mutation_id = Uint8Array.from(candidate.mutation_id)
  }
  return {
    code,
    operation,
    phase,
    backend,
    retryable,
    ambiguous,
    mutation_id,
  }
}

function is_safe_non_negative_integer(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
}

function error_message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
