//! Python API and contract rendering.

import type { Api_Type, Client_Contract } from "./model"
import {
  bytes_from_hex,
  encode_vu128,
  pascal_case,
  snake_case,
} from "./utils"

function python_api_name(identifier: string): string {
  return `Smithy${pascal_case(snake_case(identifier))}`
}

function python_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "bytes"
      break
    case "boolean":
      rendered = "bool"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = python_api_name(type.name)
      break
    case "integer":
      rendered = "int"
      break
    case "long":
      rendered = "int"
      break
    case "structure":
      if (type.name === undefined) throw new Error("structure API type has no name")
      rendered = python_api_name(type.name)
      break
    case "string":
      rendered = "str"
      break
    case "unsigned_long":
      rendered = "int"
      break
  }
  return required ? rendered : `${rendered} | None`
}

/** Renders Smithy operation types and a Python async protocol interface.
 *
 * @param contract - Validated language-neutral wire and API contract.
 * @returns Deterministic Python source with a trailing newline.
 */
export function render_python_api(contract: Client_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const members = enum_.members
      .map(
        (member) =>
          `    ${snake_case(member.name).toUpperCase()} = ${JSON.stringify(member.value)}`,
      )
      .join("\n")
    return `class ${python_api_name(enum_.name)}(str, Enum):
    """Values defined by the Smithy ${enum_.name} shape."""

${members}`
  })
  const structures = contract.api.structures.map((structure) => {
    // Dataclasses require non-default fields before default fields. Smithy
    // member order is not a source-level guarantee, so keep required members
    // first while preserving each group's model order.
    const ordered_members = [...structure.members].sort(
      (left, right) => Number(!left.required) - Number(!right.required),
    )
    const members = ordered_members.map((member) => {
      const default_value = member.required ? "" : " = None"
      return `    ${snake_case(member.name)}: ${python_api_type(member.type, member.required)}${default_value}`
    })
    const body = members.length === 0 ? "    pass" : members.join("\n")
    return `@dataclass(frozen=True, slots=True)
class ${python_api_name(structure.name)}:
    """Smithy ${structure.name} structure."""

${body}`
  })
  const operations = contract.api.operations
    .map(
      (operation) =>
        `    async def ${snake_case(operation.name)}(
        self, input: ${python_api_name(operation.input)}
    ) -> ${python_api_name(operation.output)}: ...`,
    )
    .join("\n")
  return `# Generated from the OpenKache Smithy contract. Do not edit.

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Protocol

${[...enums, ...structures].join("\n\n")}


class SmithyOpenKacheApi(Protocol):
    """Async operations defined by the OpenKache Smithy service."""

${operations}
`
}

/** Renders the Python constants shared with the core-backed adapter.
 *
 * @param contract - Validated language-neutral wire and value-format contract.
 * @returns Deterministic Python source with a trailing newline.
 */
export function render_python_contract(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  const envelope = contract.value_envelope
  const descriptor_layout = contract.ffi.namespace_descriptor_layout
  const descriptor_fields = contract.ffi.namespace_descriptor_fields
  const version_bytes = encode_vu128(value.version)
  const magic = bytes_from_hex(envelope.magic_and_version_hex, "value envelope magic")
  const ffi_operations = contract.ffi.operations
    .map(
      (entry) =>
        `SMITHY_FFI_OPERATION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_result_kinds = contract.ffi.result_kinds
    .map(
      (entry) =>
        `SMITHY_FFI_RESULT_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_status_categories = contract.ffi.status_categories
    .map(
      (entry) =>
        `SMITHY_FFI_STATUS_CATEGORY_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_error_categories = contract.ffi.error_categories
    .map(
      (entry) =>
        `SMITHY_FFI_ERROR_CATEGORY_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_connection_states = contract.ffi.connection_states
    .map(
      (entry) =>
        `SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()} = ${entry.value}
SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()}_NAME = ${JSON.stringify(entry.text)}`,
    )
    .join("\n")
  const ffi_transports = contract.ffi.transports
    .map(
      (entry) =>
        `SMITHY_FFI_TRANSPORT_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_set_conditions = contract.ffi.set_conditions
    .map(
      (entry) =>
        `SMITHY_FFI_SET_CONDITION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_namespace_descriptor_decode_statuses =
    contract.ffi.namespace_descriptor_decode_statuses
      .map(
        (entry) =>
          `SMITHY_FFI_NAMESPACE_DESCRIPTOR_DECODE_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
      )
      .join("\n")
  const ffi_namespace_default_expirations = contract.ffi.namespace_default_expirations
    .map(
      (entry) =>
        `SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_namespace_default_evictions = contract.ffi.namespace_default_evictions
    .map(
      (entry) =>
        `SMITHY_FFI_NAMESPACE_DEFAULT_EVICTION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_namespace_override_policies = contract.ffi.namespace_override_policies
    .map(
      (entry) =>
        `SMITHY_FFI_NAMESPACE_OVERRIDE_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const opcodes = contract.opcodes
    .map((entry) => `SMITHY_OPCODE_${snake_case(entry.name).toUpperCase()} = ${entry.value}`)
    .join("\n")
  const statuses = contract.statuses
    .map((entry) => `SMITHY_STATUS_${snake_case(entry.name).toUpperCase()} = ${entry.value}`)
    .join("\n")
  const python_namespace_descriptor_fields = descriptor_fields.map(
    (field) => `        ("${field.name}", ${field.python_type}),`,
  ).join("\n")
  const python_descriptor_offsets = descriptor_fields
    .map(
      (field) =>
        `SMITHY_FFI_NAMESPACE_DESCRIPTOR_${snake_case(field.name).toUpperCase()}_OFFSET = ${field.offset}`,
    )
    .join("\n")
  return `# Generated from the OpenKache Smithy contract. Do not edit.

import ctypes as _ctypes

class SmithyFFINamespaceDescriptor(_ctypes.Structure):
    """C-compatible namespace descriptor returned by the native ABI decoder."""

    _fields_ = [
${python_namespace_descriptor_fields}
    ]

SMITHY_PROTOCOL_ALPN = ${JSON.stringify(contract.v1.alpn)}
SMITHY_OPCODE_BYTES = ${contract.v1.opcode_bytes}
SMITHY_STATUS_BYTES = ${contract.v1.status_bytes}
SMITHY_REQUEST_FIXED_BYTES = ${contract.v1.request_fixed_bytes}
SMITHY_RESPONSE_FIXED_BYTES = ${contract.v1.response_fixed_bytes}
SMITHY_MIN_VARUINT_BYTES = ${contract.v1.min_varuint_bytes}
SMITHY_MAX_VARUINT_BYTES = ${contract.v1.max_varuint_bytes}
SMITHY_ITEM_ID_BYTES = ${contract.item_id_bytes}
SMITHY_MAX_VALUE_BYTES = ${contract.max_value_bytes}
SMITHY_NAMESPACE_ID_BYTES = ${contract.v1.namespace_id_bytes}
SMITHY_NAMESPACE_REVISION_BYTES = ${contract.v1.namespace_revision_bytes}
SMITHY_NAMESPACE_NAME_LENGTH_BYTES = ${contract.v1.namespace_name_length_bytes}
SMITHY_NAMESPACE_NAME_MAX_BYTES = ${contract.v1.namespace_name_max_bytes}
SMITHY_SET_FLAGS_BYTES = ${contract.v1.set_flags_bytes}
SMITHY_SET_CONDITION_MASK = ${contract.v1.set_condition_mask}
SMITHY_SET_CONDITION_ANY_BITS = ${contract.v1.set_condition_any_bits}
SMITHY_SET_IF_ABSENT_BITS = ${contract.v1.set_if_absent_flag}
SMITHY_SET_IF_PRESENT_BITS = ${contract.v1.set_if_present_flag}
SMITHY_SET_CONDITION_RESERVED_BITS = ${contract.v1.set_condition_reserved_bits}
SMITHY_SET_EXPIRATION_MASK = ${contract.v1.set_expiration_mask}
SMITHY_SET_INHERIT_EXPIRATION_BITS = ${contract.v1.set_inherit_expiration_bits}
SMITHY_SET_NO_EXPIRY_BITS = ${contract.v1.set_no_expiry_bits}
SMITHY_SET_EXPLICIT_TTL_BITS = ${contract.v1.set_ttl_flag}
SMITHY_SET_EXPIRATION_RESERVED_BITS = ${contract.v1.set_expiration_reserved_bits}
SMITHY_SET_EVICTION_MASK = ${contract.v1.set_eviction_mask}
SMITHY_SET_INHERIT_EVICTION_BITS = ${contract.v1.set_inherit_eviction_bits}
SMITHY_SET_EVICTABLE_BITS = ${contract.v1.set_evictable_bits}
SMITHY_SET_EVICTION_PROTECTED_BITS = ${contract.v1.set_eviction_protected_bits}
SMITHY_SET_EVICTION_RESERVED_BITS = ${contract.v1.set_eviction_reserved_bits}
SMITHY_SET_RESERVED_MASK = ${contract.v1.set_reserved_mask}
SMITHY_OPEN_FLAGS_BYTES = ${contract.v1.open_flags_bytes}
SMITHY_OPEN_CREATE_IF_MISSING = ${contract.v1.open_create_if_missing_flag}
SMITHY_OPEN_RESERVED_MASK = ${contract.v1.open_reserved_mask}
SMITHY_DELETE_FLAGS_BYTES = ${contract.v1.delete_flags_bytes}
SMITHY_DELETE_IF_EMPTY = ${contract.v1.delete_if_empty_bits}
SMITHY_DELETE_MODE_MASK = ${contract.v1.delete_mode_mask}
SMITHY_DELETE_RESERVED_MASK = ${contract.v1.delete_reserved_mask}
SMITHY_POLICY_FLAGS_BYTES = ${contract.v1.policy_flags_bytes}
SMITHY_POLICY_DEFAULT_EXPIRATION_MASK = ${contract.v1.policy_default_expiration_mask}
SMITHY_POLICY_NO_EXPIRY = ${contract.v1.policy_no_expiry_bits}
SMITHY_POLICY_FIXED_TTL = ${contract.v1.policy_fixed_ttl_bits}
SMITHY_POLICY_DEFAULT_EXPIRATION_RESERVED_BITS = ${contract.v1.policy_default_expiration_reserved_bits}
SMITHY_POLICY_EXPIRATION_OVERRIDE = ${contract.v1.policy_expiration_override_flag}
SMITHY_POLICY_EVICTION_PROTECTED = ${contract.v1.policy_eviction_protected_flag}
SMITHY_POLICY_EVICTION_OVERRIDE = ${contract.v1.policy_eviction_override_flag}
SMITHY_POLICY_RESERVED_MASK = ${contract.v1.policy_reserved_mask}
SMITHY_ERROR_STATUS_MINIMUM = ${contract.v1.error_status_minimum}
SMITHY_DEFAULT_MAX_IN_FLIGHT = ${defaults.max_in_flight}
SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS = ${defaults.connect_timeout_milliseconds}
SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS = ${defaults.request_timeout_milliseconds}
SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS = ${defaults.retry_max_attempts}
SMITHY_DEFAULT_ZSTANDARD_LEVEL = ${defaults.zstandard_level}
SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES = ${defaults.zstandard_minimum_input_bytes}
SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES = ${defaults.zstandard_minimum_savings_bytes}
SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN = ${defaults.zstandard_level_min}
SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX = ${defaults.zstandard_level_max}
SMITHY_CLIENT_DEFAULT_SERVER_NAME = ${JSON.stringify(defaults.server_name)}
SMITHY_CLIENT_CERTIFICATE_PEM_TYPE = ${JSON.stringify(defaults.certificate_pem_type)}
SMITHY_CLIENT_MINIMUM_POSITIVE_VALUE = ${defaults.minimum_positive_value}
SMITHY_FFI_ABI_VERSION = ${contract.ffi.abi_version}
${ffi_operations}
${ffi_result_kinds}
${ffi_status_categories}
${ffi_error_categories}
${ffi_connection_states}
${ffi_transports}
${ffi_set_conditions}
${ffi_namespace_descriptor_decode_statuses}
${ffi_namespace_default_expirations}
${ffi_namespace_default_evictions}
${ffi_namespace_override_policies}
SMITHY_FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES = ${descriptor_layout.size_bytes}
${python_descriptor_offsets}
SMITHY_SET_TTL_FLAG = ${contract.v1.set_ttl_flag}
SMITHY_SET_IF_ABSENT_FLAG = ${contract.v1.set_if_absent_flag}
SMITHY_SET_IF_PRESENT_FLAG = ${contract.v1.set_if_present_flag}
SMITHY_VALUE_FORMAT_VERSION = ${value.version}
SMITHY_VALUE_FORMAT_VERSION_BYTES = bytes([${version_bytes.join(", ")}])
SMITHY_VALUE_FORMAT_MAX_VU128_BYTES = ${value.max_vu128_bytes}
SMITHY_VALUE_FORMAT_FORMAT_BYTE_BYTES = ${value.format_byte_bytes}
SMITHY_VALUE_FORMAT_COMPRESSION_MASK = ${value.format_compression_mask}
SMITHY_VALUE_FORMAT_ENCRYPTION_SHIFT = ${value.format_encryption_shift}
SMITHY_VALUE_SERIALIZATION_RAW = ${value.serialization_raw}
SMITHY_VALUE_SERIALIZATION_JSON = ${value.serialization_json}
SMITHY_VALUE_COMPRESSION_NONE = ${value.compression_none}
SMITHY_VALUE_COMPRESSION_ZSTANDARD = ${value.compression_zstandard}
SMITHY_VALUE_ENCRYPTION_NONE = ${value.encryption_none}
SMITHY_VALUE_ENCRYPTION_COMPACT = ${value.encryption_compact}
SMITHY_VALUE_ENCRYPTION_ROBUST = ${value.encryption_robust}
SMITHY_VALUE_COMPACT_SYNTHETIC_IV_BYTES = ${value.compact_synthetic_iv_bytes}
SMITHY_VALUE_ROBUST_NONCE_BYTES = ${value.robust_nonce_bytes}
SMITHY_VALUE_ROBUST_TAG_BYTES = ${value.robust_tag_bytes}
SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES = ${value.data_protection_key_bytes}
SMITHY_VALUE_ITEM_ID_ROOT_CONTEXT = ${JSON.stringify(value.item_id_root_context)}
SMITHY_VALUE_AAD_DOMAIN = ${JSON.stringify(value.aad_domain)}
SMITHY_VALUE_VALUE_ROOT_CONTEXT = ${JSON.stringify(value.value_root_context)}
SMITHY_VALUE_COMPACT_MAC_CONTEXT = ${JSON.stringify(value.compact_mac_context)}
SMITHY_VALUE_COMPACT_ENCRYPTION_CONTEXT = ${JSON.stringify(value.compact_encryption_context)}
SMITHY_VALUE_ROBUST_CONTEXT = ${JSON.stringify(value.robust_context)}
SMITHY_VALUE_ENVELOPE_MAGIC_AND_VERSION = bytes([${magic.join(", ")}])
SMITHY_VALUE_ENVELOPE_MAX_ENCODING_BYTES = ${envelope.max_encoding_bytes}
SMITHY_VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES = ${envelope.max_type_name_bytes}
SMITHY_VALUE_ENVELOPE_JSON_ENCODING = ${JSON.stringify(envelope.json_encoding)}

${opcodes}
${statuses}
`
}
