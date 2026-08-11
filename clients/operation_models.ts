/**
 * Language-neutral Smithy operation model used by contract extraction and
 * every generated-client renderer.
 *
 * Keeping these types outside `generate.ts` prevents the operation IR from
 * depending on the renderer entry point. New operation shapes therefore meet
 * the generator through data, not through renderer-specific helpers.
 */

import type { Wire_Operation_Contract } from "../protocol/wire_types"

export type Api_Type_Kind =
  | "blob"
  | "boolean"
  | "double"
  | "enum"
  | "integer"
  | "list"
  | "long"
  | "map"
  | "string"
  | "structure"
  | "union"
  | "unsigned_long"

/**
 * Operation roles are model data. Keep the string boundary so adding a
 * model-only role does not require a generator union edit before extraction
 * can preserve it.
 */
export type Operation_Field_Role = string

/** Retry policies exposed by the client compatibility adapters. */
export const OPERATION_RETRY_MODES = [
  "always",
  "never",
  "when_not_creating",
] as const

export type Api_Operation_Retry_Mode = (typeof OPERATION_RETRY_MODES)[number]

/**
 * Client-owned operation semantics layered over the generic wire contract.
 *
 * These labels are deliberately absent from `Wire_Operation_Contract`: server
 * framing and codec infrastructure must not acquire client retry policy,
 * scope vocabulary, or ergonomic result names merely because one client
 * adapter uses them.
 */
export interface Api_Operation_Contract extends Wire_Operation_Contract {
  /** Open result label interpreted by an optional client adapter. */
  readonly response_semantics?: string
  /** Retry policy interpreted by a client invocation adapter. */
  readonly retry_mode: Api_Operation_Retry_Mode
  /** API-owned invocation scope label. */
  readonly scope: string
}

/** One resolved Smithy API field type. */
export interface Api_Type {
  readonly kind: Api_Type_Kind
  /** Smithy string values for an enum codec. */
  readonly enum_values?: readonly string[]
  /** Smithy map key type. */
  readonly key?: Api_Type
  readonly member?: Api_Type
  readonly name?: string
  /** Smithy map value type. */
  readonly value?: Api_Type
  /** Optional model-declared codec name resolved by the shared registry. */
  readonly wire_codec?: string
}

/** One field in a Smithy operation input or output structure. */
export interface Api_Member {
  readonly name: string
  readonly operation_field_role?: Operation_Field_Role
  readonly required: boolean
  readonly type: Api_Type
}

/** One Smithy operation input or output structure. */
export interface Api_Structure {
  readonly members: readonly Api_Member[]
  readonly name: string
}

/** One string-valued Smithy enum member. */
export interface Api_Enum_Member {
  readonly name: string
  readonly value: string
}

/** One string-valued Smithy API enum. */
export interface Api_Enum {
  readonly members: readonly Api_Enum_Member[]
  readonly name: string
}

/** One operation exposed by the Smithy service. */
export interface Api_Operation {
  readonly contract?: Api_Operation_Contract
  readonly input: string
  readonly name: string
  readonly output: string
}

/** Language-neutral service API extracted from the Smithy model. */
export interface Api_Contract {
  readonly enums: readonly Api_Enum[]
  readonly operations: readonly Api_Operation[]
  readonly structures: readonly Api_Structure[]
}
