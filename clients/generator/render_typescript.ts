//! TypeScript API model rendering.

import type { Api_Type, Client_Contract } from "./model"
import { snake_case, typescript_api_name } from "./utils"

function typescript_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "Uint8Array"
      break
    case "boolean":
      rendered = "boolean"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = typescript_api_name(type.name)
      break
    case "integer":
      rendered = "number"
      break
    case "long":
      rendered = "number"
      break
    case "structure":
      if (type.name === undefined) throw new Error("structure API type has no name")
      rendered = typescript_api_name(type.name)
      break
    case "string":
      rendered = "string"
      break
    case "unsigned_long":
      rendered = "bigint"
      break
  }
  return required ? rendered : `${rendered} | undefined`
}

/** Renders Smithy operation types and an API interface for TypeScript.
 *
 * @param contract - Validated language-neutral wire and API contract.
 * @returns Deterministic TypeScript source with a trailing newline.
 */
export function render_typescript_api(contract: Client_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const values = enum_.members.map((member) => JSON.stringify(member.value)).join(" | ")
    return `/** Values defined by the Smithy ${enum_.name} shape. */
export type ${typescript_api_name(enum_.name)} = ${values}`
  })
  const structures = contract.api.structures.map((structure) => {
    const members = structure.members.map((member) => {
      const optional = member.required ? "" : "?"
      return `  /** Smithy ${member.name} member. */
  readonly ${snake_case(member.name)}${optional}: ${typescript_api_type(member.type, member.required)}`
    })
    return `/** Smithy ${structure.name} structure. */
export interface ${typescript_api_name(structure.name)} {
${members.join("\n")}
}`
  })
  const operations = contract.api.operations.map(
    (operation) =>
      `  /** Invokes the Smithy ${operation.name} operation. */
  ${snake_case(operation.name)}(input: ${typescript_api_name(operation.input)}): Promise<${typescript_api_name(operation.output)}>`,
  )
  const enum_constants = contract.api.enums
    .flatMap((enum_) =>
      enum_.members.map(
        (member) =>
          `/** Smithy ${enum_.name} value ${member.name}. */
export const SMITHY_${snake_case(enum_.name).toUpperCase()}_${snake_case(member.name).toUpperCase()} = ${JSON.stringify(member.value)} as const`,
      ),
    )
    .join("\n")
  const descriptor_offsets = contract.ffi.namespace_descriptor_fields
    .map(
      (field) =>
        `export const SMITHY_FFI_NAMESPACE_DESCRIPTOR_${snake_case(field.name).toUpperCase()}_OFFSET = ${field.offset}`,
    )
    .join("\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.

/** Exact number of bytes in a protocol item identifier. */
export const SMITHY_ITEM_ID_BYTES = ${contract.item_id_bytes}
/** Maximum opaque value bytes accepted by the protocol. */
export const SMITHY_MAX_VALUE_BYTES = ${contract.max_value_bytes}
/** Width of the request opcode and response status fields. */
export const SMITHY_OPCODE_BYTES = ${contract.v1.opcode_bytes}
export const SMITHY_STATUS_BYTES = ${contract.v1.status_bytes}
/** Fixed request/response prefix widths and unsigned integer ceiling. */
export const SMITHY_REQUEST_FIXED_BYTES = ${contract.v1.request_fixed_bytes}
export const SMITHY_RESPONSE_FIXED_BYTES = ${contract.v1.response_fixed_bytes}
export const SMITHY_MIN_VARUINT_BYTES = ${contract.v1.min_varuint_bytes}
export const SMITHY_MAX_VARUINT_BYTES = ${contract.v1.max_varuint_bytes}
/** Namespace identity, revision, and name constraints. */
export const SMITHY_NAMESPACE_ID_BYTES = ${contract.v1.namespace_id_bytes}
export const SMITHY_NAMESPACE_REVISION_BYTES = ${contract.v1.namespace_revision_bytes}
export const SMITHY_NAMESPACE_NAME_LENGTH_BYTES = ${contract.v1.namespace_name_length_bytes}
export const SMITHY_NAMESPACE_NAME_MAX_BYTES = ${contract.v1.namespace_name_max_bytes}
/** SET flag masks and values. */
export const SMITHY_SET_FLAGS_BYTES = ${contract.v1.set_flags_bytes}
export const SMITHY_SET_CONDITION_MASK = ${contract.v1.set_condition_mask}
export const SMITHY_SET_CONDITION_ANY_BITS = ${contract.v1.set_condition_any_bits}
export const SMITHY_SET_IF_ABSENT_BITS = ${contract.v1.set_if_absent_flag}
export const SMITHY_SET_IF_PRESENT_BITS = ${contract.v1.set_if_present_flag}
export const SMITHY_SET_CONDITION_RESERVED_BITS = ${contract.v1.set_condition_reserved_bits}
export const SMITHY_SET_EXPIRATION_MASK = ${contract.v1.set_expiration_mask}
export const SMITHY_SET_INHERIT_EXPIRATION_BITS = ${contract.v1.set_inherit_expiration_bits}
export const SMITHY_SET_NO_EXPIRY_BITS = ${contract.v1.set_no_expiry_bits}
export const SMITHY_SET_EXPLICIT_TTL_BITS = ${contract.v1.set_ttl_flag}
export const SMITHY_SET_EXPIRATION_RESERVED_BITS = ${contract.v1.set_expiration_reserved_bits}
export const SMITHY_SET_EVICTION_MASK = ${contract.v1.set_eviction_mask}
export const SMITHY_SET_INHERIT_EVICTION_BITS = ${contract.v1.set_inherit_eviction_bits}
export const SMITHY_SET_EVICTABLE_BITS = ${contract.v1.set_evictable_bits}
export const SMITHY_SET_EVICTION_PROTECTED_BITS = ${contract.v1.set_eviction_protected_bits}
export const SMITHY_SET_EVICTION_RESERVED_BITS = ${contract.v1.set_eviction_reserved_bits}
export const SMITHY_SET_RESERVED_MASK = ${contract.v1.set_reserved_mask}
/** Namespace-management flags. */
export const SMITHY_OPEN_FLAGS_BYTES = ${contract.v1.open_flags_bytes}
export const SMITHY_OPEN_CREATE_IF_MISSING = ${contract.v1.open_create_if_missing_flag}
export const SMITHY_OPEN_RESERVED_MASK = ${contract.v1.open_reserved_mask}
export const SMITHY_DELETE_FLAGS_BYTES = ${contract.v1.delete_flags_bytes}
export const SMITHY_DELETE_IF_EMPTY = ${contract.v1.delete_if_empty_bits}
export const SMITHY_DELETE_MODE_MASK = ${contract.v1.delete_mode_mask}
export const SMITHY_DELETE_RESERVED_MASK = ${contract.v1.delete_reserved_mask}
/** Namespace-policy flags and error boundary. */
export const SMITHY_POLICY_FLAGS_BYTES = ${contract.v1.policy_flags_bytes}
export const SMITHY_POLICY_DEFAULT_EXPIRATION_MASK = ${contract.v1.policy_default_expiration_mask}
export const SMITHY_POLICY_NO_EXPIRY = ${contract.v1.policy_no_expiry_bits}
export const SMITHY_POLICY_FIXED_TTL = ${contract.v1.policy_fixed_ttl_bits}
export const SMITHY_POLICY_DEFAULT_EXPIRATION_RESERVED_BITS = ${contract.v1.policy_default_expiration_reserved_bits}
export const SMITHY_POLICY_EXPIRATION_OVERRIDE = ${contract.v1.policy_expiration_override_flag}
export const SMITHY_POLICY_EVICTION_PROTECTED = ${contract.v1.policy_eviction_protected_flag}
export const SMITHY_POLICY_EVICTION_OVERRIDE = ${contract.v1.policy_eviction_override_flag}
export const SMITHY_POLICY_RESERVED_MASK = ${contract.v1.policy_reserved_mask}
export const SMITHY_ERROR_STATUS_MINIMUM = ${contract.v1.error_status_minimum}
/** Native ABI discriminators and namespace descriptor values. */
export const SMITHY_FFI_ABI_VERSION = ${contract.ffi.abi_version}
${contract.ffi.operations
  .map(
    (entry) =>
      `export const SMITHY_FFI_OPERATION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.result_kinds
  .map(
    (entry) =>
      `export const SMITHY_FFI_RESULT_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.connection_states
  .map(
    (entry) =>
      `export const SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()} = ${entry.value}
export const SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()}_NAME = ${JSON.stringify(entry.text)} as const`,
  )
  .join("\n")}
${contract.ffi.set_conditions
  .map(
    (entry) =>
      `export const SMITHY_FFI_SET_CONDITION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.namespace_descriptor_decode_statuses
  .map(
    (entry) =>
      `export const SMITHY_FFI_NAMESPACE_DESCRIPTOR_DECODE_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.namespace_default_expirations
  .map(
    (entry) =>
      `export const SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.namespace_default_evictions
  .map(
    (entry) =>
      `export const SMITHY_FFI_NAMESPACE_DEFAULT_EVICTION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.namespace_override_policies
  .map(
    (entry) =>
      `export const SMITHY_FFI_NAMESPACE_OVERRIDE_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
  )
  .join("\n")}
export const SMITHY_FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES = ${contract.ffi.namespace_descriptor_layout.size_bytes}
${descriptor_offsets}
/** Default maximum number of concurrent request lanes. */
export const SMITHY_DEFAULT_MAX_IN_FLIGHT = ${contract.client_defaults.max_in_flight}
/** Default connection-establishment timeout in milliseconds. */
export const SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS = ${contract.client_defaults.connect_timeout_milliseconds}
/** Default complete-request timeout in milliseconds. */
export const SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS = ${contract.client_defaults.request_timeout_milliseconds}
/** Default maximum total attempts for response-safe operations. */
export const SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS = ${contract.client_defaults.retry_max_attempts}
/** Default Zstandard compression level. */
export const SMITHY_DEFAULT_ZSTANDARD_LEVEL = ${contract.client_defaults.zstandard_level}
/** Default minimum serialized input size considered for Zstandard compression. */
export const SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES = ${contract.client_defaults.zstandard_minimum_input_bytes}
/** Default minimum Zstandard savings required to retain compression. */
export const SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES = ${contract.client_defaults.zstandard_minimum_savings_bytes}
/** Inclusive minimum supported Zstandard compression level. */
export const SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN = ${contract.client_defaults.zstandard_level_min}
/** Inclusive maximum supported Zstandard compression level. */
export const SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX = ${contract.client_defaults.zstandard_level_max}
/** Default TLS server name used when no explicit name is supplied. */
export const SMITHY_CLIENT_DEFAULT_SERVER_NAME = ${JSON.stringify(contract.client_defaults.server_name)}
/** PEM label used for adapter-assembled certificate chains. */
export const SMITHY_CLIENT_CERTIFICATE_PEM_TYPE = ${JSON.stringify(contract.client_defaults.certificate_pem_type)}
/** Minimum positive setting value when zero selects a default. */
export const SMITHY_CLIENT_MINIMUM_POSITIVE_VALUE = ${contract.client_defaults.minimum_positive_value}

${[...enums, ...structures].join("\n\n")}

${enum_constants}

/** Operations defined by the OpenKache Smithy service. */
export interface Smithy_OpenKache_Api {
${operations.join("\n")}
}
`
}
