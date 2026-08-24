/** Rust client-core and native C contract renderers. */

import {
  derive_wire_operation_descriptor,
  render_rust_wire as render_protocol_rust_wire,
  type Wire_Entry,
} from "../../protocol/wire"
import {
  render_rust_semantic_constants as render_protocol_rust_semantic_constants,
} from "../../protocol/compatibility_v1_renderer"
import type { Api_Operation, Api_Operation_Contract } from "../operation_models"
import type { Client_Contract } from "../client_contract"
import {
  operation_compatibility_response_adapter,
  operation_response_context,
} from "../compatibility_result_projections"
import { compatibility_response_result_kind } from "../compatibility_response_adapters"
import { request_transport_plan } from "../compatibility_request_adapters"
import { compatibility_ffi_operation_contract } from "../compatibility_ffi_adapters"
import {
  render_c_native_function_typedefs,
  render_c_native_functions,
  render_c_native_structure_assertions,
  render_c_native_structures,
} from "../native_abi_renderers"
import { pascal_case, snake_case } from "../generator_names"
import { encode_vu128 } from "../generator_values"
import {
  bytes_from_hex,
  c_string_literal,
  c_unsigned_literal,
  formatted_byte,
  formatted_decimal,
  rust_byte_array_literal,
  rust_byte_string_literal,
  rust_string_literal,
} from "./rendering"

function gate0_defaults(contract: Client_Contract["client_defaults"]): {
  readonly alpn_version: number
  readonly compression: number
  readonly encryption: number
  readonly item_id_root_key_hex: string
  readonly namespace_id: number
  readonly value_selector: number
} {
  return {
    alpn_version: contract.gate0_alpn_version,
    compression: contract.gate0_compression,
    encryption: contract.gate0_encryption,
    item_id_root_key_hex: contract.gate0_item_id_root_key_hex,
    namespace_id: contract.gate0_namespace_id,
    value_selector: contract.gate0_value_selector,
  }
}

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

function rust_status_variant(contract: Client_Contract, status: string): string {
  const entry = contract.statuses.find(
    (candidate) =>
      candidate.name === status ||
      candidate.text === status ||
      snake_case(candidate.name) === status,
  )
  if (entry === undefined) {
    throw new Error(`operation metadata references unknown status ${status}`)
  }
  return entry.name
}

function render_rust_operation_contract(contract: Client_Contract): string {
  const operations = contract.api.operations
  if (
    operations.length !== contract.opcodes.length ||
    operations.some((operation) => operation.contract === undefined)
  ) {
    return ""
  }
  const ffi_result_variant = (name: string): string | undefined => {
    const entry = contract.ffi.result_kinds.find(
      (candidate) => candidate.name === name,
    )
    return entry === undefined ? undefined : `FfiResultKind::${entry.name}`
  }
  const response_result_kind = (
    operation: Api_Operation & { readonly contract: Api_Operation_Contract },
  ): string => {
    const request = request_transport_plan(operation.contract)
    const context = operation_response_context(operation.contract, request)
    const compatibility_adapter = operation_compatibility_response_adapter(
      operation.contract,
      request,
    )
    // Generic operations use the shape-neutral RAW discriminator. Explicit
    // compatibility adapters own domain result semantics; this keeps an
    // operation such as EXPERIMENTAL_SYNC on the canonical `Ok` result while
    // leaving route-less future operations shape-neutral.
    const generic_result_name = "raw"
    const status_mapping = operation.contract.success_statuses.map((status) => {
      const status_variant = rust_status_variant(contract, status)
      const result_name = compatibility_adapter === undefined
        ? undefined
        : compatibility_response_result_kind(compatibility_adapter, status, context)
      // A newly modeled success status may intentionally have no dedicated
      // ergonomic FFI discriminator yet.  Preserve the operation through the
      // generic result envelope instead of making the shared generator fail:
      // status validation still comes from the canonical contract, while
      // language adapters can expose the raw status/payload until a
      // domain-specific convenience is added.
      const result_variant = (result_name === undefined
        ? undefined
        : ffi_result_variant(pascal_case(snake_case(result_name)))) ??
        ffi_result_variant(pascal_case(snake_case(generic_result_name))) ??
        // Synthetic and future API contracts may not yet declare a native
        // convenience discriminator. Keep the generic raw envelope stable;
        // production FFI models provide the Raw member, while render-only
        // fixtures can still inspect the generated mapping.
        "FfiResultKind::Raw"
      return `            openkache_protocol::Status::${status_variant} => Some(${result_variant}),`
    })
    return `        openkache_protocol::Opcode::${operation.name} => match status {
${status_mapping.join("\n")}
            _ => None,
        },`
  }
  const response_result_kind_metadata = operations
    .map((operation) =>
      response_result_kind({
        ...operation,
        contract: operation.contract!,
      })
    )
    .join("\n")
  const client_projection_metadata = operations
    .map((operation) => {
      const semantic = operation.contract!.response_semantics ??
        derive_wire_operation_descriptor(operation.contract!).response_framing
      return `    OperationClientProjection {
        retry_mode: OperationRetryMode::${pascal_case(operation.contract!.retry_mode ?? "always")},
        result: OperationResultSpec {
            response_semantics: ${rust_string_literal(semantic)},
        },
    }`
    })
    .join(",\n")
  return `// Operation framing, field plans, codec descriptors, and response
// layouts are generated once by protocol/wire.ts. Client retry/result policy
// is rendered here, at the client adapter boundary, so the protocol crate
// remains free of client execution metadata.
pub use openkache_protocol::operation::{
    OperationFieldLayout,
    OperationFramePolicy,
    OperationFieldPlan,
    OperationLayoutFraming,
    OperationLayoutPlan,
    OperationRequestFraming,
    OperationResponseFraming,
    OperationWireSpec,
    WireCodecDescriptor,
    WireCodecKind,
    MAX_OPERATION_FIELDS,
    MAX_OPERATION_REQUEST_FIELDS,
    OPERATION_CODEC_NAMES,
    WIRE_CODEC_DESCRIPTORS,
    WIRE_CODEC_NAMES,
};
/// Generated replay policy owned by the client adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationRetryMode {
    Always,
    Never,
    WhenNotCreating,
}

/// Open response semantic label owned by a client result adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationResultSpec {
    pub response_semantics: &'static str,
}

/// Client-only projection derived from the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationClientProjection {
    pub retry_mode: OperationRetryMode,
    pub result: OperationResultSpec,
}

/// Generated client projections in opcode order.
pub const OPERATION_CLIENT_PROJECTIONS: [OperationClientProjection; openkache_protocol::Opcode::COUNT] = [
${client_projection_metadata}
];

/// Returns the canonical wire-only operation spec.
pub const fn operation_wire_spec(
    opcode: openkache_protocol::Opcode,
) -> OperationWireSpec {
    openkache_protocol::operation::operation_wire_spec(opcode)
}

/// Returns the client-only projection for one operation.
pub const fn operation_client_projection(
    opcode: openkache_protocol::Opcode,
) -> Option<OperationClientProjection> {
    Some(OPERATION_CLIENT_PROJECTIONS[opcode.index()])
}

/// Resolves a canonical generated codec identifier.
pub fn wire_codec_kind(
    name: &str,
) -> Option<openkache_protocol::codec::CodecKind> {
    openkache_protocol::wire_codec_kind(name)
}

/// Maps a contract-approved response status to the native result discriminator
/// consumed by generated language adapters. The mapping is generated from the
/// operation's semantic result plan; the transport executor does not maintain
/// an operation-name table.
pub const fn operation_result_kind(
    opcode: openkache_protocol::Opcode,
    status: openkache_protocol::Status,
) -> Option<FfiResultKind> {
    match opcode {
${response_result_kind_metadata}
    }
}
`
}

function render_rust_ffi_operation_contract(contract: Client_Contract): string {
  const protocol_contracts = new Map(
    contract.api.operations.map((operation) => [
      operation.name,
      compatibility_ffi_operation_contract(operation),
    ]),
  )
  const ffi_contracts = new Map(
    contract.ffi.operations.map((operation) => [
      operation.name,
      operation.operation_contract,
    ]),
  )
  const entries = [...contract.opcodes, ...contract.ffi.operations]
    .sort((left, right) => left.value - right.value)
    .map((entry) => ({
      entry,
      operation_contract:
        protocol_contracts.get(entry.name) ?? ffi_contracts.get(entry.name),
    }))
  if (
    entries.some(
      ({ operation_contract }) => operation_contract === undefined,
    )
  ) {
    return ""
  }
  const rendered_entries = entries
    .map(({ entry, operation_contract }) => {
      const metadata = operation_contract!
      const input_kind = pascal_case(snake_case(metadata.input_kind))
      return `        FfiOperation::${entry.name} => FfiOperationContract {
            input_kind: FfiInputKind::${input_kind},
            request_item_count: ${metadata.request_item_count},
            accepts_value: ${metadata.accepts_value},
            accepts_set_options: ${metadata.accepts_set_options},
            supports_protected: ${metadata.supports_protected},
            supports_raw: ${metadata.supports_raw},
            supports_scoped: ${metadata.supports_scoped},
            dedicated_abi: ${metadata.dedicated_abi},
        },`
    })
    .join("\n")
  const protocol_opcode_arms = contract.opcodes
    .map(
      (entry) =>
        `        FfiOperation::${entry.name} => Some(openkache_protocol::Opcode::${entry.name}),`,
    )
    .join("\n")
  return `/// Input buffer kind declared by the native FFI contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FfiInputKind {
    None,
    ApplicationKey,
    ItemId,
}

/// Native dispatch and buffer contract for one FFI operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiOperationContract {
    pub input_kind: FfiInputKind,
    pub request_item_count: usize,
    pub accepts_value: bool,
    pub accepts_set_options: bool,
    pub supports_protected: bool,
    pub supports_raw: bool,
    pub supports_scoped: bool,
    pub dedicated_abi: bool,
}

/// Returns the generated native contract for one FFI operation.
pub const fn ffi_operation_contract(
    operation: FfiOperation,
) -> FfiOperationContract {
    match operation {
${rendered_entries}
    }
}

/// Resolves a protocol opcode from the shared native operation enum.
pub const fn protocol_opcode(
    operation: FfiOperation,
) -> Option<openkache_protocol::Opcode> {
    match operation {
${protocol_opcode_arms}
        _ => None,
    }
}
`
}

/** Renders Rust associated operation constants for the shared client core. */
export function render_rust_operation_constants(contract: Client_Contract): string {
  const constants = contract.opcodes
    .map(
      (entry) =>
        `/// \`${entry.text ?? snake_case(entry.name)}\` request.
pub const ${entry.name}: Self = Self::Protocol(Opcode::${entry.name});`,
    )
    .join("\n")
  return `#[allow(non_upper_case_globals)]
impl Operation {
${constants}
}
`
}

/** Renders the client-owned Rust defaults, ABI, and value-format declarations. */
export function render_rust_client(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  const gate0 = gate0_defaults(defaults)
  const gate0_item_id_root = bytes_from_hex(
    gate0.item_id_root_key_hex,
    "clientDefaults.gate0ItemIdRootKeyHex",
  )
  if (gate0_item_id_root.length !== 32) {
    throw new Error(
      "clientDefaults.gate0ItemIdRootKeyHex must contain exactly 32 bytes",
    )
  }
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
  const ffi_status_categories = ffi.status_categories
    .map(
      (entry) =>
        `/// Native FFI status-category identifier for ${entry.name}.
pub const FFI_STATUS_CATEGORY_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_error_categories = ffi.error_categories
    .map(
      (entry) =>
        `/// Native FFI error-category identifier for ${entry.name}.
pub const FFI_ERROR_CATEGORY_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_request_states = ffi.request_states
    .map(
      (entry) =>
        `/// Native FFI request-state identifier for ${entry.name}.
pub const FFI_REQUEST_STATE_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_value_representations = ffi.value_representations
    .map(
      (entry) =>
        `/// Native FFI value-representation identifier for ${entry.name}.
pub const FFI_VALUE_REPRESENTATION_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_value_modes = ffi.value_modes
    .map(
      (entry) =>
        `/// Native FFI value-mode identifier for ${entry.name}.
pub const FFI_VALUE_MODE_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
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
  const ffi_key_specs = ffi.key_specs
    .map(
      (entry) =>
        `/// Native FFI logical-key specification identifier for ${entry.name}.
pub const FFI_KEY_SPEC_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
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
  const descriptor_offset_constants = descriptor_fields
    .map(
      (field) =>
        `pub const FFI_NAMESPACE_DESCRIPTOR_${snake_case(field.name).toUpperCase()}_OFFSET: usize = ${formatted_decimal(field.offset)};`,
    )
    .join("\n")
  const ffi_operation_entries = [...contract.opcodes, ...ffi.operations].sort(
    (left, right) => left.value - right.value,
  )
  const rust_native_structures = ffi.native_abi_structures
    .filter((structure) => structure.name === "FfiOperationField")
    .map(
      (structure) => `/// Borrowed field passed through a generated structured native operation.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ${structure.name} {
${structure.fields.map((field) => {
  const rust_type = (() => {
    switch (field.type) {
      case "u8_pointer":
        return "*const u8"
      case "size":
        return "usize"
      case "uint8":
        return "u8"
      default:
        throw new Error(
          `unsupported Rust native operation-field type ${field.type}`,
        )
    }
  })()
  return `    pub ${field.name}: ${rust_type},`
}).join("\n")}
}
`,
    )
    .join("\n")
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

${render_protocol_rust_semantic_constants(contract)}

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
/// Gate 0's fixed ALPN version.
pub const CLIENT_GATE0_ALPN_VERSION: u32 = ${formatted_decimal(gate0.alpn_version)};
/// Gate 0's fixed value-compression selector.
pub const CLIENT_GATE0_COMPRESSION: u8 = ${formatted_byte(gate0.compression)};
/// Gate 0's fixed value-encryption selector.
pub const CLIENT_GATE0_ENCRYPTION: u8 = ${formatted_byte(gate0.encryption)};
/// Gate 0's fixed public Item-ID root.
pub const CLIENT_GATE0_ITEM_ID_ROOT: [u8; ${gate0_item_id_root.length}] = ${rust_byte_array_literal(gate0_item_id_root)};
/// Gate 0's fixed server-assigned namespace ID.
pub const CLIENT_GATE0_NAMESPACE_ID: u64 = ${formatted_decimal(gate0.namespace_id)};
/// Gate 0's fixed value-format selector byte.
pub const CLIENT_GATE0_VALUE_SELECTOR: u8 = ${formatted_byte(gate0.value_selector)};
/// Gate 0 ALPN protocol version selected by maintained facades.
pub const GATE0_ALPN_VERSION: usize = ${formatted_decimal(gate0.alpn_version)};
/// Gate 0 compression identifier.
pub const GATE0_COMPRESSION: u8 = ${formatted_byte(gate0.compression)};
/// Gate 0 value-protection identifier.
pub const GATE0_ENCRYPTION: u8 = ${formatted_byte(gate0.encryption)};
/// Gate 0 public development Item-ID root key.
pub const GATE0_ITEM_ID_ROOT_KEY: [u8; 32] = ${rust_byte_array_literal(gate0_item_id_root)};
/// Gate 0 namespace identity.
pub const GATE0_NAMESPACE_ID: u64 = ${formatted_decimal(gate0.namespace_id)};
/// Gate 0 value-format selector byte.
pub const GATE0_VALUE_SELECTOR: u8 = ${formatted_byte(gate0.value_selector)};

/// Version of the native client FFI contract.
pub const FFI_ABI_VERSION: u32 = ${formatted_decimal(ffi.abi_version)};
${ffi_operations}
${ffi_result_kinds}
${ffi_status_categories}
${ffi_error_categories}
${ffi_request_states}
${ffi_value_representations}
${ffi_value_modes}
${ffi_connection_states}
${ffi_transports}
${ffi_set_conditions}
${ffi_key_specs}
${ffi_namespace_descriptor_decode_statuses}
${ffi_namespace_default_expirations}
${ffi_namespace_default_evictions}
${ffi_namespace_override_policies}
/// Size of the C-compatible native namespace descriptor.
pub const FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES: usize = ${formatted_decimal(descriptor_layout.size_bytes)};
/// Native namespace descriptor field offsets.
${descriptor_offset_constants}

${ffi_namespace_descriptor}
${rust_native_structures}
${api_enum_constants}
${render_rust_operation_contract(contract)}
${rust_ffi_enum(
  "FfiOperation",
  "Native FFI operation identifiers shared by every language adapter.",
  "Native FFI operation",
  ffi_operation_entries,
)}

${render_rust_ffi_operation_contract(contract)}

${rust_ffi_enum(
  "FfiResultKind",
  "Native FFI result-kind identifiers shared by every language adapter.",
  "Native FFI result-kind",
  ffi.result_kinds,
)}

${rust_ffi_enum(
  "FfiStatusCategory",
  "Native FFI completion-status categories shared by every language adapter.",
  "Native FFI status category",
  ffi.status_categories,
)}

${rust_ffi_enum(
  "FfiTransport",
  "Native FFI transport selectors shared by every language adapter.",
  "Native FFI transport",
  ffi.transports,
)}

${rust_ffi_enum(
  "FfiErrorCategory",
  "Native FFI structured error categories shared by every language adapter.",
  "Native FFI error category",
  ffi.error_categories,
)}

${rust_ffi_enum(
  "FfiRequestState",
  "Native FFI asynchronous request lifecycle states shared by every language adapter.",
  "Native FFI request state",
  ffi.request_states,
)}

${rust_ffi_enum(
  "FfiValueRepresentation",
  "Native FFI value-representation options shared by every language adapter.",
  "Native FFI value representation",
  ffi.value_representations,
)}

${rust_ffi_enum(
  "FfiValueMode",
  "Native FFI value-mode options shared by every language adapter.",
  "Native FFI value mode",
  ffi.value_modes,
)}

${rust_ffi_enum(
  "ConnectionState",
  "Native FFI connection-state identifiers shared by every language adapter.",
  "Native FFI connection-state",
  ffi.connection_states,
)}

${rust_ffi_enum(
  "FfiSetCondition",
  "Native FFI SET-condition identifiers shared by every language adapter.",
  "Native FFI SET-condition",
  ffi.set_conditions,
)}

${rust_ffi_enum(
  "FfiKeySpec",
  "Native FFI logical-key specification identifiers shared by every language adapter.",
  "Native FFI key spec",
  ffi.key_specs,
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

function c_contract_enum(
  name: string,
  entries: readonly Wire_Entry[],
  prefix: string,
): string {
  const variants = entries
    .map(
      (entry) =>
        `    ${prefix}_${snake_case(entry.name).toUpperCase()} = ${formatted_byte(entry.value)},`,
    )
    .join("\n")
  return `typedef enum ${name} {
${variants}
} ${name};`
}

function c_contract_client_operation_enum(contract: Client_Contract): string {
  const protocol_entries = contract.opcodes.map((entry) => ({
    ...entry,
    expression: `OPENKACHE_SMITHY_OPCODE_${snake_case(entry.name).toUpperCase()}`,
  }))
  const ffi_entries = contract.ffi.operations
    .filter((entry) => entry.value <= 0x7fff_ffff)
    .map((entry) => ({
      ...entry,
      expression: `OPENKACHE_SMITHY_FFI_OPERATION_${snake_case(entry.name).toUpperCase()}`,
    }))
  const enum_entries = [...protocol_entries, ...ffi_entries]
    .map(
      (entry) =>
        `    OPENKACHE_CLIENT_OPERATION_${snake_case(entry.name).toUpperCase()} = ${entry.expression},`,
    )
    .join("\n")
  const macro_entries = contract.ffi.operations
    .filter((entry) => entry.value > 0x7fff_ffff)
    .map(
      (entry) =>
        `#define OPENKACHE_CLIENT_OPERATION_${snake_case(entry.name).toUpperCase()} OPENKACHE_SMITHY_FFI_OPERATION_${snake_case(entry.name).toUpperCase()}`,
    )
    .join("\n")
  return `/*
 * Client operation identifiers combine protocol opcodes with local FFI
 * operations. Values that do not fit a C enum remain macros so the public
 * uint32_t ABI can represent the complete Smithy contract.
 */
typedef enum openkache_client_operation {
${enum_entries}
} openkache_client_operation_t;

${macro_entries}`
}

function c_contract_api_enum(
  contract: Client_Contract,
  name: string,
  prefix: string,
): string {
  const enum_ = contract.api.enums.find((candidate) => candidate.name === name)
  if (enum_ === undefined) {
    throw new Error(`Smithy API enum ${name} is required by the C contract`)
  }
  return enum_.members
    .map(
      (member) =>
        `#define ${prefix}_${snake_case(member.name).toUpperCase()} ${c_string_literal(member.value)}`,
    )
    .join("\n")
}

function c_contract_client_compatibility(contract: Client_Contract): string {
  const result_entries = contract.ffi.result_kinds
    .map(
      (entry) =>
        `    OPENKACHE_CLIENT_RESULT_${snake_case(entry.name).toUpperCase()} = OPENKACHE_SMITHY_FFI_RESULT_${snake_case(entry.name).toUpperCase()},`,
    )
    .join("\n")
  const connection_entries = contract.ffi.connection_states
    .map(
      (entry) =>
        `    OPENKACHE_CLIENT_CONNECTION_${snake_case(entry.name).toUpperCase()} = OPENKACHE_SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()},`,
    )
    .join("\n")
  const transport_entries = contract.ffi.transports
    .map(
      (entry) =>
        `    OPENKACHE_CLIENT_TRANSPORT_${snake_case(entry.name).toUpperCase()} = OPENKACHE_SMITHY_FFI_TRANSPORT_${snake_case(entry.name).toUpperCase()},`,
    )
    .join("\n")
  const set_condition_entries = contract.ffi.set_conditions
    .map(
      (entry) =>
        `    OPENKACHE_CLIENT_SET_CONDITION_${snake_case(entry.name).toUpperCase()} = OPENKACHE_SMITHY_FFI_SET_CONDITION_${snake_case(entry.name).toUpperCase()},`,
    )
    .join("\n")
  const status_category_entries = contract.ffi.status_categories
    .map(
      (entry) =>
        `    OPENKACHE_CLIENT_STATUS_${snake_case(entry.name).toUpperCase()} = OPENKACHE_SMITHY_FFI_STATUS_CATEGORY_${snake_case(entry.name).toUpperCase()},`,
    )
    .join("\n")
  const error_category_entries = contract.ffi.error_categories
    .map(
      (entry) =>
        `    OPENKACHE_CLIENT_ERROR_${snake_case(entry.name).toUpperCase()} = OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_${snake_case(entry.name).toUpperCase()},`,
    )
    .join("\n")
  const request_state_entries = contract.ffi.request_states
    .map(
      (entry) =>
        `    OPENKACHE_CLIENT_REQUEST_${snake_case(entry.name).toUpperCase()} = OPENKACHE_SMITHY_FFI_REQUEST_STATE_${snake_case(entry.name).toUpperCase()},`,
    )
    .join("\n")
  const value_representation_entries = contract.ffi.value_representations
    .map(
      (entry) =>
        `    OPENKACHE_CLIENT_VALUE_${snake_case(entry.name).toUpperCase()} = OPENKACHE_SMITHY_FFI_VALUE_REPRESENTATION_${snake_case(entry.name).toUpperCase()},`,
    )
    .join("\n")
  const value_mode_entries = contract.ffi.value_modes
    .map(
      (entry) =>
        `    OPENKACHE_CLIENT_VALUE_MODE_${snake_case(entry.name).toUpperCase()} = OPENKACHE_SMITHY_FFI_VALUE_MODE_${snake_case(entry.name).toUpperCase()},`,
    )
    .join("\n")
  return `/* Source-compatible aliases generated from the Smithy FFI contract. */
#define OPENKACHE_CLIENT_ABI_VERSION OPENKACHE_SMITHY_FFI_ABI_VERSION
#define OPENKACHE_CLIENT_DATA_PROTECTION_KEY_BYTES \\
    OPENKACHE_SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES

typedef openkache_client_t openkache_client_handle;
typedef openkache_client_result_t openkache_client_result;

#define OPENKACHE_CLIENT_NAMESPACE_DESCRIPTOR_DECODE_OK \\
    OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_DECODE_OK
#define OPENKACHE_CLIENT_NAMESPACE_DESCRIPTOR_DECODE_INVALID \\
    OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_DECODE_INVALID
#define OPENKACHE_CLIENT_NAMESPACE_DEFAULT_EXPIRATION_NO_EXPIRY \\
    OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_NO_EXPIRY
#define OPENKACHE_CLIENT_NAMESPACE_DEFAULT_EXPIRATION_FIXED_TTL \\
    OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_FIXED_TTL
#define OPENKACHE_CLIENT_NAMESPACE_DEFAULT_EVICTION_EVICTABLE \\
    OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EVICTION_EVICTABLE
#define OPENKACHE_CLIENT_NAMESPACE_DEFAULT_EVICTION_PROTECTED \\
    OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EVICTION_PROTECTED
#define OPENKACHE_CLIENT_NAMESPACE_OVERRIDE_DISALLOWED \\
    OPENKACHE_SMITHY_FFI_NAMESPACE_OVERRIDE_DISALLOWED
#define OPENKACHE_CLIENT_NAMESPACE_OVERRIDE_ALLOWED \\
    OPENKACHE_SMITHY_FFI_NAMESPACE_OVERRIDE_ALLOWED

typedef enum openkache_client_result_kind {
${result_entries}
} openkache_client_result_kind_t;

typedef enum openkache_client_connection_state {
${connection_entries}
} openkache_client_connection_state_t;

typedef enum openkache_client_transport {
${transport_entries}
} openkache_client_transport_t;

typedef enum openkache_client_set_condition {
${set_condition_entries}
} openkache_client_set_condition_t;

typedef enum openkache_client_status_category {
${status_category_entries}
} openkache_client_status_category_t;

typedef enum openkache_client_error_category {
${error_category_entries}
} openkache_client_error_category_t;

typedef enum openkache_client_request_state {
${request_state_entries}
} openkache_client_request_state_t;

typedef enum openkache_client_value_representation {
${value_representation_entries}
} openkache_client_value_representation_t;

typedef enum openkache_client_value_mode {
${value_mode_entries}
} openkache_client_value_mode_t;

typedef enum openkache_client_encryption {
    OPENKACHE_CLIENT_ENCRYPTION_NONE = OPENKACHE_SMITHY_VALUE_ENCRYPTION_NONE,
    OPENKACHE_CLIENT_ENCRYPTION_COMPACT = OPENKACHE_SMITHY_VALUE_ENCRYPTION_COMPACT,
    OPENKACHE_CLIENT_ENCRYPTION_ROBUST = OPENKACHE_SMITHY_VALUE_ENCRYPTION_ROBUST,
} openkache_client_encryption_t;`
}

/** Renders the Smithy constants consumed by native C and C++ adapters.
 *
 * @param contract - Validated language-neutral wire and value-format contract.
 * @returns Deterministic C declarations with a trailing newline.
 */
export function render_c_contract(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  const gate0 = gate0_defaults(defaults)
  const gate0_item_id_root = bytes_from_hex(
    gate0.item_id_root_key_hex,
    "clientDefaults.gate0ItemIdRootKeyHex",
  )
  if (gate0_item_id_root.length !== 32) {
    throw new Error(
      "clientDefaults.gate0ItemIdRootKeyHex must contain exactly 32 bytes",
    )
  }
  const gate0_item_id_root_bytes = gate0_item_id_root
    .map((byte) => `${formatted_byte(byte)}u`)
    .join(", ")
  const envelope = contract.value_envelope
  const ffi = contract.ffi
  const descriptor_fields = ffi.namespace_descriptor_fields
  const descriptor_layout = ffi.namespace_descriptor_layout
  const ffi_defines = [
    `#define OPENKACHE_SMITHY_FFI_ABI_VERSION ${c_unsigned_literal(ffi.abi_version)}`,
    ...ffi.operations.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_OPERATION_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.result_kinds.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_RESULT_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.status_categories.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_STATUS_CATEGORY_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.error_categories.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.request_states.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_REQUEST_STATE_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.value_representations.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_VALUE_REPRESENTATION_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.value_modes.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_VALUE_MODE_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.connection_states.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.transports.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_TRANSPORT_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.set_conditions.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_SET_CONDITION_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.key_specs.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_KEY_SPEC_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.namespace_descriptor_decode_statuses.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_DECODE_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.namespace_default_expirations.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.namespace_default_evictions.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_NAMESPACE_DEFAULT_EVICTION_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.namespace_override_policies.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_NAMESPACE_OVERRIDE_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
  ].join("\n")
  const operation_enum = c_contract_enum(
    "openkache_smithy_opcode",
    contract.opcodes,
    "OPENKACHE_SMITHY_OPCODE",
  )
  const status_enum = c_contract_enum(
    "openkache_smithy_status",
    contract.statuses,
    "OPENKACHE_SMITHY_STATUS",
  )
  const c_namespace_descriptor_fields = descriptor_fields.map(
    (field) => `    ${field.c_type} ${field.name};`,
  ).join("\n")
  const native_structures = render_c_native_structures(contract)
  const native_structure_assertions =
    render_c_native_structure_assertions(contract)
  const native_functions = render_c_native_functions(contract)
  const native_function_typedefs = render_c_native_function_typedefs(contract)
  const client_compatibility = c_contract_client_compatibility(contract)
  const descriptor_offset_defines = descriptor_fields
    .map(
      (field) =>
        `#define OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_${snake_case(field.name).toUpperCase()}_OFFSET ${field.offset}u`,
    )
    .join("\n")
  const descriptor_offset_asserts = descriptor_fields
    .map(
      (field) =>
        `OPENKACHE_SMITHY_STATIC_ASSERT(offsetof(openkache_smithy_namespace_descriptor_t, ${field.name}) ==\n                   OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_${snake_case(field.name).toUpperCase()}_OFFSET,\n               "Smithy namespace descriptor ${field.name} offset changed");`,
    )
    .join("\n")
  return `/* Generated from the OpenKache Smithy contract. Do not edit. */
#ifndef OPENKACHE_SMITHY_CONTRACT_H
#define OPENKACHE_SMITHY_CONTRACT_H

#include <stddef.h>
#include <stdint.h>

#define OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES ${descriptor_layout.size_bytes}u
${descriptor_offset_defines}

typedef struct openkache_smithy_namespace_descriptor {
${c_namespace_descriptor_fields}
} openkache_smithy_namespace_descriptor_t;

typedef openkache_smithy_namespace_descriptor_t
    openkache_client_namespace_descriptor_t;

typedef struct openkache_client openkache_client_t;
typedef struct openkache_client_result openkache_client_result_t;
typedef struct openkache_client_request openkache_client_request_t;

${native_structures}

#ifdef __cplusplus
#define OPENKACHE_SMITHY_STATIC_ASSERT static_assert
#define OPENKACHE_SMITHY_ALIGNOF(type) alignof(type)
#else
#define OPENKACHE_SMITHY_STATIC_ASSERT _Static_assert
#define OPENKACHE_SMITHY_ALIGNOF(type) _Alignof(type)
#endif
#define OPENKACHE_SMITHY_ALIGN_UP(value, alignment) \
    (((value) + (alignment) - 1u) / (alignment) * (alignment))

${native_structure_assertions}

/* Stable native function declarations shared by every language adapter. */
#ifdef __cplusplus
extern "C" {
#endif
${native_functions}
#ifdef __cplusplus
} /* extern "C" */
#endif

/* Function-pointer types used by dynamic language loaders. */
${native_function_typedefs}

OPENKACHE_SMITHY_STATIC_ASSERT(sizeof(openkache_smithy_namespace_descriptor_t) ==
                   OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES,
               "Smithy namespace descriptor size changed");
${descriptor_offset_asserts}
#undef OPENKACHE_SMITHY_STATIC_ASSERT
#undef OPENKACHE_SMITHY_ALIGNOF
#undef OPENKACHE_SMITHY_ALIGN_UP

#define OPENKACHE_SMITHY_ITEM_ID_BYTES ${contract.item_id_bytes}u
#define OPENKACHE_SMITHY_MAX_VALUE_BYTES ${contract.max_value_bytes}u
#define OPENKACHE_SMITHY_ALPN ${c_string_literal(contract.v1.alpn)}
#define OPENKACHE_SMITHY_OPCODE_BYTES ${contract.v1.opcode_bytes}u
#define OPENKACHE_SMITHY_STATUS_BYTES ${contract.v1.status_bytes}u
#define OPENKACHE_SMITHY_REQUEST_FIXED_BYTES ${contract.v1.request_fixed_bytes}u
#define OPENKACHE_SMITHY_RESPONSE_FIXED_BYTES ${contract.v1.response_fixed_bytes}u
#define OPENKACHE_SMITHY_MIN_VARUINT_BYTES ${contract.v1.min_varuint_bytes}u
#define OPENKACHE_SMITHY_MAX_VARUINT_BYTES ${contract.v1.max_varuint_bytes}u
#define OPENKACHE_SMITHY_NAMESPACE_ID_BYTES ${contract.v1.namespace_id_bytes}u
#define OPENKACHE_SMITHY_NAMESPACE_REVISION_BYTES ${contract.v1.namespace_revision_bytes}u
#define OPENKACHE_SMITHY_NAMESPACE_NAME_LENGTH_BYTES ${contract.v1.namespace_name_length_bytes}u
#define OPENKACHE_SMITHY_NAMESPACE_NAME_MAX_BYTES ${contract.v1.namespace_name_max_bytes}u
#define OPENKACHE_SMITHY_SET_FLAGS_BYTES ${contract.v1.set_flags_bytes}u
#define OPENKACHE_SMITHY_SET_CONDITION_MASK ${c_unsigned_literal(contract.v1.set_condition_mask)}
#define OPENKACHE_SMITHY_SET_CONDITION_ANY_BITS ${c_unsigned_literal(contract.v1.set_condition_any_bits)}
#define OPENKACHE_SMITHY_SET_IF_ABSENT_BITS ${c_unsigned_literal(contract.v1.set_if_absent_flag)}
#define OPENKACHE_SMITHY_SET_IF_PRESENT_BITS ${c_unsigned_literal(contract.v1.set_if_present_flag)}
#define OPENKACHE_SMITHY_SET_CONDITION_RESERVED_BITS ${c_unsigned_literal(contract.v1.set_condition_reserved_bits)}
#define OPENKACHE_SMITHY_SET_EXPIRATION_MASK ${c_unsigned_literal(contract.v1.set_expiration_mask)}
#define OPENKACHE_SMITHY_SET_INHERIT_EXPIRATION_BITS ${c_unsigned_literal(contract.v1.set_inherit_expiration_bits)}
#define OPENKACHE_SMITHY_SET_NO_EXPIRY_BITS ${c_unsigned_literal(contract.v1.set_no_expiry_bits)}
#define OPENKACHE_SMITHY_SET_EXPLICIT_TTL_BITS ${c_unsigned_literal(contract.v1.set_ttl_flag)}
#define OPENKACHE_SMITHY_SET_EXPIRATION_RESERVED_BITS ${c_unsigned_literal(contract.v1.set_expiration_reserved_bits)}
#define OPENKACHE_SMITHY_SET_EVICTION_MASK ${c_unsigned_literal(contract.v1.set_eviction_mask)}
#define OPENKACHE_SMITHY_SET_INHERIT_EVICTION_BITS ${c_unsigned_literal(contract.v1.set_inherit_eviction_bits)}
#define OPENKACHE_SMITHY_SET_EVICTABLE_BITS ${c_unsigned_literal(contract.v1.set_evictable_bits)}
#define OPENKACHE_SMITHY_SET_EVICTION_PROTECTED_BITS ${c_unsigned_literal(contract.v1.set_eviction_protected_bits)}
#define OPENKACHE_SMITHY_SET_EVICTION_RESERVED_BITS ${c_unsigned_literal(contract.v1.set_eviction_reserved_bits)}
#define OPENKACHE_SMITHY_SET_RESERVED_MASK ${c_unsigned_literal(contract.v1.set_reserved_mask)}
#define OPENKACHE_SMITHY_OPEN_FLAGS_BYTES ${contract.v1.open_flags_bytes}u
#define OPENKACHE_SMITHY_OPEN_CREATE_IF_MISSING ${c_unsigned_literal(contract.v1.open_create_if_missing_flag)}
#define OPENKACHE_SMITHY_OPEN_RESERVED_MASK ${c_unsigned_literal(contract.v1.open_reserved_mask)}
#define OPENKACHE_SMITHY_DELETE_FLAGS_BYTES ${contract.v1.delete_flags_bytes}u
#define OPENKACHE_SMITHY_DELETE_IF_EMPTY ${c_unsigned_literal(contract.v1.delete_if_empty_bits)}
#define OPENKACHE_SMITHY_DELETE_MODE_MASK ${c_unsigned_literal(contract.v1.delete_mode_mask)}
#define OPENKACHE_SMITHY_DELETE_RESERVED_MASK ${c_unsigned_literal(contract.v1.delete_reserved_mask)}
#define OPENKACHE_SMITHY_POLICY_FLAGS_BYTES ${contract.v1.policy_flags_bytes}u
#define OPENKACHE_SMITHY_POLICY_DEFAULT_EXPIRATION_MASK ${c_unsigned_literal(contract.v1.policy_default_expiration_mask)}
#define OPENKACHE_SMITHY_POLICY_NO_EXPIRY ${c_unsigned_literal(contract.v1.policy_no_expiry_bits)}
#define OPENKACHE_SMITHY_POLICY_FIXED_TTL ${c_unsigned_literal(contract.v1.policy_fixed_ttl_bits)}
#define OPENKACHE_SMITHY_POLICY_DEFAULT_EXPIRATION_RESERVED_BITS ${c_unsigned_literal(contract.v1.policy_default_expiration_reserved_bits)}
#define OPENKACHE_SMITHY_POLICY_EXPIRATION_OVERRIDE ${c_unsigned_literal(contract.v1.policy_expiration_override_flag)}
#define OPENKACHE_SMITHY_POLICY_EVICTION_PROTECTED ${c_unsigned_literal(contract.v1.policy_eviction_protected_flag)}
#define OPENKACHE_SMITHY_POLICY_EVICTION_OVERRIDE ${c_unsigned_literal(contract.v1.policy_eviction_override_flag)}
#define OPENKACHE_SMITHY_POLICY_RESERVED_MASK ${c_unsigned_literal(contract.v1.policy_reserved_mask)}
#define OPENKACHE_SMITHY_ERROR_STATUS_MINIMUM ${c_unsigned_literal(contract.v1.error_status_minimum)}
#define OPENKACHE_SMITHY_DEFAULT_MAX_IN_FLIGHT ${defaults.max_in_flight}u
#define OPENKACHE_SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS ${defaults.connect_timeout_milliseconds}u
#define OPENKACHE_SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS ${defaults.request_timeout_milliseconds}u
#define OPENKACHE_SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS ${defaults.retry_max_attempts}u
#define OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_LEVEL ${defaults.zstandard_level}u
#define OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES ${defaults.zstandard_minimum_input_bytes}u
#define OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES ${defaults.zstandard_minimum_savings_bytes}u
#define OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN ${defaults.zstandard_level_min}u
#define OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX ${defaults.zstandard_level_max}u
#define OPENKACHE_SMITHY_CLIENT_DEFAULT_SERVER_NAME ${c_string_literal(defaults.server_name)}
#define OPENKACHE_SMITHY_CLIENT_CERTIFICATE_PEM_TYPE ${c_string_literal(defaults.certificate_pem_type)}
#define OPENKACHE_SMITHY_CLIENT_MINIMUM_POSITIVE_VALUE ${defaults.minimum_positive_value}u
#define OPENKACHE_SMITHY_GATE0_ALPN_VERSION ${gate0.alpn_version}u
#define OPENKACHE_SMITHY_GATE0_COMPRESSION ${formatted_byte(gate0.compression)}u
#define OPENKACHE_SMITHY_GATE0_ENCRYPTION ${formatted_byte(gate0.encryption)}u
#define OPENKACHE_SMITHY_GATE0_ITEM_ID_ROOT_KEY_BYTES ${gate0_item_id_root_bytes}
#define OPENKACHE_SMITHY_GATE0_ITEM_ID_ROOT_KEY_LENGTH ${gate0_item_id_root.length}u
#define OPENKACHE_SMITHY_GATE0_NAMESPACE_ID ${gate0.namespace_id}u
#define OPENKACHE_SMITHY_GATE0_VALUE_SELECTOR ${formatted_byte(gate0.value_selector)}u
${ffi_defines}
#define OPENKACHE_SMITHY_VALUE_FORMAT_VERSION ${value.version}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_MAX_VU128_BYTES ${value.max_vu128_bytes}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_FORMAT_BYTE_BYTES ${value.format_byte_bytes}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_COMPRESSION_MASK ${formatted_byte(value.format_compression_mask)}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_ENCRYPTION_SHIFT ${formatted_byte(value.format_encryption_shift)}u
#define OPENKACHE_SMITHY_VALUE_SERIALIZATION_RAW ${formatted_byte(value.serialization_raw)}u
#define OPENKACHE_SMITHY_VALUE_SERIALIZATION_JSON ${formatted_byte(value.serialization_json)}u
#define OPENKACHE_SMITHY_VALUE_SERIALIZATION_STRUCTURED ${formatted_byte(value.serialization_structured)}u
#define OPENKACHE_SMITHY_VALUE_COMPRESSION_NONE ${formatted_byte(value.compression_none)}u
#define OPENKACHE_SMITHY_VALUE_COMPRESSION_ZSTANDARD ${formatted_byte(value.compression_zstandard)}u
#define OPENKACHE_SMITHY_VALUE_ENCRYPTION_NONE ${formatted_byte(value.encryption_none)}u
#define OPENKACHE_SMITHY_VALUE_ENCRYPTION_COMPACT ${formatted_byte(value.encryption_compact)}u
#define OPENKACHE_SMITHY_VALUE_ENCRYPTION_ROBUST ${formatted_byte(value.encryption_robust)}u
#define OPENKACHE_SMITHY_VALUE_COMPACT_SYNTHETIC_IV_BYTES ${value.compact_synthetic_iv_bytes}u
#define OPENKACHE_SMITHY_VALUE_ROBUST_NONCE_BYTES ${value.robust_nonce_bytes}u
#define OPENKACHE_SMITHY_VALUE_ROBUST_TAG_BYTES ${value.robust_tag_bytes}u
#define OPENKACHE_SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES ${value.data_protection_key_bytes}u
#define OPENKACHE_SMITHY_VALUE_ENVELOPE_MAX_ENCODING_BYTES ${envelope.max_encoding_bytes}u
#define OPENKACHE_SMITHY_VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES ${envelope.max_type_name_bytes}u

${client_compatibility}

${operation_enum}

${status_enum}

${c_contract_client_operation_enum(contract)}

/* Smithy string-enum values used by the language-neutral set API. */
${c_contract_api_enum(contract, "SetCondition", "OPENKACHE_SMITHY_SET_CONDITION")}
${c_contract_api_enum(contract, "SetOutcome", "OPENKACHE_SMITHY_SET_OUTCOME")}

#endif
`
}
