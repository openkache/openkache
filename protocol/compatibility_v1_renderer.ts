/** Rust rendering for compact policy and flag constants. */

import type { Wire_Contract } from "./wire_types"

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function formatted_byte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`
}

/**
 * Renders protocol-v1 flag and semantic constants for compatibility adapters.
 *
 * These constants are deliberately absent from the generic server contract.
 * They describe the historical namespace/item/SET projection and therefore
 * belong to the adapter that interprets those bytes.
 *
 * @param contract - Validated Smithy wire contract containing protocol-v1 values.
 * @returns Rust source for the protocol-v1 semantic constants.
 */
export function render_rust_semantic_constants(contract: Wire_Contract): string {
  const v1 = contract.v1
  return `/// Maximum UTF-8 octets accepted in a namespace name.
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
`
}

/**
 * Renders compact policy and flag constants.
 *
 * Request layouts, numeric field indexes, and frame bounds are generated from
 * the canonical operation contract. This adapter output contains no operation
 * names, route families, or duplicate field tables.
 *
 * @param contract - Validated Smithy wire contract containing compact values.
 * @returns Rust source for the compact policy and flag constants.
 */
export function render_rust_compatibility_contract(contract: Wire_Contract): string {
  return render_rust_semantic_constants(contract)
}
