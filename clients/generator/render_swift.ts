//! Swift API and contract rendering.

import type { Api_Type, Client_Contract, Ffi_Entry } from "./model"
import {
  encode_vu128,
  pascal_case,
  swift_name,
  swift_property_name,
  typescript_name,
} from "./utils"

function swift_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "Data"
      break
    case "boolean":
      rendered = "Bool"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = `Smithy_${typescript_name(type.name)}`
      break
    case "integer":
      rendered = "Int32"
      break
    case "long":
      rendered = "Int64"
      break
    case "structure":
      if (type.name === undefined) throw new Error("structure API type has no name")
      rendered = `Smithy_${typescript_name(type.name)}`
      break
    case "string":
      rendered = "String"
      break
    case "unsigned_long":
      rendered = "UInt64"
      break
  }
  return required ? rendered : `${rendered}?`
}

function swift_string_literal(value: string): string {
  let literal = '"'
  for (const character of value) {
    const code_point = character.codePointAt(0)
    if (code_point === undefined) continue
    switch (character) {
      case "\\":
        literal += "\\\\"
        break
      case '"':
        literal += '\\"'
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
      default:
        if (code_point >= 0x20 && code_point <= 0x7e) {
          literal += character
        } else {
          literal += `\\u{${code_point.toString(16)}}`
        }
    }
  }
  return `${literal}"`
}

/** Renders Smithy operation and shared contract declarations for Swift.
 *
 * @param contract - Validated language-neutral wire, value, and FFI contract.
 * @returns Deterministic Swift source with a trailing newline.
 */
export function render_swift_api(contract: Client_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const members = enum_.members
      .map(
        (member) =>
          `  case ${swift_property_name(member.name)} = ${swift_string_literal(member.value)}`,
      )
      .join("\n")
    return `/// Values defined by the Smithy ${enum_.name} shape.
public enum Smithy_${typescript_name(enum_.name)}: String, Equatable, Sendable {
${members}
}`
  })
  const structures = contract.api.structures.map((structure) => {
    const name = `Smithy_${typescript_name(structure.name)}`
    if (structure.members.length === 0) {
      return `/// Smithy ${structure.name} structure.
public struct ${name}: Equatable, Sendable {
  public init() {}
}`
    }
    const members = structure.members
      .map(
        (member) =>
          `  /// Smithy ${member.name} member.
  public let ${swift_property_name(member.name)}: ${swift_api_type(member.type, member.required)}`,
      )
      .join("\n")
    const parameters = structure.members
      .map((member) => {
        const default_value = member.required ? "" : " = nil"
        return `    ${swift_property_name(member.name)}: ${swift_api_type(member.type, member.required)}${default_value}`
      })
      .join(",\n")
    const assignments = structure.members
      .map(
        (member) =>
          `    self.${swift_property_name(member.name)} = ${swift_property_name(member.name)}`,
      )
      .join("\n")
    return `/// Smithy ${structure.name} structure.
public struct ${name}: Equatable, Sendable {
${members}

  public init(
${parameters}
  ) {
${assignments}
  }
}`
  })
  const operations = contract.api.operations
    .map(
      (operation) =>
        `  /// Invokes the Smithy ${operation.name} operation.
  func ${swift_property_name(operation.name)}(
    _ input: Smithy_${typescript_name(operation.input)}
  ) async throws -> Smithy_${typescript_name(operation.output)}`,
    )
    .join("\n")
  const opcodes = contract.opcodes
    .map(
      (opcode) =>
        `  case ${swift_property_name(opcode.name)} = ${opcode.value}`,
    )
    .join("\n")
  const value = contract.value_format
  const ffi = contract.ffi
  const version_bytes = encode_vu128(value.version)
  const descriptor_layout = ffi.namespace_descriptor_layout
  const swift_native_constants = [
    ["operation", ffi.operations],
    ["result", ffi.result_kinds],
    ["transport", ffi.transports],
    ["setCondition", ffi.set_conditions],
    ["namespaceDescriptorDecode", ffi.namespace_descriptor_decode_statuses],
    ["namespaceDefaultExpiration", ffi.namespace_default_expirations],
    ["namespaceDefaultEviction", ffi.namespace_default_evictions],
    ["namespaceOverride", ffi.namespace_override_policies],
  ]
    .flatMap(([prefix, entries]) =>
      (entries as readonly Ffi_Entry[]).map(
        (entry) =>
          `  public static let ${prefix}${entry.name}: UInt32 = ${entry.value}`,
      ),
    )
    .join("\n")
  const connection_states = ffi.connection_states
    .map(
      (entry) =>
        `  case ${swift_property_name(entry.name)} = ${entry.value}`,
    )
    .join("\n")
  const descriptor_fields = ffi.namespace_descriptor_fields
  const swift_namespace_descriptor_fields = descriptor_fields.map(
    (field) => `  var ${field.swift_name}: ${field.swift_type} = 0`,
  ).join("\n")
  const swift_descriptor_offsets = descriptor_fields
    .map(
      (field) =>
        `  public static let namespaceDescriptor${pascal_case(field.name)}Offset: Int = ${field.offset}`,
    )
    .join("\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.

import Foundation

${[...enums, ...structures].join("\n\n")}

/// Operations defined by the OpenKache Smithy service.
public protocol Smithy_OpenKache_Api: Sendable {
${operations}
}

/// Operation identifiers assigned by the Smithy wire contract.
public enum Smithy_Opcode: UInt8, Equatable, Sendable {
${opcodes}
}

/// Wire and value-format identifiers shared by all language bindings.
public enum Smithy_Value_Format: Sendable {
  public static let protocolAlpn: String = ${swift_string_literal(contract.v1.alpn)}
  public static let itemIdBytes: Int = ${contract.item_id_bytes}
  public static let maxValueBytes: Int = ${contract.max_value_bytes}
  public static let opcodeBytes: Int = ${contract.v1.opcode_bytes}
  public static let statusBytes: Int = ${contract.v1.status_bytes}
  public static let requestFixedBytes: Int = ${contract.v1.request_fixed_bytes}
  public static let responseFixedBytes: Int = ${contract.v1.response_fixed_bytes}
  public static let minVaruintBytes: Int = ${contract.v1.min_varuint_bytes}
  public static let maxVaruintBytes: Int = ${contract.v1.max_varuint_bytes}
  public static let namespaceIdBytes: Int = ${contract.v1.namespace_id_bytes}
  public static let namespaceRevisionBytes: Int = ${contract.v1.namespace_revision_bytes}
  public static let namespaceNameLengthBytes: Int = ${contract.v1.namespace_name_length_bytes}
  public static let namespaceNameMaxBytes: Int = ${contract.v1.namespace_name_max_bytes}
  public static let setFlagsBytes: Int = ${contract.v1.set_flags_bytes}
  public static let setConditionMask: UInt8 = ${contract.v1.set_condition_mask}
  public static let setConditionAnyBits: UInt8 = ${contract.v1.set_condition_any_bits}
  public static let setIfAbsentBits: UInt8 = ${contract.v1.set_if_absent_flag}
  public static let setIfPresentBits: UInt8 = ${contract.v1.set_if_present_flag}
  public static let setConditionReservedBits: UInt8 = ${contract.v1.set_condition_reserved_bits}
  public static let setExpirationMask: UInt8 = ${contract.v1.set_expiration_mask}
  public static let setInheritExpirationBits: UInt8 = ${contract.v1.set_inherit_expiration_bits}
  public static let setNoExpiryBits: UInt8 = ${contract.v1.set_no_expiry_bits}
  public static let setExplicitTtlBits: UInt8 = ${contract.v1.set_ttl_flag}
  public static let setExpirationReservedBits: UInt8 = ${contract.v1.set_expiration_reserved_bits}
  public static let setEvictionMask: UInt8 = ${contract.v1.set_eviction_mask}
  public static let setInheritEvictionBits: UInt8 = ${contract.v1.set_inherit_eviction_bits}
  public static let setEvictableBits: UInt8 = ${contract.v1.set_evictable_bits}
  public static let setEvictionProtectedBits: UInt8 = ${contract.v1.set_eviction_protected_bits}
  public static let setEvictionReservedBits: UInt8 = ${contract.v1.set_eviction_reserved_bits}
  public static let setReservedMask: UInt8 = ${contract.v1.set_reserved_mask}
  public static let openFlagsBytes: Int = ${contract.v1.open_flags_bytes}
  public static let openCreateIfMissing: UInt8 = ${contract.v1.open_create_if_missing_flag}
  public static let openReservedMask: UInt8 = ${contract.v1.open_reserved_mask}
  public static let deleteFlagsBytes: Int = ${contract.v1.delete_flags_bytes}
  public static let deleteIfEmpty: UInt8 = ${contract.v1.delete_if_empty_bits}
  public static let deleteModeMask: UInt8 = ${contract.v1.delete_mode_mask}
  public static let deleteReservedMask: UInt8 = ${contract.v1.delete_reserved_mask}
  public static let policyFlagsBytes: Int = ${contract.v1.policy_flags_bytes}
  public static let policyDefaultExpirationMask: UInt8 = ${contract.v1.policy_default_expiration_mask}
  public static let policyNoExpiry: UInt8 = ${contract.v1.policy_no_expiry_bits}
  public static let policyFixedTtl: UInt8 = ${contract.v1.policy_fixed_ttl_bits}
  public static let policyDefaultExpirationReservedBits: UInt8 = ${contract.v1.policy_default_expiration_reserved_bits}
  public static let policyExpirationOverride: UInt8 = ${contract.v1.policy_expiration_override_flag}
  public static let policyEvictionProtected: UInt8 = ${contract.v1.policy_eviction_protected_flag}
  public static let policyEvictionOverride: UInt8 = ${contract.v1.policy_eviction_override_flag}
  public static let policyReservedMask: UInt8 = ${contract.v1.policy_reserved_mask}
  public static let errorStatusMinimum: UInt8 = ${contract.v1.error_status_minimum}
  public static let defaultMaxInFlight: Int = ${contract.client_defaults.max_in_flight}
  public static let defaultConnectTimeoutMilliseconds: Int = ${contract.client_defaults.connect_timeout_milliseconds}
  public static let defaultRequestTimeoutMilliseconds: Int = ${contract.client_defaults.request_timeout_milliseconds}
  public static let defaultRetryMaxAttempts: Int = ${contract.client_defaults.retry_max_attempts}
  public static let defaultZstandardLevel: Int32 = ${contract.client_defaults.zstandard_level}
  public static let defaultZstandardMinimumInputBytes: Int = ${contract.client_defaults.zstandard_minimum_input_bytes}
  public static let defaultZstandardMinimumSavingsBytes: Int = ${contract.client_defaults.zstandard_minimum_savings_bytes}
  public static let defaultZstandardLevelMin: Int32 = ${contract.client_defaults.zstandard_level_min}
  public static let defaultZstandardLevelMax: Int32 = ${contract.client_defaults.zstandard_level_max}
  public static let defaultServerName: String = ${swift_string_literal(contract.client_defaults.server_name)}
  public static let certificatePemType: String = ${swift_string_literal(contract.client_defaults.certificate_pem_type)}
  public static let minimumPositiveValue: Int = ${contract.client_defaults.minimum_positive_value}
  public static let version: Int = ${value.version}
  public static let versionBytes: [UInt8] = [${version_bytes.join(", ")}]
  public static let maxVu128Bytes: Int = ${value.max_vu128_bytes}
  public static let formatByteBytes: Int = ${value.format_byte_bytes}
  public static let setTtlFlag: UInt8 = ${contract.v1.set_ttl_flag}
  public static let setIfAbsentFlag: UInt8 = ${contract.v1.set_if_absent_flag}
  public static let setIfPresentFlag: UInt8 = ${contract.v1.set_if_present_flag}
  public static let formatCompressionMask: UInt8 = ${value.format_compression_mask}
  public static let formatEncryptionShift: UInt8 = ${value.format_encryption_shift}
  public static let serializationRaw: UInt8 = ${value.serialization_raw}
  public static let serializationJson: UInt8 = ${value.serialization_json}
  public static let serializationStructured: UInt8 = ${value.serialization_structured}
  public static let compressionNone: UInt8 = ${value.compression_none}
  public static let compressionZstandard: UInt8 = ${value.compression_zstandard}
  public static let encryptionNone: UInt8 = ${value.encryption_none}
  public static let encryptionCompact: UInt8 = ${value.encryption_compact}
  public static let encryptionRobust: UInt8 = ${value.encryption_robust}
  public static let compactSyntheticIvBytes: Int = ${value.compact_synthetic_iv_bytes}
  public static let robustNonceBytes: Int = ${value.robust_nonce_bytes}
  public static let robustTagBytes: Int = ${value.robust_tag_bytes}
  public static let dataProtectionKeyBytes: Int = ${value.data_protection_key_bytes}
  public static let itemIdRootContext: String = ${swift_string_literal(value.item_id_root_context)}
  public static let aadDomain: String = ${swift_string_literal(value.aad_domain)}
  public static let valueRootContext: String = ${swift_string_literal(value.value_root_context)}
  public static let compactMacContext: String = ${swift_string_literal(value.compact_mac_context)}
  public static let compactEncryptionContext: String = ${swift_string_literal(value.compact_encryption_context)}
  public static let robustContext: String = ${swift_string_literal(value.robust_context)}
}

/// Native ABI connection-state identifiers shared by every language adapter.
public enum Smithy_Connection_State: UInt32, Equatable, Sendable {
${connection_states}
}

/// C-compatible namespace descriptor returned by the native ABI decoder.
internal struct Smithy_Native_Namespace_Descriptor {
${swift_namespace_descriptor_fields}
}

/// Native ABI identifiers shared by every language adapter.
public enum Smithy_Native_Contract: Sendable {
  public static let abiVersion: UInt32 = ${ffi.abi_version}
${swift_native_constants}
  public static let namespaceDescriptorSizeBytes: Int = ${descriptor_layout.size_bytes}
${swift_descriptor_offsets}
}
`
}
