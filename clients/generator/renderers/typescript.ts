/** TypeScript operation and value-contract renderers. */

import type { Client_Contract } from "../../client_contract"
import { snake_case } from "../../generator_names"
import { encode_vu128 } from "../../generator_values"
import { operation_field_count } from "../../operation_plans"
import { typescript_api_name } from "../../api_shape_renderers"
import type { Operation_Result_Kind } from "../../compatibility_result_projections"
import {
  operation_uses_optional_value_layout,
  optional_value_framing,
} from "../../compatibility_response_framing"
import { bytes_from_hex } from "../rendering"
import {
  field_sequence_framing,
  has_application_value_codec,
  has_wire_codec,
  managed_operation_entries,
  operation_composite_field_codec,
  operation_composite_fields,
  operation_composite_value_count,
  operation_convenience_fields,
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
  operation_result_kind_constants,
  operation_result_kind_imports,
  operation_uses_compact_item_request,
  operation_uses_compact_namespace_request,
  operation_uses_compact_request_route,
  operation_uses_field_sequence_helpers,
  operation_uses_item_id_helpers,
  render_application_value_codec,
  render_composite_field_decode,
  render_composite_output,
  render_expression_generic_invocation,
  render_field_sequence_request_payload,
  render_field_sequence_response_decode,
  render_operation_result,
  render_opaque_request_expression,
  render_typescript_container_helpers,
  render_typescript_field_sequence_helpers,
  type Managed_Api_Operation,
} from "../managed"

function render_typescript_operation_method(
  contract: Client_Contract,
  operation: Managed_Api_Operation,
): string {
  const method_name = snake_case(operation.name)
  const operation_value = operation.opcode.value
  const result_constant = (kind: Operation_Result_Kind): string =>
    operation_result_constant(operation, kind, "typescript")
  const result_kinds = operation_result_kind_constants(operation)
  const {
    input_condition,
    input_create_if_missing,
    input_eviction_mode,
    input_expected_revision,
    input_expiration_mode,
    input_name,
    input_namespace_id,
    input_policy,
    input_ttl_milliseconds,
    input_value,
    output_descriptor,
    output_deleted,
    output_json,
    output_outcome,
    output_value,
  } = operation_convenience_fields(operation, "typescript")
  const input_item_ids = operation_item_fields(operation).map(
    (member) => operation_field_name(member, "typescript"),
  )
  const input_item_id_expression = input_item_ids.length === 0
    ? "new Uint8Array()"
    : input_item_ids.length === 1
    ? `input.${input_item_ids[0]!}`
    : `smithy_concat_item_ids([${input_item_ids.map((name) => `input.${name}`).join(", ")}])`
  const {
    policy_default_eviction,
    policy_default_expiration,
    policy_default_ttl_milliseconds,
    policy_eviction_override,
    policy_expiration_override,
  } = operation_policy_fields(contract, operation, "typescript")
  const application_value_codecs = operation.plan.application_value_codecs
  const input_request_value = operation_request_value_name(operation, "typescript")
  const scoped_request_value = input_request_value === undefined
    ? ""
    : `        value: input.${input_request_value},\n`
  return render_operation_result(operation, "TypeScript", {
    raw_payload: () => {
      const output_payload = operation_opaque_field_name(operation, "output", "typescript")
      const invocation = render_expression_generic_invocation(
        "typescript",
        operation,
        String(operation_value),
        String(operation_value),
        result_kinds,
      ) ??
        `this.#transport.invoke(${operation_value}, {
      expected_kinds: [${result_kinds}],
    })`
      return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    const result = await ${invocation};
    return { ${output_payload}: result.payload };
      }`
    },
    opaque: () => {
        const input_payload = operation_request_is_opaque(operation)
          ? operation_opaque_field_name(operation, "input", "typescript")
          : undefined
        const output_payload = operation_opaque_field_name(operation, "output", "typescript")
        const codec = render_application_value_codec(
          "typescript",
          application_value_codecs!,
          input_payload === undefined ? "new Uint8Array()" : `input.${input_payload}`,
          "result.payload",
          String(operation_value),
        )
        const decoded_payload = codec.decode
        const invocation = render_expression_generic_invocation(
          "typescript",
          operation,
          String(operation_value),
          String(operation_value),
          result_kinds,
        ) ??
          (operation_is_global_empty(operation)
            ? `this.#transport.invoke(${operation_value}, {
      value: new Uint8Array(),
      expected_kinds: [${result_kinds}],
    })`
            : `this.#transport.invoke_scoped(${operation_value}, input.${input_namespace_id}, {
      item_id: ${input_item_id_expression},
${scoped_request_value}      expected_kinds: [${result_kinds}],
    })`)
        return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    const result = await ${invocation};
    return {
      ${output_payload}: ${decoded_payload},
    };
  }`
    },
    field_sequence: () => {
        const output_decoded_values = operation_composite_fields(operation)
          .map((field, index) =>
            render_composite_field_decode(
              "typescript",
              operation_composite_field_codec(operation, field),
              `values[${index}]`,
              String(operation_value),
              field.required,
              field.type,
            ),
          )
        const output_expression = render_composite_output(
          operation,
          "typescript",
          output_decoded_values,
        )
        const response_values = render_field_sequence_response_decode(
          "typescript",
          operation,
          "result.payload",
          String(operation_value),
        )
        const invocation = render_expression_generic_invocation(
          "typescript",
          operation,
          String(operation_value),
          String(operation_value),
          result_kinds,
        ) ??
          (operation_is_global_empty(operation)
            ? `this.#transport.invoke(${operation_value}, {
      expected_kinds: [${result_kinds}],
    })`
            : `this.#transport.invoke_scoped(
      ${operation_value},
      input.${input_namespace_id},
      {
        item_id: ${input_item_id_expression},
${scoped_request_value}        expected_kinds: [${result_kinds}],
      },
    )`)
        return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    const result = await ${invocation};
    const values = ${response_values};
    return ${output_expression};
  }`
    },
    optional_payload: () => {
      if (operation_field_count(operation.plan.operation, "output", "value") > 1) {
        const output_values = operation_fields(operation, "output", "value")
          .map((member) => operation_field_name(member, "typescript"))
          .map((name, index) => `${name}: values[${index}]`)
          .join(",\n      ")
        return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    const result = await this.#transport.invoke_scoped(
      ${operation_value},
      input.${input_namespace_id},
      {
        item_id: ${input_item_id_expression},
${scoped_request_value}        expected_kinds: [${result_kinds}],
      },
    );
    const values = smithy_decode_optional_values(
      result.payload, ${operation_field_count(operation.plan.operation, "output", "value")}, ${operation_value});
    return {
      ${output_values},
    };
  }`
      }
      return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    const result = await this.#transport.invoke_scoped(
      ${operation_value},
      input.${input_namespace_id},
      {
        item_id: ${input_item_id_expression},
${scoped_request_value}        expected_kinds: [${result_kinds}],
      },
    );
    return result.kind === ${result_constant("not_found")}
      ? {}
      : { ${output_value}: result.payload };
  }`
    },
    status_outcome: () => {
      return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    const result = await this.#transport.invoke_scoped(
      ${operation_value},
      input.${input_namespace_id},
      {
        item_id: ${input_item_id_expression},
        value: input.${input_value},
        condition: input.${input_condition},
        expiration_mode: input.${input_expiration_mode},
        eviction_mode: input.${input_eviction_mode},
        ttl_milliseconds: input.${input_ttl_milliseconds},
        expected_kinds: [${result_kinds}],
      },
    );
    switch (result.kind) {
      case ${result_constant("created")}:
        return { ${output_outcome}: "created" };
      case ${result_constant("replaced")}:
        return { ${output_outcome}: "replaced" };
      case ${result_constant("not_stored")}:
        return { ${output_outcome}: "not_stored" };
      default:
        throw new Error("SET returned an unexpected native result");
    }
  }`
    },
    boolean_outcome: () => {
      return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    const result = await this.#transport.invoke_scoped(
      ${operation_value},
      input.${input_namespace_id},
      {
        item_id: ${input_item_id_expression},
        expected_kinds: [${result_kinds}],
      },
    );
    return { ${output_deleted}: result.kind === ${result_constant("deleted")} };
  }`
    },
    text_payload: () => {
      return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    const result = await this.#transport.invoke_scoped(
      ${operation_value},
      input.${input_namespace_id},
      {
        expected_kinds: [${result_kinds}],
      },
    );
    return {
      ${output_json}: this.#transport.decode_utf8(result.payload, ${operation_value}),
    };
  }`
    },
    empty: () => {
      if (operation_is_global_empty(operation)) {
        return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    await this.#transport.invoke(
      ${operation_value},
      {
        value: new Uint8Array(),
        expected_kinds: [${result_kinds}],
      },
    );
    return {};
  }`
      }
      if (
        operation_is_global_opaque(operation) ||
        operation_is_global_field_sequence(operation)
      ) {
        const request_payload = operation_is_global_opaque(operation)
          ? render_opaque_request_expression("typescript", operation, String(operation_value))
          : render_field_sequence_request_payload(
            "typescript",
            operation,
            String(operation_value),
          )
        return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    await this.#transport.invoke(
      ${operation_value},
      {
        value: ${request_payload},
        expected_kinds: [${result_kinds}],
      },
    );
    return {};
  }`
      }
      if (operation_uses_compact_item_request(operation)) {
        const has_request_value = operation_request_value_count(operation) > 0
        const request_value = has_request_value
          ? `value: input.${input_value},
        condition: input.${input_condition},
        expiration_mode: input.${input_expiration_mode},
        eviction_mode: input.${input_eviction_mode},
        ttl_milliseconds: input.${input_ttl_milliseconds},`
          : ""
        return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    await this.#transport.invoke_scoped(${operation_value}, input.${input_namespace_id}, {
      item_id: ${input_item_id_expression},
      ${request_value}
      expected_kinds: [${result_kinds}],
    });
    return {};
  }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_delete")) {
        return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    await this.#transport.namespace_delete(
      input.${input_namespace_id},
      input.${input_expected_revision},
    );
    return {};
  }`
      }
      if (!operation_uses_compact_namespace_request(operation)) {
        throw new Error(`unsupported generated TypeScript empty operation ${operation.name}`)
      }
      return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    await this.#transport.invoke_scoped(
      ${operation_value},
        input.${input_namespace_id},
      {
        expected_kinds: [${result_kinds}],
      },
    );
    return {};
  }`
    },
    descriptor: () => {
      if (operation_uses_compact_request_route(operation, "namespace_open")) {
        return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    const policy = input.${input_policy};
    return await this.#transport.namespace_open(
      input.${input_name},
      input.${input_create_if_missing},
      policy?.${policy_default_expiration},
      policy?.${policy_expiration_override},
      policy?.${policy_default_eviction},
      policy?.${policy_eviction_override},
      policy?.${policy_default_ttl_milliseconds},
    );
  }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_update_policy")) {
        return `  async ${method_name}(
    input: ${typescript_api_name(operation.input)},
  ): Promise<${typescript_api_name(operation.output)}> {
    this.#transport.assert_open();
    const policy = input.${input_policy};
    return { ${output_descriptor}: await this.#transport.namespace_update_policy(
      input.${input_namespace_id},
      input.${input_expected_revision},
      policy.${policy_default_expiration},
      policy.${policy_expiration_override},
      policy.${policy_default_eviction},
      policy.${policy_eviction_override},
      policy.${policy_default_ttl_milliseconds},
    ) };
  }`
      }
      throw new Error(`unsupported generated TypeScript namespace operation ${operation.name}`)
    },
  })
}

/** Renders generated TypeScript Smithy operation implementations. */
export function render_typescript_operations(contract: Client_Contract): string {
  const managed_operations = managed_operation_entries(contract)
  const result_imports = operation_result_kind_imports(managed_operations)
  const framing = optional_value_framing(contract)
  const field_framing = field_sequence_framing(contract)
  const container_helpers = has_wire_codec(
    managed_operations,
    ["list", "map", "union"],
  )
    ? render_typescript_container_helpers(contract.max_value_bytes)
    : ""
  const field_sequence_helpers = managed_operations.some(
    operation_uses_field_sequence_helpers,
  )
    ? render_typescript_field_sequence_helpers(field_framing)
    : ""
  const imported_types = new Set<string>(["Smithy_OpenKache_Api"])
  for (const operation of managed_operations) {
    imported_types.add(typescript_api_name(operation.input))
    imported_types.add(typescript_api_name(operation.output))
  }
  imported_types.add("Smithy_Namespace_Descriptor")
  imported_types.add("Smithy_Namespace_Open_Output")
  imported_types.add("Smithy_Set_Condition")
  imported_types.add("Smithy_Expiration_Default")
  imported_types.add("Smithy_Expiration_Mode")
  imported_types.add("Smithy_Eviction_Mode")
  imported_types.add("Smithy_Eviction_Default")
  imported_types.add("Smithy_Override_Policy")
  const imports = [...imported_types].sort().join(",\n  ")
  const methods = managed_operations
    .map((operation) => render_typescript_operation_method(contract, operation))
    .join("\n\n")
  const f64_array_helpers = has_application_value_codec(
    managed_operations,
    "packed_f64_be",
  )
    ? `function smithy_encode_f64_array(values: readonly number[]): Uint8Array {
  const payload = new Uint8Array(values.length * 8);
  const view = new DataView(payload.buffer);
  values.forEach((value, index) => {
    if (!Number.isFinite(value)) {
      throw new Error("binary64 array input must contain finite values");
    }
    view.setFloat64(index * 8, value, false);
  });
  return payload;
}

function smithy_decode_f64_array(payload: Uint8Array, operation: number): readonly number[] {
  if (payload.byteLength % 8 !== 0) {
    throw new Error(\`operation \${operation} response has a malformed binary64 array length\`);
  }
  const view = new DataView(payload.buffer, payload.byteOffset, payload.byteLength);
  const values: number[] = [];
  for (let offset = 0; offset < payload.byteLength; offset += 8) {
    const value = view.getFloat64(offset, false);
    if (!Number.isFinite(value)) {
      throw new Error(\`operation \${operation} response contains a non-finite binary64 value\`);
    }
    values.push(value);
  }
  return values;
}
`
    : ""
  const item_id_helpers = managed_operations.some(
    operation_uses_item_id_helpers,
  )
    ? `function smithy_concat_item_ids(itemIds: readonly Uint8Array[]): Uint8Array {
  for (const itemId of itemIds) {
    if (itemId.byteLength !== ${contract.item_id_bytes}) {
      throw new Error("item IDs must contain exactly ${contract.item_id_bytes} bytes");
    }
  }
  const combined = new Uint8Array(itemIds.length * ${contract.item_id_bytes});
  let offset = 0;
  for (const itemId of itemIds) {
    combined.set(itemId, offset);
    offset += itemId.byteLength;
  }
  return combined;
}
`
    : ""
  const optional_values_helpers = managed_operations.some(
    operation_uses_optional_value_layout,
  )
    ? `function smithy_decode_optional_values(
  payload: Uint8Array,
  valueCount: number,
  operation: number,
): readonly (Uint8Array | undefined)[] {
  const view = new DataView(payload.buffer, payload.byteOffset, payload.byteLength);
  const values: Array<Uint8Array | undefined> = [];
  let offset = 0;
  for (let index = 0; index < valueCount; index++) {
    if (offset + ${framing.length_bytes} > payload.byteLength) {
      throw new Error(\`operation \${operation} response is missing an optional-value length\`);
    }
    const length = view.getUint32(offset, false);
    offset += ${framing.length_bytes};
    if (length === ${framing.missing_sentinel}) {
      values.push(undefined);
      continue;
    }
    if (length > ${framing.max_value_bytes}) {
      throw new Error(
        \`operation \${operation} response optional-value entry exceeds the maximum value size\`,
      );
    }
    if (length > payload.byteLength - offset) {
      throw new Error(\`operation \${operation} response contains a truncated optional-value entry\`);
    }
    values.push(payload.slice(offset, offset + length));
    offset += length;
  }
  if (offset !== payload.byteLength) {
    throw new Error(\`operation \${operation} response contains trailing optional-value bytes\`);
  }
  return values;
}
`
    : ""
  const compatibility_helpers = `${item_id_helpers}${optional_values_helpers}`
  return `// Generated from the OpenKache Smithy contract. Do not edit.

import type {
  ${imports},
} from "./smithy-api.js"
import {
  ${result_imports.join(",\n  ")},
} from "./smithy-api.js"

export interface Smithy_Operation_Request {
  readonly item_id?: Uint8Array
  readonly value?: Uint8Array
  readonly condition?: Smithy_Set_Condition
  readonly expiration_mode?: Smithy_Expiration_Mode
  readonly eviction_mode?: Smithy_Eviction_Mode
  readonly ttl_milliseconds?: bigint
  readonly expected_kinds: readonly number[]
}

export interface Smithy_Operation_Result {
  readonly kind: number
  readonly payload: Uint8Array
}

${f64_array_helpers}

${container_helpers}
${field_sequence_helpers}
${compatibility_helpers}

/** Native-facing hooks used by generated Smithy operations. */
export interface Smithy_Operation_Transport {
  assert_open(): void
  invoke(
    operation: number,
    request: Smithy_Operation_Request,
  ): Promise<Smithy_Operation_Result>
  invoke_scoped(
    operation: number,
    namespace_id: bigint,
    request: Smithy_Operation_Request,
  ): Promise<Smithy_Operation_Result>
  decode_utf8(payload: Uint8Array, operation: number): string
  namespace_open(
    name: string,
    create_if_missing: boolean,
    policy_default_expiration?: Smithy_Expiration_Default,
    policy_expiration_override?: Smithy_Override_Policy,
    policy_default_eviction?: Smithy_Eviction_Default,
    policy_eviction_override?: Smithy_Override_Policy,
    policy_default_ttl_milliseconds?: bigint,
  ): Promise<Smithy_Namespace_Open_Output>
  namespace_update_policy(
    namespace_id: bigint,
    expected_revision: bigint,
    default_expiration: Smithy_Expiration_Default,
    expiration_override: Smithy_Override_Policy,
    default_eviction: Smithy_Eviction_Default,
    eviction_override: Smithy_Override_Policy,
    default_ttl_milliseconds?: bigint,
  ): Promise<Smithy_Namespace_Descriptor>
  namespace_delete(namespace_id: bigint, expected_revision: bigint): Promise<void>
}

/** Generated Smithy operations backed by the language adapter's native hooks. */
export class Smithy_Generated_Operations implements Smithy_OpenKache_Api {
  readonly #transport: Smithy_Operation_Transport

  constructor(transport: Smithy_Operation_Transport) {
    this.#transport = transport
  }

${methods}
}
`
}

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

/** Renders constants for the legacy TypeScript metadata envelope.
 *
 * @param contract - Validated language-neutral value-envelope contract.
 * @returns Deterministic TypeScript source with a trailing newline.
 */
export function render_typescript_value_envelope(contract: Client_Contract): string {
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
