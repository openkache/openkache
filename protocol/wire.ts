/** Smithy extraction and rendering for the server-visible OpenKache wire contract. */

import { dirname, resolve } from "node:path"
import { existsSync } from "node:fs"
import { fileURLToPath } from "node:url"

type Json_Object = Readonly<Record<string, unknown>>

/** One numeric protocol member assigned by the wire contract. */
export interface Wire_Entry {
  readonly name: string
  readonly value: number
}

/** Protocol v1 constants consumed by the Rust protocol crate. */
export interface Wire_V1_Contract {
  readonly alpn: string
  readonly request_fixed_bytes: number
  readonly response_fixed_bytes: number
  readonly max_varuint_bytes: number
  readonly set_ttl_flag: number
  readonly set_if_absent_flag: number
  readonly set_if_present_flag: number
}

/** Language-neutral server-visible subset of the OpenKache Smithy model. */
export interface Wire_Contract {
  readonly item_id_bytes: number
  readonly max_value_bytes: number
  readonly opcodes: readonly Wire_Entry[]
  readonly statuses: readonly Wire_Entry[]
  readonly v1: Wire_V1_Contract
}

const PROTOCOL_DIRECTORY = dirname(fileURLToPath(import.meta.url))
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

function object_value(value: unknown, location: string): Json_Object {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${location} must be an object`)
  }
  return value as Json_Object
}

function object_member(object: Json_Object, member: string, location: string): Json_Object {
  return object_value(object[member], `${location}.${member}`)
}

function array_member(
  object: Json_Object,
  member: string,
  location: string,
): readonly unknown[] {
  const value = object[member]
  if (!Array.isArray(value)) throw new Error(`${location}.${member} must be an array`)
  return value
}

function string_member(object: Json_Object, member: string, location: string): string {
  const value = object[member]
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${location}.${member} must be a non-empty string`)
  }
  return value
}

function integer_member(
  object: Json_Object,
  member: string,
  location: string,
  minimum = 0,
  maximum = Number.MAX_SAFE_INTEGER,
): number {
  const value = object[member]
  if (
    typeof value !== "number" ||
    !Number.isSafeInteger(value) ||
    value < minimum ||
    value > maximum
  ) {
    throw new Error(
      `${location}.${member} must be an integer from ${minimum} through ${maximum}`,
    )
  }
  return value
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

function trait_value(
  shape: Json_Object,
  trait_id: string,
  location: string,
): Json_Object {
  const traits = object_member(shape, "traits", location)
  return object_member(traits, trait_id, `${location}.traits`)
}

function unique_wire_values(entries: readonly Wire_Entry[], kind: string): void {
  const names = new Set<string>()
  const values = new Set<number>()
  for (const entry of entries) {
    if (names.has(entry.name)) throw new Error(`duplicate ${kind} name ${entry.name}`)
    if (values.has(entry.value)) {
      throw new Error(`duplicate ${kind} wire value ${entry.value}`)
    }
    names.add(entry.name)
    values.add(entry.value)
  }
}

function wire_v1_contract(value: unknown): Wire_V1_Contract {
  const contract = object_value(value, `${WIRE_CONTRACT_TRAIT_ID}.v1`)
  const v1 = {
    alpn: string_member(contract, "alpn", "wireContract.v1"),
    request_fixed_bytes: integer_member(contract, "requestFixedBytes", "wireContract.v1", 1),
    response_fixed_bytes: integer_member(contract, "responseFixedBytes", "wireContract.v1", 1),
    max_varuint_bytes: integer_member(contract, "maxVaruintBytes", "wireContract.v1", 1),
    set_ttl_flag: integer_member(contract, "setTtlFlag", "wireContract.v1", 0, 0xff),
    set_if_absent_flag: integer_member(
      contract,
      "setIfAbsentFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_if_present_flag: integer_member(
      contract,
      "setIfPresentFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
  } satisfies Wire_V1_Contract
  if (v1.alpn !== "openkache/1") {
    throw new Error(
      `${WIRE_CONTRACT_TRAIT_ID}.v1.alpn must be "openkache/1" for the current protocol implementation`,
    )
  }
  if (v1.request_fixed_bytes !== 2 || v1.response_fixed_bytes !== 1) {
    throw new Error(
      `${WIRE_CONTRACT_TRAIT_ID}.v1 fixed header sizes must be request=2 and response=1`,
    )
  }
  if (v1.max_varuint_bytes !== 9) {
    throw new Error(
      `${WIRE_CONTRACT_TRAIT_ID}.v1.maxVaruintBytes must be 9 for the unsigned 64-bit protocol`,
    )
  }
  const flags = [
    { name: "SetTtl", value: v1.set_ttl_flag },
    { name: "SetIfAbsent", value: v1.set_if_absent_flag },
    { name: "SetIfPresent", value: v1.set_if_present_flag },
  ] as const
  unique_wire_values(flags, "SET flag")
  if (flags.some(({ value }) => value === 0 || (value & (value - 1)) !== 0)) {
    throw new Error("SET flags must each contain exactly one non-zero bit")
  }
  return v1
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

/** Extracts the server-visible wire contract from a Smithy AST. */
export function extract_wire_contract(ast: unknown): Wire_Contract {
  const ast_object = object_value(ast, "Smithy AST")
  const shapes = object_member(ast_object, "shapes", "Smithy AST")
  const service = object_member(shapes, SERVICE_SHAPE_ID, "Smithy AST.shapes")
  const contract_trait = trait_value(
    service,
    WIRE_CONTRACT_TRAIT_ID,
    `Smithy AST.shapes.${SERVICE_SHAPE_ID}`,
  )
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

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function formatted_byte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`
}

function rust_byte_string_literal(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let literal = 'b"'
  for (const byte of bytes) {
    if (byte >= 0x20 && byte <= 0x7e && byte !== 0x22 && byte !== 0x5c) {
      literal += String.fromCharCode(byte)
    } else {
      literal += `\\x${byte.toString(16).padStart(2, "0")}`
    }
  }
  return `${literal}"`
}

function rust_wire_enum(
  name: string,
  documentation: string,
  entries: readonly Wire_Entry[],
  unknown_variant: string,
): string {
  const variants = entries
    .map((entry) => `        ${entry.name} = ${formatted_byte(entry.value)},`)
    .join("\n")
  return `wire_enum! {
    /// ${documentation}
    pub enum ${name} {
${variants}
    }
    unknown => ${unknown_variant}
}`
}

/** Renders protocol v1 Rust definitions without client-only declarations. */
export function render_rust_wire(contract: Wire_Contract): string {
  return `// Generated from the OpenKache Smithy wire contract. Do not edit.

/// QUIC application protocol identifier for wire protocol version 1.
pub const ALPN: &[u8] = ${rust_byte_string_literal(contract.v1.alpn)};
/// Bytes before the variable-length request lengths.
pub const REQUEST_FIXED_BYTES: usize = ${formatted_decimal(contract.v1.request_fixed_bytes)};
/// Bytes before the variable-length response payload length.
pub const RESPONSE_FIXED_BYTES: usize = ${formatted_decimal(contract.v1.response_fixed_bytes)};
/// Maximum bytes in one unsigned \`vu128\` accepted by this protocol.
pub const MAX_VARUINT_BYTES: usize = ${formatted_decimal(contract.v1.max_varuint_bytes)};
/// Bytes in every canonical item ID carried by the protocol.
pub const ITEM_ID_BYTES: usize = ${formatted_decimal(contract.item_id_bytes)};
/// Absolute value or response payload ceiling representable by protocol v1.
pub const MAX_VALUE_BYTES: usize = ${formatted_decimal(contract.max_value_bytes)};

const SET_TTL_FLAG: u8 = ${formatted_byte(contract.v1.set_ttl_flag)};
const SET_IF_ABSENT_FLAG: u8 = ${formatted_byte(contract.v1.set_if_absent_flag)};
const SET_IF_PRESENT_FLAG: u8 = ${formatted_byte(contract.v1.set_if_present_flag)};

${rust_wire_enum("Opcode", "Operations supported by protocol v1.", contract.opcodes, "UnknownOpcode")}

${rust_wire_enum("Status", "Status returned in every protocol response.", contract.statuses, "UnknownStatus")}
`
}
