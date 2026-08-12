/**
 * Language-neutral operation IR for generated clients.
 *
 * Smithy extraction and target-language renderers deliberately meet at this
 * module. The IR preserves ordered fields, repeated roles, requiredness, and
 * canonical wire codec metadata without selecting a protocol-v1 convenience
 * route or a target-language syntax.
 */

import type {
  Api_Contract,
  Api_Operation,
  Api_Operation_Contract,
  Api_Structure,
  Api_Type,
  Operation_Field_Role,
} from "./operation_models"
import {
  derive_wire_operation_descriptor,
  layout_encoded_len_from_lengths,
  type Wire_Operation_Field_Layout,
  type Wire_Operation_Field_Plan,
} from "../protocol/wire"

export interface Operation_Field_Requirement {
  readonly direction: "input" | "output"
  /** Exact width for flat required fixed-width scalar fields, when known. */
  readonly encoded_width?: number
  readonly parent?: Operation_Field_Role
  readonly role: Operation_Field_Role
}

/** One field selected directly from an operation input/output shape. */
export interface Operation_Field_Plan {
  /** Codec identifiers copied from the canonical protocol field plan. */
  readonly codecs?: readonly string[]
  readonly direction: "input" | "output"
  /** Exact width copied from the canonical fixed-width field plan. */
  readonly encoded_width?: number
  /** Enum members copied from the canonical protocol field plan. */
  readonly enum_values?: readonly string[]
  readonly name: string
  readonly nested_codecs?: readonly string[]
  /** Fixed widths known for nested codecs; undefined means variable/unknown. */
  readonly nested_widths?: readonly (number | undefined)[]
  readonly nested_enum_values?: readonly (readonly string[])[]
  /** Union tags copied from the canonical protocol field plan. */
  readonly nested_union_tags?: readonly (readonly number[])[]
  readonly path: readonly string[]
  readonly required: boolean
  readonly role: Operation_Field_Role
  readonly type: Api_Type
  readonly union_tags?: readonly number[]
}

/** Operation request/response shape projection shared by every renderer. */
export interface Operation_Shape_Plan {
  readonly fields: readonly Operation_Field_Plan[]
  /** Shape-selected layout copied from the canonical wire descriptor. */
  readonly layout?: Wire_Operation_Field_Layout
  readonly name: string
}

/** Operation-agnostic plan derived from Smithy roles and shapes. */
export interface Operation_Plan {
  readonly input: Operation_Shape_Plan
  readonly output: Operation_Shape_Plan
}

/**
 * Computes one operation payload size from field lengths alone.
 *
 * The operation IR deliberately exposes this as a layout primitive rather
 * than a response-kind switch. Renderers and transport adapters can reserve
 * exactly what the generated descriptor requires without constructing an
 * intermediate payload.
 */
export function operation_payload_encoded_len(
  operation: { readonly contract: Api_Operation_Contract },
  direction: "input" | "output",
  lengths: readonly (number | undefined)[],
): number {
  const descriptor = derive_wire_operation_descriptor(operation.contract)
  const layout = direction === "input"
    ? descriptor.request_layout
    : descriptor.response_layout
  return layout_encoded_len_from_lengths(layout, lengths)
}

function operation_structure_for(
  api: Api_Contract,
  operation: Api_Operation,
  direction: "input" | "output",
): Api_Structure {
  const name = direction === "input" ? operation.input : operation.output
  const structure = api.structures.find((candidate) => candidate.name === name)
  if (structure === undefined) {
    throw new Error(
      `operation ${operation.name} ${direction} structure ${name} is missing from the Smithy contract`,
    )
  }
  return structure
}

function operation_shape_plan(
  contract: { readonly api: Api_Contract },
  operation: Api_Operation,
  direction: "input" | "output",
): Operation_Shape_Plan {
  const operation_contract = operation.contract
  const structure = operation_structure_for(contract.api, operation, direction)
  const canonical_plan = operation_contract === undefined
    ? undefined
    : direction === "input"
    ? operation_contract.request_plan
    : operation_contract.response_plan
  if (canonical_plan !== undefined && operation_contract !== undefined) {
    const resolve_canonical_field = (
      field: Wire_Operation_Field_Plan,
    ): Operation_Field_Plan => {
      let current = structure
      for (const [path_index, path_member] of field.path.entries()) {
        const member = current.members.find(
          (candidate) => candidate.name === path_member,
        )
        if (member === undefined) {
          throw new Error(
            `operation ${operation.name} ${direction} plan path ${field.path.join(".")} ` +
              `does not exist in ${current.name}`,
          )
        }
        if (path_index === field.path.length - 1) {
          if (member.operation_field_role !== field.role) {
            throw new Error(
              `operation ${operation.name} ${direction} plan role ${field.role} ` +
                `does not match ${field.path.join(".")}`,
            )
          }
          if (
            field.nested_codecs !== undefined &&
            (
              (field.nested_widths !== undefined &&
                field.nested_codecs.length !== field.nested_widths.length) ||
              field.nested_codecs.length !== (field.nested_enum_values?.length ?? 0) ||
              field.nested_codecs.length !== (field.nested_union_tags?.length ?? 0)
            )
          ) {
            throw new Error(
              `operation ${operation.name} ${direction} plan ${field.path.join(".")} ` +
                "has misaligned nested codec metadata",
            )
          }
          return {
            direction,
            ...(field.encoded_width === undefined
              ? {}
              : { encoded_width: field.encoded_width }),
            name: member.name,
            ...(field.codecs === undefined ? {} : { codecs: field.codecs }),
            ...(field.enum_values === undefined ? {} : { enum_values: field.enum_values }),
            ...(field.nested_codecs === undefined
              ? {}
              : { nested_codecs: field.nested_codecs }),
            ...(field.nested_widths === undefined
              ? {}
              : { nested_widths: field.nested_widths }),
            ...(field.nested_enum_values === undefined
              ? {}
              : { nested_enum_values: field.nested_enum_values }),
            ...(field.nested_union_tags === undefined
              ? {}
              : { nested_union_tags: field.nested_union_tags }),
            path: field.path,
            required: field.required,
            role: field.role,
            type: member.type,
            ...(field.union_tags === undefined ? {} : { union_tags: field.union_tags }),
          }
        }
        if (member.type.kind !== "structure" || member.type.name === undefined) {
          throw new Error(
            `operation ${operation.name} ${direction} plan path ${field.path.join(".")} ` +
              `passes through non-structure member ${member.name}`,
          )
        }
        const nested = contract.api.structures.find(
          (candidate) => candidate.name === member.type.name,
        )
        if (nested === undefined) {
          throw new Error(
            `operation ${operation.name} ${direction} plan path ${field.path.join(".")} ` +
              `targets missing structure ${member.type.name}`,
          )
        }
        current = nested
      }
      throw new Error(
        `operation ${operation.name} ${direction} canonical plan contains an empty path`,
      )
    }
    const fields = canonical_plan.map(resolve_canonical_field)
    const descriptor = derive_wire_operation_descriptor(operation_contract)
    const layout = direction === "input"
      ? descriptor.request_layout
      : descriptor.response_layout
    if (
      layout === "dense" &&
      fields.some((field) => field.encoded_width === undefined || field.encoded_width === 0)
    ) {
      throw new Error(
        `operation ${operation.name} ${direction} dense layout is missing a fixed field width`,
      )
    }
    return { fields, layout, name: structure.name }
  }

  const fields: Operation_Field_Plan[] = []
  const visit = (
    current: Api_Structure,
    path: readonly string[],
    ancestors: ReadonlySet<string>,
    required_parent: boolean,
  ): void => {
    if (ancestors.has(current.name)) {
      throw new Error(
        `operation ${operation.name} ${direction} shape cycle through ${current.name}`,
      )
    }
    const next_ancestors = new Set(ancestors).add(current.name)
    for (const member of current.members) {
      const member_path = [...path, member.name]
      const role = member.operation_field_role
      if (role !== undefined) {
        fields.push({
          direction,
          name: member.name,
          path: member_path,
          required: required_parent && member.required,
          role,
          type: member.type,
        })
      }
      if (member.type.kind === "structure" && member.type.name !== undefined) {
        const nested = contract.api.structures.find(
          (candidate) => candidate.name === member.type.name,
        )
        if (nested === undefined) {
          throw new Error(
            `operation ${operation.name} ${direction} field ${member.name} targets missing structure ${member.type.name}`,
          )
        }
        visit(
          nested,
          member_path,
          next_ancestors,
          required_parent && member.required,
        )
      }
    }
  }
  visit(structure, [], new Set(), true)
  return { fields, name: structure.name }
}

/**
 * Derives the language-neutral operation plan from shape members. No request
 * or response family table is consulted here; repeated roles are preserved in
 * Smithy order so GET3/GETSET-like shapes remain data rather than generator
 * branches.
 */
export function derive_operation_plan(
  contract: { readonly api: Api_Contract },
  operation: Api_Operation,
): Operation_Plan {
  const input = operation_shape_plan(contract, operation, "input")
  const output = operation_shape_plan(contract, operation, "output")
  return {
    input,
    output,
  }
}

/**
 * Counts a modeled role without baking a domain vocabulary into the IR.
 *
 * Consumers that need a compatibility ABI cardinality can ask for that role at the
 * adapter boundary; generic renderers keep the complete open field plan.
 */
export function operation_field_count(
  plan: Operation_Plan,
  direction: "input" | "output",
  role: Operation_Field_Role,
): number {
  return plan[direction].fields.filter((field) => field.role === role).length
}

/**
 * Derives required role bindings from the actual Smithy structures. Required
 * fields are the only generation obligations; optional and repeated roles are
 * carried by the operation plan without a special-case operation matrix.
 */
export function operation_field_requirements(
  contract: { readonly api: Api_Contract },
  operation: Api_Operation & { readonly contract: Api_Operation_Contract },
): readonly Operation_Field_Requirement[] {
  const plan = derive_operation_plan(contract, operation)
  return [...plan.input.fields, ...plan.output.fields]
    .filter((field) => field.required && field.path.length === 1)
    .map(({ direction, role }) => ({ direction, role }))
}

/**
 * Fails closed for the public client model when a renderer-consumed semantic
 * role is absent. The protocol-only compatibility fixture path remains permissive,
 * but production clients cannot silently fall back to a historical member
 * name after a Smithy rename.
 */
export function validate_operation_field_bindings(api: Api_Contract): void {
  for (const operation of api.operations) {
    if (operation.contract === undefined) continue
    const managed_operation = operation as Api_Operation & {
      readonly contract: Api_Operation_Contract
    }
    const requirements = operation_field_requirements({ api }, managed_operation)
    const structures = new Map<"input" | "output", Api_Structure>([
      ["input", operation_structure_for(api, operation, "input")],
      ["output", operation_structure_for(api, operation, "output")],
    ])
    for (const requirement of requirements) {
      const parent_structure = structures.get(requirement.direction)!
      let structure = parent_structure
      if (requirement.parent !== undefined) {
        const parent = parent_structure.members.find(
          (member) => member.operation_field_role === requirement.parent,
        )
        if (parent === undefined) {
          throw new Error(
            `operation ${operation.name} ${requirement.direction} is missing operationField role ${requirement.parent}`,
          )
        }
        if (parent.type.kind !== "structure" || parent.type.name === undefined) {
          throw new Error(
            `operation ${operation.name} ${requirement.direction} operationField role ${requirement.parent} must target a structure`,
          )
        }
        const nested = api.structures.find(
          (candidate) => candidate.name === parent.type.name,
        )
        if (nested === undefined) {
          throw new Error(
            `operation ${operation.name} ${requirement.direction} operationField role ${requirement.parent} targets missing structure ${parent.type.name}`,
          )
        }
        structure = nested
      }
      if (
        !structure.members.some(
          (member) => member.operation_field_role === requirement.role,
        )
      ) {
        const parent = requirement.parent === undefined
          ? ""
          : ` inside ${requirement.parent}`
        throw new Error(
          `operation ${operation.name} ${requirement.direction} is missing operationField role ${requirement.role}${parent}`,
        )
      }
    }
  }
}
