//! Smithy AST primitives and API model extraction.

import { UNSIGNED_LONG_TRAIT_ID } from "./config"
import type {
  Api_Contract,
  Api_Enum,
  Api_Enum_Member,
  Api_Member,
  Api_Operation,
  Api_Structure,
  Api_Type,
  Api_Type_Kind,
  Json_Object,
} from "./model"

export function object_value(value: unknown, location: string): Json_Object {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${location} must be an object`)
  }
  return value as Json_Object
}

export function object_member(object: Json_Object, member: string, location: string): Json_Object {
  return object_value(object[member], `${location}.${member}`)
}

export function array_member(
  object: Json_Object,
  member: string,
  location: string,
): readonly unknown[] {
  const value = object[member]
  if (!Array.isArray(value)) throw new Error(`${location}.${member} must be an array`)
  return value
}

export function string_member(object: Json_Object, member: string, location: string): string {
  const value = object[member]
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${location}.${member} must be a non-empty string`)
  }
  return value
}

export function integer_member(
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

export function shape_name(shape_id: string): string {
  const separator = shape_id.lastIndexOf("#")
  if (separator < 0 || separator === shape_id.length - 1) {
    throw new Error(`shape ID ${JSON.stringify(shape_id)} has no shape name`)
  }
  return shape_id.slice(separator + 1)
}

export function pascal_case(identifier: string): string {
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

export function trait_value_any(
  shape: Json_Object,
  trait_ids: readonly string[],
  location: string,
): Json_Object {
  const traits = object_member(shape, "traits", location)
  for (const trait_id of trait_ids) {
    const value = traits[trait_id]
    if (value !== undefined) return object_value(value, `${location}.traits.${trait_id}`)
  }
  throw new Error(
    `${location}.traits is missing one of ${trait_ids.map((trait_id) => JSON.stringify(trait_id)).join(", ")}`,
  )
}

export function optional_object_member(
  object: Json_Object,
  member: string,
  location: string,
): Json_Object | undefined {
  const value = object[member]
  return value === undefined ? undefined : object_value(value, `${location}.${member}`)
}

export function shape_type(shape: Json_Object, location: string): string {
  return string_member(shape, "type", location)
}

export function api_type(
  shapes: Json_Object,
  target: string,
  member_traits?: Json_Object,
): Api_Type {
  const prelude_types: Readonly<Record<string, Api_Type_Kind>> = {
    "smithy.api#Boolean": "boolean",
    "smithy.api#Integer": "integer",
    "smithy.api#Long": "long",
    "smithy.api#String": "string",
  }
  const prelude =
    member_traits?.[UNSIGNED_LONG_TRAIT_ID] !== undefined &&
    target === "smithy.api#Long"
      ? "unsigned_long"
      : prelude_types[target]
  if (prelude !== undefined) return { kind: prelude }

  const shape = object_member(shapes, target, "Smithy AST.shapes")
  const kind = shape_type(shape, `Smithy AST.shapes.${target}`)
  switch (kind) {
    case "blob":
      return { kind: "blob" }
    case "enum":
      return { kind: "enum", name: shape_name(target) }
    case "structure":
      return { kind: "structure", name: shape_name(target) }
    default:
      throw new Error(`unsupported API member target ${target} with shape type ${kind}`)
  }
}

export function api_structure(shapes: Json_Object, target: string): Api_Structure {
  const shape = object_member(shapes, target, "Smithy AST.shapes")
  if (shape_type(shape, `Smithy AST.shapes.${target}`) !== "structure") {
    throw new Error(`${target} must be a structure`)
  }
  const members = object_member(shape, "members", target)
  return {
    name: shape_name(target),
    members: Object.entries(members).map(([name, value]): Api_Member => {
      const member = object_value(value, `${target}.${name}`)
      const traits = optional_object_member(member, "traits", `${target}.${name}`)
      return {
        name,
        required: traits?.["smithy.api#required"] !== undefined,
        type: api_type(
          shapes,
          string_member(member, "target", `${target}.${name}`),
          traits,
        ),
      }
    }),
  }
}

export function api_enum(shapes: Json_Object, namespace: string, name: string): Api_Enum {
  const shape_id = `${namespace}#${name}`
  const shape = object_member(shapes, shape_id, "Smithy AST.shapes")
  if (shape_type(shape, `Smithy AST.shapes.${shape_id}`) !== "enum") {
    throw new Error(`${shape_id} must be an enum`)
  }
  const members = object_member(shape, "members", shape_id)
  const enum_members = Object.entries(members).map(
    ([member_name, value]): Api_Enum_Member => {
      const member = object_value(value, `${shape_id}.${member_name}`)
      const traits = object_member(member, "traits", `${shape_id}.${member_name}`)
      return {
        name: pascal_case(member_name),
        value: string_member(
          traits,
          "smithy.api#enumValue",
          `${shape_id}.${member_name}.traits`,
        ),
      }
    },
  )
  const member_names = new Set<string>()
  const member_values = new Set<string>()
  for (const member of enum_members) {
    if (member_names.has(member.name)) {
      throw new Error(`duplicate ${name} enum member name ${member.name}`)
    }
    if (member_values.has(member.value)) {
      throw new Error(`duplicate ${name} enum value ${member.value}`)
    }
    member_names.add(member.name)
    member_values.add(member.value)
  }
  return {
    name,
    members: enum_members,
  }
}

export function api_contract(
  shapes: Json_Object,
  service_shape_id: string,
  namespace: string,
): Api_Contract {
  const service = object_member(shapes, service_shape_id, "Smithy AST.shapes")
  const operation_shapes = array_member(service, "operations", service_shape_id)
    .map((operation, index): Api_Operation => {
      const reference = object_value(operation, `${service_shape_id}.operations[${index}]`)
      const target = string_member(
        reference,
        "target",
        `${service_shape_id}.operations[${index}]`,
      )
      const shape = object_member(shapes, target, "Smithy AST.shapes")
      const input = string_member(
        object_member(shape, "input", target),
        "target",
        `${target}.input`,
      )
      const output = string_member(
        object_member(shape, "output", target),
        "target",
        `${target}.output`,
      )
      return {
        input: shape_name(input),
        name: shape_name(target),
        output: shape_name(output),
      }
    })

  const structure_names = new Set<string>()
  for (const operation of operation_shapes) {
    structure_names.add(operation.input)
    structure_names.add(operation.output)
  }
  const enum_names = new Set<string>()
  const structures_by_name = new Map<string, Api_Structure>()
  const pending_structure_names = [...structure_names]
  while (pending_structure_names.length > 0) {
    const name = pending_structure_names.pop()
    if (name === undefined || structures_by_name.has(name)) continue
    const structure = api_structure(shapes, `${namespace}#${name}`)
    structures_by_name.set(name, structure)
    for (const member of structure.members) {
      if (member.type.name === undefined) continue
      if (member.type.kind === "enum") {
        enum_names.add(member.type.name)
      } else if (member.type.kind === "structure") {
        pending_structure_names.push(member.type.name)
      }
    }
  }
  const structures = [...structures_by_name.values()].sort((left, right) =>
    left.name.localeCompare(right.name),
  )

  return {
    enums: [...enum_names]
      .map((name) => api_enum(shapes, namespace, name))
      .sort((left, right) => left.name.localeCompare(right.name)),
    operations: operation_shapes,
    structures,
  }
}
