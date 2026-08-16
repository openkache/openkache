/** Explicit draft-v1 transport profile validation. */

import type { Wire_V1_Contract } from "./wire_types"
import {
  integer_member,
  object_value,
  optional_integer_member,
  string_member,
  unique_wire_values,
} from "./wire/validate_contract"

const WIRE_CONTRACT_TRAIT_ID = "openkache.protocol#wireContract"

/**
 * Extracts the unpublished draft-v1 transport profile.
 *
 * Generic extraction invokes this only through an explicit contract adapter.
 * Keeping the profile extractor separately exported lets adapters select the
 * existing compact contract without making it the generic extractor's default.
 */
export function extract_draft_v1_contract(value: unknown): Wire_V1_Contract {
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
