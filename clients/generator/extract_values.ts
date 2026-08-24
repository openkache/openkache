//! Value-format, envelope, and client-default contract extraction.

import {
  CLIENT_DEFAULTS_TRAIT_ID,
  VALUE_ENVELOPE_TRAIT_ID,
  VALUE_FORMAT_TRAIT_ID,
} from "./config"
import { integer_member, object_value, string_member } from "./extract_ast"
import { unique_wire_values } from "./extract_ffi"
import type {
  Client_Defaults_Contract,
  Value_Envelope_Contract,
  Value_Format_Contract,
} from "./model"
import { encode_vu128 } from "./utils"

function gate0_integer_member(
  object: Readonly<Record<string, unknown>>,
  member: string,
  minimum: number,
  maximum = Number.MAX_SAFE_INTEGER,
): number {
  return integer_member(object, member, CLIENT_DEFAULTS_TRAIT_ID, minimum, maximum)
}

export function value_format_contract(value: unknown): Value_Format_Contract {
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
    serialization_structured: integer_member(
      contract,
      "serializationStructured",
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
      { name: "Structured", value: values.serialization_structured },
    ],
    "serialization",
  )
  if (values.serialization_structured !== 1) {
    throw new Error(
      `${VALUE_FORMAT_TRAIT_ID}.serializationStructured must be selector 1`,
    )
  }
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

export function value_envelope_contract(value: unknown): Value_Envelope_Contract {
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

export function client_defaults_contract(value: unknown): Client_Defaults_Contract {
  const contract = object_value(value, CLIENT_DEFAULTS_TRAIT_ID)
  const gate0_item_id_root_key_hex = string_member(
    contract,
    "gate0ItemIdRootKeyHex",
    CLIENT_DEFAULTS_TRAIT_ID,
  )
  if (
    gate0_item_id_root_key_hex.length !== 64 ||
    !/^[0-9a-f]{64}$/i.test(gate0_item_id_root_key_hex)
  ) {
    throw new Error(
      `${CLIENT_DEFAULTS_TRAIT_ID}.gate0ItemIdRootKeyHex must contain exactly 32 bytes of hexadecimal digits`,
    )
  }
  const defaults = {
    max_in_flight: integer_member(
      contract,
      "maxInFlight",
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
    gate0_alpn_version: gate0_integer_member(
      contract,
      "gate0AlpnVersion",
      1,
      0xff,
    ),
    gate0_compression: gate0_integer_member(
      contract,
      "gate0Compression",
      0,
      0xff,
    ),
    gate0_encryption: gate0_integer_member(
      contract,
      "gate0Encryption",
      0,
      0xff,
    ),
    gate0_item_id_root_key_hex,
    gate0_namespace_id: gate0_integer_member(
      contract,
      "gate0NamespaceId",
      1,
    ),
    gate0_value_selector: gate0_integer_member(
      contract,
      "gate0ValueSelector",
      0,
      0xff,
    ),
  } satisfies Client_Defaults_Contract
  if (!/^[0-9a-f]{64}$/i.test(defaults.gate0_item_id_root_key_hex)) {
    throw new Error(
      `${CLIENT_DEFAULTS_TRAIT_ID}.gate0ItemIdRootKeyHex must contain exactly 32 hexadecimal bytes`,
    )
  }
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

export function valid_encoding_identifier(encoding: string, maximum_bytes: number): boolean {
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
