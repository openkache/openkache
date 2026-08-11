/**
 * Compatibility projections for the historical protocol-v1 ABI.
 *
 * The generic wire descriptor owns framing, field plans, and codecs. This
 * module owns only the old namespace/item/SET route vocabulary and its
 * role/cardinality validation.
 */

import type {
  Wire_Contract_Adapter,
  Wire_Operation_Contract,
  Wire_Operation_Field_Plan,
} from "./wire_types"
import { extract_wire_contract as extract_generic_wire_contract } from "./wire"
import {
  PROTOCOL_V1_COMPACT_ROUTE_EXTENSION,
  PROTOCOL_V1_RESPONSE_SEMANTICS_EXTENSION,
  PROTOCOL_V1_RETRY_MODE_EXTENSION,
  PROTOCOL_V1_SCOPE_EXTENSION,
  WIRE_COMPACT_REQUEST_ROUTES,
  WIRE_RESPONSE_ROUTES,
  type Wire_Compact_Request_Route,
  type Wire_Response_Route,
} from "./compat_v1_types"
export {
  PROTOCOL_V1_COMPACT_ROUTE_EXTENSION,
  PROTOCOL_V1_RESPONSE_SEMANTICS_EXTENSION,
  PROTOCOL_V1_RETRY_MODE_EXTENSION,
  PROTOCOL_V1_SCOPE_EXTENSION,
  WIRE_COMPACT_REQUEST_ROUTES,
  WIRE_RESPONSE_ROUTES,
} from "./compat_v1_types"
export type {
  Wire_Compact_Request_Route,
  Wire_Response_Route,
} from "./compat_v1_types"

const POLICY_ROLES = [
  "policy",
  "default_expiration",
  "default_ttl_milliseconds",
  "expiration_override",
  "default_eviction",
  "eviction_override",
] as const

const COMPACT_REQUEST_ROLES: Record<
  Wire_Compact_Request_Route,
  readonly string[]
> = {
  item: ["namespace_id", "item_id"],
  set: [
    "namespace_id",
    "item_id",
    "value",
    "condition",
    "expiration_mode",
    "ttl_milliseconds",
    "eviction_mode",
  ],
  namespace: ["namespace_id"],
  namespace_open: ["name", "create_if_missing", ...POLICY_ROLES],
  namespace_update_policy: [
    "namespace_id",
    "expected_revision",
    ...POLICY_ROLES,
  ],
  namespace_delete: ["namespace_id", "expected_revision"],
}

/**
 * Validates the optional protocol-v1 projection after generic extraction.
 *
 * The generic extractor accepts extension route names as opaque metadata. This
 * adapter is the only place that narrows the historical route vocabulary and
 * applies its field-cardinality rules.
 */
export const PROTOCOL_V1_WIRE_ADAPTER: Wire_Contract_Adapter = {
  extract_extensions(
    contract: Readonly<Record<string, unknown>>,
    operation_location: string,
  ): Readonly<Record<string, unknown>> | undefined {
    const extension_members = [
      ["compactRoute", PROTOCOL_V1_COMPACT_ROUTE_EXTENSION],
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
    const route = contract.extensions?.[PROTOCOL_V1_COMPACT_ROUTE_EXTENSION]
    if (route === undefined) return
    if (
      typeof route !== "string" ||
      !WIRE_COMPACT_REQUEST_ROUTES.includes(route as Wire_Compact_Request_Route)
    ) {
      throw new Error(
        `${operation_location}.compactRoute must name a supported protocol-v1 compact route`,
      )
    }
    if (contract.request_framing !== "ordered_fields") {
      throw new Error(
        `${operation_location}.compactRoute requires requestFraming ordered_fields`,
      )
    }
    validate_compact_request_route(
      route as Wire_Compact_Request_Route,
      contract.request_plan ?? [],
      operation_location,
    )
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
): ReturnType<typeof extract_generic_wire_contract> {
  return extract_generic_wire_contract(
    ast,
    strict_operations,
    PROTOCOL_V1_WIRE_ADAPTER,
  )
}

/** Returns a validated protocol-v1 route for compatibility renderers. */
export function derive_wire_compatibility_route(
  contract: Wire_Operation_Contract,
): Wire_Compact_Request_Route | undefined {
  const route = contract.extensions?.[PROTOCOL_V1_COMPACT_ROUTE_EXTENSION]
  return typeof route === "string" &&
      WIRE_COMPACT_REQUEST_ROUTES.includes(route as Wire_Compact_Request_Route)
    ? route as Wire_Compact_Request_Route
    : undefined
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

function operation_field_count(
  fields: readonly Wire_Operation_Field_Plan[],
  role: string,
): number {
  return fields.filter((field) => field.role === role).length
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

/**
 * Validates the fixed cardinalities required by a protocol-v1 compact route.
 *
 * Repeated item/value mutations are generic ordered-field requests, never a
 * one-item SET prefix.
 */
export function validate_compact_request_route(
  route: Wire_Compact_Request_Route,
  fields: readonly Wire_Operation_Field_Plan[],
  operation_location: string,
): void {
  const count = (role: string): number => operation_field_count(fields, role)
  const invalid = (requirements: string): never => {
    throw new Error(
      `${operation_location}.compactRoute ${route} requires ${requirements}`,
    )
  }
  switch (route) {
    case "item":
      if (
        count("namespace_id") !== 1 ||
        ![1, 2].includes(count("item_id"))
      ) {
        invalid("exactly one namespace_id and one or two item_id fields")
      }
      break
    case "set":
      if (
        count("namespace_id") !== 1 ||
        count("item_id") !== 1 ||
        count("value") !== 1
      ) {
        invalid("exactly one namespace_id, item_id, and value field")
      }
      break
    case "namespace":
      if (count("namespace_id") !== 1) invalid("exactly one namespace_id field")
      break
    case "namespace_open":
      if (count("name") !== 1 || count("create_if_missing") !== 1) {
        invalid("exactly one name and create_if_missing field")
      }
      break
    case "namespace_update_policy":
      if (count("namespace_id") !== 1 || count("expected_revision") !== 1) {
        invalid("exactly one namespace_id and expected_revision field")
      }
      break
    case "namespace_delete":
      if (count("namespace_id") !== 1 || count("expected_revision") !== 1) {
        invalid("exactly one namespace_id and expected_revision field")
      }
      break
  }
  validate_operation_field_roles(fields, COMPACT_REQUEST_ROLES[route], "request")
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
          : contract.response_framing === "optional_values"
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
