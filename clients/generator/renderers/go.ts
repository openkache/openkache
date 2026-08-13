/** Go API, contract, and operation renderers. */

import type { Api_Operation, Api_Type } from "../../operation_models"
import type { Client_Contract } from "../../client_contract"
import { go_exported_name, snake_case } from "../../generator_names"
import { derive_operation_plan, operation_field_count } from "../../operation_plans"
import type { Operation_Result_Kind } from "../../compatibility_result_projections"
import {
  operation_uses_optional_value_layout,
  optional_value_framing,
} from "../../compatibility_response_framing"
import { is_packed_f64_type } from "../rendering"
import {
  adapter_contract_values,
  field_sequence_framing,
  go_api_name,
  has_application_value_codec,
  has_wire_codec,
  managed_operation_entries,
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
  operation_uses_compact_item_request,
  operation_uses_compact_namespace_request,
  operation_uses_compact_request_route,
  operation_uses_field_sequence_helpers,
  operation_uses_item_id_helpers,
  render_application_value_codec,
  render_composite_output,
  render_field_sequence_response_decode,
  render_go_composite_field,
  render_go_container_helpers,
  render_go_field_sequence_helpers,
  render_go_field_sequence_request,
  render_go_generic_invocation,
  render_operation_result,
  type Managed_Api_Operation,
} from "../managed"

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
    case "double":
      rendered = "float64"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = go_api_name(type.name)
      break
    case "integer":
      rendered = "int32"
      break
    case "list":
      rendered = is_packed_f64_type(type)
        ? "[]float64"
        : `[]${go_api_type(type.member ?? { kind: "blob" }, true)}`
      break
    case "map":
      rendered = `map[${go_api_type(type.key ?? { kind: "string" }, true)}]${
        go_api_type(type.value ?? { kind: "blob" }, true)
      }`
      break
    case "long":
      rendered = "int64"
      break
    case "structure":
      if (type.name === undefined) throw new Error("structure API type has no name")
      rendered = go_api_name(type.name)
      break
    case "string":
      rendered = "string"
      break
    case "union":
      rendered = "[]byte"
      break
    case "unsigned_long":
      rendered = "uint64"
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
  const adapter_values = adapter_contract_values(contract)
  const descriptor_layout = contract.ffi.namespace_descriptor_layout
  const descriptor_fields = contract.ffi.namespace_descriptor_fields
  const go_namespace_descriptor_fields = descriptor_fields.map(
    (field) => `\t${field.go_name} ${field.go_type}`,
  ).join("\n")
  const go_descriptor_offsets = descriptor_fields
    .map(
      (field) =>
        `\tSmithyFFINamespaceDescriptor${field.go_name}Offset = ${field.offset}`,
    )
    .join("\n")
  const item_id_operations = contract.api.operations.filter(
    (operation) =>
      operation.contract !== undefined &&
      operation_field_count(
        derive_operation_plan(contract, operation),
        "input",
        "item_id",
      ) > 0,
  )
  const item_id_cases = item_id_operations
    .map((operation) => `SmithyOpcode${operation.name}`)
    .join(", ")
  return `// Code generated from the OpenKache Smithy contract. DO NOT EDIT.

package openkache

// SmithyFFINamespaceDescriptor is the C-compatible namespace descriptor
// returned by the shared native ABI decoder.
type SmithyFFINamespaceDescriptor struct {
${go_namespace_descriptor_fields}
}

const (
\t// SmithyProtocolALPN is the negotiated protocol identifier.
\tSmithyProtocolALPN = ${JSON.stringify(contract.v1.alpn)}
\t// SmithyItemIDBytes is the exact protocol item-ID width.
\tSmithyItemIDBytes = ${contract.item_id_bytes}
\t// SmithyMaxValueBytes is the protocol value and payload ceiling.
\tSmithyMaxValueBytes = ${contract.max_value_bytes}
\t// SmithyOpcodeBytes and SmithyStatusBytes are fixed opcode/status widths.
\tSmithyOpcodeBytes = ${contract.v1.opcode_bytes}
\tSmithyStatusBytes = ${contract.v1.status_bytes}
\tSmithyRequestFixedBytes = ${contract.v1.request_fixed_bytes}
\tSmithyResponseFixedBytes = ${contract.v1.response_fixed_bytes}
\tSmithyMinVaruintBytes = ${contract.v1.min_varuint_bytes}
\tSmithyMaxVaruintBytes = ${contract.v1.max_varuint_bytes}
\tSmithyNamespaceIDBytes = ${contract.v1.namespace_id_bytes}
\tSmithyNamespaceRevisionBytes = ${contract.v1.namespace_revision_bytes}
\tSmithyNamespaceNameLengthBytes = ${contract.v1.namespace_name_length_bytes}
\tSmithyNamespaceNameMaxBytes = ${contract.v1.namespace_name_max_bytes}
\tSmithySetFlagsBytes = ${contract.v1.set_flags_bytes}
\tSmithySetConditionMask = ${contract.v1.set_condition_mask}
\tSmithySetConditionAnyBits = ${contract.v1.set_condition_any_bits}
\tSmithySetIfAbsentBits = ${contract.v1.set_if_absent_flag}
\tSmithySetIfPresentBits = ${contract.v1.set_if_present_flag}
\tSmithySetConditionReservedBits = ${contract.v1.set_condition_reserved_bits}
\tSmithySetExpirationMask = ${contract.v1.set_expiration_mask}
\tSmithySetInheritExpirationBits = ${contract.v1.set_inherit_expiration_bits}
\tSmithySetNoExpiryBits = ${contract.v1.set_no_expiry_bits}
\tSmithySetExplicitTTLBits = ${contract.v1.set_ttl_flag}
\tSmithySetExpirationReservedBits = ${contract.v1.set_expiration_reserved_bits}
\tSmithySetEvictionMask = ${contract.v1.set_eviction_mask}
\tSmithySetInheritEvictionBits = ${contract.v1.set_inherit_eviction_bits}
\tSmithySetEvictableBits = ${contract.v1.set_evictable_bits}
\tSmithySetEvictionProtectedBits = ${contract.v1.set_eviction_protected_bits}
\tSmithySetEvictionReservedBits = ${contract.v1.set_eviction_reserved_bits}
\tSmithySetReservedMask = ${contract.v1.set_reserved_mask}
\tSmithyOpenFlagsBytes = ${contract.v1.open_flags_bytes}
\tSmithyOpenCreateIfMissing = ${contract.v1.open_create_if_missing_flag}
\tSmithyOpenReservedMask = ${contract.v1.open_reserved_mask}
\tSmithyDeleteFlagsBytes = ${contract.v1.delete_flags_bytes}
\tSmithyDeleteIfEmpty = ${contract.v1.delete_if_empty_bits}
\tSmithyDeleteModeMask = ${contract.v1.delete_mode_mask}
\tSmithyDeleteReservedMask = ${contract.v1.delete_reserved_mask}
\tSmithyPolicyFlagsBytes = ${contract.v1.policy_flags_bytes}
\tSmithyPolicyDefaultExpirationMask = ${contract.v1.policy_default_expiration_mask}
\tSmithyPolicyNoExpiry = ${contract.v1.policy_no_expiry_bits}
\tSmithyPolicyFixedTTL = ${contract.v1.policy_fixed_ttl_bits}
\tSmithyPolicyDefaultExpirationReservedBits = ${contract.v1.policy_default_expiration_reserved_bits}
\tSmithyPolicyExpirationOverride = ${contract.v1.policy_expiration_override_flag}
\tSmithyPolicyEvictionProtected = ${contract.v1.policy_eviction_protected_flag}
\tSmithyPolicyEvictionOverride = ${contract.v1.policy_eviction_override_flag}
\tSmithyPolicyReservedMask = ${contract.v1.policy_reserved_mask}
\tSmithyErrorStatusMinimum = ${contract.v1.error_status_minimum}
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

// smithyOperationUsesItemID reports whether the Smithy operation's native input is an item ID.
// The switch is generated from operation semantics so native adapters do not name individual
// operations when marshalling a generic scoped request.
func smithyOperationUsesItemID(operation uint32) bool {
\tswitch operation {
\tcase ${item_id_cases}:
\t\treturn true
\tdefault:
\t\treturn false
\t}
}

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
	// SmithyFFIKeySpecText/Bytes/Integer identify logical key encodings accepted by typed execute.
	SmithyFFIKeySpecText uint32 = ${adapter_values.key_spec_text}
	SmithyFFIKeySpecBytes uint32 = ${adapter_values.key_spec_bytes}
	SmithyFFIKeySpecInteger uint32 = ${adapter_values.key_spec_integer}
${contract.ffi.connection_states
  .map(
    (entry) =>
      `\t// SmithyFFIConnectionState${go_ffi_name(entry.name)} identifies a native connection state.
\tSmithyFFIConnectionState${go_ffi_name(entry.name)} uint32 = ${entry.value}
\t// SmithyFFIConnectionState${go_ffi_name(entry.name)}Name is its stable text name.
\tSmithyFFIConnectionState${go_ffi_name(entry.name)}Name = ${JSON.stringify(entry.text)}`,
  )
  .join("\n")}
${contract.ffi.namespace_descriptor_decode_statuses
  .map(
    (entry) =>
      `\t// SmithyFFINamespaceDescriptorDecode${go_ffi_name(entry.name)} is the namespace descriptor decode status ${entry.name}.
\tSmithyFFINamespaceDescriptorDecode${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.namespace_default_expirations
  .map(
    (entry) =>
      `\t// SmithyFFINamespaceDefaultExpiration${go_ffi_name(entry.name)} is the namespace default expiration value ${entry.name}.
\tSmithyFFINamespaceDefaultExpiration${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.namespace_default_evictions
  .map(
    (entry) =>
      `\t// SmithyFFINamespaceDefaultEviction${go_ffi_name(entry.name)} is the namespace default eviction value ${entry.name}.
\tSmithyFFINamespaceDefaultEviction${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.namespace_override_policies
  .map(
    (entry) =>
      `\t// SmithyFFINamespaceOverride${go_ffi_name(entry.name)} is the namespace override-policy value ${entry.name}.
\tSmithyFFINamespaceOverride${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
\t// SmithyFFINamespaceDescriptorSizeBytes and offsets describe the C-compatible descriptor ABI.
\tSmithyFFINamespaceDescriptorSizeBytes = ${descriptor_layout.size_bytes}
${go_descriptor_offsets}
)

// Shared client defaults extracted from the Smithy service contract.
const (
\t// SmithyDefaultMaxInFlight is the default number of request lanes.
\tSmithyDefaultMaxInFlight = ${defaults.max_in_flight}
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

function go_operation_label(operation: Api_Operation): string {
  return snake_case(operation.name).replaceAll("_", " ")
}

function render_go_operation_method(
  contract: Client_Contract,
  operation: Managed_Api_Operation,
): string {
  const input = go_api_name(operation.input)
  const output = go_api_name(operation.output)
  const method = operation.name
  const opcode = `SmithyOpcode${operation.name}`
  const label = go_operation_label(operation)
  const result_constant = (kind: Operation_Result_Kind): string =>
    operation_result_constant(operation, kind, "go")
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
  } = operation_convenience_fields(operation, "go")
  const input_item_ids = operation_item_fields(operation).map(
    (member) => operation_field_name(member, "go"),
  )
  const {
    policy_default_eviction,
    policy_default_expiration,
    policy_default_ttl_milliseconds,
    policy_eviction_override,
    policy_expiration_override,
  } = operation_policy_fields(contract, operation, "go")
  const application_value_codecs = operation.plan.application_value_codecs
  const input_request_value = operation_request_value_name(operation, "go")
    ?? "nil"
  return render_operation_result(operation, "Go", {
    raw_payload: () => {
      const output_payload = operation_opaque_field_name(operation, "output", "go")
      const generic_invocation = render_go_generic_invocation(
        operation,
        `"${label}"`,
        output,
        output_payload,
      )
      const invocation = generic_invocation?.expression ??
        `s.client.invoke(
			ctx,
			${opcode},
			nil,
			nil,
			SetOptions{},
		)`
      return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
${generic_invocation === undefined ? "" : `${generic_invocation.statements}
`}\tresult, err := ${invocation}
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	if result.kind != ${result_constant("ok")} {
		return ${output}{}, unexpectedResult("${label}", result.kind)
	}
	return ${output}{${output_payload}: result.data}, nil
}`
    },
    opaque: () => {
        const input_payload = operation_request_is_opaque(operation)
          ? operation_opaque_field_name(operation, "input", "go")
          : undefined
        const output_payload = operation_opaque_field_name(operation, "output", "go")
        const codec = render_application_value_codec(
          "go",
          application_value_codecs!,
          input_payload === undefined ? "nil" : `input.${input_payload}`,
          "result.data",
          `"${label}"`,
          output,
          output_payload,
        )
        const decoded_payload = codec.decode
        const generic_invocation = render_go_generic_invocation(
          operation,
          `"${label}"`,
          output,
          output_payload,
        )
        const invocation = generic_invocation?.expression ??
          (operation_is_global_empty(operation)
            ? `s.client.invoke(
			ctx,
			${opcode},
			nil,
			nil,
			SetOptions{},
		)`
            : `s.client.invokeScopedBytes(
			ctx,
			${opcode},
			input.${input_namespace_id},
			itemIDs,
			${input_request_value},
			SetOptions{},
		)`
          )
        const item_id_setup = generic_invocation === undefined &&
          !operation_is_global_empty(operation)
          ? `\titemIDs, err := smithyConcatItemIDs(${input_item_ids.map((name) => `input.${name}`).join(", ")})
		if err != nil {
			return ${output}{}, err
		}
`
          : ""
        return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
${generic_invocation === undefined ? "" : `${generic_invocation.statements}
`}${item_id_setup}	result, err := ${invocation}
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	if result.kind != ${result_constant("value")} {
		return ${output}{}, unexpectedResult("${label}", result.kind)
	}
	${decoded_payload}
}`
    },
    field_sequence: () => {
        const decoded_fields = operation_composite_fields(operation)
          .map((field, index) =>
            render_go_composite_field(operation, field, index, `"${label}"`),
          )
        const decode_statements = decoded_fields
          .map((field) => field.statements)
          .join("\n")
        const response_values = render_field_sequence_response_decode(
          "go",
          operation,
          "result.data",
          `"${label}"`,
        )
        const output_expression = render_composite_output(
          operation,
          "go",
          decoded_fields.map((field) => field.expression),
        )
        const generic_invocation = render_go_generic_invocation(
          operation,
          `"${label}"`,
          output,
        )
        const invocation = generic_invocation?.expression ??
          (operation_is_global_empty(operation)
            ? `s.client.invoke(
			ctx,
			${opcode},
			nil,
			nil,
			SetOptions{},
		)`
            : `s.client.invokeScopedBytes(
			ctx,
			${opcode},
			input.${input_namespace_id},
			itemIDs,
			${input_request_value === "nil" ? "nil" : `input.${input_request_value}`},
			SetOptions{},
		)`
          )
        const item_id_setup = generic_invocation === undefined
          ? `		itemIDs, err := smithyConcatItemIDs(${input_item_ids.map((name) => `input.${name}`).join(", ")})
		if err != nil {
			return ${output}{}, err
		}
`
          : ""
        return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
${generic_invocation === undefined ? "" : `${generic_invocation.statements}
`}${item_id_setup}
		result, err := ${invocation}
		if err != nil {
			return ${output}{}, operationError("${label}", err)
		}
		if result.kind != ${result_constant("value")} {
			return ${output}{}, unexpectedResult("${label}", result.kind)
		}
		values, err := ${response_values}
		if err != nil {
			return ${output}{}, operationError("${label}", err)
		}
${decode_statements}
		return ${output_expression}, nil
	}`
    },
    optional_payload: () => {
      if (operation_field_count(operation.plan.operation, "output", "value") > 1) {
        const output_values = operation_fields(operation, "output", "value")
          .map((member) => operation_field_name(member, "go"))
          .map((name, index) => `${name}: values[${index}]`)
          .join(",\n\t\t")
        return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
		itemIDs, err := smithyConcatItemIDs(${input_item_ids.map((name) => `input.${name}`).join(", ")})
		if err != nil {
			return ${output}{}, err
		}
		result, err := s.client.invokeScopedBytes(
			ctx,
			${opcode},
			input.${input_namespace_id},
			itemIDs,
			${input_request_value === "nil" ? "nil" : `input.${input_request_value}`},
			SetOptions{},
		)
		if err != nil {
			return ${output}{}, operationError("${label}", err)
		}
		if result.kind != ${result_constant("value")} {
			return ${output}{}, unexpectedResult("${label}", result.kind)
		}
		values, err := smithyDecodeOptionalValues(result.data, ${operation_field_count(operation.plan.operation, "output", "value")})
		if err != nil {
			return ${output}{}, operationError("${label}", err)
		}
		return ${output}{
			${output_values},
		}, nil
	}`
      }
      return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
	itemID, err := NewItemID(input.${input_item_id})
	if err != nil {
		return ${output}{}, err
	}
	result, err := s.client.invokeScoped(
		ctx,
		${opcode},
		input.${input_namespace_id},
		itemID,
		${input_request_value === "nil" ? "nil" : `input.${input_request_value}`},
		SetOptions{},
	)
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	value, found, err := getResult("${label}", result)
	if err != nil || !found {
		return ${output}{}, err
	}
	return ${output}{${output_value}: &value}, nil
}`
    },
    status_outcome: () => {
      return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
	itemID, err := NewItemID(input.${input_item_id})
	if err != nil {
		return ${output}{}, err
	}
	options, err := smithySetOptions(
		input.${input_condition},
		input.${input_expiration_mode},
		input.${input_ttl_milliseconds},
		input.${input_eviction_mode},
	)
	if err != nil {
		return ${output}{}, err
	}
	result, err := s.client.invokeScoped(
		ctx,
		${opcode},
		input.${input_namespace_id},
		itemID,
		input.${input_value},
		options,
	)
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	outcome, err := setResult("${label}", result)
	return ${output}{${output_outcome}: SmithySetOutcome(outcome)}, err
}`
    },
    boolean_outcome: () => {
      return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
	itemID, err := NewItemID(input.${input_item_id})
	if err != nil {
		return ${output}{}, err
	}
	result, err := s.client.invokeScoped(
		ctx,
		${opcode},
		input.${input_namespace_id},
		itemID,
		nil,
		SetOptions{},
	)
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	deleted, err := deleteResult("${label}", result)
	return ${output}{${output_deleted}: deleted}, err
}`
    },
    text_payload: () => {
      return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
	result, err := s.client.invokeScoped(
		ctx,
		${opcode},
		input.${input_namespace_id},
		ItemID{},
		nil,
		SetOptions{},
	)
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	if result.kind != ${result_constant("value")} {
		return ${output}{}, unexpectedResult("${label}", result.kind)
	}
	return ${output}{${output_json}: string(result.data)}, nil
}`
    },
    empty: () => {
      if (operation_is_global_empty(operation)) {
        return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
	result, err := s.client.invoke(
		ctx,
		${opcode},
		nil,
		nil,
		SetOptions{},
	)
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	if result.kind != ${result_constant("ok")} {
		return ${output}{}, unexpectedResult("${label}", result.kind)
	}
	return ${output}{}, nil
}`
      }
      if (
        operation_is_global_opaque(operation) ||
        operation_is_global_field_sequence(operation)
      ) {
        const request = operation_is_global_opaque(operation)
          ? {
            statements: render_application_value_codec(
              "go",
              operation.plan.application_value_codecs!,
              operation_request_is_opaque(operation)
                ? `input.${operation_opaque_field_name(operation, "input", "go")}`
                : "nil",
              "result.data",
              `"${label}"`,
              output,
            ).encode,
            payload: "wireValue",
          }
          : render_go_field_sequence_request(operation, `"${label}"`)
        return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
	${request.statements}
	result, err := s.client.invoke(
		ctx,
		${opcode},
		nil,
		${request.payload},
		SetOptions{},
	)
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	if result.kind != ${result_constant("ok")} {
		return ${output}{}, unexpectedResult("${label}", result.kind)
	}
	return ${output}{}, nil
}`
      }
      if (operation_uses_compact_item_request(operation)) {
        const has_request_value = operation_request_value_count(operation) > 0
        const request_value = has_request_value
          ? `input.${input_request_value}`
          : "nil"
        const options = has_request_value
          ? `options, err := smithySetOptions(
		input.${input_condition},
		input.${input_expiration_mode},
		input.${input_ttl_milliseconds},
		input.${input_eviction_mode},
	)
	if err != nil {
		return ${output}{}, err
	}`
          : `options := SetOptions{}`
        return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
	itemIDs, err := smithyConcatItemIDs(${input_item_ids.map((name) => `input.${name}`).join(", ")})
	if err != nil {
		return ${output}{}, err
	}
	${options}
	result, err := s.client.invokeScopedBytes(
		ctx,
		${opcode},
		input.${input_namespace_id},
		itemIDs,
		${request_value},
		options,
	)
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	if result.kind != ${result_constant("ok")} {
		return ${output}{}, unexpectedResult("${label}", result.kind)
	}
	return ${output}{}, nil
}`
      }
      if (operation_uses_compact_namespace_request(operation)) {
        return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
	result, err := s.client.invokeScoped(
		ctx,
		${opcode},
		input.${input_namespace_id},
		ItemID{},
		nil,
		SetOptions{},
	)
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	if result.kind != ${result_constant("ok")} {
		return ${output}{}, unexpectedResult("${label}", result.kind)
	}
	return ${output}{}, nil
}`
      }
      if (operation_uses_compact_request_route(operation, "namespace_delete")) {
        return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
	result, err := s.client.invokeNamespaceDelete(
		ctx,
		input.${input_namespace_id},
		input.${input_expected_revision},
	)
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	if result.kind != ${result_constant("ok")} {
		return ${output}{}, unexpectedResult("${label}", result.kind)
	}
	return ${output}{}, nil
}`
      }
      throw new Error(`unsupported generated Go empty operation ${operation.name}`)
    },
    descriptor: () => {
      if (operation_uses_compact_request_route(operation, "namespace_open")) {
        return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
	if input.${input_create_if_missing} && input.${input_policy} == nil {
		return ${output}{}, validationError(
			"namespace.policy",
			"is required when create_if_missing is true",
		)
	}
	if !input.${input_create_if_missing} && input.${input_policy} != nil {
		return ${output}{}, validationError(
			"namespace.policy",
			"is only valid when create_if_missing is true",
		)
	}
	var policyFlags uint8
	var ttl uint64
	var err error
	if input.${input_policy} != nil {
		policyFlags, ttl, err = smithyNamespacePolicyWire(
			input.${input_policy}.${policy_default_expiration},
			input.${input_policy}.${policy_default_ttl_milliseconds},
			input.${input_policy}.${policy_expiration_override},
			input.${input_policy}.${policy_default_eviction},
			input.${input_policy}.${policy_eviction_override},
		)
	}
	if err != nil {
		return ${output}{}, err
	}
	result, err := s.client.invokeNamespaceOpen(
		ctx,
		[]byte(input.${input_name}),
		input.${input_create_if_missing},
		policyFlags,
		ttl,
	)
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	if result.kind != ${result_constant("ok")} && result.kind != ${result_constant("created")} {
		return ${output}{}, unexpectedResult("${label}", result.kind)
	}
	decoded, err := s.client.decodeNamespaceDescriptor(ctx, result.data)
	if err != nil {
		return ${output}{}, err
	}
	return ${output}{
		${output_descriptor}: smithyNamespaceDescriptor(decoded),
		${output_created}:    result.kind == ${result_constant("created")},
	}, nil
}`
      }
      if (operation_uses_compact_request_route(operation, "namespace_update_policy")) {
        return `func (s smithyClient) ${method}(
	ctx context.Context,
	input ${input},
) (${output}, error) {
	policyFlags, ttl, err := smithyNamespacePolicyWire(
		input.${input_policy}.${policy_default_expiration},
		input.${input_policy}.${policy_default_ttl_milliseconds},
		input.${input_policy}.${policy_expiration_override},
		input.${input_policy}.${policy_default_eviction},
		input.${input_policy}.${policy_eviction_override},
	)
	if err != nil {
		return ${output}{}, err
	}
	result, err := s.client.invokeNamespaceUpdatePolicy(
		ctx,
		input.${input_namespace_id},
		input.${input_expected_revision},
		policyFlags,
		ttl,
	)
	if err != nil {
		return ${output}{}, operationError("${label}", err)
	}
	if result.kind != ${result_constant("value")} {
		return ${output}{}, unexpectedResult("${label}", result.kind)
	}
	decoded, err := s.client.decodeNamespaceDescriptor(ctx, result.data)
	if err != nil {
		return ${output}{}, err
	}
	return ${output}{${output_descriptor}: smithyNamespaceDescriptor(decoded)}, nil
}`
      }
      throw new Error(`unsupported generated Go descriptor operation ${operation.name}`)
    },
  })
}

/** Renders generated Go operation implementations backed by the shared client core. */
export function render_go_operations(contract: Client_Contract): string {
  const managed_operations = managed_operation_entries(contract)
  const framing = optional_value_framing(contract)
  const field_framing = field_sequence_framing(contract)
  const container_helpers = has_wire_codec(
    managed_operations,
    ["list", "map", "union"],
  )
    ? render_go_container_helpers(contract.max_value_bytes)
    : ""
  const field_sequence_helpers = managed_operations.some(
    operation_uses_field_sequence_helpers,
  )
    ? render_go_field_sequence_helpers(field_framing)
    : ""
  const methods = managed_operations
    .map((operation) => render_go_operation_method(contract, operation))
    .join("\n\n")
  const f64_array_helpers = has_application_value_codec(
    managed_operations,
    "packed_f64_be",
  )
    ? `func smithyEncodeF64Array(values []float64) ([]byte, error) {
	payload := make([]byte, len(values)*8)
	for index, value := range values {
		if math.IsNaN(value) || math.IsInf(value, 0) {
			return nil, validationError("binary64 array", "must contain finite values")
		}
		binary.BigEndian.PutUint64(payload[index*8:], math.Float64bits(value))
	}
	return payload, nil
}

func smithyDecodeF64Array(payload []byte) ([]float64, error) {
	if len(payload)%8 != 0 {
		return nil, validationError("binary64 array", "has a malformed binary64 array length")
	}
	values := make([]float64, len(payload)/8)
	for index := range values {
		value := math.Float64frombits(binary.BigEndian.Uint64(payload[index*8:]))
		if math.IsNaN(value) || math.IsInf(value, 0) {
			return nil, validationError("binary64 array", "contains a non-finite binary64 value")
		}
		values[index] = value
	}
	return values, nil
}
`
    : ""
  const item_id_helpers = managed_operations.some(
    operation_uses_item_id_helpers,
  )
    ? `func smithyConcatItemIDs(itemIDs ...[]byte) ([]byte, error) {
	var total int
	for _, itemID := range itemIDs {
		if len(itemID) > SmithyItemIDBytes {
			return nil, validationError(
				"item_id",
				fmt.Sprintf("each item ID must contain at most %d bytes", SmithyItemIDBytes),
			)
		}
		if total > int(^uint(0)>>1)-1-len(itemID) {
			return nil, validationError("item_id", "combined item IDs are too large")
		}
		total += 1 + len(itemID)
	}
	combined := make([]byte, 0, total)
	for _, itemID := range itemIDs {
		combined = append(combined, byte(len(itemID)))
		combined = append(combined, itemID...)
	}
	return combined, nil
}
`
    : ""
  const optional_values_helpers = managed_operations.some(
    operation_uses_optional_value_layout,
  )
    ? `func smithyDecodeOptionalValues(payload []byte, valueCount int) ([]*[]byte, error) {
	values := make([]*[]byte, valueCount)
	offset := 0
	for index := range values {
		if len(payload)-offset < ${framing.length_bytes} {
			return values, validationError("optional_values", "response is missing an entry length")
		}
		length := binary.BigEndian.Uint32(payload[offset : offset+${framing.length_bytes}])
		offset += ${framing.length_bytes}
		if length == uint32(${framing.missing_sentinel}) {
			continue
		}
		if uint64(length) > uint64(${framing.max_value_bytes}) {
			return values, validationError(
				"optional_values",
				"response optional-value entry exceeds the maximum value size",
			)
		}
		if uint64(length) > uint64(len(payload)-offset) {
			return values, validationError("optional_values", "response contains a truncated entry")
		}
		value := append([]byte(nil), payload[offset:offset+int(length)]...)
		values[index] = &value
		offset += int(length)
	}
	if offset != len(payload) {
		return values, validationError("optional_values", "response contains trailing bytes")
	}
	return values, nil
}

func smithyDecodeOptionalF64Array(value *[]byte, operation string) (*[]float64, error) {
	if value == nil {
		return nil, nil
	}
	decoded, err := smithyDecodeF64Array(*value)
	if err != nil {
		return nil, operationError(operation, err)
	}
	return &decoded, nil
}

func smithyDecodeOptionalUTF8(value *[]byte) *string {
	if value == nil {
		return nil
	}
	decoded := string(*value)
	return &decoded
}
`
    : ""
  const compatibility_helpers = `${item_id_helpers}${optional_values_helpers}`
  return `// Code generated from the OpenKache Smithy client contract. DO NOT EDIT.

package openkache

import (
	"context"
	"encoding/binary"
${compatibility_helpers.length > 0 ? '\t"fmt"\n' : ""}
	"math"
)

${methods}

${f64_array_helpers}

${container_helpers}
${field_sequence_helpers}
${compatibility_helpers}
`
}

export function format_go_source(source: string): string {
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
