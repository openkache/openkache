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

const OPERATION_CONTRACT_TRAIT_ID = "openkache.protocol#operationContract"

/** Retry policies understood by this optional client projection. */
export const CLIENT_RETRY_MODES = [
  "always",
  "never",
  "when_not_creating",
] as const

export type Client_Retry_Mode = (typeof CLIENT_RETRY_MODES)[number]

/**
 * Optional client-only metadata layered over a neutral wire contract.
 *
 * The generic operation model and server never need to understand these
 * labels. A client adapter may provide any subset and resolve defaults here.
 */
export interface Api_Operation_Client_Projection {
  readonly response_semantics?: string
  readonly retry_mode?: Client_Retry_Mode
  readonly scope?: string
}

export interface Operation_Client_Projection {
  readonly response_semantics: string
  readonly retry_mode: Client_Retry_Mode
}

function extension_string(
  contract: Wire_Operation_Contract,
  member: string,
): string | undefined {
  const value =
    contract.extensions?.[`${OPERATION_CONTRACT_TRAIT_ID}.${member}`]
  if (value === undefined) return undefined
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(
      `${OPERATION_CONTRACT_TRAIT_ID}.${member} must be a non-empty string`,
    )
  }
  return value
}

function client_retry_mode(
  contract: Wire_Operation_Contract & Api_Operation_Client_Projection,
): Client_Retry_Mode {
  const value =
    contract.retry_mode ?? extension_string(contract, "retryMode") ?? "always"
  switch (value) {
    case "always":
    case "never":
    case "when_not_creating":
      return value
    default:
      throw new Error(
        `${OPERATION_CONTRACT_TRAIT_ID}.retryMode must be always, never, or when_not_creating`,
      )
  }
}

export function derive_operation_client_projection(
  contract: Wire_Operation_Contract & Api_Operation_Client_Projection,
): Operation_Client_Projection {
  const response_framing =
    derive_wire_operation_descriptor(contract).response_framing
  return {
    response_semantics:
      contract.response_semantics ??
      extension_string(contract, "responseSemantics") ??
      response_framing,
    retry_mode: client_retry_mode(contract),
  }
}
