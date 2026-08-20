/** Shared operation plans, codecs, and framing used by language renderers. */

import type { Api_Type } from "../../operation_models"
import type { Client_Contract } from "../../client_contract"
import type { Operation_Field_Plan } from "../../operation_plans"

import { go_api_name } from "./contract"
import { WIRE_CODEC_REGISTRY, type Application_Value_Codec, type Application_Value_Language } from "./registry"
import { operation_composite_field_codec, type Managed_Api_Operation } from "./shapes"

export interface Optional_Value_Framing {
  readonly encoding: "big_endian"
  readonly length_bytes: number
  readonly max_encoded_entry_bytes: number
  readonly max_value_bytes: number
  readonly missing_sentinel: number
}

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

/** Descriptor for the generic compact field-sequence primitive. */
export interface Field_Sequence_Framing {
  readonly max_value_bytes: number
}

export function field_sequence_framing(
  contract: Pick<Client_Contract, "max_value_bytes">,
): Field_Sequence_Framing {
  return { max_value_bytes: contract.max_value_bytes }
}

/**
 * Renders a nullable decode for one field in the optional-value sequence.
 * The sequence is always byte-oriented; the registered field codec supplies
 * the language-specific conversion after the missing sentinel is handled.
 */
export function render_composite_field_decode(
  language: Application_Value_Language,
  codec: Application_Value_Codec,
  payload: string,
  diagnostic: string,
  required: boolean,
  type?: Api_Type,
): string {
  const registration = WIRE_CODEC_REGISTRY.find((candidate) =>
    candidate.name === codec
  )
  if (registration === undefined) {
    throw new Error(`wire codec ${JSON.stringify(codec)} has no renderer registration`)
  }
  const required_payload = required
    ? (() => {
        switch (language) {
          case "java":
            return `Objects.requireNonNull(${payload}, ${diagnostic} + " required field is missing")`
          case "kotlin":
            return `${payload}!!`
          case "dart":
          case "typescript":
          case "swift":
          case "csharp":
            return `${payload}!`
          case "rust":
            return `${payload}.ok_or_else(|| Error::Protocol(format!("{} response required field is missing", ${diagnostic})))?`
          case "go":
          case "python":
            return payload
        }
      })()
    : payload
  return required
    ? registration.render(language, "value", required_payload, diagnostic, undefined, undefined, type).decode
    : registration.render_optional(language, payload, diagnostic, type)
}

export interface Rendered_Go_Composite_Field {
  readonly expression: string
  readonly statements: string
}

/**
 * Renders one Go composite field through a statement boundary.
 *
 * Go decoders for non-opaque codecs return `(value, error)`, which cannot be
 * embedded directly in a struct literal. Keeping that boundary here lets the
 * operation renderer stay shape-driven while preserving normal error
 * propagation for every registered codec.
 */
export function render_go_composite_field(
  operation: Managed_Api_Operation,
  field: Operation_Field_Plan,
  index: number,
  diagnostic: string,
): Rendered_Go_Composite_Field {
  const codec = operation_composite_field_codec(operation, field)
  const registration = WIRE_CODEC_REGISTRY.find((candidate) =>
    candidate.name === codec
  )
  if (registration === undefined) {
    throw new Error(`wire codec ${JSON.stringify(codec)} has no renderer registration`)
  }
  const payload = `values[${index}]`
  const decoded = `decodedValue${index}`
  const rendered = registration.render_go_optional(
    payload,
    decoded,
    diagnostic,
    go_api_name(operation.output),
    field.type,
  )
  if (!field.required) return rendered
  return {
    expression: `*${decoded}`,
    statements: `${rendered.statements}
		if ${decoded} == nil {
			return ${go_api_name(operation.output)}{}, operationError(${diagnostic}, fmt.Errorf("required field is missing"))
		}`,
  }
}
