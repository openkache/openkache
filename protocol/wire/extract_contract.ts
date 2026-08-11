/** Smithy extraction for the neutral OpenKache wire contract. */

import { existsSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import {
  MAX_GENERATED_NESTED_CODEC_DEPTH,
  MAX_GENERATED_NESTED_CODEC_ENTRIES,
  MAX_GENERATED_OPERATION_FIELDS,
  WIRE_CODEC_DESCRIPTORS,
  WIRE_CODEC_NAMES,
  WIRE_REQUEST_FRAMINGS,
  WIRE_RESPONSE_FRAMINGS,
  type Wire_Codec_Name,
  type Wire_Contract,
  type Wire_Contract_Adapter,
  type Wire_Entry,
  type Wire_Model_Request_Framing,
  type Wire_Operation,
  type Wire_Operation_Contract,
  type Wire_Operation_Descriptor,
  type Wire_Operation_Field_Plan,
  type Wire_Request_Step,
  type Wire_Response_Framing,
  type Wire_V1_Contract,
} from "../wire_types"
import { fixed_field_width } from "../wire_layout"
import {
  array_member,
  ensure_wire_codec_name,
  integer_member,
  object_member,
  object_value,
  optional_boolean_member,
  optional_enum_value,
  optional_integer_member,
  optional_object_member,
  optional_string_member,
  shape_type,
  string_member,
  trait_value,
  unique_wire_values,
  type Json_Object,
} from "./validate_contract"

/**
 * Computes an admission bound from the selected layout and codec widths.
 *
 * The protocol still enforces one aggregate value ceiling. This tighter
 * shape-derived bound prevents a fixed tuple or a small field sequence from
 * reserving the maximum value buffer for every in-flight request.
 */
const PROTOCOL_DIRECTORY = dirname(dirname(fileURLToPath(import.meta.url)))
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
const WIRE_CODEC_TRAIT_ID = "openkache.protocol#wireCodec"
const GENERIC_OPERATION_CONTRACT_MEMBERS = [
  "requestFraming",
  "requestWire",
  "responseFraming",
  "opaqueAggregate",
  "successStatuses",
  "errorStatuses",
] as const

function wire_name(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase()
}

/**
 * Keeps operation-contract extensions available to an adapter without making
 * the generic wire contract enumerate every future semantic trait member.
 *
 * Generic fields are removed from this opaque map; every other member is
 * namespaced by the trait ID and remains uninterpreted until an API or
 * compatibility adapter opts into it.
 */
function operation_extensions(
  contract: Json_Object,
  operation_location: string,
  adapter: Wire_Contract_Adapter | undefined,
): Readonly<Record<string, unknown>> | undefined {
  const extensions: Record<string, unknown> = {}
  for (const [member, value] of Object.entries(contract)) {
    if (
      value !== undefined &&
      !GENERIC_OPERATION_CONTRACT_MEMBERS.includes(
        member as (typeof GENERIC_OPERATION_CONTRACT_MEMBERS)[number],
      )
    ) {
      extensions[`${OPERATION_CONTRACT_TRAIT_ID}.${member}`] = value
    }
  }
  const adapter_extensions = adapter?.extract_extensions?.(
    contract,
    operation_location,
  )
  if (adapter_extensions !== undefined) {
    Object.assign(extensions, adapter_extensions)
  }
  return Object.keys(extensions).length === 0 ? undefined : extensions
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

function wire_v1_contract(value: unknown): Wire_V1_Contract {
  const contract = object_value(value, `${WIRE_CONTRACT_TRAIT_ID}.v1`)
  const optional_value_length_bytes = optional_integer_member(
    contract,
    "optionalValueLengthBytes",
    "wireContract.v1",
    1,
    0xff,
  )
  const optional_value_missing = optional_integer_member(
    contract,
    "optionalValueMissing",
    "wireContract.v1",
    0,
    Number.MAX_SAFE_INTEGER,
  )
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
    ...(optional_value_length_bytes === undefined
      ? {}
      : { optional_value_length_bytes }),
    ...(optional_value_missing === undefined ? {} : { optional_value_missing }),
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
    (v1.optional_value_length_bytes !== undefined &&
      v1.optional_value_length_bytes !== 4) ||
    (v1.optional_value_missing !== undefined &&
      v1.optional_value_missing !== 0xffff_ffff)
  ) {
    throw new Error(
      "wire v1 optional-value framing must use four big-endian length bytes and 0xffffffff as the missing sentinel",
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

function operation_shape_field_plan(
  shapes: Json_Object,
  operation_shape: Json_Object,
  operation_target: string,
  direction: "input" | "output",
): readonly Wire_Operation_Field_Plan[] {
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
  type Nested_Codec = {
    readonly name: string
    readonly width?: number
    readonly enum_values?: readonly string[]
    readonly union_tags?: readonly number[]
  }
  const enum_values_for_shape = (target: string): readonly string[] | undefined => {
    const shape = shapes[target]
    if (shape === undefined) return undefined
    if (
      shape_type(
        object_value(shape, `Smithy AST.shapes.${target}`),
        `Smithy AST.shapes.${target}`,
      ) !== "enum"
    ) return undefined
    const members = optional_object_member(
      object_value(shape, `Smithy AST.shapes.${target}`),
      "members",
      `Smithy AST.shapes.${target}`,
    )
    return Object.entries(members ?? {}).map(([member_name, member_value]) => {
      const traits = optional_object_member(
        object_value(member_value, `${target}.${member_name}`),
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
  }
  const union_tags_for_shape = (
    target: string,
    members: Json_Object | undefined,
  ): readonly number[] => {
    const count = Object.keys(members ?? {}).length
    // Union tags are encoded as one byte by the shared container codec.
    // Reject an unrepresentable Smithy shape during generation instead of
    // emitting a descriptor that truncates or fails in a later language.
    if (count > 0x100) {
      throw new Error(
        `${target} union has ${count} members; protocol union tags support at most 256`,
      )
    }
    return Object.keys(members ?? {}).map((_, index) => index)
  }
  const nested_codec_descriptors = (
    target: string,
    ancestors: ReadonlySet<string> = new Set(),
    infer_child_codecs = false,
    depth = 0,
  ): readonly Nested_Codec[] => {
    if (depth > MAX_GENERATED_NESTED_CODEC_DEPTH) {
      throw new Error(
        `${operation_target}.${direction} nested codec depth exceeds ` +
          `${MAX_GENERATED_NESTED_CODEC_DEPTH}`,
      )
    }
    if (ancestors.has(target)) {
      throw new Error(`${operation_target}.${direction} shape cycle through ${target}`)
    }
    const shape = shapes[target]
    if (shape === undefined) return []
    const next_ancestors = new Set(ancestors).add(target)
    const traits = optional_object_member(
      object_value(shape, `Smithy AST.shapes.${target}`),
      "traits",
      `Smithy AST.shapes.${target}`,
    )
    const codec = traits?.[WIRE_CODEC_TRAIT_ID]
    const type = shape_type(
      object_value(shape, `Smithy AST.shapes.${target}`),
      `Smithy AST.shapes.${target}`,
    )
    const explicit_codec = codec === undefined
      ? undefined
      : string_member(
          object_value(
            codec,
            `Smithy AST.shapes.${target}.${WIRE_CODEC_TRAIT_ID}`,
          ),
          "name",
          `Smithy AST.shapes.${target}.${WIRE_CODEC_TRAIT_ID}`,
        )
    const inferred_codec = infer_child_codecs
      ? ({
          blob: "raw_bytes",
          boolean: "bool_u8",
          double: "f64_be",
          enum: "enum",
          integer: "i32_be",
          list: "list",
          long: "u64_be",
          map: "map",
          string: "utf8",
          union: "union",
        } as const)[type as
          | "blob"
          | "boolean"
          | "double"
          | "enum"
          | "integer"
          | "list"
          | "long"
          | "map"
          | "string"
          | "union"]
      : undefined
    const codec_name = explicit_codec ?? inferred_codec
    const names: Nested_Codec[] = []
    if (codec_name !== undefined) {
      ensure_wire_codec_name(
        codec_name,
        `${operation_target}.${direction} nested shape ${target}`,
      )
      const members = optional_object_member(
        object_value(shape, `Smithy AST.shapes.${target}`),
        "members",
        `Smithy AST.shapes.${target}`,
      )
      names.push({
        name: codec_name,
        ...(() => {
          const descriptor =
            WIRE_CODEC_DESCRIPTORS[codec_name as Wire_Codec_Name]
          const explicit_width = codec === undefined
            ? undefined
            : optional_integer_member(
              object_value(codec, `Smithy AST.shapes.${target}.${WIRE_CODEC_TRAIT_ID}`),
              "width",
              `Smithy AST.shapes.${target}.${WIRE_CODEC_TRAIT_ID}`,
              1,
            )
          const width = explicit_width ??
            (descriptor?.width === "fixed" ? descriptor.min_width : undefined)
          return width === undefined ? {} : { width }
        })(),
        ...(codec_name === "enum"
          ? { enum_values: enum_values_for_shape(target) ?? [] }
          : {}),
        ...(codec_name === "union"
          ? { union_tags: union_tags_for_shape(target, members) }
          : {}),
      })
    }
    const child_targets: string[] = []
    if (type === "list") {
      const member = optional_object_member(
        object_value(shape, `Smithy AST.shapes.${target}`),
        "member",
        `Smithy AST.shapes.${target}`,
      )
      const child = member?.["target"]
      if (typeof child === "string") child_targets.push(child)
    } else if (type === "map") {
      const key = optional_object_member(
        object_value(shape, `Smithy AST.shapes.${target}`),
        "key",
        `Smithy AST.shapes.${target}`,
      )?.["target"]
      const value = optional_object_member(
        object_value(shape, `Smithy AST.shapes.${target}`),
        "value",
        `Smithy AST.shapes.${target}`,
      )?.["target"]
      if (typeof key === "string") child_targets.push(key)
      if (typeof value === "string") child_targets.push(value)
    } else if (type === "union" || type === "structure") {
      const members = optional_object_member(
        object_value(shape, `Smithy AST.shapes.${target}`),
        "members",
        `Smithy AST.shapes.${target}`,
      )
      for (const member of Object.values(members ?? {})) {
        const child = object_value(member, `Smithy AST.shapes.${target}`).target
        if (typeof child === "string") child_targets.push(child)
      }
    }
    const infer_grandchildren =
      codec_name === "list" || codec_name === "map" || codec_name === "union"
    for (const child of child_targets) {
      names.push(...nested_codec_descriptors(
        child,
        next_ancestors,
        infer_grandchildren,
        depth + 1,
      ))
      if (names.length > MAX_GENERATED_NESTED_CODEC_ENTRIES) {
        throw new Error(
          `${operation_target}.${direction} nested codec metadata exceeds ` +
            `${MAX_GENERATED_NESTED_CODEC_ENTRIES} entries`,
        )
      }
    }
    return names
  }
  const fields: Wire_Operation_Field_Plan[] = []
  const visit = (
    target: string,
    path: readonly string[],
    ancestors: ReadonlySet<string>,
    required_parent: boolean,
  ): void => {
    if (ancestors.has(target)) {
      throw new Error(`${operation_target}.${direction} shape cycle through ${target}`)
    }
    const next_ancestors = new Set(ancestors).add(target)
    const current = object_member(shapes, target, "Smithy AST.shapes")
    const members = object_member(current, "members", target)
    for (const [member_name, value] of Object.entries(members)) {
      const member = object_value(value, `${target}.${member_name}`)
      const traits = optional_object_member(member, "traits", `${target}.${member_name}`)
      const field = traits?.[OPERATION_FIELD_TRAIT_ID]
      if (field !== undefined) {
        const role = string_member(
          object_value(field, `${target}.${member_name}.${OPERATION_FIELD_TRAIT_ID}`),
          "role",
          `${target}.${member_name}.${OPERATION_FIELD_TRAIT_ID}`,
        )
        const member_target = string_member(
          member,
          "target",
          `${target}.${member_name}`,
        )
        const codecs: string[] = []
        const codec_widths: (number | undefined)[] = []
        const member_shape = shapes[member_target]
        const member_shape_traits = member_shape === undefined
          ? undefined
          : optional_object_member(
            object_value(member_shape, `Smithy AST.shapes.${member_target}`),
            "traits",
            `Smithy AST.shapes.${member_target}`,
          )
        // A codec may be declared on the operation member or on the target
        // shape. Reusable list/map/enum/union shapes commonly carry the
        // declaration on the shape itself, so preserve it in the generated
        // server descriptor as well.
        const codec = traits?.[WIRE_CODEC_TRAIT_ID] ??
          member_shape_traits?.[WIRE_CODEC_TRAIT_ID]
        if (codec !== undefined) {
          const codec_location = traits?.[WIRE_CODEC_TRAIT_ID] !== undefined
            ? `${target}.${member_name}.${WIRE_CODEC_TRAIT_ID}`
            : `Smithy AST.shapes.${member_target}.${WIRE_CODEC_TRAIT_ID}`
          const codec_name = string_member(
            object_value(codec, codec_location),
            "name",
            codec_location,
          )
          ensure_wire_codec_name(codec_name, codec_location)
          codecs.push(codec_name)
          codec_widths.push(
            optional_integer_member(
              object_value(codec, codec_location),
              "width",
              codec_location,
              1,
            ),
          )
        }
        const enum_values = !codecs.includes("enum") ||
            member_shape === undefined ||
            shape_type(
              object_value(member_shape, `Smithy AST.shapes.${member_target}`),
              `Smithy AST.shapes.${member_target}`,
            ) !== "enum"
          ? undefined
          : Object.entries(
              object_member(
                object_value(member_shape, `Smithy AST.shapes.${member_target}`),
                "members",
                `Smithy AST.shapes.${member_target}`,
              ),
            ).map(([member_name, member_value]) => {
              const traits = optional_object_member(
                object_value(member_value, `${member_target}.${member_name}`),
                "traits",
                `${member_target}.${member_name}`,
              )
              return traits?.["smithy.api#enumValue"] === undefined
                ? member_name
                : string_member(
                    traits,
                    "smithy.api#enumValue",
                    `${member_target}.${member_name}.traits`,
                )
            })
        const nested_descriptors = nested_codec_descriptors(
          member_target,
          new Set(),
          codecs.some((codec) => codec === "list" || codec === "map" || codec === "union"),
        )
        const nested_codecs = nested_descriptors[0]?.name === codecs[0]
          ? nested_descriptors.slice(1)
          : nested_descriptors
        const nested_enum_values = nested_codecs.map(
          (nested) => nested.enum_values ?? [],
        )
        const nested_widths = nested_codecs.map((nested) => nested.width)
        const union_tags = codecs.includes("union") && member_shape !== undefined
          ? union_tags_for_shape(
              member_target,
              optional_object_member(
                object_value(member_shape, `Smithy AST.shapes.${member_target}`),
                "members",
                `Smithy AST.shapes.${member_target}`,
              ),
            )
          : undefined
        const nested_union_tags = nested_codecs.map(
          (nested) => nested.union_tags ?? [],
        )
        const required = required_parent && traits?.["smithy.api#required"] !== undefined
        const field_plan: Wire_Operation_Field_Plan = {
          index: fields.length,
          ...(codecs.length === 0 ? {} : { codecs }),
          ...(codec_widths.some((width) => width !== undefined)
            ? { codec_widths }
            : {}),
          ...(enum_values === undefined ? {} : { enum_values }),
          ...(union_tags === undefined ? {} : { union_tags }),
          ...(nested_codecs.length === 0 ? {} : {
            nested_codecs: nested_codecs.map((nested) => nested.name),
            nested_widths,
            nested_enum_values,
            nested_union_tags,
          }),
          path: [...path, member_name],
          required,
          role,
          shape: shape_name(member_target),
        }
        if (fields.length >= MAX_GENERATED_OPERATION_FIELDS) {
          throw new Error(
            `${operation_target}.${direction} operation field plan exceeds ` +
              `${MAX_GENERATED_OPERATION_FIELDS} fields; use a bounded/streaming shape`,
          )
        }
        const encoded_width = fixed_field_width(field_plan)
        fields.push({
          ...field_plan,
          ...(encoded_width === undefined ? {} : { encoded_width }),
        })
      }
      const nested_target = member["target"]
      if (typeof nested_target === "string") {
        const nested = shapes[nested_target]
        if (
          nested !== undefined &&
          shape_type(
            object_value(nested, `Smithy AST.shapes.${nested_target}`),
            `Smithy AST.shapes.${nested_target}`,
          ) === "structure"
        ) {
          visit(
            nested_target,
            [...path, member_name],
            next_ancestors,
            required_parent && traits?.["smithy.api#required"] !== undefined,
          )
        }
      }
    }
  }
  visit(shape_target, [], new Set(), true)
  return fields
}

function request_wire_plan(
  contract: Json_Object,
  fields: readonly Wire_Operation_Field_Plan[],
  operation_location: string,
): readonly Wire_Request_Step[] | undefined {
  const raw = contract.requestWire
  if (raw === undefined) return undefined
  if (!Array.isArray(raw) || raw.length === 0) {
    throw new Error(`${operation_location}.requestWire must be a non-empty array`)
  }
  const by_path = new Map(fields.map((field) => [field.path.join("."), field]))
  const resolve_field = (name: string, location: string): Wire_Operation_Field_Plan => {
    const field = by_path.get(name)
    if (field === undefined) {
      throw new Error(`${location} references unknown request field ${JSON.stringify(name)}`)
    }
    return field
  }
  const validate_symbolic_value = (
    field: Wire_Operation_Field_Plan,
    value: string,
    location: string,
  ): void => {
    const allowed = field.shape === "Boolean"
      ? ["false", "true"]
      : field.enum_values
    if (allowed !== undefined && !allowed.includes(value)) {
      throw new Error(
        `${location} must be one of the modeled values: ${allowed.join(", ")}`,
      )
    }
  }
  const parse_steps = (
    values: readonly unknown[],
    location: string,
    depth = 0,
  ): readonly Wire_Request_Step[] => {
    if (depth > MAX_GENERATED_NESTED_CODEC_DEPTH) {
      throw new Error(`${location} exceeds the generated request-wire nesting bound`)
    }
    return values.map((raw_step, index) => {
      const step_location = `${location}[${index}]`
      const step = object_value(raw_step, step_location)
      const entries = Object.entries(step)
      if (entries.length !== 1) {
        throw new Error(`${step_location} must select exactly one request-wire primitive`)
      }
      const [kind, raw_value] = entries[0]!
      const value = object_value(raw_value, `${step_location}.${kind}`)
      switch (kind) {
        case "fixedField": {
          const name = string_member(value, "field", `${step_location}.${kind}`)
          const field = resolve_field(name, `${step_location}.${kind}.field`)
          const bytes = integer_member(value, "bytes", `${step_location}.${kind}`, 1)
          if (field.encoded_width !== undefined && field.encoded_width !== bytes) {
            throw new Error(
              `${step_location}.${kind}.bytes must match ${name}'s encoded width ` +
                `${field.encoded_width}`,
            )
          }
          return { kind: "fixed_field", field: field.index, bytes }
        }
        case "packed": {
          const raw_fields = array_member(value, "fields", `${step_location}.${kind}`)
          if (raw_fields.length === 0) {
            throw new Error(`${step_location}.${kind}.fields must not be empty`)
          }
          let occupied = 0
          const packed_fields = raw_fields.map((raw_field, field_index) => {
            const field_location = `${step_location}.${kind}.fields[${field_index}]`
            const packed = object_value(raw_field, field_location)
            const name = string_member(packed, "field", field_location)
            const field = resolve_field(name, `${field_location}.field`)
            const mask = integer_member(packed, "mask", field_location, 1, 0xff)
            if ((occupied & mask) !== 0) {
              throw new Error(`${field_location}.mask overlaps another packed field`)
            }
            occupied |= mask
            const raw_values = array_member(packed, "values", field_location)
            if (raw_values.length === 0) {
              throw new Error(`${field_location}.values must not be empty`)
            }
            const seen_values = new Set<string>()
            const seen_bits = new Set<number>()
            const values = raw_values.map((raw_mapping, mapping_index) => {
              const mapping_location = `${field_location}.values[${mapping_index}]`
              const mapping = object_value(raw_mapping, mapping_location)
              const symbolic = string_member(mapping, "value", mapping_location)
              validate_symbolic_value(field, symbolic, `${mapping_location}.value`)
              const bits = integer_member(mapping, "bits", mapping_location, 0, 0xff)
              if ((bits & ~mask) !== 0) {
                throw new Error(`${mapping_location}.bits exceed the packed field mask`)
              }
              if (seen_values.has(symbolic) || seen_bits.has(bits)) {
                throw new Error(`${mapping_location} duplicates a packed value or bit pattern`)
              }
              seen_values.add(symbolic)
              seen_bits.add(bits)
              return { value: symbolic, bits }
            })
            return { field: field.index, mask, values }
          })
          const reserved_mask = optional_integer_member(
            value,
            "reservedMask",
            `${step_location}.${kind}`,
            0,
            0xff,
          ) ?? 0
          const constant_bits = optional_integer_member(
            value,
            "constantBits",
            `${step_location}.${kind}`,
            0,
            0xff,
          ) ?? 0
          if ((reserved_mask & occupied) !== 0 || (constant_bits & occupied) !== 0) {
            throw new Error(
              `${step_location}.${kind} reserved/constant bits overlap packed fields`,
            )
          }
          if ((reserved_mask & constant_bits) !== 0) {
            throw new Error(
              `${step_location}.${kind} constant bits overlap the reserved mask`,
            )
          }
          return {
            kind: "packed",
            fields: packed_fields,
            reserved_mask,
            constant_bits,
          }
        }
        case "byteLengthField":
        case "varuintField": {
          const name = string_member(value, "field", `${step_location}.${kind}`)
          const field = resolve_field(name, `${step_location}.${kind}.field`)
          return kind === "byteLengthField"
            ? { kind: "byte_length_field", field: field.index }
            : { kind: "varuint_field", field: field.index }
        }
        case "conditional": {
          const name = string_member(value, "field", `${step_location}.${kind}`)
          const field = resolve_field(name, `${step_location}.${kind}.field`)
          const equals = string_member(value, "equals", `${step_location}.${kind}`)
          validate_symbolic_value(field, equals, `${step_location}.${kind}.equals`)
          const nested = array_member(value, "steps", `${step_location}.${kind}`)
          if (nested.length === 0) {
            throw new Error(`${step_location}.${kind}.steps must not be empty`)
          }
          return {
            kind: "conditional",
            field: field.index,
            equals,
            steps: parse_steps(nested, `${step_location}.${kind}.steps`, depth + 1),
          }
        }
        case "constant": {
          const hex = string_member(value, "hex", `${step_location}.${kind}`)
          if (!/^(?:[0-9a-f]{2})+$/.test(hex)) {
            throw new Error(
              `${step_location}.${kind}.hex must be non-empty lowercase hexadecimal bytes`,
            )
          }
          return {
            kind: "constant",
            bytes: Array.from(
              { length: hex.length / 2 },
              (_, byte) => Number.parseInt(hex.slice(byte * 2, byte * 2 + 2), 16),
            ),
          }
        }
        case "trailingField": {
          const name = string_member(value, "field", `${step_location}.${kind}`)
          const field = resolve_field(name, `${step_location}.${kind}.field`)
          const length = string_member(value, "length", `${step_location}.${kind}`)
          if (length !== "varuint") {
            throw new Error(`${step_location}.${kind}.length must be varuint`)
          }
          return { kind: "trailing_field", field: field.index, length }
        }
        default:
          throw new Error(`${step_location} selects unknown request-wire primitive ${kind}`)
      }
    })
  }
  const request_wire = parse_steps(raw, `${operation_location}.requestWire`)
  const covered = new Set<number>()
  const validate_steps = (
    steps: readonly Wire_Request_Step[],
    location: string,
    assigned: ReadonlySet<number>,
    packed_values: ReadonlyMap<number, ReadonlySet<string>>,
    allow_trailing: boolean,
  ): void => {
    const available = new Set(assigned)
    const mappings = new Map(packed_values)
    for (const [index, step] of steps.entries()) {
      const step_location = `${location}[${index}]`
      const assign = (field: number): void => {
        if (available.has(field)) {
          throw new Error(
            `${step_location} assigns request field ${fields[field]?.path.join(".")} more than once`,
          )
        }
        available.add(field)
        covered.add(field)
      }
      switch (step.kind) {
        case "fixed_field":
        case "byte_length_field":
        case "varuint_field":
          assign(step.field)
          break
        case "packed":
          for (const field of step.fields) {
            assign(field.field)
            mappings.set(
              field.field,
              new Set(field.values.map((value) => value.value)),
            )
          }
          break
        case "conditional": {
          if (!mappings.get(step.field)?.has(step.equals)) {
            throw new Error(
              `${step_location} condition must reference a preceding packed field mapping`,
            )
          }
          validate_steps(
            step.steps,
            `${step_location}.conditional.steps`,
            available,
            mappings,
            false,
          )
          break
        }
        case "constant":
          break
        case "trailing_field":
          if (!allow_trailing || index + 1 !== steps.length) {
            throw new Error(
              `${step_location} trailing field must be the final top-level requestWire step`,
            )
          }
          assign(step.field)
          break
      }
    }
  }
  validate_steps(
    request_wire,
    `${operation_location}.requestWire`,
    new Set(),
    new Map(),
    true,
  )

  const leaf_fields = fields.filter((field) =>
    !fields.some((candidate) =>
      candidate.path.length > field.path.length &&
      field.path.every((part, index) => candidate.path[index] === part)
    )
  )
  const missing = leaf_fields.find((field) => !covered.has(field.index))
  if (missing !== undefined) {
    throw new Error(
      `${operation_location}.requestWire does not encode leaf request field ` +
        missing.path.join("."),
    )
  }
  return request_wire
}

function operation_contract(
  shapes: Json_Object,
  shape: Json_Object,
  target: string,
  statuses: readonly Wire_Entry[],
  strict: boolean,
  adapter: Wire_Contract_Adapter | undefined,
): Wire_Operation_Contract | undefined {
  const traits = optional_object_member(shape, "traits", target)
  const value = traits?.[OPERATION_CONTRACT_TRAIT_ID]
  if (value === undefined) return undefined
  const contract = object_value(value, `${target}.traits.${OPERATION_CONTRACT_TRAIT_ID}`)
  const request_plan = operation_shape_field_plan(
    shapes,
    shape,
    target,
    "input",
  )
  const response_plan = operation_shape_field_plan(
    shapes,
    shape,
    target,
    "output",
  )
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
  const operation_location = `${target}.${OPERATION_CONTRACT_TRAIT_ID}`
  const request_wire = request_wire_plan(contract, request_plan, operation_location)
  const request_framing_value = optional_string_member(
    contract,
    "requestFraming",
    operation_location,
  )
  if (
    request_framing_value !== undefined &&
    !WIRE_REQUEST_FRAMINGS.includes(request_framing_value as Wire_Model_Request_Framing)
  ) {
    throw new Error(
      `${operation_location}.requestFraming must be empty, opaque, or ordered_fields`,
    )
  }
  const response_framing_value = optional_string_member(
    contract,
    "responseFraming",
    operation_location,
  )
  if (
    response_framing_value !== undefined &&
    !WIRE_RESPONSE_FRAMINGS.includes(response_framing_value as Wire_Response_Framing)
  ) {
    throw new Error(
      `${operation_location}.responseFraming must be empty, opaque, optional_values, or field_sequence`,
    )
  }
  if (
    strict &&
    request_framing_value === undefined
  ) {
    throw new Error(
      `${operation_location}.requestFraming is required for strict protocol generation`,
    )
  }
  if (strict && response_framing_value === undefined) {
    throw new Error(
      `${operation_location}.responseFraming is required for strict protocol generation`,
    )
  }
  const opaque_aggregate = optional_boolean_member(
    contract,
    "opaqueAggregate",
    operation_location,
  )
  const extensions = operation_extensions(contract, operation_location, adapter)
  const derived_contract = {
    error_statuses,
    request_plan,
    ...(request_wire === undefined ? {} : { request_wire }),
    ...(request_framing_value === undefined
      ? {}
      : { request_framing: request_framing_value as Wire_Model_Request_Framing }),
    ...(extensions === undefined ? {} : { extensions }),
    response_plan,
    ...(response_framing_value === undefined
      ? {}
      : { response_framing: response_framing_value as Wire_Response_Framing }),
    ...(opaque_aggregate === undefined ? {} : { opaque_aggregate }),
    success_statuses,
  }
  const request_framing =
    request_framing_value as Wire_Model_Request_Framing | undefined
  if (request_framing === undefined) {
    throw new Error(
      `${operation_location}.requestFraming is required for protocol generation`,
    )
  }
  // Unknown operation-contract members are preserved as opaque extensions.
  // Compatibility adapters may interpret their own namespaced values, while
  // the canonical generic framing remains independent of those projections.
  const response_framing: Wire_Response_Framing =
    response_framing_value === undefined
      ? (() => {
        /*
         * Strict operation contracts declare responseFraming explicitly.
         * Permissive AST fixtures still get a shape-neutral fallback based
         * only on field cardinality; response semantic labels are owned by
         * compatibility adapters and never select generic wire bytes.
         */
        if (response_plan.length === 0) return "empty"
        return response_plan.length === 1 ? "opaque" : "field_sequence"
      })()
      : response_framing_value as Wire_Response_Framing
  if (
    strict &&
    response_framing === "opaque" &&
    response_plan.length !== 1 &&
    opaque_aggregate !== true
  ) {
    throw new Error(
      `${operation_location}.responseFraming opaque requires exactly one modeled field`,
    )
  }
  if (
    opaque_aggregate === true &&
    response_framing !== "opaque"
  ) {
    throw new Error(
      `${operation_location}.opaqueAggregate requires responseFraming opaque`,
    )
  }
  if (opaque_aggregate === true && response_plan.length === 0) {
    throw new Error(
      `${operation_location}.opaqueAggregate requires at least one modeled response field`,
    )
  }
  const request_plan_count = request_plan.length
  switch (request_framing) {
    case "empty":
      if (request_plan_count !== 0) {
        throw new Error(
          `${operation_location}.requestFraming empty requires an empty request plan`,
        )
      }
      break
    case "opaque":
      if (request_plan_count !== 1) {
        throw new Error(
          `${operation_location}.requestFraming opaque requires exactly one modeled field`,
        )
      }
      break
    case "ordered_fields":
      if (request_plan_count === 0) {
        throw new Error(
          `${operation_location}.requestFraming ordered_fields requires at least one modeled field`,
        )
      }
      break
  }
  if (
    (response_framing_value === "optional_values" ||
      response_framing_value === "field_sequence") &&
    response_plan.length === 0
  ) {
    throw new Error(
      `${operation_location}.responseFraming ${response_framing_value} requires at least one modeled field`,
    )
  }
  const result: Wire_Operation_Contract = {
    ...derived_contract,
    request_framing,
    response_framing,
  }
  adapter?.validate_operation?.(result, operation_location)
  return result
}

function wire_operations(
  shapes: Json_Object,
  opcodes: readonly Wire_Entry[],
  statuses: readonly Wire_Entry[],
  strict: boolean,
  adapter: Wire_Contract_Adapter | undefined,
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
      strict,
      adapter,
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
export function extract_wire_contract(
  ast: unknown,
  strict_operations = false,
  adapter?: Wire_Contract_Adapter,
): Wire_Contract {
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
  const contract = {
    item_id_bytes: integer_member(contract_trait, "itemIdBytes", "wireContract", 1),
    max_value_bytes: integer_member(contract_trait, "maxValueBytes", "wireContract", 1),
    opcodes,
    statuses,
    v1: wire_v1_contract(contract_trait.v1),
  }
  const operations = wire_operations(
    shapes,
    opcodes,
    statuses,
    strict_operations,
    adapter,
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
