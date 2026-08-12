/**
 * Compatibility projections for the historical protocol-v1 ABI.
 *
 * The generic wire descriptor owns framing, field plans, and codecs. This
 * module owns only public API response semantics and client policy metadata.
 */

import type {
  Wire_Contract,
  Wire_Contract_Adapter,
  Wire_Operation_Contract,
  Wire_Operation_Field_Plan,
  Wire_V1_Contract,
} from "./wire_types"
import { WIRE_RESPONSE_FRAMINGS } from "./wire_types"
import { extract_wire_contract as extract_generic_wire_contract } from "./wire"
import {
  PROTOCOL_V1_RESPONSE_SEMANTICS_EXTENSION,
  PROTOCOL_V1_RETRY_MODE_EXTENSION,
  PROTOCOL_V1_SCOPE_EXTENSION,
  WIRE_RESPONSE_ROUTES,
  type Wire_Response_Route,
} from "./compat_v1_types"
import { optional_integer_member } from "./wire/validate_contract"
export {
  PROTOCOL_V1_RESPONSE_SEMANTICS_EXTENSION,
  PROTOCOL_V1_RETRY_MODE_EXTENSION,
  PROTOCOL_V1_SCOPE_EXTENSION,
  WIRE_RESPONSE_ROUTES,
} from "./compat_v1_types"
export type { Wire_Response_Route } from "./compat_v1_types"

/** Concrete v1 fields owned by the historical compatibility adapter. */
export interface Wire_Compatibility_V1_Contract extends Wire_V1_Contract {
  readonly optional_value_length_bytes: number
  readonly optional_value_missing: number
}

/** Contract returned by the protocol-v1 generation composition root. */
export interface Wire_Compatibility_Contract extends Omit<Wire_Contract, "v1"> {
  readonly v1: Wire_Compatibility_V1_Contract
}

export const OPTIONAL_VALUES_RESPONSE_FRAMING = "optional_values" as const

const POLICY_ROLES = [
  "policy",
  "default_expiration",
  "default_ttl_milliseconds",
  "expiration_override",
  "default_eviction",
  "eviction_override",
] as const

/**
 * Validates the optional protocol-v1 projection after generic extraction.
 *
   * Exact request plans are validated entirely by the generic extractor.
 */
export const PROTOCOL_V1_WIRE_ADAPTER: Wire_Contract_Adapter = {
  extract_v1(
    contract: Readonly<Record<string, unknown>>,
    contract_location: string,
  ) {
    const optional_value_length_bytes = optional_integer_member(
      contract,
      "optionalValueLengthBytes",
      contract_location,
      1,
      0xff,
    )
    const optional_value_missing = optional_integer_member(
      contract,
      "optionalValueMissing",
      contract_location,
      0,
      Number.MAX_SAFE_INTEGER,
    )
    if (
      optional_value_length_bytes !== undefined &&
      optional_value_length_bytes !== 4
    ) {
      throw new Error(
        `${contract_location}.optionalValueLengthBytes must be 4 for protocol-v1`,
      )
    }
    if (
      optional_value_missing !== undefined &&
      optional_value_missing !== 0xffff_ffff
    ) {
      throw new Error(
        `${contract_location}.optionalValueMissing must be 0xffffffff for protocol-v1`,
      )
    }
    return {
      optional_value_length_bytes: optional_value_length_bytes ?? 4,
      optional_value_missing: optional_value_missing ?? 0xffff_ffff,
    }
  },
  extract_extensions(
    contract: Readonly<Record<string, unknown>>,
    operation_location: string,
  ): Readonly<Record<string, unknown>> | undefined {
    const extension_members = [
      ["responseSemantics", PROTOCOL_V1_RESPONSE_SEMANTICS_EXTENSION],
      ["scope", PROTOCOL_V1_SCOPE_EXTENSION],
      ["retryMode", PROTOCOL_V1_RETRY_MODE_EXTENSION],
    ] as const
    const extensions: Record<string, string> = {}
    for (const [member, extension] of extension_members) {
      const value = contract[member]
      if (value === undefined) continue
      if (typeof value !== "string" || value.length === 0) {
        throw new Error(
          `${operation_location}.${member} must be a non-empty string`,
        )
      }
      extensions[extension] = value
    }
    return Object.keys(extensions).length === 0 ? undefined : extensions
  },
  validate_operation(contract: Wire_Operation_Contract, operation_location: string): void {
    if (
      contract.request_wire !== undefined &&
      contract.request_framing !== "ordered_fields"
    ) {
      throw new Error(
        `${operation_location}.requestWire compatibility projection requires requestFraming ordered_fields`,
      )
    }
    if (
      contract.response_framing !== undefined &&
      !WIRE_RESPONSE_FRAMINGS.includes(contract.response_framing as
        (typeof WIRE_RESPONSE_FRAMINGS)[number]) &&
      contract.response_framing !== OPTIONAL_VALUES_RESPONSE_FRAMING
    ) {
      throw new Error(
        `${operation_location}.responseFraming is not supported by protocol-v1 compatibility`,
      )
    }
    if (
      contract.response_framing === OPTIONAL_VALUES_RESPONSE_FRAMING &&
      (contract.response_plan?.length ?? 0) === 0
    ) {
      throw new Error(
        `${operation_location}.responseFraming optional_values requires at least one modeled field`,
      )
    }
  },
}

/**
 * Extracts a contract for the protocol-v1 generation pipeline.
 *
 * Generic callers should use the extractor from `wire.ts` directly and pass
 * their own adapter when they define an extension projection.
 */
export function extract_compatibility_wire_contract(
  ast: unknown,
  strict_operations = false,
): Wire_Compatibility_Contract {
  return extract_generic_wire_contract(
    ast,
    strict_operations,
    PROTOCOL_V1_WIRE_ADAPTER,
  ) as Wire_Compatibility_Contract
}

/** Returns an adapter-owned open semantic label, when one was modeled. */
export function derive_wire_compatibility_response_semantics(
  contract: Wire_Operation_Contract,
): string | undefined {
  const value = contract.extensions?.[PROTOCOL_V1_RESPONSE_SEMANTICS_EXTENSION]
  return typeof value === "string" ? value : undefined
}

/** Returns the optional client/API scope retained for compatibility docs. */
export function derive_wire_compatibility_scope(
  contract: Wire_Operation_Contract,
): string {
  const value = contract.extensions?.[PROTOCOL_V1_SCOPE_EXTENSION]
  return typeof value === "string" ? value : "global"
}

/** Returns the optional client retry policy retained for compatibility docs. */
export function derive_wire_compatibility_retry_mode(
  contract: Wire_Operation_Contract,
): string {
  const value = contract.extensions?.[PROTOCOL_V1_RETRY_MODE_EXTENSION]
  return typeof value === "string" ? value : "always"
}

function operation_has_role(
  fields: readonly Wire_Operation_Field_Plan[],
  role: string,
): boolean {
  return fields.some((field) => field.role === role)
}

function validate_operation_field_roles(
  fields: readonly Wire_Operation_Field_Plan[],
  allowed: readonly string[] | undefined,
  direction: "request" | "response",
): void {
  if (allowed === undefined) return
  const allowed_roles = new Set(allowed)
  const unsupported = [
    ...new Set(
      fields
        .filter((field) => !allowed_roles.has(field.role))
        .map((field) => field.role),
    ),
  ]
  if (unsupported.length > 0) {
    throw new Error(
      `${direction} roles are not supported by protocol-v1 framing: ${unsupported.join(", ")}`,
    )
  }
}

/** Derives the protocol response route for compatibility fixtures. */
export function derive_wire_response_route(
  contract: Wire_Operation_Contract,
): Wire_Response_Route {
  const response_fields = contract.response_plan ?? []
  const response_field_count = response_fields.length
  const has_role = (role: string): boolean => operation_has_role(response_fields, role)
  const has_only_roles = (roles: readonly string[]): boolean => {
    const allowed_roles = new Set(roles)
    return response_fields.every((field) => allowed_roles.has(field.role))
  }
  const compatibility_descriptor_roles = [
    "descriptor",
    "created",
    "namespace_id",
    "revision",
    ...POLICY_ROLES,
  ]
  const explicit_semantics = derive_wire_compatibility_response_semantics(contract)
  let route: Wire_Response_Route

  if (explicit_semantics !== undefined) {
    switch (explicit_semantics) {
      case "pong":
        route = response_field_count === 0 ? "pong" : "application_value"
        break
      case "application_value":
      case "value":
      case "set_outcome":
      case "delete_outcome":
      case "stats_json":
      case "namespace_descriptor":
      case "empty":
        route = explicit_semantics
        break
      default:
        route = contract.response_framing === "empty"
          ? "empty"
          : contract.response_framing === "opaque"
          ? "application_value"
          : contract.response_framing === OPTIONAL_VALUES_RESPONSE_FRAMING
          ? "field_sequence"
          : response_field_count === 0
          ? "empty"
          : "field_sequence"
        break
    }
  } else if (contract.response_framing !== undefined) {
    route = contract.response_framing === "empty"
      ? "empty"
      : contract.response_framing === "opaque"
      ? "application_value"
      : "field_sequence"
  } else if (
    has_role("descriptor") &&
    has_only_roles(compatibility_descriptor_roles)
  ) {
    route = "namespace_descriptor"
  } else if (response_field_count > 1) {
    route = "field_sequence"
  } else if (has_role("payload") && response_fields.length === 1) {
    route = "application_value"
  } else if (has_role("value") && response_fields.length === 1) {
    route = "value"
  } else if (has_role("outcome") && response_fields.length === 1) {
    route = "set_outcome"
  } else if (has_role("deleted") && response_fields.length === 1) {
    route = "delete_outcome"
  } else if (has_role("json") && response_fields.length === 1) {
    route = "stats_json"
  } else {
    route = response_fields.length === 0 ? "empty" : "field_sequence"
  }

  const allowed: Partial<Record<Wire_Response_Route, readonly string[]>> = {
    pong: [],
    empty: [],
    value: ["value"],
    set_outcome: ["outcome"],
    delete_outcome: ["deleted"],
    stats_json: ["json"],
    namespace_descriptor: compatibility_descriptor_roles,
  }
  const opaque_field_count = response_fields.length
  if (
    route === "application_value" &&
    (contract.response_framing === "opaque" ||
      explicit_semantics === "application_value")
  ) {
    if (opaque_field_count !== 1) {
      throw new Error("opaque response framing requires exactly one modeled field")
    }
  } else if (!(route === "empty" && contract.response_framing === "empty")) {
    validate_operation_field_roles(response_fields, allowed[route], "response")
  }
  return route
}
