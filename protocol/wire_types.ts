/** Shared type and codec metadata for the Smithy wire contract. */

/** One numeric protocol member assigned by the wire contract. */
export interface Wire_Entry {
  readonly name: string
  /** Optional Smithy enum value used for generated labels. */
  readonly text?: string
  readonly value: number
}

/** Canonical request framing values selected by the shared wire layout. */
export const WIRE_REQUEST_FRAMINGS = [
  "empty",
  "opaque",
  "ordered_fields",
] as const

export type Wire_Model_Request_Framing = (typeof WIRE_REQUEST_FRAMINGS)[number]

/**
 * Canonical request framing consumed by generic protocol infrastructure.
 *
 * Historical protocol-v1 route bytes are selected only by a compatibility
 * adapter. They do not form a fourth generic framing family.
 */
export type Wire_Request_Framing = Wire_Model_Request_Framing

/** One canonical value encoded inside a packed request byte. */
export interface Wire_Request_Packed_Value {
  readonly value: string
  readonly bits: number
}

/** One modeled field encoded inside a packed request byte. */
export interface Wire_Request_Packed_Field {
  readonly field: number
  readonly mask: number
  readonly values: readonly Wire_Request_Packed_Value[]
}

/** Operation-neutral compact request byte primitives. */
export type Wire_Request_Step =
  | { readonly kind: "fixed_field"; readonly field: number; readonly bytes: number }
  | {
      readonly kind: "packed"
      readonly fields: readonly Wire_Request_Packed_Field[]
      readonly reserved_mask: number
      readonly constant_bits: number
    }
  | { readonly kind: "byte_length_field"; readonly field: number }
  | { readonly kind: "byte_length_prefix_field"; readonly field: number }
  | { readonly kind: "byte_field"; readonly field: number }
  | { readonly kind: "varuint_field"; readonly field: number }
  | {
      readonly kind: "value_length_field"
      readonly field: number
      readonly length: "varuint"
    }
  | {
      readonly kind: "conditional"
      readonly field: number
      readonly equals: string
      readonly steps: readonly Wire_Request_Step[]
    }
  | { readonly kind: "constant"; readonly bytes: readonly number[] }
  | {
      readonly kind: "trailing_field"
      readonly field: number
      readonly length: "varuint"
    }

/** Generic response payload framing selected by the modeled operation. */
export const WIRE_RESPONSE_FRAMINGS = [
  "empty",
  "opaque",
  "optional_values",
  "field_sequence",
] as const

export type Wire_Response_Framing = (typeof WIRE_RESPONSE_FRAMINGS)[number]

/** Generic payload layout selected from generated field shape metadata. */
export type Wire_Operation_Field_Layout =
  | "empty"
  | "opaque"
  | "sequence"
  | "dense"
  | "optional_values"

/** Generic request/response frame policy selected by the shape plan. */
export type Wire_Operation_Frame_Policy = "length_delimited" | "fixed_body"

/**
 * Upper bound for generated offset/field storage.
 *
 * Runtime views keep ordinary plans inline and spill larger valid plans. A
 * shape beyond this bound needs a streaming representation rather than
 * silently accepting unbounded metadata.
 */
export const MAX_GENERATED_OPERATION_FIELDS = 256

/**
 * Upper bound for one recursive codec descriptor path.
 *
 * This is a resource-safety bound for generated metadata, not a domain limit.
 * It prevents an accidentally recursive/deeply nested model from creating
 * unbounded generator output or runtime validation depth.
 */
export const MAX_GENERATED_NESTED_CODEC_DEPTH = 64

/** Maximum flattened codec descriptors retained for one modeled field. */
export const MAX_GENERATED_NESTED_CODEC_ENTRIES = 256

/**
 * Codec identifiers understood by the protocol adapters.
 *
 * This list is intentionally transport-oriented. Language generators may
 * attach a renderer for a subset of these codecs, while server generation
 * validates the same identifiers from the canonical operation descriptor.
 */
export const WIRE_CODEC_NAMES = [
  "bool_u8",
  "enum",
  "f64_be",
  "i32_be",
  "list",
  "map",
  "packed_f64_be",
  "raw_bytes",
  "u64_be",
  "union",
  "utf8",
] as const

export type Wire_Codec_Name = (typeof WIRE_CODEC_NAMES)[number]

/**
 * Shape-independent codec properties used by the layout planner.
 *
 * A codec owns its encoded-width contract. The planner may still infer the
 * default codec from a Smithy primitive shape for older models, but width
 * selection never switches on a domain shape or operation name.
 */
export type Wire_Codec_Width = "fixed" | "variable"
export type Wire_Codec_Cardinality =
  | "scalar"
  | "repeated"
  | "associative"
  | "tagged"
export type Wire_Codec_Length_Encoding = "none" | "byte" | "varuint"

export interface Wire_Codec_Descriptor {
  readonly name: Wire_Codec_Name
  readonly width: Wire_Codec_Width
  readonly min_width: number
  readonly max_width?: number
  readonly borrowable: boolean
  readonly cardinality: Wire_Codec_Cardinality
  readonly length_encoding: Wire_Codec_Length_Encoding
  readonly container: boolean
  readonly recursive: boolean
}

/**
 * The planner's codec proof vocabulary.
 *
 * Containers deliberately remain variable until a future shape descriptor
 * supplies fixed cardinality and child-width metadata. A declared member
 * width can refine a scalar/opaque codec, but it cannot turn a list, map, or
 * union into a dense field by itself.
 */
export const WIRE_CODEC_DESCRIPTORS: Readonly<
  Record<Wire_Codec_Name, Wire_Codec_Descriptor>
> = {
  bool_u8: {
    name: "bool_u8",
    width: "fixed",
    min_width: 1,
    max_width: 1,
    borrowable: true,
    cardinality: "scalar",
    length_encoding: "none",
    container: false,
    recursive: false,
  },
  enum: {
    name: "enum",
    width: "variable",
    min_width: 0,
    borrowable: true,
    cardinality: "scalar",
    length_encoding: "none",
    container: false,
    recursive: false,
  },
  f64_be: {
    name: "f64_be",
    width: "fixed",
    min_width: 8,
    max_width: 8,
    borrowable: true,
    cardinality: "scalar",
    length_encoding: "none",
    container: false,
    recursive: false,
  },
  i32_be: {
    name: "i32_be",
    width: "fixed",
    min_width: 4,
    max_width: 4,
    borrowable: true,
    cardinality: "scalar",
    length_encoding: "none",
    container: false,
    recursive: false,
  },
  list: {
    name: "list",
    width: "variable",
    min_width: 1,
    borrowable: true,
    cardinality: "repeated",
    length_encoding: "varuint",
    container: true,
    recursive: true,
  },
  map: {
    name: "map",
    width: "variable",
    min_width: 1,
    borrowable: true,
    cardinality: "associative",
    length_encoding: "varuint",
    container: true,
    recursive: true,
  },
  packed_f64_be: {
    name: "packed_f64_be",
    width: "variable",
    min_width: 0,
    borrowable: true,
    cardinality: "repeated",
    length_encoding: "none",
    container: false,
    recursive: false,
  },
  raw_bytes: {
    name: "raw_bytes",
    width: "variable",
    min_width: 0,
    borrowable: true,
    cardinality: "scalar",
    length_encoding: "none",
    container: false,
    recursive: false,
  },
  u64_be: {
    name: "u64_be",
    width: "fixed",
    min_width: 8,
    max_width: 8,
    borrowable: true,
    cardinality: "scalar",
    length_encoding: "none",
    container: false,
    recursive: false,
  },
  union: {
    name: "union",
    width: "variable",
    min_width: 2,
    borrowable: true,
    cardinality: "tagged",
    length_encoding: "byte",
    container: true,
    recursive: true,
  },
  utf8: {
    name: "utf8",
    width: "variable",
    min_width: 0,
    borrowable: true,
    cardinality: "scalar",
    length_encoding: "none",
    container: false,
    recursive: false,
  },
}

/**
 * Default codecs inferred from primitive Smithy shapes when a model does not
 * declare `@wireCodec`. Keeping this map in the shared contract module gives
 * the layout planner and generated metadata one source of truth.
 */
export const DEFAULT_SHAPE_CODECS: Readonly<Record<string, Wire_Codec_Name>> = {
  Boolean: "bool_u8",
  Double: "f64_be",
  Integer: "i32_be",
  Long: "u64_be",
}

export interface Wire_Operation_Contract {
  readonly error_statuses: readonly string[]
  /**
   * Ordered request field plan. This preserves requiredness, shape, and
   * member order for server-owned extensions; permissive fixtures may omit it.
   */
  readonly request_plan?: readonly Wire_Operation_Field_Plan[]
  /** Exact request bytes expressed as operation-neutral field primitives. */
  readonly request_wire?: readonly Wire_Request_Step[]
  /**
   * Generic request framing from the modeled contract. Permissive fixtures may
   * omit this only when a caller is constructing a partial descriptor for
   * inspection; operation extraction itself requires an explicit framing.
   *
   * A future API that uses an existing generic primitive can declare only this
   * member; it does not need to select a namespace/item/SET route.
   */
  readonly request_framing?: Wire_Model_Request_Framing
  /**
   * Opaque operation-contract extensions preserved for an adapter.
   *
   * Keys are namespaced by the extractor with the Smithy trait ID and member
   * name. Generic infrastructure never interprets or enumerates those keys;
   * an adapter may narrow its own values after extraction.
   */
  readonly extensions?: Readonly<Record<string, unknown>>
  /**
   * Ordered response field plan. This is the generic field-sequence source of
   * truth; optional-value framing is only one encoding used by that plan.
   */
  readonly response_plan?: readonly Wire_Operation_Field_Plan[]
  /**
   * Explicit generic response framing. This is independent from the semantic
   * result represented by the response. Production contracts always provide
   * this member; the optional type keeps non-strict AST fixtures readable.
   */
  readonly response_framing?: Wire_Response_Framing
  /** Explicit adapter-owned aggregate opaque payload marker. */
  readonly opaque_aggregate?: boolean
  readonly success_statuses: readonly string[]
}

/**
 * Optional validation owned by a wire compatibility adapter.
 *
 * The generic Smithy extractor preserves adapter-selected values in the
 * operation's namespaced `extensions` map. A protocol adapter may validate and
 * narrow that metadata for its own renderer, but the shared extractor must not
 * import or enumerate any adapter's route vocabulary.
 */
export interface Wire_Contract_Adapter {
  /**
   * Adds or validates adapter-owned operation-contract extensions.
   *
   * Generic extraction already preserves every non-wire member under a
   * namespaced key. The callback receives the raw Smithy trait object so an
   * adapter can validate or normalize only the values it owns.
   */
  readonly extract_extensions?: (
    contract: Readonly<Record<string, unknown>>,
    operation_location: string,
  ) => Readonly<Record<string, unknown>> | undefined
  readonly validate_operation?: (
    contract: Wire_Operation_Contract,
    operation_location: string,
  ) => void
}

/** One ordered Smithy field projected into the server operation plan. */
export interface Wire_Operation_Field_Plan {
  /** Stable zero-based position in the flattened Smithy field plan. */
  readonly index: number
  readonly codecs?: readonly string[]
  /** Optional exact widths supplied by codec declarations, aligned to codecs. */
  readonly codec_widths?: readonly (number | undefined)[]
  /**
   * Codec names used by members nested inside a container/union shape.
   * Keeping this descriptor alongside the top-level field lets server and
   * client validators verify the same recursive support matrix without
   * teaching either transport adapter about a particular API.
   */
  readonly nested_codecs?: readonly string[]
  /** Exact widths known for each nested codec, aligned to nested_codecs. */
  readonly nested_widths?: readonly (number | undefined)[]
  /** Allowed values for nested enum codecs, aligned with nested_codecs. */
  readonly nested_enum_values?: readonly (readonly string[])[]
  /** Allowed numeric tags for a top-level union codec. */
  readonly union_tags?: readonly number[]
  /** Allowed numeric tags for nested union codecs, aligned with nested_codecs. */
  readonly nested_union_tags?: readonly (readonly number[])[]
  readonly path: readonly string[]
  readonly required: boolean
  /** Exact width for required fixed-width leaves, when known. */
  readonly encoded_width?: number
  readonly role: string
  /** Smithy shape name used by codec adapters and diagnostics. */
  readonly shape: string
  /** Generated string members for an enum codec, when the shape is an enum. */
  readonly enum_values?: readonly string[]
}

/** One protocol opcode and its Smithy semantic operation contract. */
export interface Wire_Operation {
  readonly contract: Wire_Operation_Contract
  readonly name: string
}

/** Protocol v1 constants consumed by generated protocol adapters. */
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
  /** Optional-value response framing; defaults preserve permissive AST fixtures. */
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
  /** Maximum number of octets in one length-delimited opaque Item ID. */
  readonly max_item_id_bytes: number
  readonly max_value_bytes: number
  /**
   * Operation metadata is optional for permissive AST fixtures. Production
   * protocol generation runs in strict mode and always emits it.
   */
  readonly operations?: readonly Wire_Operation[]
  readonly opcodes: readonly Wire_Entry[]
  readonly statuses: readonly Wire_Entry[]
  readonly v1: Wire_V1_Contract
}

/**
 * The canonical transport projection for one modeled operation.
 *
 * This is deliberately a descriptor rather than a closed operation-family
 * enum. Historical compact routes and named response routes remain
 * compatibility projections, while generic dispatchers consume only their
 * canonical framing and field plans.
 */
export interface Wire_Operation_Descriptor {
  readonly request_framing: Wire_Request_Framing
  readonly request_frame: Wire_Operation_Frame_Policy
  readonly request_layout: Wire_Operation_Field_Layout
  readonly response_framing: Wire_Response_Framing
  readonly response_frame: Wire_Operation_Frame_Policy
  readonly response_layout: Wire_Operation_Field_Layout
}
