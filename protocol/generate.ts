#!/usr/bin/env bun
/** Generates language-specific wire values from the canonical Smithy contract. */

import { mkdirSync, mkdtempSync, renameSync, rmSync, writeFileSync } from "node:fs"
import { basename, dirname, join } from "node:path"
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

/** Protocol v1 constants consumed by the Rust protocol crate. */
export interface Wire_V3_Contract {
  readonly alpn: string
  readonly request_fixed_bytes: number
  readonly response_fixed_bytes: number
  readonly max_varuint_bytes: number
  readonly set_ttl_flag: number
  readonly set_if_absent_flag: number
  readonly set_if_present_flag: number
}

/** Cross-language value-format wire layout, identifiers, and cryptographic metadata. */
export interface Value_Format_Contract {
  readonly aad_domain: string
  readonly compact_encryption_context: string
  readonly compact_mac_context: string
  readonly compact_synthetic_iv_bytes: number
  readonly compression_none: number
  readonly compression_zstandard: number
  readonly data_protection_key_bytes: number
  readonly encryption_compact: number
  readonly encryption_none: number
  readonly encryption_robust: number
  readonly format_byte_bytes: number
  readonly format_compression_mask: number
  readonly format_encryption_shift: number
  readonly item_id_root_context: string
  readonly robust_context: string
  readonly robust_nonce_bytes: number
  readonly robust_tag_bytes: number
  readonly serialization_json: number
  readonly serialization_raw: number
  readonly value_root_context: string
  readonly max_vu128_bytes: number
  readonly version: number
}

/** Legacy metadata envelope retained for the TypeScript adapter migration. */
export interface Value_Envelope_Contract {
  readonly json_encoding: string
  readonly magic_and_version_hex: string
  readonly max_encoding_bytes: number
  readonly max_type_name_bytes: number
}

/** Native client ABI discriminators shared by every language adapter. */
export interface Native_Contract {
  readonly abi_version: number
  readonly operation_reconnect: number
  readonly operation_connection_state: number
  readonly result_error: number
  readonly result_ok: number
  readonly result_value: number
  readonly result_not_found: number
  readonly result_created: number
  readonly result_replaced: number
  readonly result_deleted: number
  readonly result_not_deleted: number
  readonly result_connected: number
  readonly result_not_stored: number
  readonly result_connection_state: number
  readonly set_condition_none: number
  readonly set_condition_if_absent: number
  readonly set_condition_if_present: number
  readonly connection_state_connected: number
  readonly connection_state_reconnecting: number
  readonly connection_state_disconnected: number
  readonly connection_state_closed: number
  readonly connection_state_unknown: number
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
  readonly item_id_bytes: number
  readonly max_value_bytes: number
  readonly native: Native_Contract
  readonly opcodes: readonly Wire_Entry[]
  readonly statuses: readonly Wire_Entry[]
  readonly value_envelope: Value_Envelope_Contract
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
const VALUE_ENVELOPE_TRAIT_ID = "openkache.protocol#valueEnvelope"
const NATIVE_CONTRACT_TRAIT_ID = "openkache.protocol#nativeContract"
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
  c_header: join(PUBLIC_ROOT, "clients/core/include/openkache/smithy_contract.h"),
  typescript_api: join(
    PUBLIC_ROOT,
    "clients/typescript/src/generated_local/smithy-api.ts",
  ),
  typescript_value_format: join(
    PUBLIC_ROOT,
    "clients/typescript/src/generated_local/smithy-value-format.ts",
  ),
  typescript_value_envelope: join(
    PUBLIC_ROOT,
    "clients/typescript/src/generated_local/smithy-value-envelope.ts",
  ),
  swift_api: join(
    PUBLIC_ROOT,
    "clients/swift/Sources/OpenKache/Generated/SmithyAPI.swift",
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
    aad_domain: string_member(contract, "aadDomain", VALUE_FORMAT_TRAIT_ID),
    compact_encryption_context: string_member(
      contract,
      "compactEncryptionContext",
      VALUE_FORMAT_TRAIT_ID,
    ),
    compact_mac_context: string_member(
      contract,
      "compactMacContext",
      VALUE_FORMAT_TRAIT_ID,
    ),
    compact_synthetic_iv_bytes: integer_member(
      contract,
      "compactSyntheticIvBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
    ),
    compression_none: integer_member(contract, "compressionNone", VALUE_FORMAT_TRAIT_ID, 0, 0xff),
    compression_zstandard: integer_member(
      contract,
      "compressionZstandard",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    data_protection_key_bytes: integer_member(
      contract,
      "dataProtectionKeyBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
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
    format_byte_bytes: integer_member(
      contract,
      "formatByteBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
      1,
    ),
    format_compression_mask: integer_member(
      contract,
      "formatCompressionMask",
      VALUE_FORMAT_TRAIT_ID,
      0,
      0xff,
    ),
    format_encryption_shift: integer_member(
      contract,
      "formatEncryptionShift",
      VALUE_FORMAT_TRAIT_ID,
      0,
      7,
    ),
    item_id_root_context: string_member(
      contract,
      "itemIdRootContext",
      VALUE_FORMAT_TRAIT_ID,
    ),
    robust_context: string_member(contract, "robustContext", VALUE_FORMAT_TRAIT_ID),
    robust_nonce_bytes: integer_member(
      contract,
      "robustNonceBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
    ),
    robust_tag_bytes: integer_member(
      contract,
      "robustTagBytes",
      VALUE_FORMAT_TRAIT_ID,
      1,
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
    value_root_context: string_member(
      contract,
      "valueRootContext",
      VALUE_FORMAT_TRAIT_ID,
    ),
    version: integer_member(contract, "version", VALUE_FORMAT_TRAIT_ID, 1),
    max_vu128_bytes: integer_member(
      contract,
      "maxVu128Bytes",
      VALUE_FORMAT_TRAIT_ID,
      17,
      17,
    ),
  } satisfies Value_Format_Contract

  for (const [member, actual, expected] of [
    ["compactSyntheticIvBytes", values.compact_synthetic_iv_bytes, 16],
    ["robustNonceBytes", values.robust_nonce_bytes, 12],
    ["robustTagBytes", values.robust_tag_bytes, 16],
    ["dataProtectionKeyBytes", values.data_protection_key_bytes, 32],
  ] as const) {
    if (actual !== expected) {
      throw new Error(
        `${VALUE_FORMAT_TRAIT_ID}.${member} must be ${expected} for the current core implementation, got ${actual}`,
      )
    }
  }
  if (values.format_compression_mask !== 0x0f) {
    throw new Error(
      "value format compression mask must cover exactly the low four format bits",
    )
  }
  if (values.format_encryption_shift !== 4) {
    throw new Error("value format encryption shift must be exactly four bits")
  }
  const format_encryption_mask =
    values.format_compression_mask << values.format_encryption_shift
  if (format_encryption_mask !== 0xf0) {
    throw new Error("value format encryption mask must cover exactly the high four format bits")
  }
  const version_bytes = encode_vu128(values.version)
  if (version_bytes.length > values.max_vu128_bytes) {
    throw new Error(
      `value format version encodes to ${version_bytes.length} bytes, exceeding maxVu128Bytes ${values.max_vu128_bytes}`,
    )
  }
  for (const [kind, value] of [
    ["compression", values.compression_none],
    ["compression", values.compression_zstandard],
    ["encryption", values.encryption_none],
    ["encryption", values.encryption_compact],
    ["encryption", values.encryption_robust],
  ] as const) {
    if (value > values.format_compression_mask) {
      throw new Error(`${kind} identifier ${value} does not fit in a format nibble`)
    }
  }

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

function value_envelope_contract(value: unknown): Value_Envelope_Contract {
  const contract = object_value(value, VALUE_ENVELOPE_TRAIT_ID)
  const magic_and_version_hex = string_member(
    contract,
    "magicAndVersionHex",
    VALUE_ENVELOPE_TRAIT_ID,
  ).toLowerCase()
  if (
    magic_and_version_hex.length === 0 ||
    magic_and_version_hex.length % 2 !== 0 ||
    magic_and_version_hex.length !== 8 ||
    !/^[0-9a-f]+$/.test(magic_and_version_hex)
  ) {
    throw new Error(
      "openkache.protocol#valueEnvelope.magicAndVersionHex must contain exactly four bytes of hexadecimal digits",
    )
  }
  const max_encoding_bytes = integer_member(
    contract,
    "maxEncodingBytes",
    VALUE_ENVELOPE_TRAIT_ID,
    1,
    0xffff,
  )
  const json_encoding = string_member(
    contract,
    "jsonEncoding",
    VALUE_ENVELOPE_TRAIT_ID,
  )
  if (!valid_encoding_identifier(json_encoding, max_encoding_bytes)) {
    throw new Error(
      `${VALUE_ENVELOPE_TRAIT_ID}.jsonEncoding must match the portable lowercase encoding identifier grammar and fit maxEncodingBytes`,
    )
  }
  return {
    json_encoding,
    magic_and_version_hex,
    max_encoding_bytes,
    max_type_name_bytes: integer_member(
      contract,
      "maxTypeNameBytes",
      VALUE_ENVELOPE_TRAIT_ID,
      1,
      0xffff,
    ),
  }
}

function native_contract(value: unknown): Native_Contract {
  const contract = object_value(value, NATIVE_CONTRACT_TRAIT_ID)
  const values = {
    abi_version: integer_member(
      contract,
      "abiVersion",
      NATIVE_CONTRACT_TRAIT_ID,
      1,
      0xffff_ffff,
    ),
    operation_reconnect: integer_member(
      contract,
      "operationReconnect",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xffff_ffff,
    ),
    operation_connection_state: integer_member(
      contract,
      "operationConnectionState",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xffff_ffff,
    ),
    result_error: integer_member(
      contract,
      "resultError",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    result_ok: integer_member(contract, "resultOk", NATIVE_CONTRACT_TRAIT_ID, 0, 0xff),
    result_value: integer_member(
      contract,
      "resultValue",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    result_not_found: integer_member(
      contract,
      "resultNotFound",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    result_created: integer_member(
      contract,
      "resultCreated",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    result_replaced: integer_member(
      contract,
      "resultReplaced",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    result_deleted: integer_member(
      contract,
      "resultDeleted",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    result_not_deleted: integer_member(
      contract,
      "resultNotDeleted",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    result_connected: integer_member(
      contract,
      "resultConnected",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    result_not_stored: integer_member(
      contract,
      "resultNotStored",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    result_connection_state: integer_member(
      contract,
      "resultConnectionState",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    set_condition_none: integer_member(
      contract,
      "setConditionNone",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    set_condition_if_absent: integer_member(
      contract,
      "setConditionIfAbsent",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    set_condition_if_present: integer_member(
      contract,
      "setConditionIfPresent",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    connection_state_connected: integer_member(
      contract,
      "connectionStateConnected",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    connection_state_reconnecting: integer_member(
      contract,
      "connectionStateReconnecting",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    connection_state_disconnected: integer_member(
      contract,
      "connectionStateDisconnected",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    connection_state_closed: integer_member(
      contract,
      "connectionStateClosed",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
    connection_state_unknown: integer_member(
      contract,
      "connectionStateUnknown",
      NATIVE_CONTRACT_TRAIT_ID,
      0,
      0xff,
    ),
  } satisfies Native_Contract

  unique_wire_values(
    [
      { name: "Error", value: values.result_error },
      { name: "Ok", value: values.result_ok },
      { name: "Value", value: values.result_value },
      { name: "NotFound", value: values.result_not_found },
      { name: "Created", value: values.result_created },
      { name: "Replaced", value: values.result_replaced },
      { name: "Deleted", value: values.result_deleted },
      { name: "NotDeleted", value: values.result_not_deleted },
      { name: "Connected", value: values.result_connected },
      { name: "NotStored", value: values.result_not_stored },
      { name: "ConnectionState", value: values.result_connection_state },
    ],
    "native result",
  )
  unique_wire_values(
    [
      { name: "None", value: values.set_condition_none },
      { name: "IfAbsent", value: values.set_condition_if_absent },
      { name: "IfPresent", value: values.set_condition_if_present },
    ],
    "native SET condition",
  )
  unique_wire_values(
    [
      { name: "Connected", value: values.connection_state_connected },
      { name: "Reconnecting", value: values.connection_state_reconnecting },
      { name: "Disconnected", value: values.connection_state_disconnected },
      { name: "Closed", value: values.connection_state_closed },
      { name: "Unknown", value: values.connection_state_unknown },
    ],
    "native connection state",
  )
  if (values.operation_reconnect <= 0xff || values.operation_connection_state <= 0xff) {
    throw new Error(
      `${NATIVE_CONTRACT_TRAIT_ID} lifecycle operations must remain outside the Smithy opcode range`,
    )
  }
  return values
}

function valid_encoding_identifier(encoding: string, maximum_bytes: number): boolean {
  const bytes = new TextEncoder().encode(encoding)
  return (
    bytes.length >= 1 &&
    bytes.length <= maximum_bytes &&
    bytes[0] !== undefined &&
    bytes[0] >= 0x61 &&
    bytes[0] <= 0x7a &&
    bytes.slice(1).every(
      (byte) =>
        (byte >= 0x61 && byte <= 0x7a) ||
        (byte >= 0x30 && byte <= 0x39) ||
        byte === 0x2e ||
        byte === 0x2d,
    )
  )
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
  const value_envelope_trait = trait_value(
    service,
    VALUE_ENVELOPE_TRAIT_ID,
    `Smithy AST.shapes.${SERVICE_SHAPE_ID}`,
  )
  const native_contract_trait = trait_value(
    service,
    NATIVE_CONTRACT_TRAIT_ID,
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
    item_id_bytes: integer_member(contract_trait, "itemIdBytes", "wireContract", 1),
    max_value_bytes: integer_member(contract_trait, "maxValueBytes", "wireContract", 1),
    native: native_contract(native_contract_trait),
    opcodes,
    statuses,
    value_envelope: value_envelope_contract(value_envelope_trait),
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

function encode_vu128(value: number): readonly number[] {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`cannot VU128-encode invalid integer ${value}`)
  }
  let encoded = BigInt(value)
  if (encoded < 0x80n) return [Number(encoded)]
  if (encoded < 0x1000_0000n) {
    if (encoded < 0x4000n) {
      encoded <<= 2n
      return [
        0x80 | ((Number(encoded) & 0xff) >> 2),
        Number(encoded >> 8n) & 0xff,
      ]
    }
    if (encoded < 0x20_0000n) {
      encoded <<= 3n
      return [
        0xc0 | ((Number(encoded) & 0xff) >> 3),
        Number(encoded >> 8n) & 0xff,
        Number(encoded >> 16n) & 0xff,
      ]
    }
    encoded <<= 4n
    return [
      0xe0 | ((Number(encoded) & 0xff) >> 4),
      Number(encoded >> 8n) & 0xff,
      Number(encoded >> 16n) & 0xff,
      Number(encoded >> 24n) & 0xff,
    ]
  }

  const bytes: number[] = []
  let remaining = encoded
  while (remaining > 0n) {
    bytes.push(Number(remaining & 0xffn))
    remaining >>= 8n
  }
  const length = bytes.length
  if (length < 4 || length > 16) {
    throw new Error(`cannot VU128-encode integer ${value}`)
  }
  return [
    0xf0 | (length - 1),
    ...bytes,
  ]
}

function bytes_from_hex(value: string, location: string): readonly number[] {
  const bytes: number[] = []
  for (let index = 0; index < value.length; index += 2) {
    const pair = value.slice(index, index + 2)
    if (!/^[0-9a-f]{2}$/i.test(pair)) {
      throw new Error(`${location} contains invalid hexadecimal`)
    }
    const byte = Number.parseInt(pair, 16)
    bytes.push(byte)
  }
  return bytes
}

function rust_string_literal(value: string): string {
  let literal = '"'
  for (const character of value) {
    const code_point = character.codePointAt(0)
    if (code_point === undefined) continue
    switch (character) {
      case "\\":
        literal += "\\\\"
        break
      case '"':
        literal += '\\"'
        break
      case "\n":
        literal += "\\n"
        break
      case "\r":
        literal += "\\r"
        break
      case "\t":
        literal += "\\t"
        break
      default:
        if (code_point >= 0x20 && code_point <= 0x7e) {
          literal += character
        } else {
          literal += `\\u{${code_point.toString(16)}}`
        }
    }
  }
  return `${literal}"`
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

function rust_byte_array_literal(bytes: readonly number[]): string {
  return `[${bytes.map(formatted_byte).join(", ")}]`
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

function swift_name(identifier: string): string {
  return identifier
    .split(/[_-]/)
    .filter((part) => part.length > 0)
    .map((part) => {
      const normalized =
        part === part.toUpperCase()
          ? part.toLowerCase()
          : `${part[0]?.toLowerCase()}${part.slice(1)}`
      return `${normalized[0]?.toUpperCase()}${normalized.slice(1)}`
    })
    .join("")
}

function swift_property_name(identifier: string): string {
  const name = swift_name(identifier)
  return name.length === 0 ? name : `${name[0]?.toLowerCase()}${name.slice(1)}`
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
  const value = contract.value_format
  const value_version_bytes = encode_vu128(value.version)
  const envelope = contract.value_envelope
  const native = contract.native
  const envelope_magic = bytes_from_hex(
    envelope.magic_and_version_hex,
    "value envelope magic",
  )
  return `// Generated from the OpenKache Smithy contract. Do not edit.

/// Version of the native client ABI.
pub const FFI_ABI_VERSION: u32 = ${formatted_decimal(native.abi_version)};
/// Native operation used to replace a failed connection.
pub const FFI_OPERATION_RECONNECT: u32 = ${formatted_decimal(native.operation_reconnect)};
/// Native operation used to read the connection state.
pub const FFI_OPERATION_CONNECTION_STATE: u32 = ${formatted_decimal(native.operation_connection_state)};
/// Native result discriminator for an error.
pub const FFI_RESULT_ERROR: u32 = ${formatted_decimal(native.result_error)};
/// Native result discriminator for a successful operation.
pub const FFI_RESULT_OK: u32 = ${formatted_decimal(native.result_ok)};
/// Native result discriminator carrying a value.
pub const FFI_RESULT_VALUE: u32 = ${formatted_decimal(native.result_value)};
/// Native result discriminator for a missing value.
pub const FFI_RESULT_NOT_FOUND: u32 = ${formatted_decimal(native.result_not_found)};
/// Native result discriminator for a newly created value.
pub const FFI_RESULT_CREATED: u32 = ${formatted_decimal(native.result_created)};
/// Native result discriminator for a replaced value.
pub const FFI_RESULT_REPLACED: u32 = ${formatted_decimal(native.result_replaced)};
/// Native result discriminator for a deleted value.
pub const FFI_RESULT_DELETED: u32 = ${formatted_decimal(native.result_deleted)};
/// Native result discriminator for a value that was not deleted.
pub const FFI_RESULT_NOT_DELETED: u32 = ${formatted_decimal(native.result_not_deleted)};
/// Native result discriminator for a connected client handle.
pub const FFI_RESULT_CONNECTED: u32 = ${formatted_decimal(native.result_connected)};
/// Native result discriminator for a conditional SET that did not store.
pub const FFI_RESULT_NOT_STORED: u32 = ${formatted_decimal(native.result_not_stored)};
/// Native result discriminator carrying a connection state.
pub const FFI_RESULT_CONNECTION_STATE: u32 = ${formatted_decimal(native.result_connection_state)};
/// Native unconditional SET condition.
pub const FFI_SET_CONDITION_NONE: u32 = ${formatted_decimal(native.set_condition_none)};
/// Native SET condition requiring an absent item.
pub const FFI_SET_CONDITION_IF_ABSENT: u32 = ${formatted_decimal(native.set_condition_if_absent)};
/// Native SET condition requiring a present item.
pub const FFI_SET_CONDITION_IF_PRESENT: u32 = ${formatted_decimal(native.set_condition_if_present)};
/// Connection state for an available connection.
pub const FFI_CONNECTION_STATE_CONNECTED: u8 = ${formatted_byte(native.connection_state_connected)};
/// Connection state while replacing a failed connection.
pub const FFI_CONNECTION_STATE_RECONNECTING: u8 = ${formatted_byte(native.connection_state_reconnecting)};
/// Connection state after a failed connection.
pub const FFI_CONNECTION_STATE_DISCONNECTED: u8 = ${formatted_byte(native.connection_state_disconnected)};
/// Connection state after explicit closure.
pub const FFI_CONNECTION_STATE_CLOSED: u8 = ${formatted_byte(native.connection_state_closed)};
/// Fallback connection state for an invalid native handle or payload.
pub const FFI_CONNECTION_STATE_UNKNOWN: u8 = ${formatted_byte(native.connection_state_unknown)};

/// QUIC application protocol identifier for wire protocol version 1.
pub const ALPN: &[u8] = ${rust_byte_string_literal(contract.v3.alpn)};
/// Bytes before the variable-length request lengths.
pub const REQUEST_FIXED_BYTES: usize = ${formatted_decimal(contract.v3.request_fixed_bytes)};
/// Bytes before the variable-length response payload length.
pub const RESPONSE_FIXED_BYTES: usize = ${formatted_decimal(contract.v3.response_fixed_bytes)};
/// Maximum bytes in one unsigned \`vu128\` accepted by this protocol.
pub const MAX_VARUINT_BYTES: usize = ${formatted_decimal(contract.v3.max_varuint_bytes)};
/// Bytes in every canonical item ID carried by the protocol.
pub const ITEM_ID_BYTES: usize = ${formatted_decimal(contract.item_id_bytes)};
/// Absolute value or response payload ceiling representable by protocol v1.
pub const MAX_VALUE_BYTES: usize = ${formatted_decimal(contract.max_value_bytes)};

/// Current client-owned value-format version.
pub const VALUE_FORMAT_VERSION: u128 = ${formatted_decimal(value.version)};
/// Canonical VU128 bytes for the current value-format version.
pub const VALUE_FORMAT_VERSION_BYTES: &[u8] = &[${value_version_bytes.map(formatted_byte).join(", ")}];
/// Maximum bytes accepted for a canonical value-format VU128.
pub const VALUE_FORMAT_MAX_VU128_BYTES: usize = ${formatted_decimal(value.max_vu128_bytes)};
/// Bytes occupied by the value-format transform byte.
pub const VALUE_FORMAT_FORMAT_BYTE_BYTES: usize = ${formatted_decimal(value.format_byte_bytes)};
/// Low-nibble mask for the value-format compression identifier.
pub const VALUE_FORMAT_COMPRESSION_MASK: u8 = ${formatted_byte(value.format_compression_mask)};
/// Number of bits to shift the value-format encryption identifier.
pub const VALUE_FORMAT_ENCRYPTION_SHIFT: u8 = ${formatted_byte(value.format_encryption_shift)};
/// Raw serialized-value identifier.
pub const VALUE_FORMAT_SERIALIZATION_RAW: u8 = ${formatted_byte(value.serialization_raw)};
/// Canonical JSON serialized-value identifier.
pub const VALUE_FORMAT_SERIALIZATION_JSON: u8 = ${formatted_byte(value.serialization_json)};
/// Uncompressed value-format identifier.
pub const VALUE_FORMAT_COMPRESSION_NONE: u8 = ${formatted_byte(value.compression_none)};
/// Zstandard value-format identifier.
pub const VALUE_FORMAT_COMPRESSION_ZSTANDARD: u8 = ${formatted_byte(value.compression_zstandard)};
/// Unencrypted value-format identifier.
pub const VALUE_FORMAT_ENCRYPTION_NONE: u8 = ${formatted_byte(value.encryption_none)};
/// Compact AES-SIV value-format identifier.
pub const VALUE_FORMAT_ENCRYPTION_COMPACT: u8 = ${formatted_byte(value.encryption_compact)};
/// Robust AES-GCM-SIV value-format identifier.
pub const VALUE_FORMAT_ENCRYPTION_ROBUST: u8 = ${formatted_byte(value.encryption_robust)};
/// Compact AES-SIV synthetic-IV and authentication-tag size.
pub const VALUE_FORMAT_COMPACT_SYNTHETIC_IV_BYTES: usize = ${formatted_decimal(value.compact_synthetic_iv_bytes)};
/// Robust AES-GCM-SIV nonce size.
pub const VALUE_FORMAT_ROBUST_NONCE_BYTES: usize = ${formatted_decimal(value.robust_nonce_bytes)};
/// Robust AES-GCM-SIV authentication-tag size.
pub const VALUE_FORMAT_ROBUST_TAG_BYTES: usize = ${formatted_decimal(value.robust_tag_bytes)};
/// Application-managed data-protection key size.
pub const VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES: usize = ${formatted_decimal(value.data_protection_key_bytes)};
/// BLAKE3 protected-item-ID root derivation context.
pub const VALUE_FORMAT_ITEM_ID_ROOT_CONTEXT: &str = ${rust_string_literal(value.item_id_root_context)};
/// Associated-data domain separator.
pub const VALUE_FORMAT_AAD_DOMAIN: &[u8] = ${rust_byte_string_literal(value.aad_domain)};
/// BLAKE3 value-root derivation context.
pub const VALUE_FORMAT_VALUE_ROOT_CONTEXT: &str = ${rust_string_literal(value.value_root_context)};
/// BLAKE3 Compact AES-SIV MAC-key derivation context.
pub const VALUE_FORMAT_COMPACT_MAC_CONTEXT: &str = ${rust_string_literal(value.compact_mac_context)};
/// BLAKE3 Compact AES-SIV encryption-key derivation context.
pub const VALUE_FORMAT_COMPACT_ENCRYPTION_CONTEXT: &str = ${rust_string_literal(value.compact_encryption_context)};
/// BLAKE3 Robust AES-GCM-SIV key derivation context.
pub const VALUE_FORMAT_ROBUST_CONTEXT: &str = ${rust_string_literal(value.robust_context)};

/// Legacy metadata-envelope magic and version.
pub const VALUE_ENVELOPE_MAGIC_AND_VERSION: [u8; ${envelope_magic.length}] = ${rust_byte_array_literal(envelope_magic)};
/// Maximum UTF-8 byte length of a legacy metadata-envelope encoding identifier.
pub const VALUE_ENVELOPE_MAX_ENCODING_BYTES: usize = ${formatted_decimal(envelope.max_encoding_bytes)};
/// Maximum UTF-8 byte length of a legacy metadata-envelope logical type name.
pub const VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES: usize = ${formatted_decimal(envelope.max_type_name_bytes)};
/// Built-in canonical JSON codec identifier used by the legacy envelope adapter.
pub const VALUE_ENVELOPE_JSON_ENCODING: &str = ${rust_string_literal(envelope.json_encoding)};

const SET_TTL_FLAG: u8 = ${formatted_byte(contract.v3.set_ttl_flag)};
const SET_IF_ABSENT_FLAG: u8 = ${formatted_byte(contract.v3.set_if_absent_flag)};
const SET_IF_PRESENT_FLAG: u8 = ${formatted_byte(contract.v3.set_if_present_flag)};

${rust_wire_enum("Opcode", "Operations supported by protocol v1.", contract.opcodes, "UnknownOpcode")}

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

/** Renders protocol v1 C# definitions.
 *
 * @param contract - Validated language-neutral wire contract.
 * @returns Deterministic C# source with a trailing newline.
 */
export function render_csharp(contract: Wire_Contract): string {
  const value = contract.value_format
  const version_bytes = encode_vu128(value.version)
  const envelope = contract.value_envelope
  const native = contract.native
  const envelope_magic = bytes_from_hex(
    envelope.magic_and_version_hex,
    "value envelope magic",
  )
  return `// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

// Generated from the OpenKache Smithy contract. Do not edit.

namespace OpenKache;

internal static partial class Protocol
{
    internal const string ApplicationProtocol = ${JSON.stringify(contract.v3.alpn)};
    internal const int MaximumValueBytes = ${formatted_decimal(contract.max_value_bytes)};
    internal const uint NativeAbiVersion = ${formatted_decimal(native.abi_version)}u;
    internal const uint NativeOperationReconnect = ${formatted_decimal(native.operation_reconnect)}u;
    internal const uint NativeOperationConnectionState = ${formatted_decimal(native.operation_connection_state)}u;
    internal const uint NativeResultError = ${formatted_decimal(native.result_error)}u;
    internal const uint NativeResultOk = ${formatted_decimal(native.result_ok)}u;
    internal const uint NativeResultValue = ${formatted_decimal(native.result_value)}u;
    internal const uint NativeResultNotFound = ${formatted_decimal(native.result_not_found)}u;
    internal const uint NativeResultCreated = ${formatted_decimal(native.result_created)}u;
    internal const uint NativeResultReplaced = ${formatted_decimal(native.result_replaced)}u;
    internal const uint NativeResultDeleted = ${formatted_decimal(native.result_deleted)}u;
    internal const uint NativeResultNotDeleted = ${formatted_decimal(native.result_not_deleted)}u;
    internal const uint NativeResultConnected = ${formatted_decimal(native.result_connected)}u;
    internal const uint NativeResultNotStored = ${formatted_decimal(native.result_not_stored)}u;
    internal const uint NativeResultConnectionState = ${formatted_decimal(native.result_connection_state)}u;
    internal const uint NativeSetConditionNone = ${formatted_decimal(native.set_condition_none)}u;
    internal const uint NativeSetConditionIfAbsent = ${formatted_decimal(native.set_condition_if_absent)}u;
    internal const uint NativeSetConditionIfPresent = ${formatted_decimal(native.set_condition_if_present)}u;
    internal const byte NativeConnectionStateConnected = ${formatted_byte(native.connection_state_connected)};
    internal const byte NativeConnectionStateReconnecting = ${formatted_byte(native.connection_state_reconnecting)};
    internal const byte NativeConnectionStateDisconnected = ${formatted_byte(native.connection_state_disconnected)};
    internal const byte NativeConnectionStateClosed = ${formatted_byte(native.connection_state_closed)};
    internal const byte NativeConnectionStateUnknown = ${formatted_byte(native.connection_state_unknown)};

    private const int MaximumVarUIntBytes = ${formatted_decimal(contract.v3.max_varuint_bytes)};
    private const int ItemIdBytes = ${formatted_decimal(contract.item_id_bytes)};
    private const byte SetTtlBit = ${formatted_byte(contract.v3.set_ttl_flag)};
    private const byte SetIfAbsentBit = ${formatted_byte(contract.v3.set_if_absent_flag)};
    private const byte SetIfPresentBit = ${formatted_byte(contract.v3.set_if_present_flag)};

    internal const uint ValueFormatVersion = ${formatted_decimal(value.version)}u;
    internal const int ValueFormatMaxVu128Bytes = ${formatted_decimal(value.max_vu128_bytes)};
    internal const int ValueFormatFormatByteBytes = ${formatted_decimal(value.format_byte_bytes)};
    internal const byte ValueFormatCompressionMask = ${formatted_byte(value.format_compression_mask)};
    internal const byte ValueFormatEncryptionShift = ${formatted_byte(value.format_encryption_shift)};
    internal const byte ValueFormatSerializationRaw = ${formatted_byte(value.serialization_raw)};
    internal const byte ValueFormatSerializationJson = ${formatted_byte(value.serialization_json)};
    internal const byte ValueFormatCompressionNone = ${formatted_byte(value.compression_none)};
    internal const byte ValueFormatCompressionZstandard = ${formatted_byte(value.compression_zstandard)};
    internal const byte ValueFormatEncryptionNone = ${formatted_byte(value.encryption_none)};
    internal const byte ValueFormatEncryptionCompact = ${formatted_byte(value.encryption_compact)};
    internal const byte ValueFormatEncryptionRobust = ${formatted_byte(value.encryption_robust)};
    internal const int ValueFormatCompactSyntheticIvBytes = ${formatted_decimal(value.compact_synthetic_iv_bytes)};
    internal const int ValueFormatRobustNonceBytes = ${formatted_decimal(value.robust_nonce_bytes)};
    internal const int ValueFormatRobustTagBytes = ${formatted_decimal(value.robust_tag_bytes)};
    internal const int ValueFormatDataProtectionKeyBytes = ${formatted_decimal(value.data_protection_key_bytes)};
    internal const string ValueFormatItemIdRootContext = ${JSON.stringify(value.item_id_root_context)};
    internal const string ValueFormatAadDomain = ${JSON.stringify(value.aad_domain)};
    internal const string ValueFormatValueRootContext = ${JSON.stringify(value.value_root_context)};
    internal const string ValueFormatCompactMacContext = ${JSON.stringify(value.compact_mac_context)};
    internal const string ValueFormatCompactEncryptionContext = ${JSON.stringify(value.compact_encryption_context)};
    internal const string ValueFormatRobustContext = ${JSON.stringify(value.robust_context)};
    internal const int ValueEnvelopeMaxEncodingBytes = ${formatted_decimal(envelope.max_encoding_bytes)};
    internal const int ValueEnvelopeMaxTypeNameBytes = ${formatted_decimal(envelope.max_type_name_bytes)};
    internal const string ValueEnvelopeJsonEncoding = ${JSON.stringify(envelope.json_encoding)};
    internal static ReadOnlySpan<byte> ValueFormatVersionBytes =>
        [${version_bytes.map(formatted_byte).join(", ")}];
    internal static ReadOnlySpan<byte> ValueEnvelopeMagicAndVersion =>
        [${envelope_magic.map(formatted_byte).join(", ")}];

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
  const native = contract.native
  return `// Generated from the OpenKache Smithy contract. Do not edit.

${[...enums, ...structures].join("\n\n")}

/** Native ABI discriminators shared by every language adapter. */
export const SMITHY_NATIVE_CONTRACT = {
  abi_version: ${native.abi_version},
  operation_reconnect: ${native.operation_reconnect},
  operation_connection_state: ${native.operation_connection_state},
  result_error: ${native.result_error},
  result_ok: ${native.result_ok},
  result_value: ${native.result_value},
  result_not_found: ${native.result_not_found},
  result_created: ${native.result_created},
  result_replaced: ${native.result_replaced},
  result_deleted: ${native.result_deleted},
  result_not_deleted: ${native.result_not_deleted},
  result_connected: ${native.result_connected},
  result_not_stored: ${native.result_not_stored},
  result_connection_state: ${native.result_connection_state},
  set_condition_none: ${native.set_condition_none},
  set_condition_if_absent: ${native.set_condition_if_absent},
  set_condition_if_present: ${native.set_condition_if_present},
  connection_state_connected: ${native.connection_state_connected},
  connection_state_reconnecting: ${native.connection_state_reconnecting},
  connection_state_disconnected: ${native.connection_state_disconnected},
  connection_state_closed: ${native.connection_state_closed},
  connection_state_unknown: ${native.connection_state_unknown},
} as const

/** Operations defined by the OpenKache Smithy service. */
export interface Smithy_OpenKache_Api {
${operations.join("\n")}
}
`
}

function swift_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "Data"
      break
    case "boolean":
      rendered = "Bool"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = `Smithy_${typescript_name(type.name)}`
      break
    case "long":
      rendered = "Int64"
      break
    case "string":
      rendered = "String"
      break
  }
  return required ? rendered : `${rendered}?`
}

function swift_string_literal(value: string): string {
  let literal = '"'
  for (const character of value) {
    const code_point = character.codePointAt(0)
    if (code_point === undefined) continue
    switch (character) {
      case "\\":
        literal += "\\\\"
        break
      case '"':
        literal += '\\"'
        break
      case "\n":
        literal += "\\n"
        break
      case "\r":
        literal += "\\r"
        break
      case "\t":
        literal += "\\t"
        break
      default:
        if (code_point >= 0x20 && code_point <= 0x7e) {
          literal += character
        } else {
          literal += `\\u{${code_point.toString(16)}}`
        }
    }
  }
  return `${literal}"`
}

function c_identifier(identifier: string): string {
  return snake_case(identifier).toUpperCase()
}

function c_string_literal(value: string): string {
  return JSON.stringify(value)
}

function c_uint8_literal(value: number): string {
  return `UINT8_C(${value})`
}

function c_uint32_literal(value: number): string {
  return `UINT32_C(${value})`
}

function c_define(name: string, value: string): string {
  return `#define ${name} ${value}`
}

/** Renders the generated C contract consumed by every native ABI adapter. */
export function render_c_header(contract: Wire_Contract): string {
  const value = contract.value_format
  const envelope = contract.value_envelope
  const envelope_magic = bytes_from_hex(
    envelope.magic_and_version_hex,
    "value envelope magic",
  )
  const native = contract.native
  const lines = [
    "#ifndef OPENKACHE_SMITHY_CONTRACT_H",
    "#define OPENKACHE_SMITHY_CONTRACT_H",
    "",
    "/* Generated from the OpenKache Smithy contract. Do not edit. */",
    "",
    "#include <stdint.h>",
    "",
    c_define("OPENKACHE_SMITHY_V2_ALPN", c_string_literal(contract.v2.alpn)),
    c_define(
      "OPENKACHE_SMITHY_V2_REQUEST_HEADER_BYTES",
      c_uint32_literal(contract.v2.request_header_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_V2_RESPONSE_HEADER_BYTES",
      c_uint32_literal(contract.v2.response_header_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_V2_SET_TTL_BYTES",
      c_uint32_literal(contract.v2.set_ttl_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_V2_RESPONSE_VALUE_LENGTH_MASK",
      c_uint32_literal(contract.v2.response_value_length_mask),
    ),
    c_define(
      "OPENKACHE_SMITHY_V2_VALUE_COMPRESSED_BIT",
      c_uint32_literal(contract.v2.value_compressed_bit),
    ),
    c_define(
      "OPENKACHE_SMITHY_V2_VALUE_ENCRYPTED_BIT",
      c_uint32_literal(contract.v2.value_encrypted_bit),
    ),
    c_define("OPENKACHE_SMITHY_V2_SET_TTL_BIT", c_uint32_literal(contract.v2.set_ttl_bit)),
    c_define(
      "OPENKACHE_SMITHY_V2_SET_IF_ABSENT_BIT",
      c_uint32_literal(contract.v2.set_if_absent_bit),
    ),
    c_define(
      "OPENKACHE_SMITHY_V2_SET_IF_PRESENT_BIT",
      c_uint32_literal(contract.v2.set_if_present_bit),
    ),
    "",
    c_define("OPENKACHE_SMITHY_V3_ALPN", c_string_literal(contract.v3.alpn)),
    c_define(
      "OPENKACHE_SMITHY_V3_REQUEST_FIXED_BYTES",
      c_uint32_literal(contract.v3.request_fixed_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_V3_RESPONSE_FIXED_BYTES",
      c_uint32_literal(contract.v3.response_fixed_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_V3_MAX_VARUINT_BYTES",
      c_uint32_literal(contract.v3.max_varuint_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_V3_SET_TTL_FLAG",
      c_uint8_literal(contract.v3.set_ttl_flag),
    ),
    c_define(
      "OPENKACHE_SMITHY_V3_SET_IF_ABSENT_FLAG",
      c_uint8_literal(contract.v3.set_if_absent_flag),
    ),
    c_define(
      "OPENKACHE_SMITHY_V3_SET_IF_PRESENT_FLAG",
      c_uint8_literal(contract.v3.set_if_present_flag),
    ),
    c_define("OPENKACHE_SMITHY_ITEM_ID_BYTES", c_uint32_literal(contract.item_id_bytes)),
    c_define("OPENKACHE_SMITHY_MAX_VALUE_BYTES", c_uint32_literal(contract.max_value_bytes)),
    "",
    c_define("OPENKACHE_SMITHY_VALUE_FORMAT_VERSION", c_uint32_literal(value.version)),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_MAX_VU128_BYTES",
      c_uint32_literal(value.max_vu128_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_FORMAT_BYTE_BYTES",
      c_uint32_literal(value.format_byte_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_COMPRESSION_MASK",
      c_uint8_literal(value.format_compression_mask),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_ENCRYPTION_SHIFT",
      c_uint8_literal(value.format_encryption_shift),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_SERIALIZATION_RAW",
      c_uint8_literal(value.serialization_raw),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_SERIALIZATION_JSON",
      c_uint8_literal(value.serialization_json),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_COMPRESSION_NONE",
      c_uint8_literal(value.compression_none),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_COMPRESSION_ZSTANDARD",
      c_uint8_literal(value.compression_zstandard),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_ENCRYPTION_NONE",
      c_uint8_literal(value.encryption_none),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_ENCRYPTION_COMPACT",
      c_uint8_literal(value.encryption_compact),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_ENCRYPTION_ROBUST",
      c_uint8_literal(value.encryption_robust),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_COMPACT_SYNTHETIC_IV_BYTES",
      c_uint32_literal(value.compact_synthetic_iv_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_ROBUST_NONCE_BYTES",
      c_uint32_literal(value.robust_nonce_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_ROBUST_TAG_BYTES",
      c_uint32_literal(value.robust_tag_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES",
      c_uint32_literal(value.data_protection_key_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_ITEM_ID_ROOT_CONTEXT",
      c_string_literal(value.item_id_root_context),
    ),
    c_define("OPENKACHE_SMITHY_VALUE_FORMAT_AAD_DOMAIN", c_string_literal(value.aad_domain)),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_VALUE_ROOT_CONTEXT",
      c_string_literal(value.value_root_context),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_COMPACT_MAC_CONTEXT",
      c_string_literal(value.compact_mac_context),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_COMPACT_ENCRYPTION_CONTEXT",
      c_string_literal(value.compact_encryption_context),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_FORMAT_ROBUST_CONTEXT",
      c_string_literal(value.robust_context),
    ),
    ...envelope_magic.map((byte, index) =>
      c_define(
        `OPENKACHE_SMITHY_VALUE_ENVELOPE_MAGIC_AND_VERSION_${index}`,
        c_uint8_literal(byte),
      )),
    c_define(
      "OPENKACHE_SMITHY_VALUE_ENVELOPE_MAX_ENCODING_BYTES",
      c_uint32_literal(envelope.max_encoding_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES",
      c_uint32_literal(envelope.max_type_name_bytes),
    ),
    c_define(
      "OPENKACHE_SMITHY_VALUE_ENVELOPE_JSON_ENCODING",
      c_string_literal(envelope.json_encoding),
    ),
    "",
    ...contract.opcodes.map((entry) =>
      c_define(
        `OPENKACHE_SMITHY_OPERATION_${c_identifier(entry.name)}`,
        c_uint32_literal(entry.value),
      )),
    ...contract.statuses.map((entry) =>
      c_define(
        `OPENKACHE_SMITHY_STATUS_${c_identifier(entry.name)}`,
        c_uint8_literal(entry.value),
      )),
    "",
    c_define("OPENKACHE_SMITHY_ABI_VERSION", c_uint32_literal(native.abi_version)),
    c_define(
      "OPENKACHE_SMITHY_OPERATION_RECONNECT",
      c_uint32_literal(native.operation_reconnect),
    ),
    c_define(
      "OPENKACHE_SMITHY_OPERATION_CONNECTION_STATE",
      c_uint32_literal(native.operation_connection_state),
    ),
    c_define("OPENKACHE_SMITHY_RESULT_ERROR", c_uint32_literal(native.result_error)),
    c_define("OPENKACHE_SMITHY_RESULT_OK", c_uint32_literal(native.result_ok)),
    c_define("OPENKACHE_SMITHY_RESULT_VALUE", c_uint32_literal(native.result_value)),
    c_define("OPENKACHE_SMITHY_RESULT_NOT_FOUND", c_uint32_literal(native.result_not_found)),
    c_define("OPENKACHE_SMITHY_RESULT_CREATED", c_uint32_literal(native.result_created)),
    c_define("OPENKACHE_SMITHY_RESULT_REPLACED", c_uint32_literal(native.result_replaced)),
    c_define("OPENKACHE_SMITHY_RESULT_DELETED", c_uint32_literal(native.result_deleted)),
    c_define(
      "OPENKACHE_SMITHY_RESULT_NOT_DELETED",
      c_uint32_literal(native.result_not_deleted),
    ),
    c_define("OPENKACHE_SMITHY_RESULT_CONNECTED", c_uint32_literal(native.result_connected)),
    c_define("OPENKACHE_SMITHY_RESULT_NOT_STORED", c_uint32_literal(native.result_not_stored)),
    c_define(
      "OPENKACHE_SMITHY_RESULT_CONNECTION_STATE",
      c_uint32_literal(native.result_connection_state),
    ),
    c_define(
      "OPENKACHE_SMITHY_SET_CONDITION_NONE",
      c_uint32_literal(native.set_condition_none),
    ),
    c_define(
      "OPENKACHE_SMITHY_SET_CONDITION_IF_ABSENT",
      c_uint32_literal(native.set_condition_if_absent),
    ),
    c_define(
      "OPENKACHE_SMITHY_SET_CONDITION_IF_PRESENT",
      c_uint32_literal(native.set_condition_if_present),
    ),
    c_define(
      "OPENKACHE_SMITHY_CONNECTION_STATE_CONNECTED",
      c_uint8_literal(native.connection_state_connected),
    ),
    c_define(
      "OPENKACHE_SMITHY_CONNECTION_STATE_RECONNECTING",
      c_uint8_literal(native.connection_state_reconnecting),
    ),
    c_define(
      "OPENKACHE_SMITHY_CONNECTION_STATE_DISCONNECTED",
      c_uint8_literal(native.connection_state_disconnected),
    ),
    c_define(
      "OPENKACHE_SMITHY_CONNECTION_STATE_CLOSED",
      c_uint8_literal(native.connection_state_closed),
    ),
    c_define(
      "OPENKACHE_SMITHY_CONNECTION_STATE_UNKNOWN",
      c_uint8_literal(native.connection_state_unknown),
    ),
    "",
    "#endif /* OPENKACHE_SMITHY_CONTRACT_H */",
    "",
  ]
  return `${lines.join("\n")}`
}

/** Renders Smithy operation and value-format declarations for Swift.
 *
 * @param contract - Validated language-neutral wire and value contract.
 * @returns Deterministic Swift source with a trailing newline.
 */
export function render_swift_api(contract: Wire_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const members = enum_.members
      .map(
        (member) =>
          `  case ${swift_property_name(member.name)} = ${swift_string_literal(member.value)}`,
      )
      .join("\n")
    return `/// Values defined by the Smithy ${enum_.name} shape.
public enum Smithy_${typescript_name(enum_.name)}: String, Equatable, Sendable {
${members}
}`
  })
  const structures = contract.api.structures.map((structure) => {
    const name = `Smithy_${typescript_name(structure.name)}`
    if (structure.members.length === 0) {
      return `/// Smithy ${structure.name} structure.
public struct ${name}: Equatable, Sendable {
  public init() {}
}`
    }
    const members = structure.members
      .map(
        (member) =>
          `  /// Smithy ${member.name} member.
  public let ${swift_property_name(member.name)}: ${swift_api_type(member.type, member.required)}`,
      )
      .join("\n")
    const parameters = structure.members
      .map((member) => {
        const default_value = member.required ? "" : " = nil"
        return `    ${swift_property_name(member.name)}: ${swift_api_type(member.type, member.required)}${default_value}`
      })
      .join(",\n")
    const assignments = structure.members
      .map(
        (member) =>
          `    self.${swift_property_name(member.name)} = ${swift_property_name(member.name)}`,
      )
      .join("\n")
    return `/// Smithy ${structure.name} structure.
public struct ${name}: Equatable, Sendable {
${members}

  public init(
${parameters}
  ) {
${assignments}
  }
}`
  })
  const operations = contract.api.operations
    .map(
      (operation) =>
        `  /// Invokes the Smithy ${operation.name} operation.
  func ${swift_property_name(operation.name)}(
    _ input: Smithy_${typescript_name(operation.input)}
  ) async throws -> Smithy_${typescript_name(operation.output)}`,
    )
    .join("\n")
  const opcodes = contract.opcodes
    .map(
      (opcode) =>
        `  case ${swift_property_name(opcode.name)} = ${opcode.value}`,
    )
    .join("\n")
  const value = contract.value_format
  const wire = contract
  const native = contract.native
  const version_bytes = encode_vu128(value.version)
  const connection_states = [
    ["connected", native.connection_state_connected],
    ["reconnecting", native.connection_state_reconnecting],
    ["disconnected", native.connection_state_disconnected],
    ["closed", native.connection_state_closed],
    ["unknown", native.connection_state_unknown],
  ] as const
  return `// Generated from the OpenKache Smithy contract. Do not edit.

import Foundation

${[...enums, ...structures].join("\n\n")}

/// Operations defined by the OpenKache Smithy service.
public protocol Smithy_OpenKache_Api: Sendable {
${operations}
}

/// Operation identifiers assigned by the Smithy wire contract.
public enum Smithy_Opcode: UInt8, Equatable, Sendable {
${opcodes}
}

/// Wire and value-format identifiers shared by all language bindings.
public enum Smithy_Value_Format: Sendable {
  public static let protocolAlpn: String = ${swift_string_literal(wire.v3.alpn)}
  public static let itemIdBytes: Int = ${wire.item_id_bytes}
  public static let maxValueBytes: Int = ${wire.max_value_bytes}
  public static let version: Int = ${value.version}
  public static let versionBytes: [UInt8] = [${version_bytes.join(", ")}]
  public static let maxVu128Bytes: Int = ${value.max_vu128_bytes}
  public static let formatByteBytes: Int = ${value.format_byte_bytes}
  public static let maxVaruintBytes: Int = ${wire.v3.max_varuint_bytes}
  public static let setTtlFlag: UInt8 = ${wire.v3.set_ttl_flag}
  public static let setIfAbsentFlag: UInt8 = ${wire.v3.set_if_absent_flag}
  public static let setIfPresentFlag: UInt8 = ${wire.v3.set_if_present_flag}
  public static let formatCompressionMask: UInt8 = ${value.format_compression_mask}
  public static let formatEncryptionShift: UInt8 = ${value.format_encryption_shift}
  public static let serializationRaw: UInt8 = ${value.serialization_raw}
  public static let serializationJson: UInt8 = ${value.serialization_json}
  public static let compressionNone: UInt8 = ${value.compression_none}
  public static let compressionZstandard: UInt8 = ${value.compression_zstandard}
  public static let encryptionNone: UInt8 = ${value.encryption_none}
  public static let encryptionCompact: UInt8 = ${value.encryption_compact}
  public static let encryptionRobust: UInt8 = ${value.encryption_robust}
  public static let compactSyntheticIvBytes: Int = ${value.compact_synthetic_iv_bytes}
  public static let robustNonceBytes: Int = ${value.robust_nonce_bytes}
  public static let robustTagBytes: Int = ${value.robust_tag_bytes}
  public static let dataProtectionKeyBytes: Int = ${value.data_protection_key_bytes}
  public static let itemIdRootContext: String = ${swift_string_literal(value.item_id_root_context)}
  public static let aadDomain: String = ${swift_string_literal(value.aad_domain)}
  public static let valueRootContext: String = ${swift_string_literal(value.value_root_context)}
  public static let compactMacContext: String = ${swift_string_literal(value.compact_mac_context)}
  public static let compactEncryptionContext: String = ${swift_string_literal(value.compact_encryption_context)}
  public static let robustContext: String = ${swift_string_literal(value.robust_context)}
}

/// Native ABI discriminators shared by every language adapter.
public enum Smithy_Connection_State: UInt32, Sendable {
${connection_states.map(([name, discriminator]) => `  case ${name} = ${discriminator}`).join("\n")}
}

/// Native ABI discriminators shared by every language adapter.
public enum Smithy_Native_Contract: Sendable {
  public static let abiVersion: UInt32 = ${native.abi_version}
  public static let operationReconnect: UInt32 = ${native.operation_reconnect}
  public static let operationConnectionState: UInt32 = ${native.operation_connection_state}
  public static let resultError: UInt32 = ${native.result_error}
  public static let resultOk: UInt32 = ${native.result_ok}
  public static let resultValue: UInt32 = ${native.result_value}
  public static let resultNotFound: UInt32 = ${native.result_not_found}
  public static let resultCreated: UInt32 = ${native.result_created}
  public static let resultReplaced: UInt32 = ${native.result_replaced}
  public static let resultDeleted: UInt32 = ${native.result_deleted}
  public static let resultNotDeleted: UInt32 = ${native.result_not_deleted}
  public static let resultConnected: UInt32 = ${native.result_connected}
  public static let resultNotStored: UInt32 = ${native.result_not_stored}
  public static let resultConnectionState: UInt32 = ${native.result_connection_state}
  public static let setConditionNone: UInt32 = ${native.set_condition_none}
  public static let setConditionIfAbsent: UInt32 = ${native.set_condition_if_absent}
  public static let setConditionIfPresent: UInt32 = ${native.set_condition_if_present}
  public static let connectionStateConnected: UInt32 = ${native.connection_state_connected}
  public static let connectionStateReconnecting: UInt32 = ${native.connection_state_reconnecting}
  public static let connectionStateDisconnected: UInt32 = ${native.connection_state_disconnected}
  public static let connectionStateClosed: UInt32 = ${native.connection_state_closed}
  public static let connectionStateUnknown: UInt32 = ${native.connection_state_unknown}
}
`
}

/** Renders the cross-language value-format wire and cryptographic contract for TypeScript.
 *
 * @param contract - Validated language-neutral wire and value-format contract.
 * @returns Deterministic TypeScript source with a trailing newline.
 */
export function render_typescript_value_format(contract: Wire_Contract): string {
  const value = contract.value_format
  const version_bytes = encode_vu128(value.version)
  return `// Generated from the OpenKache Smithy contract. Do not edit.

/** Current client-owned value-format version. */
export const SMITHY_VALUE_FORMAT_VERSION = ${value.version}
/** Canonical VU128 bytes for the current value-format version. */
export const SMITHY_VALUE_FORMAT_VERSION_BYTES = [${version_bytes.join(", ")}] as const
/** Maximum bytes accepted for a canonical value-format VU128. */
export const SMITHY_VALUE_FORMAT_MAX_VU128_BYTES = ${value.max_vu128_bytes}
/** Bytes occupied by the value-format transform byte. */
export const SMITHY_VALUE_FORMAT_FORMAT_BYTE_BYTES = ${value.format_byte_bytes}
/** Low-nibble mask for the value-format compression identifier. */
export const SMITHY_VALUE_FORMAT_COMPRESSION_MASK = ${value.format_compression_mask}
/** Number of bits to shift the value-format encryption identifier. */
export const SMITHY_VALUE_FORMAT_ENCRYPTION_SHIFT = ${value.format_encryption_shift}
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
/** Compact AES-SIV synthetic-IV and authentication-tag size. */
export const SMITHY_VALUE_COMPACT_SYNTHETIC_IV_BYTES = ${value.compact_synthetic_iv_bytes}
/** Robust AES-GCM-SIV nonce size. */
export const SMITHY_VALUE_ROBUST_NONCE_BYTES = ${value.robust_nonce_bytes}
/** Robust AES-GCM-SIV authentication-tag size. */
export const SMITHY_VALUE_ROBUST_TAG_BYTES = ${value.robust_tag_bytes}
/** Application-managed data-protection key size. */
export const SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES = ${value.data_protection_key_bytes}
/** BLAKE3 protected-item-ID root derivation context. */
export const SMITHY_VALUE_ITEM_ID_ROOT_CONTEXT = ${JSON.stringify(value.item_id_root_context)}
/** Associated-data domain separator. */
export const SMITHY_VALUE_AAD_DOMAIN = ${JSON.stringify(value.aad_domain)}
/** BLAKE3 value-root derivation context. */
export const SMITHY_VALUE_VALUE_ROOT_CONTEXT = ${JSON.stringify(value.value_root_context)}
/** BLAKE3 Compact AES-SIV MAC-key derivation context. */
export const SMITHY_VALUE_COMPACT_MAC_CONTEXT = ${JSON.stringify(value.compact_mac_context)}
/** BLAKE3 Compact AES-SIV encryption-key derivation context. */
export const SMITHY_VALUE_COMPACT_ENCRYPTION_CONTEXT = ${JSON.stringify(value.compact_encryption_context)}
/** BLAKE3 Robust AES-GCM-SIV key derivation context. */
export const SMITHY_VALUE_ROBUST_CONTEXT = ${JSON.stringify(value.robust_context)}
`
}

/** Renders constants for the legacy TypeScript metadata envelope.
 *
 * @param contract - Validated language-neutral value-envelope contract.
 * @returns Deterministic TypeScript source with a trailing newline.
 */
export function render_typescript_value_envelope(contract: Wire_Contract): string {
  const envelope = contract.value_envelope
  const magic = bytes_from_hex(envelope.magic_and_version_hex, "value envelope magic")
  return `// Generated from the OpenKache Smithy contract. Do not edit.

/** Legacy metadata-envelope magic and version. */
export const SMITHY_VALUE_ENVELOPE_MAGIC_AND_VERSION = [${magic.join(", ")}] as const
/** Maximum UTF-8 byte length of a legacy metadata-envelope encoding identifier. */
export const SMITHY_VALUE_ENVELOPE_MAX_ENCODING_BYTES = ${envelope.max_encoding_bytes}
/** Maximum UTF-8 byte length of a legacy metadata-envelope logical type name. */
export const SMITHY_VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES = ${envelope.max_type_name_bytes}
/** Built-in canonical JSON codec identifier used by the legacy envelope adapter. */
export const SMITHY_VALUE_ENVELOPE_JSON_ENCODING = ${JSON.stringify(envelope.json_encoding)}
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

type Generation_Target =
  | "all"
  | "c-header"
  | "dotnet"
  | "rust-api"
  | "rust-wire"
  | "swift"
  | "typescript"

function generation_target(value: string | undefined): Generation_Target {
  switch (value) {
    case undefined:
    case "all":
      return "all"
    case "c-header":
      return "c-header"
    case "dotnet":
      return "dotnet"
    case "rust-api":
      return "rust-api"
    case "rust-wire":
      return "rust-wire"
    case "swift":
      return "swift"
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
  switch (target) {
    case "all":
      return {
        [GENERATED_OUTPUTS.c_header]: render_c_header(contract),
        [GENERATED_OUTPUTS.csharp_api]: render_csharp_api(contract),
        [GENERATED_OUTPUTS.csharp_wire]: render_csharp(contract),
        [GENERATED_OUTPUTS.rust_api]: render_rust_api(contract),
        [GENERATED_OUTPUTS.rust_wire]: render_rust(contract),
        [GENERATED_OUTPUTS.typescript_api]: render_typescript_api(contract),
        [GENERATED_OUTPUTS.typescript_value_format]:
          render_typescript_value_format(contract),
        [GENERATED_OUTPUTS.typescript_value_envelope]:
          render_typescript_value_envelope(contract),
        [GENERATED_OUTPUTS.swift_api]: render_swift_api(contract),
      }
    case "c-header":
      return {
        [GENERATED_OUTPUTS.c_header]: render_c_header(contract),
      }
    case "dotnet":
      return {
        [GENERATED_OUTPUTS.csharp_api]: render_csharp_api(contract),
        [GENERATED_OUTPUTS.csharp_wire]: render_csharp(contract),
      }
    case "rust-api":
      return {
        [GENERATED_OUTPUTS.rust_api]: render_rust_api(contract),
      }
    case "rust-wire":
      return {
        [GENERATED_OUTPUTS.rust_wire]: render_rust(contract),
      }
    case "swift":
      return {
        [GENERATED_OUTPUTS.swift_api]: render_swift_api(contract),
      }
    case "typescript":
      return {
        [GENERATED_OUTPUTS.typescript_api]: render_typescript_api(contract),
        [GENERATED_OUTPUTS.typescript_value_format]:
          render_typescript_value_format(contract),
        [GENERATED_OUTPUTS.typescript_value_envelope]:
          render_typescript_value_envelope(contract),
      }
  }
}

function write_outputs(outputs: Readonly<Record<string, string>>): void {
  for (const [output_path, content] of Object.entries(outputs)) {
    const output_directory = dirname(output_path)
    mkdirSync(output_directory, { recursive: true })
    // Parallel build recipes may generate overlapping targets; rename a complete
    // temporary file so readers never observe a partially written contract.
    const temporary_directory = mkdtempSync(join(output_directory, "generate.local."))
    const temporary_path = join(temporary_directory, basename(output_path))
    try {
      writeFileSync(temporary_path, content)
      renameSync(temporary_path, output_path)
      console.log(`Generated ${output_path}`)
    } finally {
      rmSync(temporary_directory, { force: true, recursive: true })
    }
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
