/** Names owned by the protocol-v1 client compatibility projection. */

/**
 * Client/API semantic labels preserved only for the compatibility/documentation
 * projection. They are not part of the generic wire operation contract.
 */
export const PROTOCOL_V1_RESPONSE_SEMANTICS_EXTENSION =
  "openkache.protocol#operationContract.responseSemantics" as const
export const PROTOCOL_V1_SCOPE_EXTENSION =
  "openkache.protocol#operationContract.scope" as const
export const PROTOCOL_V1_RETRY_MODE_EXTENSION =
  "openkache.protocol#operationContract.retryMode" as const

/**
 * Response labels retained for compatibility projections and documentation.
 *
 * Generic code should consume `Wire_Response_Framing` and the open semantic
 * label instead of matching these names.
 */
export const WIRE_RESPONSE_ROUTES = [
  "empty",
  "pong",
  "application_value",
  "field_sequence",
  "composite",
  "value",
  "set_outcome",
  "delete_outcome",
  "stats_json",
  "namespace_descriptor",
] as const

export type Wire_Response_Route = (typeof WIRE_RESPONSE_ROUTES)[number]
