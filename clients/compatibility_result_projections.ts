/**
 * Protocol-v1/client convenience result projections.
 *
 * Generic response planning lives in `operation_results.ts`. This module
 * resolves the optional historical semantic adapter only after transport
 * framing has been derived, so a new API can use the raw result envelope
 * without extending the generic result plan or every language renderer.
 */

import {
  derive_wire_operation_descriptor,
  type Wire_Operation_Contract,
} from "../protocol/wire"
import {
  request_transport_plan,
  type Request_Transport_Plan,
} from "./compatibility_request_adapters"
import {
  compatibility_response_adapter,
  compatibility_response_result_kinds,
  type Compatibility_Response_Adapter,
  type Compatibility_Response_Adapter_Context,
} from "./compatibility_response_adapters"
import type {
  Operation_Response_Transport,
  Operation_Result_Kind,
} from "./operation_results"
import {
  derive_operation_client_projection,
} from "./operation_client_projection"

import type { Compatibility_Response_Projection_Kind } from "./compatibility_response_adapters"

/** Renderer projection labels available after generic planning. */
export type Operation_Response_Projection_Kind =
  | Operation_Response_Transport
  | Compatibility_Response_Projection_Kind

export type { Operation_Result_Kind }

/** Result projection consumed by target-language renderers. */
export interface Operation_Result_Projection {
  readonly projection: Operation_Response_Projection_Kind
  readonly result_kinds: readonly Operation_Result_Kind[]
  readonly compatibility_adapter?: Compatibility_Response_Adapter
}

const GENERIC_RESULT_KINDS: readonly Operation_Result_Kind[] = ["raw"]

/** Resolves the context passed to an optional compatibility result adapter. */
export function operation_response_context(
  contract: Wire_Operation_Contract,
  request = request_transport_plan(contract),
): Compatibility_Response_Adapter_Context {
  const descriptor = derive_wire_operation_descriptor(contract)
  return {
    request_adapter_route: request.compact_adapter?.route,
    request_framing: contract.request_framing ?? descriptor.request_framing,
    response_framing: descriptor.response_framing,
  }
}

/** Resolves the typed compatibility adapter, if the contract explicitly opts in. */
export function operation_compatibility_response_adapter(
  contract: Wire_Operation_Contract,
  request = request_transport_plan(contract),
): Compatibility_Response_Adapter | undefined {
  return compatibility_response_adapter(
    derive_operation_client_projection(contract).response_semantics,
    operation_response_context(contract, request),
  )
}

/** Resolves the optional compatibility/client projection for a generic result plan. */
export function operation_result_projection(
  contract: Wire_Operation_Contract,
  request: Request_Transport_Plan,
): Operation_Result_Projection {
  const context = operation_response_context(contract, request)
  const adapter = compatibility_response_adapter(
    derive_operation_client_projection(contract).response_semantics,
    context,
  )
  const response_framing = derive_wire_operation_descriptor(contract).response_framing
  const generic_projection: Operation_Response_Transport =
    response_framing === "field_sequence" || response_framing === "optional_values"
      ? "field_sequence"
      : response_framing
  return {
    ...(adapter === undefined ? {} : { compatibility_adapter: adapter }),
    projection: adapter?.projection ?? generic_projection,
    result_kinds: adapter === undefined
      ? GENERIC_RESULT_KINDS
      : compatibility_response_result_kinds(adapter, context),
  }
}
