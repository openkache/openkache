//! C ABI contract rendering.

import type { Wire_Entry } from "../../protocol/wire"
import type { Client_Contract } from "./model"
import {
  c_string_literal,
  c_unsigned_literal,
  formatted_byte,
  snake_case,
} from "./utils"

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

/** Renders the Smithy constants consumed by native C and C++ adapters.
 *
 * @param contract - Validated language-neutral wire and value-format contract.
 * @returns Deterministic C declarations with a trailing newline.
 */
export function render_c_contract(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
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
    ...ffi.connection_states.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.set_conditions.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_SET_CONDITION_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
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
  const descriptor_offset_defines = descriptor_fields
    .map(
      (field) =>
        `#define OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_${snake_case(field.name).toUpperCase()}_OFFSET ${field.offset}u`,
    )
    .join("\n")
  const descriptor_offset_asserts = descriptor_fields
    .map(
      (field) =>
        `_Static_assert(offsetof(openkache_smithy_namespace_descriptor_t, ${field.name}) ==\n                   OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_${snake_case(field.name).toUpperCase()}_OFFSET,\n               "Smithy namespace descriptor ${field.name} offset changed");`,
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

_Static_assert(sizeof(openkache_smithy_namespace_descriptor_t) ==
                   OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES,
               "Smithy namespace descriptor size changed");
${descriptor_offset_asserts}

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
${ffi_defines}
#define OPENKACHE_SMITHY_VALUE_FORMAT_VERSION ${value.version}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_MAX_VU128_BYTES ${value.max_vu128_bytes}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_FORMAT_BYTE_BYTES ${value.format_byte_bytes}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_COMPRESSION_MASK ${formatted_byte(value.format_compression_mask)}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_ENCRYPTION_SHIFT ${formatted_byte(value.format_encryption_shift)}u
#define OPENKACHE_SMITHY_VALUE_SERIALIZATION_RAW ${formatted_byte(value.serialization_raw)}u
#define OPENKACHE_SMITHY_VALUE_SERIALIZATION_JSON ${formatted_byte(value.serialization_json)}u
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

${operation_enum}

${status_enum}

/* Smithy string-enum values used by the language-neutral set API. */
${c_contract_api_enum(contract, "SetCondition", "OPENKACHE_SMITHY_SET_CONDITION")}
${c_contract_api_enum(contract, "SetOutcome", "OPENKACHE_SMITHY_SET_OUTCOME")}

#endif
`
}
