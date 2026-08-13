/** Values extracted from the transport wire contract. */

/**
 * Constants shared by every API module using the transport.
 *
 * Operation codes, response codes, field layouts, and semantic meanings are
 * deliberately absent. Those values belong to the API that owns them.
 */
export interface Wire_V1_Contract {
  readonly alpn: string
  readonly request_code_bytes: number
  readonly response_code_bytes: number
  readonly min_varuint_bytes: number
  readonly max_varuint_bytes: number
}

/** Values shared by every API module using this transport. */
export interface Wire_Contract {
  readonly max_payload_bytes: number
  readonly v1: Wire_V1_Contract
}

/** Byte-level request framing metadata authored by an API contract. */
export type Wire_Request_Step =
  | { readonly kind: "fixed"; readonly bytes: number }
  | { readonly kind: "fixed_body"; readonly bytes: number }
  | { readonly kind: "payload_length" }
  | {
      readonly kind: "conditional_varuint"
      readonly selector_offset: number
      readonly mask: number
      readonly expected: number
    }
  | { readonly kind: "byte_length" }
  | {
      readonly kind: "byte_then_varuint"
      readonly prefix_bytes: number
      readonly mask: number
      readonly expected: number
    }
  | {
      readonly kind: "conditional_byte_then_varuint"
      readonly selector_offset: number
      readonly mask: number
      readonly expected: number
      readonly prefix_bytes: number
      readonly value_mask: number
      readonly value_expected: number
    }

/** Caller-owned request framing description for API-local tooling. */
export interface Wire_Request_Layout {
  readonly steps: readonly Wire_Request_Step[]
}
