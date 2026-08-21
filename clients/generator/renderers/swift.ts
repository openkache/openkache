/** Swift API and operation renderers. */

import type { Api_Type } from "../../operation_models"
import type { Client_Contract, Ffi_Entry } from "../../client_contract"
import { pascal_case, swift_property_name, typescript_name } from "../../generator_names"
import { encode_vu128 } from "../../generator_values"
import { operation_field_count } from "../../operation_plans"
import type { Operation_Result_Kind } from "../../compatibility_result_projections"
import { is_packed_f64_type } from "../rendering"
import {
  field_sequence_framing,
  has_application_value_codec,
  has_wire_codec,
  managed_operation_entries,
  managed_operation_label,
  operation_composite_field_codec,
  operation_composite_fields,
  operation_composite_value_count,
  operation_convenience_fields,
  operation_empty_result_constant,
  operation_field_name,
  operation_fields,
  operation_is_global_empty,
  operation_is_global_field_sequence,
  operation_is_global_opaque,
  operation_item_fields,
  operation_opaque_field_name,
  operation_policy_fields,
  operation_request_is_opaque,
  operation_request_value_count,
  operation_request_value_name,
  operation_result_constant,
  operation_uses_compact_item_request,
  operation_uses_compact_namespace_request,
  operation_uses_compact_request_route,
  operation_uses_field_sequence_helpers,
  operation_uses_item_id_helpers,
  operation_uses_optional_value_layout,
  optional_value_framing,
  render_application_value_codec,
  render_composite_field_decode,
  render_composite_output,
  render_expression_generic_invocation,
  render_field_sequence_request_payload,
  render_field_sequence_response_decode,
  render_operation_result,
  render_opaque_request_expression,
  render_swift_container_helpers,
  render_swift_field_sequence_helpers,
  type Managed_Api_Operation,
} from "../managed"

function swift_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "Data"
      break
    case "boolean":
      rendered = "Bool"
      break
    case "double":
      rendered = "Double"
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
    case "list":
      rendered = is_packed_f64_type(type)
        ? "[Double]"
        : `[${swift_api_type(type.member ?? { kind: "string" }, true)}]`
      break
    case "map":
      rendered = `[${swift_api_type(type.key ?? { kind: "string" }, true)}: ${
        swift_api_type(type.value ?? { kind: "blob" }, true)
      }]`
      break
    case "structure":
      if (type.name === undefined) throw new Error("structure API type has no name")
      rendered = `Smithy_${typescript_name(type.name)}`
      break
    case "string":
      rendered = "String"
      break
    case "union":
      rendered = "Data"
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
    ["keySpec", ffi.key_specs],
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

function render_swift_operation_method(
  contract: Client_Contract,
  operation: Managed_Api_Operation,
): string {
  const method_name = swift_property_name(operation.name)
  const operation_constant = `UInt32(Smithy_Opcode.${method_name}.rawValue)`
  const input = `Smithy_${typescript_name(operation.input)}`
  const output = `Smithy_${typescript_name(operation.output)}`
  const operation_label = managed_operation_label(operation)
  const result_constant = (kind: Operation_Result_Kind): string =>
    operation_result_constant(operation, kind, "swift")
  const empty_result_constant = operation_empty_result_constant(operation, "swift")
  const {
    input_condition,
    input_create_if_missing,
    input_eviction_mode,
    input_expected_revision,
    input_expiration_mode,
    input_name,
    input_namespace_id,
    input_policy,
    input_ttl_milliseconds,
    input_value,
    output_created,
    output_deleted,
    output_descriptor,
    output_json,
    output_outcome,
    output_value,
  } = operation_convenience_fields(operation, "swift")
  const input_item_ids = operation_item_fields(operation).map(
    (member) => operation_field_name(member, "swift"),
  )
  const input_item_id_expression = input_item_ids.length === 1
    ? `input.${input_item_ids[0]!}`
    : `smithyConcatItemIDs([${input_item_ids.map((name) => `input.${name}`).join(", ")}])`
  const {
    policy_default_eviction,
    policy_default_expiration,
    policy_default_ttl_milliseconds,
    policy_eviction_override,
    policy_expiration_override,
  } = operation_policy_fields(contract, operation, "swift")
  const application_value_codecs = operation.plan.application_value_codecs
  const input_request_value = operation_request_value_name(operation, "swift")
    ?? "Data()"
  return render_operation_result(operation, "Swift", {
    raw_payload: () => {
      const output_payload = operation_opaque_field_name(operation, "output", "swift")
      const invocation = render_expression_generic_invocation(
        "swift",
        operation,
        operation_constant,
        `"${operation_label}"`,
      ) ??
        `smithyInvoke(${operation_constant})`
      return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await ${invocation}
    guard result.kind == ${result_constant("ok")} else {
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
    return ${output}(${output_payload}: result.payload)
  }`
    },
    opaque: () => {
        const input_payload = operation_request_is_opaque(operation)
          ? operation_opaque_field_name(operation, "input", "swift")
          : undefined
        const output_payload = operation_opaque_field_name(operation, "output", "swift")
        const codec = render_application_value_codec(
          "swift",
          application_value_codecs!,
          input_payload === undefined ? "Data()" : `input.${input_payload}`,
          "result.payload",
          `"${operation_label}"`,
        )
        const decoded_payload = codec.decode
        const invocation = render_expression_generic_invocation(
          "swift",
          operation,
          operation_constant,
          `"${operation_label}"`,
        ) ??
          (operation_is_global_empty(operation)
            ? `smithyInvoke(
      ${operation_constant},
      value: Data()
    )`
            : `smithyInvokeScoped(
      ${operation_constant},
      namespaceID: input.${input_namespace_id},
      itemID: ${input_item_id_expression},
      value: ${input_request_value}
    )`)
        return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await ${invocation}
    guard result.kind == ${result_constant("value")} else {
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
    let value = ${decoded_payload}
    return ${output}(${output_payload}: value)
  }`
    },
    field_sequence: () => {
        const output_decoded_values = operation_composite_fields(operation)
          .map((field, index) =>
            render_composite_field_decode(
              "swift",
              operation_composite_field_codec(operation, field),
              `values[${index}]`,
              `"${operation_label}"`,
              field.required,
              field.type,
            ),
          )
        const output_expression = render_composite_output(
          operation,
          "swift",
          output_decoded_values,
        )
        const response_values = operation.plan.contract.response_framing === "field_sequence"
          ? render_field_sequence_response_decode(
            "swift",
            operation,
            "result.payload",
            `"${operation_label}"`,
          )
          : `try smithyDecodeOptionalValues(result.payload, valueCount: ${operation_composite_value_count(operation)}, operation: "${operation_label}")`
        const invocation = render_expression_generic_invocation(
          "swift",
          operation,
          operation_constant,
          `"${operation_label}"`,
        ) ??
          (operation_is_global_empty(operation)
            ? `smithyInvoke(
      ${operation_constant},
      value: Data()
    )`
            : `smithyInvokeScoped(
      ${operation_constant},
      namespaceID: input.${input_namespace_id},
      itemID: ${input_item_id_expression},
      value: ${input_request_value === "Data()" ? "Data()" : `input.${input_request_value}`}
    )`)
        return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await ${invocation}
    guard result.kind == ${result_constant("value")} else {
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
    let values = ${response_values}
    return ${output_expression}
  }`
    },
    optional_payload: () => {
      if (operation_field_count(operation.plan.operation, "output", "value") > 1) {
        const output_values = operation_fields(operation, "output", "value")
          .map((member, index) =>
            `${operation_field_name(member, "swift")}: values[${index}]`,
          )
          .join(",\n      ")
        return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await smithyInvokeScoped(
      ${operation_constant},
      namespaceID: input.${input_namespace_id},
      itemID: ${input_item_id_expression},
      value: ${input_request_value === "Data()" ? "Data()" : `input.${input_request_value}`}
    )
    guard result.kind == ${result_constant("value")} else {
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
    let values = try smithyDecodeOptionalValues(
      result.payload,
      valueCount: ${operation_field_count(operation.plan.operation, "output", "value")},
      operation: "${operation_label}"
    )
    return ${output}(
      ${output_values}
    )
  }`
      }
      return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await smithyInvokeScoped(
      ${operation_constant},
      namespaceID: input.${input_namespace_id},
      itemID: ${input_item_id_expression},
      value: ${input_request_value === "Data()" ? "Data()" : `input.${input_request_value}`}
    )
    switch result.kind {
    case ${result_constant("value")}:
      return ${output}(${output_value}: result.payload)
    case ${result_constant("not_found")}:
      return ${output}(${output_value}: nil)
    default:
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
  }`
    },
    status_outcome: () => {
      return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await smithyInvokeScoped(
      ${operation_constant},
      namespaceID: input.${input_namespace_id},
      itemID: ${input_item_id_expression},
      value: input.${input_value},
      condition: input.${input_condition},
      expirationMode: input.${input_expiration_mode},
      evictionMode: input.${input_eviction_mode},
      ttlMilliseconds: input.${input_ttl_milliseconds}
    )
    let outcome: Smithy_Set_Outcome
    switch result.kind {
    case ${result_constant("created")}:
      outcome = .created
    case ${result_constant("replaced")}:
      outcome = .replaced
    case ${result_constant("not_stored")}:
      outcome = .notStored
    default:
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
    return ${output}(${output_outcome}: outcome)
  }`
    },
    boolean_outcome: () => {
      return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await smithyInvokeScoped(
      ${operation_constant},
      namespaceID: input.${input_namespace_id},
      itemID: ${input_item_id_expression}
    )
    switch result.kind {
    case ${result_constant("deleted")}:
      return ${output}(${output_deleted}: true)
    case ${result_constant("not_deleted")}:
      return ${output}(${output_deleted}: false)
    default:
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
  }`
    },
    text_payload: () => {
      return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await smithyInvokeScoped(
      ${operation_constant},
      namespaceID: input.${input_namespace_id}
    )
    guard result.kind == ${result_constant("value")} else {
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
    guard let json = String(data: result.payload, encoding: .utf8) else {
      throw OpenKacheError("${operation_label} response is not valid UTF-8")
    }
    return ${output}(${output_json}: json)
  }`
    },
    empty: () => {
      if (operation_is_global_empty(operation)) {
        return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await smithyInvoke(
      ${operation_constant},
      value: Data()
    )
    guard result.kind == ${empty_result_constant} else {
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
    return ${output}()
  }`
      }
      if (
        operation_is_global_opaque(operation) ||
        operation_is_global_field_sequence(operation)
      ) {
        const request_payload = operation_is_global_opaque(operation)
          ? render_opaque_request_expression("swift", operation, `"${operation_label}"`)
          : render_field_sequence_request_payload(
            "swift",
            operation,
            `"${operation_label}"`,
          )
        return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await smithyInvoke(
      ${operation_constant},
      value: ${request_payload}
    )
    guard result.kind == ${empty_result_constant} else {
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
    return ${output}()
  }`
      }
      if (operation_uses_compact_item_request(operation)) {
        const has_request_value = operation_request_value_count(operation) > 0
        if (has_request_value) {
          return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await smithyInvokeScoped(
      ${operation_constant},
      namespaceID: input.${input_namespace_id},
      itemID: ${input_item_id_expression},
      value: input.${input_value},
      condition: input.${input_condition},
      expirationMode: input.${input_expiration_mode},
      evictionMode: input.${input_eviction_mode},
      ttlMilliseconds: input.${input_ttl_milliseconds}
    )
    guard result.kind == ${empty_result_constant} else {
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
    return ${output}()
  }`
        }
        return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await smithyInvokeScoped(
      ${operation_constant},
      namespaceID: input.${input_namespace_id},
      itemID: ${input_item_id_expression}
    )
    guard result.kind == ${empty_result_constant} else {
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
    return ${output}()
  }`
      }
      if (operation_uses_compact_namespace_request(operation)) {
        return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let result = try await smithyInvokeScoped(
      ${operation_constant},
      namespaceID: input.${input_namespace_id}
    )
    guard result.kind == ${empty_result_constant} else {
      throw OpenKacheError("${operation_label} returned unexpected native result \\(result.kind)")
    }
    return ${output}()
  }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_delete")) {
        return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    try await smithyNamespaceDelete(
      input.${input_namespace_id},
      expectedRevision: input.${input_expected_revision}
    )
    return ${output}()
  }`
      }
      throw new Error(`unsupported generated Swift empty operation ${operation.name}`)
    },
    descriptor: () => {
      if (operation_uses_compact_request_route(operation, "namespace_open")) {
        return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let policy = try input.${input_policy}.map { policy in
      try smithyPolicyFlags(
        defaultExpiration: policy.${policy_default_expiration},
        defaultTtlMilliseconds: policy.${policy_default_ttl_milliseconds},
        expirationOverride: policy.${policy_expiration_override},
        defaultEviction: policy.${policy_default_eviction},
        evictionOverride: policy.${policy_eviction_override}
      )
    } ?? (flags: UInt8(0), ttl: UInt64(0))
    let result = try await smithyNamespaceOpen(
      name: input.${input_name},
      createIfMissing: input.${input_create_if_missing},
      policyFlags: policy.flags,
      ttl: policy.ttl
    )
    return ${output}(
      ${output_descriptor}: result.${output_descriptor},
      ${output_created}: result.${output_created}
    )
  }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_update_policy")) {
        return `  public func ${method_name}(
    _ input: ${input}
  ) async throws -> ${output} {
    let policy = try smithyPolicyFlags(
      defaultExpiration: input.${input_policy}.${policy_default_expiration},
      defaultTtlMilliseconds: input.${input_policy}.${policy_default_ttl_milliseconds},
      expirationOverride: input.${input_policy}.${policy_expiration_override},
      defaultEviction: input.${input_policy}.${policy_default_eviction},
      evictionOverride: input.${input_policy}.${policy_eviction_override}
    )
    return ${output}(${output_descriptor}: try await smithyNamespaceUpdatePolicy(
      namespaceID: input.${input_namespace_id},
      expectedRevision: input.${input_expected_revision},
      policyFlags: policy.flags,
      ttl: policy.ttl
    ))
  }`
      }
      throw new Error(`unsupported generated Swift namespace operation ${operation.name}`)
    },
  })
}

/** Renders generated Swift Smithy operation implementations. */
export function render_swift_operations(contract: Client_Contract): string {
  const managed_operations = managed_operation_entries(contract)
  const framing = optional_value_framing(contract)
  const field_framing = field_sequence_framing(contract)
  const container_helpers = has_wire_codec(
    managed_operations,
    ["list", "map", "union"],
  )
    ? render_swift_container_helpers(contract.max_value_bytes)
    : ""
  const field_sequence_helpers = managed_operations.some(
    operation_uses_field_sequence_helpers,
  )
    ? render_swift_field_sequence_helpers(field_framing)
    : ""
  const methods = managed_operations
    .map((operation) => render_swift_operation_method(contract, operation))
    .join("\n\n")
  const f64_array_helpers = has_application_value_codec(
    managed_operations,
    "packed_f64_be",
  )
    ? `private func smithyEncodeF64Array(_ values: [Double]) throws -> Data {
  var payload = Data(capacity: values.count * 8)
  for value in values {
    guard value.isFinite else {
      throw OpenKacheError("binary64 array input must contain finite values")
    }
    let bits = value.bitPattern
    for shift in stride(from: 56, through: 0, by: -8) {
      payload.append(UInt8((bits >> UInt64(shift)) & 0xff))
    }
  }
  return payload
}

private func smithyDecodeF64Array(
  _ payload: Data,
  operation: String
) throws -> [Double] {
  guard payload.count % 8 == 0 else {
    throw OpenKacheError("\(operation) response has a malformed binary64 array length")
  }
  var values: [Double] = []
  values.reserveCapacity(payload.count / 8)
  for offset in stride(from: 0, to: payload.count, by: 8) {
    var bits: UInt64 = 0
    for index in 0..<8 {
      bits = (bits << 8) | UInt64(payload[offset + index])
    }
    let value = Double(bitPattern: bits)
    guard value.isFinite else {
      throw OpenKacheError("\(operation) response contains a non-finite binary64 value")
    }
    values.append(value)
  }
  return values
}
`
    : ""
  const item_id_helpers = managed_operations.some(
    operation_uses_item_id_helpers,
  )
    ? `private func smithyConcatItemIDs(_ itemIDs: [Data]) -> Data {
  var total = 0
  for itemID in itemIDs {
    precondition(
      itemID.count <= Smithy_Value_Format.itemIdBytes,
      "item IDs must contain at most \(Smithy_Value_Format.itemIdBytes) bytes"
    )
    precondition(
      total <= Smithy_Value_Format.itemIdBytes - itemID.count,
      "combined item IDs must contain at most \(Smithy_Value_Format.itemIdBytes) bytes"
    )
    total += itemID.count
  }
  var combined = Data(capacity: total)
  for itemID in itemIDs {
    combined.append(itemID)
  }
  return combined
}
`
    : ""
  const optional_values_helpers = managed_operations.some(
    operation_uses_optional_value_layout,
  )
    ? `private func smithyDecodeOptionalValues(
  _ payload: Data,
  valueCount: Int,
  operation: String
) throws -> [Data?] {
  var offset = 0
  var values: [Data?] = []
  values.reserveCapacity(valueCount)
  for _ in 0..<valueCount {
    guard payload.count - offset >= ${framing.length_bytes} else {
      throw OpenKacheError("\(operation) response is missing an optional-value length")
    }
    let length = payload[offset..<(offset + ${framing.length_bytes})].reduce(UInt32(0)) {
      ($0 << 8) | UInt32($1)
    }
    offset += ${framing.length_bytes}
    if length == UInt32(${framing.missing_sentinel}) {
      values.append(nil)
      continue
    }
    guard length <= UInt32(${framing.max_value_bytes}) else {
      throw OpenKacheError(
        "\(operation) response optional-value entry exceeds the maximum value size"
      )
    }
    let count = Int(length)
    guard count <= payload.count - offset else {
      throw OpenKacheError("\(operation) response contains a truncated optional-value entry")
    }
    values.append(Data(payload[offset..<(offset + count)]))
    offset += count
  }
  guard offset == payload.count else {
    throw OpenKacheError("\(operation) response contains trailing optional-value bytes")
  }
  return values
}
`
    : ""
  const compatibility_helpers = `${item_id_helpers}${optional_values_helpers}`
  return `// Generated from the OpenKache Smithy contract. Do not edit.

import Foundation

${f64_array_helpers}
${container_helpers}
${field_sequence_helpers}
${compatibility_helpers}
extension OpenKacheRawClient: Smithy_OpenKache_Api {
${methods}
}
`
}

/** Renders the cross-language value-format wire and cryptographic contract for TypeScript.
 *
 * @param contract - Validated language-neutral wire and value-format contract.
 * @returns Deterministic TypeScript source with a trailing newline.
 */
