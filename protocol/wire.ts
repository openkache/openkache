/** Smithy extraction and rendering for the server-visible OpenKache wire contract. */

import { existsSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

type Json_Object = Readonly<Record<string, unknown>>

/** One numeric protocol member assigned by the wire contract. */
export interface Wire_Entry {
  readonly name: string
  /** Optional Smithy enum value used for generated labels. */
  readonly text?: string
  readonly value: number
}

/** Semantic contract declared by one Smithy protocol operation. */
export const OPERATION_SCOPES = [
  "global",
  "item",
  "namespace",
  "namespace_management",
] as const

export type Wire_Operation_Scope = (typeof OPERATION_SCOPES)[number]

export const OPERATION_REQUEST_KINDS = [
  "empty",
  "application_value",
  "scoped_item",
  "scoped_namespace",
  "namespace_open",
  "namespace_update_policy",
  "namespace_delete",
] as const

export type Wire_Operation_Request_Kind = (typeof OPERATION_REQUEST_KINDS)[number]

export const OPERATION_RESPONSE_KINDS = [
  "empty",
  "pong",
  "application_value",
  "value",
  "set_outcome",
  "delete_outcome",
  "stats_json",
  "namespace_descriptor",
] as const

export type Wire_Operation_Response_Kind = (typeof OPERATION_RESPONSE_KINDS)[number]

export const OPERATION_VALUE_TRANSFORMS = [
  "identity",
  "reverse_utf8",
] as const

/**
 * Runtime Smithy enum values are authoritative. The literal fallback above
 * remains only for legacy AST fixtures that do not contain the enum shape.
 */
export type Wire_Operation_Value_Transform = string

export const OPERATION_RETRY_MODES = [
  "always",
  "never",
  "when_not_creating",
] as const

export type Wire_Operation_Retry_Mode = (typeof OPERATION_RETRY_MODES)[number]

export const OPERATION_REQUEST_SCOPES: Readonly<
  Record<Wire_Operation_Request_Kind, Wire_Operation_Scope>
> = {
  empty: "global",
  application_value: "global",
  scoped_item: "item",
  scoped_namespace: "namespace",
  namespace_open: "namespace_management",
  namespace_update_policy: "namespace_management",
  namespace_delete: "namespace_management",
}

/** Response shapes permitted for each protocol-owned request shape. */
export const OPERATION_RESPONSE_KINDS_BY_REQUEST: Readonly<
  Record<Wire_Operation_Request_Kind, readonly Wire_Operation_Response_Kind[]>
> = {
  empty: ["pong"],
  application_value: ["application_value"],
  scoped_item: ["value", "set_outcome", "delete_outcome"],
  scoped_namespace: ["stats_json", "empty"],
  namespace_open: ["namespace_descriptor"],
  namespace_update_policy: ["namespace_descriptor"],
  namespace_delete: ["empty"],
}

export interface Wire_Operation_Contract {
  readonly error_statuses: readonly string[]
  readonly request_kind: Wire_Operation_Request_Kind
  /**
   * Number of value fields in the request shape. Production extraction uses
   * this role count to choose the SET wire layout without consulting the
   * response contract. Legacy AST fixtures may omit it.
   */
  readonly request_value_count?: number
  /** Number of item IDs carried by a scoped-item request, derived from Smithy roles. */
  readonly request_item_count: number
  /** Number of optional values carried by a value response, derived from Smithy roles. */
  readonly response_value_count: number
  readonly response_kind: Wire_Operation_Response_Kind
  readonly retry_mode: Wire_Operation_Retry_Mode
  readonly scope: Wire_Operation_Scope
  readonly success_statuses: readonly string[]
  /** Optional application-value transform; omitted means identity. */
  readonly value_transform?: Wire_Operation_Value_Transform
}

/** Smithy operation vocabularies extracted from the protocol model. */
export interface Wire_Operation_Vocabularies {
  readonly value_transforms: readonly string[]
}

/** One protocol opcode and its Smithy semantic operation contract. */
export interface Wire_Operation {
  readonly contract: Wire_Operation_Contract
  readonly name: string
}

/** Protocol v1 constants consumed by the Rust protocol crate. */
export interface Wire_V1_Contract {
  readonly alpn: string
  readonly opcode_bytes: number
  readonly status_bytes: number
  readonly request_fixed_bytes: number
  readonly response_fixed_bytes: number
  readonly min_varuint_bytes: number
  readonly max_varuint_bytes: number
  readonly namespace_id_bytes: number
  readonly namespace_revision_bytes: number
  readonly namespace_name_length_bytes: number
  readonly namespace_name_max_bytes: number
  readonly set_flags_bytes: number
  readonly set_condition_mask: number
  readonly set_condition_any_bits: number
  readonly set_condition_reserved_bits: number
  readonly set_expiration_mask: number
  readonly set_inherit_expiration_bits: number
  readonly set_no_expiry_bits: number
  /** The SET expiration-mode bit pattern for ExplicitTtl. */
  readonly set_ttl_flag: number
  readonly set_expiration_reserved_bits: number
  readonly set_eviction_mask: number
  readonly set_inherit_eviction_bits: number
  readonly set_evictable_bits: number
  readonly set_eviction_protected_bits: number
  readonly set_eviction_reserved_bits: number
  readonly set_reserved_mask: number
  readonly open_flags_bytes: number
  readonly open_create_if_missing_flag: number
  readonly open_reserved_mask: number
  readonly delete_flags_bytes: number
  readonly delete_if_empty_bits: number
  readonly delete_mode_mask: number
  readonly delete_reserved_mask: number
  readonly policy_flags_bytes: number
  readonly policy_default_expiration_mask: number
  readonly policy_no_expiry_bits: number
  readonly policy_fixed_ttl_bits: number
  readonly policy_default_expiration_reserved_bits: number
  readonly policy_expiration_override_flag: number
  readonly policy_eviction_protected_flag: number
  readonly policy_eviction_override_flag: number
  readonly policy_reserved_mask: number
  readonly error_status_minimum: number
  readonly set_if_absent_flag: number
  readonly set_if_present_flag: number
}

/** Language-neutral server-visible subset of the OpenKache Smithy model. */
export interface Wire_Contract {
  readonly item_id_bytes: number
  readonly max_value_bytes: number
  /**
   * Operation metadata is optional for legacy AST fixtures. Production
   * protocol generation runs in strict mode and always emits it.
   */
  readonly operations?: readonly Wire_Operation[]
  readonly opcodes: readonly Wire_Entry[]
  /**
   * Values extracted from `OperationValueTransform` when the model exposes
   * that enum. Legacy fixtures may omit this field.
   */
  readonly operation_vocabularies?: Wire_Operation_Vocabularies
  readonly statuses: readonly Wire_Entry[]
  readonly v1: Wire_V1_Contract
}

const PROTOCOL_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const MODEL_DIRECTORY = "model"
const SMITHY_EXECUTABLE = process.env.OPENKACHE_SMITHY_EXECUTABLE ?? "smithy"
const SMITHY_USE_SHELL = process.env.OPENKACHE_SMITHY_USE_SHELL === "1"

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
const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
const OPCODE_SHAPE_ID = "openkache.protocol#Opcode"
const STATUS_SHAPE_ID = "openkache.protocol#Status"
const WIRE_CONTRACT_TRAIT_ID = "openkache.protocol#wireContract"
const WIRE_OPCODE_TRAIT_ID = "openkache.protocol#wireOpcode"
const WIRE_STATUS_TRAIT_ID = "openkache.protocol#wireStatus"
const OPERATION_CONTRACT_TRAIT_ID = "openkache.protocol#operationContract"
const OPERATION_FIELD_TRAIT_ID = "openkache.protocol#operationField"
const OPERATION_VALUE_TRANSFORM_SHAPE_ID =
  "openkache.protocol#OperationValueTransform"

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

function shape_type(shape: Json_Object, location: string): string {
  return string_member(shape, "type", location)
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

function trait_value(
  shape: Json_Object,
  trait_id: string,
  location: string,
): Json_Object {
  const traits = object_member(shape, "traits", location)
  return object_member(traits, trait_id, `${location}.traits`)
}

function optional_enum_value(shape: Json_Object, location: string): string | undefined {
  const traits = object_member(shape, "traits", location)
  const value = traits["smithy.api#enumValue"]
  return value === undefined
    ? undefined
    : string_member(traits, "smithy.api#enumValue", `${location}.traits`)
}

function unique_wire_values(entries: readonly Wire_Entry[], kind: string): void {
  const names = new Set<string>()
  const texts = new Set<string>()
  const values = new Set<number>()
  for (const entry of entries) {
    if (names.has(entry.name)) throw new Error(`duplicate ${kind} name ${entry.name}`)
    if (entry.text !== undefined && texts.has(entry.text)) {
      throw new Error(`duplicate ${kind} enum value ${entry.text}`)
    }
    if (values.has(entry.value)) {
      throw new Error(`duplicate ${kind} wire value ${entry.value}`)
    }
    names.add(entry.name)
    if (entry.text !== undefined) texts.add(entry.text)
    values.add(entry.value)
  }
}

function wire_v1_contract(value: unknown): Wire_V1_Contract {
  const contract = object_value(value, `${WIRE_CONTRACT_TRAIT_ID}.v1`)
  const v1 = {
    alpn: string_member(contract, "alpn", "wireContract.v1"),
    opcode_bytes: integer_member(contract, "opcodeBytes", "wireContract.v1", 1, 0xff),
    status_bytes: integer_member(contract, "statusBytes", "wireContract.v1", 1, 0xff),
    request_fixed_bytes: integer_member(
      contract,
      "requestFixedBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    response_fixed_bytes: integer_member(
      contract,
      "responseFixedBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    min_varuint_bytes: integer_member(
      contract,
      "minVaruintBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    max_varuint_bytes: integer_member(contract, "maxVaruintBytes", "wireContract.v1", 1),
    namespace_id_bytes: integer_member(
      contract,
      "namespaceIdBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    namespace_revision_bytes: integer_member(
      contract,
      "namespaceRevisionBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    namespace_name_length_bytes: integer_member(
      contract,
      "namespaceNameLengthBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    namespace_name_max_bytes: integer_member(
      contract,
      "namespaceNameMaxBytes",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_flags_bytes: integer_member(
      contract,
      "setFlagsBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    set_condition_mask: integer_member(
      contract,
      "setConditionMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_condition_any_bits: integer_member(
      contract,
      "setConditionAnyBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_if_absent_flag: integer_member(
      contract,
      "setIfAbsentFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_if_present_flag: integer_member(
      contract,
      "setIfPresentFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_condition_reserved_bits: integer_member(
      contract,
      "setConditionReservedBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_expiration_mask: integer_member(
      contract,
      "setExpirationMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_inherit_expiration_bits: integer_member(
      contract,
      "setInheritExpirationBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_no_expiry_bits: integer_member(
      contract,
      "setNoExpiryBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_ttl_flag: integer_member(contract, "setTtlFlag", "wireContract.v1", 0, 0xff),
    set_expiration_reserved_bits: integer_member(
      contract,
      "setExpirationReservedBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_eviction_mask: integer_member(
      contract,
      "setEvictionMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_inherit_eviction_bits: integer_member(
      contract,
      "setInheritEvictionBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_evictable_bits: integer_member(
      contract,
      "setEvictableBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_eviction_protected_bits: integer_member(
      contract,
      "setEvictionProtectedBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_eviction_reserved_bits: integer_member(
      contract,
      "setEvictionReservedBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_reserved_mask: integer_member(
      contract,
      "setReservedMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    open_flags_bytes: integer_member(
      contract,
      "openFlagsBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    open_create_if_missing_flag: integer_member(
      contract,
      "openCreateIfMissingFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    open_reserved_mask: integer_member(
      contract,
      "openReservedMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    delete_flags_bytes: integer_member(
      contract,
      "deleteFlagsBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    delete_if_empty_bits: integer_member(
      contract,
      "deleteIfEmptyBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    delete_mode_mask: integer_member(
      contract,
      "deleteModeMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    delete_reserved_mask: integer_member(
      contract,
      "deleteReservedMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_flags_bytes: integer_member(
      contract,
      "policyFlagsBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    policy_default_expiration_mask: integer_member(
      contract,
      "policyDefaultExpirationMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_no_expiry_bits: integer_member(
      contract,
      "policyNoExpiryBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_fixed_ttl_bits: integer_member(
      contract,
      "policyFixedTtlBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_default_expiration_reserved_bits: integer_member(
      contract,
      "policyDefaultExpirationReservedBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_expiration_override_flag: integer_member(
      contract,
      "policyExpirationOverrideFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_eviction_protected_flag: integer_member(
      contract,
      "policyEvictionProtectedFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_eviction_override_flag: integer_member(
      contract,
      "policyEvictionOverrideFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_reserved_mask: integer_member(
      contract,
      "policyReservedMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    error_status_minimum: integer_member(
      contract,
      "errorStatusMinimum",
      "wireContract.v1",
      0,
      0xff,
    ),
  } satisfies Wire_V1_Contract
  if (v1.alpn !== "openkache/1") {
    throw new Error(
      `${WIRE_CONTRACT_TRAIT_ID}.v1.alpn must be "openkache/1" for the current protocol implementation`,
    )
  }
  if (
    v1.opcode_bytes !== 1 ||
    v1.status_bytes !== 1 ||
    v1.request_fixed_bytes !== 1 ||
    v1.response_fixed_bytes !== 1
  ) {
    throw new Error(
      `${WIRE_CONTRACT_TRAIT_ID}.v1 opcode, status, request, and response fixed sizes must all be 1`,
    )
  }
  if (v1.min_varuint_bytes !== 1 || v1.max_varuint_bytes !== 9) {
    throw new Error(
      `${WIRE_CONTRACT_TRAIT_ID}.v1 vu128 widths must be minimum=1 and maximum=9 for the unsigned 64-bit protocol`,
    )
  }
  if (
    v1.namespace_id_bytes !== 8 ||
    v1.namespace_revision_bytes !== 8 ||
    v1.namespace_name_length_bytes !== 1 ||
    v1.namespace_name_max_bytes !== 0xff ||
    v1.set_flags_bytes !== 1 ||
    v1.open_flags_bytes !== 1 ||
    v1.delete_flags_bytes !== 1 ||
    v1.policy_flags_bytes !== 1
  ) {
    throw new Error(
      "wire v1 fixed field widths must be namespace/revision=8, name length and flag fields=1, and name max=255",
    )
  }
  const flag_groups = [
    {
      name: "SET condition",
      mask: v1.set_condition_mask,
      values: [
        v1.set_condition_any_bits,
        v1.set_if_absent_flag,
        v1.set_if_present_flag,
        v1.set_condition_reserved_bits,
      ],
    },
    {
      name: "SET expiration",
      mask: v1.set_expiration_mask,
      values: [
        v1.set_inherit_expiration_bits,
        v1.set_no_expiry_bits,
        v1.set_ttl_flag,
        v1.set_expiration_reserved_bits,
      ],
    },
    {
      name: "SET eviction",
      mask: v1.set_eviction_mask,
      values: [
        v1.set_inherit_eviction_bits,
        v1.set_evictable_bits,
        v1.set_eviction_protected_bits,
        v1.set_eviction_reserved_bits,
      ],
    },
    {
      name: "namespace policy expiration",
      mask: v1.policy_default_expiration_mask,
      values: [
        v1.policy_no_expiry_bits,
        v1.policy_fixed_ttl_bits,
        v1.policy_default_expiration_reserved_bits,
      ],
    },
  ] as const
  for (const group of flag_groups) {
    unique_wire_values(
      group.values.map((value, index) => ({ name: `${group.name} ${index}`, value })),
      group.name,
    )
    if (group.values.some((value) => (value & ~group.mask) !== 0)) {
      throw new Error(`${group.name} values must fit within mask 0x${group.mask.toString(16)}`)
    }
  }
  if (
    v1.set_if_absent_flag !== 0x01 ||
    v1.set_if_present_flag !== 0x02 ||
    v1.set_condition_reserved_bits !== v1.set_condition_mask ||
    v1.set_expiration_reserved_bits !== v1.set_expiration_mask ||
    v1.set_eviction_reserved_bits !== v1.set_eviction_mask ||
    v1.set_reserved_mask !== 0xc0
  ) {
    throw new Error("SET masks and reserved values do not match the v1 bit layout")
  }
  if (
    v1.open_create_if_missing_flag === 0 ||
    v1.open_reserved_mask !== (0xff ^ v1.open_create_if_missing_flag) ||
    v1.delete_if_empty_bits !== 0 ||
    v1.delete_reserved_mask !== (0xff ^ v1.delete_mode_mask) ||
    v1.policy_expiration_override_flag !== 0x04 ||
    v1.policy_eviction_protected_flag !== 0x08 ||
    v1.policy_eviction_override_flag !== 0x10 ||
    v1.policy_reserved_mask !== 0xe0
  ) {
    throw new Error("namespace open/delete/policy flags do not match the v1 bit layout")
  }
  if (v1.error_status_minimum !== 0x80) {
    throw new Error("wire v1 errorStatusMinimum must be 0x80")
  }
  return v1
}

function wire_enum_entries(
  shapes: Json_Object,
  shape_id: string,
  trait_id: string,
  kind: string,
): readonly Wire_Entry[] {
  const enum_shape = object_member(shapes, shape_id, "Smithy AST.shapes")
  const members = object_member(enum_shape, "members", shape_id)
  const entries = Object.entries(members)
    .map(([name, member]): Wire_Entry => {
      const member_shape = object_value(member, `${shape_id}.${name}`)
      const trait = trait_value(member_shape, trait_id, `${shape_id}.${name}`)
      return {
        name: pascal_case(name),
        text: optional_enum_value(member_shape, `${shape_id}.${name}`),
        value: integer_member(
          trait,
          "value",
          `${shape_id}.${name}.${trait_id}`,
          0,
          0xff,
        ),
      }
    })
    .sort((left, right) => left.value - right.value)
  unique_wire_values(entries, kind)
  if (entries.length === 0) throw new Error(`${kind} contract must define at least one entry`)
  return entries
}

function smithy_enum_values(
  shapes: Json_Object,
  shape_id: string,
  kind: string,
): readonly string[] {
  const enum_shape = object_member(shapes, shape_id, "Smithy AST.shapes")
  if (shape_type(enum_shape, `Smithy AST.shapes.${shape_id}`) !== "enum") {
    throw new Error(`${shape_id} must be an enum`)
  }
  const members = object_member(enum_shape, "members", shape_id)
  const values = Object.entries(members).map(([name, value]) => {
    const member_shape = object_value(value, `${shape_id}.${name}`)
    return string_member(
      object_member(member_shape, "traits", `${shape_id}.${name}`),
      "smithy.api#enumValue",
      `${shape_id}.${name}.traits`,
    )
  })
  if (values.length === 0) {
    throw new Error(`${kind} enum must define at least one value`)
  }
  if (new Set(values).size !== values.length) {
    throw new Error(`duplicate ${kind} enum value`)
  }
  return values
}

function optional_object_member(
  object: Json_Object,
  member: string,
  location: string,
): Json_Object | undefined {
  const value = object[member]
  return value === undefined ? undefined : object_value(value, `${location}.${member}`)
}

function operation_shape_field_count(
  shapes: Json_Object,
  operation_shape: Json_Object,
  operation_target: string,
  direction: "input" | "output",
  role: string,
): number {
  const shape_reference = object_member(
    operation_shape,
    direction,
    operation_target,
  )
  const shape_target = string_member(
    shape_reference,
    "target",
    `${operation_target}.${direction}`,
  )
  const structure = object_member(shapes, shape_target, "Smithy AST.shapes")
  if (shape_type(structure, `Smithy AST.shapes.${shape_target}`) !== "structure") {
    throw new Error(`${shape_target} must be a structure`)
  }
  const members = object_member(structure, "members", shape_target)
  return Object.entries(members).filter(([member_name, value]) => {
    const member = object_value(value, `${shape_target}.${member_name}`)
    const traits = optional_object_member(
      member,
      "traits",
      `${shape_target}.${member_name}`,
    )
    const field = traits?.[OPERATION_FIELD_TRAIT_ID]
    if (field === undefined) return false
    return string_member(
      object_value(field, `${shape_target}.${member_name}.${OPERATION_FIELD_TRAIT_ID}`),
      "role",
      `${shape_target}.${member_name}.${OPERATION_FIELD_TRAIT_ID}`,
    ) === role
  }).length
}

function operation_contract(
  shapes: Json_Object,
  shape: Json_Object,
  target: string,
  statuses: readonly Wire_Entry[],
  value_transforms: readonly string[] = OPERATION_VALUE_TRANSFORMS,
): Wire_Operation_Contract | undefined {
  const traits = optional_object_member(shape, "traits", target)
  const value = traits?.[OPERATION_CONTRACT_TRAIT_ID]
  if (value === undefined) return undefined
  const contract = object_value(value, `${target}.traits.${OPERATION_CONTRACT_TRAIT_ID}`)
  const scope = string_member(contract, "scope", `${target}.${OPERATION_CONTRACT_TRAIT_ID}`)
  if (!OPERATION_SCOPES.includes(scope as Wire_Operation_Scope)) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.scope must be global, item, namespace, or namespace_management`,
    )
  }
  const request_kind = string_member(
    contract,
    "requestKind",
    `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
  )
  if (!OPERATION_REQUEST_KINDS.includes(request_kind as Wire_Operation_Request_Kind)) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.requestKind is not a supported request kind`,
    )
  }
  if (OPERATION_REQUEST_SCOPES[request_kind as Wire_Operation_Request_Kind] !== scope) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.requestKind ${request_kind} is incompatible with scope ${scope}`,
    )
  }

  const response_kind = string_member(
    contract,
    "responseKind",
    `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
  )
  if (!OPERATION_RESPONSE_KINDS.includes(response_kind as Wire_Operation_Response_Kind)) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.responseKind is not a supported response kind`,
    )
  }
  const allowed_response_kinds =
    OPERATION_RESPONSE_KINDS_BY_REQUEST[
      request_kind as Wire_Operation_Request_Kind
    ]
  if (!allowed_response_kinds.includes(response_kind as Wire_Operation_Response_Kind)) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID} responseKind ${response_kind} is incompatible with requestKind ${request_kind}`,
    )
  }

  const retry_mode = string_member(
    contract,
    "retryMode",
    `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
  )
  if (!OPERATION_RETRY_MODES.includes(retry_mode as Wire_Operation_Retry_Mode)) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.retryMode must be always, never, or when_not_creating`,
    )
  }

  const value_transform_value = contract["valueTransform"]
  const value_transform =
    value_transform_value === undefined
      ? undefined
      : string_member(
          contract,
          "valueTransform",
          `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
        )
  if (
    value_transform !== undefined &&
    !value_transforms.includes(value_transform)
  ) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.valueTransform must be one of ${value_transforms.join(", ")}`,
    )
  }
  const parsed_value_transform =
    value_transform as Wire_Operation_Value_Transform | undefined
  const request_item_count = operation_shape_field_count(
    shapes,
    shape,
    target,
    "input",
    "item_id",
  )
  const request_value_count = operation_shape_field_count(
    shapes,
    shape,
    target,
    "input",
    "value",
  )
  const response_value_count = operation_shape_field_count(
    shapes,
    shape,
    target,
    "output",
    "value",
  )
  if (request_kind === "scoped_item" && request_item_count === 0) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID} scoped_item operations must define at least one item_id member`,
    )
  }
  if (request_kind !== "scoped_item" && request_item_count !== 0) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID} non-scoped-item operations cannot define item_id members`,
    )
  }
  if (response_kind === "value" && response_value_count === 0) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID} value operations must define at least one value member`,
    )
  }

  const status_names = new Set(
    statuses.flatMap((status) => [
      status.name,
      status.text ?? wire_name(status.name),
    ]),
  )
  const status_values = (member: string): readonly string[] => {
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
      if (!status_names.has(value)) {
        throw new Error(
          `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member}[${index}] references unknown protocol status ${value}`,
        )
      }
      return value
    })
    if (new Set(values).size !== values.length) {
      throw new Error(
        `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member} must not contain duplicate statuses`,
      )
    }
    if (values.length === 0) {
      throw new Error(
        `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member} must not be empty`,
      )
    }
    return values
  }
  const success_statuses = status_values("successStatuses")
  const error_statuses = status_values("errorStatuses")
  if (success_statuses.some((status) => error_statuses.includes(status))) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID} has overlapping success and error statuses`,
    )
  }
  return {
    error_statuses,
    request_kind: request_kind as Wire_Operation_Contract["request_kind"],
    request_value_count,
    request_item_count,
    response_value_count,
    response_kind: response_kind as Wire_Operation_Contract["response_kind"],
    retry_mode: retry_mode as Wire_Operation_Contract["retry_mode"],
    scope: scope as Wire_Operation_Contract["scope"],
    success_statuses,
    ...(parsed_value_transform === undefined
      ? {}
      : { value_transform: parsed_value_transform }),
  }
}

function wire_operations(
  shapes: Json_Object,
  opcodes: readonly Wire_Entry[],
  statuses: readonly Wire_Entry[],
  value_transforms: readonly string[],
  strict: boolean,
): readonly Wire_Operation[] | undefined {
  const operations: Wire_Operation[] = []
  for (const opcode of opcodes) {
    const target = `${SERVICE_SHAPE_ID.slice(0, SERVICE_SHAPE_ID.lastIndexOf("#"))}#${opcode.name}`
    const shape = shapes[target]
    if (shape === undefined) {
      if (strict) throw new Error(`opcode ${opcode.name} has no matching Smithy operation`)
      return undefined
    }
    const contract = operation_contract(
      shapes,
      object_value(shape, `Smithy AST.shapes.${target}`),
      target,
      statuses,
      value_transforms,
    )
    if (contract === undefined) {
      if (strict) {
        throw new Error(`operation ${opcode.name} is missing ${OPERATION_CONTRACT_TRAIT_ID}`)
      }
      return undefined
    }
    operations.push({ contract, name: opcode.name })
  }
  return operations
}

/** Extracts the server-visible wire contract from a Smithy AST. */
export function extract_wire_contract(ast: unknown, strict_operations = false): Wire_Contract {
  const ast_object = object_value(ast, "Smithy AST")
  const shapes = object_member(ast_object, "shapes", "Smithy AST")
  const service = object_member(shapes, SERVICE_SHAPE_ID, "Smithy AST.shapes")
  const contract_trait = trait_value(
    service,
    WIRE_CONTRACT_TRAIT_ID,
    `Smithy AST.shapes.${SERVICE_SHAPE_ID}`,
  )
  const opcode_shape = shapes[OPCODE_SHAPE_ID]
  const opcodes =
    opcode_shape === undefined
      ? array_member(service, "operations", SERVICE_SHAPE_ID)
          .map((operation, index): Wire_Entry => {
            const reference = object_value(operation, `${SERVICE_SHAPE_ID}.operations[${index}]`)
            const target = string_member(
              reference,
              "target",
              `${SERVICE_SHAPE_ID}.operations[${index}]`,
            )
            const operation_shape = object_member(shapes, target, "Smithy AST.shapes")
            const trait = trait_value(
              operation_shape,
              WIRE_OPCODE_TRAIT_ID,
              `Smithy AST.shapes.${target}`,
            )
            return {
              name: pascal_case(shape_name(target)),
              value: integer_member(
                trait,
                "value",
                `${target}.${WIRE_OPCODE_TRAIT_ID}`,
                0,
                0xff,
              ),
            }
          })
          .sort((left, right) => left.value - right.value)
      : wire_enum_entries(
          shapes,
          OPCODE_SHAPE_ID,
          WIRE_OPCODE_TRAIT_ID,
          "opcode",
        )
  unique_wire_values(opcodes, "opcode")
  if (opcodes.length === 0) throw new Error("opcode contract must define at least one entry")
  const statuses = wire_enum_entries(
    shapes,
    STATUS_SHAPE_ID,
    WIRE_STATUS_TRAIT_ID,
    "status",
  )
  const operation_value_transform_shape =
    shapes[OPERATION_VALUE_TRANSFORM_SHAPE_ID]
  const value_transforms =
    operation_value_transform_shape === undefined
      ? undefined
      : smithy_enum_values(
          shapes,
          OPERATION_VALUE_TRANSFORM_SHAPE_ID,
          "operation value transform",
        )

  const contract = {
    item_id_bytes: integer_member(contract_trait, "itemIdBytes", "wireContract", 1),
    max_value_bytes: integer_member(contract_trait, "maxValueBytes", "wireContract", 1),
    opcodes,
    statuses,
    v1: wire_v1_contract(contract_trait.v1),
    ...(value_transforms === undefined
      ? {}
      : { operation_vocabularies: { value_transforms } }),
  }
  const operations = wire_operations(
    shapes,
    opcodes,
    statuses,
    value_transforms ?? OPERATION_VALUE_TRANSFORMS,
    strict_operations,
  )
  return operations === undefined ? contract : { ...contract, operations }
}

/** Loads the protocol Smithy AST from the model owned by this directory. */
export function smithy_wire_ast(): unknown {
  const smithy_executable = resolve_smithy_executable()
  const smithy_command =
    SMITHY_USE_SHELL && process.platform !== "win32"
      ? ["sh", smithy_executable, "ast", MODEL_DIRECTORY]
      : [smithy_executable, "ast", MODEL_DIRECTORY]
  const result = Bun.spawnSync(smithy_command, {
    cwd: PROTOCOL_DIRECTORY,
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

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function formatted_byte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`
}

function wire_name(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase()
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

function rust_string_literal(value: string): string {
  let literal = '"'
  for (const character of value) {
    switch (character) {
      case '"':
        literal += '\\"'
        break
      case "\\":
        literal += "\\\\"
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
      default: {
        const code_point = character.codePointAt(0) ?? 0
        literal +=
          code_point < 0x20
            ? `\\u{${code_point.toString(16)}}`
            : character
        break
      }
    }
  }
  return `${literal}"`
}

function rust_wire_enum(
  name: string,
  documentation: string,
  entries: readonly Wire_Entry[],
  unknown_variant: string,
): string {
  const variants = entries
    .map((entry) => `        ${entry.name} = ${formatted_byte(entry.value)},`)
    .join("\n")
  const all_variants = entries.map((entry) => `        Self::${entry.name},`).join("\n")
  const name_literals = entries
    .map((entry) => `        ${rust_string_literal(entry.text ?? wire_name(entry.name))},`)
    .join("\n")
  const names = entries
    .map(
      (entry) =>
        `        Self::${entry.name} => ${rust_string_literal(entry.text ?? wire_name(entry.name))},`,
    )
    .join("\n")
  return `wire_enum! {
    /// ${documentation}
    pub enum ${name} {
${variants}
    }
    unknown => ${unknown_variant}
}

impl ${name} {
    /// Number of values assigned by the Smithy ${name} contract.
    pub const COUNT: usize = ${entries.length};

    /// Every assigned Smithy ${name} value in wire-value order.
    pub const ALL: [Self; Self::COUNT] = [
${all_variants}
    ];

    /// Stable lowercase Smithy names in wire-value order.
    pub const NAMES: [&'static str; Self::COUNT] = [
${name_literals}
    ];

    /// Zero-based position in the Smithy value-order arrays.
    ///
    /// Wire values are intentionally allowed to be sparse. Callers that use
    /// an enum as an array index must use this generated position instead of
    /// the wire discriminant.
    pub const fn index(self) -> usize {
        match self {
${entries
  .map((entry, index) => `        Self::${entry.name} => ${index},`)
  .join("\n")}
        }
    }

    /// Stable lowercase Smithy name for this assigned value.
    pub const fn name(self) -> &'static str {
        match self {
${names}
        }
    }
}`
}

function rust_operation_contract(contract: Wire_Contract): string {
  const operations = contract.operations
  if (operations === undefined) return ""
  const modeled_value_transforms = operations.flatMap((operation) =>
    operation.contract.value_transform === undefined
      ? []
      : [operation.contract.value_transform])
  const value_transforms = contract.operation_vocabularies?.value_transforms ??
    (modeled_value_transforms.length === 0
      ? []
      : [...new Set(["identity", ...modeled_value_transforms])])
  const has_value_transforms = value_transforms.length > 0
  const status_variant = (status: string): string => {
    const entry = contract.statuses.find(
      (candidate) =>
        candidate.name === status ||
        candidate.text === status ||
        wire_name(candidate.name) === status,
    )
    if (entry === undefined) {
      throw new Error(`operation metadata references unknown status ${status}`)
    }
    return entry.name
  }
  const enum_variant = (value: string): string =>
    pascal_case(value)
  const status_slice = (statuses: readonly string[]): string =>
    `&[${statuses
      .map((status) => `Status::${status_variant(status)}`)
      .join(", ")}]`
  const metadata = operations
    .map(
      (operation) => `        Opcode::${operation.name} => OperationContract {
            scope: OperationScope::${enum_variant(operation.contract.scope)},
            request_kind: OperationRequestKind::${enum_variant(operation.contract.request_kind)},
            request_item_count: ${operation.contract.request_item_count},
            response_kind: OperationResponseKind::${enum_variant(operation.contract.response_kind)},
            response_value_count: ${operation.contract.response_value_count},
            retry_mode: OperationRetryMode::${enum_variant(operation.contract.retry_mode)},
${has_value_transforms
  ? `            value_transform: OperationValueTransform::${enum_variant(operation.contract.value_transform ?? "identity")},`
  : ""}
            success_statuses: ${status_slice(operation.contract.success_statuses)},
            error_statuses: ${status_slice(operation.contract.error_statuses)},
        },`,
    )
    .join("\n")
  return `/// Request scope declared by the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationScope {
    Global,
    Item,
    Namespace,
    NamespaceManagement,
}

/// Native request shape declared by the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationRequestKind {
    Empty,
    ApplicationValue,
    ScopedItem,
    ScopedNamespace,
    NamespaceOpen,
    NamespaceUpdatePolicy,
    NamespaceDelete,
}

/// Response payload shape declared by the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationResponseKind {
    Empty,
    Pong,
    ApplicationValue,
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

${has_value_transforms
  ? `/// Application-value transformation declared by the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationValueTransform {
${value_transforms.map((value) => `    ${enum_variant(value)},`).join("\n")}
}
`
  : ""}

/// Generated semantic metadata for one protocol operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationContract {
    pub scope: OperationScope,
    pub request_kind: OperationRequestKind,
    pub request_item_count: usize,
    pub response_kind: OperationResponseKind,
    pub response_value_count: usize,
    pub retry_mode: OperationRetryMode,
${has_value_transforms ? "    pub value_transform: OperationValueTransform,\n" : ""}
    pub success_statuses: &'static [Status],
    pub error_statuses: &'static [Status],
}

/// Returns the generated contract for a protocol operation.
pub const fn operation_contract(opcode: Opcode) -> OperationContract {
    match opcode {
${metadata}
    }
}
`
}

/**
 * Renders only the request metadata needed to find a v1 frame boundary.
 *
 * This is intentionally separate from the semantic operation contract below.
 * The protocol runtime must not inspect response meanings, retry policy, or
 * server behavior just to read a request from a stream. Those fields remain
 * available to generated client/server adapters, while the wire crate gets a
 * small, request-only descriptor.
 */
function rust_request_layout(contract: Wire_Contract): string {
  const operations = contract.operations
  if (operations === undefined) return ""
  const request_kind = (operation: Wire_Operation): string => {
    const { request_kind, request_value_count, response_kind } = operation.contract
    // `request_value_count` is present for all production Smithy ASTs. Keep
    // the response-kind fallback only for old unit fixtures that predate the
    // role count; it is never used by generated production output.
    if (
      request_kind === "scoped_item" &&
      (request_value_count ?? (response_kind === "set_outcome" ? 1 : 0)) > 0
    ) {
      return "Set"
    }
    if (request_kind === "scoped_item") return "Item"
    if (request_kind === "scoped_namespace") return "Namespace"
    return pascal_case(request_kind)
  }
  const step_expression = (operation: Wire_Operation): string => {
    const kind = request_kind(operation)
    const fixed = (bytes: string): string =>
      `WireRequestStep::Fixed { bytes: ${bytes} }`
    switch (kind) {
      case "Empty":
        return `[${fixed("OPCODE_BYTES")}]`
      case "ApplicationValue":
        return `[${fixed("OPCODE_BYTES")}, WireRequestStep::ValueLength]`
      case "Item":
        return `[
            ${fixed(
              "OPCODE_BYTES + NAMESPACE_ID_BYTES + ITEM_ID_BYTES * " +
                operation.contract.request_item_count,
            )},
        ]`
      case "Set":
        return `[
            ${fixed(
              "OPCODE_BYTES + NAMESPACE_ID_BYTES + SET_FLAGS_BYTES + ITEM_ID_BYTES",
            )},
            WireRequestStep::ConditionalVarUInt {
                selector_offset: OPCODE_BYTES + NAMESPACE_ID_BYTES,
                mask: SET_EXPIRATION_MASK,
                expected: SET_EXPLICIT_TTL_BITS,
            },
            WireRequestStep::ValueLength,
        ]`
      case "Namespace":
        return `[${fixed("OPCODE_BYTES + NAMESPACE_ID_BYTES")}]`
      case "NamespaceOpen":
        return `[
            ${fixed("OPCODE_BYTES + OPEN_FLAGS_BYTES + NAMESPACE_NAME_LENGTH_BYTES")},
            WireRequestStep::ByteLength,
            WireRequestStep::ConditionalPolicy {
                selector_offset: OPCODE_BYTES,
                mask: OPEN_CREATE_IF_MISSING,
                expected: OPEN_CREATE_IF_MISSING,
            },
        ]`
      case "NamespaceUpdatePolicy":
        return `[
            ${fixed("OPCODE_BYTES + NAMESPACE_ID_BYTES + NAMESPACE_REVISION_BYTES")},
            WireRequestStep::Policy,
        ]`
      case "NamespaceDelete":
        return `[
            ${fixed(
              "OPCODE_BYTES + DELETE_FLAGS_BYTES + NAMESPACE_ID_BYTES + NAMESPACE_REVISION_BYTES",
            )},
        ]`
      default:
        return `[]`
    }
  }
  const metadata = operations
    .map(
      (operation) => `        Opcode::${operation.name} => WireRequestLayout {
            kind: WireRequestLayoutKind::${request_kind(operation)},
            item_id_count: ${operation.contract.request_item_count},
            steps: &${step_expression(operation)},
        },`,
    )
    .join("\n")
  return `/// Wire-only request layouts used to delimit protocol v1 frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireRequestLayoutKind {
    Empty,
    ApplicationValue,
    Item,
    Set,
    Namespace,
    NamespaceOpen,
    NamespaceUpdatePolicy,
    NamespaceDelete,
}

/// Primitive request parsing steps generated from the wire layout.
///
/// These steps describe only byte consumption. They do not assign a meaning
/// to namespace IDs, item IDs, flags, policies, or response behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireRequestStep {
    Fixed { bytes: usize },
    ValueLength,
    ConditionalVarUInt {
        selector_offset: usize,
        mask: u8,
        expected: u8,
    },
    ByteLength,
    Policy,
    ConditionalPolicy {
        selector_offset: usize,
        mask: u8,
        expected: u8,
    },
}

/// Generated request metadata used only to delimit protocol v1 frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireRequestLayout {
    pub kind: WireRequestLayoutKind,
    pub item_id_count: usize,
    pub steps: &'static [WireRequestStep],
}

/// Returns the wire-level request layout for one assigned opcode.
pub const fn wire_request_layout(opcode: Opcode) -> WireRequestLayout {
    match opcode {
${metadata}
    }
}
`
}

/** Renders protocol v1 Rust definitions without client-only declarations. */
export function render_rust_wire(contract: Wire_Contract): string {
  const v1 = contract.v1
  return `// Generated from the OpenKache Smithy wire contract. Do not edit.

/// QUIC application protocol identifier for wire protocol version 1.
pub const ALPN: &[u8] = ${rust_byte_string_literal(v1.alpn)};
/// Bytes occupied by the request opcode.
pub const OPCODE_BYTES: usize = ${formatted_decimal(v1.opcode_bytes)};
/// Bytes occupied by the response status.
pub const STATUS_BYTES: usize = ${formatted_decimal(v1.status_bytes)};
/// Bytes before the variable-length request lengths.
pub const REQUEST_FIXED_BYTES: usize = ${formatted_decimal(v1.request_fixed_bytes)};
/// Bytes before the variable-length response payload length.
pub const RESPONSE_FIXED_BYTES: usize = ${formatted_decimal(v1.response_fixed_bytes)};
/// Minimum bytes in one canonical unsigned \`vu128\`.
pub const MIN_VARUINT_BYTES: usize = ${formatted_decimal(v1.min_varuint_bytes)};
/// Maximum bytes in one unsigned \`vu128\` accepted by this protocol.
pub const MAX_VARUINT_BYTES: usize = ${formatted_decimal(v1.max_varuint_bytes)};
/// Bytes in every canonical item ID carried by the protocol.
pub const ITEM_ID_BYTES: usize = ${formatted_decimal(contract.item_id_bytes)};
/// Absolute value or response payload ceiling representable by protocol v1.
pub const MAX_VALUE_BYTES: usize = ${formatted_decimal(contract.max_value_bytes)};
/// Bytes in every namespace ID and namespace revision.
pub const NAMESPACE_ID_BYTES: usize = ${formatted_decimal(v1.namespace_id_bytes)};
pub const NAMESPACE_REVISION_BYTES: usize = ${formatted_decimal(v1.namespace_revision_bytes)};
/// Bytes in the fixed namespace name length field.
pub const NAMESPACE_NAME_LENGTH_BYTES: usize = ${formatted_decimal(v1.namespace_name_length_bytes)};
/// Maximum UTF-8 octets in a namespace name.
pub const NAMESPACE_NAME_MAX_BYTES: usize = ${formatted_decimal(v1.namespace_name_max_bytes)};

/// Width of the SET flags field.
pub const SET_FLAGS_BYTES: usize = ${formatted_decimal(v1.set_flags_bytes)};
pub const SET_CONDITION_MASK: u8 = ${formatted_byte(v1.set_condition_mask)};
pub const SET_CONDITION_ANY_BITS: u8 = ${formatted_byte(v1.set_condition_any_bits)};
pub const SET_IF_ABSENT_BITS: u8 = ${formatted_byte(v1.set_if_absent_flag)};
pub const SET_IF_PRESENT_BITS: u8 = ${formatted_byte(v1.set_if_present_flag)};
pub const SET_CONDITION_RESERVED_BITS: u8 = ${formatted_byte(v1.set_condition_reserved_bits)};
pub const SET_EXPIRATION_MASK: u8 = ${formatted_byte(v1.set_expiration_mask)};
pub const SET_INHERIT_EXPIRATION_BITS: u8 = ${formatted_byte(v1.set_inherit_expiration_bits)};
pub const SET_NO_EXPIRY_BITS: u8 = ${formatted_byte(v1.set_no_expiry_bits)};
pub const SET_EXPLICIT_TTL_BITS: u8 = ${formatted_byte(v1.set_ttl_flag)};
pub const SET_EXPIRATION_RESERVED_BITS: u8 = ${formatted_byte(v1.set_expiration_reserved_bits)};
pub const SET_EVICTION_MASK: u8 = ${formatted_byte(v1.set_eviction_mask)};
pub const SET_INHERIT_EVICTION_BITS: u8 = ${formatted_byte(v1.set_inherit_eviction_bits)};
pub const SET_EVICTABLE_BITS: u8 = ${formatted_byte(v1.set_evictable_bits)};
pub const SET_EVICTION_PROTECTED_BITS: u8 = ${formatted_byte(v1.set_eviction_protected_bits)};
pub const SET_EVICTION_RESERVED_BITS: u8 = ${formatted_byte(v1.set_eviction_reserved_bits)};
pub const SET_RESERVED_MASK: u8 = ${formatted_byte(v1.set_reserved_mask)};

/// Namespace-open flag fields.
pub const OPEN_FLAGS_BYTES: usize = ${formatted_decimal(v1.open_flags_bytes)};
pub const OPEN_CREATE_IF_MISSING: u8 = ${formatted_byte(v1.open_create_if_missing_flag)};
pub const OPEN_RESERVED_MASK: u8 = ${formatted_byte(v1.open_reserved_mask)};
/// Namespace-delete flag fields.
pub const DELETE_FLAGS_BYTES: usize = ${formatted_decimal(v1.delete_flags_bytes)};
pub const DELETE_IF_EMPTY: u8 = ${formatted_byte(v1.delete_if_empty_bits)};
pub const DELETE_MODE_MASK: u8 = ${formatted_byte(v1.delete_mode_mask)};
pub const DELETE_RESERVED_MASK: u8 = ${formatted_byte(v1.delete_reserved_mask)};

/// Namespace-policy flag fields.
pub const POLICY_FLAGS_BYTES: usize = ${formatted_decimal(v1.policy_flags_bytes)};
pub const POLICY_DEFAULT_EXPIRATION_MASK: u8 = ${formatted_byte(v1.policy_default_expiration_mask)};
pub const POLICY_NO_EXPIRY: u8 = ${formatted_byte(v1.policy_no_expiry_bits)};
pub const POLICY_FIXED_TTL: u8 = ${formatted_byte(v1.policy_fixed_ttl_bits)};
pub const POLICY_DEFAULT_EXPIRATION_RESERVED_BITS: u8 = ${formatted_byte(v1.policy_default_expiration_reserved_bits)};
pub const POLICY_EXPIRATION_OVERRIDE: u8 = ${formatted_byte(v1.policy_expiration_override_flag)};
pub const POLICY_EVICTION_PROTECTED: u8 = ${formatted_byte(v1.policy_eviction_protected_flag)};
pub const POLICY_EVICTION_OVERRIDE: u8 = ${formatted_byte(v1.policy_eviction_override_flag)};
pub const POLICY_RESERVED_MASK: u8 = ${formatted_byte(v1.policy_reserved_mask)};

/// First assigned status value reserved for errors.
pub const ERROR_STATUS_MINIMUM: u8 = ${formatted_byte(v1.error_status_minimum)};

${rust_wire_enum("Opcode", "Operations supported by protocol v1.", contract.opcodes, "UnknownOpcode")}

${rust_wire_enum("Status", "Status returned in every protocol response.", contract.statuses, "UnknownStatus")}

${rust_request_layout(contract)}
${rust_operation_contract(contract)}
`
}
