/** Shape and nested-codec planning for generated operation descriptors. */

import {
  MAX_GENERATED_NESTED_CODEC_DEPTH,
  MAX_GENERATED_NESTED_CODEC_ENTRIES,
  MAX_GENERATED_OPERATION_FIELDS,
  WIRE_CODEC_DESCRIPTORS,
  type Wire_Codec_Name,
  type Wire_Operation_Field_Plan,
} from "../wire_types"
import { fixed_field_width } from "../wire_layout"
import {
  ensure_wire_codec_name,
  object_member,
  object_value,
  optional_integer_member,
  optional_object_member,
  shape_type,
  string_member,
  type Json_Object,
} from "./validate_contract"

const OPERATION_FIELD_TRAIT_ID = "openkache.protocol#operationField"
const WIRE_CODEC_TRAIT_ID = "openkache.protocol#wireCodec"

function shape_name(shape_id: string): string {
  const separator = shape_id.lastIndexOf("#")
  if (separator < 0 || separator === shape_id.length - 1) {
    throw new Error(`shape ID ${JSON.stringify(shape_id)} has no shape name`)
  }
  return shape_id.slice(separator + 1)
}

export function operation_shape_field_plan(
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
  type Nested_Codec = {
    readonly name: string
    readonly width?: number
    readonly enum_values?: readonly string[]
    readonly union_tags?: readonly number[]
  }
  const enum_values_for_shape = (target: string): readonly string[] | undefined => {
    const shape = shapes[target]
    if (shape === undefined) return undefined
    if (
      shape_type(
        object_value(shape, `Smithy AST.shapes.${target}`),
        `Smithy AST.shapes.${target}`,
      ) !== "enum"
    ) return undefined
    const members = optional_object_member(
      object_value(shape, `Smithy AST.shapes.${target}`),
      "members",
      `Smithy AST.shapes.${target}`,
    )
    return Object.entries(members ?? {}).map(([member_name, member_value]) => {
      const traits = optional_object_member(
        object_value(member_value, `${target}.${member_name}`),
        "traits",
        `${target}.${member_name}`,
      )
      return traits?.["smithy.api#enumValue"] === undefined
        ? member_name
        : string_member(
          traits,
          "smithy.api#enumValue",
          `${target}.${member_name}.traits`,
        )
    })
  }
  const union_tags_for_shape = (
    target: string,
    members: Json_Object | undefined,
  ): readonly number[] => {
    const count = Object.keys(members ?? {}).length
    if (count > 0x100) {
      throw new Error(
        `${target} union has ${count} members; protocol union tags support at most 256`,
      )
    }
    return Object.keys(members ?? {}).map((_, index) => index)
  }
  const nested_codec_descriptors = (
    target: string,
    ancestors: ReadonlySet<string> = new Set(),
    infer_child_codecs = false,
    depth = 0,
  ): readonly Nested_Codec[] => {
    if (depth > MAX_GENERATED_NESTED_CODEC_DEPTH) {
      throw new Error(
        `${operation_target}.${direction} nested codec depth exceeds ` +
          `${MAX_GENERATED_NESTED_CODEC_DEPTH}`,
      )
    }
    if (ancestors.has(target)) {
      throw new Error(`${operation_target}.${direction} shape cycle through ${target}`)
    }
    const shape = shapes[target]
    if (shape === undefined) return []
    const next_ancestors = new Set(ancestors).add(target)
    const traits = optional_object_member(
      object_value(shape, `Smithy AST.shapes.${target}`),
      "traits",
      `Smithy AST.shapes.${target}`,
    )
    const codec = traits?.[WIRE_CODEC_TRAIT_ID]
    const type = shape_type(
      object_value(shape, `Smithy AST.shapes.${target}`),
      `Smithy AST.shapes.${target}`,
    )
    const explicit_codec = codec === undefined
      ? undefined
      : string_member(
          object_value(
            codec,
            `Smithy AST.shapes.${target}.${WIRE_CODEC_TRAIT_ID}`,
          ),
          "name",
          `Smithy AST.shapes.${target}.${WIRE_CODEC_TRAIT_ID}`,
        )
    const inferred_codec = infer_child_codecs
      ? ({
          blob: "raw_bytes",
          boolean: "bool_u8",
          double: "f64_be",
          enum: "enum",
          integer: "i32_be",
          list: "list",
          long: "u64_be",
          map: "map",
          string: "utf8",
          union: "union",
        } as const)[type as
          | "blob"
          | "boolean"
          | "double"
          | "enum"
          | "integer"
          | "list"
          | "long"
          | "map"
          | "string"
          | "union"]
      : undefined
    const codec_name = explicit_codec ?? inferred_codec
    const names: Nested_Codec[] = []
    if (codec_name !== undefined) {
      ensure_wire_codec_name(
        codec_name,
        `${operation_target}.${direction} nested shape ${target}`,
      )
      const members = optional_object_member(
        object_value(shape, `Smithy AST.shapes.${target}`),
        "members",
        `Smithy AST.shapes.${target}`,
      )
      names.push({
        name: codec_name,
        ...(() => {
          const descriptor =
            WIRE_CODEC_DESCRIPTORS[codec_name as Wire_Codec_Name]
          const explicit_width = codec === undefined
            ? undefined
            : optional_integer_member(
              object_value(codec, `Smithy AST.shapes.${target}.${WIRE_CODEC_TRAIT_ID}`),
              "width",
              `Smithy AST.shapes.${target}.${WIRE_CODEC_TRAIT_ID}`,
              1,
            )
          const width = explicit_width ??
            (descriptor?.width === "fixed" ? descriptor.min_width : undefined)
          return width === undefined ? {} : { width }
        })(),
        ...(codec_name === "enum"
          ? { enum_values: enum_values_for_shape(target) ?? [] }
          : {}),
        ...(codec_name === "union"
          ? { union_tags: union_tags_for_shape(target, members) }
          : {}),
      })
    }
    const child_targets: string[] = []
    if (type === "list") {
      const member = optional_object_member(
        object_value(shape, `Smithy AST.shapes.${target}`),
        "member",
        `Smithy AST.shapes.${target}`,
      )
      const child = member?.["target"]
      if (typeof child === "string") child_targets.push(child)
    } else if (type === "map") {
      const key = optional_object_member(
        object_value(shape, `Smithy AST.shapes.${target}`),
        "key",
        `Smithy AST.shapes.${target}`,
      )?.["target"]
      const value = optional_object_member(
        object_value(shape, `Smithy AST.shapes.${target}`),
        "value",
        `Smithy AST.shapes.${target}`,
      )?.["target"]
      if (typeof key === "string") child_targets.push(key)
      if (typeof value === "string") child_targets.push(value)
    } else if (type === "union" || type === "structure") {
      const members = optional_object_member(
        object_value(shape, `Smithy AST.shapes.${target}`),
        "members",
        `Smithy AST.shapes.${target}`,
      )
      for (const member of Object.values(members ?? {})) {
        const child = object_value(member, `Smithy AST.shapes.${target}`).target
        if (typeof child === "string") child_targets.push(child)
      }
    }
    const infer_grandchildren =
      codec_name === "list" || codec_name === "map" || codec_name === "union"
    for (const child of child_targets) {
      names.push(...nested_codec_descriptors(
        child,
        next_ancestors,
        infer_grandchildren,
        depth + 1,
      ))
      if (names.length > MAX_GENERATED_NESTED_CODEC_ENTRIES) {
        throw new Error(
          `${operation_target}.${direction} nested codec metadata exceeds ` +
            `${MAX_GENERATED_NESTED_CODEC_ENTRIES} entries`,
        )
      }
    }
    return names
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
        const codec_widths: (number | undefined)[] = []
        const member_shape = shapes[member_target]
        const member_shape_traits = member_shape === undefined
          ? undefined
          : optional_object_member(
            object_value(member_shape, `Smithy AST.shapes.${member_target}`),
            "traits",
            `Smithy AST.shapes.${member_target}`,
          )
        const codec = traits?.[WIRE_CODEC_TRAIT_ID] ??
          member_shape_traits?.[WIRE_CODEC_TRAIT_ID]
        if (codec !== undefined) {
          const codec_location = traits?.[WIRE_CODEC_TRAIT_ID] !== undefined
            ? `${target}.${member_name}.${WIRE_CODEC_TRAIT_ID}`
            : `Smithy AST.shapes.${member_target}.${WIRE_CODEC_TRAIT_ID}`
          const codec_name = string_member(
            object_value(codec, codec_location),
            "name",
            codec_location,
          )
          ensure_wire_codec_name(codec_name, codec_location)
          codecs.push(codec_name)
          codec_widths.push(
            optional_integer_member(
              object_value(codec, codec_location),
              "width",
              codec_location,
              1,
            ),
          )
        }
        const enum_values = !codecs.includes("enum") ||
            member_shape === undefined ||
            shape_type(
              object_value(member_shape, `Smithy AST.shapes.${member_target}`),
              `Smithy AST.shapes.${member_target}`,
            ) !== "enum"
          ? undefined
          : Object.entries(
              object_member(
                object_value(member_shape, `Smithy AST.shapes.${member_target}`),
                "members",
                `Smithy AST.shapes.${member_target}`,
              ),
            ).map(([member_name, member_value]) => {
              const traits = optional_object_member(
                object_value(member_value, `${member_target}.${member_name}`),
                "traits",
                `${member_target}.${member_name}`,
              )
              return traits?.["smithy.api#enumValue"] === undefined
                ? member_name
                : string_member(
                    traits,
                    "smithy.api#enumValue",
                    `${member_target}.${member_name}.traits`,
                )
            })
        const nested_descriptors = nested_codec_descriptors(
          member_target,
          new Set(),
          codecs.some((codec) => codec === "list" || codec === "map" || codec === "union"),
        )
        const nested_codecs = nested_descriptors[0]?.name === codecs[0]
          ? nested_descriptors.slice(1)
          : nested_descriptors
        const nested_enum_values = nested_codecs.map(
          (nested) => nested.enum_values ?? [],
        )
        const nested_widths = nested_codecs.map((nested) => nested.width)
        const union_tags = codecs.includes("union") && member_shape !== undefined
          ? union_tags_for_shape(
              member_target,
              optional_object_member(
                object_value(member_shape, `Smithy AST.shapes.${member_target}`),
                "members",
                `Smithy AST.shapes.${member_target}`,
              ),
            )
          : undefined
        const nested_union_tags = nested_codecs.map(
          (nested) => nested.union_tags ?? [],
        )
        const required = required_parent && traits?.["smithy.api#required"] !== undefined
        const field_plan: Wire_Operation_Field_Plan = {
          index: fields.length,
          ...(codecs.length === 0 ? {} : { codecs }),
          ...(codec_widths.some((width) => width !== undefined)
            ? { codec_widths }
            : {}),
          ...(enum_values === undefined ? {} : { enum_values }),
          ...(union_tags === undefined ? {} : { union_tags }),
          ...(nested_codecs.length === 0 ? {} : {
            nested_codecs: nested_codecs.map((nested) => nested.name),
            nested_widths,
            nested_enum_values,
            nested_union_tags,
          }),
          path: [...path, member_name],
          required,
          role,
          shape: shape_name(member_target),
        }
        if (fields.length >= MAX_GENERATED_OPERATION_FIELDS) {
          throw new Error(
            `${operation_target}.${direction} operation field plan exceeds ` +
              `${MAX_GENERATED_OPERATION_FIELDS} fields; use a bounded/streaming shape`,
          )
        }
        const encoded_width = fixed_field_width(field_plan)
        fields.push({
          ...field_plan,
          ...(encoded_width === undefined ? {} : { encoded_width }),
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
