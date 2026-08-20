/** Shared operation plans, codecs, and framing used by language renderers. */

import type { Wire_Entry } from "../../../protocol/wire"
import type {
  Api_Member,
  Api_Type,
  Operation_Field_Role,
} from "../../operation_models"
import type { Client_Contract, Ffi_Input_Kind } from "../../client_contract"
import { compatibility_ffi_operation_contract } from "../../compatibility_ffi_adapters"
import {
  go_exported_name,
  pascal_case,
  snake_case,
  swift_property_name,
} from "../../generator_names"

interface Adapter_Contract_Values {
  readonly abi_version: number
  readonly operation_contracts: readonly {
    readonly accepts_value: boolean
    readonly accepts_set_options: boolean
    readonly input_kind: Ffi_Input_Kind
    readonly name: string
    readonly request_item_count: number
    readonly supports_protected: boolean
    readonly supports_raw: boolean
    readonly supports_scoped: boolean
  }[]
  readonly operations: readonly Wire_Entry[]
  readonly result_error: number
  readonly result_ok: number
  readonly result_value: number
  readonly result_not_found: number
  readonly result_created: number
  readonly result_replaced: number
  readonly result_deleted: number
  readonly result_not_deleted: number
  readonly result_connected: number
  readonly result_not_stored: number
  readonly result_raw: number
  readonly descriptor_decode_ok: number
  readonly default_expiration_no_expiry: number
  readonly default_expiration_fixed_ttl: number
  readonly default_eviction_evictable: number
  readonly default_eviction_protected: number
  readonly override_disallowed: number
  readonly override_allowed: number
  readonly set_condition_any: number
  readonly set_condition_if_absent: number
  readonly set_condition_if_present: number
  readonly key_spec_text: number
  readonly key_spec_bytes: number
  readonly key_spec_integer: number
  readonly set_inherit_expiration: number
  readonly set_no_expiry: number
  readonly set_explicit_ttl: number
  readonly set_inherit_eviction: number
  readonly set_evictable: number
  readonly set_eviction_protected: number
  readonly policy_no_expiry: number
  readonly policy_fixed_ttl: number
  readonly policy_expiration_override: number
  readonly policy_eviction_protected: number
  readonly policy_eviction_override: number
  readonly item_id_bytes: number
  readonly namespace_name_max_bytes: number
  readonly max_value_bytes: number
  readonly default_zstandard_level: number
  readonly default_zstandard_minimum_input_bytes: number
  readonly default_zstandard_minimum_savings_bytes: number
  readonly default_connect_timeout_milliseconds: number
  readonly default_request_timeout_milliseconds: number
}

function adapter_operation_contracts(
  contract: Client_Contract,
): Adapter_Contract_Values["operation_contracts"] {
  const contracts = contract.opcodes.map((opcode) => {
    const operation = contract.api.operations.find(
      (candidate) => candidate.name === opcode.name,
    )
    const operation_contract =
      operation === undefined
        ? undefined
        : compatibility_ffi_operation_contract(operation)
    if (operation_contract === undefined) return undefined
    return {
      accepts_value: operation_contract.accepts_value,
      accepts_set_options: operation_contract.accepts_set_options,
      input_kind: operation_contract.input_kind,
      name: opcode.name,
      request_item_count: operation_contract.request_item_count,
      supports_protected: operation_contract.supports_protected,
      supports_raw: operation_contract.supports_raw,
      supports_scoped: operation_contract.supports_scoped,
    }
  })
  return contracts.some((entry) => entry === undefined)
    ? []
    : contracts as Adapter_Contract_Values["operation_contracts"]
}

function required_contract_entry(
  entries: readonly Wire_Entry[],
  name: string,
  location: string,
): Wire_Entry {
  const entry = entries.find((candidate) => candidate.name === name)
  if (entry === undefined) {
    throw new Error(`${location} is missing required ${name} entry`)
  }
  return entry
}

export function adapter_contract_values(contract: Client_Contract): Adapter_Contract_Values {
  const result = (name: string): number =>
    required_contract_entry(
      contract.ffi.result_kinds,
      name,
      "FFI result-kind contract",
    ).value
  const set_condition = (name: string): number =>
    required_contract_entry(
      contract.ffi.set_conditions,
      name,
      "FFI SET-condition contract",
    ).value
  const key_spec = (name: string): number =>
    required_contract_entry(
      contract.ffi.key_specs,
      name,
      "FFI key-spec contract",
    ).value
  const descriptor_decode = (name: string): number =>
    required_contract_entry(
      contract.ffi.namespace_descriptor_decode_statuses,
      name,
      "FFI namespace-descriptor decode-status contract",
    ).value
  const default_expiration = (name: string): number =>
    required_contract_entry(
      contract.ffi.namespace_default_expirations,
      name,
      "FFI namespace default-expiration contract",
    ).value
  const default_eviction = (name: string): number =>
    required_contract_entry(
      contract.ffi.namespace_default_evictions,
      name,
      "FFI namespace default-eviction contract",
    ).value
  const override_policy = (name: string): number =>
    required_contract_entry(
      contract.ffi.namespace_override_policies,
      name,
      "FFI namespace override-policy contract",
    ).value
  return {
    abi_version: contract.ffi.abi_version,
    operation_contracts: adapter_operation_contracts(contract),
    operations: contract.opcodes,
    result_error: result("Error"),
    result_ok: result("Ok"),
    result_value: result("Value"),
    result_not_found: result("NotFound"),
    result_created: result("Created"),
    result_replaced: result("Replaced"),
    result_deleted: result("Deleted"),
    result_not_deleted: result("NotDeleted"),
    result_connected: result("Connected"),
    result_not_stored: result("NotStored"),
    result_raw: result("Raw"),
    descriptor_decode_ok: descriptor_decode("Ok"),
    default_expiration_no_expiry: default_expiration("NoExpiry"),
    default_expiration_fixed_ttl: default_expiration("FixedTtl"),
    default_eviction_evictable: default_eviction("Evictable"),
    default_eviction_protected: default_eviction("Protected"),
    override_disallowed: override_policy("Disallowed"),
    override_allowed: override_policy("Allowed"),
    set_condition_any: set_condition("Any"),
    set_condition_if_absent: set_condition("IfAbsent"),
    set_condition_if_present: set_condition("IfPresent"),
    key_spec_text: key_spec("Text"),
    key_spec_bytes: key_spec("Bytes"),
    key_spec_integer: key_spec("Integer"),
    set_inherit_expiration: contract.v1.set_inherit_expiration_bits,
    set_no_expiry: contract.v1.set_no_expiry_bits,
    set_explicit_ttl: contract.v1.set_ttl_flag,
    set_inherit_eviction: contract.v1.set_inherit_eviction_bits,
    set_evictable: contract.v1.set_evictable_bits,
    set_eviction_protected: contract.v1.set_eviction_protected_bits,
    policy_no_expiry: contract.v1.policy_no_expiry_bits,
    policy_fixed_ttl: contract.v1.policy_fixed_ttl_bits,
    policy_expiration_override: contract.v1.policy_expiration_override_flag,
    policy_eviction_protected: contract.v1.policy_eviction_protected_flag,
    policy_eviction_override: contract.v1.policy_eviction_override_flag,
    item_id_bytes: contract.item_id_bytes,
    namespace_name_max_bytes: contract.v1.namespace_name_max_bytes,
    max_value_bytes: contract.max_value_bytes,
    default_zstandard_level: contract.client_defaults.zstandard_level,
    default_zstandard_minimum_input_bytes:
      contract.client_defaults.zstandard_minimum_input_bytes,
    default_zstandard_minimum_savings_bytes:
      contract.client_defaults.zstandard_minimum_savings_bytes,
    default_connect_timeout_milliseconds:
      contract.client_defaults.connect_timeout_milliseconds,
    default_request_timeout_milliseconds:
      contract.client_defaults.request_timeout_milliseconds,
  }
}

function operation_cases(
  operations: Adapter_Contract_Values["operation_contracts"],
  predicate: (
    operation: Adapter_Contract_Values["operation_contracts"][number],
  ) => boolean,
  render: (
    operation: Adapter_Contract_Values["operation_contracts"][number],
  ) => string,
): string {
  return operations.filter(predicate).map(render).join(", ")
}

export function render_java_operation_metadata(
  values: Adapter_Contract_Values,
): string {
  if (values.operation_contracts.length === 0) return ""
  const scoped = operation_cases(
    values.operation_contracts,
    (operation) => operation.supports_scoped,
    (operation) => `OPERATION_${snake_case(operation.name).toUpperCase()}`,
  )
  const item = operation_cases(
    values.operation_contracts,
    (operation) => operation.input_kind === "item_id",
    (operation) => `OPERATION_${snake_case(operation.name).toUpperCase()}`,
  )
  const scoped_clause = scoped.length > 0
    ? `case ${scoped} -> true;\n            default -> false;`
    : "default -> false;"
  const item_clause = item.length > 0
    ? `case ${item} -> true;\n            default -> false;`
    : "default -> false;"
  const item_bytes_clause = [
    ...values.operation_contracts
      .filter((operation) => operation.input_kind === "item_id")
      .map(
        (operation) =>
          `case OPERATION_${snake_case(operation.name).toUpperCase()} -> ${values.item_id_bytes * operation.request_item_count};`,
      ),
    "default -> 0;",
  ].join("\n            ")
  return `    /** Returns whether the Smithy operation is valid on the scoped exact-ID ABI. */
    public static boolean operationSupportsScoped(int operation) {
        return switch (operation) {
            ${scoped_clause}
        };
    }

    /** Returns whether the scoped ABI requires one exact item ID. */
    public static boolean operationRequiresItemId(int operation) {
        return switch (operation) {
            ${item_clause}
        };
    }

    /** Returns the exact item-ID byte span required by the scoped ABI. */
    public static int operationItemIdBytes(int operation) {
        return switch (operation) {
            ${item_bytes_clause}
        };
    }
`
}

export function render_kotlin_operation_metadata(
  values: Adapter_Contract_Values,
): string {
  if (values.operation_contracts.length === 0) return ""
  const scoped = operation_cases(
    values.operation_contracts,
    (operation) => operation.supports_scoped,
    (operation) => `OPERATION_${snake_case(operation.name).toUpperCase()}`,
  )
  const item = operation_cases(
    values.operation_contracts,
    (operation) => operation.input_kind === "item_id",
    (operation) => `OPERATION_${snake_case(operation.name).toUpperCase()}`,
  )
  const scoped_clause = scoped.length > 0
    ? `${scoped} -> true\n        else -> false`
    : "else -> false"
  const item_clause = item.length > 0
    ? `${item} -> true\n        else -> false`
    : "else -> false"
  const item_bytes_clause = [
    ...values.operation_contracts
      .filter((operation) => operation.input_kind === "item_id")
      .map(
        (operation) =>
          `OPERATION_${snake_case(operation.name).toUpperCase()} -> ITEM_ID_BYTES * ${operation.request_item_count}`,
      ),
    "else -> 0",
  ].join("\n        ")
  return `    /** Returns whether the Smithy operation is valid on the scoped exact-ID ABI. */
    public fun operationSupportsScoped(operation: Int): Boolean = when (operation) {
        ${scoped_clause}
    }

    /** Returns whether the scoped ABI requires one exact item ID. */
    public fun operationRequiresItemId(operation: Int): Boolean = when (operation) {
        ${item_clause}
    }

    /** Returns the exact item-ID byte span required by the scoped ABI. */
    public fun operationItemIdBytes(operation: Int): Int = when (operation) {
        ${item_bytes_clause}
    }
`
}

export function render_dart_operation_metadata(
  values: Adapter_Contract_Values,
): string {
  if (values.operation_contracts.length === 0) return ""
  const scoped = operation_cases(
    values.operation_contracts,
    (operation) => operation.supports_scoped,
    (operation) => `smithyOperation${pascal_case(snake_case(operation.name))}`,
  )
  const item = operation_cases(
    values.operation_contracts,
    (operation) => operation.input_kind === "item_id",
    (operation) => `smithyOperation${pascal_case(snake_case(operation.name))}`,
  )
  const scoped_clause = scoped.length > 0
    ? `${scoped.replaceAll(", ", " || ")} => true,\n  _ => false,`
    : "_ => false,"
  const item_clause = item.length > 0
    ? `${item.replaceAll(", ", " || ")} => true,\n  _ => false,`
    : "_ => false,"
  const item_bytes_clause = [
    ...values.operation_contracts
      .filter((operation) => operation.input_kind === "item_id")
      .map(
        (operation) =>
          `smithyOperation${pascal_case(snake_case(operation.name))} => smithyItemIdBytes * ${operation.request_item_count},`,
      ),
    "_ => 0,",
  ].join("\n  ")
  return `/// Returns whether the Smithy operation is valid on the scoped exact-ID ABI.
bool smithyOperationSupportsScoped(int operation) => switch (operation) {
  ${scoped_clause}
};

/// Returns whether the scoped ABI requires one exact item ID.
bool smithyOperationRequiresItemId(int operation) => switch (operation) {
  ${item_clause}
};

/// Returns the exact item-ID byte span required by the scoped ABI.
int smithyOperationItemIdBytes(int operation) => switch (operation) {
  ${item_bytes_clause}
};
`
}

export interface Operation_Field_Binding {
  readonly input: Readonly<Partial<Record<Operation_Field_Role, readonly Api_Member[]>>>
  readonly output: Readonly<Partial<Record<Operation_Field_Role, readonly Api_Member[]>>>
}

export function go_api_name(identifier: string): string {
  return `Smithy${pascal_case(snake_case(identifier))}`
}

export function python_api_name(identifier: string): string {
  return `Smithy${pascal_case(snake_case(identifier))}`
}

export function go_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "[]byte"
      break
    case "boolean":
      rendered = "bool"
      break
    case "double":
      rendered = "float64"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = go_api_name(type.name)
      break
    case "integer":
      rendered = "int32"
      break
    case "list":
      rendered =
        type.member?.kind === "double" &&
          (type.wire_codec === undefined || type.wire_codec === "packed_f64_be")
          ? "[]float64"
          : `[]${go_api_type(type.member ?? { kind: "blob" }, true)}`
      break
    case "map":
      rendered = `map[${go_api_type(type.key ?? { kind: "string" }, true)}]${
        go_api_type(type.value ?? { kind: "blob" }, true)
      }`
      break
    case "long":
      rendered = "int64"
      break
    case "structure":
      if (type.name === undefined) throw new Error("structure API type has no name")
      rendered = go_api_name(type.name)
      break
    case "string":
      rendered = "string"
      break
    case "union":
      rendered = "[]byte"
      break
    case "unsigned_long":
      rendered = "uint64"
      break
  }
  return required ? rendered : `*${rendered}`
}

export function operation_field_name(
  member: Pick<Api_Member, "name">,
  language: "csharp" | "dart" | "go" | "java" | "kotlin" | "python" | "rust" | "swift" | "typescript",
): string {
  switch (language) {
    case "csharp":
      return pascal_case(snake_case(member.name))
    case "dart":
    case "java":
    case "kotlin":
      return member.name
    case "go":
      return go_exported_name(member.name)
    case "python":
    case "rust":
    case "typescript":
      return snake_case(member.name)
    case "swift":
      return swift_property_name(member.name)
  }
}
