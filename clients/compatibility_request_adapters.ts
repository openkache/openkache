import {
  derive_wire_operation_descriptor,
  type Wire_Model_Request_Framing,
  type Wire_Operation_Frame_Policy,
  type Wire_Operation_Field_Layout,
  type Wire_Operation_Contract,
} from "../protocol/wire"

const SCOPE_EXTENSION = "openkache.protocol#operationContract.scope"

/**
 * Protocol-v1 request routes retained for the handwritten client convenience
 * surface.
 *
 * These routes describe an ABI projection, not a generic operation family.
 * Generic operations use only their declared request framing below.
 */
export type Compact_Request_Adapter_Route =
  | "item"
  | "set"
  | "namespace"
  | "namespace_open"
  | "namespace_update_policy"
  | "namespace_delete"

/**
 * One compact request adapter selected from modeled transport metadata.
 *
 * The adapter intentionally does not contain an operation name. This keeps
 * the protocol-v1 namespace/item/SET ABI reusable while preventing generic
 * request generation from learning domain semantics.
 */
export interface Compact_Request_Adapter {
  readonly route: Compact_Request_Adapter_Route
}

/** Request framing and the optional protocol-v1 convenience projection. */
export interface Request_Transport_Plan {
  /** Canonical framing understood by the transport-neutral executor. */
  readonly request_framing: Wire_Model_Request_Framing
  /** Shape-selected layout for generic ordered-field requests. */
  readonly request_layout?: Wire_Operation_Field_Layout
  /** Generic frame policy selected by the same generated layout plan. */
  readonly request_frame?: Wire_Operation_Frame_Policy
  /** Optional compact protocol-v1 projection. */
  readonly compact_adapter?: Compact_Request_Adapter
}

const COMPACT_REQUEST_ADAPTERS: Readonly<
  Record<string, Compact_Request_Adapter>
> = {
  item: { route: "item" },
  set: { route: "set" },
  namespace: { route: "namespace" },
  namespace_open: { route: "namespace_open" },
  namespace_update_policy: { route: "namespace_update_policy" },
  namespace_delete: { route: "namespace_delete" },
}

/**
 * Resolves the protocol-v1 convenience route from the canonical operation
 * scope and semantic field roles. The current Smithy model deliberately
 * exposes only generic request framing; route names remain a client adapter
 * concern and are never serialized into the wire contract.
 */
function derive_wire_compatibility_route(
  contract: Wire_Operation_Contract,
): Compact_Request_Adapter_Route | undefined {
  const scope = contract.extensions?.[SCOPE_EXTENSION]
  const roles = new Set((contract.request_plan ?? []).map((field) => field.role))
  switch (scope) {
    case "item":
      return roles.has("value") ? "set" : "item"
    case "namespace":
      return "namespace"
    case "namespace_management":
      if (roles.has("name") || roles.has("create_if_missing")) {
        return "namespace_open"
      }
      if (roles.has("policy") || roles.has("default_expiration")) {
        return "namespace_update_policy"
      }
      if (roles.has("expected_revision")) return "namespace_delete"
      return undefined
    default:
      return undefined
  }
}

/**
 * Resolves one operation's request transport boundary.
 *
 * `ordered_fields`, `opaque`, and `empty` are generic framing primitives.
 * An explicit protocol-v1 route adds an adapter projection without creating a
 * fourth generic framing family.
 */
export function request_transport_plan(
  contract: Wire_Operation_Contract,
): Request_Transport_Plan {
  const descriptor = derive_wire_operation_descriptor(contract)
  const compact_route = derive_wire_compatibility_route(contract)
  if (compact_route === undefined) {
    return {
      request_framing: descriptor.request_framing,
      request_layout: descriptor.request_layout,
      request_frame: descriptor.request_frame,
    }
  }

  const compact_adapter = COMPACT_REQUEST_ADAPTERS[compact_route]
  if (compact_adapter === undefined) {
    throw new Error(
      "protocol-v1 route is missing a supported compatibility adapter",
    )
  }
  return {
    request_framing: descriptor.request_framing,
    compact_adapter,
  }
}

/**
 * Returns whether one operation selected a particular compact adapter route.
 */
export function uses_compact_request_route(
  plan: Request_Transport_Plan,
  route: Compact_Request_Adapter_Route,
): boolean {
  return plan.compact_adapter?.route === route
}
