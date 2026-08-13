/** Smithy extraction for the neutral transport wire contract. */

import { existsSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import { type Wire_Contract, type Wire_V1_Contract } from "../wire_types"
import {
  integer_member,
  object_member,
  object_value,
  string_member,
  trait_value,
  type Json_Object,
} from "./validate_contract"

const PROTOCOL_DIRECTORY = dirname(dirname(fileURLToPath(import.meta.url)))
const MODEL_DIRECTORY = "model"
const SMITHY_EXECUTABLE = process.env.OPENKACHE_SMITHY_EXECUTABLE ?? "smithy"
const SMITHY_USE_SHELL = process.env.OPENKACHE_SMITHY_USE_SHELL === "1"
const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
const WIRE_CONTRACT_TRAIT_ID = "openkache.protocol#wireContract"

function wire_v1_contract(value: unknown): Wire_V1_Contract {
  const contract = object_value(value, `${WIRE_CONTRACT_TRAIT_ID}.v1`)
  const v1 = {
    alpn: string_member(contract, "alpn", "wireContract.v1"),
    request_code_bytes: integer_member(
      contract,
      "requestCodeBytes",
      "wireContract.v1",
      1,
      1,
    ),
    response_code_bytes: integer_member(
      contract,
      "responseCodeBytes",
      "wireContract.v1",
      1,
      1,
    ),
    min_varuint_bytes: integer_member(
      contract,
      "minVaruintBytes",
      "wireContract.v1",
      1,
      1,
    ),
    max_varuint_bytes: integer_member(contract, "maxVaruintBytes", "wireContract.v1", 9, 9),
  } satisfies Wire_V1_Contract
  if (v1.alpn.length === 0) throw new Error("wire v1 ALPN must not be empty")
  if (v1.min_varuint_bytes > v1.max_varuint_bytes) {
    throw new Error("wire v1 minimum varuint width exceeds its maximum")
  }
  return v1
}

/** Extracts only transport identifiers and framing constants. */
export function extract_wire_contract(
  ast: unknown,
): Wire_Contract {
  const ast_object = object_value(ast, "Smithy AST")
  const shapes = object_member(ast_object, "shapes", "Smithy AST")
  const service = object_member(shapes, SERVICE_SHAPE_ID, "Smithy AST.shapes")
  const contract_trait = trait_value(
    service,
    WIRE_CONTRACT_TRAIT_ID,
    `Smithy AST.shapes.${SERVICE_SHAPE_ID}`,
  )
  return {
    max_payload_bytes: integer_member(
      contract_trait,
      "maxPayloadBytes",
      "wireContract",
      1,
    ),
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
