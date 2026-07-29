/**
 * Length-prefixed IPC shared by the Node.js client and Rust transport helper.
 */

export const OPERATION_PING = 1
export const OPERATION_GET = 2
export const OPERATION_SET = 3
export const OPERATION_DELETE = 4
export const OPERATION_STATS = 5
export const OPERATION_SYNC = 6

export const RESULT_OK = 1
export const RESULT_VALUE = 2
export const RESULT_NOT_FOUND = 3
export const RESULT_CREATED = 4
export const RESULT_REPLACED = 5
export const RESULT_DELETED = 6
export const RESULT_NOT_DELETED = 7
export const RESULT_CONNECTED = 8
export const RESULT_NOT_STORED = 9

const COMMAND_CONNECT = 1
const COMMAND_EXECUTE = 2
const COMMAND_CLOSE = 3
const MAX_HELPER_FRAME_BYTES = 32 * 1024 * 1024
const TEXT_ENCODER = new TextEncoder()
const TEXT_DECODER = new TextDecoder("utf-8", { fatal: true })

export interface Helper_Identity {
  readonly certificate_chain: readonly Uint8Array[]
  readonly private_key: Uint8Array
}

export interface Helper_Connection_Options {
  readonly address: string
  readonly server_name: string
  readonly certificate: Uint8Array
  readonly identity?: Helper_Identity
  readonly encryption_key: Uint8Array
  readonly compression_enabled: boolean
  readonly compression_level: number
  readonly minimum_input_size: number
  readonly minimum_savings: number
}

export interface Helper_Execute_Request {
  readonly operation: number
  readonly condition: 0 | 1 | 2
  readonly ttl_ms: number
  readonly key: Uint8Array
  readonly value: Uint8Array
}

export interface Helper_Response {
  readonly request_id: number
  readonly ok: boolean
  readonly result_kind: number
  readonly payload: Uint8Array
}

export function encode_connect_request(
  request_id: number,
  options: Helper_Connection_Options,
): Uint8Array {
  const encoder = new Frame_Encoder(request_id, COMMAND_CONNECT)
  encoder.string(options.address)
  encoder.string(options.server_name)
  encoder.bytes(options.certificate)
  const identity = options.identity
  if (identity === undefined) {
    encoder.u16(0)
    encoder.bytes(new Uint8Array())
  } else {
    encoder.u16(identity.certificate_chain.length)
    for (const certificate of identity.certificate_chain) {
      encoder.bytes(certificate)
    }
    encoder.bytes(identity.private_key)
  }
  encoder.bytes(options.encryption_key)
  encoder.u8(options.compression_enabled ? 1 : 0)
  encoder.i32(options.compression_level)
  encoder.u64(options.minimum_input_size)
  encoder.u64(options.minimum_savings)
  return encoder.finish()
}

export function encode_execute_request(
  request_id: number,
  request: Helper_Execute_Request,
): Uint8Array {
  const encoder = new Frame_Encoder(request_id, COMMAND_EXECUTE)
  encoder.u8(request.operation)
  encoder.u8(request.condition)
  encoder.u64(request.ttl_ms)
  encoder.bytes(request.key)
  encoder.bytes(request.value)
  return encoder.finish()
}

export function encode_close_request(request_id: number): Uint8Array {
  return new Frame_Encoder(request_id, COMMAND_CLOSE).finish()
}

export class Helper_Response_Decoder {
  #buffer: Uint8Array<ArrayBufferLike> = new Uint8Array()

  push(chunk: Uint8Array): readonly Helper_Response[] {
    this.#buffer = concatenate(this.#buffer, chunk)
    const responses: Helper_Response[] = []
    let offset = 0
    while (this.#buffer.byteLength - offset >= 4) {
      const view = new DataView(
        this.#buffer.buffer,
        this.#buffer.byteOffset + offset,
        this.#buffer.byteLength - offset,
      )
      const frame_length = view.getUint32(0)
      if (frame_length > MAX_HELPER_FRAME_BYTES) {
        throw new Error(
          `helper response contains ${frame_length} bytes, maximum is ${MAX_HELPER_FRAME_BYTES}`,
        )
      }
      const encoded_length = 4 + frame_length
      if (this.#buffer.byteLength - offset < encoded_length) break
      responses.push(
        decode_response(this.#buffer.subarray(offset + 4, offset + encoded_length)),
      )
      offset += encoded_length
    }
    if (offset > 0) this.#buffer = this.#buffer.slice(offset)
    return responses
  }

  finish(): void {
    if (this.#buffer.byteLength !== 0) {
      throw new Error(`helper response ended with ${this.#buffer.byteLength} truncated bytes`)
    }
  }
}

function decode_response(frame: Uint8Array): Helper_Response {
  if (frame.byteLength < 6) {
    throw new Error(`helper response requires at least 6 bytes, got ${frame.byteLength}`)
  }
  const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength)
  const ok_byte = view.getUint8(4)
  if (ok_byte !== 0 && ok_byte !== 1) {
    throw new Error(`helper response contains invalid success discriminator ${ok_byte}`)
  }
  const payload = new Uint8Array(frame.byteLength - 6)
  payload.set(frame.subarray(6))
  return {
    request_id: view.getUint32(0),
    ok: ok_byte === 1,
    result_kind: view.getUint8(5),
    payload,
  }
}

export function decode_helper_error(payload: Uint8Array): string {
  return TEXT_DECODER.decode(payload)
}

class Frame_Encoder {
  readonly #parts: Uint8Array[] = []
  #length = 0

  constructor(request_id: number, command: number) {
    this.u32(request_id)
    this.u8(command)
  }

  u8(value: number): void {
    const bytes = new Uint8Array(1)
    new DataView(bytes.buffer).setUint8(0, value)
    this.add(bytes)
  }

  u16(value: number): void {
    if (!Number.isSafeInteger(value) || value < 0 || value > 0xffff) {
      throw new Error(`helper unsigned 16-bit value is invalid: ${value}`)
    }
    const bytes = new Uint8Array(2)
    new DataView(bytes.buffer).setUint16(0, value)
    this.add(bytes)
  }

  u32(value: number): void {
    if (!Number.isSafeInteger(value) || value < 0 || value > 0xffff_ffff) {
      throw new Error(`helper unsigned 32-bit value is invalid: ${value}`)
    }
    const bytes = new Uint8Array(4)
    new DataView(bytes.buffer).setUint32(0, value)
    this.add(bytes)
  }

  i32(value: number): void {
    if (!Number.isSafeInteger(value) || value < -0x8000_0000 || value > 0x7fff_ffff) {
      throw new Error(`helper signed 32-bit value is invalid: ${value}`)
    }
    const bytes = new Uint8Array(4)
    new DataView(bytes.buffer).setInt32(0, value)
    this.add(bytes)
  }

  u64(value: number): void {
    if (!Number.isSafeInteger(value) || value < 0) {
      throw new Error(`helper unsigned 64-bit value is invalid: ${value}`)
    }
    const bytes = new Uint8Array(8)
    new DataView(bytes.buffer).setBigUint64(0, BigInt(value))
    this.add(bytes)
  }

  bytes(value: Uint8Array): void {
    this.u32(value.byteLength)
    this.add(value)
  }

  string(value: string): void {
    this.bytes(TEXT_ENCODER.encode(value))
  }

  finish(): Uint8Array {
    if (this.#length > MAX_HELPER_FRAME_BYTES) {
      throw new Error(
        `helper request contains ${this.#length} bytes, maximum is ${MAX_HELPER_FRAME_BYTES}`,
      )
    }
    const frame = new Uint8Array(4 + this.#length)
    new DataView(frame.buffer).setUint32(0, this.#length)
    let offset = 4
    for (const part of this.#parts) {
      frame.set(part, offset)
      offset += part.byteLength
    }
    return frame
  }

  add(bytes: Uint8Array): void {
    this.#parts.push(bytes)
    this.#length += bytes.byteLength
  }
}

function concatenate(left: Uint8Array, right: Uint8Array): Uint8Array {
  if (left.byteLength === 0) return right.slice()
  if (right.byteLength === 0) return left
  const combined = new Uint8Array(left.byteLength + right.byteLength)
  combined.set(left)
  combined.set(right, left.byteLength)
  return combined
}
