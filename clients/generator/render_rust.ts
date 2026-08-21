//! Rust client contract rendering.

import {
  render_rust_wire as render_protocol_rust_wire,
  type Wire_Entry,
} from "../../protocol/wire"
import {
  derive_operation_client_projection,
} from "../operation_client_projection"
import type { Client_Contract } from "./model"
import {
  bytes_from_hex,
  encode_vu128,
  formatted_byte,
  formatted_decimal,
  pascal_case,
  rust_byte_array_literal,
  rust_byte_string_literal,
  rust_string_literal,
  snake_case,
  swift_name,
} from "./utils"

function rust_ffi_enum(
  name: string,
  documentation: string,
  member_documentation: string,
  entries: readonly (Wire_Entry & { readonly text?: string })[],
): string {
  const variants = entries
    .map(
      (entry) =>
        `    /// ${member_documentation} identifier for ${entry.name}.
    ${entry.name} = ${formatted_decimal(entry.value)},`,
    )
    .join("\n")
  const try_from_arms = entries
    .map(
      (entry) =>
        `            value if value == Self::${entry.name}.code() => Ok(Self::${entry.name}),`,
    )
    .join("\n")
  const display_arms = entries
    .map(
      (entry) =>
        `            Self::${entry.name} => ${rust_string_literal(entry.text ?? snake_case(entry.name))},`,
    )
    .join("\n")
  return `/// ${documentation}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum ${name} {
${variants}
}

impl ${name} {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for ${name} {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
${try_from_arms}
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for ${name} {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
${display_arms}
        })
    }
}`
}

function rust_api_enum_constants(contract: Client_Contract): string {
  return contract.api.enums
    .flatMap((enum_) =>
      enum_.members.map(
        (member) =>
          `/// Smithy ${enum_.name} value ${member.name}.
pub const SMITHY_${snake_case(enum_.name).toUpperCase()}_${snake_case(member.name).toUpperCase()}: &str = ${rust_string_literal(member.value)};`,
      ),
    )
    .join("\n")
}

function rust_operation_client_projections(contract: Client_Contract): string {
  const operations = new Map(
    (contract.operations ?? []).map((operation) => [
      operation.name,
      operation.contract,
    ]),
  )
  const client_operations = new Set(
    contract.api.operations.map((operation) => operation.name),
  )
  const projections = contract.opcodes
    .map((opcode) => {
      if (!client_operations.has(opcode.name)) {
        return `    None, // ${opcode.name} is wire-only.`
      }
      const operation = operations.get(opcode.name)
      if (operation === undefined) {
        if (contract.operations === undefined) {
          return `    None, // ${opcode.name} has no permissive-fixture metadata.`
        }
        throw new Error(
          `client operation ${opcode.name} has no protocol operation contract`,
        )
      }
      const projection = derive_operation_client_projection(operation)
      return `    Some(OperationClientProjection {
        retry_mode: OperationRetryMode::${pascal_case(projection.retry_mode)},
    }), // ${opcode.name}`
    })
    .join("\n")

  return `/// Generated replay policy owned by the client adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationRetryMode {
    /// Replay after a connection failure when another attempt remains.
    Always,
    /// Never replay after a request may have reached the server.
    Never,
    /// Replay only when the request cannot create server state.
    WhenNotCreating,
}

/// Client-only metadata for one generated operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationClientProjection {
    /// Replay policy selected by the modeled client operation.
    pub retry_mode: OperationRetryMode,
}

/// Generated client projections in wire-operation order.
const OPERATION_CLIENT_PROJECTIONS: [Option<OperationClientProjection>; Opcode::COUNT] = [
${projections}
];

/// Returns client-only metadata when this client exposes the wire operation.
///
/// # Arguments
///
/// * \`opcode\` - The generated wire operation identifier.
///
/// # Returns
///
/// The generated client projection, or \`None\` for a wire-only operation.
pub const fn operation_client_projection(opcode: Opcode) -> Option<OperationClientProjection> {
    OPERATION_CLIENT_PROJECTIONS[opcode.index()]
}`
}

/** Renders the client-owned Rust defaults, ABI, and value-format declarations. */
export function render_rust_client(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  const value_version_bytes = encode_vu128(value.version)
  const envelope = contract.value_envelope
  const envelope_magic = bytes_from_hex(
    envelope.magic_and_version_hex,
    "value envelope magic",
  )
  const ffi = contract.ffi
  const descriptor_layout = ffi.namespace_descriptor_layout
  const api_enum_constants = rust_api_enum_constants(contract)
  const ffi_operations = ffi.operations
    .map(
      (entry) =>
        `/// Native FFI operation identifier for ${entry.name}.
pub const FFI_OPERATION_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_result_kinds = ffi.result_kinds
    .map(
      (entry) =>
        `/// Native FFI result-kind identifier for ${entry.name}.
pub const FFI_RESULT_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_connection_states = ffi.connection_states
    .map(
      (entry) =>
        `/// Native FFI connection-state identifier for ${entry.name}.
pub const FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_transports = ffi.transports
    .map(
      (entry) =>
        `/// Native FFI transport selector for ${entry.name}.
pub const FFI_TRANSPORT_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_set_conditions = ffi.set_conditions
    .map(
      (entry) =>
        `/// Native FFI SET-condition identifier for ${entry.name}.
pub const FFI_SET_CONDITION_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_namespace_descriptor_decode_statuses =
    ffi.namespace_descriptor_decode_statuses
      .map(
        (entry) =>
          `/// Native namespace-descriptor decode status for ${entry.name}.
pub const FFI_NAMESPACE_DESCRIPTOR_DECODE_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
      )
      .join("\n")
  const ffi_namespace_default_expirations = ffi.namespace_default_expirations
    .map(
      (entry) =>
        `/// Native namespace default-expiration value for ${entry.name}.
pub const FFI_NAMESPACE_DEFAULT_EXPIRATION_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_namespace_default_evictions = ffi.namespace_default_evictions
    .map(
      (entry) =>
        `/// Native namespace default-eviction value for ${entry.name}.
pub const FFI_NAMESPACE_DEFAULT_EVICTION_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_namespace_override_policies = ffi.namespace_override_policies
    .map(
      (entry) =>
        `/// Native namespace override-policy value for ${entry.name}.
pub const FFI_NAMESPACE_OVERRIDE_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const descriptor_fields = ffi.namespace_descriptor_fields
  const operation_client_projections =
    rust_operation_client_projections(contract)
  const descriptor_offset_constants = descriptor_fields
    .map(
      (field) =>
        `pub const FFI_NAMESPACE_DESCRIPTOR_${snake_case(field.name).toUpperCase()}_OFFSET: usize = ${formatted_decimal(field.offset)};`,
    )
    .join("\n")
  const ffi_operation_entries = [...contract.opcodes, ...ffi.operations].sort(
    (left, right) => left.value - right.value,
  )
  const ffi_namespace_descriptor = `/// C-compatible namespace descriptor returned by the native ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FfiNamespaceDescriptor {
${descriptor_fields.map(
  (field) => `    pub ${field.name}: ${field.rust_type},`,
).join("\n")}
}

const _: () = {
    assert!(
        core::mem::size_of::<FfiNamespaceDescriptor>()
            == FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES
    );
${descriptor_fields.map(
  (field) =>
    `    assert!(
        core::mem::offset_of!(FfiNamespaceDescriptor, ${field.name})
            == FFI_NAMESPACE_DESCRIPTOR_${snake_case(field.name).toUpperCase()}_OFFSET
    );`,
).join("\n")}
};
`
  return `// Generated from the OpenKache client Smithy contract. Do not edit.

use openkache_protocol::Opcode;

/// Default maximum number of concurrent request lanes.
pub const DEFAULT_MAX_IN_FLIGHT: usize = ${formatted_decimal(defaults.max_in_flight)};
/// Default connection-establishment timeout in milliseconds.
pub const DEFAULT_CONNECT_TIMEOUT_MILLISECONDS: u64 = ${formatted_decimal(defaults.connect_timeout_milliseconds)};
/// Default complete-request timeout in milliseconds.
pub const DEFAULT_REQUEST_TIMEOUT_MILLISECONDS: u64 = ${formatted_decimal(defaults.request_timeout_milliseconds)};
/// Default maximum total attempts for response-safe operations.
pub const DEFAULT_RETRY_MAX_ATTEMPTS: usize = ${formatted_decimal(defaults.retry_max_attempts)};
/// Default Zstandard compression level.
pub const DEFAULT_ZSTANDARD_LEVEL: i32 = ${formatted_decimal(defaults.zstandard_level)};
/// Default minimum serialized input size considered for Zstandard compression.
pub const DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES: usize = ${formatted_decimal(defaults.zstandard_minimum_input_bytes)};
/// Default minimum Zstandard savings required to retain compression.
pub const DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES: usize = ${formatted_decimal(defaults.zstandard_minimum_savings_bytes)};
/// Inclusive minimum supported Zstandard compression level.
pub const DEFAULT_ZSTANDARD_LEVEL_MIN: i32 = ${formatted_decimal(defaults.zstandard_level_min)};
/// Inclusive maximum supported Zstandard compression level.
pub const DEFAULT_ZSTANDARD_LEVEL_MAX: i32 = ${formatted_decimal(defaults.zstandard_level_max)};
/// Default TLS server name used when an adapter does not provide one.
pub const CLIENT_DEFAULT_SERVER_NAME: &str = ${rust_string_literal(defaults.server_name)};
/// PEM label used for adapter-assembled certificate chains.
pub const CLIENT_CERTIFICATE_PEM_TYPE: &str = ${rust_string_literal(defaults.certificate_pem_type)};
/// Minimum positive setting value when zero selects a default.
pub const CLIENT_MINIMUM_POSITIVE_VALUE: usize = ${formatted_decimal(defaults.minimum_positive_value)};

${operation_client_projections}

/// Version of the native client FFI contract.
pub const FFI_ABI_VERSION: u32 = ${formatted_decimal(ffi.abi_version)};
${ffi_operations}
${ffi_result_kinds}
${ffi_connection_states}
${ffi_transports}
${ffi_set_conditions}
${ffi_namespace_descriptor_decode_statuses}
${ffi_namespace_default_expirations}
${ffi_namespace_default_evictions}
${ffi_namespace_override_policies}
/// Size of the C-compatible native namespace descriptor.
pub const FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES: usize = ${formatted_decimal(descriptor_layout.size_bytes)};
/// Native namespace descriptor field offsets.
${descriptor_offset_constants}

${ffi_namespace_descriptor}
${api_enum_constants}
${rust_ffi_enum(
  "FfiOperation",
  "Native FFI operation identifiers shared by every language adapter.",
  "Native FFI operation",
  ffi_operation_entries,
)}

${rust_ffi_enum(
  "FfiResultKind",
  "Native FFI result-kind identifiers shared by every language adapter.",
  "Native FFI result-kind",
  ffi.result_kinds,
)}

${rust_ffi_enum(
  "ConnectionState",
  "Native FFI connection-state identifiers shared by every language adapter.",
  "Native FFI connection-state",
  ffi.connection_states,
)}

${rust_ffi_enum(
  "FfiTransport",
  "Native FFI transport selectors shared by every language adapter.",
  "Native FFI transport",
  ffi.transports,
)}

${rust_ffi_enum(
  "FfiSetCondition",
  "Native FFI SET-condition identifiers shared by every language adapter.",
  "Native FFI SET-condition",
  ffi.set_conditions,
)}

/// Current client-owned value-format version.
pub const VALUE_FORMAT_VERSION: u128 = ${formatted_decimal(value.version)};
/// Canonical VU128 bytes for the current value-format version.
pub const VALUE_FORMAT_VERSION_BYTES: &[u8] = &[${value_version_bytes.map(formatted_byte).join(", ")}];
/// Maximum bytes accepted for a canonical value-format VU128.
pub const VALUE_FORMAT_MAX_VU128_BYTES: usize = ${formatted_decimal(value.max_vu128_bytes)};
/// Bytes occupied by the value-format transform byte.
pub const VALUE_FORMAT_FORMAT_BYTE_BYTES: usize = ${formatted_decimal(value.format_byte_bytes)};
/// Low-nibble mask for the value-format compression identifier.
pub const VALUE_FORMAT_COMPRESSION_MASK: u8 = ${formatted_byte(value.format_compression_mask)};
/// Number of bits to shift the value-format encryption identifier.
pub const VALUE_FORMAT_ENCRYPTION_SHIFT: u8 = ${formatted_byte(value.format_encryption_shift)};
/// Raw serialized-value identifier.
pub const VALUE_FORMAT_SERIALIZATION_RAW: u8 = ${formatted_byte(value.serialization_raw)};
/// Legacy metadata identifier; JSON helpers use OpaqueBytes selector 0.
pub const VALUE_FORMAT_SERIALIZATION_JSON: u8 = ${formatted_byte(value.serialization_json)};
/// StructuredValue-CBOR-v1 payload-format selector.
pub const VALUE_FORMAT_SERIALIZATION_STRUCTURED: u8 = ${formatted_byte(value.serialization_structured)};
/// Uncompressed value-format identifier.
pub const VALUE_FORMAT_COMPRESSION_NONE: u8 = ${formatted_byte(value.compression_none)};
/// Zstandard value-format identifier.
pub const VALUE_FORMAT_COMPRESSION_ZSTANDARD: u8 = ${formatted_byte(value.compression_zstandard)};
/// Unencrypted value-format identifier.
pub const VALUE_FORMAT_ENCRYPTION_NONE: u8 = ${formatted_byte(value.encryption_none)};
/// Compact AES-SIV value-format identifier.
pub const VALUE_FORMAT_ENCRYPTION_COMPACT: u8 = ${formatted_byte(value.encryption_compact)};
/// Robust AES-GCM-SIV value-format identifier.
pub const VALUE_FORMAT_ENCRYPTION_ROBUST: u8 = ${formatted_byte(value.encryption_robust)};
/// Compact AES-SIV synthetic-IV and authentication-tag size.
pub const VALUE_FORMAT_COMPACT_SYNTHETIC_IV_BYTES: usize = ${formatted_decimal(value.compact_synthetic_iv_bytes)};
/// Robust AES-GCM-SIV nonce size.
pub const VALUE_FORMAT_ROBUST_NONCE_BYTES: usize = ${formatted_decimal(value.robust_nonce_bytes)};
/// Robust AES-GCM-SIV authentication-tag size.
pub const VALUE_FORMAT_ROBUST_TAG_BYTES: usize = ${formatted_decimal(value.robust_tag_bytes)};
/// Application-managed data-protection key size.
pub const VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES: usize = ${formatted_decimal(value.data_protection_key_bytes)};
/// BLAKE3 protected-item-ID root derivation context.
pub const VALUE_FORMAT_ITEM_ID_ROOT_CONTEXT: &str = ${rust_string_literal(value.item_id_root_context)};
/// Associated-data domain separator.
pub const VALUE_FORMAT_AAD_DOMAIN: &[u8] = ${rust_byte_string_literal(value.aad_domain)};
/// BLAKE3 value-root derivation context.
pub const VALUE_FORMAT_VALUE_ROOT_CONTEXT: &str = ${rust_string_literal(value.value_root_context)};
/// BLAKE3 Compact AES-SIV MAC-key derivation context.
pub const VALUE_FORMAT_COMPACT_MAC_CONTEXT: &str = ${rust_string_literal(value.compact_mac_context)};
/// BLAKE3 Compact AES-SIV encryption-key derivation context.
pub const VALUE_FORMAT_COMPACT_ENCRYPTION_CONTEXT: &str = ${rust_string_literal(value.compact_encryption_context)};
/// BLAKE3 Robust AES-GCM-SIV key derivation context.
pub const VALUE_FORMAT_ROBUST_CONTEXT: &str = ${rust_string_literal(value.robust_context)};

/// Legacy metadata-envelope magic and version.
pub const VALUE_ENVELOPE_MAGIC_AND_VERSION: [u8; ${envelope_magic.length}] = ${rust_byte_array_literal(envelope_magic)};
/// Maximum UTF-8 byte length of a legacy metadata-envelope encoding identifier.
pub const VALUE_ENVELOPE_MAX_ENCODING_BYTES: usize = ${formatted_decimal(envelope.max_encoding_bytes)};
/// Maximum UTF-8 byte length of a legacy metadata-envelope logical type name.
pub const VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES: usize = ${formatted_decimal(envelope.max_type_name_bytes)};
/// Built-in canonical JSON codec identifier used by the legacy envelope adapter.
pub const VALUE_ENVELOPE_JSON_ENCODING: &str = ${rust_string_literal(envelope.json_encoding)};
`
}

/** Renders the combined Rust client contract for legacy generated consumers. */
export function render_rust(contract: Client_Contract): string {
  return `${render_protocol_rust_wire(contract)}\n${render_rust_client(contract)}`
}
