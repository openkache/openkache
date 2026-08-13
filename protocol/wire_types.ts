/** Values extracted from the transport wire contract. */

/** One numeric protocol member assigned by the wire contract. */
export interface Wire_Entry {
  readonly name: string
  /** Optional Smithy enum value used for generated labels. */
  readonly text?: string
  readonly value: number
}

/**
 * Constants shared by the transport framing.
 *
 * This type intentionally contains no operation layout, field role, codec,
 * route, status projection, retry policy, or API semantic metadata. Those
 * choices belong to the API module that owns a request or response codec.
 */
export interface Wire_V1_Contract {
  readonly alpn: string
  readonly opcode_bytes: number
  readonly status_bytes: number
  readonly request_fixed_bytes: number
  readonly response_fixed_bytes: number
  readonly min_varuint_bytes: number
  readonly max_varuint_bytes: number
}

/** Values shared by all API modules using this transport. */
export interface Wire_Contract {
  readonly item_id_bytes: number
  readonly max_value_bytes: number
  readonly opcodes: readonly Wire_Entry[]
  readonly statuses: readonly Wire_Entry[]
  readonly v1: Wire_V1_Contract
}

/**
 * A caller-owned request-frame description.
 *
 * This is a validation/tooling shape, not generated operation metadata. API
 * code may author one explicitly when it wants the shared incremental frame
 * parser; the transport never infers it from an operation model.
 */
export type Wire_Request_Step =
  | { readonly kind: "fixed"; readonly bytes: number }
  | { readonly kind: "body"; readonly bytes: number }
  | { readonly kind: "varuint" }
  | { readonly kind: "byte_length" }

export interface Wire_Request_Layout {
  readonly steps: readonly Wire_Request_Step[]
}
