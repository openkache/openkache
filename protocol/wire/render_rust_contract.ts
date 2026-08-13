/** Rust rendering for the transport-only wire contract. */

import type { Wire_Contract, Wire_Entry } from "../wire_types"

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function formatted_byte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`
}

function wire_name(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase()
}

function rust_string_literal(value: string): string {
  let literal = '"'
  for (const character of value) {
    switch (character) {
      case '"':
        literal += '\\"'
        break
      case "\\":
        literal += "\\\\"
        break
      case "\n":
        literal += "\\n"
        break
      case "\r":
        literal += "\\r"
        break
      case "\t":
        literal += "\\t"
        break
      default: {
        const code_point = character.codePointAt(0) ?? 0
        literal += code_point < 0x20
          ? `\\u{${code_point.toString(16)}}`
          : character
      }
    }
  }
  return `${literal}"`
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

function rust_wire_enum(
  name: string,
  documentation: string,
  entries: readonly Wire_Entry[],
  unknown_variant: string,
): string {
  const variants = entries
    .map((entry) => `            ${entry.name} = ${formatted_byte(entry.value)},`)
    .join("\n")
  const all_variants = entries.map((entry) => `            Self::${entry.name},`).join("\n")
  const name_literals = entries
    .map((entry) => `            ${rust_string_literal(entry.text ?? wire_name(entry.name))},`)
    .join("\n")
  const names = entries
    .map(
      (entry) =>
        `            Self::${entry.name} => ${rust_string_literal(entry.text ?? wire_name(entry.name))},`,
    )
    .join("\n")
  return `wire_enum! {
    /// ${documentation}
    pub enum ${name} {
${variants}
    }
    unknown => ${unknown_variant}
}

impl ${name} {
    /// Number of values assigned by the Smithy ${name} contract.
    pub const COUNT: usize = ${entries.length};

    /// Every assigned ${name} in wire-value order.
    pub const ALL: [Self; Self::COUNT] = [
${all_variants}
    ];

    /// Stable lowercase names in wire-value order.
    pub const NAMES: [&'static str; Self::COUNT] = [
${name_literals}
    ];

    /// Zero-based position in the wire-value arrays.
    pub const fn index(self) -> usize {
        match self {
${entries.map((entry, index) => `            Self::${entry.name} => ${index},`).join("\n")}
        }
    }

    /// Stable lowercase name for this assigned value.
    pub const fn name(self) -> &'static str {
        match self {
${names}
        }
    }

    /// Resolves a generated name at an API boundary.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
${entries
  .map((entry) =>
    `            ${rust_string_literal(entry.text ?? wire_name(entry.name))} => Some(Self::${entry.name}),`
  )
  .join("\n")}
            _ => None,
        }
    }
}`
}

/** Renders only transport constants and assigned wire enums. */
export function render_rust_wire(contract: Wire_Contract): string {
  const v1 = contract.v1
  return `// Generated from the OpenKache Smithy transport contract. Do not edit.

/// QUIC application protocol identifier.
pub const ALPN: &[u8] = ${rust_byte_string_literal(v1.alpn)};
/// Bytes occupied by a request opcode.
pub const OPCODE_BYTES: usize = ${formatted_decimal(v1.opcode_bytes)};
/// Bytes occupied by a response status.
pub const STATUS_BYTES: usize = ${formatted_decimal(v1.status_bytes)};
/// Bytes before request body framing metadata.
pub const REQUEST_FIXED_BYTES: usize = ${formatted_decimal(v1.request_fixed_bytes)};
/// Bytes before response payload framing metadata.
pub const RESPONSE_FIXED_BYTES: usize = ${formatted_decimal(v1.response_fixed_bytes)};
/// Minimum bytes in one canonical unsigned vu128.
pub const MIN_VARUINT_BYTES: usize = ${formatted_decimal(v1.min_varuint_bytes)};
/// Maximum bytes in one canonical unsigned vu128.
pub const MAX_VARUINT_BYTES: usize = ${formatted_decimal(v1.max_varuint_bytes)};
/// Bytes in one protocol item identifier.
pub const ITEM_ID_BYTES: usize = ${formatted_decimal(contract.item_id_bytes)};
/// Aggregate payload ceiling.
pub const MAX_VALUE_BYTES: usize = ${formatted_decimal(contract.max_value_bytes)};

${rust_wire_enum("Opcode", "Operations assigned by the transport contract.", contract.opcodes, "UnknownOpcode")}

${rust_wire_enum("Status", "Statuses assigned by the transport contract.", contract.statuses, "UnknownStatus")}
`
}
