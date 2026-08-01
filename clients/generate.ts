#!/usr/bin/env bun
/** Generates client-owned Smithy contracts and their generated language bindings. */

import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { basename, dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import {
  extract_wire_contract as extract_protocol_wire_contract,
  render_rust_wire as render_protocol_rust_wire,
  type Wire_Contract,
  type Wire_Entry,
} from "../protocol/wire"

type Json_Object = Readonly<Record<string, unknown>>

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

/** Defaults shared by the Rust client core and its native language adapters. */
export interface Client_Defaults_Contract {
  readonly connect_timeout_milliseconds: number
  readonly max_in_flight: number
  readonly max_previous_data_protection_keys: number
  readonly mutation_id_bytes: number
  readonly request_timeout_milliseconds: number
  readonly retry_max_attempts: number
  readonly zstandard_level: number
  readonly zstandard_minimum_input_bytes: number
  readonly zstandard_minimum_savings_bytes: number
  readonly zstandard_level_min: number
  readonly zstandard_level_max: number
  readonly server_name: string
  readonly certificate_pem_type: string
  readonly minimum_positive_value: number
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

/** Native binding ABI identifiers shared by language-neutral adapters. */
export interface Ffi_Contract {
  readonly abi_version: number
  readonly backends: readonly Wire_Entry[]
  readonly connection_states: readonly Wire_Entry[]
  readonly error_codes: readonly Wire_Entry[]
  readonly metrics: readonly Wire_Entry[]
  readonly operations: readonly Wire_Entry[]
  readonly phases: readonly Wire_Entry[]
  readonly result_kinds: readonly Wire_Entry[]
  readonly set_conditions: readonly Wire_Entry[]
}

/** Native C ABI structure sizes and byte offsets shared by raw FFI adapters. */
export interface Ffi_Layout_Contract {
  readonly connect_options_bytes: number
  readonly connect_address_offset: number
  readonly connect_address_length_offset: number
  readonly connect_server_name_offset: number
  readonly connect_server_name_length_offset: number
  readonly connect_certificate_offset: number
  readonly connect_certificate_length_offset: number
  readonly connect_client_certificate_chain_offset: number
  readonly connect_client_certificate_chain_length_offset: number
  readonly connect_client_private_key_offset: number
  readonly connect_client_private_key_length_offset: number
  readonly connect_data_protection_key_offset: number
  readonly connect_data_protection_key_length_offset: number
  readonly connect_previous_data_protection_keys_offset: number
  readonly connect_previous_data_protection_keys_length_offset: number
  readonly connect_previous_data_protection_key_count_offset: number
  readonly connect_compression_enabled_offset: number
  readonly connect_compression_level_offset: number
  readonly connect_minimum_input_size_offset: number
  readonly connect_minimum_savings_offset: number
  readonly connect_encryption_offset: number
  readonly connect_timeout_offset: number
  readonly connect_request_timeout_offset: number
  readonly connect_retry_max_attempts_offset: number
  readonly connect_max_in_flight_offset: number
  readonly error_metadata_bytes: number
  readonly error_metadata_code_offset: number
  readonly error_metadata_operation_offset: number
  readonly error_metadata_phase_offset: number
  readonly error_metadata_backend_offset: number
  readonly error_metadata_retryable_offset: number
  readonly error_metadata_ambiguous_offset: number
  readonly error_metadata_mutation_id_length_offset: number
  readonly error_metadata_mutation_id_offset: number
  readonly metrics_snapshot_bytes: number
  readonly metrics_snapshot_requests_offset: number
  readonly metrics_snapshot_hits_offset: number
  readonly metrics_snapshot_misses_offset: number
  readonly metrics_snapshot_retries_offset: number
  readonly metrics_snapshot_reconnects_offset: number
  readonly metrics_snapshot_cancellations_offset: number
  readonly metrics_snapshot_transport_errors_offset: number
  readonly metrics_snapshot_protocol_errors_offset: number
  readonly metrics_snapshot_bytes_sent_offset: number
  readonly metrics_snapshot_bytes_received_offset: number
  readonly metrics_snapshot_active_lanes_offset: number
}

/** Wire contract combined with the client-owned Smithy model. */
export interface Client_Contract extends Wire_Contract {
  readonly api: Api_Contract
  readonly client_defaults: Client_Defaults_Contract
  readonly ffi: Ffi_Contract
  readonly ffi_layout: Ffi_Layout_Contract
  readonly value_format: Value_Format_Contract
}

const CLIENTS_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const PUBLIC_ROOT = dirname(CLIENTS_DIRECTORY)
const PROTOCOL_DIRECTORY = join(PUBLIC_ROOT, "protocol")
const MODEL_DIRECTORY = "model"
const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
const CLIENT_SERVICE_SHAPE_ID = "openkache.client#OpenKacheClient"
const FFI_CONTRACT_TRAIT_ID = "openkache.client#ffiContract"
const CLIENT_DEFAULTS_TRAIT_ID = "openkache.client#clientDefaults"
const VALUE_FORMAT_TRAIT_ID = "openkache.client#valueFormat"
const FFI_LAYOUT_TRAIT_ID = "openkache.client#ffiLayout"
const FFI_OPERATION_FIELDS = [
  { name: "GetJson", field: "operationGetJson" },
  { name: "SetJson", field: "operationSetJson" },
  { name: "Reconnect", field: "operationReconnect" },
] as const
const FFI_ERROR_FIELDS = [
  { name: "Configuration", field: "errorConfiguration" },
  { name: "Connection", field: "errorConnection" },
  { name: "Timeout", field: "errorTimeout" },
  { name: "Runtime", field: "errorRuntime" },
  { name: "Transport", field: "errorTransport" },
  { name: "Server", field: "errorServer" },
  { name: "UnexpectedResponse", field: "errorUnexpectedResponse" },
  { name: "ResponseTooLarge", field: "errorResponseTooLarge" },
  { name: "Tls", field: "errorTls" },
  { name: "Protocol", field: "errorProtocol" },
  { name: "Io", field: "errorIo" },
  { name: "Value", field: "errorValue" },
  { name: "Closed", field: "errorClosed" },
  { name: "Ambiguous", field: "errorAmbiguous" },
  { name: "Cancelled", field: "errorCancelled" },
] as const
const FFI_PHASE_FIELDS = [
  { name: "Unknown", field: "phaseUnknown" },
  { name: "DnsResolution", field: "phaseDnsResolution" },
  { name: "ConnectionSetup", field: "phaseConnectionSetup" },
  { name: "ConnectionRetry", field: "phaseConnectionRetry" },
  { name: "StreamAcquisition", field: "phaseStreamAcquisition" },
  { name: "RequestWrite", field: "phaseRequestWrite" },
  { name: "ResponseHeaderRead", field: "phaseResponseHeaderRead" },
  { name: "ResponseBodyRead", field: "phaseResponseBodyRead" },
  { name: "TlsInitialization", field: "phaseTlsInitialization" },
  { name: "EndpointInitialization", field: "phaseEndpointInitialization" },
  { name: "ConnectionInitialization", field: "phaseConnectionInitialization" },
  { name: "Handshake", field: "phaseHandshake" },
  { name: "StreamOpen", field: "phaseStreamOpen" },
  { name: "StreamWrite", field: "phaseStreamWrite" },
  { name: "StreamRead", field: "phaseStreamRead" },
] as const
const FFI_BACKEND_FIELDS = [
  { name: "None", field: "backendNone" },
  { name: "Quinn", field: "backendQuinn" },
  { name: "Compio", field: "backendCompio" },
] as const
const FFI_METRICS_FIELDS = [
  { name: "Requests", field: "metricsRequests" },
  { name: "Hits", field: "metricsHits" },
  { name: "Misses", field: "metricsMisses" },
  { name: "Retries", field: "metricsRetries" },
  { name: "Reconnects", field: "metricsReconnects" },
  { name: "Cancellations", field: "metricsCancellations" },
  { name: "TransportErrors", field: "metricsTransportErrors" },
  { name: "ProtocolErrors", field: "metricsProtocolErrors" },
  { name: "BytesSent", field: "metricsBytesSent" },
  { name: "BytesReceived", field: "metricsBytesReceived" },
  { name: "ActiveLanes", field: "metricsActiveLanes" },
] as const
const FFI_RESULT_FIELDS = [
  { name: "Error", field: "resultError" },
  { name: "Ok", field: "resultOk" },
  { name: "Value", field: "resultValue" },
  { name: "NotFound", field: "resultNotFound" },
  { name: "Created", field: "resultCreated" },
  { name: "Replaced", field: "resultReplaced" },
  { name: "Deleted", field: "resultDeleted" },
  { name: "NotDeleted", field: "resultNotDeleted" },
  { name: "Connected", field: "resultConnected" },
  { name: "NotStored", field: "resultNotStored" },
] as const
const FFI_CONNECTION_STATE_FIELDS = [
  { name: "Connected", field: "connectionStateConnected" },
  { name: "Reconnecting", field: "connectionStateReconnecting" },
  { name: "Disconnected", field: "connectionStateDisconnected" },
  { name: "Closed", field: "connectionStateClosed" },
  { name: "Unknown", field: "connectionStateUnknown" },
] as const
const FFI_SET_CONDITION_FIELDS = [
  { name: "None", field: "setConditionNone" },
  { name: "IfAbsent", field: "setConditionIfAbsent" },
  { name: "IfPresent", field: "setConditionIfPresent" },
] as const
const GENERATED_OUTPUT_ROOT = resolve(
  process.env.OPENKACHE_GENERATION_OUTPUT_ROOT ?? PUBLIC_ROOT,
)
function generated_path(...segments: string[]): string {
  return join(GENERATED_OUTPUT_ROOT, ...segments)
}
const GENERATED_OUTPUTS = {
  csharp_api: generated_path("clients/dotnet/OpenKache/generated_local/SmithyApi.g.cs"),
  csharp_wire: generated_path("clients/dotnet/OpenKache/generated_local/WireValues.g.cs"),
  rust_client: process.env.OPENKACHE_RUST_CLIENT_OUTPUT ??
    generated_path("clients/core/generated_local/client_contract.rs"),
  rust_api: process.env.OPENKACHE_RUST_API_OUTPUT ??
    generated_path("clients/rust/generated_local/smithy_api.rs"),
  rust_wire: process.env.OPENKACHE_RUST_WIRE_OUTPUT ??
    generated_path("protocol/generated_local/wire_values.rs"),
  typescript_api: generated_path("clients/typescript/src/generated_local/smithy-api.ts"),
  typescript_value_format: generated_path(
    "clients/typescript/src/generated_local/smithy-value-format.ts",
  ),
  python_api: process.env.OPENKACHE_PYTHON_API_OUTPUT ??
    generated_path("clients/python/src/openkache/_generated/smithy_api.py"),
  python_contract: process.env.OPENKACHE_PYTHON_CONTRACT_OUTPUT ??
    generated_path("clients/python/src/openkache/_generated/smithy_contract.py"),
  swift_api: process.env.OPENKACHE_SWIFT_API_OUTPUT ??
    generated_path("clients/swift/generated_local/SmithyAPI.swift"),
  c_contract: process.env.OPENKACHE_C_CONTRACT_OUTPUT ??
    generated_path("clients/core/generated_local/smithy_contract.h"),
  go_api: generated_path("clients/go/smithy_api.go"),
  go_contract: generated_path("clients/go/smithy_contract.go"),
  java_api: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated/SmithyApi.java",
  ),
  java_contract: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated/SmithyContract.java",
  ),
  kotlin_api: generated_path(
    "clients/kotlin/src/main/kotlin/io/openkache/client/generated/SmithyApi.kt",
  ),
  kotlin_contract: generated_path(
    "clients/kotlin/src/main/kotlin/io/openkache/client/generated/SmithyContract.kt",
  ),
  dart_contract: generated_path("clients/dart/lib/generated_contract.dart"),
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
      const normalized = part === part.toUpperCase() ? part.toLowerCase() : part
      return normalized.length === 0
        ? ""
        : `${normalized[0]?.toUpperCase()}${normalized.slice(1)}`
    })
    .join("")
}

function trait_value_any(
  shape: Json_Object,
  trait_ids: readonly string[],
  location: string,
): Json_Object {
  const traits = object_member(shape, "traits", location)
  for (const trait_id of trait_ids) {
    const value = traits[trait_id]
    if (value !== undefined) return object_value(value, `${location}.traits.${trait_id}`)
  }
  throw new Error(
    `${location}.traits is missing one of ${trait_ids.map((trait_id) => JSON.stringify(trait_id)).join(", ")}`,
  )
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
    "smithy.api#Blob": "blob",
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

function api_enum(shapes: Json_Object, namespace: string, name: string): Api_Enum {
  const shape_id = `${namespace}#${name}`
  const shape = object_member(shapes, shape_id, "Smithy AST.shapes")
  if (shape_type(shape, `Smithy AST.shapes.${shape_id}`) !== "enum") {
    throw new Error(`${shape_id} must be an enum`)
  }
  const members = object_member(shape, "members", shape_id)
  const enum_members = Object.entries(members).map(
    ([member_name, value]): Api_Enum_Member => {
      const member = object_value(value, `${shape_id}.${member_name}`)
      const traits = object_member(member, "traits", `${shape_id}.${member_name}`)
      const enum_value = string_member(
        traits,
        "smithy.api#enumValue",
        `${shape_id}.${member_name}.traits`,
      )
      return {
        // Smithy AST normalizes uppercase enum symbols inconsistently across
        // versions (for example, `IF_ABSENT` may arrive as `IFABSENT`).
        // The wire value is stable and already uses snake_case, so derive
        // language enum identifiers from it instead of the parser spelling.
        name: pascal_case(enum_value),
        value: enum_value,
      }
    },
  )
  const member_names = new Set<string>()
  const member_values = new Set<string>()
  for (const member of enum_members) {
    if (member_values.has(member.value)) {
      throw new Error(`duplicate ${name} enum value ${member.value}`)
    }
    member_values.add(member.value)
  }
  for (const member of enum_members) {
    if (member_names.has(member.name)) {
      throw new Error(`duplicate ${name} enum member name ${member.name}`)
    }
    member_names.add(member.name)
  }
  return {
    name,
    members: enum_members,
  }
}

function api_contract(
  shapes: Json_Object,
  service_shape_id: string,
  namespace: string,
): Api_Contract {
  const service = object_member(shapes, service_shape_id, "Smithy AST.shapes")
  const operation_shapes = array_member(service, "operations", service_shape_id)
    .map((operation, index): Api_Operation => {
      const reference = object_value(operation, `${service_shape_id}.operations[${index}]`)
      const target = string_member(
        reference,
        "target",
        `${service_shape_id}.operations[${index}]`,
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
        input: shape_name(input),
        name: shape_name(target),
        output: shape_name(output),
      }
    })

  const structure_names = new Set<string>()
  for (const operation of operation_shapes) {
    structure_names.add(operation.input)
    structure_names.add(operation.output)
  }
  const structures = [...structure_names]
    .map((name) => api_structure(shapes, `${namespace}#${name}`))
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
      .map((name) => api_enum(shapes, namespace, name))
      .sort((left, right) => left.name.localeCompare(right.name)),
    operations: operation_shapes,
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

function ffi_entries(
  contract: Json_Object,
  fields: readonly { readonly name: string; readonly field: string }[],
  kind: string,
): readonly Wire_Entry[] {
  const entries = fields.map(
    ({ name, field }): Wire_Entry => ({
      name,
      value: integer_member(
        contract,
        field,
        `${FFI_CONTRACT_TRAIT_ID}.${field}`,
        0,
        0xffff_ffff,
      ),
    }),
  )
  unique_wire_values(entries, kind)
  return entries
}

function ffi_contract(value: unknown): Ffi_Contract {
  const contract = object_value(value, FFI_CONTRACT_TRAIT_ID)
  return {
    abi_version: integer_member(
      contract,
      "abiVersion",
      `${FFI_CONTRACT_TRAIT_ID}.abiVersion`,
      1,
      0xffff_ffff,
    ),
    backends: ffi_entries(contract, FFI_BACKEND_FIELDS, "FFI backend"),
    connection_states: ffi_entries(
      contract,
      FFI_CONNECTION_STATE_FIELDS,
      "FFI connection state",
    ),
    error_codes: ffi_entries(contract, FFI_ERROR_FIELDS, "FFI error code"),
    metrics: ffi_entries(contract, FFI_METRICS_FIELDS, "FFI metric"),
    operations: ffi_entries(contract, FFI_OPERATION_FIELDS, "FFI operation"),
    phases: ffi_entries(contract, FFI_PHASE_FIELDS, "FFI phase"),
    result_kinds: ffi_entries(contract, FFI_RESULT_FIELDS, "FFI result kind"),
    set_conditions: ffi_entries(
      contract,
      FFI_SET_CONDITION_FIELDS,
      "FFI SET condition",
    ),
  }
}

function ffi_layout_member(
  contract: Json_Object,
  field: string,
  minimum = 0,
): number {
  return integer_member(
    contract,
    field,
    `${FFI_LAYOUT_TRAIT_ID}.${field}`,
    minimum,
    0xffff_ffff,
  )
}

function ffi_layout_contract(
  value: unknown,
  mutation_id_bytes: number,
): Ffi_Layout_Contract {
  const contract = object_value(value, FFI_LAYOUT_TRAIT_ID)
  const layout = {
    connect_options_bytes: ffi_layout_member(contract, "connectOptionsBytes", 1),
    connect_address_offset: ffi_layout_member(contract, "connectAddressOffset"),
    connect_address_length_offset: ffi_layout_member(
      contract,
      "connectAddressLengthOffset",
    ),
    connect_server_name_offset: ffi_layout_member(contract, "connectServerNameOffset"),
    connect_server_name_length_offset: ffi_layout_member(
      contract,
      "connectServerNameLengthOffset",
    ),
    connect_certificate_offset: ffi_layout_member(contract, "connectCertificateOffset"),
    connect_certificate_length_offset: ffi_layout_member(
      contract,
      "connectCertificateLengthOffset",
    ),
    connect_client_certificate_chain_offset: ffi_layout_member(
      contract,
      "connectClientCertificateChainOffset",
    ),
    connect_client_certificate_chain_length_offset: ffi_layout_member(
      contract,
      "connectClientCertificateChainLengthOffset",
    ),
    connect_client_private_key_offset: ffi_layout_member(
      contract,
      "connectClientPrivateKeyOffset",
    ),
    connect_client_private_key_length_offset: ffi_layout_member(
      contract,
      "connectClientPrivateKeyLengthOffset",
    ),
    connect_data_protection_key_offset: ffi_layout_member(
      contract,
      "connectDataProtectionKeyOffset",
    ),
    connect_data_protection_key_length_offset: ffi_layout_member(
      contract,
      "connectDataProtectionKeyLengthOffset",
    ),
    connect_previous_data_protection_keys_offset: ffi_layout_member(
      contract,
      "connectPreviousDataProtectionKeysOffset",
    ),
    connect_previous_data_protection_keys_length_offset: ffi_layout_member(
      contract,
      "connectPreviousDataProtectionKeysLengthOffset",
    ),
    connect_previous_data_protection_key_count_offset: ffi_layout_member(
      contract,
      "connectPreviousDataProtectionKeyCountOffset",
    ),
    connect_compression_enabled_offset: ffi_layout_member(
      contract,
      "connectCompressionEnabledOffset",
    ),
    connect_compression_level_offset: ffi_layout_member(
      contract,
      "connectCompressionLevelOffset",
    ),
    connect_minimum_input_size_offset: ffi_layout_member(
      contract,
      "connectMinimumInputSizeOffset",
    ),
    connect_minimum_savings_offset: ffi_layout_member(
      contract,
      "connectMinimumSavingsOffset",
    ),
    connect_encryption_offset: ffi_layout_member(contract, "connectEncryptionOffset"),
    connect_timeout_offset: ffi_layout_member(contract, "connectTimeoutOffset"),
    connect_request_timeout_offset: ffi_layout_member(
      contract,
      "connectRequestTimeoutOffset",
    ),
    connect_retry_max_attempts_offset: ffi_layout_member(
      contract,
      "connectRetryMaxAttemptsOffset",
    ),
    connect_max_in_flight_offset: ffi_layout_member(
      contract,
      "connectMaxInFlightOffset",
    ),
    error_metadata_bytes: ffi_layout_member(contract, "errorMetadataBytes", 1),
    error_metadata_code_offset: ffi_layout_member(
      contract,
      "errorMetadataCodeOffset",
    ),
    error_metadata_operation_offset: ffi_layout_member(
      contract,
      "errorMetadataOperationOffset",
    ),
    error_metadata_phase_offset: ffi_layout_member(
      contract,
      "errorMetadataPhaseOffset",
    ),
    error_metadata_backend_offset: ffi_layout_member(
      contract,
      "errorMetadataBackendOffset",
    ),
    error_metadata_retryable_offset: ffi_layout_member(
      contract,
      "errorMetadataRetryableOffset",
    ),
    error_metadata_ambiguous_offset: ffi_layout_member(
      contract,
      "errorMetadataAmbiguousOffset",
    ),
    error_metadata_mutation_id_length_offset: ffi_layout_member(
      contract,
      "errorMetadataMutationIdLengthOffset",
    ),
    error_metadata_mutation_id_offset: ffi_layout_member(
      contract,
      "errorMetadataMutationIdOffset",
    ),
    metrics_snapshot_bytes: ffi_layout_member(contract, "metricsSnapshotBytes", 1),
    metrics_snapshot_requests_offset: ffi_layout_member(
      contract,
      "metricsSnapshotRequestsOffset",
    ),
    metrics_snapshot_hits_offset: ffi_layout_member(
      contract,
      "metricsSnapshotHitsOffset",
    ),
    metrics_snapshot_misses_offset: ffi_layout_member(
      contract,
      "metricsSnapshotMissesOffset",
    ),
    metrics_snapshot_retries_offset: ffi_layout_member(
      contract,
      "metricsSnapshotRetriesOffset",
    ),
    metrics_snapshot_reconnects_offset: ffi_layout_member(
      contract,
      "metricsSnapshotReconnectsOffset",
    ),
    metrics_snapshot_cancellations_offset: ffi_layout_member(
      contract,
      "metricsSnapshotCancellationsOffset",
    ),
    metrics_snapshot_transport_errors_offset: ffi_layout_member(
      contract,
      "metricsSnapshotTransportErrorsOffset",
    ),
    metrics_snapshot_protocol_errors_offset: ffi_layout_member(
      contract,
      "metricsSnapshotProtocolErrorsOffset",
    ),
    metrics_snapshot_bytes_sent_offset: ffi_layout_member(
      contract,
      "metricsSnapshotBytesSentOffset",
    ),
    metrics_snapshot_bytes_received_offset: ffi_layout_member(
      contract,
      "metricsSnapshotBytesReceivedOffset",
    ),
    metrics_snapshot_active_lanes_offset: ffi_layout_member(
      contract,
      "metricsSnapshotActiveLanesOffset",
    ),
  } satisfies Ffi_Layout_Contract

  const bounded = [
    ["connectAddressOffset", layout.connect_address_offset],
    ["connectAddressLengthOffset", layout.connect_address_length_offset],
    ["connectServerNameOffset", layout.connect_server_name_offset],
    ["connectServerNameLengthOffset", layout.connect_server_name_length_offset],
    ["connectCertificateOffset", layout.connect_certificate_offset],
    ["connectCertificateLengthOffset", layout.connect_certificate_length_offset],
    [
      "connectClientCertificateChainOffset",
      layout.connect_client_certificate_chain_offset,
    ],
    [
      "connectClientCertificateChainLengthOffset",
      layout.connect_client_certificate_chain_length_offset,
    ],
    ["connectClientPrivateKeyOffset", layout.connect_client_private_key_offset],
    [
      "connectClientPrivateKeyLengthOffset",
      layout.connect_client_private_key_length_offset,
    ],
    ["connectDataProtectionKeyOffset", layout.connect_data_protection_key_offset],
    [
      "connectDataProtectionKeyLengthOffset",
      layout.connect_data_protection_key_length_offset,
    ],
    [
      "connectPreviousDataProtectionKeysOffset",
      layout.connect_previous_data_protection_keys_offset,
    ],
    [
      "connectPreviousDataProtectionKeysLengthOffset",
      layout.connect_previous_data_protection_keys_length_offset,
    ],
    [
      "connectPreviousDataProtectionKeyCountOffset",
      layout.connect_previous_data_protection_key_count_offset,
    ],
    [
      "connectCompressionEnabledOffset",
      layout.connect_compression_enabled_offset,
    ],
    ["connectCompressionLevelOffset", layout.connect_compression_level_offset],
    ["connectMinimumInputSizeOffset", layout.connect_minimum_input_size_offset],
    ["connectMinimumSavingsOffset", layout.connect_minimum_savings_offset],
    ["connectEncryptionOffset", layout.connect_encryption_offset],
    ["connectTimeoutOffset", layout.connect_timeout_offset],
    ["connectRequestTimeoutOffset", layout.connect_request_timeout_offset],
    [
      "connectRetryMaxAttemptsOffset",
      layout.connect_retry_max_attempts_offset,
    ],
    ["connectMaxInFlightOffset", layout.connect_max_in_flight_offset],
  ] as const
  for (const [name, offset] of bounded) {
    if (offset >= layout.connect_options_bytes) {
      throw new Error(
        `${FFI_LAYOUT_TRAIT_ID}.${name} must be smaller than connectOptionsBytes`,
      )
    }
  }
  const metadata_offsets = [
    ["errorMetadataCodeOffset", layout.error_metadata_code_offset],
    ["errorMetadataOperationOffset", layout.error_metadata_operation_offset],
    ["errorMetadataPhaseOffset", layout.error_metadata_phase_offset],
    ["errorMetadataBackendOffset", layout.error_metadata_backend_offset],
    ["errorMetadataRetryableOffset", layout.error_metadata_retryable_offset],
    ["errorMetadataAmbiguousOffset", layout.error_metadata_ambiguous_offset],
    [
      "errorMetadataMutationIdLengthOffset",
      layout.error_metadata_mutation_id_length_offset,
    ],
    ["errorMetadataMutationIdOffset", layout.error_metadata_mutation_id_offset],
  ] as const
  for (const [name, offset] of metadata_offsets) {
    if (offset >= layout.error_metadata_bytes) {
      throw new Error(
        `${FFI_LAYOUT_TRAIT_ID}.${name} must be smaller than errorMetadataBytes`,
      )
    }
  }
  if (
    layout.error_metadata_mutation_id_offset + mutation_id_bytes >
    layout.error_metadata_bytes
  ) {
    throw new Error(
      `${FFI_LAYOUT_TRAIT_ID}.errorMetadataMutationIdOffset leaves no room for a mutation ID`,
    )
  }
  const metrics_offsets = [
    ["metricsSnapshotRequestsOffset", layout.metrics_snapshot_requests_offset],
    ["metricsSnapshotHitsOffset", layout.metrics_snapshot_hits_offset],
    ["metricsSnapshotMissesOffset", layout.metrics_snapshot_misses_offset],
    ["metricsSnapshotRetriesOffset", layout.metrics_snapshot_retries_offset],
    [
      "metricsSnapshotReconnectsOffset",
      layout.metrics_snapshot_reconnects_offset,
    ],
    [
      "metricsSnapshotCancellationsOffset",
      layout.metrics_snapshot_cancellations_offset,
    ],
    [
      "metricsSnapshotTransportErrorsOffset",
      layout.metrics_snapshot_transport_errors_offset,
    ],
    [
      "metricsSnapshotProtocolErrorsOffset",
      layout.metrics_snapshot_protocol_errors_offset,
    ],
    ["metricsSnapshotBytesSentOffset", layout.metrics_snapshot_bytes_sent_offset],
    [
      "metricsSnapshotBytesReceivedOffset",
      layout.metrics_snapshot_bytes_received_offset,
    ],
    ["metricsSnapshotActiveLanesOffset", layout.metrics_snapshot_active_lanes_offset],
  ] as const
  for (const [name, offset] of metrics_offsets) {
    if (offset + 8 > layout.metrics_snapshot_bytes) {
      throw new Error(
        `${FFI_LAYOUT_TRAIT_ID}.${name} exceeds metricsSnapshotBytes`,
      )
    }
  }
  return layout
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

function client_defaults_contract(value: unknown): Client_Defaults_Contract {
  const contract = object_value(value, CLIENT_DEFAULTS_TRAIT_ID)
  const defaults = {
    max_in_flight: integer_member(
      contract,
      "maxInFlight",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    mutation_id_bytes: integer_member(
      contract,
      "mutationIdBytes",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    max_previous_data_protection_keys: integer_member(
      contract,
      "maxPreviousDataProtectionKeys",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    connect_timeout_milliseconds: integer_member(
      contract,
      "connectTimeoutMilliseconds",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    request_timeout_milliseconds: integer_member(
      contract,
      "requestTimeoutMilliseconds",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    retry_max_attempts: integer_member(
      contract,
      "retryMaxAttempts",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    zstandard_level: integer_member(
      contract,
      "zstandardLevel",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    zstandard_minimum_input_bytes: integer_member(
      contract,
      "zstandardMinimumInputBytes",
      CLIENT_DEFAULTS_TRAIT_ID,
      0,
    ),
    zstandard_minimum_savings_bytes: integer_member(
      contract,
      "zstandardMinimumSavingsBytes",
      CLIENT_DEFAULTS_TRAIT_ID,
      0,
    ),
    zstandard_level_min: integer_member(
      contract,
      "zstandardLevelMin",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    zstandard_level_max: integer_member(
      contract,
      "zstandardLevelMax",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
    server_name: string_member(contract, "serverName", CLIENT_DEFAULTS_TRAIT_ID),
    certificate_pem_type: string_member(
      contract,
      "certificatePemType",
      CLIENT_DEFAULTS_TRAIT_ID,
    ),
    minimum_positive_value: integer_member(
      contract,
      "minimumPositiveValue",
      CLIENT_DEFAULTS_TRAIT_ID,
      1,
    ),
  } satisfies Client_Defaults_Contract
  if (defaults.zstandard_level_min > defaults.zstandard_level_max) {
    throw new Error(
      `${CLIENT_DEFAULTS_TRAIT_ID}.zstandardLevelMin must not exceed zstandardLevelMax`,
    )
  }
  if (
    defaults.zstandard_level < defaults.zstandard_level_min ||
    defaults.zstandard_level > defaults.zstandard_level_max
  ) {
    throw new Error(
      `${CLIENT_DEFAULTS_TRAIT_ID}.zstandardLevel must be within the configured range`,
    )
  }
  return defaults
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

/** Extracts the client-owned Smithy API and native/value contracts.
 *
 * The input AST must contain both `protocol/model` and `clients/model`. Keeping
 * these models separate prevents server builds from depending on client defaults,
 * ABI discriminators, or application-level value-format details.
 */
export function extract_client_contract(ast: unknown): Client_Contract {
  const ast_object = object_value(ast, "Smithy AST")
  const shapes = object_member(ast_object, "shapes", "Smithy AST")
  const wire = extract_protocol_wire_contract(ast)
  const client_service_id = shapes[CLIENT_SERVICE_SHAPE_ID] === undefined
    ? SERVICE_SHAPE_ID
    : CLIENT_SERVICE_SHAPE_ID
  const client_namespace = client_service_id.slice(0, client_service_id.lastIndexOf("#"))
  const service = object_member(shapes, client_service_id, "Smithy AST.shapes")
  const location = `Smithy AST.shapes.${client_service_id}`
  const trait_ids = (trait_id: string): readonly string[] =>
    client_service_id === SERVICE_SHAPE_ID
      ? [trait_id, trait_id.replace("openkache.client#", "openkache.protocol#")]
      : [trait_id]
  const value_format_trait = trait_value_any(service, trait_ids(VALUE_FORMAT_TRAIT_ID), location)
  const client_defaults_trait = trait_value_any(service, trait_ids(CLIENT_DEFAULTS_TRAIT_ID), location)
  const ffi_trait = trait_value_any(service, trait_ids(FFI_CONTRACT_TRAIT_ID), location)
  const ffi_layout_trait = trait_value_any(service, trait_ids(FFI_LAYOUT_TRAIT_ID), location)
  const parsed_api = api_contract(shapes, client_service_id, client_namespace)
  const api = {
    ...parsed_api,
    // Smithy AST output is not required to preserve service-operation order.
    // Use the protocol assignments so every generated API presents operations
    // in the same stable order as the wire contract.
    operations: [...parsed_api.operations].sort(
      (left, right) =>
        (wire.opcodes.find((entry) => entry.name === left.name)?.value ?? 0) -
        (wire.opcodes.find((entry) => entry.name === right.name)?.value ?? 0),
    ),
  }
  const opcode_names = new Set(wire.opcodes.map((entry) => entry.name))
  for (const operation of api.operations) {
    if (!opcode_names.has(operation.name)) {
      throw new Error(
        `client operation ${operation.name} has no matching protocol opcode`,
      )
    }
  }
  const ffi = ffi_contract(ffi_trait)
  const client_defaults = client_defaults_contract(client_defaults_trait)
  const ffi_layout = ffi_layout_contract(
    ffi_layout_trait,
    client_defaults.mutation_id_bytes,
  )
  if (client_defaults.mutation_id_bytes !== wire.mutation_id_bytes) {
    throw new Error(
      `${CLIENT_DEFAULTS_TRAIT_ID}.mutationIdBytes must match the protocol wire contract`,
    )
  }
  const opcode_values = new Set(wire.opcodes.map((entry) => entry.value))
  for (const entry of ffi.operations) {
    if (opcode_values.has(entry.value)) {
      throw new Error(
        `FFI operation ${entry.name} wire value ${entry.value} overlaps a protocol opcode`,
      )
    }
  }
  return {
    ...wire,
    api,
    client_defaults,
    ffi,
    ffi_layout,
    value_format: value_format_contract(value_format_trait),
  }
}

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function formatted_byte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`
}

function c_unsigned_literal(value: number): string {
  if (value <= 9) return `${value}u`
  return `0x${value.toString(16)}u`
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

function c_string_literal(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let literal = '"'
  for (const byte of bytes) {
    if (byte >= 0x20 && byte <= 0x7e && byte !== 0x22 && byte !== 0x5c) {
      literal += String.fromCharCode(byte)
    } else if (byte === 0x22) {
      literal += '\\"'
    } else if (byte === 0x5c) {
      literal += "\\\\"
    } else {
      literal += `\\${byte.toString(8).padStart(3, "0")}`
    }
  }
  return `${literal}"`
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

function swift_ffi_value(
  entries: readonly Wire_Entry[],
  name: string,
  kind: string,
): number {
  const entry = entries.find((candidate) => candidate.name === name)
  if (entry === undefined) {
    throw new Error(`Smithy FFI ${kind} is missing ${name}`)
  }
  return entry.value
}

function rust_ffi_enum(
  name: string,
  documentation: string,
  member_documentation: string,
  entries: readonly Wire_Entry[],
): string {
  const variants = entries
    .map(
      (entry) =>
        `    /// ${member_documentation} identifier for ${entry.name}.
    ${entry.name} = ${formatted_decimal(entry.value)},`,
    )
    .join("\n")
  const try_from_arms = entries
    .map(
      (entry) =>
        `            value if value == Self::${entry.name}.code() => Ok(Self::${entry.name}),`,
    )
    .join("\n")
  const display_arms = entries
    .map(
      (entry) =>
        `            Self::${entry.name} => "${snake_case(entry.name)}",`,
    )
    .join("\n")
  return `/// ${documentation}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum ${name} {
${variants}
}

impl ${name} {
    /// Returns the Smithy-assigned native ABI discriminator.
    pub const fn code(self) -> u32 {
        self as u32
    }
}

impl core::convert::TryFrom<u32> for ${name} {
    type Error = u32;

    fn try_from(value: u32) -> core::result::Result<Self, u32> {
        match value {
${try_from_arms}
            _ => Err(value),
        }
    }
}

impl core::fmt::Display for ${name} {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
${display_arms}
        })
    }
}`
}

/** Renders the client-owned Rust defaults, ABI, and value-format declarations. */
export function render_rust_client(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  const layout = contract.ffi_layout
  const value_version_bytes = encode_vu128(value.version)
  const ffi = contract.ffi
  const ffi_operations = ffi.operations
    .map(
      (entry) =>
        `/// Native FFI operation identifier for ${entry.name}.
pub const FFI_OPERATION_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_result_kinds = ffi.result_kinds
    .map(
      (entry) =>
        `/// Native FFI result-kind identifier for ${entry.name}.
pub const FFI_RESULT_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_connection_states = ffi.connection_states
    .map(
      (entry) =>
        `/// Native FFI connection-state identifier for ${entry.name}.
pub const FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_set_conditions = ffi.set_conditions
    .map(
      (entry) =>
        `/// Native FFI SET-condition identifier for ${entry.name}.
pub const FFI_SET_CONDITION_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_error_codes = ffi.error_codes
    .map(
      (entry) =>
        `/// Native FFI structured-error code for ${entry.name}.
pub const FFI_ERROR_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_phases = ffi.phases
    .map(
      (entry) =>
        `/// Native FFI error phase identifier for ${entry.name}.
pub const FFI_PHASE_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_backends = ffi.backends
    .map(
      (entry) =>
        `/// Native FFI transport backend identifier for ${entry.name}.
pub const FFI_BACKEND_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_metrics = ffi.metrics
    .map(
      (entry) =>
        `/// Native FFI metrics field identifier for ${entry.name}.
pub const FFI_METRICS_${snake_case(entry.name).toUpperCase()}: u32 = ${formatted_decimal(entry.value)};`,
    )
    .join("\n")
  const ffi_operation_entries = [...contract.opcodes, ...ffi.operations].sort(
    (left, right) => left.value - right.value,
  )
  return `// Generated from the OpenKache client Smithy contract. Do not edit.

/// Default maximum number of concurrent request lanes.
pub const DEFAULT_MAX_IN_FLIGHT: usize = ${formatted_decimal(defaults.max_in_flight)};
/// Fixed width of a mutation idempotency token.
pub const MUTATION_ID_BYTES: usize = ${formatted_decimal(defaults.mutation_id_bytes)};
/// Maximum number of retired data-protection keys retained for rotation.
pub const MAX_PREVIOUS_DATA_PROTECTION_KEYS: usize = ${formatted_decimal(defaults.max_previous_data_protection_keys)};
/// Default connection-establishment timeout in milliseconds.
pub const DEFAULT_CONNECT_TIMEOUT_MILLISECONDS: u64 = ${formatted_decimal(defaults.connect_timeout_milliseconds)};
/// Default complete-request timeout in milliseconds.
pub const DEFAULT_REQUEST_TIMEOUT_MILLISECONDS: u64 = ${formatted_decimal(defaults.request_timeout_milliseconds)};
/// Default maximum total attempts for response-safe operations.
pub const DEFAULT_RETRY_MAX_ATTEMPTS: usize = ${formatted_decimal(defaults.retry_max_attempts)};
/// Default Zstandard compression level.
pub const DEFAULT_ZSTANDARD_LEVEL: i32 = ${formatted_decimal(defaults.zstandard_level)};
/// Default minimum serialized input size considered for Zstandard compression.
pub const DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES: usize = ${formatted_decimal(defaults.zstandard_minimum_input_bytes)};
/// Default minimum Zstandard savings required to retain compression.
pub const DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES: usize = ${formatted_decimal(defaults.zstandard_minimum_savings_bytes)};
/// Inclusive minimum supported Zstandard compression level.
pub const DEFAULT_ZSTANDARD_LEVEL_MIN: i32 = ${formatted_decimal(defaults.zstandard_level_min)};
/// Inclusive maximum supported Zstandard compression level.
pub const DEFAULT_ZSTANDARD_LEVEL_MAX: i32 = ${formatted_decimal(defaults.zstandard_level_max)};
/// Default TLS server name used when an adapter does not provide one.
pub const CLIENT_DEFAULT_SERVER_NAME: &str = ${rust_string_literal(defaults.server_name)};
/// PEM label used for adapter-assembled certificate chains.
pub const CLIENT_CERTIFICATE_PEM_TYPE: &str = ${rust_string_literal(defaults.certificate_pem_type)};
/// Minimum positive setting value when zero selects a default.
pub const CLIENT_MINIMUM_POSITIVE_VALUE: usize = ${formatted_decimal(defaults.minimum_positive_value)};

/// Version of the native client FFI contract.
pub const FFI_ABI_VERSION: u32 = ${formatted_decimal(ffi.abi_version)};
/// Size in bytes of the native FfiConnectOptions structure.
pub const FFI_CONNECT_OPTIONS_BYTES: usize = ${formatted_decimal(layout.connect_options_bytes)};
/// Size in bytes of the native FfiErrorMetadata structure.
pub const FFI_ERROR_METADATA_BYTES: usize = ${formatted_decimal(layout.error_metadata_bytes)};
/// Size in bytes of the native FfiMetricsSnapshot structure.
pub const FFI_METRICS_SNAPSHOT_BYTES: usize = ${formatted_decimal(layout.metrics_snapshot_bytes)};
${ffi_operations}
${ffi_result_kinds}
${ffi_connection_states}
${ffi_set_conditions}
${ffi_error_codes}
${ffi_phases}
${ffi_backends}
${ffi_metrics}

${rust_ffi_enum(
  "FfiOperation",
  "Native FFI operation identifiers shared by every language adapter.",
  "Native FFI operation",
  ffi_operation_entries,
)}

${rust_ffi_enum(
  "FfiResultKind",
  "Native FFI result-kind identifiers shared by every language adapter.",
  "Native FFI result-kind",
  ffi.result_kinds,
)}

${rust_ffi_enum(
  "ConnectionState",
  "Native FFI connection-state identifiers shared by every language adapter.",
  "Native FFI connection-state",
  ffi.connection_states,
)}

${rust_ffi_enum(
  "FfiSetCondition",
  "Native FFI SET-condition identifiers shared by every language adapter.",
  "Native FFI SET-condition",
  ffi.set_conditions,
)}

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

`
}

/** Renders the combined Rust wire and client contract for package generation. */
export function render_rust(contract: Client_Contract): string {
  return `${render_protocol_rust_wire(contract)}\n${render_rust_client(contract)}`
}

function c_contract_enum(
  name: string,
  entries: readonly Wire_Entry[],
  prefix: string,
): string {
  const variants = entries
    .map(
      (entry) =>
        `    ${prefix}_${snake_case(entry.name).toUpperCase()} = ${formatted_byte(entry.value)},`,
    )
    .join("\n")
  return `typedef enum ${name} {
${variants}
} ${name};`
}

function c_contract_api_enum(
  contract: Client_Contract,
  name: string,
  prefix: string,
): string {
  const enum_ = contract.api.enums.find((candidate) => candidate.name === name)
  if (enum_ === undefined) {
    throw new Error(`Smithy API enum ${name} is required by the C contract`)
  }
  return enum_.members
    .map(
      (member) =>
        `#define ${prefix}_${snake_case(member.name).toUpperCase()} ${c_string_literal(member.value)}`,
    )
    .join("\n")
}

/** Renders the Smithy constants consumed by native C and C++ adapters.
 *
 * @param contract - Validated language-neutral wire and value-format contract.
 * @returns Deterministic C declarations with a trailing newline.
 */
export function render_c_contract(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  const ffi = contract.ffi
  const layout = contract.ffi_layout
  const ffi_defines = [
    `#define OPENKACHE_SMITHY_FFI_ABI_VERSION ${c_unsigned_literal(ffi.abi_version)}`,
    ...ffi.error_codes.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_ERROR_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.phases.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_PHASE_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.backends.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_BACKEND_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.metrics.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_METRICS_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.operations.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_OPERATION_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.result_kinds.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_RESULT_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.connection_states.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
    ...ffi.set_conditions.map(
      (entry) =>
        `#define OPENKACHE_SMITHY_FFI_SET_CONDITION_${snake_case(entry.name).toUpperCase()} ${c_unsigned_literal(entry.value)}`,
    ),
  ].join("\n")
  const operation_enum = c_contract_enum(
    "openkache_smithy_opcode",
    contract.opcodes,
    "OPENKACHE_SMITHY_OPCODE",
  )
  const status_enum = c_contract_enum(
    "openkache_smithy_status",
    contract.statuses,
    "OPENKACHE_SMITHY_STATUS",
  )
  return `/* Generated from the OpenKache Smithy contract. Do not edit. */
#ifndef OPENKACHE_SMITHY_CONTRACT_H
#define OPENKACHE_SMITHY_CONTRACT_H

#include <stdint.h>

#define OPENKACHE_SMITHY_ITEM_ID_BYTES ${contract.item_id_bytes}u
#define OPENKACHE_SMITHY_MAX_VALUE_BYTES ${contract.max_value_bytes}u
#define OPENKACHE_SMITHY_ALPN ${c_string_literal(contract.v1.alpn)}
#define OPENKACHE_SMITHY_DEFAULT_MAX_IN_FLIGHT ${defaults.max_in_flight}u
#define OPENKACHE_SMITHY_MUTATION_ID_BYTES ${defaults.mutation_id_bytes}u
#define OPENKACHE_SMITHY_MAX_PREVIOUS_DATA_PROTECTION_KEYS ${defaults.max_previous_data_protection_keys}u
#define OPENKACHE_SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS ${defaults.connect_timeout_milliseconds}u
#define OPENKACHE_SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS ${defaults.request_timeout_milliseconds}u
#define OPENKACHE_SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS ${defaults.retry_max_attempts}u
#define OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_LEVEL ${defaults.zstandard_level}u
#define OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES ${defaults.zstandard_minimum_input_bytes}u
#define OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES ${defaults.zstandard_minimum_savings_bytes}u
#define OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN ${defaults.zstandard_level_min}u
#define OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX ${defaults.zstandard_level_max}u
#define OPENKACHE_SMITHY_CLIENT_DEFAULT_SERVER_NAME ${c_string_literal(defaults.server_name)}
#define OPENKACHE_SMITHY_CLIENT_CERTIFICATE_PEM_TYPE ${c_string_literal(defaults.certificate_pem_type)}
#define OPENKACHE_SMITHY_CLIENT_MINIMUM_POSITIVE_VALUE ${defaults.minimum_positive_value}u
#define OPENKACHE_SMITHY_FFI_CONNECT_OPTIONS_BYTES ${layout.connect_options_bytes}u
#define OPENKACHE_SMITHY_FFI_CONNECT_ADDRESS_OFFSET ${layout.connect_address_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_ADDRESS_LENGTH_OFFSET ${layout.connect_address_length_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_SERVER_NAME_OFFSET ${layout.connect_server_name_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_SERVER_NAME_LENGTH_OFFSET ${layout.connect_server_name_length_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_CERTIFICATE_OFFSET ${layout.connect_certificate_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_CERTIFICATE_LENGTH_OFFSET ${layout.connect_certificate_length_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_CLIENT_CERTIFICATE_CHAIN_OFFSET ${layout.connect_client_certificate_chain_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_CLIENT_CERTIFICATE_CHAIN_LENGTH_OFFSET ${layout.connect_client_certificate_chain_length_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_CLIENT_PRIVATE_KEY_OFFSET ${layout.connect_client_private_key_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_CLIENT_PRIVATE_KEY_LENGTH_OFFSET ${layout.connect_client_private_key_length_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_DATA_PROTECTION_KEY_OFFSET ${layout.connect_data_protection_key_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_DATA_PROTECTION_KEY_LENGTH_OFFSET ${layout.connect_data_protection_key_length_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEYS_OFFSET ${layout.connect_previous_data_protection_keys_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEYS_LENGTH_OFFSET ${layout.connect_previous_data_protection_keys_length_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEY_COUNT_OFFSET ${layout.connect_previous_data_protection_key_count_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_COMPRESSION_ENABLED_OFFSET ${layout.connect_compression_enabled_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_COMPRESSION_LEVEL_OFFSET ${layout.connect_compression_level_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_MINIMUM_INPUT_SIZE_OFFSET ${layout.connect_minimum_input_size_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_MINIMUM_SAVINGS_OFFSET ${layout.connect_minimum_savings_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_ENCRYPTION_OFFSET ${layout.connect_encryption_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_TIMEOUT_OFFSET ${layout.connect_timeout_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_REQUEST_TIMEOUT_OFFSET ${layout.connect_request_timeout_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_RETRY_MAX_ATTEMPTS_OFFSET ${layout.connect_retry_max_attempts_offset}u
#define OPENKACHE_SMITHY_FFI_CONNECT_MAX_IN_FLIGHT_OFFSET ${layout.connect_max_in_flight_offset}u
#define OPENKACHE_SMITHY_FFI_ERROR_METADATA_BYTES ${layout.error_metadata_bytes}u
#define OPENKACHE_SMITHY_FFI_ERROR_METADATA_CODE_OFFSET ${layout.error_metadata_code_offset}u
#define OPENKACHE_SMITHY_FFI_ERROR_METADATA_OPERATION_OFFSET ${layout.error_metadata_operation_offset}u
#define OPENKACHE_SMITHY_FFI_ERROR_METADATA_PHASE_OFFSET ${layout.error_metadata_phase_offset}u
#define OPENKACHE_SMITHY_FFI_ERROR_METADATA_BACKEND_OFFSET ${layout.error_metadata_backend_offset}u
#define OPENKACHE_SMITHY_FFI_ERROR_METADATA_RETRYABLE_OFFSET ${layout.error_metadata_retryable_offset}u
#define OPENKACHE_SMITHY_FFI_ERROR_METADATA_AMBIGUOUS_OFFSET ${layout.error_metadata_ambiguous_offset}u
#define OPENKACHE_SMITHY_FFI_ERROR_METADATA_MUTATION_ID_LENGTH_OFFSET ${layout.error_metadata_mutation_id_length_offset}u
#define OPENKACHE_SMITHY_FFI_ERROR_METADATA_MUTATION_ID_OFFSET ${layout.error_metadata_mutation_id_offset}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_BYTES ${layout.metrics_snapshot_bytes}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_REQUESTS_OFFSET ${layout.metrics_snapshot_requests_offset}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_HITS_OFFSET ${layout.metrics_snapshot_hits_offset}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_MISSES_OFFSET ${layout.metrics_snapshot_misses_offset}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_RETRIES_OFFSET ${layout.metrics_snapshot_retries_offset}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_RECONNECTS_OFFSET ${layout.metrics_snapshot_reconnects_offset}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_CANCELLATIONS_OFFSET ${layout.metrics_snapshot_cancellations_offset}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_TRANSPORT_ERRORS_OFFSET ${layout.metrics_snapshot_transport_errors_offset}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_PROTOCOL_ERRORS_OFFSET ${layout.metrics_snapshot_protocol_errors_offset}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_BYTES_SENT_OFFSET ${layout.metrics_snapshot_bytes_sent_offset}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_BYTES_RECEIVED_OFFSET ${layout.metrics_snapshot_bytes_received_offset}u
#define OPENKACHE_SMITHY_FFI_METRICS_SNAPSHOT_ACTIVE_LANES_OFFSET ${layout.metrics_snapshot_active_lanes_offset}u
${ffi_defines}
#define OPENKACHE_SMITHY_VALUE_FORMAT_VERSION ${value.version}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_MAX_VU128_BYTES ${value.max_vu128_bytes}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_FORMAT_BYTE_BYTES ${value.format_byte_bytes}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_COMPRESSION_MASK ${formatted_byte(value.format_compression_mask)}u
#define OPENKACHE_SMITHY_VALUE_FORMAT_ENCRYPTION_SHIFT ${formatted_byte(value.format_encryption_shift)}u
#define OPENKACHE_SMITHY_VALUE_SERIALIZATION_RAW ${formatted_byte(value.serialization_raw)}u
#define OPENKACHE_SMITHY_VALUE_SERIALIZATION_JSON ${formatted_byte(value.serialization_json)}u
#define OPENKACHE_SMITHY_VALUE_COMPRESSION_NONE ${formatted_byte(value.compression_none)}u
#define OPENKACHE_SMITHY_VALUE_COMPRESSION_ZSTANDARD ${formatted_byte(value.compression_zstandard)}u
#define OPENKACHE_SMITHY_VALUE_ENCRYPTION_NONE ${formatted_byte(value.encryption_none)}u
#define OPENKACHE_SMITHY_VALUE_ENCRYPTION_COMPACT ${formatted_byte(value.encryption_compact)}u
#define OPENKACHE_SMITHY_VALUE_ENCRYPTION_ROBUST ${formatted_byte(value.encryption_robust)}u
#define OPENKACHE_SMITHY_VALUE_COMPACT_SYNTHETIC_IV_BYTES ${value.compact_synthetic_iv_bytes}u
#define OPENKACHE_SMITHY_VALUE_ROBUST_NONCE_BYTES ${value.robust_nonce_bytes}u
#define OPENKACHE_SMITHY_VALUE_ROBUST_TAG_BYTES ${value.robust_tag_bytes}u
#define OPENKACHE_SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES ${value.data_protection_key_bytes}u
${operation_enum}

${status_enum}

/* Smithy string-enum values used by the language-neutral set API. */
${c_contract_api_enum(contract, "SetCondition", "OPENKACHE_SMITHY_SET_CONDITION")}
${c_contract_api_enum(contract, "SetOutcome", "OPENKACHE_SMITHY_SET_OUTCOME")}

#endif
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

function ffi_value(
  entries: readonly Wire_Entry[],
  name: string,
  category: string,
): number {
  const entry = entries.find((candidate) => candidate.name === name)
  if (entry === undefined) {
    throw new Error(`missing ${category} entry ${name}`)
  }
  return entry.value
}

/** Renders protocol v1 C# definitions.
 *
 * @param contract - Validated language-neutral wire contract.
 * @returns Deterministic C# source with a trailing newline.
 */
export function render_csharp(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  const ffi = contract.ffi
  const ffi_operation_reconnect = ffi_value(
    ffi.operations,
    "Reconnect",
    "FFI operation",
  )
  const ffi_operation_get_json = ffi_value(
    ffi.operations,
    "GetJson",
    "FFI operation",
  )
  const ffi_operation_set_json = ffi_value(
    ffi.operations,
    "SetJson",
    "FFI operation",
  )
  const ffi_result = (name: string): number =>
    ffi_value(ffi.result_kinds, name, "FFI result")
  const ffi_connection = (name: string): number =>
    ffi_value(ffi.connection_states, name, "FFI connection state")
  const ffi_set_condition = (name: string): number =>
    ffi_value(ffi.set_conditions, name, "FFI SET condition")
  const version_bytes = encode_vu128(value.version)
  return `// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

// Generated from the OpenKache Smithy contract. Do not edit.

namespace OpenKache;

internal static partial class Protocol
{
    internal const string ApplicationProtocol = ${JSON.stringify(contract.v1.alpn)};
    internal const int MaximumValueBytes = ${formatted_decimal(contract.max_value_bytes)};
    internal const int DefaultMaxInFlight = ${formatted_decimal(defaults.max_in_flight)};
    internal const int MutationIdBytes = ${formatted_decimal(defaults.mutation_id_bytes)};
    internal const int MaxPreviousDataProtectionKeys = ${formatted_decimal(defaults.max_previous_data_protection_keys)};
    internal const long DefaultConnectTimeoutMilliseconds = ${formatted_decimal(defaults.connect_timeout_milliseconds)};
    internal const long DefaultRequestTimeoutMilliseconds = ${formatted_decimal(defaults.request_timeout_milliseconds)};
    internal const int DefaultRetryMaxAttempts = ${formatted_decimal(defaults.retry_max_attempts)};
    internal const int DefaultZstandardLevel = ${formatted_decimal(defaults.zstandard_level)};
    internal const int DefaultZstandardMinimumInputBytes = ${formatted_decimal(defaults.zstandard_minimum_input_bytes)};
    internal const int DefaultZstandardMinimumSavingsBytes = ${formatted_decimal(defaults.zstandard_minimum_savings_bytes)};
    internal const int DefaultZstandardLevelMin = ${formatted_decimal(defaults.zstandard_level_min)};
    internal const int DefaultZstandardLevelMax = ${formatted_decimal(defaults.zstandard_level_max)};
    internal const string ClientCertificatePemType = ${JSON.stringify(defaults.certificate_pem_type)};
    internal const int ClientMinimumPositiveValue = ${formatted_decimal(defaults.minimum_positive_value)};
    internal const uint FfiAbiVersion = ${formatted_decimal(ffi.abi_version)}u;
    internal const uint FfiOperationReconnect = ${formatted_decimal(ffi_operation_reconnect)}u;
    internal const uint FfiOperationGetJson = ${formatted_decimal(ffi_operation_get_json)}u;
    internal const uint FfiOperationSetJson = ${formatted_decimal(ffi_operation_set_json)}u;
    internal const uint FfiResultError = ${formatted_decimal(ffi_result("Error"))}u;
    internal const uint FfiResultOk = ${formatted_decimal(ffi_result("Ok"))}u;
    internal const uint FfiResultValue = ${formatted_decimal(ffi_result("Value"))}u;
    internal const uint FfiResultNotFound = ${formatted_decimal(ffi_result("NotFound"))}u;
    internal const uint FfiResultCreated = ${formatted_decimal(ffi_result("Created"))}u;
    internal const uint FfiResultReplaced = ${formatted_decimal(ffi_result("Replaced"))}u;
    internal const uint FfiResultDeleted = ${formatted_decimal(ffi_result("Deleted"))}u;
    internal const uint FfiResultNotDeleted = ${formatted_decimal(ffi_result("NotDeleted"))}u;
    internal const uint FfiResultConnected = ${formatted_decimal(ffi_result("Connected"))}u;
    internal const uint FfiResultNotStored = ${formatted_decimal(ffi_result("NotStored"))}u;
    internal const uint FfiConnectionConnected = ${formatted_decimal(ffi_connection("Connected"))}u;
    internal const uint FfiConnectionReconnecting = ${formatted_decimal(ffi_connection("Reconnecting"))}u;
    internal const uint FfiConnectionDisconnected = ${formatted_decimal(ffi_connection("Disconnected"))}u;
    internal const uint FfiConnectionClosed = ${formatted_decimal(ffi_connection("Closed"))}u;
    internal const uint FfiConnectionUnknown = ${formatted_decimal(ffi_connection("Unknown"))}u;
    internal const uint FfiSetConditionNone = ${formatted_decimal(ffi_set_condition("None"))}u;
    internal const uint FfiSetConditionIfAbsent = ${formatted_decimal(ffi_set_condition("IfAbsent"))}u;
    internal const uint FfiSetConditionIfPresent = ${formatted_decimal(ffi_set_condition("IfPresent"))}u;
${ffi.error_codes
  .map(
    (entry) =>
      `    internal const uint FfiError${pascal_case(entry.name)} = ${formatted_decimal(entry.value)}u;`,
  )
  .join("\n")}
${ffi.phases
  .map(
    (entry) =>
      `    internal const uint FfiPhase${pascal_case(entry.name)} = ${formatted_decimal(entry.value)}u;`,
  )
  .join("\n")}
${ffi.backends
  .map(
    (entry) =>
      `    internal const uint FfiBackend${pascal_case(entry.name)} = ${formatted_decimal(entry.value)}u;`,
  )
  .join("\n")}
${ffi.metrics
  .map(
    (entry) =>
      `    internal const uint FfiMetrics${pascal_case(entry.name)} = ${formatted_decimal(entry.value)}u;`,
  )
  .join("\n")}

    private const int MaximumVarUIntBytes = ${formatted_decimal(contract.v1.max_varuint_bytes)};
    internal const int ItemIdBytes = ${formatted_decimal(contract.item_id_bytes)};
    private const byte SetTtlBit = ${formatted_byte(contract.v1.set_ttl_flag)};
    private const byte SetIfAbsentBit = ${formatted_byte(contract.v1.set_if_absent_flag)};
    private const byte SetIfPresentBit = ${formatted_byte(contract.v1.set_if_present_flag)};
    private const byte SetMutationIdBit = ${formatted_byte(contract.v1.set_mutation_id_flag)};

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
    internal static ReadOnlySpan<byte> ValueFormatVersionBytes =>
        [${version_bytes.map(formatted_byte).join(", ")}];

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
export function render_typescript_api(contract: Client_Contract): string {
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
  const constants = (prefix: string, entries: readonly Wire_Entry[]): string =>
    entries
      .map(
        (entry) =>
          `/** Smithy ${prefix.toLowerCase()} identifier for ${entry.name}. */
export const SMITHY_${prefix}_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
      )
      .join("\n")
  const opcode_constants = constants("OPCODE", contract.opcodes)
  const status_constants = constants("STATUS", contract.statuses)
  const ffi_operation_constants = constants("FFI_OPERATION", contract.ffi.operations)
  const ffi_result_constants = constants("FFI_RESULT", contract.ffi.result_kinds)
  const ffi_set_condition_constants = constants(
    "FFI_SET_CONDITION",
    contract.ffi.set_conditions,
  )
  const ffi_connection_state_constants = constants(
    "FFI_CONNECTION_STATE",
    contract.ffi.connection_states,
  )
  const ffi_error_constants = constants("FFI_ERROR", contract.ffi.error_codes)
  const ffi_phase_constants = constants("FFI_PHASE", contract.ffi.phases)
  const ffi_backend_constants = constants("FFI_BACKEND", contract.ffi.backends)
  const ffi_metrics_constants = constants("FFI_METRICS", contract.ffi.metrics)
  return `// Generated from the OpenKache Smithy contract. Do not edit.

/** Exact number of bytes in a protocol item identifier. */
export const SMITHY_ITEM_ID_BYTES = ${contract.item_id_bytes}
/** Maximum opaque value bytes accepted by the protocol. */
export const SMITHY_MAX_VALUE_BYTES = ${contract.max_value_bytes}
/** Default maximum number of concurrent request lanes. */
export const SMITHY_DEFAULT_MAX_IN_FLIGHT = ${contract.client_defaults.max_in_flight}
/** Fixed width of a mutation idempotency token. */
export const SMITHY_MUTATION_ID_BYTES = ${contract.client_defaults.mutation_id_bytes}
/** Maximum number of retired data-protection keys retained for rotation. */
export const SMITHY_MAX_PREVIOUS_DATA_PROTECTION_KEYS = ${contract.client_defaults.max_previous_data_protection_keys}
/** Default connection-establishment timeout in milliseconds. */
export const SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS = ${contract.client_defaults.connect_timeout_milliseconds}
/** Default complete-request timeout in milliseconds. */
export const SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS = ${contract.client_defaults.request_timeout_milliseconds}
/** Default maximum total attempts for response-safe operations. */
export const SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS = ${contract.client_defaults.retry_max_attempts}
/** Default Zstandard compression level. */
export const SMITHY_DEFAULT_ZSTANDARD_LEVEL = ${contract.client_defaults.zstandard_level}
/** Default minimum serialized input size considered for Zstandard compression. */
export const SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES = ${contract.client_defaults.zstandard_minimum_input_bytes}
/** Default minimum Zstandard savings required to retain compression. */
export const SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES = ${contract.client_defaults.zstandard_minimum_savings_bytes}
/** Inclusive minimum supported Zstandard compression level. */
export const SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN = ${contract.client_defaults.zstandard_level_min}
/** Inclusive maximum supported Zstandard compression level. */
export const SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX = ${contract.client_defaults.zstandard_level_max}
/** Default TLS server name used when no explicit name is supplied. */
export const SMITHY_CLIENT_DEFAULT_SERVER_NAME = ${JSON.stringify(contract.client_defaults.server_name)}
/** PEM label used for adapter-assembled certificate chains. */
export const SMITHY_CLIENT_CERTIFICATE_PEM_TYPE = ${JSON.stringify(contract.client_defaults.certificate_pem_type)}
/** Minimum positive setting value when zero selects a default. */
export const SMITHY_CLIENT_MINIMUM_POSITIVE_VALUE = ${contract.client_defaults.minimum_positive_value}
/** Version of the native client FFI contract. */
export const SMITHY_FFI_ABI_VERSION = ${contract.ffi.abi_version}
${opcode_constants}
${status_constants}
${ffi_operation_constants}
${ffi_result_constants}
${ffi_set_condition_constants}
${ffi_connection_state_constants}
${ffi_error_constants}
${ffi_phase_constants}
${ffi_backend_constants}
${ffi_metrics_constants}

${[...enums, ...structures].join("\n\n")}

/** Operations defined by the OpenKache Smithy service. */
export interface Smithy_OpenKache_Api {
${operations.join("\n")}
}
`
}

function go_api_name(identifier: string): string {
  return `Smithy${pascal_case(snake_case(identifier))}`
}

function go_exported_name(identifier: string): string {
  return pascal_case(snake_case(identifier))
    .replace(/Id$/, "ID")
    .replace(/^Ttl/, "TTL")
    .replace(/^Json$/, "JSON")
}

function go_ffi_name(identifier: string): string {
  const name = go_exported_name(identifier)
  return name === "Ok" ? "OK" : name
}

function go_api_value_name(enum_name: string, member_name: string): string {
  return `${go_api_name(enum_name)}${member_name}Value`
}

function go_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "[]byte"
      break
    case "boolean":
      rendered = "bool"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = go_api_name(type.name)
      break
    case "long":
      rendered = "int64"
      break
    case "string":
      rendered = "string"
      break
  }
  return required ? rendered : `*${rendered}`
}

/** Renders Smithy operation types and a context-aware Go service interface. */
export function render_go_api(contract: Client_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const members = enum_.members
      .map(
        (member) =>
          `\t${go_api_name(enum_.name)}${member.name} ${go_api_name(enum_.name)} = ${go_api_value_name(enum_.name, member.name)}`,
      )
      .join("\n")
    return `// ${go_api_name(enum_.name)} is the Smithy ${enum_.name} enum.
type ${go_api_name(enum_.name)} string

const (
${members}
)`
  })
  const structures = contract.api.structures.map((structure) => {
    const members = structure.members
      .map((member) => {
        const field = go_exported_name(member.name)
        const optional = member.required ? "" : ",omitempty"
        return `\t${field} ${go_api_type(member.type, member.required)} \`json:"${snake_case(member.name)}${optional}"\``
      })
      .join("\n")
    const body = members.length === 0 ? "" : `\n${members}\n`
    return `// ${go_api_name(structure.name)} is the Smithy ${structure.name} structure.
type ${go_api_name(structure.name)} struct {${body}}`
  })
  const operations = contract.api.operations.map(
    (operation) =>
      `\t${operation.name}(context.Context, ${go_api_name(operation.input)}) (${go_api_name(operation.output)}, error)`,
  )
  return `// Code generated from the OpenKache Smithy contract. DO NOT EDIT.

package openkache

import "context"

${[...enums, ...structures].join("\n\n")}

// SmithyOpenKacheAPI describes the operations defined by the OpenKache Smithy service.
type SmithyOpenKacheAPI interface {
${operations.join("\n")}
}
`
}

/** Renders generated wire, ABI, and client-default constants for Go. */
export function render_go_contract(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  return `// Code generated from the OpenKache Smithy contract. DO NOT EDIT.

package openkache

const (
\t// SmithyProtocolALPN is the negotiated protocol identifier.
\tSmithyProtocolALPN = ${JSON.stringify(contract.v1.alpn)}
\t// SmithyItemIDBytes is the exact protocol item-ID width.
\tSmithyItemIDBytes = ${contract.item_id_bytes}
\t// SmithyMaxValueBytes is the protocol value and payload ceiling.
\tSmithyMaxValueBytes = ${contract.max_value_bytes}
\t// SmithyDataProtectionKeyBytes is the shared key width.
\tSmithyDataProtectionKeyBytes = ${value.data_protection_key_bytes}
\t// SmithyValueEncryptionNone selects unprotected values.
\tSmithyValueEncryptionNone uint32 = ${value.encryption_none}
\t// SmithyValueEncryptionCompact selects deterministic AES-SIV protection.
\tSmithyValueEncryptionCompact uint32 = ${value.encryption_compact}
\t// SmithyValueEncryptionRobust selects randomized AES-GCM-SIV protection.
\tSmithyValueEncryptionRobust uint32 = ${value.encryption_robust}
)

// Smithy operation values carried by the native ABI.
const (
${contract.opcodes
  .map((entry) => `\tSmithyOpcode${entry.name} uint32 = ${entry.value}`)
  .join("\n")}
)

// Smithy native ABI values shared by language adapters.
const (
\t// SmithyFFIABIVersion is the native ABI version implemented by the core.
\tSmithyFFIABIVersion uint32 = ${contract.ffi.abi_version}
${contract.ffi.operations
  .map(
    (entry) =>
      `\t// SmithyFFIOperation${go_ffi_name(entry.name)} identifies the native operation ${entry.name}.
\tSmithyFFIOperation${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.result_kinds
  .map(
    (entry) =>
      `\t// SmithyFFIResult${go_ffi_name(entry.name)} is the native ABI result kind for ${entry.name}.
\tSmithyFFIResult${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.set_conditions
  .map(
    (entry) =>
      `\t// SmithyFFISetCondition${go_ffi_name(entry.name)} is the native ABI SET condition for ${entry.name}.
\tSmithyFFISetCondition${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.connection_states
  .map(
    (entry) =>
      `\t// SmithyFFIConnectionState${go_ffi_name(entry.name)} identifies a native connection state.
\tSmithyFFIConnectionState${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.error_codes
  .map(
    (entry) =>
      `\t// SmithyFFIError${go_ffi_name(entry.name)} identifies a structured native error code.
\tSmithyFFIError${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.phases
  .map(
    (entry) =>
      `\t// SmithyFFIPhase${go_ffi_name(entry.name)} identifies a structured native error phase.
\tSmithyFFIPhase${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.backends
  .map(
    (entry) =>
      `\t// SmithyFFIBackend${go_ffi_name(entry.name)} identifies a native transport backend.
\tSmithyFFIBackend${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.metrics
  .map(
    (entry) =>
      `\t// SmithyFFIMetrics${go_ffi_name(entry.name)} identifies a metrics snapshot field.
\tSmithyFFIMetrics${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
)

// Shared client defaults extracted from the Smithy service contract.
const (
\t// SmithyDefaultMaxInFlight is the default number of request lanes.
\tSmithyDefaultMaxInFlight = ${defaults.max_in_flight}
\t// SmithyMutationIDBytes is the fixed width of a mutation idempotency token.
\tSmithyMutationIDBytes = ${defaults.mutation_id_bytes}
\t// SmithyMaxPreviousDataProtectionKeys bounds the retired key read/delete window.
\tSmithyMaxPreviousDataProtectionKeys = ${defaults.max_previous_data_protection_keys}
\t// SmithyDefaultConnectTimeoutMilliseconds is the default connection timeout.
\tSmithyDefaultConnectTimeoutMilliseconds uint64 = ${defaults.connect_timeout_milliseconds}
\t// SmithyDefaultRequestTimeoutMilliseconds is the default complete request timeout.
\tSmithyDefaultRequestTimeoutMilliseconds uint64 = ${defaults.request_timeout_milliseconds}
\t// SmithyDefaultRetryMaxAttempts is the default total retry attempt count.
\tSmithyDefaultRetryMaxAttempts = ${defaults.retry_max_attempts}
\t// SmithyDefaultZstandardLevel is the default Zstandard level.
\tSmithyDefaultZstandardLevel int32 = ${defaults.zstandard_level}
\t// SmithyDefaultZstandardMinimumInputBytes is the compression input threshold.
\tSmithyDefaultZstandardMinimumInputBytes = ${defaults.zstandard_minimum_input_bytes}
\t// SmithyDefaultZstandardMinimumSavingsBytes is the compression savings threshold.
\tSmithyDefaultZstandardMinimumSavingsBytes = ${defaults.zstandard_minimum_savings_bytes}
\t// SmithyDefaultZstandardLevelMin is the minimum supported Zstandard level.
\tSmithyDefaultZstandardLevelMin int32 = ${defaults.zstandard_level_min}
\t// SmithyDefaultZstandardLevelMax is the maximum supported Zstandard level.
\tSmithyDefaultZstandardLevelMax int32 = ${defaults.zstandard_level_max}
\t// SmithyClientDefaultServerName is used when no TLS server name is supplied.
\tSmithyClientDefaultServerName = ${JSON.stringify(defaults.server_name)}
\t// SmithyClientCertificatePEMType is the PEM block type used for certificate chains.
\tSmithyClientCertificatePEMType = ${JSON.stringify(defaults.certificate_pem_type)}
\t// SmithyClientMinimumPositiveValue is the minimum accepted positive setting.
\tSmithyClientMinimumPositiveValue = ${defaults.minimum_positive_value}
)

// Smithy API enum string values extracted from the Smithy service contract.
const (
${contract.api.enums
  .flatMap((enum_) =>
    enum_.members.map(
      (member) =>
        `\t// ${go_api_value_name(enum_.name, member.name)} is the Smithy ${enum_.name} value for ${member.value}.
\t${go_api_value_name(enum_.name, member.name)} = ${JSON.stringify(member.value)}`,
    ),
  )
  .join("\n")}
)
`
}

function java_api_name(identifier: string): string {
  return pascal_case(snake_case(identifier))
}

function java_member_name(identifier: string): string {
  const parts = snake_case(identifier).split("_")
  return `${parts[0]}${parts.slice(1).map((part) => pascal_case(part)).join("")}`
}

function java_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "byte[]"
      break
    case "boolean":
      rendered = required ? "boolean" : "Boolean"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = `String`
      break
    case "long":
      rendered = required ? "long" : "Long"
      break
    case "string":
      rendered = "String"
      break
  }
  return rendered
}

function kotlin_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "ByteArray"
      break
    case "boolean":
      rendered = "Boolean"
      break
    case "enum":
      rendered = "String"
      break
    case "long":
      rendered = "Long"
      break
    case "string":
      rendered = "String"
      break
  }
  return required ? rendered : `${rendered}?`
}

function java_int_literal(value: number): string {
  if (value <= 0x7fff_ffff) return `${value}`
  return `0x${value.toString(16).padStart(8, "0")}`
}

/** Renders Java contract constants generated from the Smithy model. */
export function render_java_contract(contract: Client_Contract): string {
  const ffi = contract.ffi
  const layout = contract.ffi_layout
  const defaults = contract.client_defaults
  const value = contract.value_format
  const entries = (prefix: string, values: readonly Wire_Entry[]): string =>
    values
      .map(
        (entry) =>
          `    public static final int ${prefix}${pascal_case(entry.name)} = ${java_int_literal(entry.value)};`,
      )
      .join("\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated;

/** Stable wire, FFI, value-format, and client-default constants. */
public final class SmithyContract {
    private SmithyContract() {}

    public static final int ITEM_ID_BYTES = ${contract.item_id_bytes};
    public static final int MUTATION_ID_BYTES = ${contract.mutation_id_bytes};
    public static final int MAX_PREVIOUS_DATA_PROTECTION_KEYS = ${defaults.max_previous_data_protection_keys};
    public static final int MAX_VALUE_BYTES = ${contract.max_value_bytes};
    public static final String ALPN = ${JSON.stringify(contract.v1.alpn)};
    public static final String DEFAULT_SERVER_NAME = ${JSON.stringify(defaults.server_name)};
    public static final int DEFAULT_MAX_IN_FLIGHT = ${defaults.max_in_flight};
    public static final long DEFAULT_CONNECT_TIMEOUT_MILLISECONDS = ${defaults.connect_timeout_milliseconds}L;
    public static final long DEFAULT_REQUEST_TIMEOUT_MILLISECONDS = ${defaults.request_timeout_milliseconds}L;
    public static final int DEFAULT_RETRY_MAX_ATTEMPTS = ${defaults.retry_max_attempts};
    public static final int DEFAULT_ZSTANDARD_LEVEL = ${defaults.zstandard_level};
    public static final int DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES = ${defaults.zstandard_minimum_input_bytes};
    public static final int DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES = ${defaults.zstandard_minimum_savings_bytes};
    public static final int DEFAULT_ZSTANDARD_LEVEL_MIN = ${defaults.zstandard_level_min};
    public static final int DEFAULT_ZSTANDARD_LEVEL_MAX = ${defaults.zstandard_level_max};
    public static final String CLIENT_CERTIFICATE_PEM_TYPE = ${JSON.stringify(defaults.certificate_pem_type)};
    public static final int CLIENT_MINIMUM_POSITIVE_VALUE = ${defaults.minimum_positive_value};
    public static final int VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES = ${value.data_protection_key_bytes};
    public static final int VALUE_FORMAT_ENCRYPTION_NONE = ${value.encryption_none};
    public static final int VALUE_FORMAT_ENCRYPTION_COMPACT = ${value.encryption_compact};
    public static final int VALUE_FORMAT_ENCRYPTION_ROBUST = ${value.encryption_robust};
    public static final int FFI_ABI_VERSION = ${ffi.abi_version};
    public static final int FFI_CONNECT_OPTIONS_BYTES = ${layout.connect_options_bytes};
    public static final int FFI_CONNECT_ADDRESS_OFFSET = ${layout.connect_address_offset};
    public static final int FFI_CONNECT_ADDRESS_LENGTH_OFFSET = ${layout.connect_address_length_offset};
    public static final int FFI_CONNECT_SERVER_NAME_OFFSET = ${layout.connect_server_name_offset};
    public static final int FFI_CONNECT_SERVER_NAME_LENGTH_OFFSET = ${layout.connect_server_name_length_offset};
    public static final int FFI_CONNECT_CERTIFICATE_OFFSET = ${layout.connect_certificate_offset};
    public static final int FFI_CONNECT_CERTIFICATE_LENGTH_OFFSET = ${layout.connect_certificate_length_offset};
    public static final int FFI_CONNECT_CLIENT_CERTIFICATE_CHAIN_OFFSET = ${layout.connect_client_certificate_chain_offset};
    public static final int FFI_CONNECT_CLIENT_CERTIFICATE_CHAIN_LENGTH_OFFSET = ${layout.connect_client_certificate_chain_length_offset};
    public static final int FFI_CONNECT_CLIENT_PRIVATE_KEY_OFFSET = ${layout.connect_client_private_key_offset};
    public static final int FFI_CONNECT_CLIENT_PRIVATE_KEY_LENGTH_OFFSET = ${layout.connect_client_private_key_length_offset};
    public static final int FFI_CONNECT_DATA_PROTECTION_KEY_OFFSET = ${layout.connect_data_protection_key_offset};
    public static final int FFI_CONNECT_DATA_PROTECTION_KEY_LENGTH_OFFSET = ${layout.connect_data_protection_key_length_offset};
    public static final int FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEYS_OFFSET = ${layout.connect_previous_data_protection_keys_offset};
    public static final int FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEYS_LENGTH_OFFSET = ${layout.connect_previous_data_protection_keys_length_offset};
    public static final int FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEY_COUNT_OFFSET = ${layout.connect_previous_data_protection_key_count_offset};
    public static final int FFI_CONNECT_COMPRESSION_ENABLED_OFFSET = ${layout.connect_compression_enabled_offset};
    public static final int FFI_CONNECT_COMPRESSION_LEVEL_OFFSET = ${layout.connect_compression_level_offset};
    public static final int FFI_CONNECT_MINIMUM_INPUT_SIZE_OFFSET = ${layout.connect_minimum_input_size_offset};
    public static final int FFI_CONNECT_MINIMUM_SAVINGS_OFFSET = ${layout.connect_minimum_savings_offset};
    public static final int FFI_CONNECT_ENCRYPTION_OFFSET = ${layout.connect_encryption_offset};
    public static final int FFI_CONNECT_TIMEOUT_OFFSET = ${layout.connect_timeout_offset};
    public static final int FFI_CONNECT_REQUEST_TIMEOUT_OFFSET = ${layout.connect_request_timeout_offset};
    public static final int FFI_CONNECT_RETRY_MAX_ATTEMPTS_OFFSET = ${layout.connect_retry_max_attempts_offset};
    public static final int FFI_CONNECT_MAX_IN_FLIGHT_OFFSET = ${layout.connect_max_in_flight_offset};
    public static final int FFI_ERROR_METADATA_BYTES = ${layout.error_metadata_bytes};
    public static final int FFI_ERROR_METADATA_CODE_OFFSET = ${layout.error_metadata_code_offset};
    public static final int FFI_ERROR_METADATA_OPERATION_OFFSET = ${layout.error_metadata_operation_offset};
    public static final int FFI_ERROR_METADATA_PHASE_OFFSET = ${layout.error_metadata_phase_offset};
    public static final int FFI_ERROR_METADATA_BACKEND_OFFSET = ${layout.error_metadata_backend_offset};
    public static final int FFI_ERROR_METADATA_RETRYABLE_OFFSET = ${layout.error_metadata_retryable_offset};
    public static final int FFI_ERROR_METADATA_AMBIGUOUS_OFFSET = ${layout.error_metadata_ambiguous_offset};
    public static final int FFI_ERROR_METADATA_MUTATION_ID_LENGTH_OFFSET = ${layout.error_metadata_mutation_id_length_offset};
    public static final int FFI_ERROR_METADATA_MUTATION_ID_OFFSET = ${layout.error_metadata_mutation_id_offset};
    public static final int FFI_METRICS_SNAPSHOT_BYTES = ${layout.metrics_snapshot_bytes};
    public static final int FFI_METRICS_SNAPSHOT_REQUESTS_OFFSET = ${layout.metrics_snapshot_requests_offset};
    public static final int FFI_METRICS_SNAPSHOT_HITS_OFFSET = ${layout.metrics_snapshot_hits_offset};
    public static final int FFI_METRICS_SNAPSHOT_MISSES_OFFSET = ${layout.metrics_snapshot_misses_offset};
    public static final int FFI_METRICS_SNAPSHOT_RETRIES_OFFSET = ${layout.metrics_snapshot_retries_offset};
    public static final int FFI_METRICS_SNAPSHOT_RECONNECTS_OFFSET = ${layout.metrics_snapshot_reconnects_offset};
    public static final int FFI_METRICS_SNAPSHOT_CANCELLATIONS_OFFSET = ${layout.metrics_snapshot_cancellations_offset};
    public static final int FFI_METRICS_SNAPSHOT_TRANSPORT_ERRORS_OFFSET = ${layout.metrics_snapshot_transport_errors_offset};
    public static final int FFI_METRICS_SNAPSHOT_PROTOCOL_ERRORS_OFFSET = ${layout.metrics_snapshot_protocol_errors_offset};
    public static final int FFI_METRICS_SNAPSHOT_BYTES_SENT_OFFSET = ${layout.metrics_snapshot_bytes_sent_offset};
    public static final int FFI_METRICS_SNAPSHOT_BYTES_RECEIVED_OFFSET = ${layout.metrics_snapshot_bytes_received_offset};
    public static final int FFI_METRICS_SNAPSHOT_ACTIVE_LANES_OFFSET = ${layout.metrics_snapshot_active_lanes_offset};

${entries("OPCODE_", contract.opcodes)}

${entries("FFI_OPERATION_", ffi.operations)}

${entries("FFI_RESULT_", ffi.result_kinds)}

${entries("FFI_SET_CONDITION_", ffi.set_conditions)}

${entries("FFI_CONNECTION_STATE_", ffi.connection_states)}

${entries("FFI_ERROR_", ffi.error_codes)}

${entries("FFI_PHASE_", ffi.phases)}

${entries("FFI_BACKEND_", ffi.backends)}

${entries("FFI_METRICS_", ffi.metrics)}
}
`
}

/** Renders Java Smithy operation records and a CompletionStage API interface. */
export function render_java_api(contract: Client_Contract): string {
  const enums = contract.api.enums
    .map(
      (enum_) =>
        `    public enum ${java_api_name(enum_.name)} {
${enum_.members.map((member) => `        ${member.name}(${JSON.stringify(member.value)})`).join(",\n")};
        public final String value;
        ${java_api_name(enum_.name)}(String value) { this.value = value; }
    }`,
    )
    .join("\n\n")
  const structures = contract.api.structures
    .map((structure) => {
      const members = structure.members
        .map(
          (member) =>
            `        ${java_api_type(member.type, member.required)} ${java_member_name(member.name)}`,
        )
        .join(",\n")
      return `    public record ${java_api_name(structure.name)}(
${members}
    ) {}`
    })
    .join("\n\n")
  const operations = contract.api.operations
    .map(
      (operation) =>
        `        java.util.concurrent.CompletionStage<${java_api_name(operation.output)}> ${java_member_name(operation.name)}(${java_api_name(operation.input)} input);`,
    )
    .join("\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated;

/** Smithy operation types for Java adapters. */
public final class SmithyApi {
    private SmithyApi() {}

${enums}

${structures}

    public interface OpenKacheApi {
${operations}
    }
}
`
}

/** Renders Kotlin Smithy operation types and stable contract constants. */
export function render_kotlin_contract(contract: Client_Contract): string {
  const ffi = contract.ffi
  const layout = contract.ffi_layout
  const defaults = contract.client_defaults
  const value = contract.value_format
  const entries = (prefix: string, values: readonly Wire_Entry[]): string =>
    values
      .map(
        (entry) =>
          `    const val ${prefix}${pascal_case(entry.name)}: Int = ${java_int_literal(entry.value)}`,
      )
      .join("\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated

/** Stable wire, FFI, value-format, and client-default constants. */
object SmithyContract {
    const val ITEM_ID_BYTES: Int = ${contract.item_id_bytes}
    const val MUTATION_ID_BYTES: Int = ${contract.mutation_id_bytes}
    const val MAX_PREVIOUS_DATA_PROTECTION_KEYS: Int = ${defaults.max_previous_data_protection_keys}
    const val MAX_VALUE_BYTES: Int = ${contract.max_value_bytes}
    const val ALPN: String = ${JSON.stringify(contract.v1.alpn)}
    const val DEFAULT_SERVER_NAME: String = ${JSON.stringify(defaults.server_name)}
    const val DEFAULT_MAX_IN_FLIGHT: Int = ${defaults.max_in_flight}
    const val DEFAULT_CONNECT_TIMEOUT_MILLISECONDS: Long = ${defaults.connect_timeout_milliseconds}L
    const val DEFAULT_REQUEST_TIMEOUT_MILLISECONDS: Long = ${defaults.request_timeout_milliseconds}L
    const val DEFAULT_RETRY_MAX_ATTEMPTS: Int = ${defaults.retry_max_attempts}
    const val DEFAULT_ZSTANDARD_LEVEL: Int = ${defaults.zstandard_level}
    const val DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES: Int = ${defaults.zstandard_minimum_input_bytes}
    const val DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES: Int = ${defaults.zstandard_minimum_savings_bytes}
    const val DEFAULT_ZSTANDARD_LEVEL_MIN: Int = ${defaults.zstandard_level_min}
    const val DEFAULT_ZSTANDARD_LEVEL_MAX: Int = ${defaults.zstandard_level_max}
    const val CLIENT_CERTIFICATE_PEM_TYPE: String = ${JSON.stringify(defaults.certificate_pem_type)}
    const val CLIENT_MINIMUM_POSITIVE_VALUE: Int = ${defaults.minimum_positive_value}
    const val VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES: Int = ${value.data_protection_key_bytes}
    const val VALUE_FORMAT_ENCRYPTION_NONE: Int = ${value.encryption_none}
    const val VALUE_FORMAT_ENCRYPTION_COMPACT: Int = ${value.encryption_compact}
    const val VALUE_FORMAT_ENCRYPTION_ROBUST: Int = ${value.encryption_robust}
    const val FFI_ABI_VERSION: Int = ${ffi.abi_version}
    const val FFI_CONNECT_OPTIONS_BYTES: Int = ${layout.connect_options_bytes}
    const val FFI_CONNECT_ADDRESS_OFFSET: Int = ${layout.connect_address_offset}
    const val FFI_CONNECT_ADDRESS_LENGTH_OFFSET: Int = ${layout.connect_address_length_offset}
    const val FFI_CONNECT_SERVER_NAME_OFFSET: Int = ${layout.connect_server_name_offset}
    const val FFI_CONNECT_SERVER_NAME_LENGTH_OFFSET: Int = ${layout.connect_server_name_length_offset}
    const val FFI_CONNECT_CERTIFICATE_OFFSET: Int = ${layout.connect_certificate_offset}
    const val FFI_CONNECT_CERTIFICATE_LENGTH_OFFSET: Int = ${layout.connect_certificate_length_offset}
    const val FFI_CONNECT_CLIENT_CERTIFICATE_CHAIN_OFFSET: Int = ${layout.connect_client_certificate_chain_offset}
    const val FFI_CONNECT_CLIENT_CERTIFICATE_CHAIN_LENGTH_OFFSET: Int = ${layout.connect_client_certificate_chain_length_offset}
    const val FFI_CONNECT_CLIENT_PRIVATE_KEY_OFFSET: Int = ${layout.connect_client_private_key_offset}
    const val FFI_CONNECT_CLIENT_PRIVATE_KEY_LENGTH_OFFSET: Int = ${layout.connect_client_private_key_length_offset}
    const val FFI_CONNECT_DATA_PROTECTION_KEY_OFFSET: Int = ${layout.connect_data_protection_key_offset}
    const val FFI_CONNECT_DATA_PROTECTION_KEY_LENGTH_OFFSET: Int = ${layout.connect_data_protection_key_length_offset}
    const val FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEYS_OFFSET: Int = ${layout.connect_previous_data_protection_keys_offset}
    const val FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEYS_LENGTH_OFFSET: Int = ${layout.connect_previous_data_protection_keys_length_offset}
    const val FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEY_COUNT_OFFSET: Int = ${layout.connect_previous_data_protection_key_count_offset}
    const val FFI_CONNECT_COMPRESSION_ENABLED_OFFSET: Int = ${layout.connect_compression_enabled_offset}
    const val FFI_CONNECT_COMPRESSION_LEVEL_OFFSET: Int = ${layout.connect_compression_level_offset}
    const val FFI_CONNECT_MINIMUM_INPUT_SIZE_OFFSET: Int = ${layout.connect_minimum_input_size_offset}
    const val FFI_CONNECT_MINIMUM_SAVINGS_OFFSET: Int = ${layout.connect_minimum_savings_offset}
    const val FFI_CONNECT_ENCRYPTION_OFFSET: Int = ${layout.connect_encryption_offset}
    const val FFI_CONNECT_TIMEOUT_OFFSET: Int = ${layout.connect_timeout_offset}
    const val FFI_CONNECT_REQUEST_TIMEOUT_OFFSET: Int = ${layout.connect_request_timeout_offset}
    const val FFI_CONNECT_RETRY_MAX_ATTEMPTS_OFFSET: Int = ${layout.connect_retry_max_attempts_offset}
    const val FFI_CONNECT_MAX_IN_FLIGHT_OFFSET: Int = ${layout.connect_max_in_flight_offset}
    const val FFI_ERROR_METADATA_BYTES: Int = ${layout.error_metadata_bytes}
    const val FFI_ERROR_METADATA_CODE_OFFSET: Int = ${layout.error_metadata_code_offset}
    const val FFI_ERROR_METADATA_OPERATION_OFFSET: Int = ${layout.error_metadata_operation_offset}
    const val FFI_ERROR_METADATA_PHASE_OFFSET: Int = ${layout.error_metadata_phase_offset}
    const val FFI_ERROR_METADATA_BACKEND_OFFSET: Int = ${layout.error_metadata_backend_offset}
    const val FFI_ERROR_METADATA_RETRYABLE_OFFSET: Int = ${layout.error_metadata_retryable_offset}
    const val FFI_ERROR_METADATA_AMBIGUOUS_OFFSET: Int = ${layout.error_metadata_ambiguous_offset}
    const val FFI_ERROR_METADATA_MUTATION_ID_LENGTH_OFFSET: Int = ${layout.error_metadata_mutation_id_length_offset}
    const val FFI_ERROR_METADATA_MUTATION_ID_OFFSET: Int = ${layout.error_metadata_mutation_id_offset}
    const val FFI_METRICS_SNAPSHOT_BYTES: Int = ${layout.metrics_snapshot_bytes}
    const val FFI_METRICS_SNAPSHOT_REQUESTS_OFFSET: Int = ${layout.metrics_snapshot_requests_offset}
    const val FFI_METRICS_SNAPSHOT_HITS_OFFSET: Int = ${layout.metrics_snapshot_hits_offset}
    const val FFI_METRICS_SNAPSHOT_MISSES_OFFSET: Int = ${layout.metrics_snapshot_misses_offset}
    const val FFI_METRICS_SNAPSHOT_RETRIES_OFFSET: Int = ${layout.metrics_snapshot_retries_offset}
    const val FFI_METRICS_SNAPSHOT_RECONNECTS_OFFSET: Int = ${layout.metrics_snapshot_reconnects_offset}
    const val FFI_METRICS_SNAPSHOT_CANCELLATIONS_OFFSET: Int = ${layout.metrics_snapshot_cancellations_offset}
    const val FFI_METRICS_SNAPSHOT_TRANSPORT_ERRORS_OFFSET: Int = ${layout.metrics_snapshot_transport_errors_offset}
    const val FFI_METRICS_SNAPSHOT_PROTOCOL_ERRORS_OFFSET: Int = ${layout.metrics_snapshot_protocol_errors_offset}
    const val FFI_METRICS_SNAPSHOT_BYTES_SENT_OFFSET: Int = ${layout.metrics_snapshot_bytes_sent_offset}
    const val FFI_METRICS_SNAPSHOT_BYTES_RECEIVED_OFFSET: Int = ${layout.metrics_snapshot_bytes_received_offset}
    const val FFI_METRICS_SNAPSHOT_ACTIVE_LANES_OFFSET: Int = ${layout.metrics_snapshot_active_lanes_offset}

${entries("OPCODE_", contract.opcodes)}

${entries("FFI_OPERATION_", ffi.operations)}

${entries("FFI_RESULT_", ffi.result_kinds)}

${entries("FFI_SET_CONDITION_", ffi.set_conditions)}

${entries("FFI_CONNECTION_STATE_", ffi.connection_states)}

${entries("FFI_ERROR_", ffi.error_codes)}

${entries("FFI_PHASE_", ffi.phases)}

${entries("FFI_BACKEND_", ffi.backends)}

${entries("FFI_METRICS_", ffi.metrics)}
}
`
}

/** Renders Kotlin Smithy operation records and a suspend API interface. */
export function render_kotlin_api(contract: Client_Contract): string {
  const enums = contract.api.enums
    .map(
      (enum_) =>
        `    enum class ${java_api_name(enum_.name)}(val value: String) {
${enum_.members.map((member) => `        ${member.name}(${JSON.stringify(member.value)})`).join(",\n")}
    }`,
    )
    .join("\n\n")
  const structures = contract.api.structures
    .map((structure) => {
      const members = structure.members
        .map(
          (member) =>
            `        val ${java_member_name(member.name)}: ${kotlin_api_type(member.type, member.required)}`,
        )
        .join(",\n")
      if (members.length === 0) return `    class ${java_api_name(structure.name)}`
      return `    data class ${java_api_name(structure.name)}(
${members}
    )`
    })
    .join("\n\n")
  const operations = contract.api.operations
    .map(
      (operation) =>
        `        suspend fun ${java_member_name(operation.name)}(input: ${java_api_name(operation.input)}): ${java_api_name(operation.output)}`,
    )
    .join("\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated

/** Smithy operation types for Kotlin adapters. */
object SmithyApi {
${enums}

${structures}

    interface OpenKacheApi {
${operations}
    }
}
`
}

/** Renders Dart Smithy contract constants consumed by the FFI facade. */
export function render_dart_contract(contract: Client_Contract): string {
  const ffi = contract.ffi
  const defaults = contract.client_defaults
  const entries = (prefix: string, values: readonly Wire_Entry[]): string =>
    values
      .map(
        (entry) =>
          `const int ${prefix}_${snake_case(entry.name)} = ${entry.value};`,
      )
      .join("\n")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
library;

const int smithyItemIdBytes = ${contract.item_id_bytes};
const int smithyMutationIdBytes = ${contract.mutation_id_bytes};
const int smithyMaxPreviousDataProtectionKeys = ${defaults.max_previous_data_protection_keys};
const int smithyMaxValueBytes = ${contract.max_value_bytes};
const int smithyValueDataProtectionKeyBytes = ${contract.value_format.data_protection_key_bytes};
const int smithyValueEncryptionNone = ${contract.value_format.encryption_none};
const int smithyValueEncryptionCompact = ${contract.value_format.encryption_compact};
const int smithyValueEncryptionRobust = ${contract.value_format.encryption_robust};
const String smithyProtocolAlpn = ${JSON.stringify(contract.v1.alpn)};
const int smithyDefaultMaxInFlight = ${defaults.max_in_flight};
const String smithyDefaultServerName = ${JSON.stringify(defaults.server_name)};
const int smithyDefaultConnectTimeoutMilliseconds = ${defaults.connect_timeout_milliseconds};
const int smithyDefaultRequestTimeoutMilliseconds = ${defaults.request_timeout_milliseconds};
const int smithyDefaultRetryMaxAttempts = ${defaults.retry_max_attempts};
const int smithyDefaultZstandardLevel = ${defaults.zstandard_level};
const int smithyDefaultZstandardMinimumInputBytes = ${defaults.zstandard_minimum_input_bytes};
const int smithyDefaultZstandardMinimumSavingsBytes = ${defaults.zstandard_minimum_savings_bytes};
const int smithyDefaultZstandardLevelMin = ${defaults.zstandard_level_min};
const int smithyDefaultZstandardLevelMax = ${defaults.zstandard_level_max};
const String smithyClientCertificatePemType = ${JSON.stringify(defaults.certificate_pem_type)};
const int smithyClientMinimumPositiveValue = ${defaults.minimum_positive_value};
const int smithyFfiAbiVersion = ${ffi.abi_version};

${entries("smithy_opcode", contract.opcodes)}

${entries("smithy_ffi_operation", ffi.operations)}

${entries("smithy_ffi_result", ffi.result_kinds)}

${entries("smithy_ffi_set_condition", ffi.set_conditions)}

${entries("smithy_ffi_connection_state", ffi.connection_states)}

${entries("smithy_ffi_error", ffi.error_codes)}

${entries("smithy_ffi_phase", ffi.phases)}

${entries("smithy_ffi_backend", ffi.backends)}

${entries("smithy_ffi_metrics", ffi.metrics)}
`
}

function format_go_source(source: string): string {
  const result = Bun.spawnSync({
    cmd: ["gofmt"],
    stdin: Buffer.from(source),
    stdout: "pipe",
    stderr: "pipe",
  })
  if (result.exitCode !== 0) {
    const diagnostics = result.stderr.toString().trim()
    throw new Error(
      diagnostics.length === 0
        ? "gofmt failed while formatting generated Go source"
        : `gofmt failed while formatting generated Go source:\n${diagnostics}`,
    )
  }
  return result.stdout.toString()
}

function python_api_name(identifier: string): string {
  return `Smithy${pascal_case(snake_case(identifier))}`
}

function python_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "bytes"
      break
    case "boolean":
      rendered = "bool"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = python_api_name(type.name)
      break
    case "long":
      rendered = "int"
      break
    case "string":
      rendered = "str"
      break
  }
  return required ? rendered : `${rendered} | None`
}

/** Renders Smithy operation types and a Python async protocol interface.
 *
 * @param contract - Validated language-neutral wire and API contract.
 * @returns Deterministic Python source with a trailing newline.
 */
export function render_python_api(contract: Client_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const members = enum_.members
      .map(
        (member) =>
          `    ${snake_case(member.name).toUpperCase()} = ${JSON.stringify(member.value)}`,
      )
      .join("\n")
    return `class ${python_api_name(enum_.name)}(str, Enum):
    """Values defined by the Smithy ${enum_.name} shape."""

${members}`
  })
  const structures = contract.api.structures.map((structure) => {
    // Dataclasses require non-default fields before default fields. Smithy
    // member order is not a source-level guarantee, so keep required members
    // first while preserving each group's model order.
    const ordered_members = [...structure.members].sort(
      (left, right) => Number(!left.required) - Number(!right.required),
    )
    const members = ordered_members.map((member) => {
      const default_value = member.required ? "" : " = None"
      return `    ${snake_case(member.name)}: ${python_api_type(member.type, member.required)}${default_value}`
    })
    const body = members.length === 0 ? "    pass" : members.join("\n")
    return `@dataclass(frozen=True, slots=True)
class ${python_api_name(structure.name)}:
    """Smithy ${structure.name} structure."""

${body}`
  })
  const operations = contract.api.operations
    .map(
      (operation) =>
        `    async def ${snake_case(operation.name)}(
        self, input: ${python_api_name(operation.input)}
    ) -> ${python_api_name(operation.output)}: ...`,
    )
    .join("\n")
  return `# Generated from the OpenKache Smithy contract. Do not edit.

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Protocol

${[...enums, ...structures].join("\n\n")}


class SmithyOpenKacheApi(Protocol):
    """Async operations defined by the OpenKache Smithy service."""

${operations}
`
}

/** Renders the Python constants shared with the core-backed adapter.
 *
 * @param contract - Validated language-neutral wire and value-format contract.
 * @returns Deterministic Python source with a trailing newline.
 */
export function render_python_contract(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  const version_bytes = encode_vu128(value.version)
  const ffi_operations = contract.ffi.operations
    .map(
      (entry) =>
        `SMITHY_FFI_OPERATION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_result_kinds = contract.ffi.result_kinds
    .map(
      (entry) =>
        `SMITHY_FFI_RESULT_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_connection_states = contract.ffi.connection_states
    .map(
      (entry) =>
        `SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_set_conditions = contract.ffi.set_conditions
    .map(
      (entry) =>
        `SMITHY_FFI_SET_CONDITION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_error_codes = contract.ffi.error_codes
    .map(
      (entry) =>
        `SMITHY_FFI_ERROR_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_phases = contract.ffi.phases
    .map(
      (entry) =>
        `SMITHY_FFI_PHASE_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_backends = contract.ffi.backends
    .map(
      (entry) =>
        `SMITHY_FFI_BACKEND_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_metrics = contract.ffi.metrics
    .map(
      (entry) =>
        `SMITHY_FFI_METRICS_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const opcodes = contract.opcodes
    .map((entry) => `SMITHY_OPCODE_${snake_case(entry.name).toUpperCase()} = ${entry.value}`)
    .join("\n")
  const statuses = contract.statuses
    .map((entry) => `SMITHY_STATUS_${snake_case(entry.name).toUpperCase()} = ${entry.value}`)
    .join("\n")
  return `# Generated from the OpenKache Smithy contract. Do not edit.

SMITHY_PROTOCOL_ALPN = ${JSON.stringify(contract.v1.alpn)}
SMITHY_REQUEST_FIXED_BYTES = ${contract.v1.request_fixed_bytes}
SMITHY_RESPONSE_FIXED_BYTES = ${contract.v1.response_fixed_bytes}
SMITHY_MAX_VARUINT_BYTES = ${contract.v1.max_varuint_bytes}
SMITHY_ITEM_ID_BYTES = ${contract.item_id_bytes}
SMITHY_MAX_VALUE_BYTES = ${contract.max_value_bytes}
SMITHY_DEFAULT_MAX_IN_FLIGHT = ${defaults.max_in_flight}
SMITHY_MUTATION_ID_BYTES = ${defaults.mutation_id_bytes}
SMITHY_MAX_PREVIOUS_DATA_PROTECTION_KEYS = ${defaults.max_previous_data_protection_keys}
SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS = ${defaults.connect_timeout_milliseconds}
SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS = ${defaults.request_timeout_milliseconds}
SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS = ${defaults.retry_max_attempts}
SMITHY_DEFAULT_ZSTANDARD_LEVEL = ${defaults.zstandard_level}
SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES = ${defaults.zstandard_minimum_input_bytes}
SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES = ${defaults.zstandard_minimum_savings_bytes}
SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN = ${defaults.zstandard_level_min}
SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX = ${defaults.zstandard_level_max}
SMITHY_CLIENT_DEFAULT_SERVER_NAME = ${JSON.stringify(defaults.server_name)}
SMITHY_CLIENT_CERTIFICATE_PEM_TYPE = ${JSON.stringify(defaults.certificate_pem_type)}
SMITHY_CLIENT_MINIMUM_POSITIVE_VALUE = ${defaults.minimum_positive_value}
SMITHY_FFI_ABI_VERSION = ${contract.ffi.abi_version}
${ffi_operations}
${ffi_result_kinds}
${ffi_connection_states}
${ffi_set_conditions}
${ffi_error_codes}
${ffi_phases}
${ffi_backends}
${ffi_metrics}
SMITHY_SET_TTL_FLAG = ${contract.v1.set_ttl_flag}
SMITHY_SET_IF_ABSENT_FLAG = ${contract.v1.set_if_absent_flag}
SMITHY_SET_IF_PRESENT_FLAG = ${contract.v1.set_if_present_flag}
SMITHY_VALUE_FORMAT_VERSION = ${value.version}
SMITHY_VALUE_FORMAT_VERSION_BYTES = bytes([${version_bytes.join(", ")}])
SMITHY_VALUE_FORMAT_MAX_VU128_BYTES = ${value.max_vu128_bytes}
SMITHY_VALUE_FORMAT_FORMAT_BYTE_BYTES = ${value.format_byte_bytes}
SMITHY_VALUE_FORMAT_COMPRESSION_MASK = ${value.format_compression_mask}
SMITHY_VALUE_FORMAT_ENCRYPTION_SHIFT = ${value.format_encryption_shift}
SMITHY_VALUE_SERIALIZATION_RAW = ${value.serialization_raw}
SMITHY_VALUE_SERIALIZATION_JSON = ${value.serialization_json}
SMITHY_VALUE_COMPRESSION_NONE = ${value.compression_none}
SMITHY_VALUE_COMPRESSION_ZSTANDARD = ${value.compression_zstandard}
SMITHY_VALUE_ENCRYPTION_NONE = ${value.encryption_none}
SMITHY_VALUE_ENCRYPTION_COMPACT = ${value.encryption_compact}
SMITHY_VALUE_ENCRYPTION_ROBUST = ${value.encryption_robust}
SMITHY_VALUE_COMPACT_SYNTHETIC_IV_BYTES = ${value.compact_synthetic_iv_bytes}
SMITHY_VALUE_ROBUST_NONCE_BYTES = ${value.robust_nonce_bytes}
SMITHY_VALUE_ROBUST_TAG_BYTES = ${value.robust_tag_bytes}
SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES = ${value.data_protection_key_bytes}
SMITHY_VALUE_ITEM_ID_ROOT_CONTEXT = ${JSON.stringify(value.item_id_root_context)}
SMITHY_VALUE_AAD_DOMAIN = ${JSON.stringify(value.aad_domain)}
SMITHY_VALUE_VALUE_ROOT_CONTEXT = ${JSON.stringify(value.value_root_context)}
SMITHY_VALUE_COMPACT_MAC_CONTEXT = ${JSON.stringify(value.compact_mac_context)}
SMITHY_VALUE_COMPACT_ENCRYPTION_CONTEXT = ${JSON.stringify(value.compact_encryption_context)}
SMITHY_VALUE_ROBUST_CONTEXT = ${JSON.stringify(value.robust_context)}
${opcodes}
${statuses}
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

/** Renders Smithy operation and shared contract declarations for Swift.
 *
 * @param contract - Validated language-neutral wire, value, and FFI contract.
 * @returns Deterministic Swift source with a trailing newline.
 */
export function render_swift_api(contract: Client_Contract): string {
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
  const ffi = contract.ffi
  const layout = contract.ffi_layout
  const version_bytes = encode_vu128(value.version)
  const operation_get_json = swift_ffi_value(ffi.operations, "GetJson", "operation")
  const operation_set_json = swift_ffi_value(ffi.operations, "SetJson", "operation")
  const operation_reconnect = swift_ffi_value(ffi.operations, "Reconnect", "operation")
  const result_error = swift_ffi_value(ffi.result_kinds, "Error", "result")
  const result_ok = swift_ffi_value(ffi.result_kinds, "Ok", "result")
  const result_value = swift_ffi_value(ffi.result_kinds, "Value", "result")
  const result_not_found = swift_ffi_value(ffi.result_kinds, "NotFound", "result")
  const result_created = swift_ffi_value(ffi.result_kinds, "Created", "result")
  const result_replaced = swift_ffi_value(ffi.result_kinds, "Replaced", "result")
  const result_deleted = swift_ffi_value(ffi.result_kinds, "Deleted", "result")
  const result_not_deleted = swift_ffi_value(ffi.result_kinds, "NotDeleted", "result")
  const result_connected = swift_ffi_value(ffi.result_kinds, "Connected", "result")
  const result_not_stored = swift_ffi_value(ffi.result_kinds, "NotStored", "result")
  const set_condition_none = swift_ffi_value(ffi.set_conditions, "None", "SET condition")
  const set_condition_if_absent = swift_ffi_value(
    ffi.set_conditions,
    "IfAbsent",
    "SET condition",
  )
  const set_condition_if_present = swift_ffi_value(
    ffi.set_conditions,
    "IfPresent",
    "SET condition",
  )
  const connection_states = ffi.connection_states
    .map(
      (entry) =>
        `  case ${swift_property_name(entry.name)} = ${entry.value}`,
    )
    .join("\n")
  const error_codes = ffi.error_codes
    .map(
      (entry) =>
        `  public static let error${typescript_name(entry.name)}: UInt32 = ${entry.value}`,
    )
    .join("\n")
  const phases = ffi.phases
    .map(
      (entry) =>
        `  public static let phase${typescript_name(entry.name)}: UInt32 = ${entry.value}`,
    )
    .join("\n")
  const backends = ffi.backends
    .map(
      (entry) =>
        `  public static let backend${typescript_name(entry.name)}: UInt32 = ${entry.value}`,
    )
    .join("\n")
  const metrics = ffi.metrics
    .map(
      (entry) =>
        `  public static let metrics${typescript_name(entry.name)}: UInt32 = ${entry.value}`,
    )
    .join("\n")
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
  public static let protocolAlpn: String = ${swift_string_literal(contract.v1.alpn)}
  public static let itemIdBytes: Int = ${contract.item_id_bytes}
  public static let maxValueBytes: Int = ${contract.max_value_bytes}
  public static let defaultMaxInFlight: Int = ${contract.client_defaults.max_in_flight}
  public static let mutationIdBytes: Int = ${contract.client_defaults.mutation_id_bytes}
  public static let maxPreviousDataProtectionKeys: Int = ${contract.client_defaults.max_previous_data_protection_keys}
  public static let defaultConnectTimeoutMilliseconds: Int = ${contract.client_defaults.connect_timeout_milliseconds}
  public static let defaultRequestTimeoutMilliseconds: Int = ${contract.client_defaults.request_timeout_milliseconds}
  public static let defaultRetryMaxAttempts: Int = ${contract.client_defaults.retry_max_attempts}
  public static let defaultZstandardLevel: Int32 = ${contract.client_defaults.zstandard_level}
  public static let defaultZstandardMinimumInputBytes: Int = ${contract.client_defaults.zstandard_minimum_input_bytes}
  public static let defaultZstandardMinimumSavingsBytes: Int = ${contract.client_defaults.zstandard_minimum_savings_bytes}
  public static let defaultZstandardLevelMin: Int32 = ${contract.client_defaults.zstandard_level_min}
  public static let defaultZstandardLevelMax: Int32 = ${contract.client_defaults.zstandard_level_max}
  public static let defaultServerName: String = ${swift_string_literal(contract.client_defaults.server_name)}
  public static let certificatePemType: String = ${swift_string_literal(contract.client_defaults.certificate_pem_type)}
  public static let minimumPositiveValue: Int = ${contract.client_defaults.minimum_positive_value}
  public static let version: Int = ${value.version}
  public static let versionBytes: [UInt8] = [${version_bytes.join(", ")}]
  public static let maxVu128Bytes: Int = ${value.max_vu128_bytes}
  public static let formatByteBytes: Int = ${value.format_byte_bytes}
  public static let maxVaruintBytes: Int = ${contract.v1.max_varuint_bytes}
  public static let setTtlFlag: UInt8 = ${contract.v1.set_ttl_flag}
  public static let setIfAbsentFlag: UInt8 = ${contract.v1.set_if_absent_flag}
  public static let setIfPresentFlag: UInt8 = ${contract.v1.set_if_present_flag}
  public static let setMutationIdFlag: UInt8 = ${contract.v1.set_mutation_id_flag}
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

/// Native ABI connection-state identifiers shared by every language adapter.
public enum Smithy_Connection_State: UInt32, Equatable, Sendable {
${connection_states}
}

/// Native ABI identifiers shared by every language adapter.
public enum Smithy_Native_Contract: Sendable {
  public static let abiVersion: UInt32 = ${ffi.abi_version}
  public static let connectOptionsBytes: Int = ${layout.connect_options_bytes}
  public static let connectAddressOffset: Int = ${layout.connect_address_offset}
  public static let connectAddressLengthOffset: Int = ${layout.connect_address_length_offset}
  public static let connectServerNameOffset: Int = ${layout.connect_server_name_offset}
  public static let connectServerNameLengthOffset: Int = ${layout.connect_server_name_length_offset}
  public static let connectCertificateOffset: Int = ${layout.connect_certificate_offset}
  public static let connectCertificateLengthOffset: Int = ${layout.connect_certificate_length_offset}
  public static let connectClientCertificateChainOffset: Int = ${layout.connect_client_certificate_chain_offset}
  public static let connectClientCertificateChainLengthOffset: Int = ${layout.connect_client_certificate_chain_length_offset}
  public static let connectClientPrivateKeyOffset: Int = ${layout.connect_client_private_key_offset}
  public static let connectClientPrivateKeyLengthOffset: Int = ${layout.connect_client_private_key_length_offset}
  public static let connectDataProtectionKeyOffset: Int = ${layout.connect_data_protection_key_offset}
  public static let connectDataProtectionKeyLengthOffset: Int = ${layout.connect_data_protection_key_length_offset}
  public static let connectPreviousDataProtectionKeysOffset: Int = ${layout.connect_previous_data_protection_keys_offset}
  public static let connectPreviousDataProtectionKeysLengthOffset: Int = ${layout.connect_previous_data_protection_keys_length_offset}
  public static let connectPreviousDataProtectionKeyCountOffset: Int = ${layout.connect_previous_data_protection_key_count_offset}
  public static let connectCompressionEnabledOffset: Int = ${layout.connect_compression_enabled_offset}
  public static let connectCompressionLevelOffset: Int = ${layout.connect_compression_level_offset}
  public static let connectMinimumInputSizeOffset: Int = ${layout.connect_minimum_input_size_offset}
  public static let connectMinimumSavingsOffset: Int = ${layout.connect_minimum_savings_offset}
  public static let connectEncryptionOffset: Int = ${layout.connect_encryption_offset}
  public static let connectTimeoutOffset: Int = ${layout.connect_timeout_offset}
  public static let connectRequestTimeoutOffset: Int = ${layout.connect_request_timeout_offset}
  public static let connectRetryMaxAttemptsOffset: Int = ${layout.connect_retry_max_attempts_offset}
  public static let connectMaxInFlightOffset: Int = ${layout.connect_max_in_flight_offset}
  public static let errorMetadataBytes: Int = ${layout.error_metadata_bytes}
  public static let errorMetadataCodeOffset: Int = ${layout.error_metadata_code_offset}
  public static let errorMetadataOperationOffset: Int = ${layout.error_metadata_operation_offset}
  public static let errorMetadataPhaseOffset: Int = ${layout.error_metadata_phase_offset}
  public static let errorMetadataBackendOffset: Int = ${layout.error_metadata_backend_offset}
  public static let errorMetadataRetryableOffset: Int = ${layout.error_metadata_retryable_offset}
  public static let errorMetadataAmbiguousOffset: Int = ${layout.error_metadata_ambiguous_offset}
  public static let errorMetadataMutationIdLengthOffset: Int = ${layout.error_metadata_mutation_id_length_offset}
  public static let errorMetadataMutationIdOffset: Int = ${layout.error_metadata_mutation_id_offset}
  public static let metricsSnapshotBytes: Int = ${layout.metrics_snapshot_bytes}
  public static let metricsSnapshotRequestsOffset: Int = ${layout.metrics_snapshot_requests_offset}
  public static let metricsSnapshotHitsOffset: Int = ${layout.metrics_snapshot_hits_offset}
  public static let metricsSnapshotMissesOffset: Int = ${layout.metrics_snapshot_misses_offset}
  public static let metricsSnapshotRetriesOffset: Int = ${layout.metrics_snapshot_retries_offset}
  public static let metricsSnapshotReconnectsOffset: Int = ${layout.metrics_snapshot_reconnects_offset}
  public static let metricsSnapshotCancellationsOffset: Int = ${layout.metrics_snapshot_cancellations_offset}
  public static let metricsSnapshotTransportErrorsOffset: Int = ${layout.metrics_snapshot_transport_errors_offset}
  public static let metricsSnapshotProtocolErrorsOffset: Int = ${layout.metrics_snapshot_protocol_errors_offset}
  public static let metricsSnapshotBytesSentOffset: Int = ${layout.metrics_snapshot_bytes_sent_offset}
  public static let metricsSnapshotBytesReceivedOffset: Int = ${layout.metrics_snapshot_bytes_received_offset}
  public static let metricsSnapshotActiveLanesOffset: Int = ${layout.metrics_snapshot_active_lanes_offset}
  public static let operationGetJson: UInt32 = ${operation_get_json}
  public static let operationSetJson: UInt32 = ${operation_set_json}
  public static let operationReconnect: UInt32 = ${operation_reconnect}
  public static let resultError: UInt32 = ${result_error}
  public static let resultOk: UInt32 = ${result_ok}
  public static let resultValue: UInt32 = ${result_value}
  public static let resultNotFound: UInt32 = ${result_not_found}
  public static let resultCreated: UInt32 = ${result_created}
  public static let resultReplaced: UInt32 = ${result_replaced}
  public static let resultDeleted: UInt32 = ${result_deleted}
  public static let resultNotDeleted: UInt32 = ${result_not_deleted}
  public static let resultConnected: UInt32 = ${result_connected}
  public static let resultNotStored: UInt32 = ${result_not_stored}
  public static let setConditionNone: UInt32 = ${set_condition_none}
  public static let setConditionIfAbsent: UInt32 = ${set_condition_if_absent}
  public static let setConditionIfPresent: UInt32 = ${set_condition_if_present}
${error_codes}
${phases}
${backends}
${metrics}
}
`
}

/** Renders the cross-language value-format wire and cryptographic contract for TypeScript.
 *
 * @param contract - Validated language-neutral wire and value-format contract.
 * @returns Deterministic TypeScript source with a trailing newline.
 */
export function render_typescript_value_format(contract: Client_Contract): string {
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
export function render_csharp_api(contract: Client_Contract): string {
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
export function render_rust_api(contract: Client_Contract): string {
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
    >;`,
  )
  return `// Generated from the OpenKache Smithy contract. Do not edit.

${[...enums, ...structures].join("\n\n")}

/// Operations defined by the OpenKache Smithy service.
///
/// The trait does not require Send futures because the Rust client exposes
/// both Tokio/Quinn and runtime-local Compio implementations. Callers that
/// need cross-thread scheduling can add the bound to the concrete client.
pub trait OpenKacheApi {
    /// Error returned by an operation.
    type Error;

${operations.join("\n\n")}
}
`
}

function smithy_ast(client_model: boolean): unknown {
  const cwd = client_model ? CLIENTS_DIRECTORY : PROTOCOL_DIRECTORY
  const models = client_model
    ? [join("..", "protocol", MODEL_DIRECTORY), MODEL_DIRECTORY]
    : [MODEL_DIRECTORY]
  const result = Bun.spawnSync(["smithy", "ast", ...models], {
    cwd,
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
  | "c-contract"
  | "dotnet"
  | "dart"
  | "go"
  | "java"
  | "kotlin"
  | "python"
  | "rust-api"
  | "rust-client"
  | "rust-wire"
  | "swift"
  | "typescript"

function generation_target(value: string | undefined): Generation_Target {
  switch (value) {
    case undefined:
      return "all"
    case "all":
      return "all"
    case "c-contract":
      return "c-contract"
    case "dotnet":
      return "dotnet"
    case "dart":
      return "dart"
    case "go":
      return "go"
    case "java":
      return "java"
    case "kotlin":
      return "kotlin"
    case "python":
      return "python"
    case "rust-api":
      return "rust-api"
    case "rust-client":
      return "rust-client"
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

function expected_wire_outputs(
  contract: Wire_Contract,
  target: "rust-wire",
): Readonly<Record<string, string>> {
  if (target !== "rust-wire") {
    throw new Error(`unsupported wire generation target ${target}`)
  }
  return {
    [GENERATED_OUTPUTS.rust_wire]: render_protocol_rust_wire(contract),
  }
}

function expected_outputs(
  contract: Client_Contract,
  target: Generation_Target,
): Readonly<Record<string, string>> {
  switch (target) {
    case "all":
      return {
        [GENERATED_OUTPUTS.csharp_api]: render_csharp_api(contract),
        [GENERATED_OUTPUTS.csharp_wire]: render_csharp(contract),
        [GENERATED_OUTPUTS.rust_client]: render_rust_client(contract),
        [GENERATED_OUTPUTS.rust_api]: render_rust_api(contract),
        [GENERATED_OUTPUTS.rust_wire]: render_protocol_rust_wire(contract),
        [GENERATED_OUTPUTS.typescript_api]: render_typescript_api(contract),
        [GENERATED_OUTPUTS.typescript_value_format]:
          render_typescript_value_format(contract),
        [GENERATED_OUTPUTS.python_api]: render_python_api(contract),
        [GENERATED_OUTPUTS.python_contract]: render_python_contract(contract),
        [GENERATED_OUTPUTS.swift_api]: render_swift_api(contract),
        [GENERATED_OUTPUTS.c_contract]: render_c_contract(contract),
        [GENERATED_OUTPUTS.go_api]: format_go_source(render_go_api(contract)),
        [GENERATED_OUTPUTS.go_contract]: format_go_source(render_go_contract(contract)),
        [GENERATED_OUTPUTS.java_api]: render_java_api(contract),
        [GENERATED_OUTPUTS.java_contract]: render_java_contract(contract),
        [GENERATED_OUTPUTS.kotlin_api]: render_kotlin_api(contract),
        [GENERATED_OUTPUTS.kotlin_contract]: render_kotlin_contract(contract),
        [GENERATED_OUTPUTS.dart_contract]: render_dart_contract(contract),
      }
    case "c-contract":
      return {
        [GENERATED_OUTPUTS.c_contract]: render_c_contract(contract),
      }
    case "dotnet":
      return {
        [GENERATED_OUTPUTS.csharp_api]: render_csharp_api(contract),
        [GENERATED_OUTPUTS.csharp_wire]: render_csharp(contract),
      }
    case "dart":
      return {
        [GENERATED_OUTPUTS.dart_contract]: render_dart_contract(contract),
      }
    case "go":
      return {
        [GENERATED_OUTPUTS.go_api]: format_go_source(render_go_api(contract)),
        [GENERATED_OUTPUTS.go_contract]: format_go_source(render_go_contract(contract)),
      }
    case "java":
      return {
        [GENERATED_OUTPUTS.java_api]: render_java_api(contract),
        [GENERATED_OUTPUTS.java_contract]: render_java_contract(contract),
      }
    case "kotlin":
      return {
        [GENERATED_OUTPUTS.kotlin_api]: render_kotlin_api(contract),
        [GENERATED_OUTPUTS.kotlin_contract]: render_kotlin_contract(contract),
      }
    case "rust-api":
      return {
        [GENERATED_OUTPUTS.rust_api]: render_rust_api(contract),
      }
    case "rust-client":
      return {
        [GENERATED_OUTPUTS.rust_client]: render_rust_client(contract),
      }
    case "rust-wire":
      return {
        [GENERATED_OUTPUTS.rust_wire]: render_protocol_rust_wire(contract),
      }
    case "typescript":
      return {
        [GENERATED_OUTPUTS.typescript_api]: render_typescript_api(contract),
        [GENERATED_OUTPUTS.typescript_value_format]:
          render_typescript_value_format(contract),
      }
    case "python":
      return {
        [GENERATED_OUTPUTS.python_api]: render_python_api(contract),
        [GENERATED_OUTPUTS.python_contract]: render_python_contract(contract),
      }
    case "swift":
      return {
        [GENERATED_OUTPUTS.swift_api]: render_swift_api(contract),
      }
  }
}

/** Returns generated outputs that are missing or differ from the contract. */
export function generated_output_issues(
  outputs: Readonly<Record<string, string>>,
): readonly string[] {
  const mismatches: string[] = []
  for (const [output_path, content] of Object.entries(outputs)) {
    let existing: string
    try {
      existing = readFileSync(output_path, "utf8")
    } catch {
      mismatches.push(`${output_path} (missing)`)
      continue
    }
    if (existing !== content) mismatches.push(output_path)
  }
  return mismatches
}

function write_outputs(
  outputs: Readonly<Record<string, string>>,
  check_only: boolean,
): void {
  if (check_only) {
    const mismatches = generated_output_issues(outputs)
    if (mismatches.length > 0) {
      throw new Error(
        "generated contract outputs are stale:\n" +
          mismatches.map((output_path) => `  - ${output_path}`).join("\n") +
          "\nRun `just generate-protocol-contract` to regenerate them.",
      )
    }
    return
  }
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
    const target = generation_target(process.env.OPENKACHE_GENERATION_TARGET)
    const outputs =
      target === "rust-wire"
        ? expected_wire_outputs(extract_protocol_wire_contract(smithy_ast(false)), target)
        : expected_outputs(extract_client_contract(smithy_ast(true)), target)
    write_outputs(outputs, process.env.OPENKACHE_GENERATION_CHECK === "1")
    return 0
  } catch (error) {
    console.error(
      `GENERATION_FAILED: ${error instanceof Error ? error.message : String(error)}\n` +
        "  Why: client language and ABI values can only be generated from valid, complete wire and client Smithy contracts.\n" +
        "  Fix: Run `smithy validate model` for the protocol and client models, correct the reported model or generator error, then rerun `./generate.ts` from the clients directory.",
    )
    return 1
  }
}

if (import.meta.main) process.exit(main())
