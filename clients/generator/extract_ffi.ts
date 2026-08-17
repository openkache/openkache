//! Native ABI contract extraction.

import type { Wire_Entry } from "../../protocol/wire"
import { FFI_CONTRACT_TRAIT_ID, FFI_ENUMS } from "./config"
import {
  api_structure,
  integer_member,
  object_member,
  object_value,
  pascal_case,
  shape_type,
  string_member,
} from "./extract_ast"
import type {
  Api_Member,
  Ffi_Contract,
  Ffi_Entry,
  Json_Object,
  Namespace_Descriptor_Field,
  Namespace_Descriptor_Layout,
} from "./model"
import {
  go_exported_name,
  snake_case,
  swift_property_name,
} from "./utils"

export function unique_wire_values(entries: readonly Wire_Entry[], kind: string): void {
  const names = new Set<string>()
  const values = new Set<number>()
  for (const entry of entries) {
    if (names.has(entry.name)) throw new Error(`duplicate ${kind} name ${entry.name}`)
    if (values.has(entry.value)) {
      throw new Error(`duplicate ${kind} wire value ${entry.value}`)
    }
    names.add(entry.name)
    values.add(entry.value)
  }
}

export function ffi_enum_entries(
  shapes: Json_Object,
  namespace: string,
  enum_name: string,
  kind: string,
): readonly Ffi_Entry[] {
  const shape_id = `${namespace}#${enum_name}`
  const shape = object_member(shapes, shape_id, "Smithy AST.shapes")
  if (shape_type(shape, `Smithy AST.shapes.${shape_id}`) !== "enum") {
    throw new Error(`${shape_id} must be an enum`)
  }
  const members = object_member(shape, "members", shape_id)
  const entries = Object.entries(members).map(([member_name, value]): Ffi_Entry => {
    const member = object_value(value, `${shape_id}.${member_name}`)
    const traits = object_member(member, "traits", `${shape_id}.${member_name}`)
    const assignment = object_member(
      traits,
      "openkache.client#ffiValue",
      `${shape_id}.${member_name}.traits`,
    )
    return {
      name: pascal_case(snake_case(member_name)),
      text: string_member(
        traits,
        "smithy.api#enumValue",
        `${shape_id}.${member_name}.traits`,
      ),
      value: integer_member(
        assignment,
        "value",
        `${shape_id}.${member_name}.traits.${"openkache.client#ffiValue"}`,
        0,
        0xffff_ffff,
      ),
    }
  })
    .sort((left, right) => left.value - right.value)
  if (entries.length === 0) {
    throw new Error(`${kind} contract must define at least one entry`)
  }
  unique_wire_values(entries, kind)
  const text_values = new Set<string>()
  for (const entry of entries) {
    if (text_values.has(entry.text)) {
      throw new Error(`duplicate ${kind} enum value ${entry.text}`)
    }
    text_values.add(entry.text)
  }
  return entries
}

export function descriptor_field_metadata(
  member: Api_Member,
): Omit<Namespace_Descriptor_Field, "offset"> {
  const name = snake_case(member.name)
  const common = {
    name,
    csharp_name: pascal_case(name),
    go_name: go_exported_name(member.name),
    swift_name: swift_property_name(member.name),
  }
  switch (member.type.kind) {
    case "unsigned_long":
      return {
        ...common,
        rust_type: "u64",
        c_type: "uint64_t",
        csharp_type: "ulong",
        go_type: "uint64",
        python_type: "_ctypes.c_uint64",
        swift_type: "UInt64",
        size: 8,
        alignment: 8,
      }
    case "integer":
      return {
        ...common,
        rust_type: "u32",
        c_type: "uint32_t",
        csharp_type: "uint",
        go_type: "uint32",
        python_type: "_ctypes.c_uint32",
        swift_type: "UInt32",
        size: 4,
        alignment: 4,
      }
    default:
      throw new Error(
        `Smithy FfiNamespaceDescriptor member ${member.name} must be an unsigned Long or Integer`,
      )
  }
}

interface Namespace_Descriptor_Contract {
  readonly fields: readonly Namespace_Descriptor_Field[]
  readonly layout: Namespace_Descriptor_Layout
}

export function namespace_descriptor_contract(
  shapes: Json_Object,
  namespace: string,
): Namespace_Descriptor_Contract {
  const descriptor = api_structure(shapes, `${namespace}#FfiNamespaceDescriptor`)
  if (descriptor.members.length === 0) {
    throw new Error("Smithy FfiNamespaceDescriptor must define at least one member")
  }
  let offset = 0
  let maximum_alignment = 1
  const fields = descriptor.members.map((member): Namespace_Descriptor_Field => {
    if (!member.required) {
      throw new Error(
        `Smithy FfiNamespaceDescriptor member ${member.name} must be required`,
      )
    }
    const metadata = descriptor_field_metadata(member)
    maximum_alignment = Math.max(maximum_alignment, metadata.alignment)
    const remainder = offset % metadata.alignment
    if (remainder !== 0) offset += metadata.alignment - remainder
    const field_offset = offset
    offset += metadata.size
    return {
      ...metadata,
      offset: field_offset,
    }
  })
  const trailing_remainder = offset % maximum_alignment
  if (trailing_remainder !== 0) offset += maximum_alignment - trailing_remainder
  const descriptor_size = offset
  const offsets = Object.fromEntries(fields.map((field) => [field.name, field.offset]))
  for (let index = 0; index < fields.length; index += 1) {
    const left = fields[index]!
    const left_end = left.offset + left.size
    for (const right of fields.slice(index + 1)) {
      const right_end = right.offset + right.size
      if (left.offset < right_end && right.offset < left_end) {
        throw new Error(
          `namespace descriptor ABI fields ${left.name} and ${right.name} overlap`,
        )
      }
    }
  }
  return {
    fields,
    layout: { size_bytes: descriptor_size, offsets },
  }
}

export function ffi_contract(
  value: unknown,
  shapes: Json_Object,
  namespace: string,
): Ffi_Contract {
  const contract = object_value(value, FFI_CONTRACT_TRAIT_ID)
  const descriptor = namespace_descriptor_contract(shapes, namespace)
  return {
    abi_version: integer_member(
      contract,
      "abiVersion",
      `${FFI_CONTRACT_TRAIT_ID}.abiVersion`,
      1,
      0xffff_ffff,
    ),
    connection_states: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.connection_states.name,
      FFI_ENUMS.connection_states.kind,
    ),
    namespace_default_evictions: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.namespace_default_evictions.name,
      FFI_ENUMS.namespace_default_evictions.kind,
    ),
    namespace_default_expirations: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.namespace_default_expirations.name,
      FFI_ENUMS.namespace_default_expirations.kind,
    ),
    namespace_descriptor_decode_statuses: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.namespace_descriptor_decode_statuses.name,
      FFI_ENUMS.namespace_descriptor_decode_statuses.kind,
    ),
    namespace_descriptor_fields: descriptor.fields,
    namespace_descriptor_layout: descriptor.layout,
    namespace_override_policies: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.namespace_override_policies.name,
      FFI_ENUMS.namespace_override_policies.kind,
    ),
    operations: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.operations.name,
      FFI_ENUMS.operations.kind,
    ),
    result_kinds: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.result_kinds.name,
      FFI_ENUMS.result_kinds.kind,
    ),
    set_conditions: ffi_enum_entries(
      shapes,
      namespace,
      FFI_ENUMS.set_conditions.name,
      FFI_ENUMS.set_conditions.kind,
    ),
  }
}
