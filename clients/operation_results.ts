/** Language-neutral response planning for generated clients. */

import {
  derive_wire_operation_descriptor,
  type Wire_Operation_Contract,
  type Wire_Operation_Field_Layout,
} from "../protocol/wire"

/** Open result token carried by a generated client projection. */
export type Operation_Result_Kind = string

/** Generic response transport primitives shared by all generated clients. */
export type Operation_Response_Transport =
  | "empty"
  | "opaque"
  | "optional_values"
  | "field_sequence"
  | "adapter_owned"

/** Generic response transport plan shared by every language renderer. */
export interface Operation_Result_Plan {
  /** Generic response framing selected from the canonical wire contract. */
  readonly response_transport: Operation_Response_Transport
  /** Shape-selected layout for generic ordered-field responses. */
  readonly response_layout?: Wire_Operation_Field_Layout
  /** Generic transport projection; compatibility semantics are resolved elsewhere. */
  readonly projection: Operation_Response_Transport
}

function operation_response_transport(
  contract: Wire_Operation_Contract,
): Operation_Response_Transport {
  switch (derive_wire_operation_descriptor(contract).response_framing) {
    case "empty":
      return "empty"
    case "opaque":
      return "opaque"
    case "optional_values":
      return "optional_values"
    case "field_sequence":
      return "field_sequence"
    default:
      // The generic client plan preserves the fact that a non-generic
      // response needs an adapter. It does not guess whether that adapter is
      // a historical sentinel table, a tagged envelope, or a future format.
      return "adapter_owned"
  }
}

/** Builds the generic response plan for one modeled operation. */
export function operation_result_plan(
  contract: Wire_Operation_Contract,
): Operation_Result_Plan {
  const response_transport = operation_response_transport(contract)
  return {
    response_transport,
    response_layout: derive_wire_operation_descriptor(contract).response_layout,
    projection: response_transport,
  }
}

/** Returns the generic transport projection without resolving compatibility semantics. */
export function operation_result_projection(
  plan: Operation_Result_Plan,
): Operation_Response_Transport {
  return plan.projection
}
