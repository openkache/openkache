/** Operation-contract assembly over shape and request-wire plans. */

import {
  WIRE_REQUEST_FRAMINGS,
  WIRE_RESPONSE_FRAMINGS,
  type Wire_Contract_Adapter,
  type Wire_Entry,
  type Wire_Model_Request_Framing,
  type Wire_Operation,
  type Wire_Operation_Contract,
  type Wire_Response_Framing,
} from "../wire_types"
import { operation_shape_field_plan } from "./extract_contract_shapes"
import { request_wire_plan } from "./extract_contract_request_wire"
import {
  array_member,
  object_member,
  object_value,
  optional_boolean_member,
  optional_object_member,
  optional_string_member,
  string_member,
  type Json_Object,
} from "./validate_contract"

const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
const OPERATION_CONTRACT_TRAIT_ID = "openkache.protocol#operationContract"
const GENERIC_OPERATION_CONTRACT_MEMBERS = [
  "requestFraming",
  "requestWire",
  "responseFraming",
  "opaqueAggregate",
  "successStatuses",
  "errorStatuses",
] as const

function wire_name(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase()
}

/**
 * Preserves adapter-owned trait members without teaching the generic contract
 * about future API semantics.
 */
function operation_extensions(
  contract: Json_Object,
  operation_location: string,
  adapter: Wire_Contract_Adapter | undefined,
): Readonly<Record<string, unknown>> | undefined {
  const extensions: Record<string, unknown> = {}
  for (const [member, value] of Object.entries(contract)) {
    if (
      value !== undefined &&
      !GENERIC_OPERATION_CONTRACT_MEMBERS.includes(
        member as (typeof GENERIC_OPERATION_CONTRACT_MEMBERS)[number],
      )
    ) {
      extensions[`${OPERATION_CONTRACT_TRAIT_ID}.${member}`] = value
    }
  }
  const adapter_extensions = adapter?.extract_extensions?.(
    contract,
    operation_location,
  )
  if (adapter_extensions !== undefined) {
    Object.assign(extensions, adapter_extensions)
  }
  return Object.keys(extensions).length === 0 ? undefined : extensions
}

function operation_contract(
  shapes: Json_Object,
  shape: Json_Object,
  target: string,
  statuses: readonly Wire_Entry[],
  strict: boolean,
  adapter: Wire_Contract_Adapter | undefined,
): Wire_Operation_Contract | undefined {
  const traits = optional_object_member(shape, "traits", target)
  const value = traits?.[OPERATION_CONTRACT_TRAIT_ID]
  if (value === undefined) return undefined
  const contract = object_value(value, `${target}.traits.${OPERATION_CONTRACT_TRAIT_ID}`)
  const request_plan = operation_shape_field_plan(
    shapes,
    shape,
    target,
    "input",
  )
  const response_plan = operation_shape_field_plan(
    shapes,
    shape,
    target,
    "output",
  )
  const status_names = new Set(
    statuses.flatMap((status) => [
      status.name,
      status.text ?? wire_name(status.name),
    ]),
  )
  const status_values = (member: string): readonly string[] => {
    const values = array_member(
      contract,
      member,
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
    ).map((value, index) => {
      if (typeof value !== "string" || value.length === 0) {
        throw new Error(
          `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member}[${index}] must be a non-empty string`,
        )
      }
      if (!status_names.has(value)) {
        throw new Error(
          `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member}[${index}] references unknown protocol status ${value}`,
        )
      }
      return value
    })
    if (new Set(values).size !== values.length) {
      throw new Error(
        `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member} must not contain duplicate statuses`,
      )
    }
    if (values.length === 0) {
      throw new Error(
        `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member} must not be empty`,
      )
    }
    return values
  }
  const success_statuses = status_values("successStatuses")
  const error_statuses = status_values("errorStatuses")
  if (success_statuses.some((status) => error_statuses.includes(status))) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID} has overlapping success and error statuses`,
    )
  }
  const operation_location = `${target}.${OPERATION_CONTRACT_TRAIT_ID}`
  const request_wire = request_wire_plan(contract, request_plan, operation_location)
  const request_framing_value = optional_string_member(
    contract,
    "requestFraming",
    operation_location,
  )
  if (
    request_framing_value !== undefined &&
    !WIRE_REQUEST_FRAMINGS.includes(request_framing_value as Wire_Model_Request_Framing)
  ) {
    throw new Error(
      `${operation_location}.requestFraming must be empty, opaque, or ordered_fields`,
    )
  }
  const response_framing_value = optional_string_member(
    contract,
    "responseFraming",
    operation_location,
  )
  if (
    response_framing_value !== undefined &&
    !WIRE_RESPONSE_FRAMINGS.includes(response_framing_value as
      (typeof WIRE_RESPONSE_FRAMINGS)[number])
  ) {
    // Unknown response framing is an adapter-owned extension. The generic
    // descriptor preserves it as opaque metadata and never interprets its
    // prefix, sentinel, or field semantics. An explicit adapter may reject
    // unsupported extensions in validate_operation.
  }
  if (strict && request_framing_value === undefined) {
    throw new Error(
      `${operation_location}.requestFraming is required for strict protocol generation`,
    )
  }
  if (strict && response_framing_value === undefined) {
    throw new Error(
      `${operation_location}.responseFraming is required for strict protocol generation`,
    )
  }
  const opaque_aggregate = optional_boolean_member(
    contract,
    "opaqueAggregate",
    operation_location,
  )
  const extensions = operation_extensions(contract, operation_location, adapter)
  const derived_contract = {
    error_statuses,
    request_plan,
    ...(request_wire === undefined ? {} : { request_wire }),
    ...(request_framing_value === undefined
      ? {}
      : { request_framing: request_framing_value as Wire_Model_Request_Framing }),
    ...(extensions === undefined ? {} : { extensions }),
    response_plan,
    ...(response_framing_value === undefined
      ? {}
      : { response_framing: response_framing_value as Wire_Response_Framing }),
    ...(opaque_aggregate === undefined ? {} : { opaque_aggregate }),
    success_statuses,
  }
  const request_framing =
    request_framing_value as Wire_Model_Request_Framing | undefined
  if (request_framing === undefined) {
    throw new Error(
      `${operation_location}.requestFraming is required for protocol generation`,
    )
  }
  // Unknown operation-contract members remain opaque to generic framing.
  const response_framing: Wire_Response_Framing =
    response_framing_value === undefined
      ? (() => {
        if (response_plan.length === 0) return "empty"
        return response_plan.length === 1 ? "opaque" : "field_sequence"
      })()
      : response_framing_value as Wire_Response_Framing
  if (
    strict &&
    response_framing === "opaque" &&
    response_plan.length !== 1 &&
    opaque_aggregate !== true
  ) {
    throw new Error(
      `${operation_location}.responseFraming opaque requires exactly one modeled field`,
    )
  }
  const generic_response_framing = WIRE_RESPONSE_FRAMINGS.includes(
    response_framing as (typeof WIRE_RESPONSE_FRAMINGS)[number],
  )
  if (
    opaque_aggregate === true &&
    generic_response_framing &&
    response_framing !== "opaque"
  ) {
    throw new Error(
      `${operation_location}.opaqueAggregate requires responseFraming opaque`,
    )
  }
  if (opaque_aggregate === true && response_plan.length === 0) {
    throw new Error(
      `${operation_location}.opaqueAggregate requires at least one modeled response field`,
    )
  }
  switch (request_framing) {
    case "empty":
      if (request_plan.length !== 0) {
        throw new Error(
          `${operation_location}.requestFraming empty requires an empty request plan`,
        )
      }
      break
    case "opaque":
      if (request_plan.length !== 1) {
        throw new Error(
          `${operation_location}.requestFraming opaque requires exactly one modeled field`,
        )
      }
      break
    case "ordered_fields":
      if (request_plan.length === 0) {
        throw new Error(
          `${operation_location}.requestFraming ordered_fields requires at least one modeled field`,
        )
      }
      break
  }
  if (
    response_framing_value === "field_sequence" &&
    response_plan.length === 0
  ) {
    throw new Error(
      `${operation_location}.responseFraming ${response_framing_value} requires at least one modeled field`,
    )
  }
  const result: Wire_Operation_Contract = {
    ...derived_contract,
    request_framing,
    response_framing,
  }
  adapter?.validate_operation?.(result, operation_location)
  return result
}

/** Assembles operation contracts in opcode order without semantic branching. */
export function wire_operations(
  shapes: Json_Object,
  opcodes: readonly Wire_Entry[],
  statuses: readonly Wire_Entry[],
  strict: boolean,
  adapter: Wire_Contract_Adapter | undefined,
): readonly Wire_Operation[] | undefined {
  const operations: Wire_Operation[] = []
  for (const opcode of opcodes) {
    const target = `${SERVICE_SHAPE_ID.slice(0, SERVICE_SHAPE_ID.lastIndexOf("#"))}#${opcode.name}`
    const shape = shapes[target]
    if (shape === undefined) {
      if (strict) throw new Error(`opcode ${opcode.name} has no matching Smithy operation`)
      return undefined
    }
    const contract = operation_contract(
      shapes,
      object_value(shape, `Smithy AST.shapes.${target}`),
      target,
      statuses,
      strict,
      adapter,
    )
    if (contract === undefined) {
      if (strict) {
        throw new Error(`operation ${opcode.name} is missing ${OPERATION_CONTRACT_TRAIT_ID}`)
      }
      return undefined
    }
    operations.push({ contract, name: opcode.name })
  }
  return operations
}
