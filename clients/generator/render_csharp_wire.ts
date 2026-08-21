//! .NET wire contract rendering.

import type { Wire_Entry } from "../../protocol/wire"
import type { Client_Contract, Ffi_Entry } from "./model"
import {
  bytes_from_hex,
  encode_vu128,
  formatted_byte,
  formatted_decimal,
  pascal_case,
  snake_case,
} from "./utils"

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
export function render_csharp(contract: Client_Contract): string {
  const value = contract.value_format
  const defaults = contract.client_defaults
  const ffi = contract.ffi
  const descriptor_layout = ffi.namespace_descriptor_layout
  const descriptor_fields = ffi.namespace_descriptor_fields
  const version_bytes = encode_vu128(value.version)
  const envelope = contract.value_envelope
  const envelope_magic = bytes_from_hex(
    envelope.magic_and_version_hex,
    "value envelope magic",
  )
  const csharp_namespace_descriptor_fields = descriptor_fields.map(
    (field) =>
      `        internal ${field.csharp_type} ${field.csharp_name};`,
  ).join("\n")
  const csharp_descriptor_offsets = descriptor_fields
    .map(
      (field) =>
        `    internal const int FfiNamespaceDescriptor${pascal_case(field.name)}Offset = ${formatted_decimal(field.offset)};`,
    )
    .join("\n")
  const csharp_ffi_constants = [
    ["FfiOperation", ffi.operations],
    ["FfiResult", ffi.result_kinds],
    ["FfiConnection", ffi.connection_states],
    ["FfiTransport", ffi.transports],
    ["FfiSetCondition", ffi.set_conditions],
    ["FfiNamespaceDescriptorDecode", ffi.namespace_descriptor_decode_statuses],
    ["FfiNamespaceDefaultExpiration", ffi.namespace_default_expirations],
    ["FfiNamespaceDefaultEviction", ffi.namespace_default_evictions],
    ["FfiNamespaceOverride", ffi.namespace_override_policies],
  ]
    .flatMap(([prefix, entries]) =>
      (entries as readonly Ffi_Entry[]).map(
        (entry) =>
          `    internal const uint ${prefix}${pascal_case(snake_case(entry.name))} = ${formatted_decimal(entry.value)}u;`,
      ),
    )
    .join("\n")
  const csharp_api_enum_constants = contract.api.enums
    .flatMap((enum_) =>
      enum_.members.map(
        (member) =>
          `    internal const string Smithy${enum_.name}${member.name}Value = ${JSON.stringify(member.value)};`,
      ),
    )
    .join("\n")
  return `// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

// Generated from the OpenKache Smithy contract. Do not edit.

namespace OpenKache;

internal static partial class Protocol
{
    [System.Runtime.InteropServices.StructLayout(
        System.Runtime.InteropServices.LayoutKind.Sequential)]
    internal struct FfiNamespaceDescriptor
    {
${csharp_namespace_descriptor_fields}
    }

    internal const string ApplicationProtocol = ${JSON.stringify(contract.v1.alpn)};
    internal const int MaximumValueBytes = ${formatted_decimal(contract.max_value_bytes)};
    internal const int OpcodeBytes = ${formatted_decimal(contract.v1.opcode_bytes)};
    internal const int StatusBytes = ${formatted_decimal(contract.v1.status_bytes)};
    internal const int RequestFixedBytes = ${formatted_decimal(contract.v1.request_fixed_bytes)};
    internal const int ResponseFixedBytes = ${formatted_decimal(contract.v1.response_fixed_bytes)};
    internal const int MinimumVarUIntBytes = ${formatted_decimal(contract.v1.min_varuint_bytes)};
    internal const int MaximumVarUIntBytes = ${formatted_decimal(contract.v1.max_varuint_bytes)};
    internal const int NamespaceIdBytes = ${formatted_decimal(contract.v1.namespace_id_bytes)};
    internal const int NamespaceRevisionBytes = ${formatted_decimal(contract.v1.namespace_revision_bytes)};
    internal const int NamespaceNameLengthBytes = ${formatted_decimal(contract.v1.namespace_name_length_bytes)};
    internal const int NamespaceNameMaxBytes = ${formatted_decimal(contract.v1.namespace_name_max_bytes)};
    internal const int SetFlagsBytes = ${formatted_decimal(contract.v1.set_flags_bytes)};
    internal const byte SetConditionMask = ${formatted_byte(contract.v1.set_condition_mask)};
    internal const byte SetConditionAnyBits = ${formatted_byte(contract.v1.set_condition_any_bits)};
    internal const byte SetConditionReservedBits = ${formatted_byte(contract.v1.set_condition_reserved_bits)};
    internal const byte SetExpirationMask = ${formatted_byte(contract.v1.set_expiration_mask)};
    internal const byte SetInheritExpirationBits = ${formatted_byte(contract.v1.set_inherit_expiration_bits)};
    internal const byte SetNoExpiryBits = ${formatted_byte(contract.v1.set_no_expiry_bits)};
    internal const byte SetExplicitTtlBits = ${formatted_byte(contract.v1.set_ttl_flag)};
    internal const byte SetExpirationReservedBits = ${formatted_byte(contract.v1.set_expiration_reserved_bits)};
    internal const byte SetEvictionMask = ${formatted_byte(contract.v1.set_eviction_mask)};
    internal const byte SetInheritEvictionBits = ${formatted_byte(contract.v1.set_inherit_eviction_bits)};
    internal const byte SetEvictableBits = ${formatted_byte(contract.v1.set_evictable_bits)};
    internal const byte SetEvictionProtectedBits = ${formatted_byte(contract.v1.set_eviction_protected_bits)};
    internal const byte SetEvictionReservedBits = ${formatted_byte(contract.v1.set_eviction_reserved_bits)};
    internal const byte SetReservedMask = ${formatted_byte(contract.v1.set_reserved_mask)};
    internal const int OpenFlagsBytes = ${formatted_decimal(contract.v1.open_flags_bytes)};
    internal const byte OpenCreateIfMissing = ${formatted_byte(contract.v1.open_create_if_missing_flag)};
    internal const byte OpenReservedMask = ${formatted_byte(contract.v1.open_reserved_mask)};
    internal const int DeleteFlagsBytes = ${formatted_decimal(contract.v1.delete_flags_bytes)};
    internal const byte DeleteIfEmpty = ${formatted_byte(contract.v1.delete_if_empty_bits)};
    internal const byte DeleteModeMask = ${formatted_byte(contract.v1.delete_mode_mask)};
    internal const byte DeleteReservedMask = ${formatted_byte(contract.v1.delete_reserved_mask)};
    internal const int PolicyFlagsBytes = ${formatted_decimal(contract.v1.policy_flags_bytes)};
    internal const byte PolicyDefaultExpirationMask = ${formatted_byte(contract.v1.policy_default_expiration_mask)};
    internal const byte PolicyNoExpiry = ${formatted_byte(contract.v1.policy_no_expiry_bits)};
    internal const byte PolicyFixedTtl = ${formatted_byte(contract.v1.policy_fixed_ttl_bits)};
    internal const byte PolicyDefaultExpirationReservedBits = ${formatted_byte(contract.v1.policy_default_expiration_reserved_bits)};
    internal const byte PolicyExpirationOverride = ${formatted_byte(contract.v1.policy_expiration_override_flag)};
    internal const byte PolicyEvictionProtected = ${formatted_byte(contract.v1.policy_eviction_protected_flag)};
    internal const byte PolicyEvictionOverride = ${formatted_byte(contract.v1.policy_eviction_override_flag)};
    internal const byte PolicyReservedMask = ${formatted_byte(contract.v1.policy_reserved_mask)};
    internal const byte ErrorStatusMinimum = ${formatted_byte(contract.v1.error_status_minimum)};
    internal const int DefaultMaxInFlight = ${formatted_decimal(defaults.max_in_flight)};
    internal const long DefaultConnectTimeoutMilliseconds = ${formatted_decimal(defaults.connect_timeout_milliseconds)};
    internal const long DefaultRequestTimeoutMilliseconds = ${formatted_decimal(defaults.request_timeout_milliseconds)};
    internal const int DefaultRetryMaxAttempts = ${formatted_decimal(defaults.retry_max_attempts)};
    internal const int DefaultZstandardLevel = ${formatted_decimal(defaults.zstandard_level)};
    internal const int DefaultZstandardMinimumInputBytes = ${formatted_decimal(defaults.zstandard_minimum_input_bytes)};
    internal const int DefaultZstandardMinimumSavingsBytes = ${formatted_decimal(defaults.zstandard_minimum_savings_bytes)};
    internal const uint FfiAbiVersion = ${formatted_decimal(ffi.abi_version)}u;
${csharp_ffi_constants}
${csharp_api_enum_constants}
    internal const int FfiNamespaceDescriptorSizeBytes = ${formatted_decimal(descriptor_layout.size_bytes)};
${csharp_descriptor_offsets}

    internal const int ItemIdBytes = ${formatted_decimal(contract.item_id_bytes)};
    internal const byte SetIfAbsentBits = ${formatted_byte(contract.v1.set_if_absent_flag)};
    internal const byte SetIfPresentBits = ${formatted_byte(contract.v1.set_if_present_flag)};

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
