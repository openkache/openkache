import type {
  Wire_Model_Request_Framing,
  Wire_Response_Framing,
} from "../protocol/wire"
import type { Compact_Request_Adapter_Route } from "./compatibility_request_adapters"
import type {
  Operation_Result_Kind,
  Operation_Response_Transport,
} from "./operation_results"

/** Target-language projection labels owned by the compatibility adapter. */
export type Compatibility_Response_Projection_Kind =
  | "raw_payload"
  | "optional_payload"
  | "status_outcome"
  | "boolean_outcome"
  | "text_payload"
  | "descriptor"

/** Renderer routes exposed by the protocol-v1 convenience projections. */
export type Compatibility_Response_Adapter_Route =
  | "pong"
  | "value"
  | "set_outcome"
  | "delete_outcome"
  | "stats_json"
  | "namespace_descriptor"

/** Context supplied to a compatibility response adapter without exposing API names. */
export interface Compatibility_Response_Adapter_Context {
  /** Optional compact request projection selected by the request adapter. */
  readonly request_adapter_route?: Compact_Request_Adapter_Route
  /** Canonical request framing selected before compatibility projection. */
  readonly request_framing: Wire_Model_Request_Framing
  readonly response_framing: Wire_Response_Framing
}

/** A typed projection retained for the current protocol-v1/client surface. */
export interface Compatibility_Response_Adapter {
  readonly result_kinds: readonly Operation_Result_Kind[] |
    ((context: Compatibility_Response_Adapter_Context) => readonly Operation_Result_Kind[])
  /**
   * Maps a contract status token to the native convenience discriminator.
   *
   * The mapping belongs to the adapter because status names are domain/API
   * vocabulary. Generic operations use the transport fallback below.
   */
  readonly result_kind_for_status: (
    status: string,
    context: Compatibility_Response_Adapter_Context,
  ) => Operation_Result_Kind | undefined
  /** Generic result projection consumed by all language renderers. */
  readonly projection: Compatibility_Response_Projection_Kind
  readonly route: Compatibility_Response_Adapter_Route
  readonly supports: (context: Compatibility_Response_Adapter_Context) => boolean
}

/** Generic or compatibility response route selected by one projected result. */
export type Operation_Response_Route =
  | Operation_Response_Transport
  | Compatibility_Response_Adapter_Route

export type Operation_Result_Route = Operation_Response_Route
interface Operation_Result_Route_Source {
  readonly response_transport: Operation_Response_Transport
  readonly compatibility_adapter?: Compatibility_Response_Adapter
}

/** Returns the selected generic or compatibility route for a projected result. */
export function operation_result_route(
  plan: Operation_Result_Route_Source,
): Operation_Result_Route {
  return plan.compatibility_adapter?.route ?? plan.response_transport
}

const supports_framing = (
  ...allowed_framings: readonly Wire_Response_Framing[]
): ((context: Compatibility_Response_Adapter_Context) => boolean) =>
  (context) => allowed_framings.includes(context.response_framing)

/**
 * A response projection that represents a protocol-v1 route must never be
 * selected from a route-less generic operation. Keeping this guard in the
 * adapter registry means an open semantic label such as `value` cannot
 * accidentally opt a future API into a compatibility client shape.
 */
const supports_compatibility_framing = (
  ...allowed_framings: readonly Wire_Response_Framing[]
): ((context: Compatibility_Response_Adapter_Context) => boolean) =>
  (context) =>
    context.request_adapter_route !== undefined &&
    allowed_framings.includes(context.response_framing)

/**
 * PING is the one historical result projection whose request has no compact
 * route. Keep that projection available for its empty request, but reject an
 * opaque or field-sequence request that happens to reuse the `pong` label.
 * Otherwise a generic API could silently lose its request body in the
 * language-specific raw-payload renderer.
 */
const supports_empty_request_pong = (
  context: Compatibility_Response_Adapter_Context,
): boolean =>
  context.request_framing === "empty" &&
  supports_framing("empty", "opaque")(context)

/**
 * Explicit protocol-v1/client convenience projections.
 *
 * The key is an open semantic label from the Smithy contract. Adding a new
 * API with another label never requires editing this registry: it falls back
 * to generic framing. A new typed convenience API opts in here once, instead
 * of adding branches to every language renderer.
 */
export const COMPATIBILITY_RESPONSE_ADAPTERS: Readonly<
  Record<string, Compatibility_Response_Adapter>
> = {
  pong: {
    projection: "raw_payload",
    result_kinds: ["ok"],
    result_kind_for_status: () => "ok",
    route: "pong",
    supports: supports_empty_request_pong,
  },
  value: {
    projection: "optional_payload",
    result_kinds: ["value", "not_found"],
    result_kind_for_status: (status) =>
      status === "not_found" ? "not_found" : "value",
    route: "value",
    supports: supports_compatibility_framing("opaque", "adapter_owned"),
  },
  values: {
    projection: "optional_payload",
    result_kinds: ["value"],
    result_kind_for_status: () => "value",
    route: "value",
    supports: supports_compatibility_framing("optional_values"),
  },
  set_outcome: {
    projection: "status_outcome",
    result_kinds: ["created", "replaced", "not_stored"],
    result_kind_for_status: (status) => {
      switch (status) {
        case "created":
          return "created"
        case "replaced":
          return "replaced"
        case "not_stored":
          return "not_stored"
        default:
          return undefined
      }
    },
    route: "set_outcome",
    supports: supports_compatibility_framing("empty"),
  },
  delete_outcome: {
    projection: "boolean_outcome",
    result_kinds: ["deleted", "not_deleted"],
    result_kind_for_status: (status) =>
      status === "deleted"
        ? "deleted"
        : status === "not_found"
        ? "not_deleted"
        : undefined,
    route: "delete_outcome",
    // Namespace delete uses a dedicated protocol-v1 call and only accepts
    // the generic `ok` result in generated clients.
    supports: (context) =>
      context.response_framing === "empty" &&
      context.request_adapter_route !== undefined &&
      context.request_adapter_route !== "namespace_delete",
  },
  stats_json: {
    projection: "text_payload",
    result_kinds: ["value"],
    result_kind_for_status: () => "value",
    route: "stats_json",
    supports: supports_compatibility_framing("opaque"),
  },
  namespace_descriptor: {
    projection: "descriptor",
    result_kinds: (context) =>
      context.request_adapter_route === "namespace_open"
        ? ["ok", "created"]
        : ["value"],
    result_kind_for_status: (status, context) =>
      context.request_adapter_route === "namespace_open" && status === "created"
        ? "created"
        : context.request_adapter_route === "namespace_open" &&
            status === "ok"
        ? "ok"
        : "value",
    route: "namespace_descriptor",
    supports: (context) =>
      context.response_framing === "opaque" &&
      (context.request_adapter_route === "namespace_open" ||
        context.request_adapter_route === "namespace_update_policy"),
  },
}

/**
 * Resolves an explicitly registered compatibility convenience projection.
 *
 * @param response_semantics - Open semantic label from the operation contract.
 * @param context - Resolved transport context for the operation.
 * @returns A matching adapter, or `undefined` for generic results.
 */
export function compatibility_response_adapter(
  response_semantics: string | undefined,
  context: Compatibility_Response_Adapter_Context,
): Compatibility_Response_Adapter | undefined {
  if (response_semantics === undefined) return undefined
  const adapter = COMPATIBILITY_RESPONSE_ADAPTERS[response_semantics]
  return adapter !== undefined && adapter.supports(context) ? adapter : undefined
}

/**
 * Resolves the native status discriminators for one compatibility projection.
 *
 * @param adapter - Resolved convenience adapter.
 * @param context - Resolved transport context for the operation.
 * @returns Native result discriminators accepted by the adapter.
 */
export function compatibility_response_result_kinds(
  adapter: Compatibility_Response_Adapter,
  context: Compatibility_Response_Adapter_Context,
): readonly Operation_Result_Kind[] {
  return typeof adapter.result_kinds === "function"
    ? adapter.result_kinds(context)
    : adapter.result_kinds
}

/**
 * Resolves a status token through an explicit convenience adapter.
 *
 * Undefined means that the operation uses generic transport framing and
 * should select its framing's default result envelope.
 */
export function compatibility_response_result_kind(
  adapter: Compatibility_Response_Adapter,
  status: string,
  context: Compatibility_Response_Adapter_Context,
): Operation_Result_Kind | undefined {
  const result_kind = adapter.result_kind_for_status(status, context)
  if (result_kind === undefined) return undefined
  return compatibility_response_result_kinds(adapter, context).includes(result_kind)
    ? result_kind
    : undefined
}
