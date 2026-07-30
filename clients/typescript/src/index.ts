import {
  load_native_module,
  type Native_Client,
  type Native_Client_Options,
} from "./native-binding.js"
import {
  Value_Codec_Registry,
  type Value_Codec,
  type Value_Envelope,
} from "./value-codec.js"

export type {
  Encoded_Value,
  Value_Codec,
  Value_Envelope,
} from "./value-codec.js"

const MAX_VALUE_BYTES = 64 * 1024 * 1024
const ENCRYPTION_OVERHEAD_BYTES = 40
const MAX_PLAINTEXT_BYTES = MAX_VALUE_BYTES - ENCRYPTION_OVERHEAD_BYTES
const TEXT_ENCODER = new TextEncoder()

const CLIENT_FINALIZER = new FinalizationRegistry<Native_Client>(
  (native_client): void => {
    native_client.close()
  },
)

/**
 * Zstandard compression settings applied by the Rust value codec.
 */
export interface Zstandard_Options {
  /** Enables Zstandard compression before encryption. */
  readonly enabled?: boolean
  /** Zstandard compression level from 1 through 22. */
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
 * Connection settings for the Rust-backed Node.js, Bun, and Deno client.
 */
export interface Client_Options {
  /** Server UDP socket address, such as `127.0.0.1:4433`. */
  readonly address: string
  /** Server or CA certificate trusted for the QUIC connection, encoded as DER or PEM. */
  readonly certificate: Uint8Array
  /** Exact 32-byte master secret used to derive key-hiding and value-encryption subkeys. */
  readonly data_protection_key: Uint8Array
  /** TLS server name. Defaults to `localhost`. */
  readonly server_name?: string
  /** Client certificate and private key required by production mutual TLS. */
  readonly identity?: Client_Identity
  /** Client-side compression settings. */
  readonly compression?: Zstandard_Options
  /** Bounded connection and operation durations. */
  readonly timeouts?: Client_Timeouts
  /** Optional Protobuf, FlatBuffers, or application value codecs. */
  readonly value_codecs?: readonly Value_Codec[]
  /** Explicit Node-API adapter path, primarily for custom packaging. */
  readonly native_path?: string
}

/**
 * Outcome of a successful `set` operation.
 */
export type Set_Outcome = "created" | "replaced" | "not_stored"

/**
 * Optional TTL and atomic existence condition for `set`.
 */
export interface Set_Options {
  /** Store only when the key is absent (`if_absent`) or present (`if_present`). */
  readonly condition?: "if_absent" | "if_present"
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
  #close_promise: Promise<void> | undefined
  #closed = false

  private constructor(
    native_client: Native_Client,
    value_codecs: Value_Codec_Registry,
  ) {
    this.#native_client = native_client
    this.#value_codecs = value_codecs
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
    const native_options: Native_Client_Options = {
      address: options.address,
      server_name: options.server_name ?? "localhost",
      certificate: options.certificate.slice(),
      identity: owned_identity(options.identity),
      data_protection_key: options.data_protection_key.slice(),
      compression_enabled: compression.enabled !== false,
      compression_level: compression.level ?? 1,
      minimum_input_size: compression.minimum_input_size ?? 1_024,
      minimum_savings: compression.minimum_savings ?? 64,
      connect_timeout_ms: timeouts.connect_ms ?? 5_000,
      request_timeout_ms: timeouts.request_ms ?? 2_000,
    }
    try {
      const native_module = load_native_module(options.native_path)
      const native_client = await native_module.connect(native_options)
      const client = new OpenKache_Client(native_client, value_codecs)
      CLIENT_FINALIZER.register(client, native_client, client)
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
   * Retrieves and codec-decodes a regular JavaScript object.
   *
   * @typeParam Value - Expected object shape selected by the caller.
   * @param key - Exact non-empty string or binary cache key.
   * @returns The decoded object, or `undefined` when the key does not exist.
   * @throws {OpenKache_Error} When transport, decryption, or decoding fails.
   */
  async get<Value extends object = Record<string, unknown>>(
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
   * Codec-encodes and stores a regular JavaScript object.
   *
   * @typeParam Value - Object shape to store.
   * @param key - Exact non-empty string or binary cache key.
   * @param value - Plain object accepted by a registered codec or built-in JSON.
   * @param options - Optional TTL and `if_absent` or `if_present` condition.
   * @returns Whether the operation created, replaced, or did not store the key.
   * @throws {OpenKache_Error} When validation, encoding, transport, or storage fails.
   */
  async set<Value extends object>(
    key: string | Uint8Array,
    value: Value,
    options: Set_Options = {},
  ): Promise<Set_Outcome> {
    validate_set_options(options)
    let envelope: Value_Envelope
    try {
      envelope = this.#value_codecs.encode(value)
    } catch (error) {
      throw new OpenKache_Error(`value encoding failed: ${error_message(error)}`, error)
    }
    this.#assert_open()
    try {
      const outcome = await this.#native_client.set_value(
        owned_key_bytes(key),
        envelope.encoding,
        envelope.type_name,
        envelope.payload,
        options.condition,
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
    validate_set_options(options)
    validate_value_length(value)
    return this.#set_owned_bytes(key, value.slice(), options)
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
    this.#close_promise ??= this.#close_once()
    return this.#close_promise
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
        options.ttl_ms,
      )
      return parse_set_outcome(outcome)
    } catch (error) {
      throw as_openkache_error(error)
    }
  }

  #assert_open(): void {
    if (this.#closed || this.#close_promise !== undefined) {
      throw new OpenKache_Error("client is closed")
    }
  }

  async #close_once(): Promise<void> {
    try {
      this.#native_client.close()
    } catch (error) {
      throw as_openkache_error(error)
    } finally {
      this.#closed = true
      CLIENT_FINALIZER.unregister(this)
    }
  }
}

function owned_key_bytes(key: string | Uint8Array): Uint8Array {
  const bytes = typeof key === "string" ? TEXT_ENCODER.encode(key) : key.slice()
  if (bytes.byteLength === 0) {
    throw new OpenKache_Error("key must not be empty")
  }
  return bytes
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
  if (options.address.length === 0) throw new OpenKache_Error("address must not be empty")
  if (options.certificate.byteLength === 0) {
    throw new OpenKache_Error("certificate must not be empty")
  }
  if (options.data_protection_key.byteLength !== 32) {
    throw new OpenKache_Error(
      `data_protection_key must contain 32 bytes, got ${options.data_protection_key.byteLength}`,
    )
  }
  if (options.server_name !== undefined && options.server_name.length === 0) {
    throw new OpenKache_Error("server_name must not be empty")
  }
  if (options.native_path !== undefined && options.native_path.length === 0) {
    throw new OpenKache_Error("native_path must not be empty")
  }
  validate_identity(options.identity)
  validate_compression(options.compression)
  validate_timeout(options.timeouts?.connect_ms, "timeouts.connect_ms")
  validate_timeout(options.timeouts?.request_ms, "timeouts.request_ms")
}

function validate_identity(identity: Client_Identity | undefined): void {
  if (identity === undefined) return
  if (identity.certificate_chain.length === 0) {
    throw new OpenKache_Error("identity.certificate_chain must not be empty")
  }
  if (identity.certificate_chain.length > 0xffff) {
    throw new OpenKache_Error("identity.certificate_chain contains too many certificates")
  }
  for (const certificate of identity.certificate_chain) {
    if (certificate.byteLength === 0) {
      throw new OpenKache_Error("identity certificates must not be empty")
    }
  }
  if (identity.private_key.byteLength === 0) {
    throw new OpenKache_Error("identity.private_key must not be empty")
  }
}

function validate_compression(compression: Zstandard_Options | undefined): void {
  if (compression === undefined) return
  if (
    compression.level !== undefined &&
    (!Number.isInteger(compression.level) ||
      compression.level < 1 ||
      compression.level > 22)
  ) {
    throw new OpenKache_Error("compression.level must be an integer from 1 through 22")
  }
  for (const [name, value] of [
    ["minimum_input_size", compression.minimum_input_size],
    ["minimum_savings", compression.minimum_savings],
  ] as const) {
    if (value !== undefined && (!Number.isSafeInteger(value) || value < 0)) {
      throw new OpenKache_Error(`compression.${name} must be a non-negative safe integer`)
    }
  }
}

function validate_timeout(timeout_ms: number | undefined, name: string): void {
  if (
    timeout_ms !== undefined &&
    (!Number.isSafeInteger(timeout_ms) || timeout_ms <= 0)
  ) {
    throw new OpenKache_Error(`${name} must be a positive safe integer`)
  }
}

function validate_value_length(value: Uint8Array): void {
  if (value.byteLength > MAX_PLAINTEXT_BYTES) {
    throw new OpenKache_Error(
      `value contains ${value.byteLength} bytes, maximum is ${MAX_PLAINTEXT_BYTES}`,
    )
  }
}

function validate_set_options(options: Set_Options): void {
  if (
    options.condition !== undefined &&
    options.condition !== "if_absent" &&
    options.condition !== "if_present"
  ) {
    throw new OpenKache_Error(
      "condition must be if_absent or if_present",
    )
  }
  if (
    options.ttl_ms !== undefined &&
    (!Number.isSafeInteger(options.ttl_ms) || options.ttl_ms <= 0)
  ) {
    throw new OpenKache_Error("ttl_ms must be a positive safe integer")
  }
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

function as_openkache_error(error: unknown): OpenKache_Error {
  return error instanceof OpenKache_Error
    ? error
    : new OpenKache_Error(error_message(error), error)
}

function error_message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
