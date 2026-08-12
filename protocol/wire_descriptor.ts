/** Generic operation descriptors and shape-derived payload bounds. */

import {
  field_layout,
  layout_payload_bound,
} from "./wire_layout"
import { WIRE_RESPONSE_FRAMINGS } from "./wire_types"
import type {
  Wire_Contract,
  Wire_Operation,
  Wire_Operation_Contract,
  Wire_Operation_Descriptor,
  Wire_Request_Framing,
  Wire_Request_Step,
  Wire_Response_Framing,
} from "./wire_types"

function response_framing_for(
  contract: Wire_Operation_Contract,
): Wire_Response_Framing {
  if (contract.response_framing !== undefined) {
    return WIRE_RESPONSE_FRAMINGS.includes(
      contract.response_framing as (typeof WIRE_RESPONSE_FRAMINGS)[number],
    )
      ? contract.response_framing
      : "adapter_owned"
  }
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

  // Response framing is a wire property, not a semantic route. API adapters
  // may interpret an application semantic label after this descriptor is
  // built, but generic planning must never infer bytes from names such as
  // `value`, `pong`, or `composite`.
  const response_framing = response_framing_for(contract)
  // Exact request-wire prefixes remain an explicit generated plan. Generic
  // consumers accept only the generic response subset; an unknown response
  // framing becomes adapter-owned without being interpreted here.
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
 * request's input value length. Generic field sequences use one shared
 * presence mask followed by a canonical length for every present field except
 * the final present field, which consumes the remainder. Adapter-owned
 * responses receive the aggregate protocol ceiling here; their concrete
 * prefix and sentinel costs are owned by the adapter. Status-only responses
 * carry no payload.
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
    if (!Number.isSafeInteger(value) || value < 0) {
      throw new Error("request-wire frame size requires safe non-negative integers")
    }
    if (bound > Number.MAX_SAFE_INTEGER - value) {
      throw new Error("request-wire frame size exceeds the safe integer range")
    }
    bound += value
  }

  for (const step of steps) {
    switch (step.kind) {
      case "fixed_field":
        add(step.bytes)
        break
      case "packed":
        // Every packed primitive occupies one byte regardless of the number
        // of modeled fields it projects.
        add(1)
        break
      case "byte_length_field":
        // The primitive stores an unsigned byte length, so 255 is the
        // largest possible field body even when the modeled shape is open.
        add(1 + Math.min(contract.max_value_bytes, 0xff))
        break
      case "varuint_field":
        add(contract.v1.max_varuint_bytes)
        break
      case "conditional":
        // A condition selects at most one nested branch. Taking the largest
        // branch gives a safe admission bound without interpreting its field
        // or operation semantics.
        add(request_wire_step_bound(contract, step.steps))
        break
      case "constant":
        add(step.bytes.length)
        break
      case "trailing_field":
        // The trailing value is retained as the independently owned body;
        // its canonical length prefix remains in the generated prefix.
        add(contract.v1.max_varuint_bytes)
        add(contract.max_value_bytes)
        break
    }
  }
  return bound
}

/**
 * Returns the maximum complete frame size for an exact declarative request
 * plan.
 *
 * The result is generic admission metadata. It does not infer a compatibility
 * route or inspect field roles: every fixed, packed, conditional, length, and
 * trailing primitive contributes only its operation-neutral byte cost.
 *
 * @param contract - Contract-wide wire ceilings and varuint width.
 * @param operation - Operation whose exact request plan is being budgeted.
 * @returns The maximum complete frame size, including the opcode.
 * @throws {Error} If size arithmetic exceeds JavaScript's safe integer range.
 */
export function request_wire_frame_bound(
  contract: Pick<Wire_Contract, "max_value_bytes" | "v1">,
  operation: Pick<Wire_Operation, "contract">,
): number {
  const plan = operation.contract.request_wire
  if (plan === undefined) return 0
  const bound = contract.v1.opcode_bytes + request_wire_step_bound(contract, plan)
  if (!Number.isSafeInteger(bound) || bound < 0) {
    throw new Error("request-wire frame size exceeds the safe integer range")
  }
  return bound
}
