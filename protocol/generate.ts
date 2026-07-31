#!/usr/bin/env bun
/** Generates language-specific wire values from the canonical Smithy contract. */

import { mkdirSync, writeFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

type Json_Object = Readonly<Record<string, unknown>>

/** One numeric wire enum member.
 *
 * @property name - PascalCase language-neutral member name.
 * @property value - Unsigned byte value carried on the wire.
 */
export interface Wire_Entry {
  readonly name: string
  readonly value: number
}

/** Protocol v2 constants consumed by the managed client. */
export interface Wire_V2_Contract {
  readonly alpn: string
  readonly request_header_bytes: number
  readonly response_header_bytes: number
  readonly set_ttl_bytes: number
  readonly response_value_length_mask: number
  readonly value_compressed_bit: number
  readonly value_encrypted_bit: number
  readonly set_ttl_bit: number
  readonly set_if_absent_bit: number
  readonly set_if_present_bit: number
}

/** Protocol v3 constants consumed by the Rust protocol crate. */
export interface Wire_V3_Contract {
  readonly alpn: string
  readonly request_fixed_bytes: number
  readonly response_fixed_bytes: number
  readonly max_varuint_bytes: number
  readonly set_ttl_flag: number
  readonly set_if_absent_flag: number
  readonly set_if_present_flag: number
}

/** Cross-language value-format identifiers and envelope limits. */
export interface Value_Format_Contract {
  readonly compression_none: number
  readonly compression_zstandard: number
  readonly encryption_compact: number
  readonly encryption_none: number
  readonly encryption_robust: number
  readonly json_encoding: string
  readonly max_encoding_bytes: number
  readonly max_type_name_bytes: number
  readonly serialization_json: number
  readonly serialization_raw: number
  readonly version: number
}

type Api_Type_Kind = "blob" | "boolean" | "enum" | "long" | "string"

/** One resolved Smithy API field type. */
export interface Api_Type {
  readonly kind: Api_Type_Kind
  readonly name?: string
}

/** One field in a Smithy operation input or output structure. */
export interface Api_Member {
  readonly name: string
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

/** Language-neutral subset of the OpenKache Smithy model used by generators. */
export interface Wire_Contract {
  readonly api: Api_Contract
  readonly item_key_bytes: number
  readonly max_value_bytes: number
  readonly opcodes: readonly Wire_Entry[]
  readonly statuses: readonly Wire_Entry[]
  readonly value_format: Value_Format_Contract
  readonly v2: Wire_V2_Contract
  readonly v3: Wire_V3_Contract
}

const PROTOCOL_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const PUBLIC_ROOT = dirname(PROTOCOL_DIRECTORY)
const MODEL_DIRECTORY = "model"
const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
const STATUS_SHAPE_ID = "openkache.protocol#Status"
const WIRE_CONTRACT_TRAIT_ID = "openkache.protocol#wireContract"
const WIRE_OPCODE_TRAIT_ID = "openkache.protocol#wireOpcode"
const WIRE_STATUS_TRAIT_ID = "openkache.protocol#wireStatus"
const VALUE_FORMAT_TRAIT_ID = "openkache.protocol#valueFormat"
const GENERATED_OUTPUTS = {
  csharp_api: join(
    PUBLIC_ROOT,
    "clients/dotnet/OpenKache/generated_local/SmithyApi.g.cs",
  ),
  csharp_wire: join(
    PUBLIC_ROOT,
    "clients/dotnet/OpenKache/generated_local/WireValues.g.cs",
  ),
  rust_api: process.env.OPENKACHE_RUST_API_OUTPUT ??
    join(PUBLIC_ROOT, "clients/rust/generated_local/smithy_api.rs"),
  rust_wire: process.env.OPENKACHE_RUST_WIRE_OUTPUT ??
    join(PROTOCOL_DIRECTORY, "generated_local/wire_values.rs"),
  typescript_api: join(
    PUBLIC_ROOT,
    "clients/typescript/src/generated_local/smithy-api.ts",
  ),
  typescript_value_format: join(
    PUBLIC_ROOT,
    "clients/typescript/src/generated_local/smithy-value-format.ts",
  ),
} as const

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

function optional_object_member(
  object: Json_Object,
  member: string,
  location: string,
): Json_Object | undefined {
  const value = object[member]
  return value === undefined ? undefined : object_value(value, `${location}.${member}`)
}

function shape_type(shape: Json_Object, location: string): string {
  return string_member(shape, "type", location)
}

function api_type(shapes: Json_Object, target: string): Api_Type {
  const prelude_types: Readonly<Record<string, Api_Type_Kind>> = {
    "smithy.api#Boolean": "boolean",
    "smithy.api#Long": "long",
    "smithy.api#String": "string",
  }
  const prelude = prelude_types[target]
  if (prelude !== undefined) return { kind: prelude }

  const shape = object_member(shapes, target, "Smithy AST.shapes")
  const kind = shape_type(shape, `Smithy AST.shapes.${target}`)
  switch (kind) {
    case "blob":
      return { kind: "blob" }
    case "enum":
      return { kind: "enum", name: shape_name(target) }
    default:
      throw new Error(`unsupported API member target ${target} with shape type ${kind}`)
  }
}

function api_structure(shapes: Json_Object, target: string): Api_Structure {
  const shape = object_member(shapes, target, "Smithy AST.shapes")
  if (shape_type(shape, `Smithy AST.shapes.${target}`) !== "structure") {
    throw new Error(`${target} must be a structure`)
  }
  const members = object_member(shape, "members", target)
  return {
    name: shape_name(target),
    members: Object.entries(members).map(([name, value]): Api_Member => {
      const member = object_value(value, `${target}.${name}`)
      const traits = optional_object_member(member, "traits", `${target}.${name}`)
      return {
        name,
        required: traits?.["smithy.api#required"] !== undefined,
        type: api_type(shapes, string_member(member, "target", `${target}.${name}`)),
      }
    }),
  }
}

function api_enum(shapes: Json_Object, name: string): Api_Enum {
  const shape_id = `openkache.protocol#${name}`
  const shape = object_member(shapes, shape_id, "Smithy AST.shapes")
  if (shape_type(shape, `Smithy AST.shapes.${shape_id}`) !== "enum") {
    throw new Error(`${shape_id} must be an enum`)
  }
  const members = object_member(shape, "members", shape_id)
  return {
    name,
    members: Object.entries(members).map(([member_name, value]): Api_Enum_Member => {
      const member = object_value(value, `${shape_id}.${member_name}`)
      const traits = object_member(member, "traits", `${shape_id}.${member_name}`)
      return {
        name: pascal_case(member_name),
        value: string_member(
          traits,
          "smithy.api#enumValue",
          `${shape_id}.${member_name}.traits`,
        ),
      }
    }),
  }
}

function api_contract(
  shapes: Json_Object,
  service: Json_Object,
): Api_Contract {
  const operation_shapes = array_member(service, "operations", SERVICE_SHAPE_ID)
    .map((operation, index): { readonly opcode: number; readonly operation: Api_Operation } => {
      const reference = object_value(operation, `${SERVICE_SHAPE_ID}.operations[${index}]`)
      const target = string_member(
        reference,
        "target",
        `${SERVICE_SHAPE_ID}.operations[${index}]`,
      )
      const shape = object_member(shapes, target, "Smithy AST.shapes")
      const input = string_member(
        object_member(shape, "input", target),
        "target",
        `${target}.input`,
      )
      const output = string_member(
        object_member(shape, "output", target),
        "target",
        `${target}.output`,
      )
      return {
        opcode: integer_member(
          trait_value(shape, WIRE_OPCODE_TRAIT_ID, target),
          "value",
          `${target}.${WIRE_OPCODE_TRAIT_ID}`,
          0,
          0xff,
        ),
        operation: {
          input: shape_name(input),
          name: shape_name(target),
          output: shape_name(output),
        },
      }
    })
    .sort((left, right) => left.opcode - right.opcode)

  const structure_names = new Set<string>()
  for (const { operation } of operation_shapes) {
    structure_names.add(operation.input)
    structure_names.add(operation.output)
  }
  const structures = [...structure_names]
    .map((name) => api_structure(shapes, `openkache.protocol#${name}`))
    .sort((left, right) => left.name.localeCompare(right.name))
  const enum_names = new Set<string>()
  for (const structure of structures) {
    for (const member of structure.members) {
      if (member.type.kind === "enum" && member.type.name !== undefined) {
        enum_names.add(member.type.name)
      }
    }
  }

  return {
    enums: [...enum_names]
      .map((name) => api_enum(shapes, name))
      .sort((left, right) => left.name.localeCompare(right.name)),
    operations: operation_shapes.map(({ operation }) => operation),
    structures,
  }
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

function wire_v2_contract(value: unknown): Wire_V2_Contract {
  const contract = object_value(value, `${WIRE_CONTRACT_TRAIT_ID}.v2`)
  return {
    alpn: string_member(contract, "alpn", "wireContract.v2"),
    request_header_bytes: integer_member(contract, "requestHeaderBytes", "wireContract.v2", 1),
    response_header_bytes: integer_member(contract, "responseHeaderBytes", "wireContract.v2", 1),
    set_ttl_bytes: integer_member(contract, "setTtlBytes", "wireContract.v2", 1),
    response_value_length_mask: integer_member(
      contract,
      "responseValueLengthMask",
      "wireContract.v2",
      0,
      0xffff_ffff,
    ),
    value_compressed_bit: integer_member(
      contract,
      "valueCompressedBit",
      "wireContract.v2",
      0,
      0xffff_ffff,
    ),
    value_encrypted_bit: integer_member(
      contract,
      "valueEncryptedBit",
      "wireContract.v2",
      0,
      0xffff_ffff,
    ),
    set_ttl_bit: integer_member(contract, "setTtlBit", "wireContract.v2", 0, 0xffff_ffff),
    set_if_absent_bit: integer_member(
      contract,
      "setIfAbsentBit",
      "wireContract.v2",
      0,
      0xffff_ffff,
    ),
    set_if_present_bit: integer_member(
      contract,
      "setIfPresentBit",
      "wireContract.v2",
      0,
      0xffff_ffff,
    ),
  }
}

function wire_v3_contract(value: unknown): Wire_V3_Contract {
  const contract = object_value(value, `${WIRE_CONTRACT_TRAIT_ID}.v3`)
  return {
    alpn: string_member(contract, "alpn", "wireContract.v3"),
    request_fixed_bytes: integer_member(contract, "requestFixedBytes", "wireContract.v3", 1),
    response_fixed_bytes: integer_member(contract, "responseFixedBytes", "wireContract.v3", 1),
    max_varuint_bytes: integer_member(contract, "maxVaruintBytes", "wireContract.v3", 1),
    set_ttl_flag: integer_member(contract, "setTtlFlag", "wireContract.v3", 0, 0xff),
    set_if_absent_flag: integer_member(
      contract,
      "setIfAbsentFlag",
      "wireContract.v3",
      0,
      0xff,
    ),
    set_if_present_flag: integer_member(
      contract,
      "setIfPresentFlag",
      "wireContract.v3",
      0,
      0xff,
    ),
  }
}

function value_format_contract(value: unknown): Value_Format_Contract {
  const contract = object_value(value, VALUE_FORMAT_TRAIT_ID)
  const values = {
    compression_none: integer_member(contract, "compressionNone", VALUE_FORMAT_TRAIT_ID, 0, 0xff),
    compression_zstandard: integer_member(
      contract,
      "compressionZstandard",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    encryption_compact: integer_member(
      contract,
      "encryptionCompact",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    encryption_none: integer_member(contract, "encryptionNone", VALUE_FORMAT_TRAIT_ID, 0, 0xff),
    encryption_robust: integer_member(
      contract,
      "encryptionRobust",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    json_encoding: string_member(contract, "jsonEncoding", VALUE_FORMAT_TRAIT_ID),
    max_encoding_bytes: integer_member(
      contract,
      "maxEncodingBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
      0xffff,
    ),
    max_type_name_bytes: integer_member(
      contract,
      "maxTypeNameBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
      0xffff,
    ),
    serialization_json: integer_member(
      contract,
      "serializationJson",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    serialization_raw: integer_member(
      contract,
      "serializationRaw",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    version: integer_member(contract, "version", VALUE_FORMAT_TRAIT_ID, 1, 0xffff),
  } satisfies Value_Format_Contract

  unique_wire_values(
    [
      { name: "Raw", value: values.serialization_raw },
      { name: "Json", value: values.serialization_json },
    ],
    "serialization",
  )
  unique_wire_values(
    [
      { name: "None", value: values.compression_none },
      { name: "Zstandard", value: values.compression_zstandard },
    ],
    "compression",
  )
  unique_wire_values(
    [
      { name: "None", value: values.encryption_none },
      { name: "Compact", value: values.encryption_compact },
      { name: "Robust", value: values.encryption_robust },
    ],
    "encryption",
  )
  return values
}

/** Extracts and validates the generator-facing contract from a Smithy JSON AST.
 *
 * @param ast - Unknown JSON value emitted by `smithy ast`.
 * @returns Validated wire constants and enum values.
 * @throws {Error} When required shapes, traits, values, or uniqueness invariants are missing.
 */
export function extract_wire_contract(ast: unknown): Wire_Contract {
  const ast_object = object_value(ast, "Smithy AST")
  const shapes = object_member(ast_object, "shapes", "Smithy AST")
  const service = object_member(shapes, SERVICE_SHAPE_ID, "Smithy AST.shapes")
  const contract_trait = trait_value(
    service,
    WIRE_CONTRACT_TRAIT_ID,
    `Smithy AST.shapes.${SERVICE_SHAPE_ID}`,
  )
  const value_format_trait = trait_value(
    service,
    VALUE_FORMAT_TRAIT_ID,
    `Smithy AST.shapes.${SERVICE_SHAPE_ID}`,
  )

  const opcodes = array_member(service, "operations", SERVICE_SHAPE_ID)
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
        value: integer_member(trait, "value", `${target}.${WIRE_OPCODE_TRAIT_ID}`, 0, 0xff),
      }
    })
    .sort((left, right) => left.value - right.value)

  const status_shape = object_member(shapes, STATUS_SHAPE_ID, "Smithy AST.shapes")
  const status_members = object_member(status_shape, "members", STATUS_SHAPE_ID)
  const statuses = Object.entries(status_members)
    .map(([name, member]): Wire_Entry => {
      const member_shape = object_value(member, `${STATUS_SHAPE_ID}.${name}`)
      const trait = trait_value(
        member_shape,
        WIRE_STATUS_TRAIT_ID,
        `${STATUS_SHAPE_ID}.${name}`,
      )
      return {
        name: pascal_case(name),
        value: integer_member(
          trait,
          "value",
          `${STATUS_SHAPE_ID}.${name}.${WIRE_STATUS_TRAIT_ID}`,
          0,
          0xff,
        ),
      }
    })
    .sort((left, right) => left.value - right.value)

  unique_wire_values(opcodes, "opcode")
  unique_wire_values(statuses, "status")
  if (opcodes.length === 0) throw new Error("wire contract must define at least one opcode")
  if (statuses.length === 0) throw new Error("wire contract must define at least one status")

  return {
    api: api_contract(shapes, service),
    item_key_bytes: integer_member(contract_trait, "itemKeyBytes", "wireContract", 1),
    max_value_bytes: integer_member(contract_trait, "maxValueBytes", "wireContract", 1),
    opcodes,
    statuses,
    value_format: value_format_contract(value_format_trait),
    v2: wire_v2_contract(contract_trait.v2),
    v3: wire_v3_contract(contract_trait.v3),
  }
}

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function formatted_byte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`
}

function snake_case(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .replace(/-/g, "_")
    .toLowerCase()
}

function typescript_name(identifier: string): string {
  return snake_case(identifier)
    .split("_")
    .map((part) => `${part[0]?.toUpperCase()}${part.slice(1)}`)
    .join("_")
}

function typescript_api_name(identifier: string): string {
  return `Smithy_${typescript_name(identifier)}`
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

/** Renders protocol v3 Rust definitions.
 *
 * @param contract - Validated language-neutral wire contract.
 * @returns Deterministic Rust source with a trailing newline.
 */
export function render_rust(contract: Wire_Contract): string {
  return `// Generated from the OpenKache Smithy contract. Do not edit.

/// QUIC application protocol identifier for wire protocol version 3.
pub const ALPN: &[u8] = b"${contract.v3.alpn}";
/// Bytes before the variable-length request lengths.
pub const REQUEST_FIXED_BYTES: usize = ${formatted_decimal(contract.v3.request_fixed_bytes)};
/// Bytes before the variable-length response payload length.
pub const RESPONSE_FIXED_BYTES: usize = ${formatted_decimal(contract.v3.response_fixed_bytes)};
/// Maximum bytes in one unsigned \`vu128\` accepted by this protocol.
pub const MAX_VARUINT_BYTES: usize = ${formatted_decimal(contract.v3.max_varuint_bytes)};
/// Bytes in every canonical item key carried by the protocol.
pub const ITEM_KEY_BYTES: usize = ${formatted_decimal(contract.item_key_bytes)};
/// Absolute value or response payload ceiling representable by protocol v3.
pub const MAX_VALUE_BYTES: usize = ${formatted_decimal(contract.max_value_bytes)};

const SET_TTL_FLAG: u8 = ${formatted_byte(contract.v3.set_ttl_flag)};
const SET_IF_ABSENT_FLAG: u8 = ${formatted_byte(contract.v3.set_if_absent_flag)};
const SET_IF_PRESENT_FLAG: u8 = ${formatted_byte(contract.v3.set_if_present_flag)};

${rust_wire_enum("Opcode", "Operations supported by protocol v3.", contract.opcodes, "UnknownOpcode")}

${rust_wire_enum("Status", "Status returned in every protocol response.", contract.statuses, "UnknownStatus")}
`
}

function csharp_wire_enum(name: string, entries: readonly Wire_Entry[]): string {
  const variants = entries
    .map((entry) => `        ${entry.name} = ${formatted_byte(entry.value)},`)
    .join("\n")
  return `    internal enum ${name} : byte
    {
${variants}
    }`
}

/** Renders protocol v2 C# definitions.
 *
 * @param contract - Validated language-neutral wire contract.
 * @returns Deterministic C# source with a trailing newline.
 */
export function render_csharp(contract: Wire_Contract): string {
  return `// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

// Generated from the OpenKache Smithy contract. Do not edit.

namespace OpenKache;

internal static partial class Protocol
{
    internal const string ApplicationProtocol = "${contract.v2.alpn}";
    internal const int ResponseHeaderBytes = ${formatted_decimal(contract.v2.response_header_bytes)};
    internal const int MaximumValueBytes = ${formatted_decimal(contract.max_value_bytes)};

    private const int RequestHeaderBytes = ${formatted_decimal(contract.v2.request_header_bytes)};
    private const int ItemKeyBytes = ${formatted_decimal(contract.item_key_bytes)};
    private const int SetTtlBytes = ${formatted_decimal(contract.v2.set_ttl_bytes)};
    private const uint ResponseValueLengthMask = ${formatted_decimal(contract.v2.response_value_length_mask)}u;
    private const uint ValueCompressedBit = ${formatted_decimal(contract.v2.value_compressed_bit)}u;
    private const uint ValueEncryptedBit = ${formatted_decimal(contract.v2.value_encrypted_bit)}u;
    private const uint SetTtlBit = ${formatted_decimal(contract.v2.set_ttl_bit)}u;
    private const uint SetIfAbsentBit = ${formatted_decimal(contract.v2.set_if_absent_bit)}u;
    private const uint SetIfPresentBit = ${formatted_decimal(contract.v2.set_if_present_bit)}u;

${csharp_wire_enum("Opcode", contract.opcodes)}

${csharp_wire_enum("Status", contract.statuses)}
}
`
}

function typescript_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "Uint8Array"
      break
    case "boolean":
      rendered = "boolean"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = typescript_api_name(type.name)
      break
    case "long":
      rendered = "number"
      break
    case "string":
      rendered = "string"
      break
  }
  return required ? rendered : `${rendered} | undefined`
}

/** Renders Smithy operation types and an API interface for TypeScript.
 *
 * @param contract - Validated language-neutral wire and API contract.
 * @returns Deterministic TypeScript source with a trailing newline.
 */
export function render_typescript_api(contract: Wire_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const values = enum_.members.map((member) => JSON.stringify(member.value)).join(" | ")
    return `/** Values defined by the Smithy ${enum_.name} shape. */
export type ${typescript_api_name(enum_.name)} = ${values}`
  })
  const structures = contract.api.structures.map((structure) => {
    const members = structure.members.map((member) => {
      const optional = member.required ? "" : "?"
      return `  /** Smithy ${member.name} member. */
  readonly ${snake_case(member.name)}${optional}: ${typescript_api_type(member.type, member.required)}`
    })
    return `/** Smithy ${structure.name} structure. */
export interface ${typescript_api_name(structure.name)} {
${members.join("\n")}
}`
  })
  const operations = contract.api.operations.map(
    (operation) =>
      `  /** Invokes the Smithy ${operation.name} operation. */
  ${snake_case(operation.name)}(input: ${typescript_api_name(operation.input)}): Promise<${typescript_api_name(operation.output)}>`,
  )
  return `// Generated from the OpenKache Smithy contract. Do not edit.

${[...enums, ...structures].join("\n\n")}

/** Operations defined by the OpenKache Smithy service. */
export interface Smithy_OpenKache_Api {
${operations.join("\n")}
}
`
}

/** Renders cross-language value-format identifiers and limits for TypeScript adapters.
 *
 * @param contract - Validated language-neutral wire and value-format contract.
 * @returns Deterministic TypeScript source with a trailing newline.
 */
export function render_typescript_value_format(contract: Wire_Contract): string {
  const value = contract.value_format
  return `// Generated from the OpenKache Smithy contract. Do not edit.

/** Current client-owned value-format version. */
export const SMITHY_VALUE_FORMAT_VERSION = ${value.version}
/** Maximum UTF-8 byte length of a codec identifier. */
export const SMITHY_VALUE_MAX_ENCODING_BYTES = ${value.max_encoding_bytes}
/** Maximum UTF-8 byte length of a logical codec type name. */
export const SMITHY_VALUE_MAX_TYPE_NAME_BYTES = ${value.max_type_name_bytes}
/** Built-in canonical JSON codec identifier. */
export const SMITHY_VALUE_JSON_ENCODING = ${JSON.stringify(value.json_encoding)}
/** Raw serialized-value identifier. */
export const SMITHY_VALUE_SERIALIZATION_RAW = ${value.serialization_raw}
/** Canonical JSON serialized-value identifier. */
export const SMITHY_VALUE_SERIALIZATION_JSON = ${value.serialization_json}
/** Uncompressed value-format identifier. */
export const SMITHY_VALUE_COMPRESSION_NONE = ${value.compression_none}
/** Zstandard value-format identifier. */
export const SMITHY_VALUE_COMPRESSION_ZSTANDARD = ${value.compression_zstandard}
/** Unencrypted value-format identifier. */
export const SMITHY_VALUE_ENCRYPTION_NONE = ${value.encryption_none}
/** Compact AES-SIV value-format identifier. */
export const SMITHY_VALUE_ENCRYPTION_COMPACT = ${value.encryption_compact}
/** Robust AES-GCM-SIV value-format identifier. */
export const SMITHY_VALUE_ENCRYPTION_ROBUST = ${value.encryption_robust}
`
}

function csharp_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "byte[]"
      break
    case "boolean":
      rendered = "bool"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = type.name
      break
    case "long":
      rendered = "long"
      break
    case "string":
      rendered = "string"
      break
  }
  return required ? rendered : `${rendered}?`
}

/** Renders Smithy operation types and an API interface for C#.
 *
 * @param contract - Validated language-neutral wire and API contract.
 * @returns Deterministic C# source with a trailing newline.
 */
export function render_csharp_api(contract: Wire_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const members = enum_.members
      .map((member) => `    /// <summary>Smithy ${member.value} value.</summary>
    ${member.name},`)
      .join("\n")
    return `/// <summary>Values defined by the Smithy ${enum_.name} shape.</summary>
public enum ${enum_.name}
{
${members}
}`
  })
  const structures = contract.api.structures.map((structure) => {
    if (structure.members.length === 0) {
      return `/// <summary>Smithy ${structure.name} structure.</summary>
public sealed record ${structure.name};`
    }
    const members = structure.members.map((member) => {
      const required = member.required ? "required " : ""
      return `    /// <summary>Smithy ${member.name} member.</summary>
    public ${required}${csharp_api_type(member.type, member.required)} ${pascal_case(snake_case(member.name))} { get; init; }`
    })
    return `/// <summary>Smithy ${structure.name} structure.</summary>
public sealed record ${structure.name}
{
${members.join("\n")}
}`
  })
  const operations = contract.api.operations.map(
    (operation) =>
      `    /// <summary>Invokes the Smithy ${operation.name} operation.</summary>
    ValueTask<${operation.output}> ${operation.name}Async(${operation.input} input, CancellationToken cancellationToken = default);`,
  )
  return `// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

// Generated from the OpenKache Smithy contract. Do not edit.

#nullable enable

namespace OpenKache.Smithy;

${[...enums, ...structures].join("\n\n")}

/// <summary>Operations defined by the OpenKache Smithy service.</summary>
public interface IOpenKacheApi
{
${operations.join("\n")}
}
`
}

function rust_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "Vec<u8>"
      break
    case "boolean":
      rendered = "bool"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = type.name
      break
    case "long":
      rendered = "i64"
      break
    case "string":
      rendered = "String"
      break
  }
  return required ? rendered : `Option<${rendered}>`
}

/** Renders Smithy operation types and an API trait for Rust.
 *
 * @param contract - Validated language-neutral wire and API contract.
 * @returns Deterministic Rust source with a trailing newline.
 */
export function render_rust_api(contract: Wire_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const members = enum_.members
      .map((member) => `    /// Smithy ${member.value} value.
    ${member.name},`)
      .join("\n")
    return `/// Values defined by the Smithy ${enum_.name} shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ${enum_.name} {
${members}
}`
  })
  const structures = contract.api.structures.map((structure) => {
    if (structure.members.length === 0) {
      return `/// Smithy ${structure.name} structure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ${structure.name};`
    }
    const members = structure.members.map(
      (member) =>
        `    /// Smithy ${member.name} member.
    pub ${snake_case(member.name)}: ${rust_api_type(member.type, member.required)},`,
    )
    return `/// Smithy ${structure.name} structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ${structure.name} {
${members.join("\n")}
}`
  })
  const operations = contract.api.operations.map(
    (operation) =>
      `    /// Invokes the Smithy ${operation.name} operation.
    fn ${snake_case(operation.name)}(
        &self,
        input: ${operation.input},
    ) -> impl core::future::Future<
        Output = core::result::Result<${operation.output}, Self::Error>,
    > + Send;`,
  )
  return `// Generated from the OpenKache Smithy contract. Do not edit.

${[...enums, ...structures].join("\n\n")}

/// Operations defined by the OpenKache Smithy service.
pub trait OpenKacheApi {
    /// Error returned by an operation.
    type Error;

${operations.join("\n\n")}
}
`
}

function smithy_ast(): unknown {
  const result = Bun.spawnSync(["smithy", "ast", MODEL_DIRECTORY], {
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

type Generation_Target = "all" | "dotnet" | "rust-api" | "rust-wire" | "typescript"

function generation_target(value: string | undefined): Generation_Target {
  switch (value) {
    case undefined:
    case "all":
      return "all"
    case "dotnet":
      return "dotnet"
    case "rust-api":
      return "rust-api"
    case "rust-wire":
      return "rust-wire"
    case "typescript":
      return "typescript"
    default:
      throw new Error(`unsupported OPENKACHE_GENERATION_TARGET ${JSON.stringify(value)}`)
  }
}

function expected_outputs(
  contract: Wire_Contract,
  target: Generation_Target,
): Readonly<Record<string, string>> {
  const all_outputs = {
    [GENERATED_OUTPUTS.csharp_api]: render_csharp_api(contract),
    [GENERATED_OUTPUTS.csharp_wire]: render_csharp(contract),
    [GENERATED_OUTPUTS.rust_api]: render_rust_api(contract),
    [GENERATED_OUTPUTS.rust_wire]: render_rust(contract),
    [GENERATED_OUTPUTS.typescript_api]: render_typescript_api(contract),
    [GENERATED_OUTPUTS.typescript_value_format]: render_typescript_value_format(contract),
  }
  switch (target) {
    case "all":
      return all_outputs
    case "dotnet":
      return {
        [GENERATED_OUTPUTS.csharp_api]: all_outputs[GENERATED_OUTPUTS.csharp_api],
        [GENERATED_OUTPUTS.csharp_wire]: all_outputs[GENERATED_OUTPUTS.csharp_wire],
      }
    case "rust-api":
      return {
        [GENERATED_OUTPUTS.rust_api]: all_outputs[GENERATED_OUTPUTS.rust_api],
      }
    case "rust-wire":
      return {
        [GENERATED_OUTPUTS.rust_wire]: all_outputs[GENERATED_OUTPUTS.rust_wire],
      }
    case "typescript":
      return {
        [GENERATED_OUTPUTS.typescript_api]: all_outputs[GENERATED_OUTPUTS.typescript_api],
        [GENERATED_OUTPUTS.typescript_value_format]:
          all_outputs[GENERATED_OUTPUTS.typescript_value_format],
      }
  }
}

function write_outputs(outputs: Readonly<Record<string, string>>): void {
  for (const [path, content] of Object.entries(outputs)) {
    mkdirSync(dirname(path), { recursive: true })
    writeFileSync(path, content)
    console.log(`Generated ${path}`)
  }
}

/** Runs the protocol contract generator CLI.
 *
 * @returns Process exit code.
 */
export function main(): number {
  try {
    const outputs = expected_outputs(
      extract_wire_contract(smithy_ast()),
      generation_target(process.env.OPENKACHE_GENERATION_TARGET),
    )
    write_outputs(outputs)
    return 0
  } catch (error) {
    console.error(
      `GENERATION_FAILED: ${error instanceof Error ? error.message : String(error)}\n` +
        "  Why: language wire values can only be generated from a valid, complete Smithy contract.\n" +
        "  Fix: Run `smithy validate model` from the protocol directory, correct the reported model or generator error, then rerun `./generate.ts`.",
    )
    return 1
  }
}

if (import.meta.main) process.exit(main())
