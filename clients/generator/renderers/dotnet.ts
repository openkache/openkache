/** .NET wire, API, and operation renderers. */

import type { Wire_Entry } from "../../../protocol/wire"
import type { Api_Type } from "../../operation_models"
import type { Client_Contract, Ffi_Entry } from "../../client_contract"
import { pascal_case, snake_case } from "../../generator_names"
import { encode_vu128 } from "../../generator_values"
import { operation_field_count } from "../../operation_plans"
import type { Operation_Result_Kind } from "../../compatibility_result_projections"
import {
  bytes_from_hex,
  formatted_byte,
  formatted_decimal,
  is_packed_f64_type,
} from "../rendering"
import {
  field_sequence_framing,
  has_application_value_codec,
  has_wire_codec,
  managed_operation_entries,
  managed_operation_label,
  operation_composite_field_codec,
  operation_composite_fields,
  operation_composite_value_count,
  operation_convenience_fields,
  operation_empty_result_constant,
  operation_field_name,
  operation_fields,
  operation_is_global_empty,
  operation_is_global_field_sequence,
  operation_is_global_opaque,
  operation_item_fields,
  operation_opaque_field_name,
  operation_policy_fields,
  operation_request_is_opaque,
  operation_request_value_count,
  operation_request_value_name,
  operation_result_constant,
  operation_uses_compact_item_request,
  operation_uses_compact_namespace_request,
  operation_uses_compact_request_route,
  operation_uses_field_sequence_helpers,
  operation_uses_item_id_helpers,
  operation_uses_optional_value_layout,
  optional_value_framing,
  render_application_value_codec,
  render_composite_field_decode,
  render_composite_output,
  render_csharp_container_helpers,
  render_csharp_field_sequence_helpers,
  render_expression_generic_invocation,
  render_field_sequence_request_payload,
  render_field_sequence_response_decode,
  render_operation_result,
  render_opaque_request_expression,
  type Managed_Api_Operation,
} from "../managed"

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
    ["FfiKeySpec", ffi.key_specs],
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

function csharp_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "byte[]"
      break
    case "boolean":
      rendered = "bool"
      break
    case "double":
      rendered = "double"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = type.name
      break
    case "integer":
      rendered = "int"
      break
    case "list":
      rendered = is_packed_f64_type(type)
        ? "double[]"
        : `System.Collections.Generic.IReadOnlyList<${csharp_api_type(
          type.member ?? { kind: "blob" },
          true,
        )}>`
      break
    case "map":
      rendered = `System.Collections.Generic.IReadOnlyDictionary<${
        csharp_api_type(type.key ?? { kind: "string" }, true)
      }, ${csharp_api_type(type.value ?? { kind: "blob" }, true)}>`
      break
    case "long":
      rendered = "long"
      break
    case "structure":
      if (type.name === undefined) throw new Error("structure API type has no name")
      rendered = type.name
      break
    case "string":
      rendered = "string"
      break
    case "union":
      rendered = "byte[]"
      break
    case "unsigned_long":
      rendered = "ulong"
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
    const wire_members = enum_.members
      .map((member) => `            ${enum_.name}.${member.name} => ${JSON.stringify(member.value)},`)
      .join("\n")
    const wire_values = enum_.members
      .map((member) => `            ${JSON.stringify(member.value)} => ${enum_.name}.${member.name},`)
      .join("\n")
    return `/// <summary>Values defined by the Smithy ${enum_.name} shape.</summary>
public enum ${enum_.name}
{
${members}
}

internal static class Smithy${enum_.name}Wire
{
    internal static string ToValue(${enum_.name} value) => value switch
    {
${wire_members}
        _ => throw new System.ArgumentOutOfRangeException(nameof(value)),
    };

    internal static ${enum_.name} FromValue(string value) => value switch
    {
${wire_values}
        _ => throw new System.ArgumentException("unknown ${enum_.name} value", nameof(value)),
    };
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

function render_csharp_operation_method_body(
  contract: Client_Contract,
  operation: Managed_Api_Operation,
): string {
  const method_name = `${operation.name}Async`
  const label = managed_operation_label(operation)
  const result_constant = (kind: Operation_Result_Kind): string =>
    operation_result_constant(operation, kind, "csharp")
  const empty_result_constant = operation_empty_result_constant(operation, "csharp")
  const {
    input_condition,
    input_create_if_missing,
    input_eviction_mode,
    input_expected_revision,
    input_expiration_mode,
    input_item_id,
    input_name,
    input_namespace_id,
    input_policy,
    input_ttl_milliseconds,
    input_value,
    output_created,
    output_deleted,
    output_descriptor,
    output_json,
    output_outcome,
    output_value,
  } = operation_convenience_fields(operation, "csharp")
  const input_item_ids = operation_item_fields(operation).map(
    (member) => operation_field_name(member, "csharp"),
  )
  const input_item_id_expression = input_item_ids.length === 0
    ? "ReadOnlyMemory<byte>.Empty"
    : input_item_ids.length <= 1
    ? `ValidateItemId(input.${input_item_id})`
    : `ConcatItemIds(${input_item_ids.map((name) => `input.${name}`).join(", ")})`
  const {
    policy_default_eviction,
    policy_default_expiration,
    policy_default_ttl_milliseconds,
    policy_eviction_override,
    policy_expiration_override,
  } = operation_policy_fields(contract, operation, "csharp")
  const application_value_codecs = operation.plan.application_value_codecs
  const input_request_value = operation_request_value_name(operation, "csharp")
    ?? "ReadOnlyMemory<byte>.Empty"
  return render_operation_result(operation, "C#", {
    raw_payload: () => {
      const output_payload = operation_opaque_field_name(operation, "output", "csharp")
      const invocation = render_expression_generic_invocation(
        "csharp",
        operation,
        `Protocol.Opcode.${operation.name}`,
        `"${label}"`,
      ) ??
        `RequestAsync(
            Protocol.Opcode.${operation.name},
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken)`
      return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var result = await ${invocation}.ConfigureAwait(false);
        ExpectKind("${label}", result, ${result_constant("ok")});
        return new Smithy.${operation.output}
        {
            ${output_payload} = result.Payload,
        };
    }`
    },
    opaque: () => {
        const input_payload = operation_request_is_opaque(operation)
          ? operation_opaque_field_name(operation, "input", "csharp")
          : undefined
        const output_payload = operation_opaque_field_name(operation, "output", "csharp")
        const codec = render_application_value_codec(
          "csharp",
          application_value_codecs!,
          input_payload === undefined
            ? "ReadOnlyMemory<byte>.Empty"
            : `input.${input_payload}`,
          "result.Payload",
          `"${label}"`,
        )
        const decoded_payload = codec.decode
        const invocation = render_expression_generic_invocation(
          "csharp",
          operation,
          `Protocol.Opcode.${operation.name}`,
          `"${label}"`,
        ) ??
          (operation_is_global_empty(operation)
            ? `RequestAsync(
            Protocol.Opcode.${operation.name},
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken)`
            : `RequestScopedAsync(
            Protocol.Opcode.${operation.name},
            input.${input_namespace_id},
            ${input_item_id_expression},
            ${input_request_value},
            cancellationToken: cancellationToken)`)
        return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var result = await ${invocation}.ConfigureAwait(false);
        ExpectKind("${label}", result, ${result_constant("value")});
        return new Smithy.${operation.output}
        {
            ${output_payload} = ${decoded_payload},
        };
    }`
    },
    field_sequence: () => {
        const output_decoded_values = operation_composite_fields(operation)
          .map((field, index) =>
            render_composite_field_decode(
              "csharp",
              operation_composite_field_codec(operation, field),
              `values[${index}]`,
              `"${label}"`,
              field.required,
              field.type,
            ),
          )
        const output_expression = render_composite_output(
          operation,
          "csharp",
          output_decoded_values,
        )
        const response_values = operation.plan.contract.response_framing === "field_sequence"
          ? render_field_sequence_response_decode(
            "csharp",
            operation,
            "result.Payload",
            `"${label}"`,
          )
          : `DecodeOptionalValues(result.Payload, ${operation_composite_value_count(operation)}, "${label}")`
        const invocation = render_expression_generic_invocation(
          "csharp",
          operation,
          `Protocol.Opcode.${operation.name}`,
          `"${label}"`,
        ) ??
          (operation_is_global_empty(operation)
            ? `RequestAsync(
            Protocol.Opcode.${operation.name},
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken)`
            : `RequestScopedAsync(
            Protocol.Opcode.${operation.name},
            input.${input_namespace_id},
            ${input_item_id_expression},
            ${input_request_value === "ReadOnlyMemory<byte>.Empty" ? "ReadOnlyMemory<byte>.Empty" : `input.${input_request_value}`},
            cancellationToken: cancellationToken)`)
        return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var result = await ${invocation}.ConfigureAwait(false);
        ExpectKind("${label}", result, ${result_constant("value")});
        var values = ${response_values};
        return ${output_expression};
    }`
    },
    optional_payload: () => {
      if (operation_field_count(operation.plan.operation, "output", "value") > 1) {
        const output_values = operation_fields(operation, "output", "value")
          .map((member, index) =>
            `            ${operation_field_name(member, "csharp")} = values[${index}],`)
          .join("\n")
        return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var result = await RequestScopedAsync(
            Protocol.Opcode.${operation.name},
            input.${input_namespace_id},
            ${input_item_id_expression},
            ${input_request_value === "ReadOnlyMemory<byte>.Empty" ? "ReadOnlyMemory<byte>.Empty" : `input.${input_request_value}`},
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("${label}", result, ${result_constant("value")});
        var values = DecodeOptionalValues(
            result.Payload,
            ${operation_field_count(operation.plan.operation, "output", "value")},
            "${label}");
        return new Smithy.${operation.output}
        {
${output_values}
        };
    }`
      }
      return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var result = await RequestScopedAsync(
            Protocol.Opcode.${operation.name},
            input.${input_namespace_id},
            ${input_item_id_expression},
            ${input_request_value === "ReadOnlyMemory<byte>.Empty" ? "ReadOnlyMemory<byte>.Empty" : `input.${input_request_value}`},
            cancellationToken: cancellationToken).ConfigureAwait(false);
        return new Smithy.${operation.output}
        {
            ${output_value} = result.Kind switch
            {
                var kind when kind == ${result_constant("value")} => result.Payload,
                var kind when kind == ${result_constant("not_found")} => null,
                _ => throw UnexpectedKind("${label}", result.Kind),
            },
        };
    }`
    },
    status_outcome: () => {
      return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var (setFlags, ttlMilliseconds) = NativeSetOptions(
            input.${input_condition},
            input.${input_expiration_mode},
            input.${input_ttl_milliseconds},
            input.${input_eviction_mode});
        var result = await RequestScopedAsync(
            Protocol.Opcode.${operation.name},
            input.${input_namespace_id},
            ${input_item_id_expression},
            ValidateValue(input.${input_value}),
            setFlags,
            ttlMilliseconds,
            cancellationToken).ConfigureAwait(false);
        return new Smithy.${operation.output}
        {
            ${output_outcome} = result.Kind switch
            {
                var kind when kind == ${result_constant("created")} => Smithy.SetOutcome.Created,
                var kind when kind == ${result_constant("replaced")} => Smithy.SetOutcome.Replaced,
                var kind when kind == ${result_constant("not_stored")} => Smithy.SetOutcome.NotStored,
                _ => throw UnexpectedKind("${label}", result.Kind),
            },
        };
    }`
    },
    boolean_outcome: () => {
      return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var result = await RequestScopedAsync(
            Protocol.Opcode.${operation.name},
            input.${input_namespace_id},
            ${input_item_id_expression},
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        return new Smithy.${operation.output}
        {
            ${output_deleted} = result.Kind switch
            {
                var kind when kind == ${result_constant("deleted")} => true,
                var kind when kind == ${result_constant("not_deleted")} => false,
                _ => throw UnexpectedKind("${label}", result.Kind),
            },
        };
    }`
    },
    text_payload: () => {
      return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var result = await RequestScopedAsync(
            Protocol.Opcode.${operation.name},
            input.${input_namespace_id},
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("${label}", result, ${result_constant("value")});
        return new Smithy.${operation.output}
        {
            ${output_json} = new UTF8Encoding(false, true).GetString(result.Payload),
        };
    }`
    },
    empty: () => {
      if (operation_is_global_empty(operation)) {
        return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var result = await RequestAsync(
            Protocol.Opcode.${operation.name},
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("${label}", result, ${empty_result_constant});
        return new Smithy.${operation.output}();
    }`
      }
      if (
        operation_is_global_opaque(operation) ||
        operation_is_global_field_sequence(operation)
      ) {
        const request_payload = operation_is_global_opaque(operation)
          ? render_opaque_request_expression("csharp", operation, `"${label}"`)
          : render_field_sequence_request_payload(
            "csharp",
            operation,
            `"${label}"`,
          )
        return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var result = await RequestAsync(
            Protocol.Opcode.${operation.name},
            ReadOnlyMemory<byte>.Empty,
            ${request_payload},
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("${label}", result, ${empty_result_constant});
        return new Smithy.${operation.output}();
    }`
      }
      if (operation_uses_compact_item_request(operation)) {
        const has_request_value = operation_request_value_count(operation) > 0
        if (has_request_value) {
          return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var (setFlags, ttlMilliseconds) = NativeSetOptions(
            input.${input_condition},
            input.${input_expiration_mode},
            input.${input_ttl_milliseconds},
            input.${input_eviction_mode});
        var result = await RequestScopedAsync(
            Protocol.Opcode.${operation.name},
            input.${input_namespace_id},
            ${input_item_id_expression},
            ValidateValue(input.${input_value}),
            setFlags,
            ttlMilliseconds,
            cancellationToken).ConfigureAwait(false);
        ExpectKind("${label}", result, ${empty_result_constant});
        return new Smithy.${operation.output}();
    }`
        }
        return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var result = await RequestScopedAsync(
            Protocol.Opcode.${operation.name},
            input.${input_namespace_id},
            ${input_item_id_expression},
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("${label}", result, ${empty_result_constant});
        return new Smithy.${operation.output}();
    }`
      }
      if (operation_uses_compact_namespace_request(operation)) {
        return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var result = await RequestScopedAsync(
            Protocol.Opcode.${operation.name},
            input.${input_namespace_id},
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("${label}", result, ${empty_result_constant});
        return new Smithy.${operation.output}();
    }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_delete")) {
        return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        try
        {
            var result = await _nativeClient.NamespaceDeleteAsync(
                input.${input_namespace_id},
                input.${input_expected_revision},
                cancellationToken).ConfigureAwait(false);
            ExpectKind("${label}", result, ${empty_result_constant});
            return new Smithy.${operation.output}();
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            throw new OpenKacheException("TIMEOUT", "${label} exceeded.");
        }
        catch (NativeException error)
        {
            throw MapNativeError(error, "${label}_FAILED");
        }
    }`
      }
      throw new Error(`unsupported generated C# empty operation ${operation.name}`)
    },
    descriptor: () => {
      if (operation_uses_compact_request_route(operation, "namespace_open")) {
        return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var name = Encoding.UTF8.GetBytes(input.${input_name});
        if (name.Length > Protocol.NamespaceNameMaxBytes)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                $"namespace name exceeds {Protocol.NamespaceNameMaxBytes} UTF-8 octets.");
        }
        if (input.${input_create_if_missing} && input.${input_policy} is null)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                "namespace policy is required when CreateIfMissing is true.");
        }
        if (!input.${input_create_if_missing} && input.${input_policy} is not null)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                "namespace policy is only valid when CreateIfMissing is true.");
        }
        var (policyFlags, ttlMilliseconds) = input.${input_policy} is null
            ? ((byte)0, 0UL)
            : NativePolicy(
                input.${input_policy}.${policy_default_expiration},
                input.${input_policy}.${policy_default_ttl_milliseconds},
                input.${input_policy}.${policy_expiration_override},
                input.${input_policy}.${policy_default_eviction},
                input.${input_policy}.${policy_eviction_override});
        try
        {
            var result = await _nativeClient.NamespaceOpenAsync(
                name,
                input.${input_create_if_missing},
                policyFlags,
                ttlMilliseconds,
                cancellationToken).ConfigureAwait(false);
            if (result.Kind != ${result_constant("ok")}
                && result.Kind != ${result_constant("created")})
            {
                throw UnexpectedKind("${label}", result.Kind);
            }
            return new Smithy.${operation.output}
            {
                ${output_descriptor} = DecodeNamespaceDescriptor(result.Payload),
                ${output_created} = result.Kind == ${result_constant("created")},
            };
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            throw new OpenKacheException("TIMEOUT", "${label} exceeded.");
        }
        catch (NativeException error)
        {
            throw MapNativeError(error, "${label}_FAILED");
        }
    }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_update_policy")) {
        return `    public async ValueTask<Smithy.${operation.output}> ${method_name}(
        Smithy.${operation.input} input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        var (policyFlags, ttlMilliseconds) = NativePolicy(
            input.${input_policy}.${policy_default_expiration},
            input.${input_policy}.${policy_default_ttl_milliseconds},
            input.${input_policy}.${policy_expiration_override},
            input.${input_policy}.${policy_default_eviction},
            input.${input_policy}.${policy_eviction_override});
        try
        {
            var result = await _nativeClient.NamespaceUpdatePolicyAsync(
                input.${input_namespace_id},
                input.${input_expected_revision},
                policyFlags,
                ttlMilliseconds,
                cancellationToken).ConfigureAwait(false);
            ExpectKind("${label}", result, ${result_constant("value")});
            return new Smithy.${operation.output}
            {
                ${output_descriptor} = DecodeNamespaceDescriptor(result.Payload),
            };
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            throw new OpenKacheException("TIMEOUT", "${label} exceeded.");
        }
        catch (NativeException error)
        {
            throw MapNativeError(error, "${label}_FAILED");
        }
    }`
      }
      throw new Error(`unsupported generated C# namespace operation ${operation.name}`)
    },
  })
}

function render_csharp_operation_method(
  contract: Client_Contract,
  operation: Managed_Api_Operation,
): string {
  return `    /// <summary>Invokes the generated Smithy ${operation.name} operation.</summary>
${render_csharp_operation_method_body(contract, operation)}`
}

/** Renders generated C# Smithy operation implementations. */
export function render_csharp_operations(contract: Client_Contract): string {
  const managed_operations = managed_operation_entries(contract)
  const framing = optional_value_framing(contract)
  const field_framing = field_sequence_framing(contract)
  const container_helpers = has_wire_codec(
    managed_operations,
    ["list", "map", "union"],
  )
    ? render_csharp_container_helpers(contract.max_value_bytes)
    : ""
  const field_sequence_helpers = managed_operations.some(
    operation_uses_field_sequence_helpers,
  )
    ? render_csharp_field_sequence_helpers(field_framing)
    : ""
  const methods = managed_operations
    .map((operation) => render_csharp_operation_method(contract, operation))
    .join("\n\n")
  const f64_array_helpers = has_application_value_codec(
    managed_operations,
    "packed_f64_be",
  )
    ? `    private static byte[] EncodeF64Array(double[] values)
    {
        var payload = new byte[checked(values.Length * 8)];
        for (var index = 0; index < values.Length; index++)
        {
            var value = values[index];
            if (!double.IsFinite(value))
            {
                throw new OpenKacheException(
                    "PROTOCOL_ERROR",
                    "binary64 array input must contain finite values.");
            }
            BinaryPrimitives.WriteInt64BigEndian(
                payload.AsSpan(index * 8, 8),
                BitConverter.DoubleToInt64Bits(value));
        }
        return payload;
    }

    private static double[] DecodeF64Array(byte[] payload, string operation)
    {
        if (payload.Length % 8 != 0)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                    $"{operation} response has a malformed binary64 array length.");
        }
        var values = new double[payload.Length / 8];
        for (var index = 0; index < values.Length; index++)
        {
            var value = BitConverter.Int64BitsToDouble(
                BinaryPrimitives.ReadInt64BigEndian(payload.AsSpan(index * 8, 8)));
            if (!double.IsFinite(value))
            {
                throw new OpenKacheException(
                    "PROTOCOL_ERROR",
                    $"{operation} response contains a non-finite binary64 value.");
            }
            values[index] = value;
        }
        return values;
    }
`
    : ""
  const item_id_helpers = managed_operations.some(
    operation_uses_item_id_helpers,
  )
    ? `    private static byte[] ConcatItemIds(params byte[][] itemIds)
    {
        var total = 0;
        foreach (var itemId in itemIds)
        {
            if (itemId.Length > Protocol.ItemIdBytes)
            {
                throw new OpenKacheException(
                    "PROTOCOL_ERROR",
                    $"item IDs must contain at most {Protocol.ItemIdBytes} bytes.");
            }
            if (total > Protocol.ItemIdBytes - itemId.Length)
            {
                throw new OpenKacheException(
                    "PROTOCOL_ERROR",
                    $"combined item IDs must contain at most {Protocol.ItemIdBytes} bytes.");
            }
            total = checked(total + itemId.Length);
        }
        var combined = new byte[total];
        var offset = 0;
        foreach (var itemId in itemIds)
        {
            itemId.CopyTo(combined, offset);
            offset += itemId.Length;
        }
        return combined;
    }
`
    : ""
  const optional_values_helpers = managed_operations.some(
    operation_uses_optional_value_layout,
  )
    ? `    private static byte[]?[] DecodeOptionalValues(
        byte[] payload,
        int valueCount,
        string operation)
    {
        var values = new byte[]?[valueCount];
        var offset = 0;
        for (var index = 0; index < values.Length; index++)
        {
            if (payload.Length - offset < ${framing.length_bytes})
            {
                throw new OpenKacheException(
                    "PROTOCOL_ERROR",
                    $"{operation} response is missing an optional-value length.");
            }
            var length = BinaryPrimitives.ReadUInt32BigEndian(payload.AsSpan(offset, ${framing.length_bytes}));
            offset += ${framing.length_bytes};
            if (length == ${framing.missing_sentinel}u)
            {
                continue;
            }
            if (length > ${framing.max_value_bytes}u)
            {
                throw new OpenKacheException(
                    "PROTOCOL_ERROR",
                    $"{operation} response optional-value entry exceeds the maximum value size.");
            }
            if (length > (uint)(payload.Length - offset))
            {
                throw new OpenKacheException(
                    "PROTOCOL_ERROR",
                    $"{operation} response contains a truncated optional-value entry.");
            }
            values[index] = payload.AsSpan(offset, checked((int)length)).ToArray();
            offset += checked((int)length);
        }
        if (offset != payload.Length)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                $"{operation} response contains trailing optional-value bytes.");
        }
        return values;
    }
`
    : ""
  return `// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

// Generated from the OpenKache Smithy client contract. Do not edit.

#nullable enable

using System.Text;
using System.Buffers.Binary;
using System.Collections.Generic;
using System.Linq;

namespace OpenKache;

public sealed partial class Client
{
${f64_array_helpers}
${container_helpers}
${field_sequence_helpers}
${item_id_helpers}${optional_values_helpers}
${methods}
}
`
}
