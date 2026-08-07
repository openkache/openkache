/** Smithy extraction and rendering for the server-visible OpenKache wire contract. */

import { existsSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

type Json_Object = Readonly<Record<string, unknown>>

/** One numeric protocol member assigned by the wire contract. */
export interface Wire_Entry {
  readonly name: string
  /** Optional Smithy enum value used for generated labels. */
  readonly text?: string
  readonly value: number
}

/** Semantic contract declared by one Smithy protocol operation. */
export const OPERATION_SCOPES = [
  "global",
  "item",
  "namespace",
  "namespace_management",
] as const

export type Wire_Operation_Scope = (typeof OPERATION_SCOPES)[number]

/** Generated transport descriptors understood by the protocol-v1 adapter. */
export type Wire_Operation_Request_Kind = string

/** Generated response transport descriptors understood by the adapters. */
export type Wire_Operation_Response_Kind = string

export const OPERATION_RETRY_MODES = [
  "always",
  "never",
  "when_not_creating",
] as const

export type Wire_Operation_Retry_Mode = (typeof OPERATION_RETRY_MODES)[number]

export const OPERATION_EFFECTS = ["read_only", "mutation", "barrier"] as const

export type Wire_Operation_Effect = (typeof OPERATION_EFFECTS)[number]

export interface Wire_Operation_Contract {
  readonly error_statuses: readonly string[]
  /**
   * Ordered field-role counts derived from the Smithy input structure. The
   * runtime uses these only for framing; it does not need to know an
   * operation-specific request family.
   */
  readonly request_fields: readonly Wire_Operation_Field[]
  /**
   * Ordered request field plan. This preserves requiredness, shape, and
   * member order for server-owned extensions; legacy fixtures may omit it.
   */
  readonly request_plan?: readonly Wire_Operation_Field_Plan[]
  /** Generated request transport descriptor derived from the field plan. */
  readonly request_kind: Wire_Operation_Request_Kind
  /**
   * Number of value fields in the request shape. Production extraction uses
   * this role count to choose the SET wire layout without consulting the
   * response contract. Legacy AST fixtures may omit it.
   */
  readonly request_value_count?: number
  /** Number of item IDs carried by a scoped-item request, derived from Smithy roles. */
  readonly request_item_count: number
  readonly response_fields: readonly Wire_Operation_Field[]
  /**
   * Ordered response field plan. This is the generic field-sequence source of
   * truth; optional-value framing is only one encoding used by that plan.
   */
  readonly response_plan?: readonly Wire_Operation_Field_Plan[]
  /** Number of optional values carried by a value response, derived from Smithy roles. */
  readonly response_value_count: number
  /** Generated response transport descriptor derived from the field plan. */
  readonly response_kind: Wire_Operation_Response_Kind
  readonly retry_mode: Wire_Operation_Retry_Mode
  readonly scope: Wire_Operation_Scope
  readonly success_statuses: readonly string[]
  /** Explicit storage effect used by runtime retry/timeout policy. */
  readonly effect: Wire_Operation_Effect
}

/** Count of one Smithy operation-field role in a request or response shape. */
export interface Wire_Operation_Field {
  readonly count: number
  readonly role: string
  /** Optional codec declarations attached to members of this role. */
  readonly codecs?: readonly string[]
}

/** One ordered Smithy field projected into the server operation plan. */
export interface Wire_Operation_Field_Plan {
  /** Stable zero-based position in the flattened Smithy field plan. */
  readonly index: number
  readonly codecs?: readonly string[]
  readonly path: readonly string[]
  readonly required: boolean
  readonly role: string
  /** Smithy shape name used by codec adapters and diagnostics. */
  readonly shape: string
}

/** One protocol opcode and its Smithy semantic operation contract. */
export interface Wire_Operation {
  readonly contract: Wire_Operation_Contract
  readonly name: string
}

/**
 * The finite set of request layouts understood by protocol v1.
 *
 * These are transport layouts, not operation names. A Smithy operation selects
 * one by declaring field roles; no operation-specific request label is needed.
 */
export type Wire_Request_Layout =
  | "empty"
  | "application_value"
  | "field_sequence"
  | "item"
  | "set"
  | "namespace"
  | "namespace_open"
  | "namespace_update_policy"
  | "namespace_delete"

/**
 * The transport response routes understood by the shared adapters.
 *
 * Server behavior remains outside this route classification. The route only
 * identifies the wire payload framing that clients must decode. `composite`
 * is an ordered field sequence derived from the output shape; it is not a
 * per-operation response family.
 */
export type Wire_Response_Route =
  | "empty"
  | "pong"
  | "application_value"
  | "field_sequence"
  | "composite"
  | "value"
  | "set_outcome"
  | "delete_outcome"
  | "stats_json"
  | "namespace_descriptor"

function operation_has_role(
  fields: readonly Wire_Operation_Field[],
  role: string,
): boolean {
  return fields.some((field) => field.role === role && field.count > 0)
}

function validate_operation_field_roles(
  fields: readonly Wire_Operation_Field[],
  allowed: readonly string[] | undefined,
  direction: "request" | "response",
): void {
  if (allowed === undefined) return
  const allowed_roles = new Set(allowed)
  const unsupported = fields
    .filter((field) => field.count > 0 && !allowed_roles.has(field.role))
    .map((field) => field.role)
  if (unsupported.length > 0) {
    throw new Error(
      `${direction} roles are not supported by protocol-v1 framing: ${[
        ...new Set(unsupported),
      ].join(", ")}`,
    )
  }
}

/** Derives the protocol-v1 request layout from Smithy input roles. */
export function derive_wire_request_layout(
  contract: Wire_Operation_Contract,
): Wire_Request_Layout {
  const { request_fields } = contract
  const has_role = (role: string): boolean => operation_has_role(request_fields, role)
  let layout: Wire_Request_Layout
  if (has_role("name") && has_role("create_if_missing") && has_role("policy")) {
    layout = "namespace_open"
  } else if (has_role("expected_revision") && has_role("policy")) {
    layout = "namespace_update_policy"
  } else if (
    has_role("expected_revision") &&
    has_role("namespace_id") &&
    !has_role("item_id")
  ) {
    layout = "namespace_delete"
  } else if (has_role("item_id")) {
    const value_count = contract.request_value_count ?? (
      request_fields.find((field) => field.role === "value")?.count ?? 0
    )
    layout = value_count > 0 ? "set" : "item"
  } else if (has_role("namespace_id")) {
    layout = "namespace"
  } else if (has_role("payload") && request_fields.length === 1) {
    layout = "application_value"
  } else if (request_fields.length === 0) {
    layout = "empty"
  } else {
    layout = "field_sequence"
  }
  const policy_roles = [
    "policy",
    "default_expiration",
    "default_ttl_milliseconds",
    "expiration_override",
    "default_eviction",
    "eviction_override",
  ]
  const allowed = {
    empty: [],
    application_value: ["payload"],
    item: ["namespace_id", "item_id"],
    set: [
      "namespace_id",
      "item_id",
      "value",
      "condition",
      "expiration_mode",
      "ttl_milliseconds",
      "eviction_mode",
    ],
    namespace: ["namespace_id"],
    namespace_open: ["name", "create_if_missing", ...policy_roles],
    namespace_update_policy: ["namespace_id", "expected_revision", ...policy_roles],
    namespace_delete: ["namespace_id", "expected_revision"],
  } satisfies Record<Wire_Request_Layout, readonly string[]>
  if (layout !== "field_sequence") {
    try {
      validate_operation_field_roles(request_fields, allowed[layout], "request")
    } catch {
      // Unknown semantic roles and new cardinalities use the generic field
      // sequence transport. Legacy layouts remain fail-closed only when the
      // field sequence itself cannot be represented.
      layout = "field_sequence"
    }
  }
  return layout
}

/** Derives the protocol response route from Smithy output roles and scope. */
export function derive_wire_response_route(
  contract: Wire_Operation_Contract,
): Wire_Response_Route {
  const { response_fields } = contract
  const response_field_count = contract.response_plan === undefined
    ? response_fields.reduce((count, field) => count + field.count, 0)
    : contract.response_plan.length
  const has_role = (role: string): boolean => operation_has_role(response_fields, role)
  let route: Wire_Response_Route
  const legacy_descriptor_roles = [
    "descriptor",
    "created",
    "namespace_id",
    "revision",
    "policy",
    "default_expiration",
    "default_ttl_milliseconds",
    "expiration_override",
    "default_eviction",
    "eviction_override",
  ]
  const has_only_roles = (roles: readonly string[]): boolean => {
    const allowed_roles = new Set(roles)
    return response_fields.every((field) => allowed_roles.has(field.role))
  }
  if (has_role("descriptor") && has_only_roles(legacy_descriptor_roles)) {
    route = "namespace_descriptor"
  } else if (response_field_count > 1) route = "field_sequence"
  else if (has_role("payload") && response_fields.length === 1) route = "application_value"
  else if (has_role("value") && response_fields.length === 1) route = "value"
  else if (has_role("outcome") && response_fields.length === 1) route = "set_outcome"
  else if (has_role("deleted") && response_fields.length === 1) route = "delete_outcome"
  else if (has_role("json") && response_fields.length === 1) route = "stats_json"
  else if (response_fields.length === 0) {
    route = contract.scope === "global" ? "pong" : "empty"
  } else {
    route = "field_sequence"
  }
  const policy_roles = [
    "policy",
    "default_expiration",
    "default_ttl_milliseconds",
    "expiration_override",
    "default_eviction",
    "eviction_override",
  ]
  const allowed: Partial<Record<Wire_Response_Route, readonly string[]>> = {
    pong: [],
    empty: [],
    application_value: ["payload"],
    field_sequence: undefined,
    value: ["value"],
    set_outcome: ["outcome"],
    delete_outcome: ["deleted"],
    stats_json: ["json"],
    namespace_descriptor: [
      "descriptor",
      "created",
      "namespace_id",
      "revision",
      ...policy_roles,
    ],
  }
  validate_operation_field_roles(response_fields, allowed[route], "response")
  return route
}

/** Protocol v1 constants consumed by the Rust protocol crate. */
export interface Wire_V1_Contract {
  readonly alpn: string
  readonly opcode_bytes: number
  readonly status_bytes: number
  readonly request_fixed_bytes: number
  readonly response_fixed_bytes: number
  readonly min_varuint_bytes: number
  readonly max_varuint_bytes: number
  readonly namespace_id_bytes: number
  readonly namespace_revision_bytes: number
  readonly namespace_name_length_bytes: number
  readonly namespace_name_max_bytes: number
  /** Optional-value response framing; defaults preserve legacy AST fixtures. */
  readonly optional_value_length_bytes?: number
  readonly optional_value_missing?: number
  readonly set_flags_bytes: number
  readonly set_condition_mask: number
  readonly set_condition_any_bits: number
  readonly set_condition_reserved_bits: number
  readonly set_expiration_mask: number
  readonly set_inherit_expiration_bits: number
  readonly set_no_expiry_bits: number
  /** The SET expiration-mode bit pattern for ExplicitTtl. */
  readonly set_ttl_flag: number
  readonly set_expiration_reserved_bits: number
  readonly set_eviction_mask: number
  readonly set_inherit_eviction_bits: number
  readonly set_evictable_bits: number
  readonly set_eviction_protected_bits: number
  readonly set_eviction_reserved_bits: number
  readonly set_reserved_mask: number
  readonly open_flags_bytes: number
  readonly open_create_if_missing_flag: number
  readonly open_reserved_mask: number
  readonly delete_flags_bytes: number
  readonly delete_if_empty_bits: number
  readonly delete_mode_mask: number
  readonly delete_reserved_mask: number
  readonly policy_flags_bytes: number
  readonly policy_default_expiration_mask: number
  readonly policy_no_expiry_bits: number
  readonly policy_fixed_ttl_bits: number
  readonly policy_default_expiration_reserved_bits: number
  readonly policy_expiration_override_flag: number
  readonly policy_eviction_protected_flag: number
  readonly policy_eviction_override_flag: number
  readonly policy_reserved_mask: number
  readonly error_status_minimum: number
  readonly set_if_absent_flag: number
  readonly set_if_present_flag: number
}

/** Language-neutral server-visible subset of the OpenKache Smithy model. */
export interface Wire_Contract {
  readonly item_id_bytes: number
  readonly max_value_bytes: number
  /**
   * Operation metadata is optional for legacy AST fixtures. Production
   * protocol generation runs in strict mode and always emits it.
   */
  readonly operations?: readonly Wire_Operation[]
  readonly opcodes: readonly Wire_Entry[]
  readonly statuses: readonly Wire_Entry[]
  readonly v1: Wire_V1_Contract
}

/**
 * Returns the maximum payload bytes a modeled response may occupy.
 *
 * Response budgeting is a property of the output shape, never of the
 * request's input value length.  Optional-value and composite responses carry
 * one length/sentinel prefix per modeled field, while status-only responses
 * carry no payload.  The bound is intentionally conservative for codecs whose
 * exact expansion is not part of protocol-v1 yet.
 */
export function response_payload_bound(
  contract: Pick<Wire_Contract, "max_value_bytes" | "v1">,
  operation: Pick<Wire_Operation, "contract">,
): number {
  const route = derive_wire_response_route(operation.contract)
  const response_value_count = operation.contract.response_value_count
  const composite_field_count = operation.contract.response_plan === undefined
    ? operation.contract.response_fields.reduce(
        (count, field) => count + field.count,
        0,
      )
    : operation.contract.response_plan.length
  const optional_count = route === "field_sequence" || route === "composite"
    ? composite_field_count
    : route === "value"
    ? response_value_count
    : 0
  if (optional_count > 1) {
    const entry_bytes =
      (contract.v1.optional_value_length_bytes ?? 4) + contract.max_value_bytes
    // Protocol v1 applies one aggregate payload ceiling to the complete
    // response, even when the output shape contains multiple optional
    // fields. Keep the shape-derived count while respecting that global cap.
    return Math.min(optional_count * entry_bytes, contract.max_value_bytes)
  }
  return route === "application_value" ||
      route === "field_sequence" ||
      route === "value" ||
      route === "stats_json" ||
      route === "namespace_descriptor"
    ? contract.max_value_bytes
    : 0
}

const PROTOCOL_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const MODEL_DIRECTORY = "model"
const SMITHY_EXECUTABLE = process.env.OPENKACHE_SMITHY_EXECUTABLE ?? "smithy"
const SMITHY_USE_SHELL = process.env.OPENKACHE_SMITHY_USE_SHELL === "1"

function resolve_smithy_executable(): string {
  if (
    SMITHY_EXECUTABLE.length === 0 ||
    !SMITHY_EXECUTABLE.includes("/") ||
    SMITHY_EXECUTABLE.startsWith("/")
  ) {
    return SMITHY_EXECUTABLE
  }
  let directory = resolve(process.cwd())
  for (;;) {
    if (
      SMITHY_EXECUTABLE.startsWith("external/") &&
      existsSync(resolve(directory, "external"))
    ) {
      return resolve(directory, SMITHY_EXECUTABLE)
    }
    const candidate = resolve(directory, SMITHY_EXECUTABLE)
    if (existsSync(candidate)) return candidate
    const parent = dirname(directory)
    if (parent === directory) return SMITHY_EXECUTABLE
    directory = parent
  }
}
const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
const OPCODE_SHAPE_ID = "openkache.protocol#Opcode"
const STATUS_SHAPE_ID = "openkache.protocol#Status"
const WIRE_CONTRACT_TRAIT_ID = "openkache.protocol#wireContract"
const WIRE_OPCODE_TRAIT_ID = "openkache.protocol#wireOpcode"
const WIRE_STATUS_TRAIT_ID = "openkache.protocol#wireStatus"
const OPERATION_CONTRACT_TRAIT_ID = "openkache.protocol#operationContract"
const OPERATION_FIELD_TRAIT_ID = "openkache.protocol#operationField"
const WIRE_CODEC_TRAIT_ID = "openkache.protocol#wireCodec"

function object_value(value: unknown, location: string): Json_Object {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${location} must be an object`)
  }
  return value as Json_Object
}

function object_member(object: Json_Object, member: string, location: string): Json_Object {
  return object_value(object[member], `${location}.${member}`)
}

function array_member(
  object: Json_Object,
  member: string,
  location: string,
): readonly unknown[] {
  const value = object[member]
  if (!Array.isArray(value)) throw new Error(`${location}.${member} must be an array`)
  return value
}

function string_member(object: Json_Object, member: string, location: string): string {
  const value = object[member]
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${location}.${member} must be a non-empty string`)
  }
  return value
}

function shape_type(shape: Json_Object, location: string): string {
  return string_member(shape, "type", location)
}

function integer_member(
  object: Json_Object,
  member: string,
  location: string,
  minimum = 0,
  maximum = Number.MAX_SAFE_INTEGER,
): number {
  const value = object[member]
  if (
    typeof value !== "number" ||
    !Number.isSafeInteger(value) ||
    value < minimum ||
    value > maximum
  ) {
    throw new Error(
      `${location}.${member} must be an integer from ${minimum} through ${maximum}`,
    )
  }
  return value
}

function optional_integer_member(
  object: Json_Object,
  member: string,
  location: string,
  minimum = 0,
  maximum = Number.MAX_SAFE_INTEGER,
): number | undefined {
  return object[member] === undefined
    ? undefined
    : integer_member(object, member, location, minimum, maximum)
}

function shape_name(shape_id: string): string {
  const separator = shape_id.lastIndexOf("#")
  if (separator < 0 || separator === shape_id.length - 1) {
    throw new Error(`shape ID ${JSON.stringify(shape_id)} has no shape name`)
  }
  return shape_id.slice(separator + 1)
}

function pascal_case(identifier: string): string {
  return identifier
    .split("_")
    .map((part) => {
      const normalized = part.toLowerCase()
      return normalized.length === 0
        ? ""
        : `${normalized[0]?.toUpperCase()}${normalized.slice(1)}`
    })
    .join("")
}

function trait_value(
  shape: Json_Object,
  trait_id: string,
  location: string,
): Json_Object {
  const traits = object_member(shape, "traits", location)
  return object_member(traits, trait_id, `${location}.traits`)
}

function optional_enum_value(shape: Json_Object, location: string): string | undefined {
  const traits = object_member(shape, "traits", location)
  const value = traits["smithy.api#enumValue"]
  return value === undefined
    ? undefined
    : string_member(traits, "smithy.api#enumValue", `${location}.traits`)
}

function unique_wire_values(entries: readonly Wire_Entry[], kind: string): void {
  const names = new Set<string>()
  const texts = new Set<string>()
  const values = new Set<number>()
  for (const entry of entries) {
    if (names.has(entry.name)) throw new Error(`duplicate ${kind} name ${entry.name}`)
    if (entry.text !== undefined && texts.has(entry.text)) {
      throw new Error(`duplicate ${kind} enum value ${entry.text}`)
    }
    if (values.has(entry.value)) {
      throw new Error(`duplicate ${kind} wire value ${entry.value}`)
    }
    names.add(entry.name)
    if (entry.text !== undefined) texts.add(entry.text)
    values.add(entry.value)
  }
}

function wire_v1_contract(value: unknown): Wire_V1_Contract {
  const contract = object_value(value, `${WIRE_CONTRACT_TRAIT_ID}.v1`)
  const optional_value_length_bytes = optional_integer_member(
    contract,
    "optionalValueLengthBytes",
    "wireContract.v1",
    1,
    0xff,
  )
  const optional_value_missing = optional_integer_member(
    contract,
    "optionalValueMissing",
    "wireContract.v1",
    0,
    Number.MAX_SAFE_INTEGER,
  )
  const v1 = {
    alpn: string_member(contract, "alpn", "wireContract.v1"),
    opcode_bytes: integer_member(contract, "opcodeBytes", "wireContract.v1", 1, 0xff),
    status_bytes: integer_member(contract, "statusBytes", "wireContract.v1", 1, 0xff),
    request_fixed_bytes: integer_member(
      contract,
      "requestFixedBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    response_fixed_bytes: integer_member(
      contract,
      "responseFixedBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    min_varuint_bytes: integer_member(
      contract,
      "minVaruintBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    max_varuint_bytes: integer_member(contract, "maxVaruintBytes", "wireContract.v1", 1),
    namespace_id_bytes: integer_member(
      contract,
      "namespaceIdBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    namespace_revision_bytes: integer_member(
      contract,
      "namespaceRevisionBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    namespace_name_length_bytes: integer_member(
      contract,
      "namespaceNameLengthBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    namespace_name_max_bytes: integer_member(
      contract,
      "namespaceNameMaxBytes",
      "wireContract.v1",
      0,
      0xff,
    ),
    ...(optional_value_length_bytes === undefined
      ? {}
      : { optional_value_length_bytes }),
    ...(optional_value_missing === undefined ? {} : { optional_value_missing }),
    set_flags_bytes: integer_member(
      contract,
      "setFlagsBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    set_condition_mask: integer_member(
      contract,
      "setConditionMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_condition_any_bits: integer_member(
      contract,
      "setConditionAnyBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_if_absent_flag: integer_member(
      contract,
      "setIfAbsentFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_if_present_flag: integer_member(
      contract,
      "setIfPresentFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_condition_reserved_bits: integer_member(
      contract,
      "setConditionReservedBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_expiration_mask: integer_member(
      contract,
      "setExpirationMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_inherit_expiration_bits: integer_member(
      contract,
      "setInheritExpirationBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_no_expiry_bits: integer_member(
      contract,
      "setNoExpiryBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_ttl_flag: integer_member(contract, "setTtlFlag", "wireContract.v1", 0, 0xff),
    set_expiration_reserved_bits: integer_member(
      contract,
      "setExpirationReservedBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_eviction_mask: integer_member(
      contract,
      "setEvictionMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_inherit_eviction_bits: integer_member(
      contract,
      "setInheritEvictionBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_evictable_bits: integer_member(
      contract,
      "setEvictableBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_eviction_protected_bits: integer_member(
      contract,
      "setEvictionProtectedBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_eviction_reserved_bits: integer_member(
      contract,
      "setEvictionReservedBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    set_reserved_mask: integer_member(
      contract,
      "setReservedMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    open_flags_bytes: integer_member(
      contract,
      "openFlagsBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    open_create_if_missing_flag: integer_member(
      contract,
      "openCreateIfMissingFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    open_reserved_mask: integer_member(
      contract,
      "openReservedMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    delete_flags_bytes: integer_member(
      contract,
      "deleteFlagsBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    delete_if_empty_bits: integer_member(
      contract,
      "deleteIfEmptyBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    delete_mode_mask: integer_member(
      contract,
      "deleteModeMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    delete_reserved_mask: integer_member(
      contract,
      "deleteReservedMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_flags_bytes: integer_member(
      contract,
      "policyFlagsBytes",
      "wireContract.v1",
      1,
      0xff,
    ),
    policy_default_expiration_mask: integer_member(
      contract,
      "policyDefaultExpirationMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_no_expiry_bits: integer_member(
      contract,
      "policyNoExpiryBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_fixed_ttl_bits: integer_member(
      contract,
      "policyFixedTtlBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_default_expiration_reserved_bits: integer_member(
      contract,
      "policyDefaultExpirationReservedBits",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_expiration_override_flag: integer_member(
      contract,
      "policyExpirationOverrideFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_eviction_protected_flag: integer_member(
      contract,
      "policyEvictionProtectedFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_eviction_override_flag: integer_member(
      contract,
      "policyEvictionOverrideFlag",
      "wireContract.v1",
      0,
      0xff,
    ),
    policy_reserved_mask: integer_member(
      contract,
      "policyReservedMask",
      "wireContract.v1",
      0,
      0xff,
    ),
    error_status_minimum: integer_member(
      contract,
      "errorStatusMinimum",
      "wireContract.v1",
      0,
      0xff,
    ),
  } satisfies Wire_V1_Contract
  if (v1.alpn !== "openkache/1") {
    throw new Error(
      `${WIRE_CONTRACT_TRAIT_ID}.v1.alpn must be "openkache/1" for the current protocol implementation`,
    )
  }
  if (
    v1.opcode_bytes !== 1 ||
    v1.status_bytes !== 1 ||
    v1.request_fixed_bytes !== 1 ||
    v1.response_fixed_bytes !== 1
  ) {
    throw new Error(
      `${WIRE_CONTRACT_TRAIT_ID}.v1 opcode, status, request, and response fixed sizes must all be 1`,
    )
  }
  if (v1.min_varuint_bytes !== 1 || v1.max_varuint_bytes !== 9) {
    throw new Error(
      `${WIRE_CONTRACT_TRAIT_ID}.v1 vu128 widths must be minimum=1 and maximum=9 for the unsigned 64-bit protocol`,
    )
  }
  if (
    (v1.optional_value_length_bytes !== undefined &&
      v1.optional_value_length_bytes !== 4) ||
    (v1.optional_value_missing !== undefined &&
      v1.optional_value_missing !== 0xffff_ffff)
  ) {
    throw new Error(
      "wire v1 optional-value framing must use four big-endian length bytes and 0xffffffff as the missing sentinel",
    )
  }
  if (
    v1.namespace_id_bytes !== 8 ||
    v1.namespace_revision_bytes !== 8 ||
    v1.namespace_name_length_bytes !== 1 ||
    v1.namespace_name_max_bytes !== 0xff ||
    v1.set_flags_bytes !== 1 ||
    v1.open_flags_bytes !== 1 ||
    v1.delete_flags_bytes !== 1 ||
    v1.policy_flags_bytes !== 1
  ) {
    throw new Error(
      "wire v1 fixed field widths must be namespace/revision=8, name length and flag fields=1, and name max=255",
    )
  }
  const flag_groups = [
    {
      name: "SET condition",
      mask: v1.set_condition_mask,
      values: [
        v1.set_condition_any_bits,
        v1.set_if_absent_flag,
        v1.set_if_present_flag,
        v1.set_condition_reserved_bits,
      ],
    },
    {
      name: "SET expiration",
      mask: v1.set_expiration_mask,
      values: [
        v1.set_inherit_expiration_bits,
        v1.set_no_expiry_bits,
        v1.set_ttl_flag,
        v1.set_expiration_reserved_bits,
      ],
    },
    {
      name: "SET eviction",
      mask: v1.set_eviction_mask,
      values: [
        v1.set_inherit_eviction_bits,
        v1.set_evictable_bits,
        v1.set_eviction_protected_bits,
        v1.set_eviction_reserved_bits,
      ],
    },
    {
      name: "namespace policy expiration",
      mask: v1.policy_default_expiration_mask,
      values: [
        v1.policy_no_expiry_bits,
        v1.policy_fixed_ttl_bits,
        v1.policy_default_expiration_reserved_bits,
      ],
    },
  ] as const
  for (const group of flag_groups) {
    unique_wire_values(
      group.values.map((value, index) => ({ name: `${group.name} ${index}`, value })),
      group.name,
    )
    if (group.values.some((value) => (value & ~group.mask) !== 0)) {
      throw new Error(`${group.name} values must fit within mask 0x${group.mask.toString(16)}`)
    }
  }
  if (
    v1.set_if_absent_flag !== 0x01 ||
    v1.set_if_present_flag !== 0x02 ||
    v1.set_condition_reserved_bits !== v1.set_condition_mask ||
    v1.set_expiration_reserved_bits !== v1.set_expiration_mask ||
    v1.set_eviction_reserved_bits !== v1.set_eviction_mask ||
    v1.set_reserved_mask !== 0xc0
  ) {
    throw new Error("SET masks and reserved values do not match the v1 bit layout")
  }
  if (
    v1.open_create_if_missing_flag === 0 ||
    v1.open_reserved_mask !== (0xff ^ v1.open_create_if_missing_flag) ||
    v1.delete_if_empty_bits !== 0 ||
    v1.delete_reserved_mask !== (0xff ^ v1.delete_mode_mask) ||
    v1.policy_expiration_override_flag !== 0x04 ||
    v1.policy_eviction_protected_flag !== 0x08 ||
    v1.policy_eviction_override_flag !== 0x10 ||
    v1.policy_reserved_mask !== 0xe0
  ) {
    throw new Error("namespace open/delete/policy flags do not match the v1 bit layout")
  }
  if (v1.error_status_minimum !== 0x80) {
    throw new Error("wire v1 errorStatusMinimum must be 0x80")
  }
  return v1
}

function wire_enum_entries(
  shapes: Json_Object,
  shape_id: string,
  trait_id: string,
  kind: string,
): readonly Wire_Entry[] {
  const enum_shape = object_member(shapes, shape_id, "Smithy AST.shapes")
  const members = object_member(enum_shape, "members", shape_id)
  const entries = Object.entries(members)
    .map(([name, member]): Wire_Entry => {
      const member_shape = object_value(member, `${shape_id}.${name}`)
      const trait = trait_value(member_shape, trait_id, `${shape_id}.${name}`)
      return {
        name: pascal_case(name),
        text: optional_enum_value(member_shape, `${shape_id}.${name}`),
        value: integer_member(
          trait,
          "value",
          `${shape_id}.${name}.${trait_id}`,
          0,
          0xff,
        ),
      }
    })
    .sort((left, right) => left.value - right.value)
  unique_wire_values(entries, kind)
  if (entries.length === 0) throw new Error(`${kind} contract must define at least one entry`)
  return entries
}

function optional_object_member(
  object: Json_Object,
  member: string,
  location: string,
): Json_Object | undefined {
  const value = object[member]
  return value === undefined ? undefined : object_value(value, `${location}.${member}`)
}

function operation_shape_field_plan(
  shapes: Json_Object,
  operation_shape: Json_Object,
  operation_target: string,
  direction: "input" | "output",
): readonly Wire_Operation_Field_Plan[] {
  const shape_reference = object_member(
    operation_shape,
    direction,
    operation_target,
  )
  const shape_target = string_member(
    shape_reference,
    "target",
    `${operation_target}.${direction}`,
  )
  const structure = object_member(shapes, shape_target, "Smithy AST.shapes")
  if (shape_type(structure, `Smithy AST.shapes.${shape_target}`) !== "structure") {
    throw new Error(`${shape_target} must be a structure`)
  }
  const fields: Wire_Operation_Field_Plan[] = []
  const visit = (
    target: string,
    path: readonly string[],
    ancestors: ReadonlySet<string>,
    required_parent: boolean,
  ): void => {
    if (ancestors.has(target)) {
      throw new Error(`${operation_target}.${direction} shape cycle through ${target}`)
    }
    const next_ancestors = new Set(ancestors).add(target)
    const current = object_member(shapes, target, "Smithy AST.shapes")
    const members = object_member(current, "members", target)
    for (const [member_name, value] of Object.entries(members)) {
      const member = object_value(value, `${target}.${member_name}`)
      const traits = optional_object_member(member, "traits", `${target}.${member_name}`)
      const field = traits?.[OPERATION_FIELD_TRAIT_ID]
      if (field !== undefined) {
        const role = string_member(
          object_value(field, `${target}.${member_name}.${OPERATION_FIELD_TRAIT_ID}`),
          "role",
          `${target}.${member_name}.${OPERATION_FIELD_TRAIT_ID}`,
        )
        const member_target = string_member(
          member,
          "target",
          `${target}.${member_name}`,
        )
        const codecs: string[] = []
        const codec = traits?.[WIRE_CODEC_TRAIT_ID]
        if (codec !== undefined) {
          const codec_name = string_member(
            object_value(codec, `${target}.${member_name}.${WIRE_CODEC_TRAIT_ID}`),
            "name",
            `${target}.${member_name}.${WIRE_CODEC_TRAIT_ID}`,
          )
          codecs.push(codec_name)
        }
        fields.push({
          index: fields.length,
          ...(codecs.length === 0 ? {} : { codecs }),
          path: [...path, member_name],
          required: required_parent && traits?.["smithy.api#required"] !== undefined,
          role,
          shape: shape_name(member_target),
        })
      }
      const nested_target = member["target"]
      if (typeof nested_target === "string") {
        const nested = shapes[nested_target]
        if (
          nested !== undefined &&
          shape_type(
            object_value(nested, `Smithy AST.shapes.${nested_target}`),
            `Smithy AST.shapes.${nested_target}`,
          ) === "structure"
        ) {
          visit(
            nested_target,
            [...path, member_name],
            next_ancestors,
            required_parent && traits?.["smithy.api#required"] !== undefined,
          )
        }
      }
    }
  }
  visit(shape_target, [], new Set(), true)
  return fields
}

function operation_shape_fields(
  shapes: Json_Object,
  operation_shape: Json_Object,
  operation_target: string,
  direction: "input" | "output",
): readonly Wire_Operation_Field[] {
  const fields = operation_shape_field_plan(
    shapes,
    operation_shape,
    operation_target,
    direction,
  )
  const grouped = new Map<string, { count: number; codecs: string[] }>()
  for (const field of fields) {
    const entry = grouped.get(field.role) ?? { count: 0, codecs: [] }
    entry.count += 1
    entry.codecs.push(...(field.codecs ?? []))
    grouped.set(field.role, entry)
  }
  return [...grouped].map(([role, entry]) => ({
    count: entry.count,
    role,
    ...(entry.codecs.length === 0 ? {} : { codecs: entry.codecs }),
  }))
}

function operation_field_count(
  fields: readonly Wire_Operation_Field[],
  role: string,
): number {
  return fields.find((field) => field.role === role)?.count ?? 0
}

function operation_contract(
  shapes: Json_Object,
  shape: Json_Object,
  target: string,
  statuses: readonly Wire_Entry[],
  strict: boolean,
): Wire_Operation_Contract | undefined {
  const traits = optional_object_member(shape, "traits", target)
  const value = traits?.[OPERATION_CONTRACT_TRAIT_ID]
  if (value === undefined) return undefined
  const contract = object_value(value, `${target}.traits.${OPERATION_CONTRACT_TRAIT_ID}`)
  const scope = string_member(contract, "scope", `${target}.${OPERATION_CONTRACT_TRAIT_ID}`)
  if (!OPERATION_SCOPES.includes(scope as Wire_Operation_Scope)) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.scope must be global, item, namespace, or namespace_management`,
    )
  }
  const retry_mode = string_member(
    contract,
    "retryMode",
    `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
  )
  if (!OPERATION_RETRY_MODES.includes(retry_mode as Wire_Operation_Retry_Mode)) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.retryMode must be always, never, or when_not_creating`,
    )
  }
  const effect_value = contract["effect"]
  const effect = effect_value === undefined
    ? (strict
      ? (() => {
          throw new Error(
            `${target}.${OPERATION_CONTRACT_TRAIT_ID}.effect must be read_only, mutation, or barrier`,
          )
        })()
      : "read_only")
    : string_member(
      contract,
      "effect",
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
    )
  if (!OPERATION_EFFECTS.includes(effect as Wire_Operation_Effect)) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}.effect must be read_only, mutation, or barrier`,
    )
  }

  const request_fields = operation_shape_fields(
    shapes,
    shape,
    target,
    "input",
  )
  const request_plan = operation_shape_field_plan(
    shapes,
    shape,
    target,
    "input",
  )
  const response_fields = operation_shape_fields(
    shapes,
    shape,
    target,
    "output",
  )
  const response_plan = operation_shape_field_plan(
    shapes,
    shape,
    target,
    "output",
  )
  const request_item_count = operation_field_count(request_fields, "item_id")
  const request_value_count = operation_field_count(request_fields, "value")
  const response_value_count = operation_field_count(response_fields, "value")
  const status_names = new Set(
    statuses.flatMap((status) => [
      status.name,
      status.text ?? wire_name(status.name),
    ]),
  )
  const status_values = (member: string): readonly string[] => {
    const values = array_member(
      contract,
      member,
      `${target}.${OPERATION_CONTRACT_TRAIT_ID}`,
    ).map((value, index) => {
      if (typeof value !== "string" || value.length === 0) {
        throw new Error(
          `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member}[${index}] must be a non-empty string`,
        )
      }
      if (!status_names.has(value)) {
        throw new Error(
          `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member}[${index}] references unknown protocol status ${value}`,
        )
      }
      return value
    })
    if (new Set(values).size !== values.length) {
      throw new Error(
        `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member} must not contain duplicate statuses`,
      )
    }
    if (values.length === 0) {
      throw new Error(
        `${target}.${OPERATION_CONTRACT_TRAIT_ID}.${member} must not be empty`,
      )
    }
    return values
  }
  const success_statuses = status_values("successStatuses")
  const error_statuses = status_values("errorStatuses")
  if (success_statuses.some((status) => error_statuses.includes(status))) {
    throw new Error(
      `${target}.${OPERATION_CONTRACT_TRAIT_ID} has overlapping success and error statuses`,
    )
  }
  const legacy_request_kind = typeof contract["requestKind"] === "string"
    ? contract["requestKind"] as string
    : undefined
  const legacy_response_kind = typeof contract["responseKind"] === "string"
    ? contract["responseKind"] as string
    : undefined
  const derived_contract = {
    error_statuses,
    request_fields,
    request_plan,
    request_value_count,
    request_item_count,
    response_fields,
    response_plan,
    response_value_count,
    retry_mode: retry_mode as Wire_Operation_Contract["retry_mode"],
    scope: scope as Wire_Operation_Contract["scope"],
    success_statuses,
    effect: effect as Wire_Operation_Effect,
  }
  const layout_contract = {
    ...derived_contract,
    request_kind: "",
    response_kind: "",
  } as Wire_Operation_Contract
  return {
    ...derived_contract,
    request_kind: request_fields.length > 0 || legacy_request_kind === undefined
      ? derive_wire_request_layout(layout_contract)
      : legacy_request_kind,
    response_kind: response_fields.length > 0 || legacy_response_kind === undefined
      ? derive_wire_response_route(layout_contract)
      : legacy_response_kind,
  }
}

function wire_operations(
  shapes: Json_Object,
  opcodes: readonly Wire_Entry[],
  statuses: readonly Wire_Entry[],
  strict: boolean,
): readonly Wire_Operation[] | undefined {
  const operations: Wire_Operation[] = []
  for (const opcode of opcodes) {
    const target = `${SERVICE_SHAPE_ID.slice(0, SERVICE_SHAPE_ID.lastIndexOf("#"))}#${opcode.name}`
    const shape = shapes[target]
    if (shape === undefined) {
      if (strict) throw new Error(`opcode ${opcode.name} has no matching Smithy operation`)
      return undefined
    }
    const contract = operation_contract(
      shapes,
      object_value(shape, `Smithy AST.shapes.${target}`),
      target,
      statuses,
      strict,
    )
    if (contract === undefined) {
      if (strict) {
        throw new Error(`operation ${opcode.name} is missing ${OPERATION_CONTRACT_TRAIT_ID}`)
      }
      return undefined
    }
    operations.push({ contract, name: opcode.name })
  }
  return operations
}

/** Extracts the server-visible wire contract from a Smithy AST. */
export function extract_wire_contract(ast: unknown, strict_operations = false): Wire_Contract {
  const ast_object = object_value(ast, "Smithy AST")
  const shapes = object_member(ast_object, "shapes", "Smithy AST")
  const service = object_member(shapes, SERVICE_SHAPE_ID, "Smithy AST.shapes")
  const contract_trait = trait_value(
    service,
    WIRE_CONTRACT_TRAIT_ID,
    `Smithy AST.shapes.${SERVICE_SHAPE_ID}`,
  )
  const opcode_shape = shapes[OPCODE_SHAPE_ID]
  const opcodes =
    opcode_shape === undefined
      ? array_member(service, "operations", SERVICE_SHAPE_ID)
          .map((operation, index): Wire_Entry => {
            const reference = object_value(operation, `${SERVICE_SHAPE_ID}.operations[${index}]`)
            const target = string_member(
              reference,
              "target",
              `${SERVICE_SHAPE_ID}.operations[${index}]`,
            )
            const operation_shape = object_member(shapes, target, "Smithy AST.shapes")
            const trait = trait_value(
              operation_shape,
              WIRE_OPCODE_TRAIT_ID,
              `Smithy AST.shapes.${target}`,
            )
            return {
              name: pascal_case(shape_name(target)),
              value: integer_member(
                trait,
                "value",
                `${target}.${WIRE_OPCODE_TRAIT_ID}`,
                0,
                0xff,
              ),
            }
          })
          .sort((left, right) => left.value - right.value)
      : wire_enum_entries(
          shapes,
          OPCODE_SHAPE_ID,
          WIRE_OPCODE_TRAIT_ID,
          "opcode",
        )
  unique_wire_values(opcodes, "opcode")
  if (opcodes.length === 0) throw new Error("opcode contract must define at least one entry")
  const statuses = wire_enum_entries(
    shapes,
    STATUS_SHAPE_ID,
    WIRE_STATUS_TRAIT_ID,
    "status",
  )
  const contract = {
    item_id_bytes: integer_member(contract_trait, "itemIdBytes", "wireContract", 1),
    max_value_bytes: integer_member(contract_trait, "maxValueBytes", "wireContract", 1),
    opcodes,
    statuses,
    v1: wire_v1_contract(contract_trait.v1),
  }
  const operations = wire_operations(
    shapes,
    opcodes,
    statuses,
    strict_operations,
  )
  return operations === undefined ? contract : { ...contract, operations }
}

/** Loads the protocol Smithy AST from the model owned by this directory. */
export function smithy_wire_ast(): unknown {
  const smithy_executable = resolve_smithy_executable()
  const smithy_command =
    SMITHY_USE_SHELL && process.platform !== "win32"
      ? ["sh", smithy_executable, "ast", MODEL_DIRECTORY]
      : [smithy_executable, "ast", MODEL_DIRECTORY]
  const result = Bun.spawnSync(smithy_command, {
    cwd: PROTOCOL_DIRECTORY,
    stderr: "pipe",
    stdout: "pipe",
  })
  if (result.exitCode !== 0) {
    const diagnostics = result.stderr.toString().trim()
    throw new Error(
      diagnostics.length === 0
        ? "`smithy ast` exited without diagnostics"
        : `smithy AST generation failed:\n${diagnostics}`,
    )
  }
  try {
    return JSON.parse(result.stdout.toString()) as unknown
  } catch (error) {
    throw new Error(`smithy emitted invalid JSON: ${String(error)}`)
  }
}

function formatted_decimal(value: number): string {
  return value.toString().replace(/\B(?=(\d{3})+(?!\d))/g, "_")
}

function formatted_byte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`
}

function wire_name(identifier: string): string {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase()
}

function rust_byte_string_literal(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let literal = 'b"'
  for (const byte of bytes) {
    if (byte >= 0x20 && byte <= 0x7e && byte !== 0x22 && byte !== 0x5c) {
      literal += String.fromCharCode(byte)
    } else {
      literal += `\\x${byte.toString(16).padStart(2, "0")}`
    }
  }
  return `${literal}"`
}

function rust_string_literal(value: string): string {
  let literal = '"'
  for (const character of value) {
    switch (character) {
      case '"':
        literal += '\\"'
        break
      case "\\":
        literal += "\\\\"
        break
      case "\n":
        literal += "\\n"
        break
      case "\r":
        literal += "\\r"
        break
      case "\t":
        literal += "\\t"
        break
      default: {
        const code_point = character.codePointAt(0) ?? 0
        literal +=
          code_point < 0x20
            ? `\\u{${code_point.toString(16)}}`
            : character
        break
      }
    }
  }
  return `${literal}"`
}

function rust_wire_enum(
  name: string,
  documentation: string,
  entries: readonly Wire_Entry[],
  unknown_variant: string,
): string {
  const variants = entries
    .map((entry) => `        ${entry.name} = ${formatted_byte(entry.value)},`)
    .join("\n")
  const all_variants = entries.map((entry) => `        Self::${entry.name},`).join("\n")
  const name_literals = entries
    .map((entry) => `        ${rust_string_literal(entry.text ?? wire_name(entry.name))},`)
    .join("\n")
  const names = entries
    .map(
      (entry) =>
        `        Self::${entry.name} => ${rust_string_literal(entry.text ?? wire_name(entry.name))},`,
    )
    .join("\n")
  return `wire_enum! {
    /// ${documentation}
    pub enum ${name} {
${variants}
    }
    unknown => ${unknown_variant}
}

impl ${name} {
    /// Number of values assigned by the Smithy ${name} contract.
    pub const COUNT: usize = ${entries.length};

    /// Every assigned Smithy ${name} value in wire-value order.
    pub const ALL: [Self; Self::COUNT] = [
${all_variants}
    ];

    /// Stable lowercase Smithy names in wire-value order.
    pub const NAMES: [&'static str; Self::COUNT] = [
${name_literals}
    ];

    /// Zero-based position in the Smithy value-order arrays.
    ///
    /// Wire values are intentionally allowed to be sparse. Callers that use
    /// an enum as an array index must use this generated position instead of
    /// the wire discriminant.
    pub const fn index(self) -> usize {
        match self {
${entries
  .map((entry, index) => `        Self::${entry.name} => ${index},`)
  .join("\n")}
        }
    }

    /// Stable lowercase Smithy name for this assigned value.
    pub const fn name(self) -> &'static str {
        match self {
${names}
        }
    }
}`
}

function rust_operation_contract(contract: Wire_Contract): string {
  const operations = contract.operations
  if (operations === undefined) return ""
  const max_request_fields = operations.reduce(
    (maximum, operation) => Math.max(
      maximum,
      operation.contract.request_plan?.length ??
        operation.contract.request_fields.reduce(
          (count, field) => count + field.count,
          0,
        ),
    ),
    0,
  )
  const status_variant = (status: string): string => {
    const entry = contract.statuses.find(
      (candidate) =>
        candidate.name === status ||
        candidate.text === status ||
        wire_name(candidate.name) === status,
    )
    if (entry === undefined) {
      throw new Error(`operation metadata references unknown status ${status}`)
    }
    return entry.name
  }
  const enum_variant = (value: string): string =>
    pascal_case(value)
  const request_route_variant = (operation: Wire_Operation): string =>
    pascal_case(derive_wire_request_layout(operation.contract))
  const response_route_variant = (operation: Wire_Operation): string =>
    pascal_case(derive_wire_response_route(operation.contract))
  const role_names = [
    ...new Set(
      operations.flatMap((operation) => [
        ...(operation.contract.request_plan ?? []),
        ...(operation.contract.response_plan ?? []),
      ]).map((field) => field.role),
    ),
  ]
  const role_variant_names = new Map<string, string>()
  const used_role_variants = new Set<string>()
  for (const [index, role] of role_names.entries()) {
    let variant = pascal_case(role.replace(/[^A-Za-z0-9_]+/g, "_"))
    if (variant.length === 0 || /^[0-9]/.test(variant)) {
      variant = `Role${index}${variant}`
    }
    if (["Self", "Super", "Crate", "Where", "Loop", "Match", "Ref", "Type"].includes(variant)) {
      variant = `Role${variant}`
    }
    while (used_role_variants.has(variant)) {
      variant = `${variant}${index}`
    }
    used_role_variants.add(variant)
    role_variant_names.set(role, variant)
  }
  const request_layout = (operation: Wire_Operation): string =>
    derive_wire_request_layout(operation.contract)
  const response_route = (operation: Wire_Operation): string =>
    derive_wire_response_route(operation.contract)
  const optional_value_count = (operation: Wire_Operation): number => {
    const ordered_count = operation.contract.response_plan?.length
    switch (derive_wire_response_route(operation.contract)) {
      case "field_sequence":
      case "composite":
        return ordered_count ?? operation.contract.response_fields.reduce(
          (count, field) => count + field.count,
          0,
        )
      case "value":
        return operation.contract.response_value_count
      default:
        return 0
    }
  }
  const status_slice = (statuses: readonly string[]): string =>
    `&[${statuses
      .map((status) => `Status::${status_variant(status)}`)
      .join(", ")}]`
  const field_slice = (fields: readonly Wire_Operation_Field[]): string =>
    `&[${fields
      .map(
        (field) =>
          `OperationField { role: ${rust_string_literal(field.role)}, count: ${field.count}, codecs: &[${(field.codecs ?? [])
            .map(rust_string_literal)
            .join(", ")}] }`,
      )
      .join(", ")}]`
  const plan_slice = (
    fields: readonly Wire_Operation_Field_Plan[] | undefined,
  ): string =>
    `&[${(fields ?? [])
      .map(
        (field) =>
          `OperationFieldPlan { role: ${rust_string_literal(field.role)}, required: ${field.required}, shape: ${rust_string_literal(field.shape)}, path: &[${field.path
            .map(rust_string_literal)
            .join(", ")}], index: ${field.index}, role_id: OperationFieldRole::${role_variant_names.get(field.role)}, codecs: &[${(field.codecs ?? [])
            .map(rust_string_literal)
            .join(", ")}] }`,
      )
      .join(", ")}]`
  const metadata = operations
    .map(
      (operation) => `    OperationContract {
            scope: OperationScope::${enum_variant(operation.contract.scope)},
            request_kind: ${rust_string_literal(request_layout(operation))},
            request_route: OperationRequestRoute::${request_route_variant(operation)},
            request_value_count: ${operation.contract.request_value_count ?? 0},
            request_item_count: ${operation.contract.request_item_count},
            request_fields: ${field_slice(operation.contract.request_fields)},
            request_plan: ${plan_slice(operation.contract.request_plan)},
            response_kind: ${rust_string_literal(response_route(operation))},
            response_route: OperationResponseRoute::${response_route_variant(operation)},
            response_value_count: ${operation.contract.response_value_count},
            response_payload_bound: ${formatted_decimal(response_payload_bound(contract, operation))},
            response_fields: ${field_slice(operation.contract.response_fields)},
            response_plan: ${plan_slice(operation.contract.response_plan)},
            retry_mode: OperationRetryMode::${enum_variant(operation.contract.retry_mode)},
            effect: OperationEffect::${enum_variant(operation.contract.effect)},
            success_statuses: ${status_slice(operation.contract.success_statuses)},
            error_statuses: ${status_slice(operation.contract.error_statuses)},
        }`,
    )
    .join(",\n")
  const optional_value_metadata = operations
    .map(
      (operation) => `    ${optional_value_count(operation)}`,
    )
    .join(",\n")
  return `/// Maximum number of ordered request fields in any modeled operation.
///
/// Server operation views use this generated bound for a stack-resident field
/// record array, keeping the request hot path allocation-free while allowing
/// the Smithy model to grow the array when a new shape needs more fields.
pub const MAX_OPERATION_REQUEST_FIELDS: usize = ${max_request_fields};

/// Request scope declared by the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationScope {
    Global,
    Item,
    Namespace,
    NamespaceManagement,
}

/// Retry behavior declared by the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationRetryMode {
    Always,
    Never,
    WhenNotCreating,
}

/// Storage effect declared by the Smithy operation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationEffect {
    ReadOnly,
    Mutation,
    Barrier,
}

/// Generated numeric key for a semantic operation-field role.
///
/// Role names remain open strings in the Smithy model.  This enum is a
/// generated index, not a hand-maintained infrastructure registry; adding a
/// role regenerates its key and leaves the runtime lookup allocation-free.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum OperationFieldRole {
${role_names
  .map((role) => `    ${role_variant_names.get(role)},`)
  .join("\n")}
}

impl OperationFieldRole {
    /// Returns the dense generated role index.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Returns the original Smithy role string.
    pub const fn name(self) -> &'static str {
        match self {
${role_names
  .map((role) => `            Self::${role_variant_names.get(role)} => ${rust_string_literal(role)},`)
  .join("\n")}
        }
    }

    /// Resolves a role string at a compatibility boundary.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
${role_names
  .map((role) => `            ${rust_string_literal(role)} => Some(Self::${role_variant_names.get(role)}),`)
  .join("\n")}
            _ => None,
        }
    }
}

/// Transport request routes generated from Smithy field plans.
///
/// These are wire primitives, not API families. A new operation reuses one of
/// these routes without adding an operation-specific branch to the server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationRequestRoute {
    Empty,
    ApplicationValue,
    FieldSequence,
    Item,
    Set,
    Namespace,
    NamespaceOpen,
    NamespaceUpdatePolicy,
    NamespaceDelete,
}

/// Transport response routes generated from Smithy output field plans.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationResponseRoute {
    Empty,
    Pong,
    ApplicationValue,
    FieldSequence,
    Composite,
    Value,
    SetOutcome,
    DeleteOutcome,
    StatsJson,
    NamespaceDescriptor,
}

/// One ordered Smithy operation-field role and its cardinality.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationField {
    pub role: &'static str,
    pub count: usize,
    pub codecs: &'static [&'static str],
}

/// One ordered field in a generated request or response plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationFieldPlan {
    pub index: usize,
    pub role: &'static str,
    pub role_id: OperationFieldRole,
    pub required: bool,
    pub shape: &'static str,
    pub path: &'static [&'static str],
    pub codecs: &'static [&'static str],
}

/// Generated semantic metadata for one protocol operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationContract {
    pub scope: OperationScope,
    /// Generated transport descriptor derived from the Smithy input field plan.
    ///
    /// This is intentionally an open string descriptor rather than a closed
    /// Rust enum.  Modelled operations may compose the existing wire
    /// primitives without extending shared infrastructure.
    pub request_kind: &'static str,
    pub request_route: OperationRequestRoute,
    pub request_value_count: usize,
    pub request_item_count: usize,
    pub request_fields: &'static [OperationField],
    pub request_plan: &'static [OperationFieldPlan],
    /// Generated transport descriptor derived from the Smithy output field plan.
    pub response_kind: &'static str,
    pub response_route: OperationResponseRoute,
    pub response_value_count: usize,
    /// Conservative maximum response payload bytes derived from the output shape.
    pub response_payload_bound: usize,
    pub response_fields: &'static [OperationField],
    pub response_plan: &'static [OperationFieldPlan],
    pub retry_mode: OperationRetryMode,
    pub effect: OperationEffect,
    pub success_statuses: &'static [Status],
    pub error_statuses: &'static [Status],
}

/// Returns the generated contract for a protocol operation.
pub const OPERATION_CONTRACTS: [OperationContract; Opcode::COUNT] = [
${metadata}
];

/// Returns the generated contract for a protocol operation.
pub const fn operation_contract(opcode: Opcode) -> OperationContract {
    OPERATION_CONTRACTS[opcode.index()]
}

/// Returns the number of modeled value fields in an operation response.
///
/// This count is derived from the Smithy output field plan. A single-value
/// lookup keeps its legacy raw-value framing but still returns one so clients
/// can distinguish a missing value; opaque, status-only, and descriptor
/// responses return zero.
pub const fn operation_response_field_count(opcode: Opcode) -> usize {
    [
${optional_value_metadata}
    ][opcode.index()]
}
`
}

/**
 * Renders only the request metadata needed to find a v1 frame boundary.
 *
 * This is intentionally separate from the semantic operation contract below.
 * The server runtime must not inspect response meanings, retry policy, or
 * server behavior just to read a request from a stream. Those fields remain
 * available to the generated server adapter, which owns this request-only
 * descriptor.
 */
function rust_request_layout(contract: Wire_Contract): string {
  const operations = contract.operations
  if (operations === undefined) return ""
  const step_expression = (operation: Wire_Operation): string => {
    const kind = derive_wire_request_layout(operation.contract)
    const fixed = (bytes: string): string =>
      `WireRequestStep::Fixed { bytes: ${bytes} }`
    switch (kind) {
      case "empty":
        return `[${fixed("OPCODE_BYTES")}]`
      case "application_value":
        return `[${fixed("OPCODE_BYTES")}, WireRequestStep::ValueLength]`
      case "field_sequence":
        return `[${fixed("OPCODE_BYTES")}, WireRequestStep::ValueLength]`
      case "item":
        return `[
            ${fixed(
              "OPCODE_BYTES + NAMESPACE_ID_BYTES + ITEM_ID_BYTES * " +
                operation.contract.request_item_count,
            )},
        ]`
      case "set":
        return `[
            ${fixed(
              "OPCODE_BYTES + NAMESPACE_ID_BYTES + SET_FLAGS_BYTES + ITEM_ID_BYTES",
            )},
            WireRequestStep::ConditionalVarUInt {
                selector_offset: OPCODE_BYTES + NAMESPACE_ID_BYTES,
                mask: SET_EXPIRATION_MASK,
                expected: SET_EXPLICIT_TTL_BITS,
            },
            WireRequestStep::ValueLength,
        ]`
      case "namespace":
        return `[${fixed("OPCODE_BYTES + NAMESPACE_ID_BYTES")}]`
      case "namespace_open":
        return `[
            ${fixed("OPCODE_BYTES + OPEN_FLAGS_BYTES")},
            WireRequestStep::ByteLength,
            WireRequestStep::ConditionalByteThenVarUInt {
                selector_offset: OPCODE_BYTES,
                mask: OPEN_CREATE_IF_MISSING,
                expected: OPEN_CREATE_IF_MISSING,
                prefix_bytes: POLICY_FLAGS_BYTES,
                value_mask: POLICY_DEFAULT_EXPIRATION_MASK,
                value_expected: POLICY_FIXED_TTL,
            },
        ]`
      case "namespace_update_policy":
        return `[
            ${fixed("OPCODE_BYTES + NAMESPACE_ID_BYTES + NAMESPACE_REVISION_BYTES")},
            WireRequestStep::ByteThenVarUInt {
                prefix_bytes: POLICY_FLAGS_BYTES,
                mask: POLICY_DEFAULT_EXPIRATION_MASK,
                expected: POLICY_FIXED_TTL,
            },
        ]`
      case "namespace_delete":
        return `[
            ${fixed(
              "OPCODE_BYTES + DELETE_FLAGS_BYTES + NAMESPACE_ID_BYTES + NAMESPACE_REVISION_BYTES",
            )},
        ]`
      default:
        return `[]`
    }
  }
  const metadata = operations
    .map(
      (operation) => `    WireRequestLayout {
            steps: &${step_expression(operation)},
        }`,
    )
    .join(",\n")
  return `/// Primitive request parsing steps generated from the wire layout.
///
/// These steps describe only byte consumption. They do not assign a meaning
/// to namespace IDs, item IDs, flags, policies, or response behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireRequestStep {
    Fixed { bytes: usize },
    ValueLength,
    ConditionalVarUInt {
        selector_offset: usize,
        mask: u8,
        expected: u8,
    },
    ByteLength,
        ByteThenVarUInt {
            prefix_bytes: usize,
            mask: u8,
            expected: u8,
        },
    ConditionalByteThenVarUInt {
        selector_offset: usize,
        mask: u8,
        expected: u8,
        prefix_bytes: usize,
        value_mask: u8,
        value_expected: u8,
    },
}

/// Generated request metadata used only to delimit protocol v1 frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireRequestLayout {
    pub steps: &'static [WireRequestStep],
}

/// Returns the wire-level request layout for one assigned opcode.
pub const WIRE_REQUEST_LAYOUTS: [WireRequestLayout; Opcode::COUNT] = [
${metadata}
];

/// Returns the wire-level request layout for one assigned opcode.
pub const fn wire_request_layout(opcode: Opcode) -> WireRequestLayout {
    WIRE_REQUEST_LAYOUTS[opcode.index()]
}
`
}

/** Computes the bounded receive size from modeled wire layouts. */
function max_request_frame_bytes_for_contract(contract: Wire_Contract): number {
  const v1 = contract.v1
  const policy_bytes = v1.policy_flags_bytes + v1.max_varuint_bytes
  const operations = contract.operations
  if (operations === undefined || operations.length === 0) {
    // Legacy AST fixtures may omit operations. Keep the historical bound for
    // those fixtures; production extraction always takes the modeled path.
    const set_prefix =
      v1.request_fixed_bytes +
      v1.namespace_id_bytes +
      v1.set_flags_bytes +
      contract.item_id_bytes +
      v1.max_varuint_bytes +
      v1.max_varuint_bytes
    const namespace_open_prefix =
      v1.opcode_bytes +
      v1.open_flags_bytes +
      v1.namespace_name_length_bytes +
      v1.namespace_name_max_bytes +
      policy_bytes
    return Math.max(
      set_prefix + contract.max_value_bytes,
      namespace_open_prefix,
    )
  }

  const sizes = operations.map((operation) => {
    const { request_item_count, request_value_count } = operation.contract
    const kind = derive_wire_request_layout(operation.contract)
    switch (kind) {
      case "empty":
        return v1.opcode_bytes
      case "application_value":
        return v1.opcode_bytes + v1.max_varuint_bytes + contract.max_value_bytes
      case "field_sequence":
        return v1.opcode_bytes + v1.max_varuint_bytes + contract.max_value_bytes
      case "item":
      case "set": {
        const item_count = request_item_count * contract.item_id_bytes
        const is_value_request = kind === "set" ||
          (request_value_count ?? 0) > 0
        const prefix =
          v1.opcode_bytes +
          v1.namespace_id_bytes +
          item_count +
          (is_value_request ? v1.set_flags_bytes : 0) +
          (is_value_request ? v1.max_varuint_bytes : 0) +
          (is_value_request ? v1.max_varuint_bytes : 0)
        return prefix + (is_value_request ? contract.max_value_bytes : 0)
      }
      case "namespace":
        return v1.opcode_bytes + v1.namespace_id_bytes
      case "namespace_open":
        return (
          v1.opcode_bytes +
          v1.open_flags_bytes +
          v1.namespace_name_length_bytes +
          v1.namespace_name_max_bytes +
          policy_bytes
        )
      case "namespace_update_policy":
        return v1.opcode_bytes + v1.namespace_id_bytes + v1.namespace_revision_bytes + policy_bytes
      case "namespace_delete":
        return v1.opcode_bytes + v1.delete_flags_bytes + v1.namespace_id_bytes +
          v1.namespace_revision_bytes
      default:
        // Unknown request labels remain safe to frame from their role shape.
        // A role-less operation consumes only its opcode; unknown item/value
        // shapes have already been normalized to the SET/ITEM branches.
        return v1.opcode_bytes
    }
  })
  return Math.max(...sizes)
}

/** Computes a conservative complete response-frame bound from output shapes. */
function max_response_frame_bytes_for_contract(contract: Wire_Contract): number {
  const operations = contract.operations
  const maximum_payload = operations === undefined || operations.length === 0
    ? contract.max_value_bytes
    : Math.max(
      ...operations.map((operation) => response_payload_bound(contract, operation)),
    )
  return (
    contract.v1.status_bytes +
    contract.v1.max_varuint_bytes +
    maximum_payload
  )
}

/** Renders operation-specific protocol constants for semantic adapters. */
export function render_rust_semantic_constants(contract: Wire_Contract): string {
  const v1 = contract.v1
  return `/// Maximum UTF-8 octets accepted in a namespace name.
pub const NAMESPACE_NAME_MAX_BYTES: usize = ${formatted_decimal(v1.namespace_name_max_bytes)};

/// Width of the SET flags field.
pub const SET_FLAGS_BYTES: usize = ${formatted_decimal(v1.set_flags_bytes)};
pub const SET_CONDITION_MASK: u8 = ${formatted_byte(v1.set_condition_mask)};
pub const SET_CONDITION_ANY_BITS: u8 = ${formatted_byte(v1.set_condition_any_bits)};
pub const SET_IF_ABSENT_BITS: u8 = ${formatted_byte(v1.set_if_absent_flag)};
pub const SET_IF_PRESENT_BITS: u8 = ${formatted_byte(v1.set_if_present_flag)};
pub const SET_CONDITION_RESERVED_BITS: u8 = ${formatted_byte(v1.set_condition_reserved_bits)};
pub const SET_EXPIRATION_MASK: u8 = ${formatted_byte(v1.set_expiration_mask)};
pub const SET_INHERIT_EXPIRATION_BITS: u8 = ${formatted_byte(v1.set_inherit_expiration_bits)};
pub const SET_NO_EXPIRY_BITS: u8 = ${formatted_byte(v1.set_no_expiry_bits)};
pub const SET_EXPLICIT_TTL_BITS: u8 = ${formatted_byte(v1.set_ttl_flag)};
pub const SET_EXPIRATION_RESERVED_BITS: u8 = ${formatted_byte(v1.set_expiration_reserved_bits)};
pub const SET_EVICTION_MASK: u8 = ${formatted_byte(v1.set_eviction_mask)};
pub const SET_INHERIT_EVICTION_BITS: u8 = ${formatted_byte(v1.set_inherit_eviction_bits)};
pub const SET_EVICTABLE_BITS: u8 = ${formatted_byte(v1.set_evictable_bits)};
pub const SET_EVICTION_PROTECTED_BITS: u8 = ${formatted_byte(v1.set_eviction_protected_bits)};
pub const SET_EVICTION_RESERVED_BITS: u8 = ${formatted_byte(v1.set_eviction_reserved_bits)};
pub const SET_RESERVED_MASK: u8 = ${formatted_byte(v1.set_reserved_mask)};

/// Namespace-open flag fields.
pub const OPEN_FLAGS_BYTES: usize = ${formatted_decimal(v1.open_flags_bytes)};
pub const OPEN_CREATE_IF_MISSING: u8 = ${formatted_byte(v1.open_create_if_missing_flag)};
pub const OPEN_RESERVED_MASK: u8 = ${formatted_byte(v1.open_reserved_mask)};

/// Namespace-delete flag fields.
pub const DELETE_FLAGS_BYTES: usize = ${formatted_decimal(v1.delete_flags_bytes)};
pub const DELETE_IF_EMPTY: u8 = ${formatted_byte(v1.delete_if_empty_bits)};
pub const DELETE_MODE_MASK: u8 = ${formatted_byte(v1.delete_mode_mask)};
pub const DELETE_RESERVED_MASK: u8 = ${formatted_byte(v1.delete_reserved_mask)};

/// Namespace-policy flag fields.
pub const POLICY_FLAGS_BYTES: usize = ${formatted_decimal(v1.policy_flags_bytes)};
pub const POLICY_DEFAULT_EXPIRATION_MASK: u8 = ${formatted_byte(v1.policy_default_expiration_mask)};
pub const POLICY_NO_EXPIRY: u8 = ${formatted_byte(v1.policy_no_expiry_bits)};
pub const POLICY_FIXED_TTL: u8 = ${formatted_byte(v1.policy_fixed_ttl_bits)};
pub const POLICY_DEFAULT_EXPIRATION_RESERVED_BITS: u8 = ${formatted_byte(v1.policy_default_expiration_reserved_bits)};
pub const POLICY_EXPIRATION_OVERRIDE: u8 = ${formatted_byte(v1.policy_expiration_override_flag)};
pub const POLICY_EVICTION_PROTECTED: u8 = ${formatted_byte(v1.policy_eviction_protected_flag)};
pub const POLICY_EVICTION_OVERRIDE: u8 = ${formatted_byte(v1.policy_eviction_override_flag)};
pub const POLICY_RESERVED_MASK: u8 = ${formatted_byte(v1.policy_reserved_mask)};
`
}

/** Renders protocol v1 identifiers and common wire primitives. */
export function render_rust_wire(contract: Wire_Contract): string {
  const v1 = contract.v1
  return `// Generated from the OpenKache Smithy wire contract. Do not edit.

/// QUIC application protocol identifier for wire protocol version 1.
pub const ALPN: &[u8] = ${rust_byte_string_literal(v1.alpn)};
/// Bytes occupied by the request opcode.
pub const OPCODE_BYTES: usize = ${formatted_decimal(v1.opcode_bytes)};
/// Bytes occupied by the response status.
pub const STATUS_BYTES: usize = ${formatted_decimal(v1.status_bytes)};
/// Bytes before the variable-length request lengths.
pub const REQUEST_FIXED_BYTES: usize = ${formatted_decimal(v1.request_fixed_bytes)};
/// Bytes before the variable-length response payload length.
pub const RESPONSE_FIXED_BYTES: usize = ${formatted_decimal(v1.response_fixed_bytes)};
/// Minimum bytes in one canonical unsigned \`vu128\`.
pub const MIN_VARUINT_BYTES: usize = ${formatted_decimal(v1.min_varuint_bytes)};
/// Maximum bytes in one unsigned \`vu128\` accepted by this protocol.
pub const MAX_VARUINT_BYTES: usize = ${formatted_decimal(v1.max_varuint_bytes)};
/// Bytes in every canonical item ID carried by the protocol.
pub const ITEM_ID_BYTES: usize = ${formatted_decimal(contract.item_id_bytes)};
/// Absolute value or response payload ceiling representable by protocol v1.
pub const MAX_VALUE_BYTES: usize = ${formatted_decimal(contract.max_value_bytes)};
/// Conservative maximum complete response frame size for protocol v1.
pub const MAX_RESPONSE_FRAME_BYTES: usize =
    ${formatted_decimal(max_response_frame_bytes_for_contract(contract))};
/// Bytes in every namespace ID and namespace revision.
pub const NAMESPACE_ID_BYTES: usize = ${formatted_decimal(v1.namespace_id_bytes)};
pub const NAMESPACE_REVISION_BYTES: usize = ${formatted_decimal(v1.namespace_revision_bytes)};
/// Bytes in the fixed namespace name length field.
pub const NAMESPACE_NAME_LENGTH_BYTES: usize = ${formatted_decimal(v1.namespace_name_length_bytes)};
/// Width and missing sentinel used by the generic optional-value codec.
pub const OPTIONAL_VALUE_LENGTH_BYTES: usize = ${formatted_decimal(v1.optional_value_length_bytes ?? 4)};
pub const OPTIONAL_VALUE_MISSING: u32 = ${formatted_decimal(v1.optional_value_missing ?? 0xffff_ffff)};

/// First assigned status value reserved for errors.
pub const ERROR_STATUS_MINIMUM: u8 = ${formatted_byte(v1.error_status_minimum)};

${rust_wire_enum("Opcode", "Operations supported by protocol v1.", contract.opcodes, "UnknownOpcode")}

${rust_wire_enum("Status", "Status returned in every protocol response.", contract.statuses, "UnknownStatus")}
`
}

/** Renders semantic operation metadata for the server-owned adapter. */
export function render_rust_server_contract(contract: Wire_Contract): string {
  const max_request_frame_bytes = max_request_frame_bytes_for_contract(contract)
  return `// Generated from the OpenKache Smithy operation contract. Do not edit.

use openkache_protocol::{
    ITEM_ID_BYTES, NAMESPACE_ID_BYTES, NAMESPACE_REVISION_BYTES, OPCODE_BYTES, Status, Opcode,
};

/// Conservative maximum complete request frame size for protocol v1.
pub const MAX_REQUEST_FRAME_BYTES: usize = ${formatted_decimal(max_request_frame_bytes)};

${render_rust_semantic_constants(contract)}

${rust_operation_contract(contract)}

${rust_request_layout(contract)}
`
}

/**
 * Renders the protocol operation table used by `SPEC.md`.
 *
 * Keeping this table generator-owned gives documentation a stale-checkable
 * representation of opcode assignments and the role-derived framing shape.
 */
export function render_protocol_spec_operation_table(contract: Wire_Contract): string {
  const operations = contract.operations
  if (operations === undefined) {
    throw new Error("protocol operation metadata is required for the specification table")
  }
  const field_count = (
    fields: readonly Wire_Operation_Field[],
    role: string,
  ): number => fields.find((field) => field.role === role)?.count ?? 0
  const request_layout = (operation: Wire_Operation): string => {
    const { request_fields, request_item_count } = operation.contract
    const item_count = field_count(request_fields, "item_id")
    switch (derive_wire_request_layout(operation.contract)) {
      case "empty":
        return "opcode only"
      case "application_value":
        return "opcode + value_len + value"
      case "field_sequence":
        return "opcode + field_sequence_len + ordered field sequence"
      case "item":
        return `opcode + namespace ID + ${request_item_count} item ID${request_item_count === 1 ? "" : "s"}`
      case "set":
        return `opcode + namespace ID + flags + ${item_count} item ID${item_count === 1 ? "" : "s"} + value`
      case "namespace":
        return "opcode + namespace ID"
      case "namespace_open":
        return "opcode + flags + name + optional policy"
      case "namespace_update_policy":
        return "opcode + namespace ID + revision + policy"
      case "namespace_delete":
        return "opcode + flags + namespace ID + revision"
    }
  }
  const response_payload = (operation: Wire_Operation): string => {
    const { response_fields, response_value_count } = operation.contract
    const value_count =
      response_value_count ?? field_count(response_fields, "value")
    switch (derive_wire_response_route(operation.contract)) {
      case "application_value":
        return "opaque payload"
      case "composite":
      case "field_sequence":
        return "ordered field sequence"
      case "value":
        return value_count === 1
          ? "optional value"
          : `${value_count} ordered optional values`
      case "set_outcome":
        return "set_outcome"
      case "delete_outcome":
        return "deleted"
      case "stats_json":
        return "JSON object"
      case "namespace_descriptor":
        return "namespace descriptor"
      case "pong":
        return "PONG"
      case "empty":
        return "empty"
    }
  }
  const field_codecs = (fields: readonly Wire_Operation_Field[]): string => {
    const codecs = fields.flatMap((field) => field.codecs ?? [])
    const unique = [...new Set(codecs)]
    return unique.length === 0 ? "—" : unique.map((codec) => `\`${codec}\``).join(", ")
  }
  const rows = operations
    .map((operation) => {
      const opcode = contract.opcodes.find((entry) => entry.name === operation.name)
      if (opcode === undefined) {
        throw new Error(`operation ${operation.name} has no opcode`)
      }
      return `| \`${opcode.value.toString(16).padStart(2, "0").toUpperCase()}\` | \`${wire_name(operation.name).toUpperCase()}\` | ${request_layout(operation)} | ${response_payload(operation)} | ${field_codecs(operation.contract.request_fields)} | ${field_codecs(operation.contract.response_fields)} | \`${operation.contract.effect}\` |`
    })
    .join("\n")
  return `| Opcode | Name | Request layout | Response payload | Request codecs | Response codecs | Effect |
|---|---|---|---|---|---|---|
${rows}`
}

export const PROTOCOL_SPEC_OPERATION_TABLE_START =
  "<!-- openkache:generated-protocol-operation-table:start -->"
export const PROTOCOL_SPEC_OPERATION_TABLE_END =
  "<!-- openkache:generated-protocol-operation-table:end -->"

/** Returns the stale generated operation-table paths in a protocol spec. */
export function protocol_spec_operation_table_issues(
  spec: string,
  contract: Wire_Contract,
): readonly string[] {
  const start = spec.indexOf(PROTOCOL_SPEC_OPERATION_TABLE_START)
  const end = spec.indexOf(PROTOCOL_SPEC_OPERATION_TABLE_END)
  if (start < 0 || end < 0 || end < start) {
    return ["protocol/SPEC.md (generated operation table markers missing)"]
  }
  const actual = spec
    .slice(start + PROTOCOL_SPEC_OPERATION_TABLE_START.length, end)
    .trim()
  const expected = render_protocol_spec_operation_table(contract).trim()
  return actual === expected ? [] : ["protocol/SPEC.md (generated operation table stale)"]
}
