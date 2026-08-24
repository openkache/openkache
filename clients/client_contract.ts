/** Smithy client-contract extraction independent from language renderers. */

import {
  type Wire_Contract,
  type Wire_Contract_Adapter,
  type Wire_Entry,
  type Wire_Operation_Contract,
} from "../protocol/wire"
import { extract_wire_contract as extract_protocol_wire_contract } from "../protocol/wire"
import { validate_operation_field_bindings } from "./operation_plans"
import {
  OPERATION_RETRY_MODES,
  type Api_Operation_Retry_Mode,
} from "./operation_models"
import {
  go_exported_name,
  pascal_case,
  snake_case,
  swift_property_name,
} from "./generator_names"
import { encode_vu128 } from "./generator_values"
import type {
  Api_Contract,
  Api_Enum,
  Api_Enum_Member,
  Api_Member,
  Api_Operation,
  Api_Operation_Contract,
  Api_Structure,
  Api_Type,
  Api_Type_Kind,
  Operation_Field_Role,
} from "./operation_models"

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
  /** Target selector for StructuredValue-CBOR-v1 (JSON has no selector). */
  readonly serialization_structured: number
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
  readonly gate0_alpn_version: number
  readonly gate0_compression: number
  readonly gate0_encryption: number
  readonly gate0_item_id_root_key_hex: string
  readonly gate0_namespace_id: number
  readonly gate0_value_selector: number
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

/** Native binding ABI identifiers shared by language-neutral adapters. */
export type Ffi_Input_Kind = "none" | "application_key" | "item_id"

/** Native dispatch and buffer contract for one FFI operation. */
export interface Ffi_Operation_Contract {
  readonly accepts_set_options: boolean
  readonly accepts_value: boolean
  readonly dedicated_abi: boolean
  readonly input_kind: Ffi_Input_Kind
  readonly request_item_count: number
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
  readonly transports: readonly Ffi_Entry[]
  readonly native_abi_functions: readonly Native_Abi_Function[]
  readonly native_abi_structures: readonly Native_Abi_Structure[]
  readonly error_categories: readonly Ffi_Entry[]
  readonly namespace_default_evictions: readonly Ffi_Entry[]
  readonly namespace_default_expirations: readonly Ffi_Entry[]
  readonly namespace_descriptor_decode_statuses: readonly Ffi_Entry[]
  readonly namespace_descriptor_fields: readonly Namespace_Descriptor_Field[]
  readonly namespace_descriptor_layout: Namespace_Descriptor_Layout
  readonly namespace_override_policies: readonly Ffi_Entry[]
  readonly operations: readonly Ffi_Entry[]
  readonly key_specs: readonly Ffi_Entry[]
  readonly result_kinds: readonly Ffi_Entry[]
  readonly request_states: readonly Ffi_Entry[]
  readonly status_categories: readonly Ffi_Entry[]
  readonly value_representations: readonly Ffi_Entry[]
  readonly value_modes: readonly Ffi_Entry[]
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
  /** Whether missing semantic operation fields must fail generation. */
  readonly strict_operation_bindings: boolean
  readonly value_envelope: Value_Envelope_Contract
  readonly value_format: Value_Format_Contract
}

export type Native_Abi_Type =
  | "client_pointer"
  | "result_pointer"
  | "request_pointer"
  | "u8_pointer"
  | "struct_pointer"
  | "size"
  | "uint8"
  | "int32"
  | "uint32"
  | "uint64"
  | "void"

export interface Native_Abi_Parameter {
  readonly name: string
  readonly type: Exclude<Native_Abi_Type, "void">
  readonly mutable: boolean
  readonly structure_name?: string
  readonly ownership: Native_Abi_Ownership
  readonly lifetime: "call" | "request" | "result" | "client"
}

export type Native_Abi_Ownership = "none" | "borrowed" | "copied" | "owned"
export type Native_Abi_Lifetime = "call" | "request" | "result" | "client"

export interface Native_Abi_Function {
  readonly name: string
  readonly optional: boolean
  readonly return_type: Native_Abi_Type
  readonly return_ownership: Native_Abi_Ownership
  readonly return_lifetime: Native_Abi_Lifetime
  readonly parameters: readonly Native_Abi_Parameter[]
}

export interface Native_Abi_Structure {
  readonly name: string
  readonly fields: readonly Native_Abi_Parameter[]
}

const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
const CLIENT_SERVICE_SHAPE_ID = "openkache.client#OpenKacheClient"
const API_NAMESPACE = "openkache.protocol"
const FFI_CONTRACT_TRAIT_ID = "openkache.client#ffiContract"
const CLIENT_DEFAULTS_TRAIT_ID = "openkache.client#clientDefaults"
const OPERATION_CONTRACT_TRAIT_ID = "openkache.protocol#operationContract"
const OPERATION_FIELD_TRAIT_ID = "openkache.protocol#operationField"
const WIRE_CODEC_TRAIT_ID = "openkache.protocol#wireCodec"
const FFI_OPERATION_CONTRACT_TRAIT_ID = "openkache.client#ffiOperationContract"
const VALUE_FORMAT_TRAIT_ID = "openkache.client#valueFormat"
const VALUE_ENVELOPE_TRAIT_ID = "openkache.client#valueEnvelope"
const UNSIGNED_LONG_TRAIT_ID = "openkache.protocol#unsignedLong"
const LEGACY_UNSIGNED_LONG_TRAIT_ID = "openkache.client#unsignedLong"
const FFI_ENUMS = {
  operations: { name: "FfiOperation", kind: "FFI operation" },
  transports: { name: "FfiTransport", kind: "FFI transport" },
  result_kinds: { name: "FfiResultKind", kind: "FFI result" },
  status_categories: { name: "FfiStatusCategory", kind: "FFI status category" },
  error_categories: { name: "FfiErrorCategory", kind: "FFI error category" },
  request_states: { name: "FfiRequestState", kind: "FFI request state" },
  value_representations: {
    name: "FfiValueRepresentation",
    kind: "FFI value representation",
  },
  value_modes: { name: "FfiValueMode", kind: "FFI value mode" },
  connection_states: { name: "FfiConnectionState", kind: "FFI connection state" },
  set_conditions: { name: "FfiSetCondition", kind: "FFI SET condition" },
  key_specs: { name: "FfiKeySpec", kind: "FFI key spec" },
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
    "smithy.api#Double": "double",
    "smithy.api#Integer": "integer",
    "smithy.api#Long": "long",
    "smithy.api#String": "string",
  }
  const prelude =
    (
      member_traits?.[UNSIGNED_LONG_TRAIT_ID] !== undefined ||
      member_traits?.[LEGACY_UNSIGNED_LONG_TRAIT_ID] !== undefined
    ) &&
    target === "smithy.api#Long"
      ? "unsigned_long"
      : prelude_types[target]
  const codec_from_traits = (
    traits: Json_Object | undefined,
    location: string,
  ): string | undefined => {
    const value = traits?.[WIRE_CODEC_TRAIT_ID]
    if (value === undefined) return undefined
    const trait = object_value(value, `${location}.${WIRE_CODEC_TRAIT_ID}`)
    return string_member(trait, "name", `${location}.${WIRE_CODEC_TRAIT_ID}`)
  }
  const member_codec = codec_from_traits(member_traits, target)
  if (prelude !== undefined) {
    return {
      kind: prelude,
      ...(member_codec === undefined ? {} : { wire_codec: member_codec }),
    }
  }

  const shape = object_member(shapes, target, "Smithy AST.shapes")
  const kind = shape_type(shape, `Smithy AST.shapes.${target}`)
  const shape_codec = codec_from_traits(
    optional_object_member(shape, "traits", `Smithy AST.shapes.${target}`),
    `Smithy AST.shapes.${target}`,
  )
  const wire_codec = member_codec ?? shape_codec
  const with_codec = (type: Api_Type): Api_Type =>
    wire_codec === undefined ? type : { ...type, wire_codec: wire_codec }
  switch (kind) {
    case "blob":
      return with_codec({ kind: "blob" })
    case "list": {
      const member = object_member(shape, "member", target)
      const member_target = string_member(member, "target", `${target}.member`)
      return with_codec({ kind: "list", member: api_type(shapes, member_target) })
    }
    case "map": {
      const key = object_member(shape, "key", target)
      const value = object_member(shape, "value", target)
      return with_codec({
        kind: "map",
        key: api_type(
          shapes,
          string_member(key, "target", `${target}.key`),
        ),
        value: api_type(
          shapes,
          string_member(value, "target", `${target}.value`),
        ),
      })
    }
    case "enum": {
      const members = object_member(shape, "members", target)
      const enum_values = Object.entries(members).map(([member_name, value]) => {
        const member = object_value(value, `${target}.${member_name}`)
        const traits = optional_object_member(
          member,
          "traits",
          `${target}.${member_name}`,
        )
        return traits?.["smithy.api#enumValue"] === undefined
          ? member_name
          : string_member(
            traits,
            "smithy.api#enumValue",
            `${target}.${member_name}.traits`,
          )
      })
      return with_codec({
        kind: "enum",
        name: shape_name(target),
        enum_values,
      })
    }
    case "structure":
      return with_codec({ kind: "structure", name: shape_name(target) })
    case "union":
      // Union members are intentionally exposed as the canonical opaque
      // envelope. The envelope includes the active tag and its
      // length-delimited member bytes, so a future typed union adapter can be
      // added without changing the transport framing or pretending that an
      // arbitrary host-language object is wire-safe.
      return with_codec({ kind: "union", name: shape_name(target) })
    default:
      throw new Error(`unsupported API member target ${target} with shape type ${kind}`)
  }
}

function operation_field_role(
  member_traits: Json_Object | undefined,
  location: string,
): Operation_Field_Role | undefined {
  const value = member_traits?.[OPERATION_FIELD_TRAIT_ID]
  if (value === undefined) return undefined
  const trait = object_value(value, `${location}.${OPERATION_FIELD_TRAIT_ID}`)
  const role = string_member(
    trait,
    "role",
    `${location}.${OPERATION_FIELD_TRAIT_ID}`,
  )
  return role as Operation_Field_Role
}

function api_structure(
  shapes: Json_Object,
  target: string,
): Api_Structure {
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
      const field_role = operation_field_role(
        traits,
        `${target}.${name}`,
      )
      return {
        name,
        ...(field_role === undefined
          ? {}
          : { operation_field_role: field_role }),
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
  const scope = optional_string_member(
    contract,
    "scope",
    `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
  ) ?? "global"
  if (scope.length === 0) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.scope must be a non-empty string`,
    )
  }
  const retry_mode = optional_string_member(
    contract,
    "retryMode",
    `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
  ) ?? "always"
  if (!OPERATION_RETRY_MODES.includes(retry_mode as Api_Operation_Retry_Mode)) {
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
  const response_semantics = optional_string_member(
    contract,
    "responseSemantics",
    `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
  )
  return {
    error_statuses,
    ...(response_semantics === undefined ? {} : { response_semantics }),
    retry_mode: retry_mode as Api_Operation_Retry_Mode,
    scope,
    success_statuses,
  }
}

function merge_operation_contract(
  wire_contract: Wire_Operation_Contract | undefined,
  client_contract: Api_Operation_Contract | undefined,
): Api_Operation_Contract | undefined {
  if (wire_contract === undefined) return client_contract
  return {
    ...wire_contract,
    ...(client_contract ?? {
      retry_mode: "always",
      scope: "global",
    }),
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
  protocol_operations?: readonly {
    readonly contract: Wire_Operation_Contract
    readonly name: string
  }[],
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
      const wire_contract = protocol_operations?.find(
        (operation) => operation.name === shape_name(target),
      )?.contract
      const semantic_contract = merge_operation_contract(
        wire_contract,
        operation_contract(shape, target),
      )
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
    const structure = api_structure(
      shapes,
      `${namespace}#${name}`,
    )
    structures_by_name.set(name, structure)
    const visit_type = (member_type: Api_Type): void => {
      if (member_type.name !== undefined) {
        if (member_type.kind === "enum") {
          enum_names.add(member_type.name)
        } else if (member_type.kind === "structure") {
          pending_structure_names.push(member_type.name)
        }
      }
      if (member_type.member !== undefined) visit_type(member_type.member)
      if (member_type.key !== undefined) visit_type(member_type.key)
      if (member_type.value !== undefined) visit_type(member_type.value)
    }
    for (const member of structure.members) {
      visit_type(member.type)
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
    request_item_count: 0,
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
  const descriptor = api_structure(
    shapes,
    `${namespace}#FfiNamespaceDescriptor`,
  )
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
  "request_pointer",
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
  const pointer_type =
    type === "client_pointer" ||
    type === "result_pointer" ||
    type === "request_pointer" ||
    type === "u8_pointer" ||
    type === "struct_pointer"
  // Function parameters are always inputs to the ABI boundary, including
  // opaque handle pointers. Returned handles are owned by the caller and are
  // represented by the function return type; they are never parameter-owned.
  const ownership_value = optional_string_member(parameter, "ownership", location) ??
    (pointer_type ? "borrowed" : "none")
  if (!["none", "borrowed", "copied", "owned"].includes(ownership_value)) {
    throw new Error(
      `${location}.ownership must be borrowed, copied, or owned`,
    )
  }
  if (pointer_type ? ownership_value === "none" : ownership_value !== "none") {
    throw new Error(
      `${location}.ownership must be none for scalars and a transfer mode for pointers`,
    )
  }
  const lifetime_value = optional_string_member(parameter, "lifetime", location) ??
    (type === "client_pointer"
      ? "client"
      : type === "request_pointer"
      ? "request"
      : type === "result_pointer"
      ? "result"
      : "call")
  if (!["call", "request", "result", "client"].includes(lifetime_value)) {
    throw new Error(
      `${location}.lifetime must be call, request, result, or client`,
    )
  }
  return {
    name: string_member(parameter, "name", location),
    type,
    mutable: boolean_member(parameter, "mutable", location),
    ...(structure_name === undefined ? {} : { structure_name }),
    ownership: ownership_value as Native_Abi_Parameter["ownership"],
    lifetime: lifetime_value as Native_Abi_Parameter["lifetime"],
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
      const return_type = native_abi_type(function_value, "returnType", location)
      const return_pointer = return_type.includes("pointer")
      const return_ownership = optional_string_member(
        function_value,
        "returnOwnership",
        location,
      ) ?? (return_pointer ? "owned" : "none")
      if (!["none", "borrowed", "copied", "owned"].includes(return_ownership)) {
        throw new Error(
          `${location}.returnOwnership must be borrowed, copied, or owned`,
        )
      }
      if (return_pointer ? return_ownership === "none" : return_ownership !== "none") {
        throw new Error(
          `${location}.returnOwnership must be none for scalars and a transfer mode for pointers`,
        )
      }
      const return_lifetime = optional_string_member(
        function_value,
        "returnLifetime",
        location,
      ) ?? (return_pointer ? "result" : "call")
      if (!["call", "request", "result", "client"].includes(return_lifetime)) {
        throw new Error(
          `${location}.returnLifetime must be call, request, result, or client`,
        )
      }
      return {
        name: string_member(function_value, "name", location),
        optional: optional_boolean_member(function_value, "optional", location),
        return_type,
        return_ownership: return_ownership as Native_Abi_Ownership,
        return_lifetime: return_lifetime as Native_Abi_Lifetime,
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
    transports:
      shapes[`${namespace}#${FFI_ENUMS.transports.name}`] === undefined
        ? []
        : ffi_enum_entries(
            shapes,
            namespace,
            FFI_ENUMS.transports.name,
            FFI_ENUMS.transports.kind,
          ),
    error_categories: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.error_categories.name,
      FFI_ENUMS.error_categories.kind,
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
    key_specs: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.key_specs.name,
      FFI_ENUMS.key_specs.kind,
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
    request_states: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.request_states.name,
      FFI_ENUMS.request_states.kind,
    ),
    status_categories: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.status_categories.name,
      FFI_ENUMS.status_categories.kind,
    ),
    value_representations: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.value_representations.name,
      FFI_ENUMS.value_representations.kind,
    ),
    value_modes: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.value_modes.name,
      FFI_ENUMS.value_modes.kind,
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
    serialization_structured: integer_member(
      contract,
      "serializationStructured",
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
      9,
      9,
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
      { name: "Structured", value: values.serialization_structured },
    ],
    "serialization",
  )
  if (values.serialization_structured !== 1) {
    throw new Error(
      `${VALUE_FORMAT_TRAIT_ID}.serializationStructured must be selector 1`,
    )
  }
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
    gate0_alpn_version: integer_member(
      contract,
      "gate0AlpnVersion",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    gate0_compression: integer_member(
      contract,
      "gate0Compression",
      CLIENT_DEFAULTS_TRAIT_ID,
      0,
      0xff,
    ),
    gate0_encryption: integer_member(
      contract,
      "gate0Encryption",
      CLIENT_DEFAULTS_TRAIT_ID,
      0,
      0xff,
    ),
    gate0_item_id_root_key_hex: string_member(
      contract,
      "gate0ItemIdRootKeyHex",
      CLIENT_DEFAULTS_TRAIT_ID,
    ),
    gate0_namespace_id: integer_member(
      contract,
      "gate0NamespaceId",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    gate0_value_selector: integer_member(
      contract,
      "gate0ValueSelector",
      CLIENT_DEFAULTS_TRAIT_ID,
      0,
      0xff,
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
  if (!/^[0-9a-f]{64}$/i.test(defaults.gate0_item_id_root_key_hex)) {
    throw new Error(
      `${CLIENT_DEFAULTS_TRAIT_ID}.gate0ItemIdRootKeyHex must contain exactly 32 hexadecimal bytes`,
    )
  }
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
export function extract_client_contract(
  ast: unknown,
  strict_operation_bindings = false,
  wire_adapter?: Wire_Contract_Adapter,
): Client_Contract {
  const ast_object = object_value(ast, "Smithy AST")
  const shapes = object_member(ast_object, "shapes", "Smithy AST")
  const has_client_service = shapes[CLIENT_SERVICE_SHAPE_ID] !== undefined
  const wire = extract_protocol_wire_contract(
    ast,
    strict_operation_bindings || has_client_service,
    wire_adapter,
  )
  // Preserve the canonical ordered field plans in the client contract. The
  // language renderers still project Smithy types for ergonomic source APIs,
  // but request/response cardinality and transport metadata must not be
  // re-derived independently from a second role-flattening algorithm.
  const client_wire = wire
  const client_service_id = has_client_service
    ? CLIENT_SERVICE_SHAPE_ID
    : SERVICE_SHAPE_ID
  if (client_service_id === CLIENT_SERVICE_SHAPE_ID && client_wire.operations === undefined) {
    throw new Error(
      "protocol operation semantics are incomplete; strict protocol generation must provide " +
        "an operationContract for every opcode",
    )
  }
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
    API_NAMESPACE,
    client_wire.opcodes.map((entry) => entry.name),
    client_wire.operations,
  )
  const api = {
    ...parsed_api,
    // Smithy AST output is not required to preserve service-operation order.
    // Use the protocol assignments so every generated API presents operations
    // in the same stable order as the wire contract.
    operations: [...parsed_api.operations].sort(
      (left, right) =>
        (client_wire.opcodes.find((entry) => entry.name === left.name)?.value ?? 0) -
        (client_wire.opcodes.find((entry) => entry.name === right.name)?.value ?? 0),
    ),
  }
  const opcode_names = new Set(client_wire.opcodes.map((entry) => entry.name))
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
  for (const opcode of client_wire.opcodes) {
    if (!api_operation_names.has(opcode.name)) {
      throw new Error(
        `wire opcode ${opcode.name} has no matching client operation`,
      )
    }
  }
  if (strict_operation_bindings || client_service_id === CLIENT_SERVICE_SHAPE_ID) {
    validate_operation_field_bindings(api)
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
    ...client_wire,
    api,
    client_defaults: client_defaults_contract(client_defaults_trait),
    ffi,
    strict_operation_bindings:
      strict_operation_bindings || client_service_id === CLIENT_SERVICE_SHAPE_ID,
    value_envelope: value_envelope_contract(value_envelope_trait),
    value_format: value_format_contract(value_format_trait),
  }
}
