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
  Api_Operation_Retry_Mode,
  Api_Operation_Contract,
} from "./operation_models"
import type {
  Wire_Operation_Contract,
} from "../protocol/wire_types"

export interface Operation_Client_Projection {
  readonly response_semantics: string
  readonly retry_mode: Api_Operation_Retry_Mode
}

type Client_Projection_Metadata = Pick<
  Api_Operation_Contract,
  "response_semantics" | "retry_mode"
>

export function derive_operation_client_projection(
  contract: Wire_Operation_Contract & Partial<Client_Projection_Metadata>,
): Operation_Client_Projection {
  const response_framing = derive_wire_operation_descriptor(contract).response_framing
  return {
    response_semantics: contract.response_semantics ?? response_framing,
    retry_mode: contract.retry_mode ?? "always",
  }
}
