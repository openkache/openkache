/** Shared operation plans, codecs, and framing used by language renderers. */

import type { Wire_Entry } from "../../../protocol/wire"
import type {
  Api_Contract,
  Api_Member,
  Api_Operation,
  Api_Operation_Contract,
  Api_Structure,
  Operation_Field_Role,
} from "../../operation_models"
import type { Client_Contract } from "../../client_contract"
import type { Operation_Result_Plan } from "../../operation_results"
import type { Operation_Result_Projection } from "../../compatibility_result_projections"
import type { Request_Transport_Plan } from "../../compatibility_request_adapters"
import {
  operation_field_count,
  type Operation_Field_Plan,
  type Operation_Field_Requirement,
  type Operation_Plan,
} from "../../operation_plans"

import type { Operation_Field_Binding } from "./contract"
import { wire_codec_for_field, type Application_Value_Codec, type Application_Value_Codec_Pair } from "./registry"

export function managed_result_projection(
  operation: Managed_Api_Operation,
): Operation_Result_Projection {
  return operation.plan.result_projection
}

export function operation_result_kind_constants(
  operation: Managed_Api_Operation,
  prefix = "SMITHY_FFI_RESULT_",
): string {
  return managed_result_projection(operation).result_kinds
    .map((kind) => `${prefix}${kind.toUpperCase()}`)
    .join(", ")
}

/** Returns the native result constants actually referenced by generated methods. */
export function operation_result_kind_imports(
  operations: readonly Managed_Api_Operation[],
): readonly string[] {
  const imports = new Set<string>()
  for (const operation of operations) {
    for (const kind of managed_result_projection(operation).result_kinds) {
      imports.add(`SMITHY_FFI_RESULT_${kind.toUpperCase()}`)
    }
  }
  return [...imports].sort()
}

/** One language-neutral operation plan shared by every source renderer. */
export interface Managed_Operation_Plan {
  readonly api: Api_Contract
  readonly binding: Operation_Field_Binding
  readonly application_value_codecs?: Application_Value_Codec_Pair
  readonly contract: Api_Operation_Contract
  readonly input: string
  readonly invocation: Operation_Invocation_Plan
  readonly name: string
  readonly opcode: Wire_Entry
  readonly operation: Operation_Plan
  readonly output: string
  readonly request: Request_Transport_Plan
  readonly result_plan: Operation_Result_Plan
  readonly result_projection: Operation_Result_Projection
  readonly required_fields: readonly Operation_Field_Requirement[]
  readonly strict_operation_bindings: boolean
}

export interface Managed_Api_Operation extends Api_Operation {
  readonly binding: Operation_Field_Binding
  readonly contract: Api_Operation_Contract
  readonly opcode: Wire_Entry
  readonly plan: Managed_Operation_Plan
}

/**
 * The request-side transport call is resolved once from the descriptor.
 *
 * `compatibility` is deliberately a dead end for generic renderers: compact
 * protocol-v1 calls still need their compatibility adapter, while every
 * other request is expressed as a descriptor-built payload (including an
 * explicitly empty body). Keeping this as a plan prevents each language
 * renderer from rediscovering the same framing decision beside its response
 * projection.
 */
export interface Operation_Invocation_Plan {
  /**
   * `compatibility` is an adapter-owned request projection.  Generic
   * renderers intentionally do not distinguish which historical route the
   * adapter uses; they only know that the adapter owns invocation syntax.
   */
  readonly request: "generic" | "compatibility"
}

/**
 * A codec name is model data, not a generator union.  The registry is
 * deliberately open-ended so a new `@wireCodec` value can be introduced
 * without editing a central list of operation families.
 */

export function operation_structure(
  contract: Api_Contract,
  operation: Api_Operation,
  direction: "input" | "output",
): Api_Structure {
  const name = direction === "input" ? operation.input : operation.output
  const structure = contract.structures.find((candidate) => candidate.name === name)
  if (structure === undefined) {
    throw new Error(
      `operation ${operation.name} ${direction} structure ${name} is missing from the Smithy contract`,
    )
  }
  return structure
}

export function operation_field_binding(
  contract: Client_Contract,
  operation: Api_Operation,
): Operation_Field_Binding {
  const fields = (
    direction: "input" | "output",
  ): Partial<Record<Operation_Field_Role, readonly Api_Member[]>> => {
    const bound: Partial<Record<Operation_Field_Role, Api_Member[]>> = {}
    for (const member of operation_structure(contract.api, operation, direction).members) {
      const role = member.operation_field_role
      if (role === undefined) continue
      bound[role] = [...(bound[role] ?? []), member]
    }
    return bound
  }
  const input = fields("input")
  const output = fields("output")
  return { input, output }
}

/** Returns the ordered fields carried by a generic field-sequence output. */
export function operation_composite_fields(
  operation: Managed_Api_Operation,
): readonly Operation_Field_Plan[] {
  const fields = operation.plan.operation.output.fields
  if (fields.length === 0) {
    throw new Error(
      `operation ${operation.name} field-sequence output must define at least one modeled operation field`,
    )
  }
  return fields
}

/** Resolves each composite field through the shared payload codec registry. */
export function operation_composite_field_codec(
  operation: Managed_Api_Operation,
  field: Operation_Field_Plan,
): Application_Value_Codec {
  try {
    return wire_codec_for_field(operation, field).name
  } catch (error) {
    throw new Error(
      `operation ${operation.name} composite field ${field.name} has no registered wire codec: ${
        error instanceof Error ? error.message : String(error)
      }`,
    )
  }
}

export function operation_composite_value_count(operation: Managed_Api_Operation): number {
  return operation_composite_fields(operation).length
}

/** Returns whether a route-less request uses the generic ordered-field body. */
export function operation_uses_generic_field_sequence_request(
  operation: Managed_Api_Operation,
): boolean {
  return operation.plan.request.compact_adapter === undefined &&
    operation.plan.request.request_framing === "ordered_fields"
}

/** Returns whether this operation needs the compact ordered-field helpers. */
export function operation_uses_field_sequence_helpers(
  operation: Managed_Api_Operation,
): boolean {
  return operation_uses_generic_field_sequence_request(operation) ||
    operation.plan.contract.response_framing === "field_sequence"
}

/** Returns whether this operation selected the explicit optional-value layout. */
export function operation_uses_optional_value_layout(
  operation: Managed_Api_Operation,
): boolean {
  return operation.plan.contract.response_framing === "optional_values"
}

/** Returns whether this operation uses the compact multi-item compatibility route. */
export function operation_uses_item_id_helpers(
  operation: Managed_Api_Operation,
): boolean {
  return operation_field_count(operation.plan.operation, "input", "item_id") > 1
}

/**
 * One framing descriptor feeds every generated optional-value decoder.
 *
 * The language snippets are necessarily written in each target language, but
 * their width, sentinel, aggregate value limit, and byte order come from this
 * one contract projection.  A framing change therefore cannot silently drift
 * in one adapter.
 */
