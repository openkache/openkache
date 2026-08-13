/**
 * Client-only projections over the canonical operation contract.
 *
 * The protocol generator owns bytes, layouts, and codecs. Retry policy and
 * ergonomic response labels are client concerns, so they are resolved here
 * instead of being emitted by the protocol crate or imported by the server.
 */

import {
  derive_wire_operation_descriptor,
} from "../protocol/wire_descriptor"
import type {
  Wire_Operation_Contract,
} from "../protocol/wire_types"

/** Retry policies are client invocation concerns, not wire-plan metadata. */
export const OPERATION_RETRY_MODES = [
  "always",
  "never",
  "when_not_creating",
] as const

export type Api_Operation_Retry_Mode = (typeof OPERATION_RETRY_MODES)[number]

export interface Operation_Client_Metadata {
  readonly response_semantics?: string
  readonly retry_mode?: Api_Operation_Retry_Mode
  readonly scope?: string
}

export interface Operation_Client_Projection {
  readonly response_semantics: string
  readonly retry_mode: Api_Operation_Retry_Mode
}

export function derive_operation_client_projection(
  contract: Wire_Operation_Contract & Operation_Client_Metadata,
): Operation_Client_Projection {
  const response_framing = derive_wire_operation_descriptor(contract).response_framing
  return {
    response_semantics: contract.response_semantics ?? response_framing,
    retry_mode: contract.retry_mode ?? "always",
  }
}
