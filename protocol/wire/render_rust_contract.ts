/** Rust rendering for the transport-only wire contract. */

import type { Wire_Contract } from "../wire_types"

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function rust_byte_string_literal(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let literal = 'b"'
  for (const byte of bytes) {
    if (byte >= 0x20 && byte <= 0x7e && byte !== 0x22 && byte !== 0x5c) {
      literal += String.fromCharCode(byte)
    } else {
      literal += `\\x${byte.toString(16).padStart(2, "0")}`
    }
  }
  return `${literal}"`
}

/** Renders only transport constants; API codes remain opaque to this crate. */
export function render_rust_wire(contract: Wire_Contract): string {
  const v1 = contract.v1
  return `// Generated from the OpenKache Smithy transport contract. Do not edit.

/// QUIC application protocol identifier.
pub const ALPN: &[u8] = ${rust_byte_string_literal(v1.alpn)};
/// Bytes occupied by the opaque request code.
pub const REQUEST_CODE_BYTES: usize = ${formatted_decimal(v1.request_code_bytes)};
/// Bytes occupied by the opaque response code.
pub const RESPONSE_CODE_BYTES: usize = ${formatted_decimal(v1.response_code_bytes)};
/// Minimum bytes in one canonical unsigned vu128.
pub const MIN_VARUINT_BYTES: usize = ${formatted_decimal(v1.min_varuint_bytes)};
/// Maximum bytes in one canonical unsigned vu128.
pub const MAX_VARUINT_BYTES: usize = ${formatted_decimal(v1.max_varuint_bytes)};
/// Aggregate payload ceiling.
pub const MAX_PAYLOAD_BYTES: usize = ${formatted_decimal(contract.max_payload_bytes)};
`
}
