/** Rust rendering for the historical protocol-v1 compatibility projection. */

import type {
  Wire_Contract,
  Wire_Operation_Field_Plan,
  Wire_Operation,
} from "./wire_types"
import {
  PROTOCOL_V1_COMPACT_ROUTE_EXTENSION,
} from "./compat_v1_types"

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

function rust_const_identifier(identifier: string): string {
  let value = wire_name(identifier)
    .replace(/[^a-z0-9_]/g, "_")
    .replace(/^([0-9])/, "_$1")
    .toUpperCase()
  if (value.length === 0) value = "_FIELD"
  return value
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
        literal += code_point < 0x20
          ? `\\u{${code_point.toString(16)}}`
          : character
      }
    }
  }
  return `${literal}"`
}

function compact_route(
  operation: Wire_Operation,
): string | undefined {
  const value = operation.contract.extensions?.[PROTOCOL_V1_COMPACT_ROUTE_EXTENSION]
  return typeof value === "string" ? value : undefined
}

/**
 * Computes the receive bound contributed by protocol-v1 compatibility
 * prefixes.
 *
 * The generic server renderer must not know namespace/item/SET byte widths.
 * Keeping this conservative bound in the compatibility renderer lets the
 * server composition layer take the maximum of the generic and adapter-owned
 * limits without teaching generic framing about a legacy route.
 */
export function compatibility_request_frame_bound(
  contract: Wire_Contract,
): number {
  const operations = contract.operations?.filter(
    (operation) => compact_route(operation) !== undefined,
  ) ?? []
  if (operations.length === 0) return 0

  const v1 = contract.v1
  const policy_bytes = v1.policy_flags_bytes + v1.max_varuint_bytes
  const item_count = Math.max(
    1,
    ...operations.map((operation) =>
      (operation.contract.request_plan ?? []).filter(
        (field) => field.role === "item_id",
      ).length
    ),
  )
  const item_prefix = v1.opcode_bytes +
    v1.namespace_id_bytes +
    contract.item_id_bytes * item_count +
    v1.set_flags_bytes +
    v1.max_varuint_bytes * 2
  const namespace_open_prefix = v1.opcode_bytes +
    v1.open_flags_bytes +
    v1.namespace_name_length_bytes +
    v1.namespace_name_max_bytes +
    policy_bytes
  const namespace_delete_prefix = v1.opcode_bytes +
    v1.delete_flags_bytes +
    v1.namespace_id_bytes +
    v1.namespace_revision_bytes
  return Math.max(
    item_prefix,
    namespace_open_prefix,
    namespace_delete_prefix,
  ) + contract.max_value_bytes
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
 * Renders the generated protocol-v1 route and field-index adapter artifact.
 *
 * Generic operation metadata intentionally has no compact-route enum or opcode
 * projection. The protocol crate includes this output only from `compat_v1`,
 * so adding a generic API cannot change the generic contract module's type
 * surface.
 *
 * @param contract - Validated Smithy wire contract containing v1 projections.
 * @returns Rust source for the compatibility-only generated module.
 */
export function render_rust_compatibility_contract(contract: Wire_Contract): string {
  const operations = contract.operations
  if (operations === undefined) {
    return `/// No protocol-v1 compatibility operations are present in this
/// permissive contract fixture.
pub const MAX_COMPATIBILITY_REQUEST_FRAME_BYTES: usize = 0;
`
  }
  const compatibility_operations = operations.filter(
    (operation) => compact_route(operation) !== undefined,
  )
  const role_names = [
    ...new Set(
      compatibility_operations.flatMap((operation) => [
        ...(operation.contract.request_plan ?? []),
        ...(operation.contract.response_plan ?? []),
      ]).map((field) => field.role),
    ),
  ]
  const role_variant_names = new Map<string, string>()
  const used_role_variants = new Set<string>()
  for (const [index, role] of role_names.entries()) {
    let variant = pascal_case(role.replace(/[^A-Za-z0-9_]+/g, "_"))
    if (variant.length === 0 || /^[0-9]/.test(variant)) {
      variant = `Role${index}${variant}`
    }
    if (["Self", "Super", "Crate", "Where", "Loop", "Match", "Ref", "Type"].includes(variant)) {
      variant = `Role${variant}`
    }
    while (used_role_variants.has(variant)) {
      variant = `${variant}${index}`
    }
    used_role_variants.add(variant)
    role_variant_names.set(role, variant)
  }
  const field_index_modules = (
    direction: "request" | "response",
  ): string => {
    const constants = compatibility_operations.flatMap((operation) => {
      const plan = direction === "request"
        ? operation.contract.request_plan ?? []
        : operation.contract.response_plan ?? []
      const ordinals = new Map<string, number>()
      return plan.map((field, index) => {
        const ordinal = ordinals.get(field.role) ?? 0
        ordinals.set(field.role, ordinal + 1)
        return `    /// ${direction} field ${field.path.join(".")} for ${operation.name}.
    pub const ${rust_const_identifier(operation.name)}_${rust_const_identifier(field.role)}_${ordinal}: usize = ${index};`
      })
    })
    return `/// Direct numeric indexes for generated operation fields.
///
/// These constants are derived from Smithy member order. Compatibility
/// behavior can use them with OperationInputView::field_at_index and avoid a
/// hot-path string-role scan without exposing the role vocabulary to generic
/// operation infrastructure.
#[allow(dead_code)]
pub mod ${direction}_fields {
${constants.join("\n")}
}
`
  }
  const field_index_lookup_arms = compatibility_operations.flatMap((operation) => {
    const render_direction = (
      direction: "request" | "response",
      fields: readonly Wire_Operation_Field_Plan[] | undefined,
    ): string[] => {
      const ordinals = new Map<string, number>()
      return (fields ?? []).map((field) => {
        const ordinal = ordinals.get(field.role) ?? 0
        ordinals.set(field.role, ordinal + 1)
        const constant =
          `${rust_const_identifier(operation.name)}_${rust_const_identifier(field.role)}_${ordinal}`
        const direction_variant =
          direction === "request" ? "Request" : "Response"
        return `        (Opcode::${operation.name}, OperationFieldDirection::${direction_variant}, OperationFieldRole::${role_variant_names.get(field.role)}, ${ordinal}) => Some(${direction}_fields::${constant}),`
      })
    }
    return [
      ...render_direction("request", operation.contract.request_plan),
      ...render_direction("response", operation.contract.response_plan),
    ]
  }).join("\n")
  const field_count_lookup_arms = compatibility_operations.flatMap((operation) => {
    const render_direction = (
      direction: "request" | "response",
      fields: readonly Wire_Operation_Field_Plan[] | undefined,
    ): string[] => {
      const counts = new Map<string, number>()
      for (const field of fields ?? []) {
        counts.set(field.role, (counts.get(field.role) ?? 0) + 1)
      }
      return [...counts].map(
        ([role, count]) =>
          `        (Opcode::${operation.name}, OperationFieldDirection::${direction === "request" ? "Request" : "Response"}, OperationFieldRole::${role_variant_names.get(role)}) => ${count},`,
      )
    }
    return [
      ...render_direction("request", operation.contract.request_plan),
      ...render_direction("response", operation.contract.response_plan),
    ]
  }).join("\n")
  const compatibility_routes = [
    ...new Set(
      operations
        .map((operation) => compact_route(operation))
        .filter((route): route is string => route !== undefined),
    ),
  ]
  return `${render_rust_semantic_constants(contract)}
/// Conservative maximum complete request frame size contributed by the
/// protocol-v1 compatibility adapter.
pub const MAX_COMPATIBILITY_REQUEST_FRAME_BYTES: usize =
    ${formatted_decimal(compatibility_request_frame_bound(contract))};

/// Generated protocol-v1 compatibility projection metadata.
///
/// This table is emitted separately from the canonical wire descriptor. Generic
/// operation dispatch never consumes it; the v1 adapter uses it only when an
/// explicitly compatible operation is registered.
use super::Opcode;

/// Selects the request or response side of a protocol-v1 compatibility field
/// plan. This direction is an adapter lookup concern and is intentionally not
/// part of the canonical generic operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationFieldDirection {
    Request,
    Response,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationCompactV1Route {
${compatibility_routes
  .map((route) => `    ${pascal_case(route)},`)
  .join("\n")}
}

pub const fn route_for_opcode(
    opcode: Opcode,
) -> Option<OperationCompactV1Route> {
    match opcode {
${operations
  .filter((operation) => compact_route(operation) !== undefined)
  .map(
    (operation) =>
      `        Opcode::${operation.name} => Some(OperationCompactV1Route::${pascal_case(compact_route(operation)!)}),`,
  )
  .join("\n")}
        _ => None,
    }
}

/// Generated numeric key for a protocol-v1 compatibility role.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum OperationFieldRole {
${role_names
  .map((role) => `    ${role_variant_names.get(role)},`)
  .join("\n")}
}

impl OperationFieldRole {
    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn name(self) -> &'static str {
        match self {
${role_names
  .map((role) => `            Self::${role_variant_names.get(role)} => ${rust_string_literal(role)},`)
  .join("\n")}
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
${role_names
  .map((role) => `            ${rust_string_literal(role)} => Some(Self::${role_variant_names.get(role)}),`)
  .join("\n")}
            _ => None,
        }
    }
}

${field_index_modules("request")}${field_index_modules("response")}
/// Counts one compatibility role in a generated operation plan.
pub const fn operation_field_count(
    opcode: Opcode,
    direction: OperationFieldDirection,
    role: OperationFieldRole,
) -> usize {
    match (opcode, direction, role) {
${field_count_lookup_arms}
        _ => 0,
    }
}

/// Returns a generated numeric compatibility field index.
pub const fn operation_field_index(
    opcode: Opcode,
    direction: OperationFieldDirection,
    role: OperationFieldRole,
    occurrence: usize,
) -> Option<usize> {
    match (opcode, direction, role, occurrence) {
${field_index_lookup_arms}
        _ => None,
    }
}
`
}
