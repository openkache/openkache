#!/usr/bin/env bun
/** Generates client-owned Smithy contracts and their generated language bindings. */

import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { basename, dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import {
  extract_wire_contract as extract_protocol_wire_contract,
  render_rust_wire as render_protocol_rust_wire,
  type Wire_Contract,
  type Wire_Entry,
} from "../protocol/wire"

type Json_Object = Readonly<Record<string, unknown>>

/** Cross-language value-format wire layout, identifiers, and cryptographic metadata. */
export interface Value_Format_Contract {
  readonly aad_domain: string
  readonly compact_encryption_context: string
  readonly compact_mac_context: string
  readonly compact_synthetic_iv_bytes: number
  readonly compression_none: number
  readonly compression_zstandard: number
  readonly data_protection_key_bytes: number
  readonly encryption_compact: number
  readonly encryption_none: number
  readonly encryption_robust: number
  readonly format_byte_bytes: number
  readonly format_compression_mask: number
  readonly format_encryption_shift: number
  readonly item_id_root_context: string
  readonly robust_context: string
  readonly robust_nonce_bytes: number
  readonly robust_tag_bytes: number
  readonly serialization_json: number
  readonly serialization_raw: number
  readonly value_root_context: string
  readonly max_vu128_bytes: number
  readonly version: number
}

/** Legacy metadata envelope retained for the TypeScript adapter migration. */
export interface Value_Envelope_Contract {
  readonly json_encoding: string
  readonly magic_and_version_hex: string
  readonly max_encoding_bytes: number
  readonly max_type_name_bytes: number
}

/** Defaults shared by the Rust client core and its native language adapters. */
export interface Client_Defaults_Contract {
  readonly connect_timeout_milliseconds: number
  readonly max_in_flight: number
  readonly request_timeout_milliseconds: number
  readonly retry_max_attempts: number
  readonly zstandard_level: number
  readonly zstandard_minimum_input_bytes: number
  readonly zstandard_minimum_savings_bytes: number
  readonly zstandard_level_min: number
  readonly zstandard_level_max: number
  readonly server_name: string
  readonly certificate_pem_type: string
  readonly minimum_positive_value: number
}

type Api_Type_Kind =
  | "blob"
  | "boolean"
  | "enum"
  | "integer"
  | "long"
  | "string"
  | "structure"
  | "unsigned_long"

type Api_Operation_Scope =
  | "global"
  | "item"
  | "namespace"
  | "namespace_management"

type Api_Operation_Response_Kind =
  | "empty"
  | "pong"
  | "echo"
  | "value"
  | "set_outcome"
  | "delete_outcome"
  | "stats_json"
  | "namespace_descriptor"

type Api_Operation_Retry_Mode = "always" | "never" | "when_not_creating"

/** Semantic operation metadata consumed by the shared client core. */
export interface Api_Operation_Contract {
  readonly error_statuses: readonly string[]
  readonly response_kind: Api_Operation_Response_Kind
  readonly retry_mode: Api_Operation_Retry_Mode
  readonly scope: Api_Operation_Scope
  readonly success_statuses: readonly string[]
}

/** One resolved Smithy API field type. */
export interface Api_Type {
  readonly kind: Api_Type_Kind
  readonly name?: string
}

/** One field in a Smithy operation input or output structure. */
export interface Api_Member {
  readonly name: string
  readonly required: boolean
  readonly type: Api_Type
}

/** One Smithy operation input or output structure. */
export interface Api_Structure {
  readonly members: readonly Api_Member[]
  readonly name: string
}

/** One string-valued Smithy enum member. */
export interface Api_Enum_Member {
  readonly name: string
  readonly value: string
}

/** One string-valued Smithy API enum. */
export interface Api_Enum {
  readonly members: readonly Api_Enum_Member[]
  readonly name: string
}

/** One operation exposed by the Smithy service. */
export interface Api_Operation {
  readonly contract?: Api_Operation_Contract
  readonly input: string
  readonly name: string
  readonly output: string
}

/** Language-neutral service API extracted from the Smithy model. */
export interface Api_Contract {
  readonly enums: readonly Api_Enum[]
  readonly operations: readonly Api_Operation[]
  readonly structures: readonly Api_Structure[]
}

/** Native binding ABI identifiers shared by language-neutral adapters. */
type Ffi_Input_Kind = "none" | "application_key" | "item_id"

/** Native dispatch and buffer contract for one FFI operation. */
export interface Ffi_Operation_Contract {
  readonly accepts_set_options: boolean
  readonly accepts_value: boolean
  readonly dedicated_abi: boolean
  readonly input_kind: Ffi_Input_Kind
  readonly supports_protected: boolean
  readonly supports_raw: boolean
  readonly supports_scoped: boolean
}

export interface Ffi_Entry extends Wire_Entry {
  /** Stable Smithy enum value exposed by language adapters. */
  readonly text: string
  /** Native dispatch and buffer contract for FFI operation entries. */
  readonly operation_contract?: Ffi_Operation_Contract
}

export interface Ffi_Contract {
  readonly abi_version: number
  readonly connection_states: readonly Ffi_Entry[]
  readonly native_abi_functions: readonly Native_Abi_Function[]
  readonly native_abi_structures: readonly Native_Abi_Structure[]
  readonly namespace_default_evictions: readonly Ffi_Entry[]
  readonly namespace_default_expirations: readonly Ffi_Entry[]
  readonly namespace_descriptor_decode_statuses: readonly Ffi_Entry[]
  readonly namespace_descriptor_fields: readonly Namespace_Descriptor_Field[]
  readonly namespace_descriptor_layout: Namespace_Descriptor_Layout
  readonly namespace_override_policies: readonly Ffi_Entry[]
  readonly operations: readonly Ffi_Entry[]
  readonly result_kinds: readonly Ffi_Entry[]
  readonly set_conditions: readonly Ffi_Entry[]
}

/** C-compatible layout of the namespace descriptor returned by the native ABI. */
export interface Namespace_Descriptor_Layout {
  readonly size_bytes: number
  readonly offsets: Readonly<Record<string, number>>
}

/** One field in the Smithy-defined native namespace descriptor projection. */
export interface Namespace_Descriptor_Field {
  readonly name: string
  readonly csharp_name: string
  readonly go_name: string
  readonly swift_name: string
  readonly rust_type: string
  readonly c_type: string
  readonly csharp_type: string
  readonly go_type: string
  readonly python_type: string
  readonly swift_type: string
  readonly size: number
  readonly alignment: number
  readonly offset: number
}

/** Wire contract combined with the client-owned Smithy model. */
export interface Client_Contract extends Wire_Contract {
  readonly api: Api_Contract
  readonly client_defaults: Client_Defaults_Contract
  readonly ffi: Ffi_Contract
  readonly value_envelope: Value_Envelope_Contract
  readonly value_format: Value_Format_Contract
}

type Native_Abi_Type =
  | "client_pointer"
  | "result_pointer"
  | "u8_pointer"
  | "struct_pointer"
  | "size"
  | "uint8"
  | "int32"
  | "uint32"
  | "uint64"
  | "void"

interface Native_Abi_Parameter {
  readonly name: string
  readonly type: Exclude<Native_Abi_Type, "void">
  readonly mutable: boolean
  readonly structure_name?: string
}

interface Native_Abi_Function {
  readonly name: string
  readonly optional: boolean
  readonly return_type: Native_Abi_Type
  readonly parameters: readonly Native_Abi_Parameter[]
}

interface Native_Abi_Structure {
  readonly name: string
  readonly fields: readonly Native_Abi_Parameter[]
}

const CLIENTS_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const PUBLIC_ROOT = dirname(CLIENTS_DIRECTORY)
const PROTOCOL_DIRECTORY = join(PUBLIC_ROOT, "protocol")
const MODEL_DIRECTORY = "model"
const SMITHY_EXECUTABLE = process.env.OPENKACHE_SMITHY_EXECUTABLE ?? "smithy"
const SMITHY_USE_SHELL = process.env.OPENKACHE_SMITHY_USE_SHELL === "1"
const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
const CLIENT_SERVICE_SHAPE_ID = "openkache.client#OpenKacheClient"
const FFI_CONTRACT_TRAIT_ID = "openkache.client#ffiContract"
const CLIENT_DEFAULTS_TRAIT_ID = "openkache.client#clientDefaults"
const OPERATION_CONTRACT_TRAIT_ID = "openkache.client#operationContract"
const FFI_OPERATION_CONTRACT_TRAIT_ID = "openkache.client#ffiOperationContract"
const VALUE_FORMAT_TRAIT_ID = "openkache.client#valueFormat"
const VALUE_ENVELOPE_TRAIT_ID = "openkache.client#valueEnvelope"
const UNSIGNED_LONG_TRAIT_ID = "openkache.client#unsignedLong"
const FFI_ENUMS = {
  operations: { name: "FfiOperation", kind: "FFI operation" },
  result_kinds: { name: "FfiResultKind", kind: "FFI result" },
  connection_states: { name: "FfiConnectionState", kind: "FFI connection state" },
  set_conditions: { name: "FfiSetCondition", kind: "FFI SET condition" },
  namespace_descriptor_decode_statuses: {
    name: "FfiNamespaceDescriptorDecodeStatus",
    kind: "FFI namespace descriptor decode status",
  },
  namespace_default_expirations: {
    name: "FfiNamespaceDefaultExpiration",
    kind: "FFI namespace default expiration",
  },
  namespace_default_evictions: {
    name: "FfiNamespaceDefaultEviction",
    kind: "FFI namespace default eviction",
  },
  namespace_override_policies: {
    name: "FfiNamespaceOverridePolicy",
    kind: "FFI namespace override policy",
  },
} as const
const GENERATED_OUTPUT_ROOT = resolve(
  process.env.OPENKACHE_GENERATION_OUTPUT_ROOT ?? PUBLIC_ROOT,
)
function generated_path(...segments: string[]): string {
  return join(GENERATED_OUTPUT_ROOT, ...segments)
}

function resolve_smithy_executable(): string {
  if (
    SMITHY_EXECUTABLE.length === 0 ||
    !SMITHY_EXECUTABLE.includes("/") ||
    SMITHY_EXECUTABLE.startsWith("/")
  ) {
    return SMITHY_EXECUTABLE
  }
  let directory = resolve(process.cwd())
  for (;;) {
    if (
      SMITHY_EXECUTABLE.startsWith("external/") &&
      existsSync(resolve(directory, "external"))
    ) {
      return resolve(directory, SMITHY_EXECUTABLE)
    }
    const candidate = resolve(directory, SMITHY_EXECUTABLE)
    if (existsSync(candidate)) return candidate
    const parent = dirname(directory)
    if (parent === directory) return SMITHY_EXECUTABLE
    directory = parent
  }
}
const GENERATED_OUTPUTS = {
  csharp_api: generated_path("clients/dotnet/OpenKache/generated_local/SmithyApi.g.cs"),
  csharp_wire: generated_path("clients/dotnet/OpenKache/generated_local/WireValues.g.cs"),
  csharp_native_abi: generated_path(
    "clients/dotnet/OpenKache/generated_local/SmithyNativeAbi.g.cs",
  ),
  rust_client: process.env.OPENKACHE_RUST_CLIENT_OUTPUT ??
    generated_path("clients/core/generated_local/client_contract.rs"),
  rust_api: process.env.OPENKACHE_RUST_API_OUTPUT ??
    generated_path("clients/rust/generated_local/smithy_api.rs"),
  rust_wire: process.env.OPENKACHE_RUST_WIRE_OUTPUT ??
    generated_path("protocol/generated_local/wire_values.rs"),
  typescript_api: generated_path("clients/typescript/src/generated_local/smithy-api.ts"),
  typescript_value_format: generated_path(
    "clients/typescript/src/generated_local/smithy-value-format.ts",
  ),
  typescript_value_envelope: generated_path(
    "clients/typescript/src/generated_local/smithy-value-envelope.ts",
  ),
  python_api: process.env.OPENKACHE_PYTHON_API_OUTPUT ??
    generated_path("clients/python/src/openkache/_generated/smithy_api.py"),
  python_contract: process.env.OPENKACHE_PYTHON_CONTRACT_OUTPUT ??
    generated_path("clients/python/src/openkache/_generated/smithy_contract.py"),
  python_native_abi: process.env.OPENKACHE_PYTHON_NATIVE_ABI_OUTPUT ??
    generated_path("clients/python/src/openkache/_generated/smithy_native_abi.py"),
  swift_api: process.env.OPENKACHE_SWIFT_API_OUTPUT ??
    generated_path("clients/swift/generated_local/SmithyAPI.swift"),
  swift_native_abi: process.env.OPENKACHE_SWIFT_NATIVE_ABI_OUTPUT ??
    generated_path("clients/swift/generated_local/SmithyNativeABI.swift"),
  c_contract: process.env.OPENKACHE_C_CONTRACT_OUTPUT ??
    generated_path("clients/core/generated_local/smithy_contract.h"),
  go_api: generated_path("clients/go/smithy_api.go"),
  go_contract: generated_path("clients/go/smithy_contract.go"),
  go_native_abi: generated_path("clients/go/generated_local/smithy_native_abi.h"),
  java_api_root: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local",
  ),
  java_contract: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local/SmithyContract.java",
  ),
  java_native_api: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local/SmithyNativeApi.java",
  ),
  java_native_descriptor: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local/SmithyNativeDescriptor.java",
  ),
  java_native_connect_options: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local/SmithyNativeConnectOptions.java",
  ),
  java_operations: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local/SmithyGeneratedOperations.java",
  ),
  kotlin_api: generated_path(
    "clients/kotlin/src/main/kotlin/io/openkache/client/generated_local/SmithyApi.kt",
  ),
  kotlin_contract: generated_path(
    "clients/kotlin/src/main/kotlin/io/openkache/client/generated_local/SmithyContract.kt",
  ),
  kotlin_native_api: generated_path(
    "clients/kotlin/src/main/kotlin/io/openkache/client/generated_local/SmithyNativeApi.kt",
  ),
  kotlin_operations: generated_path(
    "clients/kotlin/src/main/kotlin/io/openkache/client/generated_local/SmithyGeneratedOperations.kt",
  ),
  dart_api: generated_path("clients/dart/lib/generated_local/smithy_api.dart"),
  dart_contract: generated_path("clients/dart/lib/generated_local/smithy_contract.dart"),
  dart_native_api: generated_path("clients/dart/lib/generated_local/smithy_native_api.dart"),
  dart_operations: generated_path("clients/dart/lib/generated_local/smithy_operations.dart"),
} as const

function object_value(value: unknown, location: string): Json_Object {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${location} must be an object`)
  }
  return value as Json_Object
}

function object_member(object: Json_Object, member: string, location: string): Json_Object {
  return object_value(object[member], `${location}.${member}`)
}

function array_member(
  object: Json_Object,
  member: string,
  location: string,
): readonly unknown[] {
  const value = object[member]
  if (!Array.isArray(value)) throw new Error(`${location}.${member} must be an array`)
  return value
}

function string_member(object: Json_Object, member: string, location: string): string {
  const value = object[member]
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${location}.${member} must be a non-empty string`)
  }
  return value
}

function optional_string_member(
  object: Json_Object,
  member: string,
  location: string,
): string | undefined {
  const value = object[member]
  return value === undefined
    ? undefined
    : string_member(object, member, location)
}

function boolean_member(object: Json_Object, member: string, location: string): boolean {
  const value = object[member]
  if (typeof value !== "boolean") {
    throw new Error(`${location}.${member} must be a boolean`)
  }
  return value
}

function optional_boolean_member(
  object: Json_Object,
  member: string,
  location: string,
): boolean {
  const value = object[member]
  return value === undefined ? false : boolean_member(object, member, location)
}

function integer_member(
  object: Json_Object,
  member: string,
  location: string,
  minimum = 0,
  maximum = Number.MAX_SAFE_INTEGER,
): number {
  const value = object[member]
  if (
    typeof value !== "number" ||
    !Number.isSafeInteger(value) ||
    value < minimum ||
    value > maximum
  ) {
    throw new Error(
      `${location}.${member} must be an integer from ${minimum} through ${maximum}`,
    )
  }
  return value
}

function shape_name(shape_id: string): string {
  const separator = shape_id.lastIndexOf("#")
  if (separator < 0 || separator === shape_id.length - 1) {
    throw new Error(`shape ID ${JSON.stringify(shape_id)} has no shape name`)
  }
  return shape_id.slice(separator + 1)
}

function pascal_case(identifier: string): string {
  return identifier
    .split("_")
    .map((part) => {
      const normalized = part.toLowerCase()
      return normalized.length === 0
        ? ""
        : `${normalized[0]?.toUpperCase()}${normalized.slice(1)}`
    })
    .join("")
}

function trait_value_any(
  shape: Json_Object,
  trait_ids: readonly string[],
  location: string,
): Json_Object {
  const traits = object_member(shape, "traits", location)
  for (const trait_id of trait_ids) {
    const value = traits[trait_id]
    if (value !== undefined) return object_value(value, `${location}.traits.${trait_id}`)
  }
  throw new Error(
    `${location}.traits is missing one of ${trait_ids.map((trait_id) => JSON.stringify(trait_id)).join(", ")}`,
  )
}

function optional_object_member(
  object: Json_Object,
  member: string,
  location: string,
): Json_Object | undefined {
  const value = object[member]
  return value === undefined ? undefined : object_value(value, `${location}.${member}`)
}

function shape_type(shape: Json_Object, location: string): string {
  return string_member(shape, "type", location)
}

function api_type(
  shapes: Json_Object,
  target: string,
  member_traits?: Json_Object,
): Api_Type {
  const prelude_types: Readonly<Record<string, Api_Type_Kind>> = {
    "smithy.api#Boolean": "boolean",
    "smithy.api#Integer": "integer",
    "smithy.api#Long": "long",
    "smithy.api#String": "string",
  }
  const prelude =
    member_traits?.[UNSIGNED_LONG_TRAIT_ID] !== undefined &&
    target === "smithy.api#Long"
      ? "unsigned_long"
      : prelude_types[target]
  if (prelude !== undefined) return { kind: prelude }

  const shape = object_member(shapes, target, "Smithy AST.shapes")
  const kind = shape_type(shape, `Smithy AST.shapes.${target}`)
  switch (kind) {
    case "blob":
      return { kind: "blob" }
    case "enum":
      return { kind: "enum", name: shape_name(target) }
    case "structure":
      return { kind: "structure", name: shape_name(target) }
    default:
      throw new Error(`unsupported API member target ${target} with shape type ${kind}`)
  }
}

function api_structure(shapes: Json_Object, target: string): Api_Structure {
  const shape = object_member(shapes, target, "Smithy AST.shapes")
  if (shape_type(shape, `Smithy AST.shapes.${target}`) !== "structure") {
    throw new Error(`${target} must be a structure`)
  }
  const members = object_member(shape, "members", target)
  return {
    name: shape_name(target),
    members: Object.entries(members).map(([name, value]): Api_Member => {
      const member = object_value(value, `${target}.${name}`)
      const traits = optional_object_member(member, "traits", `${target}.${name}`)
      return {
        name,
        required: traits?.["smithy.api#required"] !== undefined,
        type: api_type(
          shapes,
          string_member(member, "target", `${target}.${name}`),
          traits,
        ),
      }
    }),
  }
}

function operation_contract(
  shape: Json_Object,
  target: string,
): Api_Operation_Contract | undefined {
  const traits = optional_object_member(shape, "traits", target)
  const value = traits?.[OPERATION_CONTRACT_TRAIT_ID] ??
    traits?.["openkache.protocol#operationContract"]
  if (value === undefined) return undefined
  const contract = object_value(value, `${target}.traits.${OPERATION_CONTRACT_TRAIT_ID}`)
  const scope = string_member(contract, "scope", `${target}.${OPERATION_CONTRACT_TRAIT_ID}`)
  if (!["global", "item", "namespace", "namespace_management"].includes(scope)) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.scope must be global, item, namespace, or namespace_management`,
    )
  }
  const response_kind = string_member(
    contract,
    "responseKind",
    `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
  )
  if (
    ![
      "empty",
      "pong",
      "echo",
      "value",
      "set_outcome",
      "delete_outcome",
      "stats_json",
      "namespace_descriptor",
    ].includes(response_kind)
  ) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.responseKind is not a supported response kind`,
    )
  }
  const retry_mode = string_member(
    contract,
    "retryMode",
    `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
  )
  if (!["always", "never", "when_not_creating"].includes(retry_mode)) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.retryMode must be always, never, or when_not_creating`,
    )
  }
  const statuses = (member: string): readonly string[] => {
    const values = array_member(
      contract,
      member,
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
    ).map((value, index) => {
      if (typeof value !== "string" || value.length === 0) {
        throw new Error(
          `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member}[${index}] must be a non-empty string`,
        )
      }
      return value
    })
    const unique = new Set(values)
    if (unique.size !== values.length) {
      throw new Error(
        `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member} must not contain duplicate statuses`,
      )
    }
    return values
  }
  const success_statuses = statuses("successStatuses")
  const error_statuses = statuses("errorStatuses")
  if (success_statuses.length === 0 || error_statuses.length === 0) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID} successStatuses and errorStatuses must not be empty`,
    )
  }
  return {
    error_statuses,
    response_kind: response_kind as Api_Operation_Response_Kind,
    retry_mode: retry_mode as Api_Operation_Retry_Mode,
    scope: scope as Api_Operation_Scope,
    success_statuses,
  }
}

function api_enum(shapes: Json_Object, namespace: string, name: string): Api_Enum {
  const shape_id = `${namespace}#${name}`
  const shape = object_member(shapes, shape_id, "Smithy AST.shapes")
  if (shape_type(shape, `Smithy AST.shapes.${shape_id}`) !== "enum") {
    throw new Error(`${shape_id} must be an enum`)
  }
  const members = object_member(shape, "members", shape_id)
  const enum_members = Object.entries(members).map(
    ([member_name, value]): Api_Enum_Member => {
      const member = object_value(value, `${shape_id}.${member_name}`)
      const traits = object_member(member, "traits", `${shape_id}.${member_name}`)
      return {
        name: pascal_case(member_name),
        value: string_member(
          traits,
          "smithy.api#enumValue",
          `${shape_id}.${member_name}.traits`,
        ),
      }
    },
  )
  const member_names = new Set<string>()
  const member_values = new Set<string>()
  for (const member of enum_members) {
    if (member_names.has(member.name)) {
      throw new Error(`duplicate ${name} enum member name ${member.name}`)
    }
    if (member_values.has(member.value)) {
      throw new Error(`duplicate ${name} enum value ${member.value}`)
    }
    member_names.add(member.name)
    member_values.add(member.value)
  }
  return {
    name,
    members: enum_members,
  }
}

function api_contract(
  shapes: Json_Object,
  service_shape_id: string,
  namespace: string,
  fallback_operation_names?: readonly string[],
): Api_Contract {
  const service = object_member(shapes, service_shape_id, "Smithy AST.shapes")
  // The protocol Opcode enum is the canonical operation-name source. A
  // service-local list remains accepted for legacy fixtures, but production
  // client models omit it so adding an opcode cannot require a second name
  // registration.
  const service_operations = service.operations
  const operation_references =
    service_operations === undefined ||
    (Array.isArray(service_operations) && service_operations.length === 0)
      ? (fallback_operation_names ?? []).map((name) => ({
          target: `${namespace}#${name}`,
        }))
      : array_member(service, "operations", service_shape_id)
  if (operation_references.length === 0) {
    throw new Error(`${service_shape_id} must define at least one operation`)
  }
  const operation_shapes = operation_references
    .map((operation, index): Api_Operation => {
      const reference = object_value(operation, `${service_shape_id}.operations[${index}]`)
      const target = string_member(
        reference,
        "target",
        `${service_shape_id}.operations[${index}]`,
      )
      const shape = object_member(shapes, target, "Smithy AST.shapes")
      const input = string_member(
        object_member(shape, "input", target),
        "target",
        `${target}.input`,
      )
      const output = string_member(
        object_member(shape, "output", target),
        "target",
        `${target}.output`,
      )
      const semantic_contract = operation_contract(shape, target)
      return {
        ...(semantic_contract === undefined ? {} : { contract: semantic_contract }),
        input: shape_name(input),
        name: shape_name(target),
        output: shape_name(output),
      }
    })

  const structure_names = new Set<string>()
  for (const operation of operation_shapes) {
    structure_names.add(operation.input)
    structure_names.add(operation.output)
  }
  const enum_names = new Set<string>()
  const structures_by_name = new Map<string, Api_Structure>()
  const pending_structure_names = [...structure_names]
  while (pending_structure_names.length > 0) {
    const name = pending_structure_names.pop()
    if (name === undefined || structures_by_name.has(name)) continue
    const structure = api_structure(shapes, `${namespace}#${name}`)
    structures_by_name.set(name, structure)
    for (const member of structure.members) {
      if (member.type.name === undefined) continue
      if (member.type.kind === "enum") {
        enum_names.add(member.type.name)
      } else if (member.type.kind === "structure") {
        pending_structure_names.push(member.type.name)
      }
    }
  }
  const structures = [...structures_by_name.values()].sort((left, right) =>
    left.name.localeCompare(right.name),
  )

  return {
    enums: [...enum_names]
      .map((name) => api_enum(shapes, namespace, name))
      .sort((left, right) => left.name.localeCompare(right.name)),
    operations: operation_shapes,
    structures,
  }
}

function unique_wire_values(entries: readonly Wire_Entry[], kind: string): void {
  const names = new Set<string>()
  const values = new Set<number>()
  for (const entry of entries) {
    if (names.has(entry.name)) throw new Error(`duplicate ${kind} name ${entry.name}`)
    if (values.has(entry.value)) {
      throw new Error(`duplicate ${kind} wire value ${entry.value}`)
    }
    names.add(entry.name)
    values.add(entry.value)
  }
}

function ffi_operation_contract(
  traits: Json_Object,
  location: string,
): Ffi_Operation_Contract | undefined {
  const value = traits[FFI_OPERATION_CONTRACT_TRAIT_ID]
  if (value === undefined) return undefined
  const contract = object_value(value, `${location}.${FFI_OPERATION_CONTRACT_TRAIT_ID}`)
  const input_kind = string_member(
    contract,
    "inputKind",
    `${location}.${FFI_OPERATION_CONTRACT_TRAIT_ID}`,
  )
  if (!["none", "application_key", "item_id"].includes(input_kind)) {
    throw new Error(
      `${location}.${FFI_OPERATION_CONTRACT_TRAIT_ID}.inputKind must be none, application_key, or item_id`,
    )
  }
  return {
    accepts_set_options: boolean_member(
      contract,
      "acceptsSetOptions",
      `${location}.${FFI_OPERATION_CONTRACT_TRAIT_ID}`,
    ),
    accepts_value: boolean_member(
      contract,
      "acceptsValue",
      `${location}.${FFI_OPERATION_CONTRACT_TRAIT_ID}`,
    ),
    dedicated_abi: boolean_member(
      contract,
      "dedicatedAbi",
      `${location}.${FFI_OPERATION_CONTRACT_TRAIT_ID}`,
    ),
    input_kind: input_kind as Ffi_Input_Kind,
    supports_protected: boolean_member(
      contract,
      "supportsProtected",
      `${location}.${FFI_OPERATION_CONTRACT_TRAIT_ID}`,
    ),
    supports_raw: boolean_member(
      contract,
      "supportsRaw",
      `${location}.${FFI_OPERATION_CONTRACT_TRAIT_ID}`,
    ),
    supports_scoped: boolean_member(
      contract,
      "supportsScoped",
      `${location}.${FFI_OPERATION_CONTRACT_TRAIT_ID}`,
    ),
  }
}

function ffi_enum_entries(
  shapes: Json_Object,
  namespace: string,
  enum_name: string,
  kind: string,
): readonly Ffi_Entry[] {
  const shape_id = `${namespace}#${enum_name}`
  const shape = object_member(shapes, shape_id, "Smithy AST.shapes")
  if (shape_type(shape, `Smithy AST.shapes.${shape_id}`) !== "enum") {
    throw new Error(`${shape_id} must be an enum`)
  }
  const members = object_member(shape, "members", shape_id)
  const entries = Object.entries(members).map(([member_name, value]): Ffi_Entry => {
    const member = object_value(value, `${shape_id}.${member_name}`)
    const traits = object_member(member, "traits", `${shape_id}.${member_name}`)
    const assignment = object_member(
      traits,
      "openkache.client#ffiValue",
      `${shape_id}.${member_name}.traits`,
    )
    const operation_contract =
      enum_name === FFI_ENUMS.operations.name
        ? ffi_operation_contract(traits, `${shape_id}.${member_name}.traits`)
        : undefined
    return {
      name: pascal_case(snake_case(member_name)),
      text: string_member(
        traits,
        "smithy.api#enumValue",
        `${shape_id}.${member_name}.traits`,
      ),
      value: integer_member(
        assignment,
        "value",
        `${shape_id}.${member_name}.traits.${"openkache.client#ffiValue"}`,
        0,
        0xffff_ffff,
      ),
      ...(operation_contract === undefined
        ? {}
        : { operation_contract }),
    }
  })
    .sort((left, right) => left.value - right.value)
  if (entries.length === 0) {
    throw new Error(`${kind} contract must define at least one entry`)
  }
  unique_wire_values(entries, kind)
  const text_values = new Set<string>()
  for (const entry of entries) {
    if (text_values.has(entry.text)) {
      throw new Error(`duplicate ${kind} enum value ${entry.text}`)
    }
    text_values.add(entry.text)
  }
  return entries
}

function descriptor_field_metadata(
  member: Api_Member,
): Omit<Namespace_Descriptor_Field, "offset"> {
  const name = snake_case(member.name)
  const common = {
    name,
    csharp_name: pascal_case(name),
    go_name: go_exported_name(member.name),
    swift_name: swift_property_name(member.name),
  }
  switch (member.type.kind) {
    case "unsigned_long":
      return {
        ...common,
        rust_type: "u64",
        c_type: "uint64_t",
        csharp_type: "ulong",
        go_type: "uint64",
        python_type: "_ctypes.c_uint64",
        swift_type: "UInt64",
        size: 8,
        alignment: 8,
      }
    case "integer":
      return {
        ...common,
        rust_type: "u32",
        c_type: "uint32_t",
        csharp_type: "uint",
        go_type: "uint32",
        python_type: "_ctypes.c_uint32",
        swift_type: "UInt32",
        size: 4,
        alignment: 4,
      }
    default:
      throw new Error(
        `Smithy FfiNamespaceDescriptor member ${member.name} must be an unsigned Long or Integer`,
      )
  }
}

interface Namespace_Descriptor_Contract {
  readonly fields: readonly Namespace_Descriptor_Field[]
  readonly layout: Namespace_Descriptor_Layout
}

function namespace_descriptor_contract(
  shapes: Json_Object,
  namespace: string,
): Namespace_Descriptor_Contract {
  const descriptor = api_structure(shapes, `${namespace}#FfiNamespaceDescriptor`)
  if (descriptor.members.length === 0) {
    throw new Error("Smithy FfiNamespaceDescriptor must define at least one member")
  }
  let offset = 0
  let maximum_alignment = 1
  const fields = descriptor.members.map((member): Namespace_Descriptor_Field => {
    if (!member.required) {
      throw new Error(
        `Smithy FfiNamespaceDescriptor member ${member.name} must be required`,
      )
    }
    const metadata = descriptor_field_metadata(member)
    maximum_alignment = Math.max(maximum_alignment, metadata.alignment)
    const remainder = offset % metadata.alignment
    if (remainder !== 0) offset += metadata.alignment - remainder
    const field_offset = offset
    offset += metadata.size
    return {
      ...metadata,
      offset: field_offset,
    }
  })
  const trailing_remainder = offset % maximum_alignment
  if (trailing_remainder !== 0) offset += maximum_alignment - trailing_remainder
  const descriptor_size = offset
  const offsets = Object.fromEntries(fields.map((field) => [field.name, field.offset]))
  for (let index = 0; index < fields.length; index += 1) {
    const left = fields[index]!
    const left_end = left.offset + left.size
    for (const right of fields.slice(index + 1)) {
      const right_end = right.offset + right.size
      if (left.offset < right_end && right.offset < left_end) {
        throw new Error(
          `namespace descriptor ABI fields ${left.name} and ${right.name} overlap`,
        )
      }
    }
  }
  return {
    fields,
    layout: { size_bytes: descriptor_size, offsets },
  }
}

const NATIVE_ABI_TYPES: readonly Native_Abi_Type[] = [
  "client_pointer",
  "result_pointer",
  "u8_pointer",
  "struct_pointer",
  "size",
  "uint8",
  "int32",
  "uint32",
  "uint64",
  "void",
]

function native_abi_type(
  object: Json_Object,
  member: string,
  location: string,
): Native_Abi_Type {
  const value = string_member(object, member, location)
  if (!(NATIVE_ABI_TYPES as readonly string[]).includes(value)) {
    throw new Error(
      `${location}.${member} must be one of ${NATIVE_ABI_TYPES.join(", ")}`,
    )
  }
  return value as Native_Abi_Type
}

function native_abi_parameter(
  value: unknown,
  location: string,
): Native_Abi_Parameter {
  const parameter = object_value(value, location)
  const type = native_abi_type(parameter, "type", location)
  if (type === "void") {
    throw new Error(`${location}.type cannot be void for a parameter`)
  }
  const structure_name = optional_string_member(parameter, "structureName", location)
  if ((type === "struct_pointer") !== (structure_name !== undefined)) {
    throw new Error(
      `${location}.structureName is required only for struct_pointer parameters`,
    )
  }
  return {
    name: string_member(parameter, "name", location),
    type,
    mutable: boolean_member(parameter, "mutable", location),
    ...(structure_name === undefined ? {} : { structure_name }),
  }
}

function native_abi_parameters(
  object: Json_Object,
  member: string,
  location: string,
): readonly Native_Abi_Parameter[] {
  const value = object[member]
  if (value === undefined) return []
  return array_member(object, member, location).map((parameter, index) =>
    native_abi_parameter(parameter, `${location}.${member}[${index}]`),
  )
}

function native_abi_functions(value: Json_Object): readonly Native_Abi_Function[] {
  const functions = array_member(value, "nativeFunctions", FFI_CONTRACT_TRAIT_ID)
    .map((entry, index): Native_Abi_Function => {
      const function_value = object_value(
        entry,
        `${FFI_CONTRACT_TRAIT_ID}.nativeFunctions[${index}]`,
      )
      const location = `${FFI_CONTRACT_TRAIT_ID}.nativeFunctions[${index}]`
      return {
        name: string_member(function_value, "name", location),
        optional: optional_boolean_member(function_value, "optional", location),
        return_type: native_abi_type(function_value, "returnType", location),
        parameters: native_abi_parameters(function_value, "parameters", location),
      }
    })
  if (functions.length === 0) {
    throw new Error(`${FFI_CONTRACT_TRAIT_ID}.nativeFunctions must not be empty`)
  }
  const names = new Set<string>()
  for (const function_ of functions) {
    if (names.has(function_.name)) {
      throw new Error(`duplicate native ABI function ${function_.name}`)
    }
    names.add(function_.name)
    const parameters = new Set<string>()
    for (const parameter of function_.parameters) {
      if (parameters.has(parameter.name)) {
        throw new Error(
          `duplicate parameter ${parameter.name} in native ABI function ${function_.name}`,
        )
      }
      parameters.add(parameter.name)
    }
  }
  return functions
}

function native_abi_structures(value: Json_Object): readonly Native_Abi_Structure[] {
  const structures = array_member(value, "nativeStructures", FFI_CONTRACT_TRAIT_ID)
    .map((entry, index): Native_Abi_Structure => {
      const structure_value = object_value(
        entry,
        `${FFI_CONTRACT_TRAIT_ID}.nativeStructures[${index}]`,
      )
      const location = `${FFI_CONTRACT_TRAIT_ID}.nativeStructures[${index}]`
      const fields = array_member(structure_value, "fields", location).map((field, field_index) =>
        native_abi_parameter(field, `${location}.fields[${field_index}]`),
      )
      if (fields.length === 0) {
        throw new Error(`${location}.fields must not be empty`)
      }
      return {
        name: string_member(structure_value, "name", location),
        fields,
      }
    })
  const names = new Set<string>()
  for (const structure of structures) {
    if (names.has(structure.name)) {
      throw new Error(`duplicate native ABI structure ${structure.name}`)
    }
    names.add(structure.name)
  }
  return structures
}

function ffi_contract(
  value: unknown,
  shapes: Json_Object,
  namespace: string,
): Ffi_Contract {
  const contract = object_value(value, FFI_CONTRACT_TRAIT_ID)
  const descriptor = namespace_descriptor_contract(shapes, namespace)
  const native_functions = native_abi_functions(contract)
  const native_structures = native_abi_structures(contract)
  const structure_names = new Set([
    "FfiNamespaceDescriptor",
    ...native_structures.map((structure) => structure.name),
  ])
  for (const function_ of native_functions) {
    for (const parameter of function_.parameters) {
      if (
        parameter.type === "struct_pointer" &&
        parameter.structure_name !== undefined &&
        !structure_names.has(parameter.structure_name)
      ) {
        throw new Error(
          `native ABI function ${function_.name} references unknown structure ${parameter.structure_name}`,
        )
      }
    }
  }
  for (const structure of native_structures) {
    const field_names = new Set<string>()
    for (const field of structure.fields) {
      if (field_names.has(field.name)) {
        throw new Error(
          `duplicate field ${field.name} in native ABI structure ${structure.name}`,
        )
      }
      field_names.add(field.name)
      if (
        field.type === "struct_pointer" &&
        field.structure_name !== undefined &&
        !structure_names.has(field.structure_name)
      ) {
        throw new Error(
          `native ABI structure ${structure.name} references unknown structure ${field.structure_name}`,
        )
      }
    }
  }
  return {
    abi_version: integer_member(
      contract,
      "abiVersion",
      `${FFI_CONTRACT_TRAIT_ID}.abiVersion`,
      1,
      0xffff_ffff,
    ),
    connection_states: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.connection_states.name,
      FFI_ENUMS.connection_states.kind,
    ),
    native_abi_functions: native_functions,
    native_abi_structures: native_structures,
    namespace_default_evictions: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.namespace_default_evictions.name,
      FFI_ENUMS.namespace_default_evictions.kind,
    ),
    namespace_default_expirations: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.namespace_default_expirations.name,
      FFI_ENUMS.namespace_default_expirations.kind,
    ),
    namespace_descriptor_decode_statuses: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.namespace_descriptor_decode_statuses.name,
      FFI_ENUMS.namespace_descriptor_decode_statuses.kind,
    ),
    namespace_descriptor_fields: descriptor.fields,
    namespace_descriptor_layout: descriptor.layout,
    namespace_override_policies: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.namespace_override_policies.name,
      FFI_ENUMS.namespace_override_policies.kind,
    ),
    operations: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.operations.name,
      FFI_ENUMS.operations.kind,
    ),
    result_kinds: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.result_kinds.name,
      FFI_ENUMS.result_kinds.kind,
    ),
    set_conditions: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.set_conditions.name,
      FFI_ENUMS.set_conditions.kind,
    ),
  }
}

function value_format_contract(value: unknown): Value_Format_Contract {
  const contract = object_value(value, VALUE_FORMAT_TRAIT_ID)
  const values = {
    aad_domain: string_member(contract, "aadDomain", VALUE_FORMAT_TRAIT_ID),
    compact_encryption_context: string_member(
      contract,
      "compactEncryptionContext",
      VALUE_FORMAT_TRAIT_ID,
    ),
    compact_mac_context: string_member(
      contract,
      "compactMacContext",
      VALUE_FORMAT_TRAIT_ID,
    ),
    compact_synthetic_iv_bytes: integer_member(
      contract,
      "compactSyntheticIvBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
    ),
    compression_none: integer_member(contract, "compressionNone", VALUE_FORMAT_TRAIT_ID, 0, 0xff),
    compression_zstandard: integer_member(
      contract,
      "compressionZstandard",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    data_protection_key_bytes: integer_member(
      contract,
      "dataProtectionKeyBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
    ),
    encryption_compact: integer_member(
      contract,
      "encryptionCompact",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    encryption_none: integer_member(contract, "encryptionNone", VALUE_FORMAT_TRAIT_ID, 0, 0xff),
    encryption_robust: integer_member(
      contract,
      "encryptionRobust",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    format_byte_bytes: integer_member(
      contract,
      "formatByteBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
      1,
    ),
    format_compression_mask: integer_member(
      contract,
      "formatCompressionMask",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    format_encryption_shift: integer_member(
      contract,
      "formatEncryptionShift",
      VALUE_FORMAT_TRAIT_ID,
      0,
      7,
    ),
    item_id_root_context: string_member(
      contract,
      "itemIdRootContext",
      VALUE_FORMAT_TRAIT_ID,
    ),
    robust_context: string_member(contract, "robustContext", VALUE_FORMAT_TRAIT_ID),
    robust_nonce_bytes: integer_member(
      contract,
      "robustNonceBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
    ),
    robust_tag_bytes: integer_member(
      contract,
      "robustTagBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
    ),
    serialization_json: integer_member(
      contract,
      "serializationJson",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    serialization_raw: integer_member(
      contract,
      "serializationRaw",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    value_root_context: string_member(
      contract,
      "valueRootContext",
      VALUE_FORMAT_TRAIT_ID,
    ),
    version: integer_member(contract, "version", VALUE_FORMAT_TRAIT_ID, 1),
    max_vu128_bytes: integer_member(
      contract,
      "maxVu128Bytes",
      VALUE_FORMAT_TRAIT_ID,
      17,
      17,
    ),
  } satisfies Value_Format_Contract

  for (const [member, actual, expected] of [
    ["compactSyntheticIvBytes", values.compact_synthetic_iv_bytes, 16],
    ["robustNonceBytes", values.robust_nonce_bytes, 12],
    ["robustTagBytes", values.robust_tag_bytes, 16],
    ["dataProtectionKeyBytes", values.data_protection_key_bytes, 32],
  ] as const) {
    if (actual !== expected) {
      throw new Error(
        `${VALUE_FORMAT_TRAIT_ID}.${member} must be ${expected} for the current core implementation, got ${actual}`,
      )
    }
  }
  if (values.format_compression_mask !== 0x0f) {
    throw new Error(
      "value format compression mask must cover exactly the low four format bits",
    )
  }
  if (values.format_encryption_shift !== 4) {
    throw new Error("value format encryption shift must be exactly four bits")
  }
  const format_encryption_mask =
    values.format_compression_mask << values.format_encryption_shift
  if (format_encryption_mask !== 0xf0) {
    throw new Error("value format encryption mask must cover exactly the high four format bits")
  }
  const version_bytes = encode_vu128(values.version)
  if (version_bytes.length > values.max_vu128_bytes) {
    throw new Error(
      `value format version encodes to ${version_bytes.length} bytes, exceeding maxVu128Bytes ${values.max_vu128_bytes}`,
    )
  }
  for (const [kind, value] of [
    ["compression", values.compression_none],
    ["compression", values.compression_zstandard],
    ["encryption", values.encryption_none],
    ["encryption", values.encryption_compact],
    ["encryption", values.encryption_robust],
  ] as const) {
    if (value > values.format_compression_mask) {
      throw new Error(`${kind} identifier ${value} does not fit in a format nibble`)
    }
  }

  unique_wire_values(
    [
      { name: "Raw", value: values.serialization_raw },
      { name: "Json", value: values.serialization_json },
    ],
    "serialization",
  )
  unique_wire_values(
    [
      { name: "None", value: values.compression_none },
      { name: "Zstandard", value: values.compression_zstandard },
    ],
    "compression",
  )
  unique_wire_values(
    [
      { name: "None", value: values.encryption_none },
      { name: "Compact", value: values.encryption_compact },
      { name: "Robust", value: values.encryption_robust },
    ],
    "encryption",
  )
  return values
}

function value_envelope_contract(value: unknown): Value_Envelope_Contract {
  const contract = object_value(value, VALUE_ENVELOPE_TRAIT_ID)
  const magic_and_version_hex = string_member(
    contract,
    "magicAndVersionHex",
    VALUE_ENVELOPE_TRAIT_ID,
  ).toLowerCase()
  if (
    magic_and_version_hex.length === 0 ||
    magic_and_version_hex.length % 2 !== 0 ||
    magic_and_version_hex.length !== 8 ||
    !/^[0-9a-f]+$/.test(magic_and_version_hex)
  ) {
    throw new Error(
      "openkache.protocol#valueEnvelope.magicAndVersionHex must contain exactly four bytes of hexadecimal digits",
    )
  }
  const max_encoding_bytes = integer_member(
    contract,
    "maxEncodingBytes",
    VALUE_ENVELOPE_TRAIT_ID,
    1,
    0xffff,
  )
  const json_encoding = string_member(
    contract,
    "jsonEncoding",
    VALUE_ENVELOPE_TRAIT_ID,
  )
  if (!valid_encoding_identifier(json_encoding, max_encoding_bytes)) {
    throw new Error(
      `${VALUE_ENVELOPE_TRAIT_ID}.jsonEncoding must match the portable lowercase encoding identifier grammar and fit maxEncodingBytes`,
    )
  }
  return {
    json_encoding,
    magic_and_version_hex,
    max_encoding_bytes,
    max_type_name_bytes: integer_member(
      contract,
      "maxTypeNameBytes",
      VALUE_ENVELOPE_TRAIT_ID,
      1,
      0xffff,
    ),
  }
}

function client_defaults_contract(value: unknown): Client_Defaults_Contract {
  const contract = object_value(value, CLIENT_DEFAULTS_TRAIT_ID)
  const defaults = {
    max_in_flight: integer_member(
      contract,
      "maxInFlight",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    connect_timeout_milliseconds: integer_member(
      contract,
      "connectTimeoutMilliseconds",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    request_timeout_milliseconds: integer_member(
      contract,
      "requestTimeoutMilliseconds",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    retry_max_attempts: integer_member(
      contract,
      "retryMaxAttempts",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    zstandard_level: integer_member(
      contract,
      "zstandardLevel",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    zstandard_minimum_input_bytes: integer_member(
      contract,
      "zstandardMinimumInputBytes",
      CLIENT_DEFAULTS_TRAIT_ID,
      0,
    ),
    zstandard_minimum_savings_bytes: integer_member(
      contract,
      "zstandardMinimumSavingsBytes",
      CLIENT_DEFAULTS_TRAIT_ID,
      0,
    ),
    zstandard_level_min: integer_member(
      contract,
      "zstandardLevelMin",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    zstandard_level_max: integer_member(
      contract,
      "zstandardLevelMax",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    server_name: string_member(contract, "serverName", CLIENT_DEFAULTS_TRAIT_ID),
    certificate_pem_type: string_member(
      contract,
      "certificatePemType",
      CLIENT_DEFAULTS_TRAIT_ID,
    ),
    minimum_positive_value: integer_member(
      contract,
      "minimumPositiveValue",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
  } satisfies Client_Defaults_Contract
  if (defaults.zstandard_level_min > defaults.zstandard_level_max) {
    throw new Error(
      `${CLIENT_DEFAULTS_TRAIT_ID}.zstandardLevelMin must not exceed zstandardLevelMax`,
    )
  }
  if (
    defaults.zstandard_level < defaults.zstandard_level_min ||
    defaults.zstandard_level > defaults.zstandard_level_max
  ) {
    throw new Error(
      `${CLIENT_DEFAULTS_TRAIT_ID}.zstandardLevel must be within the configured range`,
    )
  }
  return defaults
}

function valid_encoding_identifier(encoding: string, maximum_bytes: number): boolean {
  const bytes = new TextEncoder().encode(encoding)
  return (
    bytes.length >= 1 &&
    bytes.length <= maximum_bytes &&
    bytes[0] !== undefined &&
    bytes[0] >= 0x61 &&
    bytes[0] <= 0x7a &&
    bytes.slice(1).every(
      (byte) =>
        (byte >= 0x61 && byte <= 0x7a) ||
        (byte >= 0x30 && byte <= 0x39) ||
        byte === 0x2e ||
        byte === 0x2d,
    )
  )
}

/** Extracts the client-owned Smithy API and native/value contracts.
 *
 * The input AST must contain both `protocol/model` and `clients/model`. Keeping
 * these models separate prevents server builds from depending on client defaults,
 * ABI discriminators, or application-level value-format details.
 */
export function extract_client_contract(ast: unknown): Client_Contract {
  const ast_object = object_value(ast, "Smithy AST")
  const shapes = object_member(ast_object, "shapes", "Smithy AST")
  const wire = extract_protocol_wire_contract(ast)
  const client_service_id = shapes[CLIENT_SERVICE_SHAPE_ID] === undefined
    ? SERVICE_SHAPE_ID
    : CLIENT_SERVICE_SHAPE_ID
  const client_namespace = client_service_id.slice(0, client_service_id.lastIndexOf("#"))
  const service = object_member(shapes, client_service_id, "Smithy AST.shapes")
  const location = `Smithy AST.shapes.${client_service_id}`
  const trait_ids = (trait_id: string): readonly string[] =>
    client_service_id === SERVICE_SHAPE_ID
      ? [trait_id, trait_id.replace("openkache.client#", "openkache.protocol#")]
      : [trait_id]
  const value_format_trait = trait_value_any(service, trait_ids(VALUE_FORMAT_TRAIT_ID), location)
  const value_envelope_trait = trait_value_any(service, trait_ids(VALUE_ENVELOPE_TRAIT_ID), location)
  const client_defaults_trait = trait_value_any(service, trait_ids(CLIENT_DEFAULTS_TRAIT_ID), location)
  const ffi_trait = trait_value_any(service, trait_ids(FFI_CONTRACT_TRAIT_ID), location)
  const parsed_api = api_contract(
    shapes,
    client_service_id,
    client_namespace,
    wire.opcodes.map((entry) => entry.name),
  )
  const api = {
    ...parsed_api,
    // Smithy AST output is not required to preserve service-operation order.
    // Use the protocol assignments so every generated API presents operations
    // in the same stable order as the wire contract.
    operations: [...parsed_api.operations].sort(
      (left, right) =>
        (wire.opcodes.find((entry) => entry.name === left.name)?.value ?? 0) -
        (wire.opcodes.find((entry) => entry.name === right.name)?.value ?? 0),
    ),
  }
  const opcode_names = new Set(wire.opcodes.map((entry) => entry.name))
  const status_names = new Set(
    wire.statuses.flatMap((entry) => [
      entry.name,
      entry.text ?? snake_case(entry.name),
    ]),
  )
  const api_operation_names = new Set<string>()
  for (const operation of api.operations) {
    if (api_operation_names.has(operation.name)) {
      throw new Error(`duplicate client operation ${operation.name}`)
    }
    api_operation_names.add(operation.name)
    if (!opcode_names.has(operation.name)) {
      throw new Error(
        `client operation ${operation.name} has no matching protocol opcode`,
      )
    }
    if (client_service_id === CLIENT_SERVICE_SHAPE_ID && operation.contract === undefined) {
      throw new Error(
        `client operation ${operation.name} is missing ${OPERATION_CONTRACT_TRAIT_ID}`,
      )
    }
    if (operation.contract !== undefined) {
      for (const status of [
        ...operation.contract.success_statuses,
        ...operation.contract.error_statuses,
      ]) {
        if (!status_names.has(status)) {
          throw new Error(
            `client operation ${operation.name} references unknown protocol status ${status}`,
          )
        }
      }
      if (
        operation.contract.success_statuses.some((status) =>
          operation.contract?.error_statuses.includes(status),
        )
      ) {
        throw new Error(
          `client operation ${operation.name} has overlapping success and error statuses`,
        )
      }
    }
  }
  for (const opcode of wire.opcodes) {
    if (!api_operation_names.has(opcode.name)) {
      throw new Error(
        `wire opcode ${opcode.name} has no matching client operation`,
      )
    }
  }
  const ffi = ffi_contract(ffi_trait, shapes, client_namespace)
  if (
    client_service_id === CLIENT_SERVICE_SHAPE_ID &&
    ffi.operations.some((entry) => entry.operation_contract === undefined)
  ) {
    const missing = ffi.operations
      .filter((entry) => entry.operation_contract === undefined)
      .map((entry) => entry.name)
      .join(", ")
    throw new Error(
      `FFI operations are missing ${FFI_OPERATION_CONTRACT_TRAIT_ID}: ${missing}`,
    )
  }
  const opcode_values = new Set(wire.opcodes.map((entry) => entry.value))
  for (const entry of ffi.operations) {
    if (opcode_values.has(entry.value)) {
      throw new Error(
        `FFI operation ${entry.name} wire value ${entry.value} overlaps a protocol opcode`,
      )
    }
  }
  return {
    ...wire,
    api,
    client_defaults: client_defaults_contract(client_defaults_trait),
    ffi,
    value_envelope: value_envelope_contract(value_envelope_trait),
    value_format: value_format_contract(value_format_trait),
  }
}

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function formatted_byte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`
}

function c_unsigned_literal(value: number): string {
  if (value <= 9) return `${value}u`
  return `0x${value.toString(16)}u`
}

function encode_vu128(value: number): readonly number[] {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`cannot VU128-encode invalid integer ${value}`)
  }
  let encoded = BigInt(value)
  if (encoded < 0x80n) return [Number(encoded)]
  if (encoded < 0x1000_0000n) {
    if (encoded < 0x4000n) {
      encoded <<= 2n
      return [
        0x80 | ((Number(encoded) & 0xff) >> 2),
        Number(encoded >> 8n) & 0xff,
      ]
    }
    if (encoded < 0x20_0000n) {
      encoded <<= 3n
      return [
        0xc0 | ((Number(encoded) & 0xff) >> 3),
        Number(encoded >> 8n) & 0xff,
        Number(encoded >> 16n) & 0xff,
      ]
    }
    encoded <<= 4n
    return [
      0xe0 | ((Number(encoded) & 0xff) >> 4),
      Number(encoded >> 8n) & 0xff,
      Number(encoded >> 16n) & 0xff,
      Number(encoded >> 24n) & 0xff,
    ]
  }

  const bytes: number[] = []
  let remaining = encoded
  while (remaining > 0n) {
    bytes.push(Number(remaining & 0xffn))
    remaining >>= 8n
  }
  const length = bytes.length
  if (length < 4 || length > 16) {
    throw new Error(`cannot VU128-encode integer ${value}`)
  }
  return [
    0xf0 | (length - 1),
    ...bytes,
  ]
}

function bytes_from_hex(value: string, location: string): readonly number[] {
  const bytes: number[] = []
  for (let index = 0; index < value.length; index += 2) {
    const pair = value.slice(index, index + 2)
    if (!/^[0-9a-f]{2}$/i.test(pair)) {
      throw new Error(`${location} contains invalid hexadecimal`)
    }
    const byte = Number.parseInt(pair, 16)
    bytes.push(byte)
  }
  return bytes
}

function rust_string_literal(value: string): string {
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

function rust_byte_array_literal(bytes: readonly number[]): string {
  return `[${bytes.map(formatted_byte).join(", ")}]`
}

function c_string_literal(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let literal = '"'
  for (const byte of bytes) {
    if (byte >= 0x20 && byte <= 0x7e && byte !== 0x22 && byte !== 0x5c) {
      literal += String.fromCharCode(byte)
    } else if (byte === 0x22) {
      literal += '\\"'
    } else if (byte === 0x5c) {
      literal += "\\\\"
    } else {
      literal += `\\${byte.toString(8).padStart(3, "0")}`
    }
  }
  return `${literal}"`
}

function snake_case(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .replace(/-/g, "_")
    .toLowerCase()
}

function typescript_name(identifier: string): string {
  return snake_case(identifier)
    .split("_")
    .map((part) => `${part[0]?.toUpperCase()}${part.slice(1)}`)
    .join("_")
}

function typescript_api_name(identifier: string): string {
  return `Smithy_${typescript_name(identifier)}`
}

function swift_name(identifier: string): string {
  return identifier
    .split(/[_-]/)
    .filter((part) => part.length > 0)
    .map((part) => {
      const normalized =
        part === part.toUpperCase()
          ? part.toLowerCase()
          : `${part[0]?.toLowerCase()}${part.slice(1)}`
      return `${normalized[0]?.toUpperCase()}${normalized.slice(1)}`
    })
    .join("")
}

function swift_property_name(identifier: string): string {
  const name = swift_name(identifier)
  return name.length === 0 ? name : `${name[0]?.toLowerCase()}${name.slice(1)}`
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
  const enum_variant = (
    value: string,
    allowed: readonly string[],
    label: string,
  ): string => {
    if (!allowed.includes(value)) {
      throw new Error(`operation metadata ${label} has unsupported value ${value}`)
    }
    return pascal_case(snake_case(value))
  }
  const scope_variants = ["global", "item", "namespace", "namespace_management"] as const
  const response_variants = [
    "empty",
    "pong",
    "echo",
    "value",
    "set_outcome",
    "delete_outcome",
    "stats_json",
    "namespace_descriptor",
  ] as const
  const retry_variants = ["always", "never", "when_not_creating"] as const
  const status_slice = (statuses: readonly string[]): string =>
    `&[${statuses
      .map(
        (status) =>
          `openkache_protocol::Status::${rust_status_variant(contract, status)}`,
      )
      .join(", ")}]`
  const metadata = operations
    .map((operation) => {
      const operation_contract = operation.contract!
      const opcode = contract.opcodes.find((entry) => entry.name === operation.name)
      if (opcode === undefined) {
        throw new Error(`operation metadata has no matching opcode ${operation.name}`)
      }
      return `        openkache_protocol::Opcode::${operation.name} => OperationContract {
            scope: OperationScope::${enum_variant(operation_contract.scope, scope_variants, `${operation.name}.scope`)},
            response_kind: OperationResponseKind::${enum_variant(operation_contract.response_kind, response_variants, `${operation.name}.responseKind`)},
            retry_mode: OperationRetryMode::${enum_variant(operation_contract.retry_mode, retry_variants, `${operation.name}.retryMode`)},
            success_statuses: ${status_slice(operation_contract.success_statuses)},
            error_statuses: ${status_slice(operation_contract.error_statuses)},
        },`
    })
    .join("\n")
  return `/// Request scope declared by the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationScope {
    Global,
    Item,
    Namespace,
    NamespaceManagement,
}

/// Response payload shape declared by the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationResponseKind {
    Empty,
    Pong,
    Echo,
    Value,
    SetOutcome,
    DeleteOutcome,
    StatsJson,
    NamespaceDescriptor,
}

/// Retry behavior declared by the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationRetryMode {
    Always,
    Never,
    WhenNotCreating,
}

/// Generated semantic metadata for one protocol operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationContract {
    pub scope: OperationScope,
    pub response_kind: OperationResponseKind,
    pub retry_mode: OperationRetryMode,
    pub success_statuses: &'static [openkache_protocol::Status],
    pub error_statuses: &'static [openkache_protocol::Status],
}

/// Returns the generated contract for a protocol operation.
pub const fn operation_contract(
    opcode: openkache_protocol::Opcode,
) -> OperationContract {
    match opcode {
${metadata}
    }
}
`
}

type Resolved_Ffi_Operation_Contract = Ffi_Operation_Contract

function protocol_ffi_operation_contract(
  operation: Api_Operation,
): Resolved_Ffi_Operation_Contract | undefined {
  const semantic = operation.contract
  if (semantic === undefined) return undefined
  switch (semantic.scope) {
    case "global":
      return {
        input_kind: "none",
        accepts_value: semantic.response_kind === "echo",
        accepts_set_options: false,
        supports_protected: true,
        supports_raw: true,
        supports_scoped: false,
        dedicated_abi: false,
      }
    case "item":
      return {
        input_kind: "item_id",
        accepts_value: semantic.response_kind === "set_outcome",
        accepts_set_options: semantic.response_kind === "set_outcome",
        supports_protected: true,
        supports_raw: true,
        supports_scoped: true,
        dedicated_abi: false,
      }
    case "namespace":
      return {
        input_kind: "none",
        accepts_value: false,
        accepts_set_options: false,
        supports_protected: true,
        supports_raw: true,
        supports_scoped: true,
        dedicated_abi: false,
      }
    case "namespace_management":
      return {
        input_kind: "none",
        accepts_value: false,
        accepts_set_options: false,
        supports_protected: false,
        supports_raw: false,
        supports_scoped: false,
        dedicated_abi: true,
      }
  }
}

function render_rust_ffi_operation_contract(contract: Client_Contract): string {
  const protocol_contracts = new Map(
    contract.api.operations.map((operation) => [
      operation.name,
      protocol_ffi_operation_contract(operation),
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
            accepts_value: ${metadata.accepts_value},
            accepts_set_options: ${metadata.accepts_set_options},
            supports_protected: ${metadata.supports_protected},
            supports_raw: ${metadata.supports_raw},
            supports_scoped: ${metadata.supports_scoped},
            dedicated_abi: ${metadata.dedicated_abi},
        },`
    })
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
`
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

/// Version of the native client FFI contract.
pub const FFI_ABI_VERSION: u32 = ${formatted_decimal(ffi.abi_version)};
${ffi_operations}
${ffi_result_kinds}
${ffi_connection_states}
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
/// Canonical JSON serialized-value identifier.
pub const VALUE_FORMAT_SERIALIZATION_JSON: u8 = ${formatted_byte(value.serialization_json)};
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
  const set_condition_entries = contract.ffi.set_conditions
    .map(
      (entry) =>
        `    OPENKACHE_CLIENT_SET_CONDITION_${snake_case(entry.name).toUpperCase()} = OPENKACHE_SMITHY_FFI_SET_CONDITION_${snake_case(entry.name).toUpperCase()},`,
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

typedef enum openkache_client_set_condition {
${set_condition_entries}
} openkache_client_set_condition_t;

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
  const native_structures = render_c_native_structures(contract)
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

typedef openkache_smithy_namespace_descriptor_t
    openkache_client_namespace_descriptor_t;

typedef struct openkache_client openkache_client_t;
typedef struct openkache_client_result openkache_client_result_t;

${native_structures}

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

interface Adapter_Contract_Values {
  readonly abi_version: number
  readonly operation_contracts: readonly {
    readonly accepts_value: boolean
    readonly accepts_set_options: boolean
    readonly input_kind: Ffi_Input_Kind
    readonly name: string
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
        : protocol_ffi_operation_contract(operation)
    if (operation_contract === undefined) return undefined
    return {
      accepts_value: operation_contract.accepts_value,
      accepts_set_options: operation_contract.accepts_set_options,
      input_kind: operation_contract.input_kind,
      name: opcode.name,
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

function adapter_contract_values(contract: Client_Contract): Adapter_Contract_Values {
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

function render_java_operation_metadata(
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
`
}

function render_kotlin_operation_metadata(
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
  return `    /** Returns whether the Smithy operation is valid on the scoped exact-ID ABI. */
    public fun operationSupportsScoped(operation: Int): Boolean = when (operation) {
        ${scoped_clause}
    }

    /** Returns whether the scoped ABI requires one exact item ID. */
    public fun operationRequiresItemId(operation: Int): Boolean = when (operation) {
        ${item_clause}
    }
`
}

function render_dart_operation_metadata(
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
  return `/// Returns whether the Smithy operation is valid on the scoped exact-ID ABI.
bool smithyOperationSupportsScoped(int operation) => switch (operation) {
  ${scoped_clause}
};

/// Returns whether the scoped ABI requires one exact item ID.
bool smithyOperationRequiresItemId(int operation) => switch (operation) {
  ${item_clause}
};
`
}

function lower_camel_case(identifier: string): string {
  const pascal =
    /[_-]/.test(identifier) || identifier === identifier.toUpperCase()
      ? pascal_case(identifier)
      : `${identifier[0]?.toUpperCase()}${identifier.slice(1)}`
  return pascal.length === 0
    ? pascal
    : `${pascal[0]?.toLowerCase()}${pascal.slice(1)}`
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
    case "enum":
    case "structure":
      if (type.name === undefined) throw new Error(`Java API ${type.kind} has no name`)
      rendered = type.name
      break
    case "integer":
      rendered = "int"
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
        ["blob", "enum", "string", "structure"].includes(member.type.kind),
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

function java_api_operation_interface_source(operation: Api_Operation): string {
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
): Readonly<Record<string, string>> {
  const outputs: Record<string, string> = {
    [join(GENERATED_OUTPUTS.java_api_root, "SmithyOpenKacheApi.java")]:
      java_api_interface_source(contract),
  }
  for (const operation of contract.api.operations) {
    outputs[join(GENERATED_OUTPUTS.java_api_root, `Smithy${operation.name}Api.java`)] =
      java_api_operation_interface_source(operation)
  }
  for (const enum_ of contract.api.enums) {
    outputs[join(GENERATED_OUTPUTS.java_api_root, `${enum_.name}.java`)] =
      java_api_enum_source(enum_)
  }
  for (const structure of contract.api.structures) {
    outputs[join(GENERATED_OUTPUTS.java_api_root, `${structure.name}.java`)] =
      java_api_structure_source(structure)
  }
  return outputs
}

function native_abi_structure_class_name(structure_name: string): string {
  if (structure_name === "FfiNamespaceDescriptor") return "SmithyNativeDescriptor"
  return `SmithyNative${structure_name.replace(/^Ffi/, "")}`
}

function native_abi_java_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  switch (type) {
    case "client_pointer":
    case "result_pointer":
    case "u8_pointer":
      return "Pointer"
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Java native struct pointer has no structure name")
      }
      return native_abi_structure_class_name(structure_name)
    case "size":
    case "uint64":
      return "long"
    case "int32":
    case "uint32":
      return "int"
    case "uint8":
      return "byte"
    case "void":
      return "void"
  }
}

function native_abi_kotlin_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  switch (type) {
    case "client_pointer":
    case "result_pointer":
    case "u8_pointer":
      return "Pointer?"
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Kotlin native struct pointer has no structure name")
      }
      return native_abi_structure_class_name(structure_name)
    case "size":
    case "uint64":
      return "Long"
    case "int32":
    case "uint32":
      return "Int"
    case "uint8":
      return "Byte"
    case "void":
      return "Unit"
  }
}

function native_abi_dart_native_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  switch (type) {
    case "client_pointer":
      return "ffi.Pointer<SmithyNativeClient>"
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Dart native struct pointer has no structure name")
      }
      return `ffi.Pointer<${native_abi_structure_class_name(structure_name)}>`
    case "result_pointer":
      return "ffi.Pointer<SmithyNativeResult>"
    case "u8_pointer":
      return "ffi.Pointer<ffi.Uint8>"
    case "size":
      return "ffi.UintPtr"
    case "int32":
      return "ffi.Int32"
    case "uint32":
      return "ffi.Uint32"
    case "uint8":
      return "ffi.Uint8"
    case "uint64":
      return "ffi.Uint64"
    case "void":
      return "ffi.Void"
  }
}

function native_abi_dart_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  switch (type) {
    case "client_pointer":
      return "ffi.Pointer<SmithyNativeClient>"
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Dart native struct pointer has no structure name")
      }
      return `ffi.Pointer<${native_abi_structure_class_name(structure_name)}>`
    case "result_pointer":
      return "ffi.Pointer<SmithyNativeResult>"
    case "u8_pointer":
      return "ffi.Pointer<ffi.Uint8>"
    case "size":
    case "int32":
    case "uint32":
    case "uint8":
    case "uint64":
      return "int"
    case "void":
      return "void"
  }
}

function native_abi_dart_suffix(name: string): string {
  const prefix = "openkache_client_"
  const suffix = name.startsWith(prefix) ? name.slice(prefix.length) : name
  return suffix === "free" ? "client_free" : suffix
}

function native_abi_structure_c_type_name(structure_name: string): string {
  if (structure_name === "FfiNamespaceDescriptor") {
    return "openkache_client_namespace_descriptor_t"
  }
  return `openkache_client_${snake_case(structure_name.replace(/^Ffi/, ""))}_t`
}

function native_abi_c_identifier(identifier: string): string {
  const normalized = snake_case(identifier)
  return normalized
    .replace(/_milliseconds\b/g, "_ms")
    .replace(/_namespace_id\b/g, "_namespace_id")
}

function native_abi_c_type(
  type: Native_Abi_Type,
  mutable: boolean,
  structure_name?: string,
): string {
  const pointer_qualifier = mutable ? "" : "const "
  switch (type) {
    case "client_pointer":
      return `${pointer_qualifier}openkache_client_t *`
    case "result_pointer":
      return `${pointer_qualifier}openkache_client_result_t *`
    case "u8_pointer":
      return `${pointer_qualifier}uint8_t *`
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("C native struct pointer has no structure name")
      }
      return `${pointer_qualifier}${native_abi_structure_c_type_name(structure_name)} *`
    case "size":
      return "size_t"
    case "uint8":
      return "uint8_t"
    case "int32":
      return "int32_t"
    case "uint32":
      return "uint32_t"
    case "uint64":
      return "uint64_t"
    case "void":
      return "void"
  }
}

function native_abi_c_return_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  if (type === "u8_pointer") return "const uint8_t *"
  return native_abi_c_type(type, true, structure_name)
}

function render_c_native_structures(contract: Client_Contract): string {
  return contract.ffi.native_abi_structures
    .map((structure) => {
      const fields = structure.fields
        .map(
          (field) =>
            `    ${native_abi_c_type(field.type, field.mutable, field.structure_name)} ${native_abi_c_identifier(field.name)};`,
        )
        .join("\n")
      return `typedef struct ${native_abi_structure_c_type_name(structure.name).replace(/_t$/, "")} {
${fields}
} ${native_abi_structure_c_type_name(structure.name)};`
    })
    .join("\n\n")
}

function render_c_native_functions(contract: Client_Contract): string {
  return contract.ffi.native_abi_functions
    .map((function_) => {
      const parameters = function_.parameters.length === 0
        ? "void"
        : function_.parameters
          .map(
            (parameter) =>
              `    ${native_abi_c_type(parameter.type, parameter.mutable, parameter.structure_name)} ${native_abi_c_identifier(parameter.name)}`,
          )
          .join(",\n")
      return `${native_abi_c_return_type(function_.return_type)} ${function_.name}(
${parameters}
);`
    })
    .join("\n\n")
}

function native_abi_c_function_typedef_name(function_name: string): string {
  return `${function_name}_fn`
}

function render_c_native_function_typedefs(contract: Client_Contract): string {
  return contract.ffi.native_abi_functions
    .map((function_) => {
      const parameters = function_.parameters.length === 0
        ? "void"
        : function_.parameters
          .map(
            (parameter) =>
              native_abi_c_type(parameter.type, parameter.mutable, parameter.structure_name),
          )
          .join(",\n")
      return `typedef ${native_abi_c_return_type(function_.return_type)} (*${native_abi_c_function_typedef_name(function_.name)})(
${parameters}
);`
    })
    .join("\n\n")
}

function render_java_native_structure(
  structure: Native_Abi_Structure,
): string {
  const fields = structure.fields
    .map(
      (field) =>
        `    public ${native_abi_java_type(field.type, field.structure_name)} ${lower_camel_case(field.name)};`,
    )
    .join("\n")
  const field_order = structure.fields
    .map((field) => `"${lower_camel_case(field.name)}"`)
    .join(",\n        ")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.
package io.openkache.client.generated_local;

import com.sun.jna.Structure;
${structure.fields.some((field) => ["client_pointer", "result_pointer", "u8_pointer", "struct_pointer"].includes(field.type)) ? "import com.sun.jna.Pointer;" : ""}

/** C-compatible native ${structure.name} structure. */
@Structure.FieldOrder({
        ${field_order}
})
public final class ${native_abi_structure_class_name(structure.name)} extends Structure {
${fields}
}
`
}

function render_kotlin_native_structure(
  structure: Native_Abi_Structure,
): string {
  const fields = structure.fields
    .map(
      (field) =>
        `    @JvmField var ${lower_camel_case(field.name)}: ${native_abi_kotlin_type(field.type, field.structure_name)} = ${native_abi_kotlin_default(field.type)}`,
    )
    .join("\n")
  const field_order = structure.fields
    .map((field) => `"${lower_camel_case(field.name)}"`)
    .join(",\n        ")
  return `/** C-compatible native ${structure.name} structure. */
@Structure.FieldOrder(
        ${field_order}
)
public class ${native_abi_structure_class_name(structure.name)} : Structure() {
${fields}
}`
}

function native_abi_kotlin_default(type: Native_Abi_Type): string {
  switch (type) {
    case "client_pointer":
    case "result_pointer":
    case "u8_pointer":
    case "struct_pointer":
      return "null"
    case "size":
    case "int32":
    case "uint32":
    case "uint64":
    case "uint8":
      return "0"
    case "void":
      throw new Error("Kotlin native structure field cannot be void")
  }
}

function native_abi_dart_field(
  field: Native_Abi_Parameter,
): string {
  const name = lower_camel_case(field.name)
  switch (field.type) {
    case "client_pointer":
      return `  external ffi.Pointer<SmithyNativeClient> ${name};`
    case "result_pointer":
      return `  external ffi.Pointer<SmithyNativeResult> ${name};`
    case "u8_pointer":
      return `  external ffi.Pointer<ffi.Uint8> ${name};`
    case "struct_pointer":
      return `  external ffi.Pointer<${native_abi_structure_class_name(field.structure_name!)}> ${name};`
    case "size":
      return `  @ffi.UintPtr()
  external int ${name};`
    case "uint8":
      return `  @ffi.Uint8()
  external int ${name};`
    case "int32":
      return `  @ffi.Int32()
  external int ${name};`
    case "uint32":
      return `  @ffi.Uint32()
  external int ${name};`
    case "uint64":
      return `  @ffi.Uint64()
  external int ${name};`
  }
}

function render_dart_native_structure(
  structure: Native_Abi_Structure,
): string {
  const fields = structure.fields.map(native_abi_dart_field).join("\n\n")
  return `final class ${native_abi_structure_class_name(structure.name)} extends ffi.Struct {
${fields}
}`
}

function required_native_structure(
  contract: Client_Contract,
  name: string,
): Native_Abi_Structure {
  const structure = contract.ffi.native_abi_structures.find(
    (candidate) => candidate.name === name,
  )
  if (structure === undefined) {
    throw new Error(`Smithy native ABI structure ${name} is required`)
  }
  return structure
}

export function render_java_native_connect_options(contract: Client_Contract): string {
  return render_java_native_structure(required_native_structure(contract, "FfiConnectOptions"))
}

export function render_java_native_descriptor(contract: Client_Contract): string {
  const fields = contract.ffi.namespace_descriptor_fields
  const field_order = fields.map((field) => `"${lower_camel_case(field.name)}"`).join(",\n        ")
  const declarations = fields
    .map(
      (field) =>
        `    public ${native_abi_java_type(field.rust_type === "u64" ? "uint64" : "uint32")} ${lower_camel_case(field.name)};`,
    )
    .join("\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated_local;

import com.sun.jna.Structure;

/** C-compatible namespace descriptor returned by the native ABI. */
@Structure.FieldOrder({
        ${field_order}
})
public final class SmithyNativeDescriptor extends Structure {
${declarations}
}
`
}

export function render_java_native_api(contract: Client_Contract): string {
  const methods = contract.ffi.native_abi_functions
    .map((function_) => {
      const parameters = function_.parameters
        .map(
          (parameter) =>
            `${native_abi_java_type(parameter.type, parameter.structure_name)} ${parameter.name}`,
        )
        .join(",\n            ")
      return `    ${native_abi_java_type(function_.return_type)} ${function_.name}(${parameters});`
    })
    .join("\n\n")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.
package io.openkache.client.generated_local;

import com.sun.jna.Library;
import com.sun.jna.Pointer;

/** Native function declarations shared by managed language adapters. */
public interface SmithyNativeApi extends Library {
${methods}
}
`
}

export function render_kotlin_native_api(contract: Client_Contract): string {
  const methods = contract.ffi.native_abi_functions
    .map((function_) => {
      const parameters = function_.parameters
        .map(
          (parameter) =>
            `${parameter.name}: ${native_abi_kotlin_type(parameter.type, parameter.structure_name)}`,
        )
        .join(",\n            ")
      return `    fun ${function_.name}(${parameters}): ${native_abi_kotlin_type(function_.return_type)}`
    })
    .join("\n\n")
  const fields = contract.ffi.namespace_descriptor_fields
    .map(
      (field) =>
        `    @JvmField var ${lower_camel_case(field.name)}: ${field.rust_type === "u64" ? "Long" : "Int"} = 0`,
    )
    .join("\n")
  const field_order = contract.ffi.namespace_descriptor_fields
    .map((field) => `"${lower_camel_case(field.name)}"`)
    .join(",\n        ")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.
package io.openkache.client.generated_local

import com.sun.jna.Library
import com.sun.jna.Pointer
import com.sun.jna.Structure

/** Native function declarations shared by managed language adapters. */
public interface SmithyNativeApi : Library {
${methods}
}

/** C-compatible namespace descriptor returned by the native ABI. */
@Structure.FieldOrder(
        ${field_order}
)
public class SmithyNativeDescriptor : Structure() {
${fields}
}

${contract.ffi.native_abi_structures.map(render_kotlin_native_structure).join("\n\n")}
`
}

export function render_dart_native_api(contract: Client_Contract): string {
  const fields = contract.ffi.namespace_descriptor_fields
    .map((field) => {
      const annotation = field.rust_type === "u64" ? "Uint64" : "Uint32"
      return `  @ffi.${annotation}()
  external int ${lower_camel_case(field.name)};`
    })
    .join("\n\n")
  const typedefs = contract.ffi.native_abi_functions
    .map((function_) => {
      const suffix = pascal_case(native_abi_dart_suffix(function_.name))
      const native_parameters = function_.parameters
        .map((parameter) => `  ${native_abi_dart_native_type(parameter.type, parameter.structure_name)}`)
        .join(",\n")
      const dart_parameters = function_.parameters
        .map((parameter) => `  ${native_abi_dart_type(parameter.type, parameter.structure_name)}`)
        .join(",\n")
      const native_signature = function_.parameters.length === 0
        ? "Function()"
        : `Function(\n${native_parameters}\n)`
      const dart_signature = function_.parameters.length === 0
        ? "Function()"
        : `Function(\n${dart_parameters}\n)`
      return `typedef Smithy${suffix}Native = ${native_abi_dart_native_type(function_.return_type)} ${native_signature};

typedef Smithy${suffix}Dart = ${native_abi_dart_type(function_.return_type)} ${dart_signature};`
    })
    .join("\n\n")
  const constructor_initializers = contract.ffi.native_abi_functions
    .map((function_) => {
      const suffix = pascal_case(native_abi_dart_suffix(function_.name))
      const field = lower_camel_case(native_abi_dart_suffix(function_.name))
      return `        ${field} = library.lookupFunction<Smithy${suffix}Native, Smithy${suffix}Dart>(
          '${function_.name}',
        )`
    })
    .join(",\n")
  const members = contract.ffi.native_abi_functions
    .map((function_) => {
      const field = lower_camel_case(native_abi_dart_suffix(function_.name))
      return `  final ${native_abi_dart_type(function_.return_type)} Function(${function_.parameters
        .map((parameter) => native_abi_dart_type(parameter.type, parameter.structure_name))
        .join(", ")}) ${field};`
    })
    .join("\n")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.

import 'dart:ffi' as ffi;
import 'dart:io';

final class SmithyNativeClient extends ffi.Opaque {}

final class SmithyNativeResult extends ffi.Opaque {}

final class SmithyNativeDescriptor extends ffi.Struct {
${fields}
}

${contract.ffi.native_abi_structures.map(render_dart_native_structure).join("\n\n")}

${typedefs}

/** Native function bindings shared by managed language adapters. */
final class SmithyNativeApi {
  SmithyNativeApi(ffi.DynamicLibrary library)
: ${constructor_initializers};

${members}

  static SmithyNativeApi open(String? configuredPath) {
    final path = configuredPath ??
        Platform.environment['OPENKACHE_CLIENT_NATIVE'] ??
        switch (Platform.operatingSystem) {
          'linux' => 'libopenkache_client_core.so',
          'macos' => 'libopenkache_client_core.dylib',
          'windows' => 'openkache_client_core.dll',
          _ => throw UnsupportedError(
              'unsupported platform \${Platform.operatingSystem}',
            ),
        };
    return SmithyNativeApi(ffi.DynamicLibrary.open(path));
  }
}
`
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
    case "enum":
    case "structure":
      if (type.name === undefined) throw new Error(`Kotlin API ${type.kind} has no name`)
      rendered = type.name
      break
    case "integer":
      rendered = "Int"
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

function managed_operation_entries(
  contract: Client_Contract,
): readonly Managed_Api_Operation[] {
  return contract.api.operations.filter(
    (operation): operation is Api_Operation & {
      readonly contract: Api_Operation_Contract
    } => operation.contract !== undefined,
  )
}

function managed_operation_constant(
  operation: Api_Operation,
  language: "java" | "kotlin" | "dart",
): string {
  const identifier = snake_case(operation.name).toUpperCase()
  if (language === "dart") {
    return `smithyOperation${pascal_case(snake_case(operation.name))}`
  }
  return `SmithyContract.OPERATION_${identifier}`
}

function managed_operation_label(operation: Api_Operation): string {
  return snake_case(operation.name).toUpperCase()
}

type Managed_Api_Operation = Api_Operation & {
  readonly contract: Api_Operation_Contract
}

function render_java_operation_method(operation: Managed_Api_Operation): string {
  const operation_constant = managed_operation_constant(operation, "java")
  const operation_label = managed_operation_label(operation)
  const method_name = lower_camel_case(operation.name)
  switch (operation.contract.response_kind) {
    case "pong":
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecute(
                ${operation_constant},
                new byte[0],
                new byte[0],
                0,
                0);
            smithyRequireKind(result, SmithyContract.RESULT_OK, "${operation_label}");
            return new ${operation.output}();
        });
    }`
    case "echo":
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecute(
                ${operation_constant},
                new byte[0],
                input.message().getBytes(StandardCharsets.UTF_8),
                0,
                0);
            smithyRequireKind(result, SmithyContract.RESULT_VALUE, "${operation_label}");
            return new ${operation.output}(
                smithyDecodeUtf8(result.payload(), "${operation_label}"));
        });
    }`
    case "value":
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.namespaceId(),
                input.itemId(),
                new byte[0],
                0,
                0);
            if (result.kind() == SmithyContract.RESULT_NOT_FOUND) {
                return new ${operation.output}(null);
            }
            smithyRequireKind(result, SmithyContract.RESULT_VALUE, "${operation_label}");
            return new ${operation.output}(result.payload());
        });
    }`
    case "set_outcome":
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            SmithySetFlags flags = smithySetFlags(input);
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.namespaceId(),
                input.itemId(),
                input.value(),
                flags.flags(),
                flags.ttlMilliseconds());
            SetOutcome outcome = switch (result.kind()) {
                case SmithyContract.RESULT_CREATED -> SetOutcome.CREATED;
                case SmithyContract.RESULT_REPLACED -> SetOutcome.REPLACED;
                case SmithyContract.RESULT_NOT_STORED -> SetOutcome.NOT_STORED;
                default -> throw smithyUnexpectedKind("${operation_label}", result.kind());
            };
            return new ${operation.output}(outcome);
        });
    }`
    case "delete_outcome":
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.namespaceId(),
                input.itemId(),
                new byte[0],
                0,
                0);
            if (result.kind() == SmithyContract.RESULT_DELETED) {
                return new ${operation.output}(true);
            }
            if (result.kind() == SmithyContract.RESULT_NOT_DELETED) {
                return new ${operation.output}(false);
            }
            throw smithyUnexpectedKind("${operation_label}", result.kind());
        });
    }`
    case "stats_json":
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.namespaceId(),
                new byte[0],
                new byte[0],
                0,
                0);
            smithyRequireKind(result, SmithyContract.RESULT_VALUE, "${operation_label}");
            return new ${operation.output}(
                smithyDecodeUtf8(result.payload(), "${operation_label}"));
        });
    }`
    case "empty":
      if (operation.contract.scope === "namespace") {
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.namespaceId(),
                new byte[0],
                new byte[0],
                0,
                0);
            smithyRequireKind(result, SmithyContract.RESULT_OK, "${operation_label}");
            return new ${operation.output}();
        });
    }`
      }
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyNamespaceDelete(
                input.namespaceId(),
                input.expectedRevision());
            smithyRequireKind(result, SmithyContract.RESULT_OK, "${operation_label}");
            return new ${operation.output}();
        });
    }`
    case "namespace_descriptor":
      if (operation.name === "NamespaceOpen") {
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            byte[] name = input.name().getBytes(StandardCharsets.UTF_8);
            if (name.length > SmithyContract.NAMESPACE_NAME_MAX_BYTES) {
                throw new EchoClientException("namespace name exceeds protocol limit");
            }
            SmithyPolicyFlags policy = smithyPolicyFlags(input.policy(), input.createIfMissing());
            NativeResult result = smithyNamespaceOpen(
                name,
                input.createIfMissing(),
                policy.flags(),
                policy.ttlMilliseconds());
            boolean created = result.kind() == SmithyContract.RESULT_CREATED;
            if (!created && result.kind() != SmithyContract.RESULT_OK) {
                throw smithyUnexpectedKind("${operation_label}", result.kind());
            }
            return new ${operation.output}(
                smithyDecodeDescriptor(result.payload()),
                created);
        });
    }`
      }
      if (operation.name === "NamespaceUpdatePolicy") {
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            SmithyPolicyFlags policy = smithyPolicyFlags(input.policy(), true);
            NativeResult result = smithyNamespaceUpdatePolicy(
                input.namespaceId(),
                input.expectedRevision(),
                policy.flags(),
                policy.ttlMilliseconds());
            smithyRequireKind(result, SmithyContract.RESULT_VALUE, "${operation_label}");
            return new ${operation.output}(smithyDecodeDescriptor(result.payload()));
        });
    }`
      }
      throw new Error(`unsupported generated Java namespace operation ${operation.name}`)
    default:
      throw new Error(
        `unsupported generated Java response kind ${operation.contract.response_kind}`,
      )
  }
}

export function render_java_operations(contract: Client_Contract): string {
  const methods = managed_operation_entries(contract)
    .map(render_java_operation_method)
    .join("\n\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client;

import io.openkache.client.generated_local.SmithyContract;

import java.nio.charset.StandardCharsets;
import java.util.Objects;
import java.util.concurrent.CompletionStage;
import java.util.function.Supplier;

/** Generated operation implementations backed by the shared native contract. */
public interface SmithyGeneratedOperations extends SmithyOpenKacheApi {
    record SmithySetFlags(int flags, long ttlMilliseconds) {}

    record SmithyPolicyFlags(int flags, long ttlMilliseconds) {}

    <T> CompletionStage<T> smithySubmit(Supplier<T> operation);

    NativeResult smithyExecute(
        int operation,
        byte[] applicationKey,
        byte[] value,
        int setCondition,
        long ttlMilliseconds);

    NativeResult smithyExecuteScoped(
        int operation,
        long namespaceId,
        byte[] itemId,
        byte[] value,
        int flags,
        long ttlMilliseconds);

    NativeResult smithyNamespaceOpen(
        byte[] name,
        boolean createIfMissing,
        int policyFlags,
        long ttlMilliseconds);

    NativeResult smithyNamespaceUpdatePolicy(
        long namespaceId,
        long expectedRevision,
        int policyFlags,
        long ttlMilliseconds);

    NativeResult smithyNamespaceDelete(long namespaceId, long expectedRevision);

    NamespaceDescriptor smithyDecodeDescriptor(byte[] payload);

    String smithyDecodeUtf8(byte[] payload, String operation);

    ${methods}

    private static SmithySetFlags smithySetFlags(SetInput input) {
        int flags = switch (input.condition() == null ? SetCondition.ANY : input.condition()) {
            case ANY -> SmithyContract.SET_CONDITION_ANY;
            case IF_ABSENT -> SmithyContract.SET_CONDITION_IF_ABSENT;
            case IF_PRESENT -> SmithyContract.SET_CONDITION_IF_PRESENT;
        };
        ExpirationMode expiration = input.expirationMode()
            == null
            ? (input.ttlMilliseconds() == null
                ? ExpirationMode.INHERIT
                : ExpirationMode.EXPLICIT_TTL)
            : input.expirationMode();
        switch (expiration) {
            case INHERIT -> {
                if (input.ttlMilliseconds() != null) {
                    throw new IllegalArgumentException("INHERIT cannot carry a TTL");
                }
                flags |= SmithyContract.SET_INHERIT_EXPIRATION_BITS;
            }
            case NO_EXPIRY -> {
                if (input.ttlMilliseconds() != null) {
                    throw new IllegalArgumentException("NO_EXPIRY cannot carry a TTL");
                }
                flags |= SmithyContract.SET_NO_EXPIRY_BITS;
            }
            case EXPLICIT_TTL -> {
                if (input.ttlMilliseconds() == null || input.ttlMilliseconds() <= 0) {
                    throw new IllegalArgumentException("EXPLICIT_TTL requires a positive TTL");
                }
                flags |= SmithyContract.SET_EXPLICIT_TTL_BITS;
            }
        }
        flags |= switch (input.evictionMode() == null
            ? EvictionMode.INHERIT
            : input.evictionMode()) {
            case INHERIT -> SmithyContract.SET_INHERIT_EVICTION_BITS;
            case EVICTABLE -> SmithyContract.SET_EVICTABLE_BITS;
            case EVICTION_PROTECTED -> SmithyContract.SET_EVICTION_PROTECTED_BITS;
        };
        if (input.value().length > SmithyContract.MAX_VALUE_BYTES) {
            throw new IllegalArgumentException("value exceeds protocol limit");
        }
        return new SmithySetFlags(
            flags,
            input.ttlMilliseconds() == null ? 0 : input.ttlMilliseconds());
    }

    private static SmithyPolicyFlags smithyPolicyFlags(
        NamespacePolicy policy,
        boolean required) {
        if (required && policy == null) {
            throw new IllegalArgumentException("namespace policy is required");
        }
        if (!required && policy != null) {
            throw new IllegalArgumentException("namespace policy requires createIfMissing");
        }
        if (policy == null) {
            return new SmithyPolicyFlags(0, 0);
        }
        int flags = switch (policy.defaultExpiration()) {
            case NO_EXPIRY -> SmithyContract.POLICY_NO_EXPIRY_BITS;
            case FIXED_TTL -> SmithyContract.POLICY_FIXED_TTL_BITS;
        };
        long ttl = policy.defaultTtlMilliseconds() == null
            ? 0
            : policy.defaultTtlMilliseconds();
        if (policy.defaultExpiration() == ExpirationDefault.FIXED_TTL && ttl <= 0) {
            throw new IllegalArgumentException("FIXED_TTL requires a positive TTL");
        }
        if (policy.defaultExpiration() == ExpirationDefault.NO_EXPIRY && ttl != 0) {
            throw new IllegalArgumentException("NO_EXPIRY cannot carry a TTL");
        }
        if (policy.expirationOverride() == OverridePolicy.ALLOWED) {
            flags |= SmithyContract.POLICY_EXPIRATION_OVERRIDE_FLAG;
        }
        if (policy.defaultEviction() == EvictionDefault.EVICTION_PROTECTED) {
            flags |= SmithyContract.POLICY_EVICTION_PROTECTED_FLAG;
        }
        if (policy.evictionOverride() == OverridePolicy.ALLOWED) {
            flags |= SmithyContract.POLICY_EVICTION_OVERRIDE_FLAG;
        }
        return new SmithyPolicyFlags(flags, ttl);
    }

    private static void smithyRequireKind(
        NativeResult result,
        int expected,
        String operation) {
        if (result.kind() != expected) {
            throw smithyUnexpectedKind(operation, result.kind());
        }
    }

    private static EchoClientException smithyUnexpectedKind(String operation, int kind) {
        return new EchoClientException(
            operation + " returned unexpected native result " + kind);
    }
}
`
}

function render_kotlin_operation_method(operation: Managed_Api_Operation): string {
  const operation_constant = managed_operation_constant(operation, "kotlin")
  const operation_label = managed_operation_label(operation)
  const method_name = lower_camel_case(operation.name)
  const prefix = `    override suspend fun ${method_name}(input: ${operation.input}): ${operation.output} =
        withContext(Dispatchers.IO) {
            requireNotNull(input)
`
  switch (operation.contract.response_kind) {
    case "pong":
      return `${prefix}            val result = smithyInvoke(
                ${operation_constant},
                byteArrayOf(),
                byteArrayOf(),
            )
            smithyRequireKind(result, SmithyContract.RESULT_OK, "${operation_label}")
            ${operation.output}()
        }`
    case "echo":
      return `${prefix}            val result = smithyInvoke(
                ${operation_constant},
                byteArrayOf(),
                input.message.toByteArray(),
            )
            smithyRequireKind(result, SmithyContract.RESULT_VALUE, "${operation_label}")
            ${operation.output}(
                message = smithyDecodeUtf8(result.payload, "${operation_label}"),
            )
        }`
    case "value":
      return `${prefix}            val result = smithyInvokeScoped(
                ${operation_constant},
                input.namespaceId,
                input.itemId,
                byteArrayOf(),
            )
            when (result.kind) {
                SmithyContract.RESULT_VALUE -> ${operation.output}(value = result.payload)
                SmithyContract.RESULT_NOT_FOUND -> ${operation.output}(value = null)
                else -> throw smithyUnexpectedKind("${operation_label}", result.kind)
            }
        }`
    case "set_outcome":
      return `${prefix}            val flags = smithySetFlags(input)
            val result = smithyInvokeScoped(
                ${operation_constant},
                input.namespaceId,
                input.itemId,
                input.value,
                flags.first,
                flags.second,
            )
            val outcome = when (result.kind) {
                SmithyContract.RESULT_CREATED -> SetOutcome.Created
                SmithyContract.RESULT_REPLACED -> SetOutcome.Replaced
                SmithyContract.RESULT_NOT_STORED -> SetOutcome.NotStored
                else -> throw smithyUnexpectedKind("${operation_label}", result.kind)
            }
            ${operation.output}(outcome = outcome)
        }`
    case "delete_outcome":
      return `${prefix}            val result = smithyInvokeScoped(
                ${operation_constant},
                input.namespaceId,
                input.itemId,
                byteArrayOf(),
            )
            when (result.kind) {
                SmithyContract.RESULT_DELETED -> ${operation.output}(deleted = true)
                SmithyContract.RESULT_NOT_DELETED -> ${operation.output}(deleted = false)
                else -> throw smithyUnexpectedKind("${operation_label}", result.kind)
            }
        }`
    case "stats_json":
      return `${prefix}            val result = smithyInvokeScoped(
                ${operation_constant},
                input.namespaceId,
                byteArrayOf(),
                byteArrayOf(),
            )
            smithyRequireKind(result, SmithyContract.RESULT_VALUE, "${operation_label}")
            ${operation.output}(
                json = smithyDecodeUtf8(result.payload, "${operation_label}"),
            )
        }`
    case "empty":
      if (operation.contract.scope === "namespace") {
        return `${prefix}            val result = smithyInvokeScoped(
                ${operation_constant},
                input.namespaceId,
                byteArrayOf(),
                byteArrayOf(),
            )
            smithyRequireKind(result, SmithyContract.RESULT_OK, "${operation_label}")
            ${operation.output}()
        }`
      }
      return `${prefix}            val result = smithyNamespaceDelete(
                input.namespaceId,
                input.expectedRevision,
            )
            smithyRequireKind(result, SmithyContract.RESULT_OK, "${operation_label}")
            ${operation.output}()
        }`
    case "namespace_descriptor":
      if (operation.name === "NamespaceOpen") {
        return `${prefix}            val name = input.name.toByteArray(StandardCharsets.UTF_8)
            require(name.size <= SmithyContract.NAMESPACE_NAME_MAX_BYTES) {
                "namespace name exceeds protocol limit"
            }
            val policy = smithyPolicyFlags(input.policy, input.createIfMissing)
            val result = smithyNamespaceOpen(
                name,
                input.createIfMissing,
                policy.first,
                policy.second,
            )
            val created = result.kind == SmithyContract.RESULT_CREATED
            if (!created && result.kind != SmithyContract.RESULT_OK) {
                throw smithyUnexpectedKind("${operation_label}", result.kind)
            }
            ${operation.output}(
                descriptor = smithyDecodeDescriptor(result.payload),
                created = created,
            )
        }`
      }
      if (operation.name === "NamespaceUpdatePolicy") {
        return `${prefix}            val policy = smithyPolicyFlags(input.policy, true)
            val result = smithyNamespaceUpdatePolicy(
                input.namespaceId,
                input.expectedRevision,
                policy.first,
                policy.second,
            )
            smithyRequireKind(result, SmithyContract.RESULT_VALUE, "${operation_label}")
            ${operation.output}(descriptor = smithyDecodeDescriptor(result.payload))
        }`
      }
      throw new Error(`unsupported generated Kotlin namespace operation ${operation.name}`)
    default:
      throw new Error(
        `unsupported generated Kotlin response kind ${operation.contract.response_kind}`,
      )
  }
}

export function render_kotlin_operations(contract: Client_Contract): string {
  const methods = managed_operation_entries(contract)
    .map(render_kotlin_operation_method)
    .join("\n\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client

import io.openkache.client.generated_local.SmithyContract
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.nio.charset.StandardCharsets

/** Generated operation implementations backed by the shared native contract. */
public interface SmithyGeneratedOperations : SmithyOpenKacheApi {
    public fun smithyInvoke(
        operation: Int,
        applicationKey: ByteArray,
        value: ByteArray,
        setCondition: Int = SmithyContract.SET_CONDITION_ANY,
        ttlMilliseconds: Long = 0,
    ): NativeResult

    public fun smithyInvokeScoped(
        operation: Int,
        namespaceId: Long,
        itemId: ByteArray,
        value: ByteArray,
        flags: Int = 0,
        ttlMilliseconds: Long = 0,
    ): NativeResult

    public fun smithyNamespaceOpen(
        name: ByteArray,
        createIfMissing: Boolean,
        policyFlags: Int,
        ttlMilliseconds: Long,
    ): NativeResult

    public fun smithyNamespaceUpdatePolicy(
        namespaceId: Long,
        expectedRevision: Long,
        policyFlags: Int,
        ttlMilliseconds: Long,
    ): NativeResult

    public fun smithyNamespaceDelete(
        namespaceId: Long,
        expectedRevision: Long,
    ): NativeResult

    public fun smithyDecodeDescriptor(payload: ByteArray): NamespaceDescriptor

    public fun smithyDecodeUtf8(payload: ByteArray, operation: String): String

${methods}

    private fun smithySetFlags(input: SetInput): Pair<Int, Long> {
        var flags = when (input.condition ?: SetCondition.Any) {
            SetCondition.Any -> SmithyContract.SET_CONDITION_ANY
            SetCondition.IfAbsent -> SmithyContract.SET_CONDITION_IF_ABSENT
            SetCondition.IfPresent -> SmithyContract.SET_CONDITION_IF_PRESENT
        }
        val expiration = input.expirationMode
            ?: if (input.ttlMilliseconds == null) ExpirationMode.Inherit else ExpirationMode.ExplicitTtl
        when (expiration) {
            ExpirationMode.Inherit -> {
                require(input.ttlMilliseconds == null) { "INHERIT cannot carry a TTL" }
                flags = flags or SmithyContract.SET_INHERIT_EXPIRATION_BITS
            }
            ExpirationMode.NoExpiry -> {
                require(input.ttlMilliseconds == null) { "NO_EXPIRY cannot carry a TTL" }
                flags = flags or SmithyContract.SET_NO_EXPIRY_BITS
            }
            ExpirationMode.ExplicitTtl -> {
                require(input.ttlMilliseconds != null && input.ttlMilliseconds > 0) {
                    "EXPLICIT_TTL requires a positive TTL"
                }
                flags = flags or SmithyContract.SET_EXPLICIT_TTL_BITS
            }
        }
        flags = flags or when (input.evictionMode ?: EvictionMode.Inherit) {
            EvictionMode.Inherit -> SmithyContract.SET_INHERIT_EVICTION_BITS
            EvictionMode.Evictable -> SmithyContract.SET_EVICTABLE_BITS
            EvictionMode.EvictionProtected -> SmithyContract.SET_EVICTION_PROTECTED_BITS
        }
        require(input.value.size <= SmithyContract.MAX_VALUE_BYTES) {
            "value exceeds protocol limit"
        }
        return flags to (input.ttlMilliseconds ?: 0)
    }

    private fun smithyPolicyFlags(
        policy: NamespacePolicy?,
        required: Boolean,
    ): Pair<Int, Long> {
        if (required) requireNotNull(policy) { "namespace policy is required" }
        if (!required) require(policy == null) { "namespace policy requires createIfMissing" }
        if (policy == null) return 0 to 0
        var flags = when (policy.defaultExpiration) {
            ExpirationDefault.NoExpiry -> SmithyContract.POLICY_NO_EXPIRY_BITS
            ExpirationDefault.FixedTtl -> SmithyContract.POLICY_FIXED_TTL_BITS
        }
        val ttl = policy.defaultTtlMilliseconds ?: 0
        if (policy.defaultExpiration == ExpirationDefault.FixedTtl) {
            require(ttl > 0) { "FIXED_TTL requires a positive TTL" }
        } else {
            require(ttl == 0L) { "NO_EXPIRY cannot carry a TTL" }
        }
        if (policy.expirationOverride == OverridePolicy.Allowed) {
            flags = flags or SmithyContract.POLICY_EXPIRATION_OVERRIDE_FLAG
        }
        if (policy.defaultEviction == EvictionDefault.EvictionProtected) {
            flags = flags or SmithyContract.POLICY_EVICTION_PROTECTED_FLAG
        }
        if (policy.evictionOverride == OverridePolicy.Allowed) {
            flags = flags or SmithyContract.POLICY_EVICTION_OVERRIDE_FLAG
        }
        return flags to ttl
    }

    private fun smithyRequireKind(
        result: NativeResult,
        expected: Int,
        operation: String,
    ) {
        if (result.kind != expected) {
            throw smithyUnexpectedKind(operation, result.kind)
        }
    }

    private fun smithyUnexpectedKind(operation: String, kind: Int) =
        EchoClientException("$operation returned unexpected native result $kind")
}
`
}

function render_dart_operation_method(operation: Managed_Api_Operation): string {
  const operation_constant = managed_operation_constant(operation, "dart")
  const operation_label = managed_operation_label(operation)
  const method_name = lower_camel_case(operation.name)
  const prefix = `  @override
  Future<${operation.output}> ${method_name}(${operation.input} input) => _run(() {
`
  switch (operation.contract.response_kind) {
    case "pong":
      return `${prefix}    final result = _invoke(
      ${operation_constant},
      const <int>[],
      const <int>[],
    );
    _smithyRequireKind(result, smithyResultOk, '${operation_label}');
    return const ${operation.output}();
  });`
    case "echo":
      return `${prefix}    final result = _invoke(
      ${operation_constant},
      const <int>[],
      utf8.encode(input.message),
    );
    _smithyRequireKind(result, smithyResultValue, '${operation_label}');
    return ${operation.output}(
      message: _smithyDecodeUtf8(result.payload, '${operation_label}'),
    );
  });`
    case "value":
      return `${prefix}    final result = _invokeScoped(
      ${operation_constant},
      input.namespaceId,
      input.itemId,
      const <int>[],
    );
    if (result.kind == smithyResultNotFound) {
      return const ${operation.output}();
    }
    _smithyRequireKind(result, smithyResultValue, '${operation_label}');
    return ${operation.output}(value: result.payload);
  });`
    case "set_outcome":
      return `${prefix}    final flags = _smithySetFlags(input);
    final result = _invokeScoped(
      ${operation_constant},
      input.namespaceId,
      input.itemId,
      input.value,
      flags: flags.flags,
      ttlMilliseconds: flags.ttlMilliseconds,
    );
    final outcome = switch (result.kind) {
      smithyResultCreated => SetOutcome.created,
      smithyResultReplaced => SetOutcome.replaced,
      smithyResultNotStored => SetOutcome.notStored,
      _ => throw _smithyUnexpectedKind('${operation_label}', result.kind),
    };
    return ${operation.output}(outcome: outcome);
  });`
    case "delete_outcome":
      return `${prefix}    final result = _invokeScoped(
      ${operation_constant},
      input.namespaceId,
      input.itemId,
      const <int>[],
    );
    return switch (result.kind) {
      smithyResultDeleted => const ${operation.output}(deleted: true),
      smithyResultNotDeleted => const ${operation.output}(deleted: false),
      _ => throw _smithyUnexpectedKind('${operation_label}', result.kind),
    };
  });`
    case "stats_json":
      return `${prefix}    final result = _invokeScoped(
      ${operation_constant},
      input.namespaceId,
      const <int>[],
      const <int>[],
    );
    _smithyRequireKind(result, smithyResultValue, '${operation_label}');
    return ${operation.output}(
      json: _smithyDecodeUtf8(result.payload, '${operation_label}'),
    );
  });`
    case "empty":
      if (operation.contract.scope === "namespace") {
        return `${prefix}    final result = _invokeScoped(
      ${operation_constant},
      input.namespaceId,
      const <int>[],
      const <int>[],
    );
    _smithyRequireKind(result, smithyResultOk, '${operation_label}');
    return const ${operation.output}();
  });`
      }
      return `${prefix}    final result = _readResult(
      _api,
      _api.namespaceDelete(
        _requireOpenHandle(),
        input.namespaceId,
        input.expectedRevision,
      ),
    );
    _smithyRequireKind(result, smithyResultOk, '${operation_label}');
    return const ${operation.output}();
  });`
    case "namespace_descriptor":
      if (operation.name === "NamespaceOpen") {
        return `${prefix}    final name = utf8.encode(input.name);
    if (name.length > smithyNamespaceNameMaxBytes) {
      throw const EchoClientException('namespace name exceeds protocol limit');
    }
    final policy = _smithyPolicyFlags(input.policy, input.createIfMissing);
    final nameBuffer = _Buffer(name);
    try {
      final result = _readResult(
        _api,
        _api.namespaceOpen(
          _requireOpenHandle(),
          nameBuffer.pointer,
          nameBuffer.length,
          input.createIfMissing ? 1 : 0,
          policy.flags,
          policy.ttlMilliseconds,
        ),
      );
      final created = result.kind == smithyResultCreated;
      if (!created && result.kind != smithyResultOk) {
        throw _smithyUnexpectedKind('${operation_label}', result.kind);
      }
      return ${operation.output}(
        descriptor: _decodeDescriptor(result.payload),
        created: created,
      );
    } finally {
      nameBuffer.close();
    }
  });`
      }
      if (operation.name === "NamespaceUpdatePolicy") {
        return `${prefix}    final policy = _smithyPolicyFlags(input.policy, true);
    final result = _readResult(
      _api,
      _api.namespaceUpdatePolicy(
        _requireOpenHandle(),
        input.namespaceId,
        input.expectedRevision,
        policy.flags,
        policy.ttlMilliseconds,
      ),
    );
    _smithyRequireKind(result, smithyResultValue, '${operation_label}');
    return ${operation.output}(
      descriptor: _decodeDescriptor(result.payload),
    );
  });`
      }
      throw new Error(`unsupported generated Dart namespace operation ${operation.name}`)
    default:
      throw new Error(
        `unsupported generated Dart response kind ${operation.contract.response_kind}`,
      )
  }
}

export function render_dart_operations(contract: Client_Contract): string {
  const methods = managed_operation_entries(contract)
    .map(render_dart_operation_method)
    .join("\n\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
part of '../openkache.dart';
// The generated hook keeps optional arguments aligned with the native ABI.
// ignore_for_file: unused_element_parameter

/// Generated operation implementations backed by the shared native contract.
mixin SmithyGeneratedOperations implements SmithyOpenKacheApi {
  Future<T> _run<T>(T Function() operation);

  _NativeResult _invoke(
    int operation,
    List<int> applicationKey,
    List<int> value, {
    int setCondition = smithySetConditionAny,
    int ttlMilliseconds = 0,
  });

  _NativeResult _invokeScoped(
    int operation,
    int namespaceId,
    List<int> itemId,
    List<int> value, {
    int flags = 0,
    int ttlMilliseconds = 0,
  });

  SmithyNativeApi get _api;

  ffi.Pointer<SmithyNativeClient> _requireOpenHandle();

  NamespaceDescriptor _decodeDescriptor(List<int> payload);

${methods}
}

_SmithySetFlags _smithySetFlags(SetInput input) {
  var flags = switch (input.condition ?? SetCondition.any) {
    SetCondition.any => smithySetConditionAny,
    SetCondition.ifAbsent => smithySetConditionIfAbsent,
    SetCondition.ifPresent => smithySetConditionIfPresent,
  };
  final expiration =
      input.expirationMode ??
      (input.ttlMilliseconds == null
          ? ExpirationMode.inherit
          : ExpirationMode.explicitTtl);
  switch (expiration) {
    case ExpirationMode.inherit:
      if (input.ttlMilliseconds != null) {
        throw ArgumentError('INHERIT cannot carry a TTL');
      }
      flags |= smithySetInheritExpirationBits;
    case ExpirationMode.noExpiry:
      if (input.ttlMilliseconds != null) {
        throw ArgumentError('NO_EXPIRY cannot carry a TTL');
      }
      flags |= smithySetNoExpiryBits;
    case ExpirationMode.explicitTtl:
      if (input.ttlMilliseconds == null || input.ttlMilliseconds! <= 0) {
        throw ArgumentError('EXPLICIT_TTL requires a positive TTL');
      }
      flags |= smithySetExplicitTtlBits;
  }
  flags |= switch (input.evictionMode ?? EvictionMode.inherit) {
    EvictionMode.inherit => smithySetInheritEvictionBits,
    EvictionMode.evictable => smithySetEvictableBits,
    EvictionMode.evictionProtected => smithySetEvictionProtectedBits,
  };
  if (input.value.length > smithyMaxValueBytes) {
    throw ArgumentError('value exceeds protocol limit');
  }
  return _SmithySetFlags(flags, input.ttlMilliseconds ?? 0);
}

_SmithyPolicyFlags _smithyPolicyFlags(
  NamespacePolicy? policy,
  bool required,
) {
  if (required && policy == null) {
    throw ArgumentError('namespace policy is required');
  }
  if (!required && policy != null) {
    throw ArgumentError('namespace policy requires createIfMissing');
  }
  if (policy == null) return const _SmithyPolicyFlags(0, 0);
  var flags = switch (policy.defaultExpiration) {
    ExpirationDefault.noExpiry => smithyPolicyNoExpiryBits,
    ExpirationDefault.fixedTtl => smithyPolicyFixedTtlBits,
  };
  final ttl = policy.defaultTtlMilliseconds ?? 0;
  if (policy.defaultExpiration == ExpirationDefault.fixedTtl) {
    if (ttl <= 0) throw ArgumentError('FIXED_TTL requires a positive TTL');
  } else if (ttl != 0) {
    throw ArgumentError('NO_EXPIRY cannot carry a TTL');
  }
  if (policy.expirationOverride == OverridePolicy.allowed) {
    flags |= smithyPolicyExpirationOverrideFlag;
  }
  if (policy.defaultEviction == EvictionDefault.evictionProtected) {
    flags |= smithyPolicyEvictionProtectedFlag;
  }
  if (policy.evictionOverride == OverridePolicy.allowed) {
    flags |= smithyPolicyEvictionOverrideFlag;
  }
  return _SmithyPolicyFlags(flags, ttl);
}

String _smithyDecodeUtf8(List<int> payload, String operation) {
  try {
    return utf8.decode(payload, allowMalformed: false);
  } on FormatException catch (error) {
    throw EchoClientException('$operation response is not valid UTF-8', error);
  }
}

void _smithyRequireKind(_NativeResult result, int expected, String operation) {
  if (result.kind != expected) {
    throw _smithyUnexpectedKind(operation, result.kind);
  }
}

EchoClientException _smithyUnexpectedKind(String operation, int kind) =>
    EchoClientException('$operation returned unexpected native result $kind');

final class _SmithySetFlags {
  const _SmithySetFlags(this.flags, this.ttlMilliseconds);

  final int flags;
  final int ttlMilliseconds;
}

final class _SmithyPolicyFlags {
  const _SmithyPolicyFlags(this.flags, this.ttlMilliseconds);

  final int flags;
  final int ttlMilliseconds;
}
`
}

/** Renders the native constants consumed by the Java JNA adapter. */
export function render_java_contract(contract: Client_Contract): string {
  const values = adapter_contract_values(contract)
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated_local;

/** Native values shared by the Java adapter and the Rust client-core ABI. */
public final class SmithyContract {
    private SmithyContract() {}

    public static final int ABI_VERSION = ${values.abi_version};
${values.operations
  .map(
    (entry) =>
      `    public static final int OPERATION_${snake_case(entry.name).toUpperCase()} = ${entry.value};`,
  )
  .join("\n")}
${render_java_operation_metadata(values)}
    public static final int RESULT_ERROR = ${values.result_error};
    public static final int RESULT_OK = ${values.result_ok};
    public static final int RESULT_VALUE = ${values.result_value};
    public static final int RESULT_NOT_FOUND = ${values.result_not_found};
    public static final int RESULT_CREATED = ${values.result_created};
    public static final int RESULT_REPLACED = ${values.result_replaced};
    public static final int RESULT_DELETED = ${values.result_deleted};
    public static final int RESULT_NOT_DELETED = ${values.result_not_deleted};
    public static final int RESULT_CONNECTED = ${values.result_connected};
    public static final int RESULT_NOT_STORED = ${values.result_not_stored};
    public static final int DESCRIPTOR_DECODE_OK = ${values.descriptor_decode_ok};
    public static final int DEFAULT_EXPIRATION_NO_EXPIRY = ${values.default_expiration_no_expiry};
    public static final int DEFAULT_EXPIRATION_FIXED_TTL = ${values.default_expiration_fixed_ttl};
    public static final int DEFAULT_EVICTION_EVICTABLE = ${values.default_eviction_evictable};
    public static final int DEFAULT_EVICTION_PROTECTED = ${values.default_eviction_protected};
    public static final int OVERRIDE_DISALLOWED = ${values.override_disallowed};
    public static final int OVERRIDE_ALLOWED = ${values.override_allowed};
    public static final int SET_CONDITION_ANY = ${values.set_condition_any};
    public static final int SET_CONDITION_IF_ABSENT = ${values.set_condition_if_absent};
    public static final int SET_CONDITION_IF_PRESENT = ${values.set_condition_if_present};
    public static final int SET_INHERIT_EXPIRATION_BITS = ${values.set_inherit_expiration};
    public static final int SET_NO_EXPIRY_BITS = ${values.set_no_expiry};
    public static final int SET_EXPLICIT_TTL_BITS = ${values.set_explicit_ttl};
    public static final int SET_INHERIT_EVICTION_BITS = ${values.set_inherit_eviction};
    public static final int SET_EVICTABLE_BITS = ${values.set_evictable};
    public static final int SET_EVICTION_PROTECTED_BITS = ${values.set_eviction_protected};
    public static final int POLICY_NO_EXPIRY_BITS = ${values.policy_no_expiry};
    public static final int POLICY_FIXED_TTL_BITS = ${values.policy_fixed_ttl};
    public static final int POLICY_EXPIRATION_OVERRIDE_FLAG = ${values.policy_expiration_override};
    public static final int POLICY_EVICTION_PROTECTED_FLAG = ${values.policy_eviction_protected};
    public static final int POLICY_EVICTION_OVERRIDE_FLAG = ${values.policy_eviction_override};
    public static final int ITEM_ID_BYTES = ${values.item_id_bytes};
    public static final int NAMESPACE_NAME_MAX_BYTES = ${values.namespace_name_max_bytes};
    public static final int MAX_VALUE_BYTES = ${values.max_value_bytes};
    public static final int DEFAULT_ZSTANDARD_LEVEL = ${values.default_zstandard_level};
    public static final long DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES = ${values.default_zstandard_minimum_input_bytes}L;
    public static final long DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES = ${values.default_zstandard_minimum_savings_bytes}L;
    public static final long DEFAULT_CONNECT_TIMEOUT_MILLISECONDS = ${values.default_connect_timeout_milliseconds}L;
    public static final long DEFAULT_REQUEST_TIMEOUT_MILLISECONDS = ${values.default_request_timeout_milliseconds}L;
}
`
}

/** Renders the native constants consumed by the Kotlin JNA adapter. */
export function render_kotlin_contract(contract: Client_Contract): string {
  const values = adapter_contract_values(contract)
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated_local

/** Native values shared by the Kotlin adapter and the Rust client-core ABI. */
public object SmithyContract {
    public const val ABI_VERSION: Int = ${values.abi_version}
${values.operations
  .map(
    (entry) =>
      `    public const val OPERATION_${snake_case(entry.name).toUpperCase()}: Int = ${entry.value}`,
  )
  .join("\n")}
${render_kotlin_operation_metadata(values)}
    public const val RESULT_ERROR: Int = ${values.result_error}
    public const val RESULT_OK: Int = ${values.result_ok}
    public const val RESULT_VALUE: Int = ${values.result_value}
    public const val RESULT_NOT_FOUND: Int = ${values.result_not_found}
    public const val RESULT_CREATED: Int = ${values.result_created}
    public const val RESULT_REPLACED: Int = ${values.result_replaced}
    public const val RESULT_DELETED: Int = ${values.result_deleted}
    public const val RESULT_NOT_DELETED: Int = ${values.result_not_deleted}
    public const val RESULT_CONNECTED: Int = ${values.result_connected}
    public const val RESULT_NOT_STORED: Int = ${values.result_not_stored}
    public const val DESCRIPTOR_DECODE_OK: Int = ${values.descriptor_decode_ok}
    public const val DEFAULT_EXPIRATION_NO_EXPIRY: Int = ${values.default_expiration_no_expiry}
    public const val DEFAULT_EXPIRATION_FIXED_TTL: Int = ${values.default_expiration_fixed_ttl}
    public const val DEFAULT_EVICTION_EVICTABLE: Int = ${values.default_eviction_evictable}
    public const val DEFAULT_EVICTION_PROTECTED: Int = ${values.default_eviction_protected}
    public const val OVERRIDE_DISALLOWED: Int = ${values.override_disallowed}
    public const val OVERRIDE_ALLOWED: Int = ${values.override_allowed}
    public const val SET_CONDITION_ANY: Int = ${values.set_condition_any}
    public const val SET_CONDITION_IF_ABSENT: Int = ${values.set_condition_if_absent}
    public const val SET_CONDITION_IF_PRESENT: Int = ${values.set_condition_if_present}
    public const val SET_INHERIT_EXPIRATION_BITS: Int = ${values.set_inherit_expiration}
    public const val SET_NO_EXPIRY_BITS: Int = ${values.set_no_expiry}
    public const val SET_EXPLICIT_TTL_BITS: Int = ${values.set_explicit_ttl}
    public const val SET_INHERIT_EVICTION_BITS: Int = ${values.set_inherit_eviction}
    public const val SET_EVICTABLE_BITS: Int = ${values.set_evictable}
    public const val SET_EVICTION_PROTECTED_BITS: Int = ${values.set_eviction_protected}
    public const val POLICY_NO_EXPIRY_BITS: Int = ${values.policy_no_expiry}
    public const val POLICY_FIXED_TTL_BITS: Int = ${values.policy_fixed_ttl}
    public const val POLICY_EXPIRATION_OVERRIDE_FLAG: Int = ${values.policy_expiration_override}
    public const val POLICY_EVICTION_PROTECTED_FLAG: Int = ${values.policy_eviction_protected}
    public const val POLICY_EVICTION_OVERRIDE_FLAG: Int = ${values.policy_eviction_override}
    public const val ITEM_ID_BYTES: Int = ${values.item_id_bytes}
    public const val NAMESPACE_NAME_MAX_BYTES: Int = ${values.namespace_name_max_bytes}
    public const val MAX_VALUE_BYTES: Int = ${values.max_value_bytes}
    public const val DEFAULT_ZSTANDARD_LEVEL: Int = ${values.default_zstandard_level}
    public const val DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES: Long = ${values.default_zstandard_minimum_input_bytes}L
    public const val DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES: Long = ${values.default_zstandard_minimum_savings_bytes}L
    public const val DEFAULT_CONNECT_TIMEOUT_MILLISECONDS: Long = ${values.default_connect_timeout_milliseconds}L
    public const val DEFAULT_REQUEST_TIMEOUT_MILLISECONDS: Long = ${values.default_request_timeout_milliseconds}L
}
`
}

/** Renders the native constants consumed by the Dart FFI adapter. */
export function render_dart_contract(contract: Client_Contract): string {
  const values = adapter_contract_values(contract)
  return `// Generated from the OpenKache Smithy contract. Do not edit.

/// Native values shared by the Dart adapter and the Rust client-core ABI.
const int smithyFfiAbiVersion = ${values.abi_version};
${values.operations
  .map(
    (entry) =>
      `const int smithyOperation${pascal_case(snake_case(entry.name))} = ${entry.value};`,
  )
  .join("\n")}
${render_dart_operation_metadata(values)}
const int smithyResultError = ${values.result_error};
const int smithyResultOk = ${values.result_ok};
const int smithyResultValue = ${values.result_value};
const int smithyResultNotFound = ${values.result_not_found};
const int smithyResultCreated = ${values.result_created};
const int smithyResultReplaced = ${values.result_replaced};
const int smithyResultDeleted = ${values.result_deleted};
const int smithyResultNotDeleted = ${values.result_not_deleted};
const int smithyResultConnected = ${values.result_connected};
const int smithyResultNotStored = ${values.result_not_stored};
const int smithyDescriptorDecodeOk = ${values.descriptor_decode_ok};
const int smithyDefaultExpirationNoExpiry = ${values.default_expiration_no_expiry};
const int smithyDefaultExpirationFixedTtl = ${values.default_expiration_fixed_ttl};
const int smithyDefaultEvictionEvictable = ${values.default_eviction_evictable};
const int smithyDefaultEvictionProtected = ${values.default_eviction_protected};
const int smithyOverrideDisallowed = ${values.override_disallowed};
const int smithyOverrideAllowed = ${values.override_allowed};
const int smithySetConditionAny = ${values.set_condition_any};
const int smithySetConditionIfAbsent = ${values.set_condition_if_absent};
const int smithySetConditionIfPresent = ${values.set_condition_if_present};
const int smithySetInheritExpirationBits = ${values.set_inherit_expiration};
const int smithySetNoExpiryBits = ${values.set_no_expiry};
const int smithySetExplicitTtlBits = ${values.set_explicit_ttl};
const int smithySetInheritEvictionBits = ${values.set_inherit_eviction};
const int smithySetEvictableBits = ${values.set_evictable};
const int smithySetEvictionProtectedBits = ${values.set_eviction_protected};
const int smithyPolicyNoExpiryBits = ${values.policy_no_expiry};
const int smithyPolicyFixedTtlBits = ${values.policy_fixed_ttl};
const int smithyPolicyExpirationOverrideFlag = ${values.policy_expiration_override};
const int smithyPolicyEvictionProtectedFlag = ${values.policy_eviction_protected};
const int smithyPolicyEvictionOverrideFlag = ${values.policy_eviction_override};
const int smithyItemIdBytes = ${values.item_id_bytes};
const int smithyNamespaceNameMaxBytes = ${values.namespace_name_max_bytes};
const int smithyMaxValueBytes = ${values.max_value_bytes};
const int smithyDefaultZstandardLevel = ${values.default_zstandard_level};
const int smithyDefaultZstandardMinimumInputBytes =
    ${values.default_zstandard_minimum_input_bytes};
const int smithyDefaultZstandardMinimumSavingsBytes =
    ${values.default_zstandard_minimum_savings_bytes};
const int smithyDefaultConnectTimeoutMilliseconds =
    ${values.default_connect_timeout_milliseconds};
const int smithyDefaultRequestTimeoutMilliseconds =
    ${values.default_request_timeout_milliseconds};
`
}

function csharp_wire_enum(name: string, entries: readonly Wire_Entry[]): string {
  const variants = entries
    .map((entry) => `        ${entry.name} = ${formatted_byte(entry.value)},`)
    .join("\n")
  return `    internal enum ${name} : byte
    {
${variants}
    }`
}

/** Renders protocol v1 C# definitions.
 *
 * @param contract - Validated language-neutral wire contract.
 * @returns Deterministic C# source with a trailing newline.
 */
export function render_csharp(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  const ffi = contract.ffi
  const descriptor_layout = ffi.namespace_descriptor_layout
  const descriptor_fields = ffi.namespace_descriptor_fields
  const version_bytes = encode_vu128(value.version)
  const envelope = contract.value_envelope
  const envelope_magic = bytes_from_hex(
    envelope.magic_and_version_hex,
    "value envelope magic",
  )
  const csharp_namespace_descriptor_fields = descriptor_fields.map(
    (field) =>
      `        internal ${field.csharp_type} ${field.csharp_name};`,
  ).join("\n")
  const csharp_descriptor_offsets = descriptor_fields
    .map(
      (field) =>
        `    internal const int FfiNamespaceDescriptor${pascal_case(field.name)}Offset = ${formatted_decimal(field.offset)};`,
    )
    .join("\n")
  const csharp_ffi_constants = [
    ["FfiOperation", ffi.operations],
    ["FfiResult", ffi.result_kinds],
    ["FfiConnection", ffi.connection_states],
    ["FfiSetCondition", ffi.set_conditions],
    ["FfiNamespaceDescriptorDecode", ffi.namespace_descriptor_decode_statuses],
    ["FfiNamespaceDefaultExpiration", ffi.namespace_default_expirations],
    ["FfiNamespaceDefaultEviction", ffi.namespace_default_evictions],
    ["FfiNamespaceOverride", ffi.namespace_override_policies],
  ]
    .flatMap(([prefix, entries]) =>
      (entries as readonly Ffi_Entry[]).map(
        (entry) =>
          `    internal const uint ${prefix}${pascal_case(snake_case(entry.name))} = ${formatted_decimal(entry.value)}u;`,
      ),
    )
    .join("\n")
  const csharp_api_enum_constants = contract.api.enums
    .flatMap((enum_) =>
      enum_.members.map(
        (member) =>
          `    internal const string Smithy${enum_.name}${member.name}Value = ${JSON.stringify(member.value)};`,
      ),
    )
    .join("\n")
  return `// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

// Generated from the OpenKache Smithy contract. Do not edit.

namespace OpenKache;

internal static partial class Protocol
{
    [System.Runtime.InteropServices.StructLayout(
        System.Runtime.InteropServices.LayoutKind.Sequential)]
    internal struct FfiNamespaceDescriptor
    {
${csharp_namespace_descriptor_fields}
    }

    internal const string ApplicationProtocol = ${JSON.stringify(contract.v1.alpn)};
    internal const int MaximumValueBytes = ${formatted_decimal(contract.max_value_bytes)};
    internal const int OpcodeBytes = ${formatted_decimal(contract.v1.opcode_bytes)};
    internal const int StatusBytes = ${formatted_decimal(contract.v1.status_bytes)};
    internal const int RequestFixedBytes = ${formatted_decimal(contract.v1.request_fixed_bytes)};
    internal const int ResponseFixedBytes = ${formatted_decimal(contract.v1.response_fixed_bytes)};
    internal const int MinimumVarUIntBytes = ${formatted_decimal(contract.v1.min_varuint_bytes)};
    internal const int MaximumVarUIntBytes = ${formatted_decimal(contract.v1.max_varuint_bytes)};
    internal const int NamespaceIdBytes = ${formatted_decimal(contract.v1.namespace_id_bytes)};
    internal const int NamespaceRevisionBytes = ${formatted_decimal(contract.v1.namespace_revision_bytes)};
    internal const int NamespaceNameLengthBytes = ${formatted_decimal(contract.v1.namespace_name_length_bytes)};
    internal const int NamespaceNameMaxBytes = ${formatted_decimal(contract.v1.namespace_name_max_bytes)};
    internal const int SetFlagsBytes = ${formatted_decimal(contract.v1.set_flags_bytes)};
    internal const byte SetConditionMask = ${formatted_byte(contract.v1.set_condition_mask)};
    internal const byte SetConditionAnyBits = ${formatted_byte(contract.v1.set_condition_any_bits)};
    internal const byte SetConditionReservedBits = ${formatted_byte(contract.v1.set_condition_reserved_bits)};
    internal const byte SetExpirationMask = ${formatted_byte(contract.v1.set_expiration_mask)};
    internal const byte SetInheritExpirationBits = ${formatted_byte(contract.v1.set_inherit_expiration_bits)};
    internal const byte SetNoExpiryBits = ${formatted_byte(contract.v1.set_no_expiry_bits)};
    internal const byte SetExplicitTtlBits = ${formatted_byte(contract.v1.set_ttl_flag)};
    internal const byte SetExpirationReservedBits = ${formatted_byte(contract.v1.set_expiration_reserved_bits)};
    internal const byte SetEvictionMask = ${formatted_byte(contract.v1.set_eviction_mask)};
    internal const byte SetInheritEvictionBits = ${formatted_byte(contract.v1.set_inherit_eviction_bits)};
    internal const byte SetEvictableBits = ${formatted_byte(contract.v1.set_evictable_bits)};
    internal const byte SetEvictionProtectedBits = ${formatted_byte(contract.v1.set_eviction_protected_bits)};
    internal const byte SetEvictionReservedBits = ${formatted_byte(contract.v1.set_eviction_reserved_bits)};
    internal const byte SetReservedMask = ${formatted_byte(contract.v1.set_reserved_mask)};
    internal const int OpenFlagsBytes = ${formatted_decimal(contract.v1.open_flags_bytes)};
    internal const byte OpenCreateIfMissing = ${formatted_byte(contract.v1.open_create_if_missing_flag)};
    internal const byte OpenReservedMask = ${formatted_byte(contract.v1.open_reserved_mask)};
    internal const int DeleteFlagsBytes = ${formatted_decimal(contract.v1.delete_flags_bytes)};
    internal const byte DeleteIfEmpty = ${formatted_byte(contract.v1.delete_if_empty_bits)};
    internal const byte DeleteModeMask = ${formatted_byte(contract.v1.delete_mode_mask)};
    internal const byte DeleteReservedMask = ${formatted_byte(contract.v1.delete_reserved_mask)};
    internal const int PolicyFlagsBytes = ${formatted_decimal(contract.v1.policy_flags_bytes)};
    internal const byte PolicyDefaultExpirationMask = ${formatted_byte(contract.v1.policy_default_expiration_mask)};
    internal const byte PolicyNoExpiry = ${formatted_byte(contract.v1.policy_no_expiry_bits)};
    internal const byte PolicyFixedTtl = ${formatted_byte(contract.v1.policy_fixed_ttl_bits)};
    internal const byte PolicyDefaultExpirationReservedBits = ${formatted_byte(contract.v1.policy_default_expiration_reserved_bits)};
    internal const byte PolicyExpirationOverride = ${formatted_byte(contract.v1.policy_expiration_override_flag)};
    internal const byte PolicyEvictionProtected = ${formatted_byte(contract.v1.policy_eviction_protected_flag)};
    internal const byte PolicyEvictionOverride = ${formatted_byte(contract.v1.policy_eviction_override_flag)};
    internal const byte PolicyReservedMask = ${formatted_byte(contract.v1.policy_reserved_mask)};
    internal const byte ErrorStatusMinimum = ${formatted_byte(contract.v1.error_status_minimum)};
    internal const int DefaultMaxInFlight = ${formatted_decimal(defaults.max_in_flight)};
    internal const long DefaultConnectTimeoutMilliseconds = ${formatted_decimal(defaults.connect_timeout_milliseconds)};
    internal const long DefaultRequestTimeoutMilliseconds = ${formatted_decimal(defaults.request_timeout_milliseconds)};
    internal const int DefaultRetryMaxAttempts = ${formatted_decimal(defaults.retry_max_attempts)};
    internal const int DefaultZstandardLevel = ${formatted_decimal(defaults.zstandard_level)};
    internal const int DefaultZstandardMinimumInputBytes = ${formatted_decimal(defaults.zstandard_minimum_input_bytes)};
    internal const int DefaultZstandardMinimumSavingsBytes = ${formatted_decimal(defaults.zstandard_minimum_savings_bytes)};
    internal const uint FfiAbiVersion = ${formatted_decimal(ffi.abi_version)}u;
${csharp_ffi_constants}
${csharp_api_enum_constants}
    internal const int FfiNamespaceDescriptorSizeBytes = ${formatted_decimal(descriptor_layout.size_bytes)};
${csharp_descriptor_offsets}

    internal const int ItemIdBytes = ${formatted_decimal(contract.item_id_bytes)};
    internal const byte SetIfAbsentBits = ${formatted_byte(contract.v1.set_if_absent_flag)};
    internal const byte SetIfPresentBits = ${formatted_byte(contract.v1.set_if_present_flag)};

    internal const uint ValueFormatVersion = ${formatted_decimal(value.version)}u;
    internal const int ValueFormatMaxVu128Bytes = ${formatted_decimal(value.max_vu128_bytes)};
    internal const int ValueFormatFormatByteBytes = ${formatted_decimal(value.format_byte_bytes)};
    internal const byte ValueFormatCompressionMask = ${formatted_byte(value.format_compression_mask)};
    internal const byte ValueFormatEncryptionShift = ${formatted_byte(value.format_encryption_shift)};
    internal const byte ValueFormatSerializationRaw = ${formatted_byte(value.serialization_raw)};
    internal const byte ValueFormatSerializationJson = ${formatted_byte(value.serialization_json)};
    internal const byte ValueFormatCompressionNone = ${formatted_byte(value.compression_none)};
    internal const byte ValueFormatCompressionZstandard = ${formatted_byte(value.compression_zstandard)};
    internal const byte ValueFormatEncryptionNone = ${formatted_byte(value.encryption_none)};
    internal const byte ValueFormatEncryptionCompact = ${formatted_byte(value.encryption_compact)};
    internal const byte ValueFormatEncryptionRobust = ${formatted_byte(value.encryption_robust)};
    internal const int ValueFormatCompactSyntheticIvBytes = ${formatted_decimal(value.compact_synthetic_iv_bytes)};
    internal const int ValueFormatRobustNonceBytes = ${formatted_decimal(value.robust_nonce_bytes)};
    internal const int ValueFormatRobustTagBytes = ${formatted_decimal(value.robust_tag_bytes)};
    internal const int ValueFormatDataProtectionKeyBytes = ${formatted_decimal(value.data_protection_key_bytes)};
    internal const string ValueFormatItemIdRootContext = ${JSON.stringify(value.item_id_root_context)};
    internal const string ValueFormatAadDomain = ${JSON.stringify(value.aad_domain)};
    internal const string ValueFormatValueRootContext = ${JSON.stringify(value.value_root_context)};
    internal const string ValueFormatCompactMacContext = ${JSON.stringify(value.compact_mac_context)};
    internal const string ValueFormatCompactEncryptionContext = ${JSON.stringify(value.compact_encryption_context)};
    internal const string ValueFormatRobustContext = ${JSON.stringify(value.robust_context)};
    internal const int ValueEnvelopeMaxEncodingBytes = ${formatted_decimal(envelope.max_encoding_bytes)};
    internal const int ValueEnvelopeMaxTypeNameBytes = ${formatted_decimal(envelope.max_type_name_bytes)};
    internal const string ValueEnvelopeJsonEncoding = ${JSON.stringify(envelope.json_encoding)};
    internal static ReadOnlySpan<byte> ValueFormatVersionBytes =>
        [${version_bytes.map(formatted_byte).join(", ")}];
    internal static ReadOnlySpan<byte> ValueEnvelopeMagicAndVersion =>
        [${envelope_magic.map(formatted_byte).join(", ")}];

${csharp_wire_enum("Opcode", contract.opcodes)}

${csharp_wire_enum("Status", contract.statuses)}
}
`
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

function go_api_name(identifier: string): string {
  return `Smithy${pascal_case(snake_case(identifier))}`
}

function go_exported_name(identifier: string): string {
  return pascal_case(snake_case(identifier))
    .replace(/Id$/, "ID")
    .replace(/^Ttl/, "TTL")
    .replace(/^Json$/, "JSON")
}

function go_ffi_name(identifier: string): string {
  const name = go_exported_name(identifier)
  return name === "Ok" ? "OK" : name
}

function go_api_value_name(enum_name: string, member_name: string): string {
  return `${go_api_name(enum_name)}${member_name}Value`
}

function go_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "[]byte"
      break
    case "boolean":
      rendered = "bool"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = go_api_name(type.name)
      break
    case "integer":
      rendered = "int32"
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
    case "unsigned_long":
      rendered = "uint64"
      break
  }
  return required ? rendered : `*${rendered}`
}

/** Renders Smithy operation types and a context-aware Go service interface. */
export function render_go_api(contract: Client_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const members = enum_.members
      .map(
        (member) =>
          `\t${go_api_name(enum_.name)}${member.name} ${go_api_name(enum_.name)} = ${go_api_value_name(enum_.name, member.name)}`,
      )
      .join("\n")
    return `// ${go_api_name(enum_.name)} is the Smithy ${enum_.name} enum.
type ${go_api_name(enum_.name)} string

const (
${members}
)`
  })
  const structures = contract.api.structures.map((structure) => {
    const members = structure.members
      .map((member) => {
        const field = go_exported_name(member.name)
        const optional = member.required ? "" : ",omitempty"
        return `\t${field} ${go_api_type(member.type, member.required)} \`json:"${snake_case(member.name)}${optional}"\``
      })
      .join("\n")
    const body = members.length === 0 ? "" : `\n${members}\n`
    return `// ${go_api_name(structure.name)} is the Smithy ${structure.name} structure.
type ${go_api_name(structure.name)} struct {${body}}`
  })
  const operations = contract.api.operations.map(
    (operation) =>
      `\t${operation.name}(context.Context, ${go_api_name(operation.input)}) (${go_api_name(operation.output)}, error)`,
  )
  return `// Code generated from the OpenKache Smithy contract. DO NOT EDIT.

package openkache

import "context"

${[...enums, ...structures].join("\n\n")}

// SmithyOpenKacheAPI describes the operations defined by the OpenKache Smithy service.
type SmithyOpenKacheAPI interface {
${operations.join("\n")}
}
`
}

/** Renders generated wire, ABI, and client-default constants for Go. */
export function render_go_contract(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  const descriptor_layout = contract.ffi.namespace_descriptor_layout
  const descriptor_fields = contract.ffi.namespace_descriptor_fields
  const go_namespace_descriptor_fields = descriptor_fields.map(
    (field) => `\t${field.go_name} ${field.go_type}`,
  ).join("\n")
  const go_descriptor_offsets = descriptor_fields
    .map(
      (field) =>
        `\tSmithyFFINamespaceDescriptor${field.go_name}Offset = ${field.offset}`,
    )
    .join("\n")
  return `// Code generated from the OpenKache Smithy contract. DO NOT EDIT.

package openkache

// SmithyFFINamespaceDescriptor is the C-compatible namespace descriptor
// returned by the shared native ABI decoder.
type SmithyFFINamespaceDescriptor struct {
${go_namespace_descriptor_fields}
}

const (
\t// SmithyProtocolALPN is the negotiated protocol identifier.
\tSmithyProtocolALPN = ${JSON.stringify(contract.v1.alpn)}
\t// SmithyItemIDBytes is the exact protocol item-ID width.
\tSmithyItemIDBytes = ${contract.item_id_bytes}
\t// SmithyMaxValueBytes is the protocol value and payload ceiling.
\tSmithyMaxValueBytes = ${contract.max_value_bytes}
\t// SmithyOpcodeBytes and SmithyStatusBytes are fixed opcode/status widths.
\tSmithyOpcodeBytes = ${contract.v1.opcode_bytes}
\tSmithyStatusBytes = ${contract.v1.status_bytes}
\tSmithyRequestFixedBytes = ${contract.v1.request_fixed_bytes}
\tSmithyResponseFixedBytes = ${contract.v1.response_fixed_bytes}
\tSmithyMinVaruintBytes = ${contract.v1.min_varuint_bytes}
\tSmithyMaxVaruintBytes = ${contract.v1.max_varuint_bytes}
\tSmithyNamespaceIDBytes = ${contract.v1.namespace_id_bytes}
\tSmithyNamespaceRevisionBytes = ${contract.v1.namespace_revision_bytes}
\tSmithyNamespaceNameLengthBytes = ${contract.v1.namespace_name_length_bytes}
\tSmithyNamespaceNameMaxBytes = ${contract.v1.namespace_name_max_bytes}
\tSmithySetFlagsBytes = ${contract.v1.set_flags_bytes}
\tSmithySetConditionMask = ${contract.v1.set_condition_mask}
\tSmithySetConditionAnyBits = ${contract.v1.set_condition_any_bits}
\tSmithySetIfAbsentBits = ${contract.v1.set_if_absent_flag}
\tSmithySetIfPresentBits = ${contract.v1.set_if_present_flag}
\tSmithySetConditionReservedBits = ${contract.v1.set_condition_reserved_bits}
\tSmithySetExpirationMask = ${contract.v1.set_expiration_mask}
\tSmithySetInheritExpirationBits = ${contract.v1.set_inherit_expiration_bits}
\tSmithySetNoExpiryBits = ${contract.v1.set_no_expiry_bits}
\tSmithySetExplicitTTLBits = ${contract.v1.set_ttl_flag}
\tSmithySetExpirationReservedBits = ${contract.v1.set_expiration_reserved_bits}
\tSmithySetEvictionMask = ${contract.v1.set_eviction_mask}
\tSmithySetInheritEvictionBits = ${contract.v1.set_inherit_eviction_bits}
\tSmithySetEvictableBits = ${contract.v1.set_evictable_bits}
\tSmithySetEvictionProtectedBits = ${contract.v1.set_eviction_protected_bits}
\tSmithySetEvictionReservedBits = ${contract.v1.set_eviction_reserved_bits}
\tSmithySetReservedMask = ${contract.v1.set_reserved_mask}
\tSmithyOpenFlagsBytes = ${contract.v1.open_flags_bytes}
\tSmithyOpenCreateIfMissing = ${contract.v1.open_create_if_missing_flag}
\tSmithyOpenReservedMask = ${contract.v1.open_reserved_mask}
\tSmithyDeleteFlagsBytes = ${contract.v1.delete_flags_bytes}
\tSmithyDeleteIfEmpty = ${contract.v1.delete_if_empty_bits}
\tSmithyDeleteModeMask = ${contract.v1.delete_mode_mask}
\tSmithyDeleteReservedMask = ${contract.v1.delete_reserved_mask}
\tSmithyPolicyFlagsBytes = ${contract.v1.policy_flags_bytes}
\tSmithyPolicyDefaultExpirationMask = ${contract.v1.policy_default_expiration_mask}
\tSmithyPolicyNoExpiry = ${contract.v1.policy_no_expiry_bits}
\tSmithyPolicyFixedTTL = ${contract.v1.policy_fixed_ttl_bits}
\tSmithyPolicyDefaultExpirationReservedBits = ${contract.v1.policy_default_expiration_reserved_bits}
\tSmithyPolicyExpirationOverride = ${contract.v1.policy_expiration_override_flag}
\tSmithyPolicyEvictionProtected = ${contract.v1.policy_eviction_protected_flag}
\tSmithyPolicyEvictionOverride = ${contract.v1.policy_eviction_override_flag}
\tSmithyPolicyReservedMask = ${contract.v1.policy_reserved_mask}
\tSmithyErrorStatusMinimum = ${contract.v1.error_status_minimum}
\t// SmithyDataProtectionKeyBytes is the shared key width.
\tSmithyDataProtectionKeyBytes = ${value.data_protection_key_bytes}
\t// SmithyValueEncryptionNone selects unprotected values.
\tSmithyValueEncryptionNone uint32 = ${value.encryption_none}
\t// SmithyValueEncryptionCompact selects deterministic AES-SIV protection.
\tSmithyValueEncryptionCompact uint32 = ${value.encryption_compact}
\t// SmithyValueEncryptionRobust selects randomized AES-GCM-SIV protection.
\tSmithyValueEncryptionRobust uint32 = ${value.encryption_robust}
)

// Smithy operation values carried by the native ABI.
const (
${contract.opcodes
  .map((entry) => `\tSmithyOpcode${entry.name} uint32 = ${entry.value}`)
  .join("\n")}
)

// Smithy native ABI values shared by language adapters.
const (
\t// SmithyFFIABIVersion is the native ABI version implemented by the core.
\tSmithyFFIABIVersion uint32 = ${contract.ffi.abi_version}
${contract.ffi.operations
  .map(
    (entry) =>
      `\t// SmithyFFIOperation${go_ffi_name(entry.name)} identifies the native operation ${entry.name}.
\tSmithyFFIOperation${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.result_kinds
  .map(
    (entry) =>
      `\t// SmithyFFIResult${go_ffi_name(entry.name)} is the native ABI result kind for ${entry.name}.
\tSmithyFFIResult${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.set_conditions
  .map(
    (entry) =>
      `\t// SmithyFFISetCondition${go_ffi_name(entry.name)} is the native ABI SET condition for ${entry.name}.
\tSmithyFFISetCondition${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.connection_states
  .map(
    (entry) =>
      `\t// SmithyFFIConnectionState${go_ffi_name(entry.name)} identifies a native connection state.
\tSmithyFFIConnectionState${go_ffi_name(entry.name)} uint32 = ${entry.value}
\t// SmithyFFIConnectionState${go_ffi_name(entry.name)}Name is its stable text name.
\tSmithyFFIConnectionState${go_ffi_name(entry.name)}Name = ${JSON.stringify(entry.text)}`,
  )
  .join("\n")}
${contract.ffi.namespace_descriptor_decode_statuses
  .map(
    (entry) =>
      `\t// SmithyFFINamespaceDescriptorDecode${go_ffi_name(entry.name)} is the namespace descriptor decode status ${entry.name}.
\tSmithyFFINamespaceDescriptorDecode${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.namespace_default_expirations
  .map(
    (entry) =>
      `\t// SmithyFFINamespaceDefaultExpiration${go_ffi_name(entry.name)} is the namespace default expiration value ${entry.name}.
\tSmithyFFINamespaceDefaultExpiration${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.namespace_default_evictions
  .map(
    (entry) =>
      `\t// SmithyFFINamespaceDefaultEviction${go_ffi_name(entry.name)} is the namespace default eviction value ${entry.name}.
\tSmithyFFINamespaceDefaultEviction${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.namespace_override_policies
  .map(
    (entry) =>
      `\t// SmithyFFINamespaceOverride${go_ffi_name(entry.name)} is the namespace override-policy value ${entry.name}.
\tSmithyFFINamespaceOverride${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
\t// SmithyFFINamespaceDescriptorSizeBytes and offsets describe the C-compatible descriptor ABI.
\tSmithyFFINamespaceDescriptorSizeBytes = ${descriptor_layout.size_bytes}
${go_descriptor_offsets}
)

// Shared client defaults extracted from the Smithy service contract.
const (
\t// SmithyDefaultMaxInFlight is the default number of request lanes.
\tSmithyDefaultMaxInFlight = ${defaults.max_in_flight}
\t// SmithyDefaultConnectTimeoutMilliseconds is the default connection timeout.
\tSmithyDefaultConnectTimeoutMilliseconds uint64 = ${defaults.connect_timeout_milliseconds}
\t// SmithyDefaultRequestTimeoutMilliseconds is the default complete request timeout.
\tSmithyDefaultRequestTimeoutMilliseconds uint64 = ${defaults.request_timeout_milliseconds}
\t// SmithyDefaultRetryMaxAttempts is the default total retry attempt count.
\tSmithyDefaultRetryMaxAttempts = ${defaults.retry_max_attempts}
\t// SmithyDefaultZstandardLevel is the default Zstandard level.
\tSmithyDefaultZstandardLevel int32 = ${defaults.zstandard_level}
\t// SmithyDefaultZstandardMinimumInputBytes is the compression input threshold.
\tSmithyDefaultZstandardMinimumInputBytes = ${defaults.zstandard_minimum_input_bytes}
\t// SmithyDefaultZstandardMinimumSavingsBytes is the compression savings threshold.
\tSmithyDefaultZstandardMinimumSavingsBytes = ${defaults.zstandard_minimum_savings_bytes}
\t// SmithyDefaultZstandardLevelMin is the minimum supported Zstandard level.
\tSmithyDefaultZstandardLevelMin int32 = ${defaults.zstandard_level_min}
\t// SmithyDefaultZstandardLevelMax is the maximum supported Zstandard level.
\tSmithyDefaultZstandardLevelMax int32 = ${defaults.zstandard_level_max}
\t// SmithyClientDefaultServerName is used when no TLS server name is supplied.
\tSmithyClientDefaultServerName = ${JSON.stringify(defaults.server_name)}
\t// SmithyClientCertificatePEMType is the PEM block type used for certificate chains.
\tSmithyClientCertificatePEMType = ${JSON.stringify(defaults.certificate_pem_type)}
\t// SmithyClientMinimumPositiveValue is the minimum accepted positive setting.
\tSmithyClientMinimumPositiveValue = ${defaults.minimum_positive_value}
)

// Smithy API enum string values extracted from the Smithy service contract.
const (
${contract.api.enums
  .flatMap((enum_) =>
    enum_.members.map(
      (member) =>
        `\t// ${go_api_value_name(enum_.name, member.name)} is the Smithy ${enum_.name} value for ${member.value}.
\t${go_api_value_name(enum_.name, member.name)} = ${JSON.stringify(member.value)}`,
    ),
  )
  .join("\n")}
)
`
}

function native_abi_go_field_name(function_name: string): string {
  const suffix = native_abi_dart_suffix(function_name)
  return suffix === "abi_version" ? "abi" : suffix
}

/** Renders the generated C preprocessor list consumed by the Go cgo loader. */
export function render_go_native_abi(contract: Client_Contract): string {
  const render_function_list = (
    macro_name: string,
    functions: readonly Native_Abi_Function[],
  ): string => {
    if (functions.length === 0) {
      return `#define ${macro_name}(X)`
    }
    const entries = functions
      .map((function_, index) => {
        const continuation = index === functions.length - 1 ? "" : " \\"
        return `    X(${native_abi_go_field_name(function_.name)}, ${native_abi_c_function_typedef_name(function_.name)}, "${function_.name}")${continuation}`
      })
      .join("\n")
    return `#define ${macro_name}(X) \\
${entries}`
  }
  const required_functions = contract.ffi.native_abi_functions.filter(
    (function_) => !function_.optional,
  )
  const optional_functions = contract.ffi.native_abi_functions.filter(
    (function_) => function_.optional,
  )
  return `/* Generated from the OpenKache Smithy client ABI contract. Do not edit. */
#ifndef OPENKACHE_SMITHY_NATIVE_ABI_H
#define OPENKACHE_SMITHY_NATIVE_ABI_H

#include <openkache/client_abi.h>

/*
 * Expands to (field name, function-pointer type, exported symbol) triples.
 * The Go cgo loader uses this list for both its native-library state and
 * symbol registration, so adding an ABI function to Smithy cannot silently
 * leave the loader stale.
 */
${render_function_list("OPENKACHE_SMITHY_NATIVE_FUNCTIONS", contract.ffi.native_abi_functions)}

/*
 * Required symbols must be present in every ABI implementation. Optional
 * symbols are exposed separately so loaders can preserve older-library
 * compatibility without maintaining a second hand-written list.
 */
${render_function_list("OPENKACHE_SMITHY_NATIVE_REQUIRED_FUNCTIONS", required_functions)}
${render_function_list("OPENKACHE_SMITHY_NATIVE_OPTIONAL_FUNCTIONS", optional_functions)}

#endif /* OPENKACHE_SMITHY_NATIVE_ABI_H */
`
}

function format_go_source(source: string): string {
  const result = Bun.spawnSync({
    cmd: ["gofmt"],
    stdin: Buffer.from(source),
    stdout: "pipe",
    stderr: "pipe",
  })
  if (result.exitCode !== 0) {
    const diagnostics = result.stderr.toString().trim()
    throw new Error(
      diagnostics.length === 0
        ? "gofmt failed while formatting generated Go source"
        : `gofmt failed while formatting generated Go source:\n${diagnostics}`,
    )
  }
  return result.stdout.toString()
}

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
  const ffi_connection_states = contract.ffi.connection_states
    .map(
      (entry) =>
        `SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()} = ${entry.value}
SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()}_NAME = ${JSON.stringify(entry.text)}`,
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
${ffi_connection_states}
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

function native_abi_python_structure_name(structure_name: string): string {
  if (structure_name === "FfiNamespaceDescriptor") {
    return "SmithyFFINamespaceDescriptor"
  }
  return `SmithyNative${structure_name.replace(/^Ffi/, "")}`
}

function native_abi_python_type(
  type: Native_Abi_Type,
  structure_name?: string,
): string {
  switch (type) {
    case "client_pointer":
      return "_CLIENT_POINTER"
    case "result_pointer":
      return "_RESULT_POINTER"
    case "u8_pointer":
      return "_U8_POINTER"
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Python native struct pointer has no structure name")
      }
      return `_ctypes.POINTER(${native_abi_python_structure_name(structure_name)})`
    case "size":
      return "_ctypes.c_size_t"
    case "uint8":
      return "_ctypes.c_uint8"
    case "int32":
      return "_ctypes.c_int32"
    case "uint32":
      return "_ctypes.c_uint32"
    case "uint64":
      return "_ctypes.c_uint64"
    case "void":
      return "None"
  }
}

function native_abi_python_attribute(function_name: string): string {
  const suffix = native_abi_dart_suffix(function_name)
  switch (suffix) {
    case "connect":
      return "connect_legacy"
    case "connect_ex":
      return "connect"
    case "result_data_length":
      return "result_length"
    default:
      return suffix
  }
}

/** Renders ctypes classes and signatures from the Smithy native ABI contract. */
export function render_python_native_abi(contract: Client_Contract): string {
  const structures = contract.ffi.native_abi_structures
    .map((structure) => {
      const fields = structure.fields
        .map(
          (field) =>
            `        ("${snake_case(field.name)}", ${native_abi_python_type(field.type, field.structure_name)}),`,
        )
        .join("\n")
      return `class ${native_abi_python_structure_name(structure.name)}(_ctypes.Structure):
    """C-compatible ${structure.name} layout generated from Smithy."""

    _fields_ = [
${fields}
    ]`
    })
    .join("\n\n")
  const functions = contract.ffi.native_abi_functions
    .map((function_) => {
      const arguments_ = function_.parameters.length === 0
        ? "()"
        : `(
${function_.parameters
  .map(
    (parameter) =>
      `        ${native_abi_python_type(parameter.type, parameter.structure_name)},`,
  )
  .join("\n")}
    )`
      return `    "${native_abi_python_attribute(function_.name)}": (
        "${function_.name}",
        ${arguments_},
        ${native_abi_python_type(function_.return_type)},
    ),`
    })
    .join("\n")
  return `# Generated from the OpenKache Smithy client ABI contract. Do not edit.

import ctypes as _ctypes

from .smithy_contract import SmithyFFINamespaceDescriptor

_CLIENT_POINTER = _ctypes.c_void_p
_RESULT_POINTER = _ctypes.c_void_p
_U8 = _ctypes.c_uint8
_U8_POINTER = _ctypes.POINTER(_U8)

${structures}

SMITHY_NATIVE_FUNCTIONS = {
${functions}
}
`
}

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

function native_abi_swift_structure_name(structure_name: string): string {
  if (structure_name === "FfiNamespaceDescriptor") {
    return "Smithy_Native_Namespace_Descriptor"
  }
  return `Smithy_Native_${typescript_name(structure_name.replace(/^Ffi/, ""))}`
}

function native_abi_swift_type(
  type: Native_Abi_Type,
  mutable: boolean,
  structure_name?: string,
): string {
  const pointer = mutable ? "UnsafeMutablePointer" : "UnsafePointer"
  switch (type) {
    case "client_pointer":
    case "result_pointer":
      return "OpaquePointer?"
    case "u8_pointer":
      return `${pointer}<UInt8>?`
    case "struct_pointer":
      if (structure_name === undefined) {
        throw new Error("Swift native struct pointer has no structure name")
      }
      return `${pointer}<${native_abi_swift_structure_name(structure_name)}>?`
    case "size":
      return "Int"
    case "uint8":
      return "UInt8"
    case "int32":
      return "Int32"
    case "uint32":
      return "UInt32"
    case "uint64":
      return "UInt64"
    case "void":
      return "Void"
  }
}

function native_abi_swift_function_name(function_name: string): string {
  const suffix = native_abi_dart_suffix(function_name)
  const names: Readonly<Record<string, string>> = {
    abi_version: "nativeAbiVersion",
    connect: "nativeConnectLegacy",
    connect_ex: "nativeConnect",
    connect_with_options: "nativeConnectWithOptions",
    execute: "nativeExecute",
    execute_raw: "nativeExecuteRaw",
    execute_with_options: "nativeExecuteWithOptions",
    execute_raw_with_options: "nativeExecuteRawWithOptions",
    execute_scoped: "nativeExecuteScoped",
    namespace_open: "nativeNamespaceOpen",
    namespace_update_policy: "nativeNamespaceUpdatePolicy",
    namespace_delete: "nativeNamespaceDelete",
    namespace_descriptor_decode: "nativeNamespaceDescriptorDecode",
    connection_state: "nativeConnectionState",
    result_kind: "nativeResultKind",
    result_data: "nativeResultData",
    result_data_length: "nativeResultDataLength",
    result_take_client: "nativeTakeClient",
    result_free: "nativeFreeResult",
    client_free: "nativeFreeClient",
  }
  const function_name_value = names[suffix]
  if (function_name_value !== undefined) return function_name_value
  return `native${pascal_case(suffix)}`
}

/** Renders Swift FFI declarations from the Smithy native ABI contract. */
export function render_swift_native_abi(contract: Client_Contract): string {
  const structures = contract.ffi.native_abi_structures.map((structure) => {
    const fields = structure.fields
      .map(
        (field) =>
          `  var ${swift_property_name(field.name)}: ${native_abi_swift_type(field.type, field.mutable, field.structure_name)} = ${native_abi_swift_default(field.type)}`,
      )
      .join("\n")
    return `/// C-compatible ${structure.name} layout generated from Smithy.
internal struct ${native_abi_swift_structure_name(structure.name)} {
${fields}
}`
  })
  const functions = contract.ffi.native_abi_functions
    .map((function_) => {
      const name = native_abi_swift_function_name(function_.name)
      const parameters = function_.parameters
        .map(
          (parameter) =>
            `  _ ${swift_property_name(parameter.name)}: ${native_abi_swift_type(parameter.type, parameter.mutable, parameter.structure_name)}`,
        )
        .join(",\n")
      const return_type = native_abi_swift_type(
        function_.return_type,
        function_.return_type === "u8_pointer" ? false : true,
      )
      const declaration = function_.parameters.length === 0
        ? `internal func ${name}()`
        : `internal func ${name}(
${parameters}
)`
      return `@_silgen_name("${function_.name}")
${declaration}${return_type === "Void" ? "" : ` -> ${return_type}`}`
    })
    .join("\n\n")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.

import Foundation

typealias Smithy_Native_Client_Pointer = OpaquePointer
typealias Smithy_Native_Result_Pointer = OpaquePointer

${structures.join("\n\n")}

${functions}
`
}

function native_abi_swift_default(type: Native_Abi_Type): string {
  switch (type) {
    case "client_pointer":
    case "result_pointer":
    case "u8_pointer":
    case "struct_pointer":
      return "nil"
    case "size":
    case "int32":
    case "uint8":
    case "uint32":
    case "uint64":
      return "0"
    case "void":
      throw new Error("Swift native structure field cannot be void")
  }
}

/** Renders the cross-language value-format wire and cryptographic contract for TypeScript.
 *
 * @param contract - Validated language-neutral wire and value-format contract.
 * @returns Deterministic TypeScript source with a trailing newline.
 */
export function render_typescript_value_format(contract: Client_Contract): string {
  const value = contract.value_format
  const version_bytes = encode_vu128(value.version)
  return `// Generated from the OpenKache Smithy contract. Do not edit.

/** Current client-owned value-format version. */
export const SMITHY_VALUE_FORMAT_VERSION = ${value.version}
/** Canonical VU128 bytes for the current value-format version. */
export const SMITHY_VALUE_FORMAT_VERSION_BYTES = [${version_bytes.join(", ")}] as const
/** Maximum bytes accepted for a canonical value-format VU128. */
export const SMITHY_VALUE_FORMAT_MAX_VU128_BYTES = ${value.max_vu128_bytes}
/** Bytes occupied by the value-format transform byte. */
export const SMITHY_VALUE_FORMAT_FORMAT_BYTE_BYTES = ${value.format_byte_bytes}
/** Low-nibble mask for the value-format compression identifier. */
export const SMITHY_VALUE_FORMAT_COMPRESSION_MASK = ${value.format_compression_mask}
/** Number of bits to shift the value-format encryption identifier. */
export const SMITHY_VALUE_FORMAT_ENCRYPTION_SHIFT = ${value.format_encryption_shift}
/** Raw serialized-value identifier. */
export const SMITHY_VALUE_SERIALIZATION_RAW = ${value.serialization_raw}
/** Canonical JSON serialized-value identifier. */
export const SMITHY_VALUE_SERIALIZATION_JSON = ${value.serialization_json}
/** Uncompressed value-format identifier. */
export const SMITHY_VALUE_COMPRESSION_NONE = ${value.compression_none}
/** Zstandard value-format identifier. */
export const SMITHY_VALUE_COMPRESSION_ZSTANDARD = ${value.compression_zstandard}
/** Unencrypted value-format identifier. */
export const SMITHY_VALUE_ENCRYPTION_NONE = ${value.encryption_none}
/** Compact AES-SIV value-format identifier. */
export const SMITHY_VALUE_ENCRYPTION_COMPACT = ${value.encryption_compact}
/** Robust AES-GCM-SIV value-format identifier. */
export const SMITHY_VALUE_ENCRYPTION_ROBUST = ${value.encryption_robust}
/** Compact AES-SIV synthetic-IV and authentication-tag size. */
export const SMITHY_VALUE_COMPACT_SYNTHETIC_IV_BYTES = ${value.compact_synthetic_iv_bytes}
/** Robust AES-GCM-SIV nonce size. */
export const SMITHY_VALUE_ROBUST_NONCE_BYTES = ${value.robust_nonce_bytes}
/** Robust AES-GCM-SIV authentication-tag size. */
export const SMITHY_VALUE_ROBUST_TAG_BYTES = ${value.robust_tag_bytes}
/** Application-managed data-protection key size. */
export const SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES = ${value.data_protection_key_bytes}
/** BLAKE3 protected-item-ID root derivation context. */
export const SMITHY_VALUE_ITEM_ID_ROOT_CONTEXT = ${JSON.stringify(value.item_id_root_context)}
/** Associated-data domain separator. */
export const SMITHY_VALUE_AAD_DOMAIN = ${JSON.stringify(value.aad_domain)}
/** BLAKE3 value-root derivation context. */
export const SMITHY_VALUE_VALUE_ROOT_CONTEXT = ${JSON.stringify(value.value_root_context)}
/** BLAKE3 Compact AES-SIV MAC-key derivation context. */
export const SMITHY_VALUE_COMPACT_MAC_CONTEXT = ${JSON.stringify(value.compact_mac_context)}
/** BLAKE3 Compact AES-SIV encryption-key derivation context. */
export const SMITHY_VALUE_COMPACT_ENCRYPTION_CONTEXT = ${JSON.stringify(value.compact_encryption_context)}
/** BLAKE3 Robust AES-GCM-SIV key derivation context. */
export const SMITHY_VALUE_ROBUST_CONTEXT = ${JSON.stringify(value.robust_context)}
`
}

/** Renders constants for the legacy TypeScript metadata envelope.
 *
 * @param contract - Validated language-neutral value-envelope contract.
 * @returns Deterministic TypeScript source with a trailing newline.
 */
export function render_typescript_value_envelope(contract: Client_Contract): string {
  const envelope = contract.value_envelope
  const magic = bytes_from_hex(envelope.magic_and_version_hex, "value envelope magic")
  return `// Generated from the OpenKache Smithy contract. Do not edit.

/** Legacy metadata-envelope magic and version. */
export const SMITHY_VALUE_ENVELOPE_MAGIC_AND_VERSION = [${magic.join(", ")}] as const
/** Maximum UTF-8 byte length of a legacy metadata-envelope encoding identifier. */
export const SMITHY_VALUE_ENVELOPE_MAX_ENCODING_BYTES = ${envelope.max_encoding_bytes}
/** Maximum UTF-8 byte length of a legacy metadata-envelope logical type name. */
export const SMITHY_VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES = ${envelope.max_type_name_bytes}
/** Built-in canonical JSON codec identifier used by the legacy envelope adapter. */
export const SMITHY_VALUE_ENVELOPE_JSON_ENCODING = ${JSON.stringify(envelope.json_encoding)}
`
}

function csharp_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "byte[]"
      break
    case "boolean":
      rendered = "bool"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = type.name
      break
    case "integer":
      rendered = "int"
      break
    case "long":
      rendered = "long"
      break
    case "structure":
      if (type.name === undefined) throw new Error("structure API type has no name")
      rendered = type.name
      break
    case "string":
      rendered = "string"
      break
    case "unsigned_long":
      rendered = "ulong"
      break
  }
  return required ? rendered : `${rendered}?`
}

/** Renders Smithy operation types and an API interface for C#.
 *
 * @param contract - Validated language-neutral wire and API contract.
 * @returns Deterministic C# source with a trailing newline.
 */
export function render_csharp_api(contract: Client_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const members = enum_.members
      .map((member) => `    /// <summary>Smithy ${member.value} value.</summary>
    ${member.name},`)
      .join("\n")
    return `/// <summary>Values defined by the Smithy ${enum_.name} shape.</summary>
public enum ${enum_.name}
{
${members}
}`
  })
  const structures = contract.api.structures.map((structure) => {
    if (structure.members.length === 0) {
      return `/// <summary>Smithy ${structure.name} structure.</summary>
public sealed record ${structure.name};`
    }
    const members = structure.members.map((member) => {
      const required = member.required ? "required " : ""
      return `    /// <summary>Smithy ${member.name} member.</summary>
    public ${required}${csharp_api_type(member.type, member.required)} ${pascal_case(snake_case(member.name))} { get; init; }`
    })
    return `/// <summary>Smithy ${structure.name} structure.</summary>
public sealed record ${structure.name}
{
${members.join("\n")}
}`
  })
  const operations = contract.api.operations.map(
    (operation) =>
      `    /// <summary>Invokes the Smithy ${operation.name} operation.</summary>
    ValueTask<${operation.output}> ${operation.name}Async(${operation.input} input, CancellationToken cancellationToken = default);`,
  )
  return `// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

// Generated from the OpenKache Smithy contract. Do not edit.

#nullable enable

namespace OpenKache.Smithy;

${[...enums, ...structures].join("\n\n")}

/// <summary>Operations defined by the OpenKache Smithy service.</summary>
public interface IOpenKacheApi
{
${operations.join("\n")}
}
`
}

function native_abi_csharp_structure_name(structure_name: string): string {
  if (structure_name === "FfiNamespaceDescriptor") {
    return "Protocol.FfiNamespaceDescriptor"
  }
  if (structure_name === "FfiConnectOptions") {
    return "ConnectOptions"
  }
  return `Native${pascal_case(snake_case(structure_name.replace(/^Ffi/, "")))}`
}

function native_abi_csharp_scalar_type(type: Native_Abi_Type): string {
  switch (type) {
    case "client_pointer":
    case "result_pointer":
    case "u8_pointer":
    case "struct_pointer":
      return "IntPtr"
    case "size":
      return "nuint"
    case "uint8":
      return "byte"
    case "int32":
      return "int"
    case "uint32":
      return "uint"
    case "uint64":
      return "ulong"
    case "void":
      return "void"
  }
}

function native_abi_csharp_parameter_type(
  parameter: Native_Abi_Parameter,
): string {
  if (parameter.type !== "struct_pointer") {
    return native_abi_csharp_scalar_type(parameter.type)
  }
  const modifier = parameter.mutable ? "out " : "ref "
  return `${modifier}${native_abi_csharp_structure_name(parameter.structure_name!)}`
}

function native_abi_csharp_identifier(identifier: string): string {
  const camel = lower_camel_case(identifier)
  return camel.length === 0
    ? camel
    : `${camel[0]?.toUpperCase()}${camel.slice(1)}`
}

/** Renders .NET P/Invoke declarations from the Smithy native ABI contract. */
export function render_csharp_native_abi(contract: Client_Contract): string {
  required_native_structure(contract, "FfiConnectOptions")
  const native_structures = contract.ffi.native_abi_structures
    .filter((structure) => structure.name !== "FfiNamespaceDescriptor")
    .map((structure) => {
      const fields = structure.fields
        .map(
          (field) =>
            `        internal ${native_abi_csharp_scalar_type(field.type)} ${native_abi_csharp_identifier(field.name)};`,
        )
        .join("\n")
      return `    [StructLayout(LayoutKind.Sequential)]
    internal struct ${native_abi_csharp_structure_name(structure.name)}
    {
${fields}
    }`
    })
    .join("\n\n")
  const functions = contract.ffi.native_abi_functions
    .map((function_) => {
      const parameters = function_.parameters.length === 0
        ? ""
        : function_.parameters
          .map(
            (parameter) =>
              `        ${native_abi_csharp_parameter_type(parameter)} ${native_abi_csharp_identifier(parameter.name)}`,
          )
          .join(",\n")
      return `[DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern ${native_abi_csharp_scalar_type(function_.return_type)} ${function_.name}(
${parameters}
    );`
    })
    .join("\n\n")
  return `// Generated from the OpenKache Smithy client ABI contract. Do not edit.
using System;
using System.Runtime.InteropServices;

namespace OpenKache;

internal static partial class NativeMethods
{
${native_structures}

${functions}
}
`
}

function rust_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "Vec<u8>"
      break
    case "boolean":
      rendered = "bool"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = type.name
      break
    case "integer":
      rendered = "i32"
      break
    case "long":
      rendered = "i64"
      break
    case "structure":
      if (type.name === undefined) throw new Error("structure API type has no name")
      rendered = type.name
      break
    case "string":
      rendered = "String"
      break
    case "unsigned_long":
      rendered = "u64"
      break
  }
  return required ? rendered : `Option<${rendered}>`
}

/** Renders Smithy operation types and an API trait for Rust.
 *
 * @param contract - Validated language-neutral wire and API contract.
 * @returns Deterministic Rust source with a trailing newline.
 */
export function render_rust_api(contract: Client_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const members = enum_.members
      .map((member) => `    /// Smithy ${member.value} value.
    ${member.name},`)
      .join("\n")
    return `/// Values defined by the Smithy ${enum_.name} shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ${enum_.name} {
${members}
}`
  })
  const structures = contract.api.structures.map((structure) => {
    if (structure.members.length === 0) {
      return `/// Smithy ${structure.name} structure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ${structure.name};`
    }
    const members = structure.members.map(
      (member) =>
        `    /// Smithy ${member.name} member.
    pub ${snake_case(member.name)}: ${rust_api_type(member.type, member.required)},`,
    )
    return `/// Smithy ${structure.name} structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ${structure.name} {
${members.join("\n")}
}`
  })
  const operations = contract.api.operations.map(
    (operation) =>
      `    /// Invokes the Smithy ${operation.name} operation.
    fn ${snake_case(operation.name)}(
        &self,
        input: ${operation.input},
    ) -> impl core::future::Future<
        Output = core::result::Result<${operation.output}, Self::Error>,
    >;`,
  )
  return `// Generated from the OpenKache Smithy contract. Do not edit.

${[...enums, ...structures].join("\n\n")}

/// Operations defined by the OpenKache Smithy service.
///
/// The trait does not require Send futures because the Rust client exposes
/// both Tokio/Quinn and runtime-local Compio implementations. Callers that
/// need cross-thread scheduling can add the bound to the concrete client.
pub trait OpenKacheApi {
    /// Error returned by an operation.
    type Error;

${operations.join("\n\n")}
}
`
}

function smithy_ast(client_model: boolean): unknown {
  const cwd = client_model ? CLIENTS_DIRECTORY : PROTOCOL_DIRECTORY
  const models = client_model
    ? [join("..", "protocol", MODEL_DIRECTORY), MODEL_DIRECTORY]
    : [MODEL_DIRECTORY]
  const smithy_executable = resolve_smithy_executable()
  const smithy_command =
    SMITHY_USE_SHELL && process.platform !== "win32"
      ? ["sh", smithy_executable, "ast", ...models]
      : [smithy_executable, "ast", ...models]
  const result = Bun.spawnSync(smithy_command, {
    cwd,
    stderr: "pipe",
    stdout: "pipe",
  })
  if (result.exitCode !== 0) {
    const diagnostics = result.stderr.toString().trim()
    throw new Error(
      diagnostics.length === 0
        ? "`smithy ast` exited without diagnostics"
        : `smithy AST generation failed:\n${diagnostics}`,
    )
  }
  try {
    return JSON.parse(result.stdout.toString()) as unknown
  } catch (error) {
    throw new Error(`smithy emitted invalid JSON: ${String(error)}`)
  }
}

type Generation_Target =
  | "all"
  | "c-contract"
  | "dart"
  | "dotnet"
  | "go"
  | "java"
  | "kotlin"
  | "python"
  | "rust-api"
  | "rust-client"
  | "rust-wire"
  | "swift"
  | "typescript"

function generation_target(value: string | undefined): Generation_Target {
  switch (value) {
    case undefined:
      return "all"
    case "all":
      return "all"
    case "c-contract":
      return "c-contract"
    case "dart":
      return "dart"
    case "dotnet":
      return "dotnet"
    case "go":
      return "go"
    case "java":
      return "java"
    case "kotlin":
      return "kotlin"
    case "python":
      return "python"
    case "rust-api":
      return "rust-api"
    case "rust-client":
      return "rust-client"
    case "rust-wire":
      return "rust-wire"
    case "swift":
      return "swift"
    case "typescript":
      return "typescript"
    default:
      throw new Error(`unsupported OPENKACHE_GENERATION_TARGET ${JSON.stringify(value)}`)
  }
}

function expected_wire_outputs(
  contract: Wire_Contract,
  target: "rust-wire",
): Readonly<Record<string, string>> {
  if (target !== "rust-wire") {
    throw new Error(`unsupported wire generation target ${target}`)
  }
  return {
    [GENERATED_OUTPUTS.rust_wire]: render_protocol_rust_wire(contract),
  }
}

function expected_outputs(
  contract: Client_Contract,
  target: Generation_Target,
): Readonly<Record<string, string>> {
  switch (target) {
    case "all":
      return {
        [GENERATED_OUTPUTS.csharp_api]: render_csharp_api(contract),
        [GENERATED_OUTPUTS.csharp_wire]: render_csharp(contract),
        [GENERATED_OUTPUTS.csharp_native_abi]: render_csharp_native_abi(contract),
        [GENERATED_OUTPUTS.rust_client]: render_rust_client(contract),
        [GENERATED_OUTPUTS.rust_api]: render_rust_api(contract),
        [GENERATED_OUTPUTS.rust_wire]: render_protocol_rust_wire(contract),
        [GENERATED_OUTPUTS.typescript_api]: render_typescript_api(contract),
        [GENERATED_OUTPUTS.typescript_value_format]:
          render_typescript_value_format(contract),
        [GENERATED_OUTPUTS.typescript_value_envelope]:
          render_typescript_value_envelope(contract),
        [GENERATED_OUTPUTS.python_api]: render_python_api(contract),
        [GENERATED_OUTPUTS.python_contract]: render_python_contract(contract),
        [GENERATED_OUTPUTS.python_native_abi]: render_python_native_abi(contract),
        [GENERATED_OUTPUTS.swift_api]: render_swift_api(contract),
        [GENERATED_OUTPUTS.swift_native_abi]: render_swift_native_abi(contract),
        [GENERATED_OUTPUTS.c_contract]: render_c_contract(contract),
        [GENERATED_OUTPUTS.go_api]: format_go_source(render_go_api(contract)),
        [GENERATED_OUTPUTS.go_contract]: format_go_source(render_go_contract(contract)),
        [GENERATED_OUTPUTS.go_native_abi]: render_go_native_abi(contract),
        ...render_java_api(contract),
        [GENERATED_OUTPUTS.java_contract]: render_java_contract(contract),
        [GENERATED_OUTPUTS.java_native_api]: render_java_native_api(contract),
        [GENERATED_OUTPUTS.java_native_descriptor]: render_java_native_descriptor(contract),
        [GENERATED_OUTPUTS.java_native_connect_options]:
          render_java_native_connect_options(contract),
        [GENERATED_OUTPUTS.java_operations]: render_java_operations(contract),
        [GENERATED_OUTPUTS.kotlin_api]: render_kotlin_api(contract),
        [GENERATED_OUTPUTS.kotlin_contract]: render_kotlin_contract(contract),
        [GENERATED_OUTPUTS.kotlin_native_api]: render_kotlin_native_api(contract),
        [GENERATED_OUTPUTS.kotlin_operations]: render_kotlin_operations(contract),
        [GENERATED_OUTPUTS.dart_api]: render_dart_api(contract),
        [GENERATED_OUTPUTS.dart_contract]: render_dart_contract(contract),
        [GENERATED_OUTPUTS.dart_native_api]: render_dart_native_api(contract),
        [GENERATED_OUTPUTS.dart_operations]: render_dart_operations(contract),
      }
    case "c-contract":
      return {
        [GENERATED_OUTPUTS.c_contract]: render_c_contract(contract),
      }
    case "dart":
      return {
        [GENERATED_OUTPUTS.dart_api]: render_dart_api(contract),
        [GENERATED_OUTPUTS.dart_contract]: render_dart_contract(contract),
        [GENERATED_OUTPUTS.dart_native_api]: render_dart_native_api(contract),
        [GENERATED_OUTPUTS.dart_operations]: render_dart_operations(contract),
      }
    case "dotnet":
      return {
        [GENERATED_OUTPUTS.csharp_api]: render_csharp_api(contract),
        [GENERATED_OUTPUTS.csharp_wire]: render_csharp(contract),
        [GENERATED_OUTPUTS.csharp_native_abi]: render_csharp_native_abi(contract),
      }
    case "go":
      return {
        [GENERATED_OUTPUTS.go_api]: format_go_source(render_go_api(contract)),
        [GENERATED_OUTPUTS.go_contract]: format_go_source(render_go_contract(contract)),
        [GENERATED_OUTPUTS.go_native_abi]: render_go_native_abi(contract),
      }
    case "java":
      return {
        ...render_java_api(contract),
        [GENERATED_OUTPUTS.java_contract]: render_java_contract(contract),
        [GENERATED_OUTPUTS.java_native_api]: render_java_native_api(contract),
        [GENERATED_OUTPUTS.java_native_descriptor]: render_java_native_descriptor(contract),
        [GENERATED_OUTPUTS.java_native_connect_options]:
          render_java_native_connect_options(contract),
        [GENERATED_OUTPUTS.java_operations]: render_java_operations(contract),
      }
    case "kotlin":
      return {
        [GENERATED_OUTPUTS.kotlin_api]: render_kotlin_api(contract),
        [GENERATED_OUTPUTS.kotlin_contract]: render_kotlin_contract(contract),
        [GENERATED_OUTPUTS.kotlin_native_api]: render_kotlin_native_api(contract),
        [GENERATED_OUTPUTS.kotlin_operations]: render_kotlin_operations(contract),
      }
    case "rust-api":
      return {
        [GENERATED_OUTPUTS.rust_api]: render_rust_api(contract),
      }
    case "rust-client":
      return {
        [GENERATED_OUTPUTS.rust_client]: render_rust_client(contract),
      }
    case "rust-wire":
      return {
        [GENERATED_OUTPUTS.rust_wire]: render_protocol_rust_wire(contract),
      }
    case "typescript":
      return {
        [GENERATED_OUTPUTS.typescript_api]: render_typescript_api(contract),
        [GENERATED_OUTPUTS.typescript_value_format]:
          render_typescript_value_format(contract),
        [GENERATED_OUTPUTS.typescript_value_envelope]:
          render_typescript_value_envelope(contract),
      }
    case "python":
      return {
        [GENERATED_OUTPUTS.python_api]: render_python_api(contract),
        [GENERATED_OUTPUTS.python_contract]: render_python_contract(contract),
        [GENERATED_OUTPUTS.python_native_abi]: render_python_native_abi(contract),
      }
    case "swift":
      return {
        [GENERATED_OUTPUTS.swift_api]: render_swift_api(contract),
        [GENERATED_OUTPUTS.swift_native_abi]: render_swift_native_abi(contract),
      }
  }
}

/** Directory whose files are fully owned by one generated target. */
export interface Generated_Output_Scope {
  readonly directory: string
  readonly extensions?: readonly string[]
}

function generated_scope_files(scope: Generated_Output_Scope): readonly string[] {
  const files: string[] = []
  const extensions = scope.extensions === undefined ? undefined : new Set(scope.extensions)
  const visit = (directory: string): void => {
    let entries
    try {
      entries = readdirSync(directory, { withFileTypes: true })
    } catch {
      return
    }
    for (const entry of entries) {
      const path = join(directory, entry.name)
      if (entry.isDirectory()) {
        visit(path)
      } else if (
        entry.isFile() &&
        (extensions === undefined || extensions.has(entry.name.slice(entry.name.lastIndexOf("."))))
      ) {
        files.push(path)
      }
    }
  }
  visit(scope.directory)
  return files.sort()
}

function generated_scope_obsolete_files(
  outputs: Readonly<Record<string, string>>,
  scopes: readonly Generated_Output_Scope[],
): readonly string[] {
  const expected_paths = new Set(Object.keys(outputs).map((path) => resolve(path)))
  const obsolete = new Set<string>()
  for (const scope of scopes) {
    for (const path of generated_scope_files(scope)) {
      if (!expected_paths.has(resolve(path))) obsolete.add(path)
    }
  }
  return [...obsolete].sort()
}

/** Returns generated outputs that are missing or differ from the contract. */
export function generated_output_issues(
  outputs: Readonly<Record<string, string>>,
  scopes: readonly Generated_Output_Scope[] = [],
): readonly string[] {
  const mismatches: string[] = []
  for (const [output_path, content] of Object.entries(outputs)) {
    let existing: string
    try {
      existing = readFileSync(output_path, "utf8")
    } catch {
      mismatches.push(`${output_path} (missing)`)
      continue
    }
    if (existing !== content) mismatches.push(output_path)
  }
  for (const output_path of generated_scope_obsolete_files(outputs, scopes)) {
    mismatches.push(`${output_path} (obsolete)`)
  }
  return mismatches
}

function generated_output_scopes(target: Generation_Target): readonly Generated_Output_Scope[] {
  switch (target) {
    case "all":
    case "java":
      return [{ directory: GENERATED_OUTPUTS.java_api_root, extensions: [".java"] }]
    default:
      return []
  }
}

function write_outputs(
  outputs: Readonly<Record<string, string>>,
  check_only: boolean,
  scopes: readonly Generated_Output_Scope[],
): void {
  if (check_only) {
    const mismatches = generated_output_issues(outputs, scopes)
    if (mismatches.length > 0) {
      throw new Error(
        "generated contract outputs are stale:\n" +
          mismatches.map((output_path) => `  - ${output_path}`).join("\n") +
          "\nRun `just generate-protocol-contract` to regenerate them.",
      )
    }
    return
  }
  for (const output_path of generated_scope_obsolete_files(outputs, scopes)) {
    rmSync(output_path, { force: true })
    console.log(`Removed obsolete generated output ${output_path}`)
  }
  for (const [output_path, content] of Object.entries(outputs)) {
    const output_directory = dirname(output_path)
    mkdirSync(output_directory, { recursive: true })
    // Parallel build recipes may generate overlapping targets; rename a complete
    // temporary file so readers never observe a partially written contract.
    const temporary_directory = mkdtempSync(join(output_directory, "generate.local."))
    const temporary_path = join(temporary_directory, basename(output_path))
    try {
      writeFileSync(temporary_path, content)
      renameSync(temporary_path, output_path)
      console.log(`Generated ${output_path}`)
    } finally {
      rmSync(temporary_directory, { force: true, recursive: true })
    }
  }
}

/** Runs the protocol contract generator CLI.
 *
 * @returns Process exit code.
 */
export function main(): number {
  try {
    const target = generation_target(process.env.OPENKACHE_GENERATION_TARGET)
    const outputs =
      target === "rust-wire"
        ? expected_wire_outputs(extract_protocol_wire_contract(smithy_ast(false)), target)
        : expected_outputs(extract_client_contract(smithy_ast(true)), target)
    write_outputs(
      outputs,
      process.env.OPENKACHE_GENERATION_CHECK === "1",
      generated_output_scopes(target),
    )
    return 0
  } catch (error) {
    console.error(
      `GENERATION_FAILED: ${error instanceof Error ? error.message : String(error)}\n` +
        "  Why: client language and ABI values can only be generated from valid, complete wire and client Smithy contracts.\n" +
        "  Fix: Run `smithy validate model` for the protocol and client models, correct the reported model or generator error, then rerun `./generate.ts` from the clients directory.",
    )
    return 1
  }
}

if (import.meta.main) process.exit(main())
