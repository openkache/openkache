/** Smithy extraction for the neutral OpenKache wire contract. */

import { existsSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import {
  WIRE_REQUEST_FRAMINGS,
  WIRE_RESPONSE_FRAMINGS,
  type Wire_Contract,
  type Wire_Contract_Adapter,
  type Wire_Entry,
  type Wire_Model_Request_Framing,
  type Wire_Operation,
  type Wire_Operation_Contract,
  type Wire_Operation_Descriptor,
  type Wire_Response_Framing,
} from "../wire_types"
import { request_wire_plan } from "./extract_contract_request_wire"
import { operation_shape_field_plan } from "./extract_contract_shapes"
import {
  array_member,
  integer_member,
  object_member,
  object_value,
  optional_boolean_member,
  optional_enum_value,
  optional_object_member,
  optional_string_member,
  string_member,
  trait_value,
  unique_wire_values,
  type Json_Object,
} from "./validate_contract"

/**
 * Computes an admission bound from the selected layout and codec widths.
 *
 * The protocol still enforces one aggregate value ceiling. This tighter
 * shape-derived bound prevents a fixed tuple or a small field sequence from
 * reserving the maximum value buffer for every in-flight request.
 */
const PROTOCOL_DIRECTORY = dirname(dirname(fileURLToPath(import.meta.url)))
const MODEL_DIRECTORY = "model"
const SMITHY_EXECUTABLE = process.env.OPENKACHE_SMITHY_EXECUTABLE ?? "smithy"
const SMITHY_USE_SHELL = process.env.OPENKACHE_SMITHY_USE_SHELL === "1"

function resolve_smithy_executable(): string {
  if (
    SMITHY_EXECUTABLE.length === 0 ||
    !SMITHY_EXECUTABLE.includes("/") ||
    SMITHY_EXECUTABLE.startsWith("/")
  ) {
    return SMITHY_EXECUTABLE
  }
  let directory = resolve(process.cwd())
  for (;;) {
    if (
      SMITHY_EXECUTABLE.startsWith("external/") &&
      existsSync(resolve(directory, "external"))
    ) {
      return resolve(directory, SMITHY_EXECUTABLE)
    }
    const candidate = resolve(directory, SMITHY_EXECUTABLE)
    if (existsSync(candidate)) return candidate
    const parent = dirname(directory)
    if (parent === directory) return SMITHY_EXECUTABLE
    directory = parent
  }
}
const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
const OPCODE_SHAPE_ID = "openkache.protocol#Opcode"
const STATUS_SHAPE_ID = "openkache.protocol#Status"
const WIRE_CONTRACT_TRAIT_ID = "openkache.protocol#wireContract"
const WIRE_OPCODE_TRAIT_ID = "openkache.protocol#wireOpcode"
const WIRE_STATUS_TRAIT_ID = "openkache.protocol#wireStatus"
const OPERATION_CONTRACT_TRAIT_ID = "openkache.protocol#operationContract"
const GENERIC_OPERATION_CONTRACT_MEMBERS = [
  "requestFraming",
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
 * Keeps operation-contract extensions available to an adapter without making
 * the generic wire contract enumerate every future semantic trait member.
 *
 * Generic fields are removed from this opaque map; every other member is
 * namespaced by the trait ID and remains uninterpreted until an API or
 * compatibility adapter opts into it.
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

function shape_name(shape_id: string): string {
  const separator = shape_id.lastIndexOf("#")
  if (separator < 0 || separator === shape_id.length - 1) {
    throw new Error(`shape ID ${JSON.stringify(shape_id)} has no shape name`)
  }
  return shape_id.slice(separator + 1)
}

function pascal_case(identifier: string): string {
  return identifier
    .split("_")
    .map((part) => {
      const normalized = part.toLowerCase()
      return normalized.length === 0
        ? ""
        : `${normalized[0]?.toUpperCase()}${normalized.slice(1)}`
    })
    .join("")
}


function wire_enum_entries(
  shapes: Json_Object,
  shape_id: string,
  trait_id: string,
  kind: string,
): readonly Wire_Entry[] {
  const enum_shape = object_member(shapes, shape_id, "Smithy AST.shapes")
  const members = object_member(enum_shape, "members", shape_id)
  const entries = Object.entries(members)
    .map(([name, member]): Wire_Entry => {
      const member_shape = object_value(member, `${shape_id}.${name}`)
      const trait = trait_value(member_shape, trait_id, `${shape_id}.${name}`)
      return {
        name: pascal_case(name),
        text: optional_enum_value(member_shape, `${shape_id}.${name}`),
        value: integer_member(
          trait,
          "value",
          `${shape_id}.${name}.${trait_id}`,
          0,
          0xff,
        ),
      }
    })
    .sort((left, right) => left.value - right.value)
  unique_wire_values(entries, kind)
  if (entries.length === 0) throw new Error(`${kind} contract must define at least one entry`)
  return entries
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
  const request_wire = request_wire_plan(
    contract,
    request_plan,
    `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
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
    !WIRE_RESPONSE_FRAMINGS.includes(response_framing_value as Wire_Response_Framing)
  ) {
    throw new Error(
      `${operation_location}.responseFraming must be empty, opaque, optional_values, or field_sequence`,
    )
  }
  if (
    strict &&
    request_framing_value === undefined
  ) {
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
  // Unknown operation-contract members are preserved as opaque extensions.
  // Compatibility adapters may interpret their own namespaced values, while
  // the canonical generic framing remains independent of those projections.
  const response_framing: Wire_Response_Framing =
    response_framing_value === undefined
      ? (() => {
        /*
         * Strict operation contracts declare responseFraming explicitly.
         * Permissive AST fixtures still get a shape-neutral fallback based
         * only on field cardinality; response semantic labels are owned by
         * compatibility adapters and never select generic wire bytes.
         */
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
  if (
    opaque_aggregate === true &&
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
  const request_plan_count = request_plan.length
  switch (request_framing) {
    case "empty":
      if (request_plan_count !== 0) {
        throw new Error(
          `${operation_location}.requestFraming empty requires an empty request plan`,
        )
      }
      break
    case "opaque":
      if (request_plan_count !== 1) {
        throw new Error(
          `${operation_location}.requestFraming opaque requires exactly one modeled field`,
        )
      }
      break
    case "ordered_fields":
      if (request_plan_count === 0) {
        throw new Error(
          `${operation_location}.requestFraming ordered_fields requires at least one modeled field`,
        )
      }
      break
  }
  if (
    (response_framing_value === "optional_values" ||
      response_framing_value === "field_sequence") &&
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

function wire_operations(
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

function model_operation_entries(
  shapes: Json_Object,
  service: Json_Object,
  opcodes: readonly Wire_Entry[],
  strict: boolean,
): readonly Wire_Entry[] {
  const declared_operations = service.operations
  if (!Array.isArray(declared_operations) || declared_operations.length === 0) {
    return opcodes
  }
  return declared_operations.map((operation, index) => {
    const reference = object_value(
      operation,
      `${SERVICE_SHAPE_ID}.operations[${index}]`,
    )
    const target = string_member(
      reference,
      "target",
      `${SERVICE_SHAPE_ID}.operations[${index}]`,
    )
    const name = pascal_case(shape_name(target))
    const entry = opcodes.find((opcode) => opcode.name === name)
    if (entry === undefined) {
      if (strict) {
        throw new Error(
          `modeled operation ${name} has no matching wire opcode entry`,
        )
      }
      return opcodes
    }
    // Resolve the operation shape here so malformed service declarations fail
    // before a runtime identity can be generated from them.
    object_member(shapes, target, "Smithy AST.shapes")
    return entry
  })
}

/** Extracts the server-visible wire contract from a Smithy AST. */
export function extract_wire_contract(
  ast: unknown,
  strict_operations = false,
  adapter?: Wire_Contract_Adapter,
): Wire_Contract {
  const ast_object = object_value(ast, "Smithy AST")
  const shapes = object_member(ast_object, "shapes", "Smithy AST")
  const service = object_member(shapes, SERVICE_SHAPE_ID, "Smithy AST.shapes")
  const contract_trait = trait_value(
    service,
    WIRE_CONTRACT_TRAIT_ID,
    `Smithy AST.shapes.${SERVICE_SHAPE_ID}`,
  )
  if (adapter === undefined) {
    throw new Error(
      "generic wire extraction requires an explicit transport-profile adapter",
    )
  }
  const opcode_shape = shapes[OPCODE_SHAPE_ID]
  const opcodes =
    opcode_shape === undefined
      ? array_member(service, "operations", SERVICE_SHAPE_ID)
          .map((operation, index): Wire_Entry => {
            const reference = object_value(operation, `${SERVICE_SHAPE_ID}.operations[${index}]`)
            const target = string_member(
              reference,
              "target",
              `${SERVICE_SHAPE_ID}.operations[${index}]`,
            )
            const operation_shape = object_member(shapes, target, "Smithy AST.shapes")
            const trait = trait_value(
              operation_shape,
              WIRE_OPCODE_TRAIT_ID,
              `Smithy AST.shapes.${target}`,
            )
            return {
              name: pascal_case(shape_name(target)),
              value: integer_member(
                trait,
                "value",
                `${target}.${WIRE_OPCODE_TRAIT_ID}`,
                0,
                0xff,
              ),
            }
          })
          .sort((left, right) => left.value - right.value)
      : wire_enum_entries(
          shapes,
          OPCODE_SHAPE_ID,
          WIRE_OPCODE_TRAIT_ID,
          "opcode",
        )
  unique_wire_values(opcodes, "opcode")
  if (opcodes.length === 0) throw new Error("opcode contract must define at least one entry")
  const model_opcodes = model_operation_entries(
    shapes,
    service,
    opcodes,
    strict_operations,
  )
  const statuses = wire_enum_entries(
    shapes,
    STATUS_SHAPE_ID,
    WIRE_STATUS_TRAIT_ID,
    "status",
  )
  const contract = {
    item_id_bytes: integer_member(contract_trait, "itemIdBytes", "wireContract", 1),
    max_value_bytes: integer_member(contract_trait, "maxValueBytes", "wireContract", 1),
    model_opcodes,
    opcodes,
    statuses,
    v1: adapter.extract_profile(
      contract_trait.v1,
      `${WIRE_CONTRACT_TRAIT_ID}.v1`,
    ),
  }
  const operations = wire_operations(
    shapes,
    opcodes,
    statuses,
    strict_operations,
    adapter,
  )
  return operations === undefined ? contract : { ...contract, operations }
}

/** Loads the protocol Smithy AST from the model owned by this directory. */
export function smithy_wire_ast(): unknown {
  const smithy_executable = resolve_smithy_executable()
  const smithy_command =
    SMITHY_USE_SHELL && process.platform !== "win32"
      ? ["sh", smithy_executable, "ast", MODEL_DIRECTORY]
      : [smithy_executable, "ast", MODEL_DIRECTORY]
  const result = Bun.spawnSync(smithy_command, {
    cwd: PROTOCOL_DIRECTORY,
    stderr: "pipe",
    stdout: "pipe",
  })
  if (result.exitCode !== 0) {
    const diagnostics = result.stderr.toString().trim()
    throw new Error(
      diagnostics.length === 0
        ? "`smithy ast` exited without diagnostics"
        : `smithy AST generation failed:\n${diagnostics}`,
    )
  }
  try {
    return JSON.parse(result.stdout.toString()) as unknown
  } catch (error) {
    throw new Error(`smithy emitted invalid JSON: ${String(error)}`)
  }
}
