/** Renders language-level Smithy API shapes and operation interfaces. */

import { join } from "node:path"

import type {
  Api_Contract,
  Api_Enum,
  Api_Structure,
  Api_Type,
} from "./operation_models"
import type { Client_Contract } from "./client_contract"
import {
  lower_camel_case,
  snake_case,
  typescript_name,
} from "./generator_names"

function is_packed_f64_type(type: Api_Type): boolean {
  return type.kind === "list" &&
    type.member?.kind === "double" &&
    (type.wire_codec === undefined || type.wire_codec === "packed_f64_be")
}

function java_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "byte[]"
      break
    case "boolean":
      rendered = "boolean"
      break
    case "double":
      rendered = "double"
      break
    case "enum":
    case "structure":
      if (type.name === undefined) throw new Error(`Java API ${type.kind} has no name`)
      rendered = type.name
      break
    case "integer":
      rendered = "int"
      break
    case "list":
      rendered = is_packed_f64_type(type)
        ? "double[]"
        : `java.util.List<${java_api_type(type.member ?? { kind: "blob" }, true)}>`
      break
    case "map":
      rendered = `java.util.Map<${java_api_type(type.key ?? { kind: "string" }, true)}, ${
        java_api_type(type.value ?? { kind: "blob" }, true)
      }>`
      break
    case "union":
      rendered = "byte[]"
      break
    case "long":
    case "unsigned_long":
      rendered = "long"
      break
    case "string":
      rendered = "String"
      break
  }
  if (required) return rendered
  switch (rendered) {
    case "boolean":
      return "Boolean"
    case "int":
      return "Integer"
    case "long":
      return "Long"
    default:
      return rendered
  }
}

function java_api_enum_source(enum_: Api_Enum): string {
  const members = enum_.members
    .map(
      (member) =>
        `    ${snake_case(member.name).toUpperCase()}(${JSON.stringify(member.value)})`,
    )
    .join(",\n")
  return `package io.openkache.client;

/** Generated Smithy ${enum_.name} string enum. */
public enum ${enum_.name} {
${members};

    private final String smithyValue;

    ${enum_.name}(String smithyValue) {
        this.smithyValue = smithyValue;
    }

    public String smithyValue() {
        return smithyValue;
    }

    public static ${enum_.name} fromSmithyValue(String value) {
        for (${enum_.name} member : values()) {
            if (member.smithyValue.equals(value)) return member;
        }
        throw new IllegalArgumentException("unknown ${enum_.name} value: " + value);
    }
}
`
}

function java_api_structure_source(structure: Api_Structure): string {
  if (structure.members.length === 0) {
    return `package io.openkache.client;

/** Generated Smithy ${structure.name} structure. */
public record ${structure.name}() {}
`
  }
  const members = structure.members
    .map(
      (member) =>
        `    ${java_api_type(member.type, member.required)} ${member.name}`,
    )
    .join(",\n")
  const required_references = structure.members
    .filter(
      (member) =>
        member.required &&
        ["blob", "enum", "list", "string", "structure"].includes(member.type.kind),
    )
    .map((member) => `        Objects.requireNonNull(${member.name}, "${member.name}");`)
    .join("\n")
  return `package io.openkache.client;

import java.util.Objects;

/** Generated Smithy ${structure.name} structure. */
public record ${structure.name}(
${members}
) {
    public ${structure.name} {
${required_references}
    }
}
`
}

function java_api_interface_source(contract: Client_Contract): string {
  const interfaces = contract.api.operations
    .map((operation) => `Smithy${operation.name}Api`)
    .join(",\n    ")
  return `package io.openkache.client;

/** Generated Smithy operation surface. */
public interface SmithyOpenKacheApi extends
    ${interfaces} {}
`
}

function java_api_operation_interface_source(operation: Api_Contract["operations"][number]): string {
  return `package io.openkache.client;

/** Generated Smithy ${operation.name} operation surface. */
public interface Smithy${operation.name}Api {
    /** Invokes the Smithy ${operation.name} operation. */
    java.util.concurrent.CompletionStage<${operation.output}> ${lower_camel_case(operation.name)}(${operation.input} input);
}
`
}

/** Renders generated Java Smithy shapes and the complete operation surface. */
export function render_java_api(
  contract: Client_Contract,
  generated_output_root = ".",
): Readonly<Record<string, string>> {
  const outputs: Record<string, string> = {
    [join(generated_output_root, "SmithyOpenKacheApi.java")]:
      java_api_interface_source(contract),
  }
  for (const operation of contract.api.operations) {
    outputs[join(generated_output_root, `Smithy${operation.name}Api.java`)] =
      java_api_operation_interface_source(operation)
  }
  for (const enum_ of contract.api.enums) {
    outputs[join(generated_output_root, `${enum_.name}.java`)] =
      java_api_enum_source(enum_)
  }
  for (const structure of contract.api.structures) {
    outputs[join(generated_output_root, `${structure.name}.java`)] =
      java_api_structure_source(structure)
  }
  return outputs
}

function kotlin_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "ByteArray"
      break
    case "boolean":
      rendered = "Boolean"
      break
    case "double":
      rendered = "Double"
      break
    case "enum":
    case "structure":
      if (type.name === undefined) throw new Error(`Kotlin API ${type.kind} has no name`)
      rendered = type.name
      break
    case "integer":
      rendered = "Int"
      break
    case "list":
      rendered = is_packed_f64_type(type)
        ? "DoubleArray"
        : `List<${kotlin_api_type(type.member ?? { kind: "blob" }, true)}>`
      break
    case "map":
      rendered = `Map<${kotlin_api_type(type.key ?? { kind: "string" }, true)}, ${
        kotlin_api_type(type.value ?? { kind: "blob" }, true)
      }>`
      break
    case "union":
      rendered = "ByteArray"
      break
    case "long":
    case "unsigned_long":
      rendered = "Long"
      break
    case "string":
      rendered = "String"
      break
  }
  return required ? rendered : `${rendered}?`
}

function kotlin_api_enum_source(enum_: Api_Enum): string {
  const members = enum_.members
    .map(
      (member) =>
        `    ${member.name}(${JSON.stringify(member.value)})`,
    )
    .join(",\n")
  return `/** Generated Smithy ${enum_.name} string enum. */
public enum class ${enum_.name}(public val smithyValue: String) {
${members};

    public companion object {
        public fun fromSmithyValue(value: String): ${enum_.name} =
            entries.firstOrNull { it.smithyValue == value }
                ?: error("unknown ${enum_.name} value: \$value")
    }
}
`
}

function kotlin_api_structure_source(structure: Api_Structure): string {
  if (structure.members.length === 0) {
    return `/** Generated Smithy ${structure.name} structure. */
public class ${structure.name}
`
  }
  const members = structure.members
    .map(
      (member) =>
        `    public val ${member.name}: ${kotlin_api_type(member.type, member.required)}`,
    )
    .join(",\n")
  return `/** Generated Smithy ${structure.name} structure. */
public data class ${structure.name}(
${members}
)
`
}

/** Renders generated Kotlin Smithy shapes and the complete operation surface. */
export function render_kotlin_api(contract: Client_Contract): string {
  const enums = contract.api.enums.map(kotlin_api_enum_source)
  const structures = contract.api.structures.map(kotlin_api_structure_source)
  const operation_interfaces = contract.api.operations.map(
    (operation) => `/** Generated Smithy ${operation.name} operation surface. */
public interface Smithy${operation.name}Api {
    /** Invokes the Smithy ${operation.name} operation. */
    public suspend fun ${lower_camel_case(operation.name)}(input: ${operation.input}): ${operation.output}
}`,
  )
  const interfaces = contract.api.operations
    .map((operation) => `Smithy${operation.name}Api`)
    .join(",\n    ")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client

${[...enums, ...structures].join("\n")}

/** Generated Smithy operation surface. */
${operation_interfaces.join("\n\n")}

public interface SmithyOpenKacheApi :
    ${interfaces}
`
}

function dart_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "List<int>"
      break
    case "boolean":
      rendered = "bool"
      break
    case "double":
      rendered = "double"
      break
    case "enum":
    case "structure":
      if (type.name === undefined) throw new Error(`Dart API ${type.kind} has no name`)
      rendered = type.name
      break
    case "integer":
    case "long":
    case "unsigned_long":
      rendered = "int"
      break
    case "list":
      rendered = is_packed_f64_type(type)
        ? "List<double>"
        : `List<${dart_api_type(type.member ?? { kind: "blob" }, true)}>`
      break
    case "map":
      rendered = `Map<${dart_api_type(type.key ?? { kind: "string" }, true)}, ${
        dart_api_type(type.value ?? { kind: "blob" }, true)
      }>`
      break
    case "union":
      rendered = "List<int>"
      break
    case "string":
      rendered = "String"
      break
  }
  return required ? rendered : `${rendered}?`
}

function dart_api_enum_source(enum_: Api_Enum): string {
  const members = enum_.members
    .map(
      (member) =>
        `  ${lower_camel_case(member.name)}('${member.value.replaceAll("'", "\\'")}')`,
    )
    .join(",\n")
  return `/// Generated Smithy ${enum_.name} string enum.
enum ${enum_.name} {
${members}
;

  const ${enum_.name}(this.smithyValue);

  final String smithyValue;

  static ${enum_.name} fromSmithyValue(String value) {
    return values.firstWhere(
      (member) => member.smithyValue == value,
      orElse: () => throw ArgumentError('unknown ${enum_.name} value: \$value'),
    );
  }
}
`
}

function dart_api_structure_source(structure: Api_Structure): string {
  const parameters = structure.members
    .map(
      (member) =>
        `    ${member.required ? "required " : ""}this.${member.name},`,
    )
    .join("\n")
  const fields = structure.members
    .map(
      (member) =>
        `  final ${dart_api_type(member.type, member.required)} ${member.name};`,
    )
    .join("\n")
  return structure.members.length === 0
    ? `/// Generated Smithy ${structure.name} structure.
final class ${structure.name} {
  const ${structure.name}();
}
`
    : `/// Generated Smithy ${structure.name} structure.
final class ${structure.name} {
  const ${structure.name}({
${parameters}
  });

${fields}
}
`
}

/** Renders generated Dart Smithy shapes and the complete operation surface. */
export function render_dart_api(contract: Client_Contract): string {
  const enums = contract.api.enums.map(dart_api_enum_source)
  const structures = contract.api.structures.map(dart_api_structure_source)
  const operation_interfaces = contract.api.operations.map(
    (operation) => `/// Generated Smithy ${operation.name} operation surface.
abstract interface class Smithy${operation.name}Api {
  /// Invokes the Smithy ${operation.name} operation.
  Future<${operation.output}> ${lower_camel_case(operation.name)}(${operation.input} input);
}`,
  )
  const interfaces = contract.api.operations
    .map((operation) => `Smithy${operation.name}Api`)
    .join(", ")
  return `// Generated from the OpenKache Smithy contract. Do not edit.

${[...enums, ...structures].join("\n")}

/// Generated Smithy operation surface.
${operation_interfaces.join("\n\n")}

abstract interface class SmithyOpenKacheApi implements ${interfaces} {}
`
}

export function typescript_api_name(identifier: string): string {
  return `Smithy_${typescript_name(identifier)}`
}

function typescript_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "Uint8Array"
      break
    case "boolean":
      rendered = "boolean"
      break
    case "double":
      rendered = "number"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = typescript_api_name(type.name)
      break
    case "integer":
      rendered = "number"
      break
    case "list":
      rendered = is_packed_f64_type(type)
        ? "readonly number[]"
        : `readonly ${typescript_api_type(type.member ?? { kind: "blob" }, true)}[]`
      break
    case "map":
      rendered = `ReadonlyMap<${typescript_api_type(type.key ?? { kind: "string" }, true)}, ${
        typescript_api_type(type.value ?? { kind: "blob" }, true)
      }>`
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
    case "union":
      rendered = "Uint8Array"
      break
    case "unsigned_long":
      rendered = "bigint"
      break
  }
  return required ? rendered : `${rendered} | undefined`
}

/** Renders Smithy operation types and an API interface for TypeScript. */
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
${contract.ffi.status_categories
  .map(
    (entry) =>
      `export const SMITHY_FFI_STATUS_CATEGORY_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.error_categories
  .map(
    (entry) =>
      `export const SMITHY_FFI_ERROR_CATEGORY_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
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
${contract.ffi.key_specs
  .map(
    (entry) =>
      `export const SMITHY_FFI_KEY_SPEC_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
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
/** Maximum complete canonical CBOR key item accepted by every SDK. */
export const SMITHY_MAX_CANONICAL_KEY_BYTES = ${contract.client_defaults.max_canonical_key_bytes}
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
/** Gate 0 ALPN protocol version selected by maintained facades. */
export const SMITHY_GATE0_ALPN_VERSION = ${contract.client_defaults.gate0_alpn_version}
/** Gate 0 compression identifier. */
export const SMITHY_GATE0_COMPRESSION = ${contract.client_defaults.gate0_compression}
/** Gate 0 value-protection identifier. */
export const SMITHY_GATE0_ENCRYPTION = ${contract.client_defaults.gate0_encryption}
/** Gate 0 public development Item-ID root key. */
export const SMITHY_GATE0_ITEM_ID_ROOT_KEY_HEX = ${JSON.stringify(contract.client_defaults.gate0_item_id_root_key_hex)}
/** Gate 0 namespace identity. */
export const SMITHY_GATE0_NAMESPACE_ID = ${contract.client_defaults.gate0_namespace_id}
/** Gate 0 value-format selector byte. */
export const SMITHY_GATE0_VALUE_SELECTOR = ${contract.client_defaults.gate0_value_selector}

${[...enums, ...structures].join("\n\n")}

${enum_constants}

/** Operations defined by the OpenKache Smithy service. */
export interface Smithy_OpenKache_Api {
${operations.join("\n")}
}
`
}
