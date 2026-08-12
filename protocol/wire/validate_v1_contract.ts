/** Validation for the historical protocol-v1 wire contract. */

import { unique_wire_values } from "./validate_contract"
import type { Wire_V1_Contract } from "../wire_types"

const WIRE_CONTRACT_TRAIT_ID = "openkache.protocol#wireContract"

/** Validates fixed v1 widths, masks, sentinels, and reserved values. */
export function validate_v1_contract(v1: Wire_V1_Contract): Wire_V1_Contract {
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
