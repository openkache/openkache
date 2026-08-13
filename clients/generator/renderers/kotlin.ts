/** Kotlin generated operation and contract renderers. */

import type { Client_Contract } from "../../client_contract"
import { lower_camel_case, snake_case } from "../../generator_names"
import { operation_field_count } from "../../operation_plans"
import type { Operation_Result_Kind } from "../../compatibility_result_projections"
import {
  operation_uses_optional_value_layout,
  optional_value_framing,
} from "../../compatibility_response_framing"
import {
  adapter_contract_values,
  field_sequence_framing,
  has_application_value_codec,
  has_wire_codec,
  managed_operation_constant,
  managed_operation_entries,
  managed_operation_label,
  operation_composite_field_codec,
  operation_composite_fields,
  operation_composite_value_count,
  operation_convenience_fields,
  operation_field_name,
  operation_fields,
  operation_is_global_empty,
  operation_item_fields,
  operation_opaque_field_name,
  operation_request_is_opaque,
  operation_request_value_count,
  operation_request_value_name,
  operation_result_constant,
  operation_uses_compact_item_request,
  operation_uses_compact_namespace_request,
  operation_uses_compact_request_route,
  operation_uses_field_sequence_helpers,
  operation_uses_item_id_helpers,
  render_application_value_codec,
  render_composite_field_decode,
  render_composite_output,
  render_expression_generic_invocation,
  render_field_sequence_response_decode,
  render_kotlin_container_helpers,
  render_kotlin_field_sequence_helpers,
  render_kotlin_operation_metadata,
  render_operation_result,
  structure_convenience_fields,
  type Managed_Api_Operation,
} from "../managed"

function render_kotlin_operation_method(operation: Managed_Api_Operation): string {
  const operation_constant = managed_operation_constant(operation, "kotlin")
  const operation_label = managed_operation_label(operation)
  const result_constant = (kind: Operation_Result_Kind): string =>
    operation_result_constant(operation, kind, "kotlin")
  const method_name = lower_camel_case(operation.name)
  const {
    input_create_if_missing,
    input_expected_revision,
    input_name,
    input_namespace_id,
    input_policy,
    input_value,
    output_created,
    output_deleted,
    output_descriptor,
    output_json,
    output_outcome,
    output_value,
  } = operation_convenience_fields(operation, "kotlin")
  const input_item_ids = operation_item_fields(operation).map(
    (member) => operation_field_name(member, "kotlin"),
  )
  const input_item_id_expression = input_item_ids.length === 0
    ? "byteArrayOf()"
    : input_item_ids.length === 1
    ? `input.${input_item_ids[0]!}`
    : `smithyConcatItemIds(${input_item_ids.map((name) => `input.${name}`).join(", ")})`
  const application_value_codecs = operation.plan.application_value_codecs
  const input_request_value = operation_request_value_name(operation, "kotlin")
    ?? "byteArrayOf()"
  const prefix = `    override suspend fun ${method_name}(input: ${operation.input}): ${operation.output} =
        withContext(Dispatchers.IO) {
            requireNotNull(input)
`
  return render_operation_result(operation, "Kotlin", {
    raw_payload: () => {
      const output_payload = operation_opaque_field_name(operation, "output", "kotlin")
      const invocation = render_expression_generic_invocation(
        "kotlin",
        operation,
        operation_constant,
        `"${operation_label}"`,
      ) ??
        `smithyInvoke(
                ${operation_constant},
                byteArrayOf(),
                byteArrayOf(),
            )`
      return `${prefix}            val result = ${invocation}
            smithyRequireKind(result, ${result_constant("ok")}, "${operation_label}")
            ${operation.output}(
                ${output_payload} = result.payload,
            )
        }`
      },
    opaque: () => {
        const input_payload = operation_request_is_opaque(operation)
          ? operation_opaque_field_name(operation, "input", "kotlin")
          : undefined
        const output_payload = operation_opaque_field_name(operation, "output", "kotlin")
        const codec = render_application_value_codec(
          "kotlin",
          application_value_codecs!,
          input_payload === undefined ? "byteArrayOf()" : `input.${input_payload}`,
          "result.payload",
          `"${operation_label}"`,
        )
        const decoded_payload = codec.decode
        const invocation = render_expression_generic_invocation(
          "kotlin",
          operation,
          operation_constant,
          `"${operation_label}"`,
        ) ??
          (operation_is_global_empty(operation)
            ? `smithyInvoke(
                ${operation_constant},
                byteArrayOf(),
                byteArrayOf(),
            )`
            : `smithyInvokeScoped(
                ${operation_constant},
                input.${input_namespace_id},
                ${input_item_id_expression},
                ${input_request_value},
            )`)
        return `${prefix}            val result = ${invocation}
            smithyRequireKind(result, ${result_constant("value")}, "${operation_label}")
            ${operation.output}(
                ${output_payload} = ${decoded_payload},
            )
        }`
      },
    field_sequence: () => {
        const output_decoded_values = operation_composite_fields(operation)
          .map((field, index) =>
            render_composite_field_decode(
              "kotlin",
              operation_composite_field_codec(operation, field),
              `values[${index}]`,
              `"${operation_label}"`,
              field.required,
              field.type,
            ),
          )
        const output_expression = render_composite_output(
          operation,
          "kotlin",
          output_decoded_values,
        )
        const response_values = render_field_sequence_response_decode(
          "kotlin",
          operation,
          "result.payload",
          `"${operation_label}"`,
        )
        const invocation = render_expression_generic_invocation(
          "kotlin",
          operation,
          operation_constant,
          `"${operation_label}"`,
        ) ??
          (operation_is_global_empty(operation)
            ? `smithyInvoke(
                ${operation_constant},
                byteArrayOf(),
                byteArrayOf(),
            )`
            : `smithyInvokeScoped(
                ${operation_constant},
                input.${input_namespace_id},
                ${input_item_id_expression},
                ${input_request_value},
            )`)
        return `${prefix}            val result = ${invocation}
            smithyRequireKind(result, ${result_constant("value")}, "${operation_label}")
            val values = ${response_values}
            ${output_expression}
        }`
      },
    optional_payload: () => {
      if (operation_field_count(operation.plan.operation, "output", "value") > 1) {
        const output_values = operation_fields(operation, "output", "value")
          .map((_member, index) => `values[${index}]`)
          .join(",\n                ")
        return `${prefix}            val result = smithyInvokeScoped(
                ${operation_constant},
                input.${input_namespace_id},
                ${input_item_id_expression},
                ${input_request_value},
            )
            smithyRequireKind(result, ${result_constant("value")}, "${operation_label}")
            val values = smithyDecodeOptionalValues(
                result.payload, ${operation_field_count(operation.plan.operation, "output", "value")}, "${operation_label}")
            ${operation.output}(
                ${output_values},
            )
        }`
      }
      return `${prefix}            val result = smithyInvokeScoped(
                ${operation_constant},
                input.${input_namespace_id},
                ${input_item_id_expression},
                ${input_request_value},
            )
            when (result.kind) {
                ${result_constant("value")} -> ${operation.output}(${output_value} = result.payload)
                ${result_constant("not_found")} -> ${operation.output}(${output_value} = null)
                else -> throw smithyUnexpectedKind("${operation_label}", result.kind)
            }
        }`
    },
    status_outcome: () => {
      return `${prefix}            val flags = smithySetFlags(input)
            val result = smithyInvokeScoped(
                ${operation_constant},
                input.${input_namespace_id},
                ${input_item_id_expression},
                input.${input_value},
                flags.first,
                flags.second,
            )
            val outcome = when (result.kind) {
                ${result_constant("created")} -> SetOutcome.Created
                ${result_constant("replaced")} -> SetOutcome.Replaced
                ${result_constant("not_stored")} -> SetOutcome.NotStored
                else -> throw smithyUnexpectedKind("${operation_label}", result.kind)
            }
            ${operation.output}(${output_outcome} = outcome)
        }`
    },
    boolean_outcome: () => {
      return `${prefix}            val result = smithyInvokeScoped(
                ${operation_constant},
                input.${input_namespace_id},
                ${input_item_id_expression},
                byteArrayOf(),
            )
            when (result.kind) {
                ${result_constant("deleted")} -> ${operation.output}(${output_deleted} = true)
                ${result_constant("not_deleted")} -> ${operation.output}(${output_deleted} = false)
                else -> throw smithyUnexpectedKind("${operation_label}", result.kind)
            }
        }`
    },
    text_payload: () => {
      return `${prefix}            val result = smithyInvokeScoped(
                ${operation_constant},
                input.${input_namespace_id},
                byteArrayOf(),
                byteArrayOf(),
            )
            smithyRequireKind(result, ${result_constant("value")}, "${operation_label}")
            ${operation.output}(
                ${output_json} = smithyDecodeUtf8(result.payload, "${operation_label}"),
            )
        }`
    },
    empty: () => {
        const invocation = render_expression_generic_invocation(
          "kotlin",
          operation,
          operation_constant,
          `"${operation_label}"`,
        )
        if (invocation !== undefined) {
          return `${prefix}            val result = ${invocation}
            smithyRequireKind(result, ${result_constant("ok")}, "${operation_label}")
            ${operation.output}()
        }`
        }
      if (operation_uses_compact_item_request(operation)) {
        const has_request_value = operation_request_value_count(operation) > 0
        const request_value = has_request_value
          ? `input.${input_value}`
          : "byteArrayOf()"
        if (has_request_value) {
          return `${prefix}            val flags = smithySetFlags(input)
            val result = smithyInvokeScoped(
                ${operation_constant},
                input.${input_namespace_id},
                ${input_item_id_expression},
                ${request_value},
                flags.first,
                flags.second,
            )
            smithyRequireKind(result, ${result_constant("ok")}, "${operation_label}")
            ${operation.output}()
        }`
        }
        return `${prefix}            val result = smithyInvokeScoped(
                ${operation_constant},
                input.${input_namespace_id},
                ${input_item_id_expression},
                ${request_value},
            )
            smithyRequireKind(result, ${result_constant("ok")}, "${operation_label}")
            ${operation.output}()
        }`
      }
      if (operation_uses_compact_namespace_request(operation)) {
        return `${prefix}            val result = smithyInvokeScoped(
                ${operation_constant},
                input.${input_namespace_id},
                byteArrayOf(),
                byteArrayOf(),
            )
            smithyRequireKind(result, ${result_constant("ok")}, "${operation_label}")
            ${operation.output}()
        }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_delete")) {
        return `${prefix}            val result = smithyNamespaceDelete(
                input.${input_namespace_id},
                input.${input_expected_revision},
            )
            smithyRequireKind(result, ${result_constant("ok")}, "${operation_label}")
            ${operation.output}()
        }`
      }
      throw new Error(`unsupported generated Kotlin empty operation ${operation.name}`)
    },
    descriptor: () => {
      if (operation_uses_compact_request_route(operation, "namespace_open")) {
        return `${prefix}            val name = input.${input_name}.toByteArray(StandardCharsets.UTF_8)
            require(name.size <= SmithyContract.NAMESPACE_NAME_MAX_BYTES) {
                "namespace name exceeds protocol limit"
            }
            val policy = smithyPolicyFlags(
                input.${input_policy},
                input.${input_create_if_missing},
            )
            val result = smithyNamespaceOpen(
                name,
                input.${input_create_if_missing},
                policy.first,
                policy.second,
            )
            val created = result.kind == ${result_constant("created")}
            if (!created && result.kind != ${result_constant("ok")}) {
                throw smithyUnexpectedKind("${operation_label}", result.kind)
            }
            ${operation.output}(
                ${output_descriptor} = smithyDecodeDescriptor(result.payload),
                ${output_created} = created,
            )
        }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_update_policy")) {
        return `${prefix}            val policy = smithyPolicyFlags(input.${input_policy}, true)
            val result = smithyNamespaceUpdatePolicy(
                input.${input_namespace_id},
                input.${input_expected_revision},
                policy.first,
                policy.second,
            )
            smithyRequireKind(result, ${result_constant("value")}, "${operation_label}")
            ${operation.output}(${output_descriptor} = smithyDecodeDescriptor(result.payload))
        }`
      }
      throw new Error(`unsupported generated Kotlin namespace operation ${operation.name}`)
    },
  })
}

export function render_kotlin_operations(contract: Client_Contract): string {
  const managed_operations = managed_operation_entries(contract)
  const framing = optional_value_framing(contract)
  const field_framing = field_sequence_framing(contract)
  const container_helpers = has_wire_codec(
    managed_operations,
    ["list", "map", "union"],
  )
    ? render_kotlin_container_helpers(contract.max_value_bytes)
    : ""
  const field_sequence_helpers = managed_operations.some(
    operation_uses_field_sequence_helpers,
  )
    ? render_kotlin_field_sequence_helpers(field_framing)
    : ""
  const methods = managed_operations
    .map(render_kotlin_operation_method)
    .join("\n\n")
  const f64_array_helpers = has_application_value_codec(
    managed_operations,
    "packed_f64_be",
  )
    ? `    private fun smithyEncodeF64Array(values: DoubleArray): ByteArray {
        val buffer = ByteBuffer.allocate(values.size * java.lang.Double.BYTES)
            .order(ByteOrder.BIG_ENDIAN)
        values.forEach { value ->
            require(value.isFinite()) {
                "binary64 array input must contain finite values"
            }
            buffer.putDouble(value)
        }
        return buffer.array()
    }

    private fun smithyDecodeF64Array(payload: ByteArray, operation: String): DoubleArray {
        require(payload.size % java.lang.Double.BYTES == 0) {
            "$operation response has a malformed binary64 array length"
        }
        val buffer = ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN)
        return DoubleArray(payload.size / java.lang.Double.BYTES) {
            buffer.double.also { value ->
                require(value.isFinite()) {
                    "$operation response contains a non-finite binary64 value"
                }
            }
        }
    }
`
    : ""
  const item_id_helpers = managed_operations.some(
    operation_uses_item_id_helpers,
  )
    ? `    private fun smithyConcatItemIds(vararg itemIds: ByteArray): ByteArray {
        val total = itemIds.sumOf { itemId ->
            require(itemId.size <= SmithyContract.ITEM_ID_BYTES) {
                "item IDs must contain at most \${SmithyContract.ITEM_ID_BYTES} bytes"
            }
            1 + itemId.size
        }
        val combined = ByteArray(total)
        var offset = 0
        itemIds.forEach { itemId ->
            combined[offset++] = itemId.size.toByte()
            itemId.copyInto(combined, offset)
            offset += itemId.size
        }
        return combined
    }
`
    : ""
  const optional_values_helpers = managed_operations.some(
    operation_uses_optional_value_layout,
  )
    ? `    private fun smithyDecodeOptionalValues(
        payload: ByteArray,
        valueCount: Int,
        operation: String,
    ): Array<ByteArray?> {
        val buffer = ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN)
        val values = arrayOfNulls<ByteArray>(valueCount)
        for (index in values.indices) {
            require(buffer.remaining() >= ${framing.length_bytes}) {
                "\$operation response is missing an optional-value length"
            }
            val length = buffer.int.toLong() and ${framing.missing_sentinel}L
            if (length == ${framing.missing_sentinel}L) continue
            require(length <= ${framing.max_value_bytes}L) {
                "\$operation response optional-value entry exceeds the maximum value size"
            }
            require(length <= buffer.remaining()) {
                "\$operation response contains a truncated optional-value entry"
            }
            val value = ByteArray(length.toInt())
            buffer.get(value)
            values[index] = value
        }
        require(!buffer.hasRemaining()) {
            "\$operation response contains trailing optional-value bytes"
        }
        return values
    }
`
    : ""
  const compatibility_helpers = `${item_id_helpers}${optional_values_helpers}`
  const {
    policy_default_eviction,
    policy_default_expiration,
    policy_default_ttl_milliseconds,
    policy_eviction_override,
    policy_expiration_override,
    set_condition,
    set_eviction_mode,
    set_expiration_mode,
    set_ttl_milliseconds,
    set_value,
  } = structure_convenience_fields(contract, "kotlin")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client

import io.openkache.client.generated_local.SmithyContract
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets

/** Generated operation implementations backed by the shared native contract. */
public interface SmithyGeneratedOperations : SmithyOpenKacheApi {
    public fun smithyInvoke(
        operation: Int,
        applicationKey: ByteArray,
        value: ByteArray,
        setCondition: Int = SmithyContract.SET_CONDITION_ANY,
        ttlMilliseconds: Long = 0,
    ): NativeResult

    public fun smithyInvokeScoped(
        operation: Int,
        namespaceId: Long,
        itemId: ByteArray,
        value: ByteArray,
        flags: Int = 0,
        ttlMilliseconds: Long = 0,
    ): NativeResult

    public fun smithyNamespaceOpen(
        name: ByteArray,
        createIfMissing: Boolean,
        policyFlags: Int,
        ttlMilliseconds: Long,
    ): NativeResult

    public fun smithyNamespaceUpdatePolicy(
        namespaceId: Long,
        expectedRevision: Long,
        policyFlags: Int,
        ttlMilliseconds: Long,
    ): NativeResult

    public fun smithyNamespaceDelete(
        namespaceId: Long,
        expectedRevision: Long,
    ): NativeResult

    public fun smithyDecodeDescriptor(payload: ByteArray): NamespaceDescriptor

    public fun smithyDecodeUtf8(payload: ByteArray, operation: String): String

${f64_array_helpers}

${container_helpers}
${field_sequence_helpers}
${compatibility_helpers}

${methods}

    private fun smithySetFlags(input: SetInput): Pair<Int, Long> {
        var flags = when (input.${set_condition} ?: SetCondition.Any) {
            SetCondition.Any -> SmithyContract.SET_CONDITION_ANY
            SetCondition.IfAbsent -> SmithyContract.SET_CONDITION_IF_ABSENT
            SetCondition.IfPresent -> SmithyContract.SET_CONDITION_IF_PRESENT
        }
        val expiration = input.${set_expiration_mode}
            ?: if (input.${set_ttl_milliseconds} == null) ExpirationMode.Inherit else ExpirationMode.ExplicitTtl
        when (expiration) {
            ExpirationMode.Inherit -> {
                require(input.${set_ttl_milliseconds} == null) { "INHERIT cannot carry a TTL" }
                flags = flags or SmithyContract.SET_INHERIT_EXPIRATION_BITS
            }
            ExpirationMode.NoExpiry -> {
                require(input.${set_ttl_milliseconds} == null) { "NO_EXPIRY cannot carry a TTL" }
                flags = flags or SmithyContract.SET_NO_EXPIRY_BITS
            }
            ExpirationMode.ExplicitTtl -> {
                require(input.${set_ttl_milliseconds} != null && input.${set_ttl_milliseconds} > 0) {
                    "EXPLICIT_TTL requires a positive TTL"
                }
                flags = flags or SmithyContract.SET_EXPLICIT_TTL_BITS
            }
        }
        flags = flags or when (input.${set_eviction_mode} ?: EvictionMode.Inherit) {
            EvictionMode.Inherit -> SmithyContract.SET_INHERIT_EVICTION_BITS
            EvictionMode.Evictable -> SmithyContract.SET_EVICTABLE_BITS
            EvictionMode.EvictionProtected -> SmithyContract.SET_EVICTION_PROTECTED_BITS
        }
        require(input.${set_value}.size <= SmithyContract.MAX_VALUE_BYTES) {
            "value exceeds protocol limit"
        }
        return flags to (input.${set_ttl_milliseconds} ?: 0)
    }

    private fun smithyPolicyFlags(
        policy: NamespacePolicy?,
        required: Boolean,
    ): Pair<Int, Long> {
        if (required) requireNotNull(policy) { "namespace policy is required" }
        if (!required) require(policy == null) { "namespace policy requires createIfMissing" }
        if (policy == null) return 0 to 0
        var flags = when (policy.${policy_default_expiration}) {
            ExpirationDefault.NoExpiry -> SmithyContract.POLICY_NO_EXPIRY_BITS
            ExpirationDefault.FixedTtl -> SmithyContract.POLICY_FIXED_TTL_BITS
        }
        val ttl = policy.${policy_default_ttl_milliseconds} ?: 0
        if (policy.${policy_default_expiration} == ExpirationDefault.FixedTtl) {
            require(ttl > 0) { "FIXED_TTL requires a positive TTL" }
        } else {
            require(ttl == 0L) { "NO_EXPIRY cannot carry a TTL" }
        }
        if (policy.${policy_expiration_override} == OverridePolicy.Allowed) {
            flags = flags or SmithyContract.POLICY_EXPIRATION_OVERRIDE_FLAG
        }
        if (policy.${policy_default_eviction} == EvictionDefault.EvictionProtected) {
            flags = flags or SmithyContract.POLICY_EVICTION_PROTECTED_FLAG
        }
        if (policy.${policy_eviction_override} == OverridePolicy.Allowed) {
            flags = flags or SmithyContract.POLICY_EVICTION_OVERRIDE_FLAG
        }
        return flags to ttl
    }

    private fun smithyRequireKind(
        result: NativeResult,
        expected: Int,
        operation: String,
    ) {
        if (result.kind != expected) {
            throw smithyUnexpectedKind(operation, result.kind)
        }
    }

    private fun smithyUnexpectedKind(operation: String, kind: Int) =
        OpenKacheClientException("$operation returned unexpected native result $kind")
}
`
}

/** Renders the native constants consumed by the Kotlin JNA adapter. */
export function render_kotlin_contract(contract: Client_Contract): string {
  const values = adapter_contract_values(contract)
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated_local

/** Native values shared by the Kotlin adapter and the Rust client-core ABI. */
public object SmithyContract {
    public const val ABI_VERSION: Int = ${values.abi_version}
${values.operations
  .map(
    (entry) =>
      `    public const val OPERATION_${snake_case(entry.name).toUpperCase()}: Int = ${entry.value}`,
  )
  .join("\n")}
${render_kotlin_operation_metadata(values)}
    public const val RESULT_ERROR: Int = ${values.result_error}
    public const val RESULT_OK: Int = ${values.result_ok}
    public const val RESULT_VALUE: Int = ${values.result_value}
    public const val RESULT_NOT_FOUND: Int = ${values.result_not_found}
    public const val RESULT_CREATED: Int = ${values.result_created}
    public const val RESULT_REPLACED: Int = ${values.result_replaced}
    public const val RESULT_DELETED: Int = ${values.result_deleted}
    public const val RESULT_NOT_DELETED: Int = ${values.result_not_deleted}
    public const val RESULT_CONNECTED: Int = ${values.result_connected}
    public const val RESULT_NOT_STORED: Int = ${values.result_not_stored}
    public const val RESULT_RAW: Int = ${values.result_raw}
    public const val DESCRIPTOR_DECODE_OK: Int = ${values.descriptor_decode_ok}
    public const val DEFAULT_EXPIRATION_NO_EXPIRY: Int = ${values.default_expiration_no_expiry}
    public const val DEFAULT_EXPIRATION_FIXED_TTL: Int = ${values.default_expiration_fixed_ttl}
    public const val DEFAULT_EVICTION_EVICTABLE: Int = ${values.default_eviction_evictable}
    public const val DEFAULT_EVICTION_PROTECTED: Int = ${values.default_eviction_protected}
    public const val OVERRIDE_DISALLOWED: Int = ${values.override_disallowed}
    public const val OVERRIDE_ALLOWED: Int = ${values.override_allowed}
    public const val SET_CONDITION_ANY: Int = ${values.set_condition_any}
    public const val SET_CONDITION_IF_ABSENT: Int = ${values.set_condition_if_absent}
    public const val SET_CONDITION_IF_PRESENT: Int = ${values.set_condition_if_present}
    public const val KEY_SPEC_TEXT: Int = ${values.key_spec_text}
    public const val KEY_SPEC_BYTES: Int = ${values.key_spec_bytes}
    public const val KEY_SPEC_INTEGER: Int = ${values.key_spec_integer}
    public const val SET_INHERIT_EXPIRATION_BITS: Int = ${values.set_inherit_expiration}
    public const val SET_NO_EXPIRY_BITS: Int = ${values.set_no_expiry}
    public const val SET_EXPLICIT_TTL_BITS: Int = ${values.set_explicit_ttl}
    public const val SET_INHERIT_EVICTION_BITS: Int = ${values.set_inherit_eviction}
    public const val SET_EVICTABLE_BITS: Int = ${values.set_evictable}
    public const val SET_EVICTION_PROTECTED_BITS: Int = ${values.set_eviction_protected}
    public const val POLICY_NO_EXPIRY_BITS: Int = ${values.policy_no_expiry}
    public const val POLICY_FIXED_TTL_BITS: Int = ${values.policy_fixed_ttl}
    public const val POLICY_EXPIRATION_OVERRIDE_FLAG: Int = ${values.policy_expiration_override}
    public const val POLICY_EVICTION_PROTECTED_FLAG: Int = ${values.policy_eviction_protected}
    public const val POLICY_EVICTION_OVERRIDE_FLAG: Int = ${values.policy_eviction_override}
    public const val ITEM_ID_BYTES: Int = ${values.item_id_bytes}
    public const val NAMESPACE_NAME_MAX_BYTES: Int = ${values.namespace_name_max_bytes}
    public const val MAX_VALUE_BYTES: Int = ${values.max_value_bytes}
    public const val DEFAULT_ZSTANDARD_LEVEL: Int = ${values.default_zstandard_level}
    public const val DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES: Long = ${values.default_zstandard_minimum_input_bytes}L
    public const val DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES: Long = ${values.default_zstandard_minimum_savings_bytes}L
    public const val DEFAULT_CONNECT_TIMEOUT_MILLISECONDS: Long = ${values.default_connect_timeout_milliseconds}L
    public const val DEFAULT_REQUEST_TIMEOUT_MILLISECONDS: Long = ${values.default_request_timeout_milliseconds}L
}
`
}
