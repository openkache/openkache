//! Shared contract types for client generation.

import type { Wire_Contract, Wire_Entry } from "../../protocol/wire"

export type Json_Object = Readonly<Record<string, unknown>>

/** Cross-language value-format wire layout, identifiers, and cryptographic metadata. */
export interface Value_Format_Contract {
  readonly aad_domain: string
  readonly compact_encryption_context: string
  readonly compact_mac_context: string
  readonly compact_synthetic_iv_bytes: number
  readonly compression_none: number
  readonly compression_zstandard: number
  readonly data_protection_key_bytes: number
  readonly encryption_compact: number
  readonly encryption_none: number
  readonly encryption_robust: number
  readonly format_byte_bytes: number
  readonly format_compression_mask: number
  readonly format_encryption_shift: number
  readonly item_id_root_context: string
  readonly robust_context: string
  readonly robust_nonce_bytes: number
  readonly robust_tag_bytes: number
  readonly serialization_json: number
  readonly serialization_raw: number
  /** Target selector for StructuredValue-CBOR-v1 (JSON has no selector). */
  readonly serialization_structured: number
  readonly value_root_context: string
  readonly max_vu128_bytes: number
  readonly version: number
}

/** Legacy metadata envelope retained for the TypeScript adapter migration. */
export interface Value_Envelope_Contract {
  readonly json_encoding: string
  readonly magic_and_version_hex: string
  readonly max_encoding_bytes: number
  readonly max_type_name_bytes: number
}

/** Defaults shared by the Rust client core and its native language adapters. */
export interface Client_Defaults_Contract {
  readonly connect_timeout_milliseconds: number
  readonly gate0_alpn_version: number
  readonly gate0_compression: number
  readonly gate0_encryption: number
  readonly gate0_item_id_root_key_hex: string
  readonly gate0_namespace_id: number
  readonly gate0_value_selector: number
  readonly max_in_flight: number
  readonly request_timeout_milliseconds: number
  readonly retry_max_attempts: number
  readonly zstandard_level: number
  readonly zstandard_minimum_input_bytes: number
  readonly zstandard_minimum_savings_bytes: number
  readonly zstandard_level_min: number
  readonly zstandard_level_max: number
  readonly server_name: string
  readonly certificate_pem_type: string
  readonly minimum_positive_value: number
}

export type Api_Type_Kind =
  | "blob"
  | "boolean"
  | "enum"
  | "integer"
  | "long"
  | "string"
  | "structure"
  | "unsigned_long"

/** One resolved Smithy API field type. */
export interface Api_Type {
  readonly kind: Api_Type_Kind
  readonly name?: string
}

/** One field in a Smithy operation input or output structure. */
export interface Api_Member {
  readonly name: string
  readonly required: boolean
  readonly type: Api_Type
}

/** One Smithy operation input or output structure. */
export interface Api_Structure {
  readonly members: readonly Api_Member[]
  readonly name: string
}

/** One string-valued Smithy enum member. */
export interface Api_Enum_Member {
  readonly name: string
  readonly value: string
}

/** One string-valued Smithy API enum. */
export interface Api_Enum {
  readonly members: readonly Api_Enum_Member[]
  readonly name: string
}

/** One operation exposed by the Smithy service. */
export interface Api_Operation {
  readonly input: string
  readonly name: string
  readonly output: string
}

/** Language-neutral service API extracted from the Smithy model. */
export interface Api_Contract {
  readonly enums: readonly Api_Enum[]
  readonly operations: readonly Api_Operation[]
  readonly structures: readonly Api_Structure[]
}

/** Native binding ABI identifiers shared by language-neutral adapters. */
export interface Ffi_Entry extends Wire_Entry {
  /** Stable Smithy enum value exposed by language adapters. */
  readonly text: string
}

export interface Ffi_Contract {
  readonly abi_version: number
  readonly connection_states: readonly Ffi_Entry[]
  readonly transports: readonly Ffi_Entry[]
  readonly namespace_default_evictions: readonly Ffi_Entry[]
  readonly namespace_default_expirations: readonly Ffi_Entry[]
  readonly namespace_descriptor_decode_statuses: readonly Ffi_Entry[]
  readonly namespace_descriptor_fields: readonly Namespace_Descriptor_Field[]
  readonly namespace_descriptor_layout: Namespace_Descriptor_Layout
  readonly namespace_override_policies: readonly Ffi_Entry[]
  readonly operations: readonly Ffi_Entry[]
  readonly result_kinds: readonly Ffi_Entry[]
  readonly set_conditions: readonly Ffi_Entry[]
}

/** C-compatible layout of the namespace descriptor returned by the native ABI. */
export interface Namespace_Descriptor_Layout {
  readonly size_bytes: number
  readonly offsets: Readonly<Record<string, number>>
}

/** One field in the Smithy-defined native namespace descriptor projection. */
export interface Namespace_Descriptor_Field {
  readonly name: string
  readonly csharp_name: string
  readonly go_name: string
  readonly swift_name: string
  readonly rust_type: string
  readonly c_type: string
  readonly csharp_type: string
  readonly go_type: string
  readonly python_type: string
  readonly swift_type: string
  readonly size: number
  readonly alignment: number
  readonly offset: number
}

/** Wire contract combined with the client-owned Smithy model. */
export interface Client_Contract extends Wire_Contract {
  readonly api: Api_Contract
  readonly client_defaults: Client_Defaults_Contract
  readonly ffi: Ffi_Contract
  readonly value_envelope: Value_Envelope_Contract
  readonly value_format: Value_Format_Contract
}
