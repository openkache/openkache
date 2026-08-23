/** Python API, contract, and operation renderers. */

import type { Api_Type } from "../../operation_models"
import type { Client_Contract } from "../../client_contract"
import { pascal_case, snake_case } from "../../generator_names"
import { encode_vu128 } from "../../generator_values"
import { operation_field_count } from "../../operation_plans"
import type { Operation_Result_Kind } from "../../compatibility_result_projections"
import { bytes_from_hex, is_packed_f64_type } from "../rendering"
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
  operation_uses_optional_value_layout,
  optional_value_framing,
  render_application_value_codec,
  render_composite_field_decode,
  render_composite_output,
  render_expression_generic_invocation,
  render_field_sequence_request_payload,
  render_field_sequence_response_decode,
  render_operation_result,
  render_opaque_request_expression,
  render_python_container_helpers,
  render_python_field_sequence_helpers,
  type Managed_Api_Operation,
} from "../managed"

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
    case "double":
      rendered = "float"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = python_api_name(type.name)
      break
    case "integer":
      rendered = "int"
      break
    case "list":
      rendered = is_packed_f64_type(type)
        ? "list[float]"
        : `list[${python_api_type(type.member ?? { kind: "blob" }, true)}]`
      break
    case "map":
      rendered = `dict[${python_api_type(type.key ?? { kind: "string" }, true)}, ${
        python_api_type(type.value ?? { kind: "blob" }, true)
      }]`
      break
    case "long":
      rendered = "int"
      break
    case "structure":
      if (type.name === undefined) throw new Error("structure API type has no name")
      rendered = python_api_name(type.name)
      break
    case "string":
      rendered = "str"
      break
    case "union":
      rendered = "bytes"
      break
    case "unsigned_long":
      rendered = "int"
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

function render_python_operation_method(
  contract: Client_Contract,
  operation: Managed_Api_Operation,
): string {
  const method_name = snake_case(operation.name)
  const input = python_api_name(operation.input)
  const output = python_api_name(operation.output)
  const operation_value = operation.opcode.value
  const result_constant = (kind: Operation_Result_Kind): string =>
    operation_result_constant(operation, kind, "python")
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
    output_deleted,
    output_descriptor,
    output_json,
    output_outcome,
    output_value,
  } = operation_convenience_fields(operation, "python")
  const input_item_ids = operation_item_fields(operation).map(
    (member) => operation_field_name(member, "python"),
  )
  const input_item_id_expression = input_item_ids.length === 0
    ? "Data()"
    : input_item_ids.length === 1
    ? `input.${input_item_ids[0]!}`
    : `_smithy_concat_item_ids([${input_item_ids.map((name) => `input.${name}`).join(", ")}])`
  const {
    policy_default_eviction,
    policy_default_expiration,
    policy_default_ttl_milliseconds,
    policy_eviction_override,
    policy_expiration_override,
  } = operation_policy_fields(contract, operation, "python")
  const application_value_codecs = operation.plan.application_value_codecs
  const input_request_value = operation_request_value_name(operation, "python")
  const scoped_request_value = input_request_value === undefined
    ? ""
    : `            value=input.${input_request_value},\n`
  return render_operation_result(operation, "Python", {
    raw_payload: () => {
      const output_payload = operation_opaque_field_name(operation, "output", "python")
      const invocation = render_expression_generic_invocation(
        "python",
        operation,
        String(operation_value),
        String(operation_value),
        result_kinds,
      ) ??
        `self._smithy_transport.invoke(
            ${operation_value},
            expected_kinds=(${result_kinds},),
        )`
      return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        _, payload = await ${invocation}
        return ${output}(${output_payload}=payload)`
    },
    opaque: () => {
        const input_payload = operation_request_is_opaque(operation)
          ? operation_opaque_field_name(operation, "input", "python")
          : undefined
        const output_payload = operation_opaque_field_name(operation, "output", "python")
        const codec = render_application_value_codec(
          "python",
          application_value_codecs!,
          input_payload === undefined ? "b''" : `input.${input_payload}`,
          "payload",
          String(operation_value),
        )
        const decoded_payload = codec.decode
        const invocation = render_expression_generic_invocation(
          "python",
          operation,
          String(operation_value),
          String(operation_value),
          result_kinds,
        ) ??
          (operation_is_global_empty(operation)
            ? `self._smithy_transport.invoke(
            ${operation_value},
            expected_kinds=(${result_kinds},),
        )`
            : `self._smithy_transport.invoke_scoped(
            ${operation_value},
            namespace_id=input.${input_namespace_id},
            item_id=${input_item_id_expression},
${scoped_request_value}            expected_kinds=(${result_kinds},),
        )`)
        return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        _, payload = await ${invocation}
        return ${output}(
            ${output_payload}=${decoded_payload}
        )`
    },
    field_sequence: () => {
        const output_decoded_values = operation_composite_fields(operation)
          .map((field, index) =>
            render_composite_field_decode(
              "python",
              operation_composite_field_codec(operation, field),
              `values[${index}]`,
              String(operation_value),
              field.required,
              field.type,
            ),
          )
        const output_expression = render_composite_output(
          operation,
          "python",
          output_decoded_values,
        )
        const response_values = operation.plan.contract.response_framing === "field_sequence"
          ? render_field_sequence_response_decode(
            "python",
            operation,
            "payload",
            String(operation_value),
          )
          : `_smithy_decode_optional_values(payload, ${operation_composite_value_count(operation)}, ${operation_value})`
        const invocation = render_expression_generic_invocation(
          "python",
          operation,
          String(operation_value),
          String(operation_value),
          result_kinds,
        ) ??
          (operation_is_global_empty(operation)
            ? `self._smithy_transport.invoke(
            ${operation_value},
            expected_kinds=(${result_kinds},),
        )`
            : `self._smithy_transport.invoke_scoped(
            ${operation_value},
            namespace_id=input.${input_namespace_id},
            item_id=${input_item_id_expression},
${scoped_request_value}            expected_kinds=(${result_kinds},),
        )`)
        return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        _, payload = await ${invocation}
        values = ${response_values}
        return ${output_expression}`
    },
    optional_payload: () => {
      if (operation_field_count(operation.plan.operation, "output", "value") > 1) {
        const output_values = operation_fields(operation, "output", "value")
          .map((member) => operation_field_name(member, "python"))
          .map((name, index) => `${name}=values[${index}]`)
          .join(",\n            ")
        return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        _, payload = await self._smithy_transport.invoke_scoped(
            ${operation_value},
            namespace_id=input.${input_namespace_id},
            item_id=${input_item_id_expression},
${scoped_request_value}            expected_kinds=(${result_kinds},),
        )
        values = _smithy_decode_optional_values(
            payload, ${operation_field_count(operation.plan.operation, "output", "value")}, ${operation_value})
        return ${output}(
            ${output_values}
        )`
      }
      return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        kind, payload = await self._smithy_transport.invoke_scoped(
            ${operation_value},
            namespace_id=input.${input_namespace_id},
            item_id=${input_item_id_expression},
${scoped_request_value}            expected_kinds=(${result_kinds},),
        )
        return ${output}(
            ${output_value}=None
            if kind == ${result_constant("not_found")}
            else payload
        )`
    },
    status_outcome: () => {
      return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        kind, _ = await self._smithy_transport.invoke_scoped(
            ${operation_value},
            namespace_id=input.${input_namespace_id},
            item_id=${input_item_id_expression},
            value=input.${input_value},
            condition=input.${input_condition},
            expiration_mode=input.${input_expiration_mode},
            eviction_mode=input.${input_eviction_mode},
            ttl_milliseconds=input.${input_ttl_milliseconds},
            expected_kinds=(${result_kinds},),
        )
        outcome = {
            ${result_constant("created")}: SmithySetOutcome.CREATED,
            ${result_constant("replaced")}: SmithySetOutcome.REPLACED,
            ${result_constant("not_stored")}: SmithySetOutcome.NOT_STORED,
        }[kind]
        return ${output}(${output_outcome}=outcome)`
    },
    boolean_outcome: () => {
      return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        kind, _ = await self._smithy_transport.invoke_scoped(
            ${operation_value},
            namespace_id=input.${input_namespace_id},
            item_id=${input_item_id_expression},
            expected_kinds=(${result_kinds},),
        )
        return ${output}(
            ${output_deleted}=kind == ${result_constant("deleted")}
        )`
    },
    text_payload: () => {
      return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        _, payload = await self._smithy_transport.invoke_scoped(
            ${operation_value},
            namespace_id=input.${input_namespace_id},
            expected_kinds=(${result_kinds},),
        )
        return ${output}(
            ${output_json}=self._smithy_transport.decode_utf8(payload, ${operation_value})
        )`
    },
    empty: () => {
      if (operation_is_global_empty(operation)) {
        return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        await self._smithy_transport.invoke(
            ${operation_value},
            value=b"",
            expected_kinds=(${result_kinds},),
        )
        return ${output}()`
      }
      if (
        operation_is_global_opaque(operation) ||
        operation_is_global_field_sequence(operation)
      ) {
        const request_payload = operation_is_global_opaque(operation)
          ? render_opaque_request_expression("python", operation, String(operation_value))
          : render_field_sequence_request_payload(
            "python",
            operation,
            String(operation_value),
          )
        return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        await self._smithy_transport.invoke(
            ${operation_value},
            value=${request_payload},
            expected_kinds=(${result_kinds},),
        )
        return ${output}()`
      }
      if (operation_uses_compact_item_request(operation)) {
        const has_request_value = operation_request_value_count(operation) > 0
        const request_arguments = has_request_value
          ? `            value=input.${input_value},
            condition=input.${input_condition},
            expiration_mode=input.${input_expiration_mode},
            eviction_mode=input.${input_eviction_mode},
            ttl_milliseconds=input.${input_ttl_milliseconds},
`
          : ""
        return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        await self._smithy_transport.invoke_scoped(
            ${operation_value},
            namespace_id=input.${input_namespace_id},
            item_id=${input_item_id_expression},
${request_arguments}            expected_kinds=(${result_kinds},),
        )
        return ${output}()`
      }
      if (operation_uses_compact_request_route(operation, "namespace_delete")) {
        return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        await self._smithy_transport.namespace_delete(
            namespace_id=input.${input_namespace_id},
            expected_revision=input.${input_expected_revision},
        )
        return ${output}()`
      }
      if (!operation_uses_compact_namespace_request(operation)) {
        throw new Error(`unsupported generated Python empty operation ${operation.name}`)
      }
      return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        await self._smithy_transport.invoke_scoped(
            ${operation_value},
            namespace_id=input.${input_namespace_id},
            expected_kinds=(${result_kinds},),
        )
        return ${output}()`
    },
    descriptor: () => {
      if (operation_uses_compact_request_route(operation, "namespace_open")) {
        return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        return await self._smithy_transport.namespace_open(
            name=input.${input_name},
            create_if_missing=input.${input_create_if_missing},
            policy_default_expiration=(
                None
                if input.${input_policy} is None
                else input.${input_policy}.${policy_default_expiration}
            ),
            policy_default_ttl_milliseconds=(
                None
                if input.${input_policy} is None
                else input.${input_policy}.${policy_default_ttl_milliseconds}
            ),
            policy_expiration_override=(
                None
                if input.${input_policy} is None
                else input.${input_policy}.${policy_expiration_override}
            ),
            policy_default_eviction=(
                None
                if input.${input_policy} is None
                else input.${input_policy}.${policy_default_eviction}
            ),
            policy_eviction_override=(
                None
                if input.${input_policy} is None
                else input.${input_policy}.${policy_eviction_override}
            ),
        )`
      }
      if (operation_uses_compact_request_route(operation, "namespace_update_policy")) {
        return `    async def ${method_name}(self, input: ${input}) -> ${output}:
        self._smithy_transport.assert_open()
        return ${output}(
            ${output_descriptor}=await self._smithy_transport.namespace_update_policy(
                namespace_id=input.${input_namespace_id},
                expected_revision=input.${input_expected_revision},
                default_expiration=input.${input_policy}.${policy_default_expiration},
                default_ttl_milliseconds=input.${input_policy}.${policy_default_ttl_milliseconds},
                expiration_override=input.${input_policy}.${policy_expiration_override},
                default_eviction=input.${input_policy}.${policy_default_eviction},
                eviction_override=input.${input_policy}.${policy_eviction_override},
            )
        )`
      }
      throw new Error(`unsupported generated Python namespace operation ${operation.name}`)
    },
  })
}

/** Renders generated Python Smithy operation implementations. */
export function render_python_operations(contract: Client_Contract): string {
  const managed_operations = managed_operation_entries(contract)
  const result_imports = operation_result_kind_imports(managed_operations)
  const framing = optional_value_framing(contract)
  const field_framing = field_sequence_framing(contract)
  const container_helpers = has_wire_codec(
    managed_operations,
    ["list", "map", "union"],
  )
    ? render_python_container_helpers(contract.max_value_bytes)
    : ""
  const field_sequence_helpers = managed_operations.some(
    operation_uses_field_sequence_helpers,
  )
    ? render_python_field_sequence_helpers(field_framing)
    : ""
  const imported_types = new Set<string>()
  for (const operation of managed_operations) {
    imported_types.add(python_api_name(operation.input))
    imported_types.add(python_api_name(operation.output))
  }
  imported_types.add("SmithyNamespaceDescriptor")
  imported_types.add("SmithyNamespaceOpenOutput")
  imported_types.add("SmithySetOutcome")
  imported_types.add("SmithySetCondition")
  imported_types.add("SmithyExpirationMode")
  imported_types.add("SmithyEvictionMode")
  imported_types.add("SmithyExpirationDefault")
  imported_types.add("SmithyOverridePolicy")
  imported_types.add("SmithyEvictionDefault")
  const imports = [...imported_types].sort().join(",\n    ")
  const methods = managed_operations
    .map((operation) => render_python_operation_method(contract, operation))
    .join("\n\n")
  const f64_array_helpers = has_application_value_codec(
    managed_operations,
    "packed_f64_be",
  )
    ? `def _smithy_encode_f64_array(values: list[float]) -> bytes:
    payload = bytearray()
    for value in values:
        if not math.isfinite(value):
            raise ValueError("binary64 array input must contain finite values")
        payload.extend(struct.pack(">d", value))
    return bytes(payload)


def _smithy_decode_f64_array(payload: bytes, operation: int) -> list[float]:
    if len(payload) % 8 != 0:
        raise ValueError(
            f"operation {operation} response has a malformed binary64 array length"
        )
    values = [
        value
        for (value,) in struct.iter_unpack(">d", payload)
    ]
    if not all(math.isfinite(value) for value in values):
        raise ValueError(
            f"operation {operation} response contains a non-finite binary64 value"
        )
    return values
`
    : ""
  const item_id_helpers = managed_operations.some(
    operation_uses_item_id_helpers,
  )
    ? `def _smithy_concat_item_ids(item_ids: list[bytes]) -> bytes:
    total = 0
    for item_id in item_ids:
        if len(item_id) > ${contract.item_id_bytes}:
            raise ValueError("item IDs must contain at most ${contract.item_id_bytes} bytes")
        if total > ${contract.item_id_bytes} - len(item_id):
            raise ValueError("combined item IDs must contain at most ${contract.item_id_bytes} bytes")
        total += len(item_id)
    return b"".join(item_ids)
`
    : ""
  const optional_values_helpers = managed_operations.some(
    operation_uses_optional_value_layout,
  )
    ? `def _smithy_decode_optional_values(
    payload: bytes,
    value_count: int,
    operation: int,
) -> list[bytes | None]:
    values: list[bytes | None] = []
    offset = 0
    for _ in range(value_count):
        if len(payload) - offset < ${framing.length_bytes}:
            raise ValueError(f"operation {operation} response is missing an optional-value length")
        length = int.from_bytes(payload[offset:offset + ${framing.length_bytes}], "big")
        offset += ${framing.length_bytes}
        if length == ${framing.missing_sentinel}:
            values.append(None)
            continue
        if length > ${framing.max_value_bytes}:
            raise ValueError(
                f"operation {operation} response optional-value entry exceeds the maximum value size"
            )
        if length > len(payload) - offset:
            raise ValueError(f"operation {operation} response contains a truncated optional-value entry")
        values.append(payload[offset:offset + length])
        offset += length
    if offset != len(payload):
        raise ValueError(f"operation {operation} response contains trailing optional-value bytes")
    return values
`
    : ""
  const compatibility_helpers = `${item_id_helpers}${optional_values_helpers}`
  return `# Generated from the OpenKache Smithy contract. Do not edit.

from __future__ import annotations

import math
import struct
from typing import Protocol

from .smithy_api import (
    ${imports},
)
from .smithy_contract import (
    ${result_imports.join(",\n    ")},
)

${f64_array_helpers}
${container_helpers}
${field_sequence_helpers}
${compatibility_helpers}
class SmithyOperationTransport(Protocol):
    """Native-facing hooks used by generated Smithy operations."""

    def assert_open(self) -> None: ...

    async def invoke(
        self,
        operation: int,
        *,
        key: bytes = b"",
        value: bytes = b"",
        condition: SmithySetCondition | None = None,
        expiration_mode: SmithyExpirationMode | None = None,
        eviction_mode: SmithyEvictionMode | None = None,
        ttl_milliseconds: int | None = None,
        expected_kinds: tuple[int, ...],
    ) -> tuple[int, bytes]: ...

    async def invoke_scoped(
        self,
        operation: int,
        *,
        namespace_id: int,
        item_id: bytes = b"",
        value: bytes = b"",
        condition: SmithySetCondition | None = None,
        expiration_mode: SmithyExpirationMode | None = None,
        eviction_mode: SmithyEvictionMode | None = None,
        ttl_milliseconds: int | None = None,
        expected_kinds: tuple[int, ...],
    ) -> tuple[int, bytes]: ...

    def decode_utf8(self, payload: bytes, operation: int) -> str: ...

    async def namespace_open(
        self,
        *,
        name: str,
        create_if_missing: bool,
        policy_default_expiration: SmithyExpirationDefault | None,
        policy_default_ttl_milliseconds: int | None,
        policy_expiration_override: SmithyOverridePolicy | None,
        policy_default_eviction: SmithyEvictionDefault | None,
        policy_eviction_override: SmithyOverridePolicy | None,
    ) -> SmithyNamespaceOpenOutput: ...

    async def namespace_update_policy(
        self,
        *,
        namespace_id: int,
        expected_revision: int,
        default_expiration: SmithyExpirationDefault,
        default_ttl_milliseconds: int | None,
        expiration_override: SmithyOverridePolicy,
        default_eviction: SmithyEvictionDefault,
        eviction_override: SmithyOverridePolicy,
    ) -> SmithyNamespaceDescriptor: ...

    async def namespace_delete(
        self, *, namespace_id: int, expected_revision: int
    ) -> None: ...


class SmithyGeneratedOperations:
    """Generated Smithy operations backed by native adapter hooks."""

    def __init__(self, transport: SmithyOperationTransport) -> None:
        self._smithy_transport = transport

${methods}
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
  const envelope = contract.value_envelope
  const descriptor_layout = contract.ffi.namespace_descriptor_layout
  const descriptor_fields = contract.ffi.namespace_descriptor_fields
  const version_bytes = encode_vu128(value.version)
  const magic = bytes_from_hex(envelope.magic_and_version_hex, "value envelope magic")
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
  const ffi_status_categories = contract.ffi.status_categories
    .map(
      (entry) =>
        `SMITHY_FFI_STATUS_CATEGORY_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_error_categories = contract.ffi.error_categories
    .map(
      (entry) =>
        `SMITHY_FFI_ERROR_CATEGORY_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_connection_states = contract.ffi.connection_states
    .map(
      (entry) =>
        `SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()} = ${entry.value}
SMITHY_FFI_CONNECTION_STATE_${snake_case(entry.name).toUpperCase()}_NAME = ${JSON.stringify(entry.text)}`,
    )
    .join("\n")
  const ffi_transports = contract.ffi.transports
    .map(
      (entry) =>
        `SMITHY_FFI_TRANSPORT_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_set_conditions = contract.ffi.set_conditions
    .map(
      (entry) =>
        `SMITHY_FFI_SET_CONDITION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_key_specs = contract.ffi.key_specs
    .map(
      (entry) =>
        `SMITHY_FFI_KEY_SPEC_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_namespace_descriptor_decode_statuses =
    contract.ffi.namespace_descriptor_decode_statuses
      .map(
        (entry) =>
          `SMITHY_FFI_NAMESPACE_DESCRIPTOR_DECODE_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
      )
      .join("\n")
  const ffi_namespace_default_expirations = contract.ffi.namespace_default_expirations
    .map(
      (entry) =>
        `SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_namespace_default_evictions = contract.ffi.namespace_default_evictions
    .map(
      (entry) =>
        `SMITHY_FFI_NAMESPACE_DEFAULT_EVICTION_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const ffi_namespace_override_policies = contract.ffi.namespace_override_policies
    .map(
      (entry) =>
        `SMITHY_FFI_NAMESPACE_OVERRIDE_${snake_case(entry.name).toUpperCase()} = ${entry.value}`,
    )
    .join("\n")
  const opcodes = contract.opcodes
    .map((entry) => `SMITHY_OPCODE_${snake_case(entry.name).toUpperCase()} = ${entry.value}`)
    .join("\n")
  const statuses = contract.statuses
    .map((entry) => `SMITHY_STATUS_${snake_case(entry.name).toUpperCase()} = ${entry.value}`)
    .join("\n")
  const python_namespace_descriptor_fields = descriptor_fields.map(
    (field) => `        ("${field.name}", ${field.python_type}),`,
  ).join("\n")
  const python_descriptor_offsets = descriptor_fields
    .map(
      (field) =>
        `SMITHY_FFI_NAMESPACE_DESCRIPTOR_${snake_case(field.name).toUpperCase()}_OFFSET = ${field.offset}`,
    )
    .join("\n")
  return `# Generated from the OpenKache Smithy contract. Do not edit.

import ctypes as _ctypes

class SmithyFFINamespaceDescriptor(_ctypes.Structure):
    """C-compatible namespace descriptor returned by the native ABI decoder."""

    _fields_ = [
${python_namespace_descriptor_fields}
    ]

SMITHY_PROTOCOL_ALPN = ${JSON.stringify(contract.v1.alpn)}
SMITHY_OPCODE_BYTES = ${contract.v1.opcode_bytes}
SMITHY_STATUS_BYTES = ${contract.v1.status_bytes}
SMITHY_REQUEST_FIXED_BYTES = ${contract.v1.request_fixed_bytes}
SMITHY_RESPONSE_FIXED_BYTES = ${contract.v1.response_fixed_bytes}
SMITHY_MIN_VARUINT_BYTES = ${contract.v1.min_varuint_bytes}
SMITHY_MAX_VARUINT_BYTES = ${contract.v1.max_varuint_bytes}
SMITHY_ITEM_ID_BYTES = ${contract.item_id_bytes}
SMITHY_MAX_VALUE_BYTES = ${contract.max_value_bytes}
SMITHY_NAMESPACE_ID_BYTES = ${contract.v1.namespace_id_bytes}
SMITHY_NAMESPACE_REVISION_BYTES = ${contract.v1.namespace_revision_bytes}
SMITHY_NAMESPACE_NAME_LENGTH_BYTES = ${contract.v1.namespace_name_length_bytes}
SMITHY_NAMESPACE_NAME_MAX_BYTES = ${contract.v1.namespace_name_max_bytes}
SMITHY_SET_FLAGS_BYTES = ${contract.v1.set_flags_bytes}
SMITHY_SET_CONDITION_MASK = ${contract.v1.set_condition_mask}
SMITHY_SET_CONDITION_ANY_BITS = ${contract.v1.set_condition_any_bits}
SMITHY_SET_IF_ABSENT_BITS = ${contract.v1.set_if_absent_flag}
SMITHY_SET_IF_PRESENT_BITS = ${contract.v1.set_if_present_flag}
SMITHY_SET_CONDITION_RESERVED_BITS = ${contract.v1.set_condition_reserved_bits}
SMITHY_SET_EXPIRATION_MASK = ${contract.v1.set_expiration_mask}
SMITHY_SET_INHERIT_EXPIRATION_BITS = ${contract.v1.set_inherit_expiration_bits}
SMITHY_SET_NO_EXPIRY_BITS = ${contract.v1.set_no_expiry_bits}
SMITHY_SET_EXPLICIT_TTL_BITS = ${contract.v1.set_ttl_flag}
SMITHY_SET_EXPIRATION_RESERVED_BITS = ${contract.v1.set_expiration_reserved_bits}
SMITHY_SET_EVICTION_MASK = ${contract.v1.set_eviction_mask}
SMITHY_SET_INHERIT_EVICTION_BITS = ${contract.v1.set_inherit_eviction_bits}
SMITHY_SET_EVICTABLE_BITS = ${contract.v1.set_evictable_bits}
SMITHY_SET_EVICTION_PROTECTED_BITS = ${contract.v1.set_eviction_protected_bits}
SMITHY_SET_EVICTION_RESERVED_BITS = ${contract.v1.set_eviction_reserved_bits}
SMITHY_SET_RESERVED_MASK = ${contract.v1.set_reserved_mask}
SMITHY_OPEN_FLAGS_BYTES = ${contract.v1.open_flags_bytes}
SMITHY_OPEN_CREATE_IF_MISSING = ${contract.v1.open_create_if_missing_flag}
SMITHY_OPEN_RESERVED_MASK = ${contract.v1.open_reserved_mask}
SMITHY_DELETE_FLAGS_BYTES = ${contract.v1.delete_flags_bytes}
SMITHY_DELETE_IF_EMPTY = ${contract.v1.delete_if_empty_bits}
SMITHY_DELETE_MODE_MASK = ${contract.v1.delete_mode_mask}
SMITHY_DELETE_RESERVED_MASK = ${contract.v1.delete_reserved_mask}
SMITHY_POLICY_FLAGS_BYTES = ${contract.v1.policy_flags_bytes}
SMITHY_POLICY_DEFAULT_EXPIRATION_MASK = ${contract.v1.policy_default_expiration_mask}
SMITHY_POLICY_NO_EXPIRY = ${contract.v1.policy_no_expiry_bits}
SMITHY_POLICY_FIXED_TTL = ${contract.v1.policy_fixed_ttl_bits}
SMITHY_POLICY_DEFAULT_EXPIRATION_RESERVED_BITS = ${contract.v1.policy_default_expiration_reserved_bits}
SMITHY_POLICY_EXPIRATION_OVERRIDE = ${contract.v1.policy_expiration_override_flag}
SMITHY_POLICY_EVICTION_PROTECTED = ${contract.v1.policy_eviction_protected_flag}
SMITHY_POLICY_EVICTION_OVERRIDE = ${contract.v1.policy_eviction_override_flag}
SMITHY_POLICY_RESERVED_MASK = ${contract.v1.policy_reserved_mask}
SMITHY_ERROR_STATUS_MINIMUM = ${contract.v1.error_status_minimum}
SMITHY_DEFAULT_MAX_IN_FLIGHT = ${defaults.max_in_flight}
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
${ffi_status_categories}
${ffi_error_categories}
${ffi_connection_states}
${ffi_transports}
${ffi_set_conditions}
${ffi_key_specs}
${ffi_namespace_descriptor_decode_statuses}
${ffi_namespace_default_expirations}
${ffi_namespace_default_evictions}
${ffi_namespace_override_policies}
SMITHY_FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES = ${descriptor_layout.size_bytes}
${python_descriptor_offsets}
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
SMITHY_VALUE_SERIALIZATION_STRUCTURED = ${value.serialization_structured}
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
SMITHY_VALUE_ENVELOPE_MAGIC_AND_VERSION = bytes([${magic.join(", ")}])
SMITHY_VALUE_ENVELOPE_MAX_ENCODING_BYTES = ${envelope.max_encoding_bytes}
SMITHY_VALUE_ENVELOPE_MAX_TYPE_NAME_BYTES = ${envelope.max_type_name_bytes}
SMITHY_VALUE_ENVELOPE_JSON_ENCODING = ${JSON.stringify(envelope.json_encoding)}

${opcodes}
${statuses}
`
}
