/** Java generated operation and contract renderers. */

import type { Client_Contract } from "../../client_contract"
import { lower_camel_case, snake_case } from "../../generator_names"
import { operation_field_count } from "../../operation_plans"
import type { Operation_Result_Kind } from "../../compatibility_result_projections"
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
  operation_empty_result_constant,
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
  operation_uses_optional_value_layout,
  optional_value_framing,
  render_application_value_codec,
  render_composite_field_decode,
  render_composite_output,
  render_expression_generic_invocation,
  render_field_sequence_response_decode,
  render_java_container_helpers,
  render_java_field_sequence_helpers,
  render_java_operation_metadata,
  render_operation_result,
  structure_convenience_fields,
  type Managed_Api_Operation,
} from "../managed"

function render_java_operation_method(operation: Managed_Api_Operation): string {
  const operation_constant = managed_operation_constant(operation, "java")
  const operation_label = managed_operation_label(operation)
  const result_constant = (kind: Operation_Result_Kind): string =>
    operation_result_constant(operation, kind, "java")
  const empty_result_constant = operation_empty_result_constant(operation, "java")
  const method_name = lower_camel_case(operation.name)
  const {
    input_create_if_missing,
    input_expected_revision,
    input_name,
    input_namespace_id,
    input_policy,
    input_value,
  } = operation_convenience_fields(operation, "java")
  const input_item_ids = operation_item_fields(operation).map(
    (member) => operation_field_name(member, "java"),
  )
  const input_item_id_expression = input_item_ids.length === 0
    ? "new byte[0]"
    : input_item_ids.length === 1
    ? `input.${input_item_ids[0]!}()`
    : `smithyConcatItemIds(${input_item_ids.map((name) => `input.${name}()`).join(", ")})`
  const application_value_codecs = operation.plan.application_value_codecs
  const input_request_value = operation_request_value_name(operation, "java")
    ?? "new byte[0]"
  return render_operation_result(operation, "Java", {
    raw_payload: () => {
      const invocation = render_expression_generic_invocation(
        "java",
        operation,
        operation_constant,
        `"${operation_label}"`,
      ) ??
        `smithyExecute(
                ${operation_constant},
                new byte[0],
                new byte[0],
                0,
                0)`
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = ${invocation};
            smithyRequireKind(result, ${result_constant("ok")}, "${operation_label}");
            return new ${operation.output}(result.payload());
        });
    }`
    },
    opaque: () => {
      const input_payload = operation_request_is_opaque(operation)
          ? operation_opaque_field_name(operation, "input", "java")
          : undefined
        const codec = render_application_value_codec(
          "java",
          application_value_codecs!,
          input_payload === undefined ? "new byte[0]" : `input.${input_payload}()`,
          "result.payload()",
          `"${operation_label}"`,
        )
        const decoded_payload = codec.decode
        const invocation = render_expression_generic_invocation(
          "java",
          operation,
          operation_constant,
          `"${operation_label}"`,
        ) ??
          (operation_is_global_empty(operation)
            ? `smithyExecute(
                ${operation_constant},
                new byte[0],
                new byte[0],
                0,
                0)`
            : `smithyExecuteScoped(
                ${operation_constant},
                input.${input_namespace_id}(),
                ${input_item_id_expression},
                ${input_request_value},
                0,
                0)`)
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = ${invocation};
            smithyRequireKind(result, ${result_constant("value")}, "${operation_label}");
            return new ${operation.output}(
                ${decoded_payload});
        });
    }`
    },
    field_sequence: () => {
      const output_decoded_values = operation_composite_fields(operation)
          .map((field, index) =>
            render_composite_field_decode(
              "java",
              operation_composite_field_codec(operation, field),
              `values[${index}]`,
              `"${operation_label}"`,
              field.required,
              field.type,
            ),
          )
        const output_expression = render_composite_output(
          operation,
          "java",
          output_decoded_values,
        )
        const response_values = operation.plan.contract.response_framing === "field_sequence"
          ? render_field_sequence_response_decode(
            "java",
            operation,
            "result.payload()",
            `"${operation_label}"`,
          )
          : `smithyDecodeOptionalValues(result.payload(), ${operation_composite_value_count(operation)}, "${operation_label}")`
        const invocation = render_expression_generic_invocation(
          "java",
          operation,
          operation_constant,
          `"${operation_label}"`,
        ) ??
          (operation_is_global_empty(operation)
            ? `smithyExecute(
                ${operation_constant},
                new byte[0],
                new byte[0],
                0,
                0)`
            : `smithyExecuteScoped(
                ${operation_constant},
                input.${input_namespace_id}(),
                ${input_item_id_expression},
                ${input_request_value},
                0,
                0)`)
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = ${invocation};
            smithyRequireKind(result, ${result_constant("value")}, "${operation_label}");
            byte[][] values = ${response_values};
            return ${output_expression};
        });
    }`
      },
    optional_payload: () => {
      if (operation_field_count(operation.plan.operation, "output", "value") > 1) {
        const output_values = operation_fields(operation, "output", "value")
          .map((_member, index) => `values[${index}]`)
          .join(", ")
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.${input_namespace_id}(),
                ${input_item_id_expression},
                ${input_request_value},
                0,
                0);
            smithyRequireKind(result, ${result_constant("value")}, "${operation_label}");
            byte[][] values = smithyDecodeOptionalValues(
                result.payload(), ${operation_field_count(operation.plan.operation, "output", "value")}, "${operation_label}");
            return new ${operation.output}(${output_values});
        });
    }`
      }
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.${input_namespace_id}(),
                ${input_item_id_expression},
                ${input_request_value},
                0,
                0);
            if (result.kind() == ${result_constant("not_found")}) {
                return new ${operation.output}(null);
            }
            smithyRequireKind(result, ${result_constant("value")}, "${operation_label}");
            return new ${operation.output}(result.payload());
        });
    }`
    },
    status_outcome: () => {
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            SmithySetFlags flags = smithySetFlags(input);
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.${input_namespace_id}(),
                ${input_item_id_expression},
                input.${input_value}(),
                flags.flags(),
                flags.ttlMilliseconds());
            SetOutcome outcome = switch (result.kind()) {
                case ${result_constant("created")} -> SetOutcome.CREATED;
                case ${result_constant("replaced")} -> SetOutcome.REPLACED;
                case ${result_constant("not_stored")} -> SetOutcome.NOT_STORED;
                default -> throw smithyUnexpectedKind("${operation_label}", result.kind());
            };
            return new ${operation.output}(outcome);
        });
    }`
    },
    boolean_outcome: () => {
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.${input_namespace_id}(),
                ${input_item_id_expression},
                new byte[0],
                0,
                0);
            if (result.kind() == ${result_constant("deleted")}) {
                return new ${operation.output}(true);
            }
            if (result.kind() == ${result_constant("not_deleted")}) {
                return new ${operation.output}(false);
            }
            throw smithyUnexpectedKind("${operation_label}", result.kind());
        });
    }`
    },
    text_payload: () => {
      return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.${input_namespace_id}(),
                new byte[0],
                new byte[0],
                0,
                0);
            smithyRequireKind(result, ${result_constant("value")}, "${operation_label}");
            return new ${operation.output}(
                smithyDecodeUtf8(result.payload(), "${operation_label}"));
        });
    }`
    },
    empty: () => {
      const invocation = render_expression_generic_invocation(
          "java",
          operation,
          operation_constant,
          `"${operation_label}"`,
        )
        if (invocation !== undefined) {
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = ${invocation};
            smithyRequireKind(result, ${empty_result_constant}, "${operation_label}");
            return new ${operation.output}();
        });
    }`
        }
      if (operation_uses_compact_item_request(operation)) {
        const has_request_value = operation_request_value_count(operation) > 0
        const request_value = has_request_value
          ? `input.${input_value}()`
          : "new byte[0]"
        if (has_request_value) {
          return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            SmithySetFlags flags = smithySetFlags(input);
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.${input_namespace_id}(),
                ${input_item_id_expression},
                ${request_value},
                flags.flags(),
                flags.ttlMilliseconds());
            smithyRequireKind(result, ${empty_result_constant}, "${operation_label}");
            return new ${operation.output}();
        });
    }`
        }
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.${input_namespace_id}(),
                ${input_item_id_expression},
                ${request_value},
                0,
                0);
            smithyRequireKind(result, ${empty_result_constant}, "${operation_label}");
            return new ${operation.output}();
        });
    }`
      }
      if (operation_uses_compact_namespace_request(operation)) {
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyExecuteScoped(
                ${operation_constant},
                input.${input_namespace_id}(),
                new byte[0],
                new byte[0],
                0,
                0);
            smithyRequireKind(result, ${empty_result_constant}, "${operation_label}");
            return new ${operation.output}();
        });
    }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_delete")) {
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            NativeResult result = smithyNamespaceDelete(
                input.${input_namespace_id}(),
                input.${input_expected_revision}());
            smithyRequireKind(result, ${empty_result_constant}, "${operation_label}");
            return new ${operation.output}();
        });
    }`
      }
      throw new Error(`unsupported generated Java empty operation ${operation.name}`)
    },
    descriptor: () => {
      if (operation_uses_compact_request_route(operation, "namespace_open")) {
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            byte[] name = input.${input_name}().getBytes(StandardCharsets.UTF_8);
            if (name.length > SmithyContract.NAMESPACE_NAME_MAX_BYTES) {
                throw new OpenKacheClientException("namespace name exceeds protocol limit");
            }
            SmithyPolicyFlags policy = smithyPolicyFlags(
                input.${input_policy}(),
                input.${input_create_if_missing}());
            NativeResult result = smithyNamespaceOpen(
                name,
                input.${input_create_if_missing}(),
                policy.flags(),
                policy.ttlMilliseconds());
            boolean created = result.kind() == ${result_constant("created")};
            if (!created && result.kind() != ${result_constant("ok")}) {
                throw smithyUnexpectedKind("${operation_label}", result.kind());
            }
            return new ${operation.output}(
                smithyDecodeDescriptor(result.payload()),
                created);
        });
    }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_update_policy")) {
        return `    @Override
    default CompletionStage<${operation.output}> ${method_name}(${operation.input} input) {
        Objects.requireNonNull(input, "input");
        return smithySubmit(() -> {
            SmithyPolicyFlags policy = smithyPolicyFlags(input.${input_policy}(), true);
            NativeResult result = smithyNamespaceUpdatePolicy(
                input.${input_namespace_id}(),
                input.${input_expected_revision}(),
                policy.flags(),
                policy.ttlMilliseconds());
            smithyRequireKind(result, ${result_constant("value")}, "${operation_label}");
            return new ${operation.output}(smithyDecodeDescriptor(result.payload()));
        });
    }`
      }
      throw new Error(`unsupported generated Java namespace operation ${operation.name}`)
    },
  })
}

export function render_java_operations(contract: Client_Contract): string {
  const managed_operations = managed_operation_entries(contract)
  const framing = optional_value_framing(contract)
  const field_framing = field_sequence_framing(contract)
  const container_helpers = has_wire_codec(
    managed_operations,
    ["list", "map", "union"],
  )
    ? render_java_container_helpers(contract.max_value_bytes)
    : ""
  const field_sequence_helpers = managed_operations.some(
    operation_uses_field_sequence_helpers,
  )
    ? render_java_field_sequence_helpers(field_framing)
    : ""
  const methods = managed_operations
    .map(render_java_operation_method)
    .join("\n\n")
  const f64_array_helpers = has_application_value_codec(
    managed_operations,
    "packed_f64_be",
  )
    ? `    private static byte[] smithyEncodeF64Array(double[] values) {
        ByteBuffer buffer = ByteBuffer.allocate(values.length * Double.BYTES)
            .order(ByteOrder.BIG_ENDIAN);
        for (double value : values) {
            if (!Double.isFinite(value)) {
                throw new IllegalArgumentException("binary64 array input must contain finite values");
            }
            buffer.putDouble(value);
        }
        return buffer.array();
    }

    private static double[] smithyDecodeF64Array(byte[] payload, String operation) {
        if ((payload.length % Double.BYTES) != 0) {
            throw new OpenKacheClientException(
                operation + " response has a malformed binary64 array length");
        }
        ByteBuffer buffer = ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN);
        double[] values = new double[payload.length / Double.BYTES];
        for (int index = 0; index < values.length; index++) {
            values[index] = buffer.getDouble();
            if (!Double.isFinite(values[index])) {
                throw new OpenKacheClientException(
                    operation + " response contains a non-finite binary64 value");
            }
        }
        return values;
    }
`
    : ""
  const item_id_helpers = managed_operations.some(
    operation_uses_item_id_helpers,
  )
    ? `    private static byte[] smithyConcatItemIds(byte[]... itemIds) {
        int total = 0;
        for (byte[] itemId : itemIds) {
            Objects.requireNonNull(itemId, "item ID");
            if (itemId.length > SmithyContract.ITEM_ID_BYTES) {
                throw new OpenKacheClientException("item IDs must contain at most "
                    + SmithyContract.ITEM_ID_BYTES + " bytes");
            }
            if (total > SmithyContract.ITEM_ID_BYTES - itemId.length) {
                throw new OpenKacheClientException("combined item IDs must contain at most "
                    + SmithyContract.ITEM_ID_BYTES + " bytes");
            }
            total += itemId.length;
        }
        byte[] combined = new byte[total];
        int offset = 0;
        for (byte[] itemId : itemIds) {
            System.arraycopy(itemId, 0, combined, offset, itemId.length);
            offset += itemId.length;
        }
        return combined;
    }

`
    : ""
  const optional_values_helpers = managed_operations.some(
    operation_uses_optional_value_layout,
  )
    ? `    private static byte[][] smithyDecodeOptionalValues(
        byte[] payload,
        int valueCount,
        String operation) {
        ByteBuffer buffer = ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN);
        byte[][] values = new byte[valueCount][];
        for (int index = 0; index < values.length; index++) {
            if (buffer.remaining() < ${framing.length_bytes}) {
                throw new OpenKacheClientException(
                    operation + " response is missing an optional-value length");
            }
            long length = Integer.toUnsignedLong(buffer.getInt());
            if (length == ${framing.missing_sentinel}L) {
                values[index] = null;
                continue;
            }
            if (length > ${framing.max_value_bytes}L) {
                throw new OpenKacheClientException(
                    operation + " response optional-value entry exceeds the maximum value size");
            }
            if (length > buffer.remaining()) {
                throw new OpenKacheClientException(
                    operation + " response contains a truncated optional-value entry");
            }
            byte[] value = new byte[(int) length];
            buffer.get(value);
            values[index] = value;
        }
        if (buffer.hasRemaining()) {
            throw new OpenKacheClientException(
                operation + " response contains trailing optional-value bytes");
        }
        return values;
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
  } = structure_convenience_fields(contract, "java")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client;

import io.openkache.client.generated_local.SmithyContract;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Objects;
import java.util.concurrent.CompletionStage;
import java.util.function.Supplier;

/** Generated operation implementations backed by the shared native contract. */
public interface SmithyGeneratedOperations extends SmithyOpenKacheApi {
    record SmithySetFlags(int flags, long ttlMilliseconds) {}

    record SmithyPolicyFlags(int flags, long ttlMilliseconds) {}

    <T> CompletionStage<T> smithySubmit(Supplier<T> operation);

    NativeResult smithyExecute(
        int operation,
        byte[] applicationKey,
        byte[] value,
        int setCondition,
        long ttlMilliseconds);

    NativeResult smithyExecuteScoped(
        int operation,
        long namespaceId,
        byte[] itemId,
        byte[] value,
        int flags,
        long ttlMilliseconds);

    NativeResult smithyNamespaceOpen(
        byte[] name,
        boolean createIfMissing,
        int policyFlags,
        long ttlMilliseconds);

    NativeResult smithyNamespaceUpdatePolicy(
        long namespaceId,
        long expectedRevision,
        int policyFlags,
        long ttlMilliseconds);

    NativeResult smithyNamespaceDelete(long namespaceId, long expectedRevision);

    NamespaceDescriptor smithyDecodeDescriptor(byte[] payload);

    String smithyDecodeUtf8(byte[] payload, String operation);

${f64_array_helpers}
${container_helpers}
${field_sequence_helpers}
${compatibility_helpers}

    ${methods}

    private static SmithySetFlags smithySetFlags(SetInput input) {
        int flags = switch (input.${set_condition}() == null ? SetCondition.ANY : input.${set_condition}()) {
            case ANY -> SmithyContract.SET_CONDITION_ANY;
            case IF_ABSENT -> SmithyContract.SET_CONDITION_IF_ABSENT;
            case IF_PRESENT -> SmithyContract.SET_CONDITION_IF_PRESENT;
        };
        ExpirationMode expiration = input.${set_expiration_mode}()
            == null
            ? (input.${set_ttl_milliseconds}() == null
                ? ExpirationMode.INHERIT
                : ExpirationMode.EXPLICIT_TTL)
            : input.${set_expiration_mode}();
        switch (expiration) {
            case INHERIT -> {
                if (input.${set_ttl_milliseconds}() != null) {
                    throw new IllegalArgumentException("INHERIT cannot carry a TTL");
                }
                flags |= SmithyContract.SET_INHERIT_EXPIRATION_BITS;
            }
            case NO_EXPIRY -> {
                if (input.${set_ttl_milliseconds}() != null) {
                    throw new IllegalArgumentException("NO_EXPIRY cannot carry a TTL");
                }
                flags |= SmithyContract.SET_NO_EXPIRY_BITS;
            }
            case EXPLICIT_TTL -> {
                if (input.${set_ttl_milliseconds}() == null || input.${set_ttl_milliseconds}() <= 0) {
                    throw new IllegalArgumentException("EXPLICIT_TTL requires a positive TTL");
                }
                flags |= SmithyContract.SET_EXPLICIT_TTL_BITS;
            }
        }
        flags |= switch (input.${set_eviction_mode}() == null
            ? EvictionMode.INHERIT
            : input.${set_eviction_mode}()) {
            case INHERIT -> SmithyContract.SET_INHERIT_EVICTION_BITS;
            case EVICTABLE -> SmithyContract.SET_EVICTABLE_BITS;
            case EVICTION_PROTECTED -> SmithyContract.SET_EVICTION_PROTECTED_BITS;
        };
        if (input.${set_value}().length > SmithyContract.MAX_VALUE_BYTES) {
            throw new IllegalArgumentException("value exceeds protocol limit");
        }
        return new SmithySetFlags(
            flags,
            input.${set_ttl_milliseconds}() == null ? 0 : input.${set_ttl_milliseconds}());
    }

    private static SmithyPolicyFlags smithyPolicyFlags(
        NamespacePolicy policy,
        boolean required) {
        if (required && policy == null) {
            throw new IllegalArgumentException("namespace policy is required");
        }
        if (!required && policy != null) {
            throw new IllegalArgumentException("namespace policy requires createIfMissing");
        }
        if (policy == null) {
            return new SmithyPolicyFlags(0, 0);
        }
        int flags = switch (policy.${policy_default_expiration}()) {
            case NO_EXPIRY -> SmithyContract.POLICY_NO_EXPIRY_BITS;
            case FIXED_TTL -> SmithyContract.POLICY_FIXED_TTL_BITS;
        };
        long ttl = policy.${policy_default_ttl_milliseconds}() == null
            ? 0
            : policy.${policy_default_ttl_milliseconds}();
        if (policy.${policy_default_expiration}() == ExpirationDefault.FIXED_TTL && ttl <= 0) {
            throw new IllegalArgumentException("FIXED_TTL requires a positive TTL");
        }
        if (policy.${policy_default_expiration}() == ExpirationDefault.NO_EXPIRY && ttl != 0) {
            throw new IllegalArgumentException("NO_EXPIRY cannot carry a TTL");
        }
        if (policy.${policy_expiration_override}() == OverridePolicy.ALLOWED) {
            flags |= SmithyContract.POLICY_EXPIRATION_OVERRIDE_FLAG;
        }
        if (policy.${policy_default_eviction}() == EvictionDefault.EVICTION_PROTECTED) {
            flags |= SmithyContract.POLICY_EVICTION_PROTECTED_FLAG;
        }
        if (policy.${policy_eviction_override}() == OverridePolicy.ALLOWED) {
            flags |= SmithyContract.POLICY_EVICTION_OVERRIDE_FLAG;
        }
        return new SmithyPolicyFlags(flags, ttl);
    }

    private static void smithyRequireKind(
        NativeResult result,
        int expected,
        String operation) {
        if (result.kind() != expected) {
            throw smithyUnexpectedKind(operation, result.kind());
        }
    }

    private static OpenKacheClientException smithyUnexpectedKind(String operation, int kind) {
        return new OpenKacheClientException(
            operation + " returned unexpected native result " + kind);
    }
}
`
}

/** Renders the native constants consumed by the Java JNA adapter. */
export function render_java_contract(contract: Client_Contract): string {
  const values = adapter_contract_values(contract)
  return `// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated_local;

/** Native values shared by the Java adapter and the Rust client-core ABI. */
public final class SmithyContract {
    private SmithyContract() {}

    public static final int ABI_VERSION = ${values.abi_version};
${values.operations
  .map(
    (entry) =>
      `    public static final int OPERATION_${snake_case(entry.name).toUpperCase()} = ${entry.value};`,
  )
  .join("\n")}
${render_java_operation_metadata(values)}
    public static final int RESULT_ERROR = ${values.result_error};
    public static final int RESULT_OK = ${values.result_ok};
    public static final int RESULT_VALUE = ${values.result_value};
    public static final int RESULT_NOT_FOUND = ${values.result_not_found};
    public static final int RESULT_CREATED = ${values.result_created};
    public static final int RESULT_REPLACED = ${values.result_replaced};
    public static final int RESULT_DELETED = ${values.result_deleted};
    public static final int RESULT_NOT_DELETED = ${values.result_not_deleted};
    public static final int RESULT_CONNECTED = ${values.result_connected};
    public static final int RESULT_NOT_STORED = ${values.result_not_stored};
    public static final int RESULT_RAW = ${values.result_raw};
    public static final int DESCRIPTOR_DECODE_OK = ${values.descriptor_decode_ok};
    public static final int DEFAULT_EXPIRATION_NO_EXPIRY = ${values.default_expiration_no_expiry};
    public static final int DEFAULT_EXPIRATION_FIXED_TTL = ${values.default_expiration_fixed_ttl};
    public static final int DEFAULT_EVICTION_EVICTABLE = ${values.default_eviction_evictable};
    public static final int DEFAULT_EVICTION_PROTECTED = ${values.default_eviction_protected};
    public static final int OVERRIDE_DISALLOWED = ${values.override_disallowed};
    public static final int OVERRIDE_ALLOWED = ${values.override_allowed};
    public static final int SET_CONDITION_ANY = ${values.set_condition_any};
    public static final int SET_CONDITION_IF_ABSENT = ${values.set_condition_if_absent};
    public static final int SET_CONDITION_IF_PRESENT = ${values.set_condition_if_present};
    public static final int KEY_SPEC_TEXT = ${values.key_spec_text};
    public static final int KEY_SPEC_BYTES = ${values.key_spec_bytes};
    public static final int KEY_SPEC_INTEGER = ${values.key_spec_integer};
    public static final int SET_INHERIT_EXPIRATION_BITS = ${values.set_inherit_expiration};
    public static final int SET_NO_EXPIRY_BITS = ${values.set_no_expiry};
    public static final int SET_EXPLICIT_TTL_BITS = ${values.set_explicit_ttl};
    public static final int SET_INHERIT_EVICTION_BITS = ${values.set_inherit_eviction};
    public static final int SET_EVICTABLE_BITS = ${values.set_evictable};
    public static final int SET_EVICTION_PROTECTED_BITS = ${values.set_eviction_protected};
    public static final int POLICY_NO_EXPIRY_BITS = ${values.policy_no_expiry};
    public static final int POLICY_FIXED_TTL_BITS = ${values.policy_fixed_ttl};
    public static final int POLICY_EXPIRATION_OVERRIDE_FLAG = ${values.policy_expiration_override};
    public static final int POLICY_EVICTION_PROTECTED_FLAG = ${values.policy_eviction_protected};
    public static final int POLICY_EVICTION_OVERRIDE_FLAG = ${values.policy_eviction_override};
    public static final int ITEM_ID_BYTES = ${values.item_id_bytes};
    public static final int NAMESPACE_NAME_MAX_BYTES = ${values.namespace_name_max_bytes};
    public static final int MAX_VALUE_BYTES = ${values.max_value_bytes};
    public static final int DEFAULT_ZSTANDARD_LEVEL = ${values.default_zstandard_level};
    public static final long DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES = ${values.default_zstandard_minimum_input_bytes}L;
    public static final long DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES = ${values.default_zstandard_minimum_savings_bytes}L;
    public static final long DEFAULT_CONNECT_TIMEOUT_MILLISECONDS = ${values.default_connect_timeout_milliseconds}L;
    public static final long DEFAULT_REQUEST_TIMEOUT_MILLISECONDS = ${values.default_request_timeout_milliseconds}L;
}
`
}
