/** Smithy extraction for the neutral OpenKache wire contract. */

import {
  type Wire_Contract_Adapter,
  type Wire_Contract,
  type Wire_Entry,
  type Wire_V1_Contract,
} from "../wire_types"
import { validate_v1_contract } from "./validate_v1_contract"
import {
  array_member,
  integer_member,
  object_member,
  object_value,
  optional_enum_value,
  string_member,
  trait_value,
  unique_wire_values,
  type Json_Object,
} from "./validate_contract"
import { wire_operations } from "./extract_contract_operations"

/** Stable Smithy shape IDs and compatibility trait identifiers. */
const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
const OPCODE_SHAPE_ID = "openkache.protocol#Opcode"
const STATUS_SHAPE_ID = "openkache.protocol#Status"
const WIRE_CONTRACT_TRAIT_ID = "openkache.protocol#wireContract"
const WIRE_OPCODE_TRAIT_ID = "openkache.protocol#wireOpcode"
const WIRE_STATUS_TRAIT_ID = "openkache.protocol#wireStatus"

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

function wire_v1_contract(
  value: unknown,
  adapter: Wire_Contract_Adapter | undefined,
): Wire_V1_Contract {
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
    ...adapter?.extract_v1?.(contract, `${WIRE_CONTRACT_TRAIT_ID}.v1`),
  } satisfies Wire_V1_Contract
  return validate_v1_contract(v1)
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
      const text = optional_enum_value(member_shape, `${shape_id}.${name}`)
      return {
        name: pascal_case(name),
        ...(text === undefined ? {} : { text }),
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
    v1: wire_v1_contract(contract_trait.v1, adapter),
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

export { smithy_wire_ast } from "./smithy_ast"
