//! Go API and contract rendering.

import type { Api_Type, Client_Contract } from "./model"
import { go_exported_name, pascal_case, snake_case } from "./utils"

function go_api_name(identifier: string): string {
  return `Smithy${pascal_case(snake_case(identifier))}`
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
    case "integer":
      rendered = "int32"
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
${contract.ffi.status_categories
  .map(
    (entry) =>
      `\t// SmithyFFIStatusCategory${go_ffi_name(entry.name)} identifies the native completion status category ${entry.name}.
\tSmithyFFIStatusCategory${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
  )
  .join("\n")}
${contract.ffi.error_categories
  .map(
    (entry) =>
      `\t// SmithyFFIErrorCategory${go_ffi_name(entry.name)} identifies the native error category ${entry.name}.
\tSmithyFFIErrorCategory${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
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
\tSmithyFFIConnectionState${go_ffi_name(entry.name)} uint32 = ${entry.value}
\t// SmithyFFIConnectionState${go_ffi_name(entry.name)}Name is its stable text name.
\tSmithyFFIConnectionState${go_ffi_name(entry.name)}Name = ${JSON.stringify(entry.text)}`,
  )
  .join("\n")}
${contract.ffi.transports
  .map(
    (entry) =>
      `\t// SmithyFFITransport${go_ffi_name(entry.name)} selects the native transport and trust policy.
\tSmithyFFITransport${go_ffi_name(entry.name)} uint32 = ${entry.value}`,
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
