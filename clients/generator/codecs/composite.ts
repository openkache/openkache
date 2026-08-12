/** Shared operation plans, codecs, and framing used by language renderers. */

import type {
  Api_Member,
  Api_Structure,
} from "../../operation_models"
import { typescript_name } from "../../generator_names"
import type { Operation_Field_Plan } from "../../operation_plans"
import { typescript_api_name } from "../../api_shape_renderers"

import {
  go_api_name,
  operation_field_name,
  python_api_name,
} from "./contract"
import type { Application_Value_Language } from "./registry"
import { operation_composite_fields, type Managed_Api_Operation } from "./shapes"

function composite_structure_name(
  structure_name: string,
  language: Application_Value_Language,
): string {
  switch (language) {
    case "typescript":
      return typescript_api_name(structure_name)
    case "go":
      return go_api_name(structure_name)
    case "python":
      return python_api_name(structure_name)
    case "swift":
      return `Smithy_${typescript_name(structure_name)}`
    case "java":
    case "kotlin":
    case "dart":
      return structure_name
    case "rust":
      return `smithy::${structure_name}`
    case "csharp":
      return `Smithy.${structure_name}`
  }
}

function composite_missing_expression(
  language: Application_Value_Language,
  expression: string,
): string {
  switch (language) {
    case "typescript":
      return `${expression} === undefined`
    case "python":
      return `${expression} is None`
    case "swift":
      return `${expression} == nil`
    case "rust":
      return `${expression}.is_none()`
    case "go":
      return `${expression} == nil`
    case "java":
    case "kotlin":
    case "dart":
    case "csharp":
      return `${expression} == null`
  }
}

interface Rendered_Composite_Structure {
  readonly expression: string
  readonly leaves: readonly string[]
}

/**
 * Recursively projects flattened response paths into generated DTOs.
 *
 * The wire decoder still returns one ordered value vector. This is the
 * language-specific object-construction edge: nested structure shape is
 * reconstructed from the Smithy model rather than an operation-name switch.
 */
export function render_composite_output(
  operation: Managed_Api_Operation,
  language: Application_Value_Language,
  decoded: readonly string[],
): string {
  const root = operation.plan.api.structures.find(
    (structure) => structure.name === operation.output,
  )
  if (root === undefined) {
    throw new Error(`operation ${operation.name} output structure ${operation.output} is missing`)
  }
  const fields = operation_composite_fields(operation)
  const values = new Map(
    fields.map((field, index) => [field.path.join("\u0000"), decoded[index]!]),
  )
  const plan_for_path = (path: readonly string[]): Operation_Field_Plan | undefined =>
    fields.find((field) =>
      field.path.length === path.length &&
      field.path.every((segment, index) => segment === path[index])
    )
  const has_prefix = (path: readonly string[]): boolean =>
    fields.some((field) =>
      field.path.length > path.length &&
      path.every((segment, index) => segment === field.path[index])
    )
  const render_structure = (
    structure: Api_Structure,
    path: readonly string[],
    required: boolean,
  ): Rendered_Composite_Structure => {
    const children: string[] = []
    const leaves: string[] = []
    for (const member of structure.members) {
      const member_path = [...path, member.name]
      const leaf = plan_for_path(member_path)
      let expression: string | undefined
      let child_leaves: readonly string[] = []
      if (leaf !== undefined) {
        expression = values.get(member_path.join("\u0000"))
        if (expression === undefined) {
          throw new Error(
            `operation ${operation.name} composite field ${member_path.join(".")} has no decoded value`,
          )
        }
        child_leaves = [expression]
      } else if (member.type.kind === "structure" && member.type.name !== undefined && has_prefix(member_path)) {
        const nested = operation.plan.api.structures.find(
          (candidate) => candidate.name === member.type.name,
        )
        if (nested === undefined) {
          throw new Error(
            `operation ${operation.name} output path ${member_path.join(".")} targets missing structure ${member.type.name}`,
          )
        }
        const rendered = render_structure(nested, member_path, member.required)
        expression = rendered.expression
        child_leaves = rendered.leaves
      } else {
        throw new Error(
          `operation ${operation.name} composite output member ${member_path.join(".")} is not present in the ordered response plan`,
        )
      }
      children.push(render_composite_member(language, member, expression, member_path))
      leaves.push(...child_leaves)
    }
    const type_name = composite_structure_name(structure.name, language)
    const constructed = render_composite_structure(language, type_name, children)
    if (required || leaves.length === 0) {
      return { expression: constructed, leaves }
    }
    const missing = leaves.map((leaf) => composite_missing_expression(language, leaf))
    return {
      expression: render_optional_composite_structure(language, constructed, missing, type_name),
      leaves,
    }
  }
  return render_structure(root, [], true).expression
}

function render_composite_member(
  language: Application_Value_Language,
  member: Api_Member,
  expression: string,
  _path: readonly string[],
): string {
  const name = operation_field_name(member, language)
  switch (language) {
    case "java":
    case "kotlin":
      return expression
    case "dart":
    case "typescript":
    case "go":
      return `${name}: ${expression}`
    case "python":
      return `${name}=${expression}`
    case "swift":
      return `${name}: ${expression}`
    case "csharp":
    case "rust":
      return `${name}${language === "csharp" ? " = " : ": "}${expression}`
  }
}

function render_composite_structure(
  language: Application_Value_Language,
  type_name: string,
  children: readonly string[],
): string {
  switch (language) {
    case "java":
      return `new ${type_name}(${children.join(", ")})`
    case "kotlin":
      return `${type_name}(${children.join(", ")})`
    case "dart":
      // Decoded members are runtime values; a const constructor cannot
      // capture them in Dart.
      return `${type_name}(${children.join(", ")})`
    case "typescript":
      return `{ ${children.join(", ")} }`
    case "go":
      return `${type_name}{${children.join(", ")}}`
    case "python":
      return `${type_name}(${children.join(", ")})`
    case "swift":
      return `${type_name}(${children.join(", ")})`
    case "csharp":
      return `new ${type_name} { ${children.join(", ")} }`
    case "rust":
      return `${type_name} { ${children.join(", ")} }`
  }
}

function render_optional_composite_structure(
  language: Application_Value_Language,
  constructed: string,
  missing: readonly string[],
  type_name: string,
): string {
  const condition = missing.join(language === "python" ? " and " : " && ")
  switch (language) {
    case "java":
    case "dart":
    case "csharp":
      return `(${condition} ? null : ${constructed})`
    case "kotlin":
      return `(if (${condition}) null else ${constructed})`
    case "typescript":
      return `(${condition} ? undefined : ${constructed})`
    case "python":
      return `(${constructed} if not (${condition}) else None)`
    case "swift":
      return `(${condition} ? nil : ${constructed})`
    case "go":
      return `func() *${type_name} { if ${condition} { return nil }; value := ${constructed}; return &value }()`
    case "rust":
      return `if ${condition} { None } else { Some(${constructed}) }`
  }
}
