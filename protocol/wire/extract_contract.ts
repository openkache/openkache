/** Smithy extraction for the neutral transport wire contract. */

import { existsSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import {
  type Wire_Contract,
  type Wire_Entry,
  type Wire_V1_Contract,
} from "../wire_types"
import {
  array_member,
  integer_member,
  object_member,
  object_value,
  optional_enum_value,
  string_member,
  trait_value,
  unique_wire_values,
  type Json_Object,
} from "./validate_contract"

const PROTOCOL_DIRECTORY = dirname(dirname(fileURLToPath(import.meta.url)))
const MODEL_DIRECTORY = "model"
const SMITHY_EXECUTABLE = process.env.OPENKACHE_SMITHY_EXECUTABLE ?? "smithy"
const SMITHY_USE_SHELL = process.env.OPENKACHE_SMITHY_USE_SHELL === "1"
const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
const OPCODE_SHAPE_ID = "openkache.protocol#Opcode"
const STATUS_SHAPE_ID = "openkache.protocol#Status"
const WIRE_CONTRACT_TRAIT_ID = "openkache.protocol#wireContract"
const WIRE_OPCODE_TRAIT_ID = "openkache.protocol#wireOpcode"
const WIRE_STATUS_TRAIT_ID = "openkache.protocol#wireStatus"

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

function operation_enum_entries(
  shapes: Json_Object,
  service: Json_Object,
): readonly Wire_Entry[] {
  const opcode_shape = shapes[OPCODE_SHAPE_ID]
  if (opcode_shape !== undefined) {
    return wire_enum_entries(
      shapes,
      OPCODE_SHAPE_ID,
      WIRE_OPCODE_TRAIT_ID,
      "opcode",
    )
  }
  const operations = array_member(service, "operations", SERVICE_SHAPE_ID)
    .map((operation, index): Wire_Entry => {
      const reference = object_value(
        operation,
        `${SERVICE_SHAPE_ID}.operations[${index}]`,
      )
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
  unique_wire_values(operations, "opcode")
  return operations
}

function wire_v1_contract(value: unknown): Wire_V1_Contract {
  const contract = object_value(value, `${WIRE_CONTRACT_TRAIT_ID}.v1`)
  const v1 = {
    alpn: string_member(contract, "alpn", "wireContract.v1"),
    opcode_bytes: integer_member(contract, "opcodeBytes", "wireContract.v1", 1, 0xff),
    status_bytes: integer_member(contract, "statusBytes", "wireContract.v1", 1, 0xff),
    request_fixed_bytes: integer_member(
      contract,
      "requestFixedBytes",
      "wireContract.v1",
      0,
      0xff,
    ),
    response_fixed_bytes: integer_member(
      contract,
      "responseFixedBytes",
      "wireContract.v1",
      0,
      0xff,
    ),
    min_varuint_bytes: integer_member(
      contract,
      "minVaruintBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    max_varuint_bytes: integer_member(contract, "maxVaruintBytes", "wireContract.v1", 1),
  } satisfies Wire_V1_Contract
  if (v1.alpn.length === 0) throw new Error("wire v1 ALPN must not be empty")
  if (
    v1.opcode_bytes !== 1 ||
    v1.status_bytes !== 1 ||
    v1.request_fixed_bytes !== 1 ||
    v1.response_fixed_bytes !== 1
  ) {
    throw new Error(
      "wire v1 currently supports exactly one opcode/status/fixed framing byte",
    )
  }
  if (v1.min_varuint_bytes > v1.max_varuint_bytes) {
    throw new Error("wire v1 minimum varuint width exceeds its maximum")
  }
  return v1
}

/** Extracts only transport identifiers and framing constants. */
export function extract_wire_contract(
  ast: unknown,
  _strict_operations = false,
): Wire_Contract {
  const ast_object = object_value(ast, "Smithy AST")
  const shapes = object_member(ast_object, "shapes", "Smithy AST")
  const service = object_member(shapes, SERVICE_SHAPE_ID, "Smithy AST.shapes")
  const contract_trait = trait_value(
    service,
    WIRE_CONTRACT_TRAIT_ID,
    `Smithy AST.shapes.${SERVICE_SHAPE_ID}`,
  )
  const opcodes = operation_enum_entries(shapes, service)
  if (opcodes.length === 0) throw new Error("opcode contract must define at least one entry")
  const statuses = wire_enum_entries(
    shapes,
    STATUS_SHAPE_ID,
    WIRE_STATUS_TRAIT_ID,
    "status",
  )
  return {
    item_id_bytes: integer_member(contract_trait, "itemIdBytes", "wireContract", 1),
    max_value_bytes: integer_member(contract_trait, "maxValueBytes", "wireContract", 1),
    opcodes,
    statuses,
    v1: wire_v1_contract(contract_trait.v1),
  }
}

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
        : `Smithy AST generation failed:\n${diagnostics}`,
    )
  }
  try {
    return JSON.parse(result.stdout.toString()) as unknown
  } catch (error) {
    throw new Error(`Smithy emitted invalid JSON: ${String(error)}`)
  }
}
