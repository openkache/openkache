/** Dart generated operation and contract renderers. */

import type { Client_Contract } from "../../client_contract"
import { lower_camel_case, pascal_case, snake_case } from "../../generator_names"
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
  operation_field_name,
  operation_fields,
  operation_is_global_empty,
  operation_is_global_field_sequence,
  operation_is_global_opaque,
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
  render_dart_container_helpers,
  render_dart_field_sequence_helpers,
  render_dart_operation_metadata,
  render_expression_generic_invocation,
  render_field_sequence_request_payload,
  render_field_sequence_response_decode,
  render_operation_result,
  render_opaque_request_expression,
  structure_convenience_fields,
  type Managed_Api_Operation,
} from "../managed"

function render_dart_operation_method(operation: Managed_Api_Operation): string {
  const operation_constant = managed_operation_constant(operation, "dart")
  const operation_label = managed_operation_label(operation)
  const result_constant = (kind: Operation_Result_Kind): string =>
    operation_result_constant(operation, kind, "dart")
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
  } = operation_convenience_fields(operation, "dart")
  const input_item_ids = operation_item_fields(operation).map(
    (member) => operation_field_name(member, "dart"),
  )
  const input_item_id_expression = input_item_ids.length === 0
    ? "const <int>[]"
    : input_item_ids.length === 1
    ? `input.${input_item_ids[0]!}`
    : `_smithyConcatItemIds([${input_item_ids.map((name) => `input.${name}`).join(", ")}])`
  const application_value_codecs = operation.plan.application_value_codecs
  const input_request_value = operation_request_value_name(operation, "dart")
    ?? "const <int>[]"
  const prefix = `  @override
  Future<${operation.output}> ${method_name}(${operation.input} input) => _run(() {
`
  return render_operation_result(operation, "Dart", {
    raw_payload: () => {
      const output_payload = operation_opaque_field_name(operation, "output", "dart")
      const invocation = render_expression_generic_invocation(
        "dart",
        operation,
        operation_constant,
        `'${operation_label}'`,
      ) ??
        `_invoke(
      ${operation_constant},
      const <int>[],
      const <int>[],
    )`
      return `${prefix}    final result = ${invocation};
    _smithyRequireKind(result, ${result_constant("ok")}, '${operation_label}');
    return ${operation.output}(
      ${output_payload}: result.payload,
    );
      });`
    },
    opaque: () => {
        const input_payload = operation_request_is_opaque(operation)
          ? operation_opaque_field_name(operation, "input", "dart")
          : undefined
        const output_payload = operation_opaque_field_name(operation, "output", "dart")
        const codec = render_application_value_codec(
          "dart",
          application_value_codecs!,
          input_payload === undefined ? "const <int>[]" : `input.${input_payload}`,
          "result.payload",
          `'${operation_label}'`,
        )
        const decoded_payload = codec.decode
        const invocation = render_expression_generic_invocation(
          "dart",
          operation,
          operation_constant,
          `'${operation_label}'`,
        ) ??
          (operation_is_global_empty(operation)
            ? `_invoke(
      ${operation_constant},
      const <int>[],
      const <int>[],
    )`
            : `_invokeScoped(
      ${operation_constant},
      input.${input_namespace_id},
      ${input_item_id_expression},
      ${input_request_value},
    )`)
        return `${prefix}    final result = ${invocation};
    _smithyRequireKind(result, ${result_constant("value")}, '${operation_label}');
    return ${operation.output}(
      ${output_payload}: ${decoded_payload},
    );
  });`
    },
    field_sequence: () => {
        const output_decoded_values = operation_composite_fields(operation)
          .map((field, index) =>
            render_composite_field_decode(
              "dart",
              operation_composite_field_codec(operation, field),
              `values[${index}]`,
              `'${operation_label}'`,
              field.required,
              field.type,
            ),
          )
        const output_expression = render_composite_output(
          operation,
          "dart",
          output_decoded_values,
        )
        const response_values = operation.plan.contract.response_framing === "field_sequence"
          ? render_field_sequence_response_decode(
            "dart",
            operation,
            "result.payload",
            `'${operation_label}'`,
          )
          : `_smithyDecodeOptionalValues(result.payload, ${operation_composite_value_count(operation)}, '${operation_label}')`
        const invocation = render_expression_generic_invocation(
          "dart",
          operation,
          operation_constant,
          `'${operation_label}'`,
        ) ??
          (operation_is_global_empty(operation)
            ? `_invoke(
      ${operation_constant},
      const <int>[],
      const <int>[],
    )`
            : `_invokeScoped(
      ${operation_constant},
      input.${input_namespace_id},
      ${input_item_id_expression},
      ${input_request_value},
    )`)
        return `${prefix}    final result = ${invocation};
    _smithyRequireKind(result, ${result_constant("value")}, '${operation_label}');
    final values = ${response_values};
    return ${output_expression};
  });`
    },
    optional_payload: () => {
      if (operation_field_count(operation.plan.operation, "output", "value") > 1) {
        const output_values = operation_fields(operation, "output", "value")
          .map((member, index) =>
            `${operation_field_name(member, "dart")}: values[${index}]`,
          )
          .join(",\n      ")
        return `${prefix}    final result = _invokeScoped(
      ${operation_constant},
      input.${input_namespace_id},
      ${input_item_id_expression},
      ${input_request_value},
    );
    _smithyRequireKind(result, ${result_constant("value")}, '${operation_label}');
    final values = _smithyDecodeOptionalValues(
      result.payload, ${operation_field_count(operation.plan.operation, "output", "value")}, '${operation_label}');
    return ${operation.output}(
      ${output_values},
    );
  });`
      }
      return `${prefix}    final result = _invokeScoped(
      ${operation_constant},
      input.${input_namespace_id},
      ${input_item_id_expression},
      ${input_request_value},
    );
    if (result.kind == ${result_constant("not_found")}) {
      return const ${operation.output}();
    }
    _smithyRequireKind(result, ${result_constant("value")}, '${operation_label}');
      return ${operation.output}(${output_value}: result.payload);
  });`
    },
    status_outcome: () => {
      return `${prefix}    final flags = _smithySetFlags(input);
    final result = _invokeScoped(
      ${operation_constant},
      input.${input_namespace_id},
      ${input_item_id_expression},
      input.${input_value},
      flags: flags.flags,
      ttlMilliseconds: flags.ttlMilliseconds,
    );
    final outcome = switch (result.kind) {
      ${result_constant("created")} => SetOutcome.created,
      ${result_constant("replaced")} => SetOutcome.replaced,
      ${result_constant("not_stored")} => SetOutcome.notStored,
      _ => throw _smithyUnexpectedKind('${operation_label}', result.kind),
    };
    return ${operation.output}(${output_outcome}: outcome);
  });`
    },
    boolean_outcome: () => {
      return `${prefix}    final result = _invokeScoped(
      ${operation_constant},
      input.${input_namespace_id},
      ${input_item_id_expression},
      const <int>[],
    );
    return switch (result.kind) {
      ${result_constant("deleted")} => const ${operation.output}(${output_deleted}: true),
      ${result_constant("not_deleted")} => const ${operation.output}(${output_deleted}: false),
      _ => throw _smithyUnexpectedKind('${operation_label}', result.kind),
    };
  });`
    },
    text_payload: () => {
      return `${prefix}    final result = _invokeScoped(
      ${operation_constant},
      input.${input_namespace_id},
      const <int>[],
      const <int>[],
    );
    _smithyRequireKind(result, ${result_constant("value")}, '${operation_label}');
    return ${operation.output}(
      ${output_json}: _smithyDecodeUtf8(result.payload, '${operation_label}'),
    );
  });`
    },
    empty: () => {
      const invocation = render_expression_generic_invocation(
          "dart",
          operation,
          operation_constant,
          `'${operation_label}'`,
        )
        if (invocation !== undefined) {
          return `${prefix}    final result = ${invocation};
    _smithyRequireKind(result, ${result_constant("ok")}, '${operation_label}');
    return const ${operation.output}();
  });`
        }
      if (
        operation_is_global_opaque(operation) ||
        operation_is_global_field_sequence(operation)
      ) {
        const request_payload = operation_is_global_opaque(operation)
          ? render_opaque_request_expression("dart", operation, `'${operation_label}'`)
          : render_field_sequence_request_payload(
            "dart",
            operation,
            `'${operation_label}'`,
          )
        return `${prefix}    final result = _invoke(
      ${operation_constant},
      const <int>[],
      ${request_payload},
    );
    _smithyRequireKind(result, ${result_constant("ok")}, '${operation_label}');
    return const ${operation.output}();
  });`
      }
      if (operation_uses_compact_item_request(operation)) {
        const has_request_value = operation_request_value_count(operation) > 0
        const request_value = has_request_value
          ? `input.${input_value}`
          : "const <int>[]"
        if (has_request_value) {
          return `${prefix}    final flags = _smithySetFlags(input);
    final result = _invokeScoped(
      ${operation_constant},
      input.${input_namespace_id},
      ${input_item_id_expression},
      ${request_value},
      flags: flags.flags,
      ttlMilliseconds: flags.ttlMilliseconds,
    );
    _smithyRequireKind(result, ${result_constant("ok")}, '${operation_label}');
    return const ${operation.output}();
  });`
        }
        return `${prefix}    final result = _invokeScoped(
      ${operation_constant},
      input.${input_namespace_id},
      ${input_item_id_expression},
      ${request_value},
    );
    _smithyRequireKind(result, ${result_constant("ok")}, '${operation_label}');
    return const ${operation.output}();
  });`
      }
      if (operation_uses_compact_namespace_request(operation)) {
        return `${prefix}    final result = _invokeScoped(
      ${operation_constant},
      input.${input_namespace_id},
      const <int>[],
      const <int>[],
    );
    _smithyRequireKind(result, ${result_constant("ok")}, '${operation_label}');
    return const ${operation.output}();
  });`
      }
      if (operation_uses_compact_request_route(operation, "namespace_delete")) {
        return `${prefix}    final result = _readResult(
      _api,
      _api.namespaceDelete(
        _requireOpenHandle(),
        input.${input_namespace_id},
        input.${input_expected_revision},
      ),
    );
    _smithyRequireKind(result, ${result_constant("ok")}, '${operation_label}');
    return const ${operation.output}();
  });`
      }
      throw new Error(`unsupported generated Dart empty operation ${operation.name}`)
    },
    descriptor: () => {
      if (operation_uses_compact_request_route(operation, "namespace_open")) {
        return `${prefix}    final name = utf8.encode(input.${input_name});
    if (name.length > smithyNamespaceNameMaxBytes) {
      throw const OpenKacheClientException('namespace name exceeds protocol limit');
    }
    final policy = _smithyPolicyFlags(
      input.${input_policy},
      input.${input_create_if_missing},
    );
    final nameBuffer = _Buffer(name);
    try {
      final result = _readResult(
        _api,
        _api.namespaceOpen(
          _requireOpenHandle(),
          nameBuffer.pointer,
          nameBuffer.length,
          input.${input_create_if_missing} ? 1 : 0,
          policy.flags,
          policy.ttlMilliseconds,
        ),
      );
      final created = result.kind == ${result_constant("created")};
      if (!created && result.kind != ${result_constant("ok")}) {
        throw _smithyUnexpectedKind('${operation_label}', result.kind);
      }
      return ${operation.output}(
        ${output_descriptor}: _decodeDescriptor(result.payload),
        ${output_created}: created,
      );
    } finally {
      nameBuffer.close();
    }
  });`
      }
      if (operation_uses_compact_request_route(operation, "namespace_update_policy")) {
        return `${prefix}    final policy = _smithyPolicyFlags(input.${input_policy}, true);
    final result = _readResult(
      _api,
      _api.namespaceUpdatePolicy(
        _requireOpenHandle(),
        input.${input_namespace_id},
        input.${input_expected_revision},
        policy.flags,
        policy.ttlMilliseconds,
      ),
    );
    _smithyRequireKind(result, ${result_constant("value")}, '${operation_label}');
    return ${operation.output}(
      ${output_descriptor}: _decodeDescriptor(result.payload),
    );
  });`
      }
      throw new Error(`unsupported generated Dart namespace operation ${operation.name}`)
    },
  })
}

export function render_dart_operations(contract: Client_Contract): string {
  const managed_operations = managed_operation_entries(contract)
  const framing = optional_value_framing(contract)
  const field_framing = field_sequence_framing(contract)
  const container_helpers = has_wire_codec(
    managed_operations,
    ["list", "map", "union"],
  )
    ? render_dart_container_helpers(contract.max_value_bytes)
    : ""
  const field_sequence_helpers = managed_operations.some(
    operation_uses_field_sequence_helpers,
  )
    ? render_dart_field_sequence_helpers(field_framing)
    : ""
  const methods = managed_operations
    .map(render_dart_operation_method)
    .join("\n\n")
  const f64_array_helpers = has_application_value_codec(
    managed_operations,
    "packed_f64_be",
  )
    ? `List<int> _smithyEncodeF64Array(List<double> values) {
  final data = ByteData(values.length * 8);
  for (var index = 0; index < values.length; index++) {
    final value = values[index];
    if (!value.isFinite) {
      throw const OpenKacheClientException(
        'binary64 array input must contain finite values',
      );
    }
    data.setFloat64(index * 8, value, Endian.big);
  }
  return data.buffer.asUint8List();
}

List<double> _smithyDecodeF64Array(List<int> payload, String operation) {
  if (payload.length % 8 != 0) {
    throw OpenKacheClientException(
      '$operation response has a malformed binary64 array length',
    );
  }
  final data = ByteData.sublistView(Uint8List.fromList(payload));
  final values = <double>[];
  for (var offset = 0; offset < payload.length; offset += 8) {
    final value = data.getFloat64(offset, Endian.big);
    if (!value.isFinite) {
      throw OpenKacheClientException(
        '$operation response contains a non-finite binary64 value',
      );
    }
    values.add(value);
  }
  return values;
}
`
    : ""
  const item_id_helpers = managed_operations.some(
    operation_uses_item_id_helpers,
  )
    ? `List<int> _smithyConcatItemIds(List<List<int>> itemIds) {
  for (final itemId in itemIds) {
    if (itemId.length != smithyItemIdBytes) {
      throw ArgumentError('item IDs must contain exactly \$smithyItemIdBytes bytes');
    }
  }
  return <int>[for (final itemId in itemIds) ...itemId];
}
`
    : ""
  const optional_values_helpers = managed_operations.some(
    operation_uses_optional_value_layout,
  )
    ? `List<List<int>?> _smithyDecodeOptionalValues(
  List<int> payload,
  int valueCount,
  String operation,
) {
  final data = ByteData.sublistView(Uint8List.fromList(payload));
  final values = <List<int>?>[];
  var offset = 0;
  for (var index = 0; index < valueCount; index++) {
    if (offset + ${framing.length_bytes} > payload.length) {
      throw OpenKacheClientException(
        '\$operation response is missing an optional-value length',
      );
    }
    final length = data.getUint32(offset, Endian.big);
    offset += ${framing.length_bytes};
    if (length == ${framing.missing_sentinel}) {
      values.add(null);
      continue;
    }
    if (length > ${framing.max_value_bytes}) {
      throw OpenKacheClientException(
        '\$operation response optional-value entry exceeds the maximum value size',
      );
    }
    if (length > payload.length - offset) {
      throw OpenKacheClientException(
        '\$operation response contains a truncated optional-value entry',
      );
    }
    values.add(payload.sublist(offset, offset + length));
    offset += length;
  }
  if (offset != payload.length) {
    throw OpenKacheClientException(
      '\$operation response contains trailing optional-value bytes',
    );
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
  } = structure_convenience_fields(contract, "dart")
  return `// Generated from the OpenKache Smithy contract. Do not edit.
part of '../openkache.dart';
// The generated hook keeps optional arguments aligned with the native ABI.
// ignore_for_file: unused_element_parameter, unused_element

/// Generated operation implementations backed by the shared native contract.
mixin SmithyGeneratedOperations implements SmithyOpenKacheApi {
  Future<T> _run<T>(T Function() operation);

  _NativeResult _invoke(
    int operation,
    List<int> applicationKey,
    List<int> value, {
    int setCondition = smithySetConditionAny,
    int ttlMilliseconds = 0,
  });

  _NativeResult _invokeScoped(
    int operation,
    int namespaceId,
    List<int> itemId,
    List<int> value, {
    int flags = 0,
    int ttlMilliseconds = 0,
  });

  SmithyNativeApi get _api;

  ffi.Pointer<SmithyNativeClient> _requireOpenHandle();

  NamespaceDescriptor _decodeDescriptor(List<int> payload);

${methods}
}

_SmithySetFlags _smithySetFlags(SetInput input) {
  var flags = switch (input.${set_condition} ?? SetCondition.any) {
    SetCondition.any => smithySetConditionAny,
    SetCondition.ifAbsent => smithySetConditionIfAbsent,
    SetCondition.ifPresent => smithySetConditionIfPresent,
  };
  final expiration =
      input.${set_expiration_mode} ??
      (input.${set_ttl_milliseconds} == null
          ? ExpirationMode.inherit
          : ExpirationMode.explicitTtl);
  switch (expiration) {
    case ExpirationMode.inherit:
      if (input.${set_ttl_milliseconds} != null) {
        throw ArgumentError('INHERIT cannot carry a TTL');
      }
      flags |= smithySetInheritExpirationBits;
    case ExpirationMode.noExpiry:
      if (input.${set_ttl_milliseconds} != null) {
        throw ArgumentError('NO_EXPIRY cannot carry a TTL');
      }
      flags |= smithySetNoExpiryBits;
    case ExpirationMode.explicitTtl:
      if (input.${set_ttl_milliseconds} == null || input.${set_ttl_milliseconds}! <= 0) {
        throw ArgumentError('EXPLICIT_TTL requires a positive TTL');
      }
      flags |= smithySetExplicitTtlBits;
  }
  flags |= switch (input.${set_eviction_mode} ?? EvictionMode.inherit) {
    EvictionMode.inherit => smithySetInheritEvictionBits,
    EvictionMode.evictable => smithySetEvictableBits,
    EvictionMode.evictionProtected => smithySetEvictionProtectedBits,
  };
  if (input.${set_value}.length > smithyMaxValueBytes) {
    throw ArgumentError('value exceeds protocol limit');
  }
  return _SmithySetFlags(flags, input.${set_ttl_milliseconds} ?? 0);
}

_SmithyPolicyFlags _smithyPolicyFlags(
  NamespacePolicy? policy,
  bool required,
) {
  if (required && policy == null) {
    throw ArgumentError('namespace policy is required');
  }
  if (!required && policy != null) {
    throw ArgumentError('namespace policy requires createIfMissing');
  }
  if (policy == null) return const _SmithyPolicyFlags(0, 0);
  var flags = switch (policy.${policy_default_expiration}) {
    ExpirationDefault.noExpiry => smithyPolicyNoExpiryBits,
    ExpirationDefault.fixedTtl => smithyPolicyFixedTtlBits,
  };
  final ttl = policy.${policy_default_ttl_milliseconds} ?? 0;
  if (policy.${policy_default_expiration} == ExpirationDefault.fixedTtl) {
    if (ttl <= 0) throw ArgumentError('FIXED_TTL requires a positive TTL');
  } else if (ttl != 0) {
    throw ArgumentError('NO_EXPIRY cannot carry a TTL');
  }
  if (policy.${policy_expiration_override} == OverridePolicy.allowed) {
    flags |= smithyPolicyExpirationOverrideFlag;
  }
  if (policy.${policy_default_eviction} == EvictionDefault.evictionProtected) {
    flags |= smithyPolicyEvictionProtectedFlag;
  }
  if (policy.${policy_eviction_override} == OverridePolicy.allowed) {
    flags |= smithyPolicyEvictionOverrideFlag;
  }
  return _SmithyPolicyFlags(flags, ttl);
}

String _smithyDecodeUtf8(List<int> payload, String operation) {
  try {
    return utf8.decode(payload, allowMalformed: false);
  } on FormatException catch (error) {
    throw OpenKacheClientException('$operation response is not valid UTF-8', error);
  }
}

${f64_array_helpers}

${container_helpers}
${field_sequence_helpers}
${compatibility_helpers}

void _smithyRequireKind(_NativeResult result, int expected, String operation) {
  if (result.kind != expected) {
    throw _smithyUnexpectedKind(operation, result.kind);
  }
}

OpenKacheClientException _smithyUnexpectedKind(String operation, int kind) =>
    OpenKacheClientException('$operation returned unexpected native result $kind');

final class _SmithySetFlags {
  const _SmithySetFlags(this.flags, this.ttlMilliseconds);

  final int flags;
  final int ttlMilliseconds;
}

final class _SmithyPolicyFlags {
  const _SmithyPolicyFlags(this.flags, this.ttlMilliseconds);

  final int flags;
  final int ttlMilliseconds;
}
`
}
/** Renders the native constants consumed by the Dart FFI adapter. */
export function render_dart_contract(contract: Client_Contract): string {
  const values = adapter_contract_values(contract)
  return `// Generated from the OpenKache Smithy contract. Do not edit.

/// Native values shared by the Dart adapter and the Rust client-core ABI.
const int smithyFfiAbiVersion = ${values.abi_version};
${values.operations
  .map(
    (entry) =>
      `const int smithyOperation${pascal_case(snake_case(entry.name))} = ${entry.value};`,
  )
  .join("\n")}
${render_dart_operation_metadata(values)}
const int smithyResultError = ${values.result_error};
const int smithyResultOk = ${values.result_ok};
const int smithyResultValue = ${values.result_value};
const int smithyResultNotFound = ${values.result_not_found};
const int smithyResultCreated = ${values.result_created};
const int smithyResultReplaced = ${values.result_replaced};
const int smithyResultDeleted = ${values.result_deleted};
const int smithyResultNotDeleted = ${values.result_not_deleted};
const int smithyResultConnected = ${values.result_connected};
const int smithyResultNotStored = ${values.result_not_stored};
const int smithyResultRaw = ${values.result_raw};
const int smithyDescriptorDecodeOk = ${values.descriptor_decode_ok};
const int smithyDefaultExpirationNoExpiry = ${values.default_expiration_no_expiry};
const int smithyDefaultExpirationFixedTtl = ${values.default_expiration_fixed_ttl};
const int smithyDefaultEvictionEvictable = ${values.default_eviction_evictable};
const int smithyDefaultEvictionProtected = ${values.default_eviction_protected};
const int smithyOverrideDisallowed = ${values.override_disallowed};
const int smithyOverrideAllowed = ${values.override_allowed};
const int smithySetConditionAny = ${values.set_condition_any};
const int smithySetConditionIfAbsent = ${values.set_condition_if_absent};
const int smithySetConditionIfPresent = ${values.set_condition_if_present};
const int smithyKeySpecText = ${values.key_spec_text};
const int smithyKeySpecBytes = ${values.key_spec_bytes};
const int smithyKeySpecInteger = ${values.key_spec_integer};
const int smithySetInheritExpirationBits = ${values.set_inherit_expiration};
const int smithySetNoExpiryBits = ${values.set_no_expiry};
const int smithySetExplicitTtlBits = ${values.set_explicit_ttl};
const int smithySetInheritEvictionBits = ${values.set_inherit_eviction};
const int smithySetEvictableBits = ${values.set_evictable};
const int smithySetEvictionProtectedBits = ${values.set_eviction_protected};
const int smithyPolicyNoExpiryBits = ${values.policy_no_expiry};
const int smithyPolicyFixedTtlBits = ${values.policy_fixed_ttl};
const int smithyPolicyExpirationOverrideFlag = ${values.policy_expiration_override};
const int smithyPolicyEvictionProtectedFlag = ${values.policy_eviction_protected};
const int smithyPolicyEvictionOverrideFlag = ${values.policy_eviction_override};
const int smithyItemIdBytes = ${values.item_id_bytes};
const int smithyNamespaceNameMaxBytes = ${values.namespace_name_max_bytes};
const int smithyMaxValueBytes = ${values.max_value_bytes};
const int smithyDefaultZstandardLevel = ${values.default_zstandard_level};
const int smithyDefaultZstandardMinimumInputBytes =
    ${values.default_zstandard_minimum_input_bytes};
const int smithyDefaultZstandardMinimumSavingsBytes =
    ${values.default_zstandard_minimum_savings_bytes};
const int smithyDefaultConnectTimeoutMilliseconds =
    ${values.default_connect_timeout_milliseconds};
const int smithyDefaultRequestTimeoutMilliseconds =
    ${values.default_request_timeout_milliseconds};
`
}
