/**
 * Bun client backed by the shared Rust OpenKache implementation.
 */

import { dlopen, FFIType, type Library, type Pointer, suffix, toArrayBuffer } from "bun:ffi"
import { join } from "node:path"

const ABI_VERSION = 1
const EMPTY_BYTES = new Uint8Array()
const TEXT_ENCODER = new TextEncoder()
const TEXT_DECODER = new TextDecoder("utf-8", { fatal: true })

const RESULT_ERROR = 0
const RESULT_OK = 1
const RESULT_VALUE = 2
const RESULT_NOT_FOUND = 3
const RESULT_CREATED = 4
const RESULT_REPLACED = 5
const RESULT_DELETED = 6
const RESULT_NOT_DELETED = 7
const RESULT_CONNECTED = 8

const OPERATION_PING = 1
const OPERATION_GET = 2
const OPERATION_SET = 3
const OPERATION_DELETE = 4
const OPERATION_STATS = 5
const OPERATION_SYNC = 6

const NATIVE_SYMBOLS = {
  openkache_client_abi_version: {
    args: [],
    returns: FFIType.u32,
  },
  openkache_client_connect: {
    args: [
      FFIType.ptr,
      FFIType.u64_fast,
      FFIType.ptr,
      FFIType.u64_fast,
      FFIType.ptr,
      FFIType.u64_fast,
      FFIType.ptr,
      FFIType.u64_fast,
      FFIType.u8,
      FFIType.i32,
      FFIType.u64_fast,
      FFIType.u64_fast,
    ],
    returns: FFIType.ptr,
  },
  openkache_client_execute: {
    args: [
      FFIType.ptr,
      FFIType.u32,
      FFIType.ptr,
      FFIType.u64_fast,
      FFIType.ptr,
      FFIType.u64_fast,
    ],
    returns: FFIType.ptr,
  },
  openkache_client_result_kind: {
    args: [FFIType.ptr],
    returns: FFIType.u32,
  },
  openkache_client_result_data: {
    args: [FFIType.ptr],
    returns: FFIType.ptr,
  },
  openkache_client_result_data_length: {
    args: [FFIType.ptr],
    returns: FFIType.u64_fast,
  },
  openkache_client_result_take_client: {
    args: [FFIType.ptr],
    returns: FFIType.ptr,
  },
  openkache_client_result_free: {
    args: [FFIType.ptr],
    returns: FFIType.void,
  },
  openkache_client_free: {
    args: [FFIType.ptr],
    returns: FFIType.void,
  },
} as const

type Native_Library = Library<typeof NATIVE_SYMBOLS>
type Native_Symbols = Native_Library["symbols"]
type Native_Pointer = Pointer

interface Finalizer_State {
  readonly client: Native_Pointer
  readonly symbols: Native_Symbols
}

const LIBRARIES = new Map<string, Native_Library>()
const CLIENT_FINALIZER = new FinalizationRegistry<Finalizer_State>(({ client, symbols }) => {
  symbols.openkache_client_free(client)
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
 * Error returned by the shared Rust client implementation.
 */
export class OpenKache_Error extends Error {
  readonly kind = "openkache_error" as const
}

/**
 * Synchronous Bun client that delegates protocol, compression, encryption, and QUIC I/O to Rust.
 */
export class OpenKache_Client {
  readonly #symbols: Native_Symbols
  #native_client: Native_Pointer | undefined

  private constructor(symbols: Native_Symbols, native_client: Native_Pointer) {
    this.#symbols = symbols
    this.#native_client = native_client
    CLIENT_FINALIZER.register(this, { client: native_client, symbols }, this)
  }

  /**
   * Connects to an OpenKache server through the shared Rust client.
   *
   * @param options - Address, trust certificate, encryption key, and compression policy.
   * @returns A connected client that reuses one QUIC connection.
   * @throws {OpenKache_Error} When configuration, TLS, runtime startup, or connection fails.
   */
  static connect(options: Client_Options): OpenKache_Client {
    validate_options(options)
    const library_path = options.library_path ?? default_library_path()
    const { symbols } = load_native_library(library_path)
    const address = TEXT_ENCODER.encode(options.address)
    const server_name = TEXT_ENCODER.encode(options.server_name ?? "localhost")
    const compression = options.compression ?? {}
    const result = symbols.openkache_client_connect(
      address,
      address.byteLength,
      server_name,
      server_name.byteLength,
      options.certificate,
      options.certificate.byteLength,
      options.encryption_key,
      options.encryption_key.byteLength,
      compression.enabled === false ? 0 : 1,
      compression.level ?? 1,
      compression.minimum_input_size ?? 1_024,
      compression.minimum_savings ?? 64,
    )
    if (result === null) {
      throw new OpenKache_Error("Rust client returned a null connection result")
    }
    try {
      const result_kind = symbols.openkache_client_result_kind(result)
      if (result_kind !== RESULT_CONNECTED) {
        throw result_error(symbols, result, result_kind)
      }
      const native_client = symbols.openkache_client_result_take_client(result)
      if (native_client === null) {
        throw new OpenKache_Error("Rust client returned a null client handle")
      }
      return new OpenKache_Client(symbols, native_client)
    } finally {
      symbols.openkache_client_result_free(result)
    }
  }

  /**
   * Verifies that the server is reachable and speaks the expected protocol.
   *
   * @returns Nothing after a successful PING/PONG exchange.
   * @throws {OpenKache_Error} When the operation fails.
   */
  ping(): void {
    this.#expect_kind(OPERATION_PING, EMPTY_BYTES, EMPTY_BYTES, RESULT_OK)
  }

  /**
   * Retrieves and decrypts a value.
   *
   * @param key - Exact string or binary cache key.
   * @returns The application value, or `undefined` when the key does not exist.
   * @throws {OpenKache_Error} When transport, authentication, or decompression fails.
   */
  get(key: string | Uint8Array): Uint8Array | undefined {
    const result = this.#execute(OPERATION_GET, as_bytes(key), EMPTY_BYTES)
    if (result.kind === RESULT_NOT_FOUND) return undefined
    if (result.kind !== RESULT_VALUE) throw unexpected_result("GET", result.kind)
    return result.payload
  }

  /**
   * Compresses, encrypts, and stores a value.
   *
   * @param key - Exact string or binary cache key.
   * @param value - Exact string or binary application value.
   * @returns Whether the operation created or replaced the key.
   * @throws {OpenKache_Error} When transformation, transport, or server execution fails.
   */
  set(key: string | Uint8Array, value: string | Uint8Array): Set_Outcome {
    const result = this.#execute(OPERATION_SET, as_bytes(key), as_bytes(value))
    const outcomes: Readonly<Record<number, Set_Outcome | undefined>> = {
      [RESULT_CREATED]: "created",
      [RESULT_REPLACED]: "replaced",
    }
    const outcome = outcomes[result.kind]
    if (outcome === undefined) throw unexpected_result("SET", result.kind)
    return outcome
  }

  /**
   * Deletes a key.
   *
   * @param key - Exact string or binary cache key.
   * @returns Whether the key existed.
   * @throws {OpenKache_Error} When the operation fails.
   */
  delete(key: string | Uint8Array): boolean {
    const result = this.#execute(OPERATION_DELETE, as_bytes(key), EMPTY_BYTES)
    if (result.kind === RESULT_DELETED) return true
    if (result.kind === RESULT_NOT_DELETED) return false
    throw unexpected_result("DELETE", result.kind)
  }

  /**
   * Retrieves server statistics.
   *
   * @returns The server JSON payload as text.
   * @throws {OpenKache_Error} When the operation fails or returns invalid UTF-8.
   */
  stats(): string {
    const result = this.#execute(OPERATION_STATS, EMPTY_BYTES, EMPTY_BYTES)
    if (result.kind !== RESULT_VALUE) throw unexpected_result("STATS", result.kind)
    return TEXT_DECODER.decode(result.payload)
  }

  /**
   * Requests a server durability barrier.
   *
   * @returns Nothing after the server acknowledges the barrier.
   * @throws {OpenKache_Error} When the operation fails.
   */
  sync(): void {
    this.#expect_kind(OPERATION_SYNC, EMPTY_BYTES, EMPTY_BYTES, RESULT_OK)
  }

  /**
   * Closes the native connection and worker thread.
   *
   * @returns Nothing. Repeated calls are safe.
   */
  close(): void {
    const native_client = this.#native_client
    if (native_client === undefined) return
    this.#native_client = undefined
    CLIENT_FINALIZER.unregister(this)
    this.#symbols.openkache_client_free(native_client)
  }

  #expect_kind(
    operation: number,
    key: Uint8Array,
    value: Uint8Array,
    expected_kind: number,
  ): void {
    const result = this.#execute(operation, key, value)
    if (result.kind !== expected_kind) throw unexpected_result("operation", result.kind)
  }

  #execute(
    operation: number,
    key: Uint8Array,
    value: Uint8Array,
  ): { readonly kind: number; readonly payload: Uint8Array } {
    const native_client = this.#native_client
    if (native_client === undefined) {
      throw new OpenKache_Error("client is closed")
    }
    const result = this.#symbols.openkache_client_execute(
      native_client,
      operation,
      key,
      key.byteLength,
      value,
      value.byteLength,
    )
    if (result === null) {
      throw new OpenKache_Error("Rust client returned a null operation result")
    }
    try {
      const kind = this.#symbols.openkache_client_result_kind(result)
      if (kind === RESULT_ERROR) {
        throw result_error(this.#symbols, result, kind)
      }
      return {
        kind,
        payload: copy_result_payload(this.#symbols, result),
      }
    } finally {
      this.#symbols.openkache_client_result_free(result)
    }
  }
}

function load_native_library(path: string): Native_Library {
  const cached = LIBRARIES.get(path)
  if (cached !== undefined) return cached
  const library = dlopen(path, NATIVE_SYMBOLS)
  if (library.symbols.openkache_client_abi_version() !== ABI_VERSION) {
    library.close()
    throw new OpenKache_Error(`native library at ${path} has an incompatible ABI`)
  }
  LIBRARIES.set(path, library)
  return library
}

function copy_result_payload(symbols: Native_Symbols, result: Native_Pointer): Uint8Array {
  const length = Number(symbols.openkache_client_result_data_length(result))
  if (length === 0) return EMPTY_BYTES
  const data = symbols.openkache_client_result_data(result)
  if (data === null) {
    throw new OpenKache_Error(`Rust client returned a null pointer for ${length} payload bytes`)
  }
  return new Uint8Array(toArrayBuffer(data, 0, length)).slice()
}

function result_error(
  symbols: Native_Symbols,
  result: Native_Pointer,
  kind: number,
): OpenKache_Error {
  const payload = copy_result_payload(symbols, result)
  const message = payload.byteLength === 0 ? `native operation failed with result ${kind}` : TEXT_DECODER.decode(payload)
  return new OpenKache_Error(message)
}

function unexpected_result(operation: string, kind: number): OpenKache_Error {
  return new OpenKache_Error(`${operation} returned unexpected native result ${kind}`)
}

function as_bytes(value: string | Uint8Array): Uint8Array {
  if (typeof value === "string") return TEXT_ENCODER.encode(value)
  return value
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
