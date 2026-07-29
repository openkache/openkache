/**
 * Bun worker that owns the synchronous native FFI handle.
 */

import { dlopen, FFIType, type Library, type Pointer, toArrayBuffer } from "bun:ffi"
import {
  RESULT_CONNECTED,
  RESULT_ERROR,
  RESULT_OK,
  type Worker_Connection_Options,
  type Worker_Request,
  type Worker_Response,
  type Worker_Success_Response,
} from "./worker-protocol.ts"

declare const self: Worker & { close(): void }

const ABI_VERSION = 2
const EMPTY_BYTES = new Uint8Array()
const TEXT_ENCODER = new TextEncoder()
const TEXT_DECODER = new TextDecoder("utf-8", { fatal: true })

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
      FFIType.u32,
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

let native_library: Native_Library | undefined
let native_client: Native_Pointer | undefined

self.onmessage = (event: MessageEvent<Worker_Request>): void => {
  const request = event.data
  let response: Worker_Response
  try {
    response = handle_request(request)
  } catch (error) {
    response = {
      request_id: request.request_id,
      ok: false,
      message: error_message(error),
    }
  }

  if (response.ok && response.payload.byteLength > 0) {
    self.postMessage(response, [response.payload.buffer as ArrayBuffer])
  } else {
    self.postMessage(response)
  }
  if (request.kind === "close") {
    setTimeout((): void => {
      self.close()
    }, 0)
  }
}

function handle_request(request: Worker_Request): Worker_Success_Response {
  switch (request.kind) {
    case "connect":
      connect(request.options)
      return success(request.request_id, RESULT_CONNECTED)
    case "execute":
      return execute(request)
    case "close":
      close_native_client()
      return success(request.request_id, RESULT_OK)
  }
}

function connect(options: Worker_Connection_Options): void {
  if (native_client !== undefined) {
    throw new Error("native client is already connected")
  }
  const library = dlopen(options.library_path, NATIVE_SYMBOLS)
  if (library.symbols.openkache_client_abi_version() !== ABI_VERSION) {
    library.close()
    throw new Error(`native library at ${options.library_path} has an incompatible ABI`)
  }

  const address = TEXT_ENCODER.encode(options.address)
  const server_name = TEXT_ENCODER.encode(options.server_name)
  const result = library.symbols.openkache_client_connect(
    address,
    address.byteLength,
    server_name,
    server_name.byteLength,
    options.certificate,
    options.certificate.byteLength,
    options.encryption_key,
    options.encryption_key.byteLength,
    options.compression_enabled ? 1 : 0,
    options.compression_level,
    options.minimum_input_size,
    options.minimum_savings,
  )
  if (result === null) {
    library.close()
    throw new Error("Rust client returned a null connection result")
  }
  try {
    const result_kind = library.symbols.openkache_client_result_kind(result)
    if (result_kind !== RESULT_CONNECTED) {
      throw result_error(library.symbols, result, result_kind)
    }
    const connected_client = library.symbols.openkache_client_result_take_client(result)
    if (connected_client === null) {
      throw new Error("Rust client returned a null client handle")
    }
    native_library = library
    native_client = connected_client
  } catch (error) {
    library.close()
    throw error
  } finally {
    library.symbols.openkache_client_result_free(result)
  }
}

function execute(
  request: Extract<Worker_Request, { readonly kind: "execute" }>,
): Worker_Success_Response {
  const client = native_client
  const library = native_library
  if (client === undefined || library === undefined) {
    throw new Error("native client is not connected")
  }
  const result = library.symbols.openkache_client_execute(
    client,
    request.operation,
    request.key,
    request.key.byteLength,
    request.value,
    request.value.byteLength,
    request.set_condition,
    request.ttl_ms,
  )
  if (result === null) {
    throw new Error("Rust client returned a null operation result")
  }
  try {
    const result_kind = library.symbols.openkache_client_result_kind(result)
    if (result_kind === RESULT_ERROR) {
      throw result_error(library.symbols, result, result_kind)
    }
    return success(
      request.request_id,
      result_kind,
      copy_result_payload(library.symbols, result),
    )
  } finally {
    library.symbols.openkache_client_result_free(result)
  }
}

function close_native_client(): void {
  const client = native_client
  const library = native_library
  native_client = undefined
  native_library = undefined
  if (client !== undefined && library !== undefined) {
    library.symbols.openkache_client_free(client)
  }
  library?.close()
}

function success(
  request_id: number,
  result_kind: number,
  payload: Uint8Array = EMPTY_BYTES,
): Worker_Success_Response {
  return {
    request_id,
    ok: true,
    result_kind,
    payload,
  }
}

function copy_result_payload(symbols: Native_Symbols, result: Native_Pointer): Uint8Array {
  const length = Number(symbols.openkache_client_result_data_length(result))
  if (length === 0) return EMPTY_BYTES
  const data = symbols.openkache_client_result_data(result)
  if (data === null) {
    throw new Error(`Rust client returned a null pointer for ${length} payload bytes`)
  }
  return new Uint8Array(toArrayBuffer(data, 0, length)).slice()
}

function result_error(
  symbols: Native_Symbols,
  result: Native_Pointer,
  kind: number,
): Error {
  const payload = copy_result_payload(symbols, result)
  const message =
    payload.byteLength === 0
      ? `native operation failed with result ${kind}`
      : TEXT_DECODER.decode(payload)
  return new Error(message)
}

function error_message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
