/** Generic operation descriptors and shape-derived payload bounds. */

import {
  field_layout,
  layout_payload_bound,
} from "./wire_layout"
import type {
  Wire_Contract,
  Wire_Operation,
  Wire_Operation_Contract,
  Wire_Operation_Descriptor,
  Wire_Request_Framing,
  Wire_Request_Step,
} from "./wire_types"

function response_framing_for(
  contract: Wire_Operation_Contract,
): NonNullable<Wire_Operation_Contract["response_framing"]> {
  if (contract.response_framing !== undefined) return contract.response_framing
  const field_count = contract.response_plan?.length ?? 0
  return field_count === 0
    ? "empty"
    : field_count === 1
    ? "opaque"
    : "field_sequence"
}

/**
 * Resolves one operation's transport and runtime capabilities.
 *
 * Explicit generic request framing and response framing are the production
 * source of truth. Adapter extension metadata remains outside the generic
 * descriptor; compatibility adapters request it explicitly.
 *
 * @param contract - The modeled operation contract.
 * @returns The canonical transport descriptor.
 * @throws {Error} If the operation does not declare request framing.
 */
export function derive_wire_operation_descriptor(
  contract: Wire_Operation_Contract,
): Wire_Operation_Descriptor {
  const request_framing: Wire_Request_Framing | undefined = contract.request_framing
  if (request_framing === undefined) {
    throw new Error("operation contract is missing request framing")
  }

  // Response framing is a wire property, not a semantic route. Compatibility
  // adapters may interpret the open semantic label after this descriptor is
  // built, but generic planning must never infer bytes from names such as
  // `value`, `pong`, or `composite`.
  const response_framing = response_framing_for(contract)
  // Historical routes are an adapter projection, not a property of the
  // canonical operation descriptor. Their prefix grammar is selected by the
  // compatibility adapter; generic layout planning remains entirely a
  // function of the modeled field plan.
  const request_layout = field_layout(contract.request_plan, request_framing)
  const response_layout = field_layout(contract.response_plan, response_framing)
  return {
    request_framing,
    request_frame: request_layout === "dense" ? "fixed_body" : "length_delimited",
    request_layout,
    response_framing,
    response_frame: "length_delimited",
    response_layout,
  }
}

/**
 * Returns the maximum payload bytes a modeled response may occupy.
 *
 * Response budgeting is a property of the output shape, never of the
 * request's input value length. Fixed optional-value responses carry one
 * length/sentinel prefix per modeled field. Generic field sequences use one
 * shared presence mask followed by a canonical length for every present field
 * except the final present field, which consumes the remainder. Status-only
 * responses carry no payload.
 *
 * @param contract - Contract-wide payload and varuint limits.
 * @param operation - Operation whose response shape is being budgeted.
 * @returns The maximum response payload bytes.
 * @throws {Error} If the modeled layout has invalid size metadata.
 */
export function response_payload_bound(
  contract: Pick<Wire_Contract, "max_value_bytes" | "v1">,
  operation: Pick<Wire_Operation, "contract">,
): number {
  const descriptor = derive_wire_operation_descriptor(operation.contract)
  const plan = operation.contract.response_plan ?? []
  return layout_payload_bound(
    contract.max_value_bytes,
    contract.v1.max_varuint_bytes,
    contract.v1.optional_value_length_bytes ?? 4,
    descriptor.response_framing,
    descriptor.response_layout,
    plan,
  )
}

/**
 * Returns the maximum body bytes accepted by a modeled request shape.
 *
 * @param contract - Contract-wide payload and varuint limits.
 * @param operation - Operation whose request shape is being budgeted.
 * @returns The maximum request body bytes.
 * @throws {Error} If the modeled layout has invalid size metadata.
 */
export function request_payload_bound(
  contract: Pick<Wire_Contract, "max_value_bytes" | "v1">,
  operation: Pick<Wire_Operation, "contract">,
): number {
  const descriptor = derive_wire_operation_descriptor(operation.contract)
  return layout_payload_bound(
    contract.max_value_bytes,
    contract.v1.max_varuint_bytes,
    contract.v1.optional_value_length_bytes ?? 4,
    descriptor.request_framing,
    descriptor.request_layout,
    operation.contract.request_plan ?? [],
  )
}

function request_wire_step_bound(
  contract: Pick<Wire_Contract, "max_value_bytes" | "v1">,
  steps: readonly Wire_Request_Step[],
): number {
  let bound = 0
  const add = (value: number): void => {
    if (!Number.isSafeInteger(value) || value < 0 || bound > Number.MAX_SAFE_INTEGER - value) {
      throw new Error("request-wire frame size exceeds the safe integer range")
    }
    bound += value
  }
  for (const step of steps) {
    switch (step.kind) {
      case "fixed_field": add(step.bytes); break
      case "packed": add(1); break
      case "byte_length_field": add(1 + Math.min(contract.max_value_bytes, 0xff)); break
      case "byte_length_prefix_field": add(1); break
      case "byte_field": add(Math.min(contract.max_value_bytes, 0xff)); break
      case "varuint_field": add(contract.v1.max_varuint_bytes); break
      case "value_length_field":
        add(contract.v1.max_varuint_bytes)
        add(contract.max_value_bytes)
        break
      case "conditional": add(request_wire_step_bound(contract, step.steps)); break
      case "constant": add(step.bytes.length); break
      case "trailing_field":
        add(contract.v1.max_varuint_bytes)
        add(contract.max_value_bytes)
        break
    }
  }
  return bound
}

/** Returns the maximum complete frame size for an explicit request-wire plan. */
export function request_wire_frame_bound(
  contract: Pick<Wire_Contract, "max_value_bytes" | "v1">,
  operation: Pick<Wire_Operation, "contract">,
): number {
  const plan = operation.contract.request_wire
  if (plan === undefined) return 0
  return contract.v1.opcode_bytes + request_wire_step_bound(contract, plan)
}
