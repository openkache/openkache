/** Historical response framing owned by the protocol-v1 client adapter. */

import type { Client_Contract } from "./client_contract"

/** The fixed-width optional-value representation used by protocol-v1. */
export interface Optional_Value_Framing {
  readonly encoding: "big_endian"
  readonly length_bytes: number
  readonly max_encoded_entry_bytes: number
  readonly max_value_bytes: number
  readonly missing_sentinel: number
}

/**
 * Resolves the protocol-v1 optional-value framing constants.
 *
 * Generic response planning does not call this function. Language renderers
 * use it only after the compatibility projection has selected the historical
 * optional-value decoder.
 */
export function optional_value_framing(
  contract: Pick<Client_Contract, "max_value_bytes" | "v1">,
): Optional_Value_Framing {
  const length_bytes = contract.v1.optional_value_length_bytes ?? 4
  const missing_sentinel = contract.v1.optional_value_missing ?? 0xffff_ffff
  if (length_bytes !== 4 || missing_sentinel !== 0xffff_ffff) {
    throw new Error(
      "optional-value framing must use four big-endian length bytes and 0xffffffff as the missing sentinel",
    )
  }
  return {
    encoding: "big_endian",
    length_bytes,
    max_encoded_entry_bytes: length_bytes + contract.max_value_bytes,
    max_value_bytes: contract.max_value_bytes,
    missing_sentinel,
  }
}

/**
 * Returns whether a managed operation selected the compact optional-value
 * layout. The layout is generic; a compatibility projection may still choose
 * a typed convenience result on top of it.
 *
 */
export function operation_uses_optional_value_layout(
  operation: {
    readonly plan: {
      readonly contract: {
        readonly response_framing?: string
      }
      readonly result_plan: {
        readonly response_transport: string
      }
      readonly result_projection: {
        readonly compatibility_adapter?: {
          readonly projection: string
        }
      }
    }
  },
): boolean {
  return operation.plan.contract.response_framing === "optional_values"
}
