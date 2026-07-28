/**
 * Promise-based Bun client backed by the shared Rust OpenKache implementation.
 */

import {
  create,
  fromBinary,
  toBinary,
  type DescMessage,
  type MessageInitShape,
  type MessageShape,
} from "@bufbuild/protobuf"
import { join } from "node:path"
import { suffix } from "bun:ffi"
import {
  OPERATION_DELETE,
  OPERATION_GET,
  OPERATION_PING,
  OPERATION_SET,
  OPERATION_STATS,
  OPERATION_SYNC,
  RESULT_CONNECTED,
  RESULT_CREATED,
  RESULT_DELETED,
  RESULT_NOT_DELETED,
  RESULT_NOT_FOUND,
  RESULT_OK,
  RESULT_REPLACED,
  RESULT_VALUE,
  type Worker_Request_Body,
  type Worker_Response,
  type Worker_Success_Response,
} from "./worker-protocol.ts"

const EMPTY_BYTES = new Uint8Array()
const MAX_VALUE_BYTES = 16 * 1024 * 1024
const TEXT_ENCODER = new TextEncoder()
const TEXT_DECODER = new TextDecoder("utf-8", { fatal: true })

interface Pending_Request {
  readonly resolve: (response: Worker_Success_Response) => void
  readonly reject: (error: OpenKache_Error) => void
}

interface Worker_Channel {
  readonly pending: Map<number, Pending_Request>
  closed: boolean
}

const CLIENT_FINALIZER = new FinalizationRegistry<Worker>((worker): void => {
  try {
    worker.postMessage({ request_id: 0, kind: "close" })
  } catch {
    worker.terminate()
  }
})

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
 * Connection settings for the Rust-backed TypeScript client.
 */
export interface Client_Options {
  /** Server UDP socket address, such as `127.0.0.1:4433`. */
  readonly address: string
  /** DER certificate trusted for the QUIC connection. */
  readonly certificate: Uint8Array
  /** Exact 32-byte XChaCha20-Poly1305 key. */
  readonly encryption_key: Uint8Array
  /** TLS server name. */
  readonly server_name?: string
  /** Client-side compression settings. */
  readonly compression?: Zstandard_Options
  /** Explicit native library path, primarily for packaging. */
  readonly library_path?: string
}

/**
 * Outcome of a successful `set` operation.
 */
export type Set_Outcome = "created" | "replaced"

/**
 * Error returned by the shared Rust client implementation or Protobuf layer.
 */
export class OpenKache_Error extends Error {
  readonly kind = "openkache_error" as const
}

/**
 * Promise-based Bun client that delegates blocking native work to a dedicated worker.
 */
export class OpenKache_Client {
  readonly #worker: Worker
  readonly #channel: Worker_Channel
  #next_request_id = 1
  #close_promise: Promise<void> | undefined
  #closed = false

  private constructor(worker: Worker) {
    this.#worker = worker
    const channel: Worker_Channel = {
      pending: new Map(),
      closed: false,
    }
    this.#channel = channel
    worker.onmessage = (event: MessageEvent<Worker_Response>): void => {
      receive_worker_response(channel, event.data)
    }
    worker.onerror = (event: ErrorEvent): void => {
      channel.closed = true
      worker.terminate()
      fail_pending_requests(
        channel,
        new OpenKache_Error(event.message || "OpenKache native worker failed"),
      )
    }
  }

  /**
   * Connects without blocking the Bun main thread.
   *
   * @param options - Address, trust certificate, encryption key, and compression policy.
   * @returns A connected client that reuses one QUIC connection.
   * @throws {OpenKache_Error} When configuration, worker startup, or connection fails.
   */
  static async connect(options: Client_Options): Promise<OpenKache_Client> {
    validate_options(options)
    const worker = new Worker(new URL("./native-worker.ts", import.meta.url).href, {
      name: "openkache-client",
      smol: true,
    })
    const client = new OpenKache_Client(worker)
    const compression = options.compression ?? {}
    const certificate = options.certificate.slice()
    const encryption_key = options.encryption_key.slice()
    try {
      const response = await client.#request(
        {
          kind: "connect",
          options: {
            address: options.address,
            certificate,
            encryption_key,
            server_name: options.server_name ?? "localhost",
            compression_enabled: compression.enabled !== false,
            compression_level: compression.level ?? 1,
            minimum_input_size: compression.minimum_input_size ?? 1_024,
            minimum_savings: compression.minimum_savings ?? 64,
            library_path: options.library_path ?? default_library_path(),
          },
        },
        [certificate.buffer, encryption_key.buffer],
      )
      if (response.result_kind !== RESULT_CONNECTED) {
        throw unexpected_result("connect", response.result_kind)
      }
      CLIENT_FINALIZER.register(client, worker, client)
      return client
    } catch (error) {
      worker.terminate()
      throw as_openkache_error(error)
    }
  }

  /**
   * Verifies that the server is reachable and speaks the expected protocol.
   */
  async ping(): Promise<void> {
    await this.#expect_kind(OPERATION_PING, EMPTY_BYTES, RESULT_OK)
  }

  /**
   * Retrieves, decrypts, decompresses, and Protobuf-decodes a value.
   *
   * @typeParam Schema - Generated Protobuf message schema type.
   * @param key - Exact string or binary cache key.
   * @param schema - Generated Protobuf schema used for runtime decoding.
   * @returns The decoded value, or `undefined` when the key does not exist.
   */
  async get<Schema extends DescMessage>(
    key: string | Uint8Array,
    schema: Schema,
  ): Promise<MessageShape<Schema> | undefined> {
    const bytes = await this.getRaw(key)
    if (bytes === undefined) return undefined
    try {
      return fromBinary(schema, bytes)
    } catch (error) {
      throw new OpenKache_Error(`Protobuf decoding failed: ${error_message(error)}`)
    }
  }

  /**
   * Protobuf-encodes, compresses, encrypts, and stores a value.
   *
   * @typeParam Schema - Generated Protobuf message schema type.
   * @param key - Exact string or binary cache key.
   * @param value - Message or initializer accepted by the generated schema.
   * @param schema - Generated Protobuf schema used for runtime encoding.
   * @returns Whether the operation created or replaced the key.
   */
  async set<Schema extends DescMessage>(
    key: string | Uint8Array,
    value: MessageInitShape<Schema>,
    schema: Schema,
  ): Promise<Set_Outcome> {
    let bytes: Uint8Array
    try {
      bytes = toBinary(schema, create(schema, value))
    } catch (error) {
      throw new OpenKache_Error(`Protobuf encoding failed: ${error_message(error)}`)
    }
    validate_value_length(bytes)
    return this.#set_owned_bytes(key, bytes)
  }

  /**
   * Retrieves exact decrypted and decompressed bytes without Protobuf decoding.
   */
  async getRaw(key: string | Uint8Array): Promise<Uint8Array | undefined> {
    const response = await this.#execute(OPERATION_GET, key, EMPTY_BYTES)
    if (response.result_kind === RESULT_NOT_FOUND) return undefined
    if (response.result_kind !== RESULT_VALUE) {
      throw unexpected_result("GET", response.result_kind)
    }
    return response.payload
  }

  /**
   * Stores exact bytes without Protobuf encoding.
   */
  async setRaw(
    key: string | Uint8Array,
    value: Uint8Array,
  ): Promise<Set_Outcome> {
    validate_value_length(value)
    return this.#set_owned_bytes(key, value.slice())
  }

  /**
   * Deletes a key.
   */
  async delete(key: string | Uint8Array): Promise<boolean> {
    const response = await this.#execute(OPERATION_DELETE, key, EMPTY_BYTES)
    if (response.result_kind === RESULT_DELETED) return true
    if (response.result_kind === RESULT_NOT_DELETED) return false
    throw unexpected_result("DELETE", response.result_kind)
  }

  /**
   * Retrieves server statistics as JSON text.
   */
  async stats(): Promise<string> {
    const response = await this.#execute(OPERATION_STATS, EMPTY_BYTES, EMPTY_BYTES)
    if (response.result_kind !== RESULT_VALUE) {
      throw unexpected_result("STATS", response.result_kind)
    }
    return TEXT_DECODER.decode(response.payload)
  }

  /**
   * Requests a server durability barrier.
   */
  async sync(): Promise<void> {
    await this.#expect_kind(OPERATION_SYNC, EMPTY_BYTES, RESULT_OK)
  }

  /**
   * Closes the native connection and worker. Repeated calls are safe.
   */
  close(): Promise<void> {
    this.#close_promise ??= this.#close_once()
    return this.#close_promise
  }

  async #set_owned_bytes(
    key: string | Uint8Array,
    bytes: Uint8Array,
  ): Promise<Set_Outcome> {
    const response = await this.#execute(OPERATION_SET, key, bytes)
    const outcomes: Readonly<Record<number, Set_Outcome | undefined>> = {
      [RESULT_CREATED]: "created",
      [RESULT_REPLACED]: "replaced",
    }
    const outcome = outcomes[response.result_kind]
    if (outcome === undefined) throw unexpected_result("SET", response.result_kind)
    return outcome
  }

  async #expect_kind(
    operation: number,
    key: Uint8Array,
    expected_kind: number,
  ): Promise<void> {
    const response = await this.#execute(operation, key, EMPTY_BYTES)
    if (response.result_kind !== expected_kind) {
      throw unexpected_result("operation", response.result_kind)
    }
  }

  #execute(
    operation: number,
    key: string | Uint8Array,
    value: Uint8Array,
  ): Promise<Worker_Success_Response> {
    const key_bytes = owned_key_bytes(key)
    const owned_value = value.byteLength === 0 ? value : owned_bytes(value)
    const transfer: Transferable[] = [key_bytes.buffer as ArrayBuffer]
    if (owned_value.byteLength > 0) {
      transfer.push(owned_value.buffer as ArrayBuffer)
    }
    return this.#request(
      {
        kind: "execute",
        operation,
        key: key_bytes,
        value: owned_value,
      },
      transfer,
    )
  }

  #request(
    request: Worker_Request_Body,
    transfer: Transferable[] = [],
    allow_closing = false,
  ): Promise<Worker_Success_Response> {
    if (
      this.#closed ||
      this.#channel.closed ||
      (!allow_closing && this.#close_promise !== undefined)
    ) {
      return Promise.reject(new OpenKache_Error("client is closed"))
    }
    const request_id = this.#next_request_id
    this.#next_request_id += 1
    return new Promise<Worker_Success_Response>((resolve, reject): void => {
      this.#channel.pending.set(request_id, { resolve, reject })
      try {
        this.#worker.postMessage({ ...request, request_id }, transfer)
      } catch (error) {
        this.#channel.pending.delete(request_id)
        reject(as_openkache_error(error))
      }
    })
  }

  async #close_once(): Promise<void> {
    try {
      const response = await this.#request({ kind: "close" }, [], true)
      if (response.result_kind !== RESULT_OK) {
        throw unexpected_result("close", response.result_kind)
      }
    } finally {
      this.#closed = true
      this.#channel.closed = true
      CLIENT_FINALIZER.unregister(this)
      this.#worker.terminate()
      fail_pending_requests(this.#channel, new OpenKache_Error("client is closed"))
    }
  }
}

function receive_worker_response(
  channel: Worker_Channel,
  response: Worker_Response,
): void {
  const pending = channel.pending.get(response.request_id)
  if (pending === undefined) return
  channel.pending.delete(response.request_id)
  if (response.ok) {
    pending.resolve(response)
  } else {
    pending.reject(new OpenKache_Error(response.message))
  }
}

function fail_pending_requests(
  channel: Worker_Channel,
  error: OpenKache_Error,
): void {
  for (const pending of channel.pending.values()) {
    pending.reject(error)
  }
  channel.pending.clear()
}

function owned_key_bytes(key: string | Uint8Array): Uint8Array {
  return typeof key === "string" ? TEXT_ENCODER.encode(key) : key.slice()
}

function owned_bytes(bytes: Uint8Array): Uint8Array {
  if (
    bytes.byteOffset === 0 &&
    bytes.buffer instanceof ArrayBuffer &&
    bytes.byteLength === bytes.buffer.byteLength
  ) {
    return bytes
  }
  return bytes.slice()
}

function validate_options(options: Client_Options): void {
  if (options.address.length === 0) throw new OpenKache_Error("address must not be empty")
  if (options.certificate.byteLength === 0) {
    throw new OpenKache_Error("certificate must not be empty")
  }
  if (options.encryption_key.byteLength !== 32) {
    throw new OpenKache_Error(
      `encryption_key must contain 32 bytes, got ${options.encryption_key.byteLength}`,
    )
  }
}

function validate_value_length(value: Uint8Array): void {
  if (value.byteLength > MAX_VALUE_BYTES) {
    throw new OpenKache_Error(
      `value contains ${value.byteLength} bytes, maximum is ${MAX_VALUE_BYTES}`,
    )
  }
}

function unexpected_result(operation: string, kind: number): OpenKache_Error {
  return new OpenKache_Error(`${operation} returned unexpected native result ${kind}`)
}

function as_openkache_error(error: unknown): OpenKache_Error {
  return error instanceof OpenKache_Error
    ? error
    : new OpenKache_Error(error_message(error))
}

function error_message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function default_library_path(): string {
  return join(
    import.meta.dir,
    "..",
    "target",
    "native",
    "release",
    native_library_name(suffix),
  )
}

function native_library_name(extension: string): string {
  switch (extension) {
    case "dll":
      return "openkache_client.dll"
    case "dylib":
      return "libopenkache_client.dylib"
    case "so":
      return "libopenkache_client.so"
    default:
      throw new OpenKache_Error(`unsupported native library suffix: ${extension}`)
  }
}
