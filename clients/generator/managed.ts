/** Shared operation plans, codecs, and framing used by language renderers. */

import type {
  Api_Contract,
  Api_Member,
  Api_Operation,
  Api_Operation_Contract,
  Operation_Field_Role,
} from "../operation_models"
import type { Client_Contract } from "../client_contract"
import {
  lower_camel_case,
  pascal_case,
  snake_case,
} from "../generator_names"
import { operation_result_plan, type Operation_Result_Plan } from "../operation_results"
import {
  operation_result_projection,
  type Operation_Response_Projection_Kind,
  type Operation_Result_Kind,
  type Operation_Result_Projection,
} from "../compatibility_result_projections"
import {
  request_transport_plan,
  uses_compact_request_route as request_adapter_uses_route,
  type Compact_Request_Adapter_Route,
  type Request_Transport_Plan,
} from "../compatibility_request_adapters"
import {
  derive_operation_plan,
  operation_field_requirements,
  type Operation_Field_Plan,
  type Operation_Plan,
} from "../operation_plans"

export * from "./codecs"
import {
  WIRE_CODEC_REGISTRY,
  go_api_name,
  managed_result_projection,
  operation_composite_value_count,
  operation_field_name,
  operation_field_binding,
  operation_structure,
  operation_uses_generic_field_sequence_request,
  render_application_value_codec,
  wire_codec_for_field,
  type Application_Value_Codec,
  type Application_Value_Codec_Pair,
  type Application_Value_Language,
  type Managed_Api_Operation,
  type Managed_Operation_Plan,
  type Operation_Invocation_Plan,
  type Wire_Codec_Registration,
} from "./codecs"

export function application_value_codecs(
  contract: Client_Contract,
  operation: Api_Operation & { readonly contract: Api_Operation_Contract },
  request: Request_Transport_Plan,
  operation_plan: Operation_Plan,
  result_plan: Operation_Result_Plan,
  result_projection: Operation_Result_Projection,
): Application_Value_Codec_Pair | undefined {
  const request_is_opaque = request.request_framing === "opaque"
  const response_is_opaque = result_plan.response_transport === "opaque" &&
    result_projection.compatibility_adapter === undefined
  if (!request_is_opaque && !response_is_opaque) return undefined
  const input_payload = request_is_opaque
    ? opaque_field_from_plan(contract.api, operation, operation_plan, "input")
    : undefined
  const output_payload = response_is_opaque
    ? opaque_field_from_plan(contract.api, operation, operation_plan, "output")
    : undefined
  /*
   * Request and response framing are independent descriptors. Keep a
   * placeholder raw-byte codec for the side that is not opaque so renderers
   * can share one codec-registration path without inventing a second
   * operation-family branch. The request/response booleans above remain the
   * source of truth for whether that expression is actually emitted.
   */
  const input_codec = input_payload === undefined
    ? "raw_bytes"
    : wire_codec_for_field(
      operation,
      operation_plan.input.fields.find(
        (field) => field.path[field.path.length - 1] === input_payload.name,
      ) ?? operation_plan.input.fields[0]!,
    ).name
  const output_codec = output_payload === undefined
    ? "raw_bytes"
    : wire_codec_for_field(
      operation,
      operation_plan.output.fields.find(
        (field) => field.path[field.path.length - 1] === output_payload.name,
      ) ?? operation_plan.output.fields[0]!,
    ).name
  return {
    input: input_codec,
    input_type: input_payload?.type ?? { kind: "blob" },
    output: output_codec,
    output_type: output_payload?.type ?? { kind: "blob" },
  }
}

export function operation_request_is_opaque(
  operation: Managed_Api_Operation,
): boolean {
  return operation.plan.request.request_framing === "opaque"
}

/**
 * Selects only the transport invocation family from the derived field plan.
 * This is a shape query, not a request-kind lookup: a new operation can
 * compose existing roles without modifying a route table.
 */
export function managed_operation_entries(
  contract: Client_Contract,
): readonly Managed_Api_Operation[] {
  return contract.api.operations.flatMap((operation) => {
    if (operation.contract === undefined) return []
    const opcode = contract.opcodes.find((entry) => entry.name === operation.name)
    if (opcode === undefined) {
      throw new Error(`operation ${operation.name} has no matching protocol opcode`)
    }
    const managed_operation: Api_Operation & {
      readonly contract: Api_Operation_Contract
    } = {
      ...operation,
      contract: operation.contract,
    }
    const operation_plan = derive_operation_plan(contract, managed_operation)
    const request = request_transport_plan(managed_operation.contract)
    const result_plan = operation_result_plan(managed_operation.contract)
    const result_projection = operation_result_projection(
      managed_operation.contract,
      request,
    )
    const binding = operation_field_binding(contract, managed_operation)
    const plan: Managed_Operation_Plan = {
      api: contract.api,
      application_value_codecs: application_value_codecs(
        contract,
        managed_operation,
        request,
        operation_plan,
        result_plan,
        result_projection,
      ),
      binding,
      contract: managed_operation.contract,
      input: managed_operation.input,
      invocation: operation_invocation_plan(request),
      name: managed_operation.name,
      opcode,
      operation: operation_plan,
      output: managed_operation.output,
      request,
      result_plan,
      result_projection,
      required_fields: operation_field_requirements(contract, managed_operation),
      strict_operation_bindings: contract.strict_operation_bindings,
    }
    return [{
      ...managed_operation,
      binding,
      opcode,
      plan,
    }]
  })
}

export function has_application_value_codec(
  operations: readonly Managed_Api_Operation[],
  codec: Application_Value_Codec,
): boolean {
  return operations.some(
    (operation) =>
      operation.plan.application_value_codecs?.input === codec ||
      operation.plan.application_value_codecs?.output === codec,
  )
}

export function has_wire_codec(
  operations: readonly Managed_Api_Operation[],
  codec_names: readonly string[],
): boolean {
  const names = new Set(codec_names)
  return operations.some((operation) => {
    const application = operation.plan.application_value_codecs
    if (
      application !== undefined &&
      (names.has(application.input) || names.has(application.output))
    ) return true
    return [
      ...operation.plan.operation.input.fields,
      ...operation.plan.operation.output.fields,
    ].some((field) => {
      /*
       * Operation plans flatten role-bearing members through enclosing
       * structures. A future non-role structure may still be present in a
       * permissive legacy plan, but it is not itself a payload codec. Keep
       * helper discovery side-effect free for those shapes; the strict
       * field-sequence renderers still resolve and validate every encoded
       * field through wire_codec_for_field.
       */
      const declared = field.codecs?.[0]
      if (declared !== undefined) {
        return names.has(wire_codec_for_field(operation, field).name)
      }
      const registration = WIRE_CODEC_REGISTRY.find((candidate) =>
        candidate.matches(field.type)
      )
      return registration !== undefined && names.has(registration.name)
    })
  })
}

/**
 * Resolves the single member carried by an opaque application-value frame.
 *
 * Generic Smithy operations may use any role (for example `ack` or `token`) as
 * long as the canonical descriptor declares one field for the opaque frame.
 * Renderers resolve that field from the plan instead of baking a role name
 * into every language implementation.
 */
function opaque_field_from_plan(
  api: Api_Contract,
  operation: Api_Operation,
  plan: Operation_Plan,
  direction: "input" | "output",
): Api_Member {
  const fields = plan[direction].fields
  if (fields.length !== 1) {
    throw new Error(
      `operation ${operation.name} ${direction} opaque framing requires exactly one modeled field`,
    )
  }
  let structure = operation_structure(api, operation, direction)
  let member: Api_Member | undefined
  for (const [path_index, segment] of fields[0]!.path.entries()) {
    member = structure.members.find((candidate) => candidate.name === segment)
    if (member === undefined) {
      throw new Error(
        `operation ${operation.name} ${direction} opaque field path ${fields[0]!.path.join(".")} is missing`,
      )
    }
    if (path_index < fields[0]!.path.length - 1) {
      if (member.type.kind !== "structure" || member.type.name === undefined) {
        throw new Error(
          `operation ${operation.name} ${direction} opaque field path passes through non-structure member ${member.name}`,
        )
      }
      const nested = api.structures.find(
        (candidate) => candidate.name === member!.type.name,
      )
      if (nested === undefined) {
        throw new Error(
          `operation ${operation.name} ${direction} opaque field path targets missing structure ${member.type.name}`,
        )
      }
      structure = nested
    }
  }
  if (member === undefined) {
    throw new Error(
      `operation ${operation.name} ${direction} opaque framing has no field`,
    )
  }
  return member
}

function operation_opaque_field(
  operation: Managed_Api_Operation,
  direction: "input" | "output",
): Api_Member {
  return opaque_field_from_plan(
    operation.plan.api,
    operation,
    operation.plan.operation,
    direction,
  )
}

export function operation_opaque_field_name(
  operation: Managed_Api_Operation,
  direction: "input" | "output",
  language: "csharp" | "dart" | "go" | "java" | "kotlin" | "python" | "rust" | "swift" | "typescript",
): string {
  return operation_field_name(operation_opaque_field(operation, direction), language)
}

/**
 * Renders an operation request body without coupling it to the response
 * semantic route. In particular, an opaque request may have a status-only
 * response, while an empty request may have an opaque response.
 */
export function render_opaque_request_expression(
  language: Exclude<Application_Value_Language, "go">,
  operation: Managed_Api_Operation,
  diagnostic: string,
): string {
  if (!operation_request_is_opaque(operation)) {
    switch (language) {
      case "java":
        return "new byte[0]"
      case "kotlin":
        return "byteArrayOf()"
      case "dart":
        return "const <int>[]"
      case "typescript":
        return "new Uint8Array()"
      case "python":
        return "b''"
      case "swift":
        return "Data()"
      case "csharp":
        return "ReadOnlyMemory<byte>.Empty"
      case "rust":
        return "Vec::new()"
    }
  }
  const input_name = operation_opaque_field_name(operation, "input", language)
  const input_expression = (() => {
    switch (language) {
      case "java":
        return `input.${input_name}()`
      default:
        return `input.${input_name}`
    }
  })()
  const codecs = operation.plan.application_value_codecs
  if (codecs === undefined) {
    throw new Error(`operation ${operation.name} has no application-value codec plan`)
  }
  return render_application_value_codec(
    language,
    codecs,
    input_expression,
    "result.payload",
    diagnostic,
  ).encode
}

export function operation_fields(
  operation: Managed_Api_Operation,
  direction: "input" | "output",
  field: Operation_Field_Role,
): readonly Api_Member[] {
  const members = operation.plan.binding[direction][field]
  if (members === undefined || members.length === 0) {
    throw new Error(
      `operation ${operation.name} has no generated ${direction} ${field} members`,
    )
  }
  return members
}

/**
 * Returns the item-ID members only for operations whose request contract
 * actually carries item IDs. Renderers build all operation methods from one
 * pass, so asking for this role on PING, STATS, or namespace operations must
 * not fail generation before their own response projection is selected.
 */
export function operation_item_fields(
  operation: Managed_Api_Operation,
): readonly Api_Member[] {
  return operation_uses_compact_item_request(operation)
    ? operation_fields(operation, "input", "item_id")
    : []
}

/**
 * Resolves the request invocation independently of response semantics.
 *
 * The compact branch is intentionally marked `compatibility`; it is rendered
 * only by the protocol-v1 adapter. Generic operation renderers therefore have
 * exactly two request cases: an empty body or a descriptor-encoded payload.
 */
function operation_invocation_plan(
  request: Request_Transport_Plan,
): Operation_Invocation_Plan {
  if (request.compact_adapter !== undefined) {
    return { request: "compatibility" }
  }
  // An empty body is still a generic request projection. Treating it as a
  // global convenience call makes an otherwise route-less API fall back to
  // namespace/item arguments in the language renderers when it declares an
  // API-owned scope. Scope is semantic metadata; generic clients carry it in
  // the modeled body (when needed), never through a built-in invocation ABI.
  return { request: "generic" }
}

export function operation_is_global_empty(operation: Managed_Api_Operation): boolean {
  return operation.plan.request.request_framing === "empty" &&
    operation.plan.request.compact_adapter === undefined
}

export function operation_is_global_opaque(operation: Managed_Api_Operation): boolean {
  return operation.plan.request.request_framing === "opaque" &&
    operation.plan.request.compact_adapter === undefined
}

export function operation_is_global_field_sequence(
  operation: Managed_Api_Operation,
): boolean {
  return operation_uses_generic_field_sequence_request(operation)
}

export function operation_uses_compact_item_request(
  operation: Managed_Api_Operation,
): boolean {
  return request_adapter_uses_route(operation.plan.request, "item") ||
    request_adapter_uses_route(operation.plan.request, "set")
}

export function operation_uses_compact_namespace_request(
  operation: Managed_Api_Operation,
): boolean {
  return request_adapter_uses_route(operation.plan.request, "namespace")
}

export function operation_uses_compact_request_route(
  operation: Managed_Api_Operation,
  route: Compact_Request_Adapter_Route,
): boolean {
  return request_adapter_uses_route(operation.plan.request, route)
}

function operation_field_name_for(
  operation: Managed_Api_Operation,
  direction: "input" | "output",
  field: Operation_Field_Role,
  language: "csharp" | "dart" | "go" | "java" | "kotlin" | "python" | "rust" | "swift" | "typescript",
): string {
  const member = operation.plan.binding[direction][field]?.[0]
  if (member !== undefined) return operation_field_name(member, language)
  if (
    operation.plan.strict_operation_bindings &&
    operation.plan.required_fields.some(
      (requirement) =>
        requirement.direction === direction &&
        requirement.parent === undefined &&
        requirement.role === field,
    )
  ) {
    throw new Error(
      `operation ${operation.name} ${direction} is missing operationField role ${field}`,
    )
  }
  const fallback: Api_Member = {
    name: lower_camel_case(field),
    required: false,
    type: { kind: "string" },
  }
  return operation_field_name(fallback, language)
}

/** Resolves one Smithy field plan path to a language-specific input expression. */
function operation_field_path_expression(
  operation: Managed_Api_Operation,
  field: Operation_Field_Plan,
  language: Application_Value_Language,
): string {
  let expression = "input"
  let structure = operation_structure(
    operation.plan.api,
    operation,
    "input",
  )
  for (const segment of field.path) {
    const member = structure.members.find((candidate) => candidate.name === segment)
    if (member === undefined) {
      throw new Error(
        `operation ${operation.name} input path ${field.path.join(".")} is not present in ${structure.name}`,
      )
    }
    const name = operation_field_name(member, language)
    switch (language) {
      case "java":
        expression = `${expression}.${name}()`
        break
      case "kotlin":
      case "dart":
      case "typescript":
      case "python":
      case "rust":
      case "swift":
      case "csharp":
        expression = `${expression}.${name}`
        break
      case "go":
        expression = `${expression}.${name}`
        break
    }
    if (member.type.kind === "structure" && member.type.name !== undefined) {
      const nested = operation.plan.api.structures.find(
        (candidate) => candidate.name === member.type.name,
      )
      if (nested === undefined) {
        throw new Error(
          `operation ${operation.name} input path ${field.path.join(".")} targets missing structure ${member.type.name}`,
        )
      }
      structure = nested
    }
  }
  return expression
}

function operation_field_sequence_fields(
  operation: Managed_Api_Operation,
  direction: "input" | "output",
): readonly Operation_Field_Plan[] {
  const fields = operation.plan.operation[direction].fields
  if (fields.length === 0) {
    throw new Error(
      `operation ${operation.name} ${direction} field sequence has no modeled fields`,
    )
  }
  return fields
}

function operation_uses_dense_layout(
  operation: Managed_Api_Operation,
  direction: "input" | "output",
): boolean {
  const shape_layout = direction === "input"
    ? operation.plan.operation.input.layout
    : operation.plan.operation.output.layout
  const transport_layout = direction === "input"
    ? operation.plan.request.request_layout
    : operation.plan.result_plan.response_layout
  if (
    shape_layout !== undefined &&
    transport_layout !== undefined &&
    shape_layout !== transport_layout
  ) {
    throw new Error(
      `operation ${operation.name} ${direction} layout disagrees between the canonical shape plan and transport plan`,
    )
  }
  return shape_layout === "dense" || transport_layout === "dense"
}

function operation_dense_widths(
  operation: Managed_Api_Operation,
  direction: "input" | "output",
): readonly number[] {
  const fields = operation_field_sequence_fields(operation, direction)
  return fields.map((field) => {
    const width = field.encoded_width
    if (width === undefined || width === 0) {
      throw new Error(
        `operation ${operation.name} selected dense ${direction} layout without fixed field width`,
      )
    }
    return width
  })
}

function render_dense_widths(
  language: Application_Value_Language,
  operation: Managed_Api_Operation,
  direction: "input" | "output",
): string {
  const widths = operation_dense_widths(operation, direction)
  const values = widths.join(", ")
  switch (language) {
    case "java":
      return `new int[] { ${values} }`
    case "kotlin":
      return `intArrayOf(${values})`
    case "dart":
      return `<int>[${values}]`
    case "typescript":
    case "python":
    case "swift":
      return `[${values}]`
    case "csharp":
      return `new[] { ${values} }`
    case "go":
      return `[]int{${values}}`
    case "rust":
      return `&[${values}]`
  }
}

export function render_field_sequence_response_decode(
  language: Application_Value_Language,
  operation: Managed_Api_Operation,
  payload: string,
  diagnostic: string,
): string {
  const count = operation_composite_value_count(operation)
  if (!operation_uses_dense_layout(operation, "output")) {
    switch (language) {
      case "java":
        return `smithyDecodeFieldSequence(${payload}, ${count}, ${diagnostic})`
      case "kotlin":
        return `smithyDecodeFieldSequence(${payload}, ${count}, ${diagnostic})`
      case "dart":
        return `_smithyDecodeFieldSequence(${payload}, ${count}, ${diagnostic})`
      case "typescript":
        return `smithy_decode_field_sequence(${payload}, ${count}, ${diagnostic})`
      case "python":
        return `_smithy_decode_field_sequence(${payload}, ${count}, ${diagnostic})`
      case "swift":
        return `try smithyDecodeFieldSequence(${payload}, fieldCount: ${count}, operation: ${diagnostic})`
      case "csharp":
        return `DecodeFieldSequence(${payload}, ${count}, ${diagnostic})`
      case "go":
        return `smithyDecodeFieldSequence(${payload}, ${count}, ${diagnostic})`
      case "rust":
        return `smithy_decode_field_sequence(${payload}, ${count}, ${diagnostic})?`
    }
  }
  const widths = render_dense_widths(language, operation, "output")
  switch (language) {
    case "java":
      return `smithyDecodeDenseFields(${payload}, ${widths}, ${diagnostic})`
    case "kotlin":
      return `smithyDecodeDenseFields(${payload}, ${widths}, ${diagnostic})`
    case "dart":
      return `_smithyDecodeDenseFields(${payload}, ${widths}, ${diagnostic})`
    case "typescript":
      return `smithy_decode_dense_fields(${payload}, ${widths}, ${diagnostic})`
    case "python":
      return `_smithy_decode_dense_fields(${payload}, ${widths}, ${diagnostic})`
    case "swift":
      return `try smithyDecodeDenseFields(${payload}, widths: ${widths}, operation: ${diagnostic})`
    case "csharp":
      return `DecodeDenseFields(${payload}, ${widths}, ${diagnostic})`
    case "go":
      return `smithyDecodeDenseFields(${payload}, ${widths}, ${diagnostic})`
    case "rust":
      return `smithy_decode_dense_fields(${payload}, ${widths}, ${diagnostic})?`
  }
}

function operation_field_sequence_codec(
  operation: Managed_Api_Operation,
  field: Operation_Field_Plan,
): Wire_Codec_Registration {
  return wire_codec_for_field(operation, field)
}

/**
 * Renders one field-sequence request value for languages whose codec syntax is
 * expression-shaped. Go has a statement/error boundary and is rendered by its
 * operation-specific helper below.
 */
function render_field_sequence_encoded_value(
  language: Exclude<Application_Value_Language, "go">,
  operation: Managed_Api_Operation,
  field: Operation_Field_Plan,
  diagnostic: string,
): string {
  const input = operation_field_path_expression(operation, field, language)
  const encoded = operation_field_sequence_codec(operation, field).render(
    language,
    input,
    "field",
    diagnostic,
    undefined,
    undefined,
    field.type,
  ).encode
  if (field.required) return language === "rust" ? `Some(${encoded})` : encoded
  const with_value = encoded.split(input).join("value")
  switch (language) {
    case "java":
      return `(${input} == null ? null : ${encoded})`
    case "kotlin":
      return `${input}?.let { value -> ${with_value} }`
    case "dart":
      return `${input} == null ? null : ${
        encoded.split(input).join(`${input}!`)
      }`
    case "typescript":
      return `${input} === undefined ? undefined : ${
        encoded.split(input).join(`${input}!`)
      }`
    case "python":
      return `${input} if ${input} is None else ${encoded}`
    case "swift":
      return `${input}.map { value in ${with_value} }`
    case "csharp":
      return `${input} is null ? null : ${
        encoded.split(input).join(`${input}!`)
      }`
    case "rust":
      // Rust operation methods own their generated input. Consume optional
      // fields so raw byte vectors move directly into the request plan rather
      // than becoming borrowed values that must be cloned.
      return `${input}.map(|value| ${with_value})`
  }
}

export function render_field_sequence_request_payload(
  language: Exclude<Application_Value_Language, "go">,
  operation: Managed_Api_Operation,
  diagnostic: string,
): string {
  const values = operation_field_sequence_fields(operation, "input")
    .map((field) =>
      render_field_sequence_encoded_value(language, operation, field, diagnostic)
    )
  switch (language) {
    case "java":
      return operation_uses_dense_layout(operation, "input")
        ? `smithyEncodeDenseFields(${values.join(", ")})`
        : `smithyEncodeFieldSequence(${values.join(", ")})`
    case "kotlin":
      return operation_uses_dense_layout(operation, "input")
        ? `smithyEncodeDenseFields(listOf(${values.join(", ")}))`
        : `smithyEncodeFieldSequence(listOf(${values.join(", ")}))`
    case "dart":
      return operation_uses_dense_layout(operation, "input")
        ? `_smithyEncodeDenseFields(<List<int>>[${values.join(", ")}])`
        : `_smithyEncodeFieldSequence(<List<int>?>[${values.join(", ")}])`
    case "typescript":
      return operation_uses_dense_layout(operation, "input")
        ? `smithy_encode_dense_fields([${values.join(", ")}])`
        : `smithy_encode_field_sequence([${values.join(", ")}])`
    case "python":
      return operation_uses_dense_layout(operation, "input")
        ? `_smithy_encode_dense_fields([${values.join(", ")}])`
        : `_smithy_encode_field_sequence([${values.join(", ")}])`
    case "swift":
      return operation_uses_dense_layout(operation, "input")
        ? `try smithyEncodeDenseFields([${values.join(", ")}])`
        : `try smithyEncodeFieldSequence([${values.join(", ")}])`
    case "csharp":
      return operation_uses_dense_layout(operation, "input")
        ? `EncodeDenseFields(new ReadOnlyMemory<byte>[] { ${values.join(", ")} })`
        : `EncodeFieldSequence(new ReadOnlyMemory<byte>?[] { ${values.join(", ")} })`
    case "rust":
      return `smithy_encode_field_sequence(&[${values.join(", ")}])?`
  }
}

/** Renders the canonical ordered field vector for the Rust core executor. */
function render_field_sequence_request_fields(
  language: "rust",
  operation: Managed_Api_Operation,
  diagnostic: string,
): string {
  const values = operation_field_sequence_fields(operation, "input")
    .map((field) =>
      render_field_sequence_encoded_value(language, operation, field, diagnostic)
    )
  return `vec![${values.join(", ")}]`
}

/**
 * Renders the request payload for a generic operation independently of its
 * response projection.
 *
 * Generic request framing is one of the few decisions shared by every
 * language renderer: ordered fields use the canonical field sequence and an
 * opaque request uses the registered value codec. Keeping that decision here
 * prevents each response-projection branch from growing another operation-family
 * matcher. Go has a statement-producing renderer and therefore keeps its
 * small adapter below.
 */
function render_generic_request_payload(
  language: Exclude<Application_Value_Language, "go">,
  operation: Managed_Api_Operation,
  diagnostic: string,
): string | undefined {
  if (operation.plan.request.request_framing === "empty") {
    switch (language) {
      case "java":
        return "new byte[0]"
      case "kotlin":
        return "byteArrayOf()"
      case "dart":
        return "const <int>[]"
      case "typescript":
        return "new Uint8Array()"
      case "python":
        return "b''"
      case "swift":
        return "Data()"
      case "csharp":
        return "ReadOnlyMemory<byte>.Empty"
      case "rust":
        return "Vec::new()"
    }
  }
  if (operation_uses_generic_field_sequence_request(operation)) {
    return language === "rust"
      ? render_field_sequence_request_fields(language, operation, diagnostic)
      : render_field_sequence_request_payload(language, operation, diagnostic)
  }
  if (operation_request_is_opaque(operation)) {
    return render_opaque_request_expression(language, operation, diagnostic)
  }
  return undefined
}

type Expression_Invocation_Language = Exclude<
  Application_Value_Language,
  "go" | "rust"
>

function expression_empty_bytes(language: Expression_Invocation_Language): string {
  switch (language) {
    case "java":
      return "new byte[0]"
    case "kotlin":
      return "byteArrayOf()"
    case "dart":
      return "const <int>[]"
    case "typescript":
      return "new Uint8Array()"
    case "python":
      return "b''"
    case "swift":
      return "Data()"
    case "csharp":
      return "ReadOnlyMemory<byte>.Empty"
  }
}

/**
 * Renders the generic request invocation used by expression-oriented
 * languages. The result route is deliberately absent from this helper:
 * request framing and scope come only from `Operation_Invocation_Plan`.
 *
 * Compact protocol-v1 requests return `undefined` so their API-owned adapter
 * can render flags, namespace lifecycle calls, and other compatibility
 * details without leaking them into generic request infrastructure.
 */
export function render_expression_generic_invocation(
  language: Expression_Invocation_Language,
  operation: Managed_Api_Operation,
  operation_constant: string,
  diagnostic: string,
  expected_result_kinds?: string,
): string | undefined {
  const invocation = operation.plan.invocation
  if (invocation.request === "compatibility") return undefined
  const generic_request = invocation.request === "generic"
    ? render_generic_request_payload(language, operation, diagnostic)
    : undefined
  if (invocation.request === "generic" && generic_request === undefined) {
    throw new Error(
      `operation ${operation.name} generic request framing has no rendered payload`,
    )
  }
  const request_payload = generic_request ?? expression_empty_bytes(language)
  const require_result_kinds = (): string => {
    if (expected_result_kinds === undefined) {
      throw new Error(
        `${language} generic invocation requires expected result kinds`,
      )
    }
    return expected_result_kinds
  }
  switch (language) {
    case "java":
      return `smithyExecute(
              ${operation_constant},
              ${expression_empty_bytes(language)},
              ${request_payload},
              0,
              0)`
    case "kotlin":
      return `smithyInvoke(
              ${operation_constant},
              ${expression_empty_bytes(language)},
              ${request_payload},
          )`
    case "dart":
      return `_invoke(
      ${operation_constant},
      ${expression_empty_bytes(language)},
      ${request_payload},
    )`
    case "typescript": {
      const result_kinds = require_result_kinds()
      const value = generic_request === undefined
        ? ""
        : `      value: ${request_payload},\n`
      return `this.#transport.invoke(${operation_constant}, {
${value}      expected_kinds: [${result_kinds}],
    })`
    }
    case "python": {
      const result_kinds = require_result_kinds()
      const value = generic_request === undefined
        ? ""
        : `            value=${request_payload},\n`
      return `self._smithy_transport.invoke(
            ${operation_constant},
${value}            expected_kinds=(${result_kinds},),
        )`
    }
    case "swift":
      return `smithyInvoke(
      ${operation_constant},
      value: ${request_payload}
    )`
    case "csharp":
      return `RequestAsync(
            ${operation_constant},
            ${expression_empty_bytes(language)},
            ${request_payload},
            cancellationToken: cancellationToken)`
  }
}

/**
 * Rust's client core exposes a typed field-sequence executor rather than the
 * expression-language scoped transport call. It still consumes the same
 * invocation plan, so only this small syntax adapter needs to differ.
 */
export function render_rust_generic_invocation(
  operation: Managed_Api_Operation,
  diagnostic: string,
): string | undefined {
  const invocation = operation.plan.invocation
  if (invocation.request === "compatibility") return undefined
  const request_payload = invocation.request === "generic"
    ? render_generic_request_payload("rust", operation, diagnostic)
    : "Vec::new()"
  if (request_payload === undefined) {
    throw new Error(
      `operation ${operation.name} generic request framing has no rendered payload`,
    )
  }
  if (operation_uses_generic_field_sequence_request(operation)) {
    return `$client::execute_fields(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    ${request_payload},
                )
                    .await?`
  }
  return `$client::execute_unary(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    ${request_payload},
                )
                    .await?`
}

export function render_go_field_sequence_request(
  operation: Managed_Api_Operation,
  diagnostic: string,
): { readonly statements: string; readonly payload: string } {
  const values: string[] = []
  const statements: string[] = []
  operation_field_sequence_fields(operation, "input").forEach((field, index) => {
    const input = operation_field_path_expression(operation, field, "go")
    const codec_input = field.required ? input : `*${input}`
    const rendered = operation_field_sequence_codec(operation, field).render(
      "go",
      codec_input,
      "field",
      diagnostic,
      go_api_name(operation.output),
      `Field${index}`,
    )
    const value_name = `fieldValue${index}`
    let encoded: string
    if (field.required) {
      encoded = rendered.encode.split("wireValue").join(value_name)
    } else {
      // Optional Go members are pointers. Keep the missing sentinel semantics
      // in the field-sequence encoder and only run the codec when present.
      // Preserve the codec's local `wireValue` declaration, then assign its
      // result to the outer optional slot. Rewriting `:=` would either shadow
      // that slot or break codecs that also declare an `err` result.
      encoded = `var ${value_name} []byte
\t\tif ${input} != nil {
\t\t\t${rendered.encode.split("\n").join("\n\t\t\t")}
\t\t\t${value_name} = wireValue
\t\t}`
    }
    statements.push(encoded)
    values.push(value_name)
  })
  const encoder = operation_uses_dense_layout(operation, "input")
    ? `smithyEncodeDenseFields(${values.join(", ")})`
    : `smithyEncodeFieldSequence(${values.join(", ")})`
    statements.push(`fieldSequencePayload, err := ${encoder}
		if err != nil {
			return ${go_api_name(operation.output)}{}, err
		}`)
  return {
    statements: statements.join("\n\t\t"),
    payload: "fieldSequencePayload",
  }
}

/**
 * Renders the transport-neutral Go invocation for one operation.
 *
 * Go has a statement-producing codec boundary, so it cannot share the
 * expression-language helper verbatim. It still follows the same plan:
 * generic operations invoke the opcode with a descriptor-built body, while
 * compatibility operations return `undefined` and let their adapter render
 * the historical scoped call. Keeping this in one helper prevents raw,
 * opaque, and field-sequence projections from independently deciding that a
 * generic request has an empty body.
 */
export function render_go_generic_invocation(
  operation: Managed_Api_Operation,
  diagnostic: string,
  output: string,
  output_field?: string,
): { readonly statements: string; readonly expression: string } | undefined {
  if (operation.plan.invocation.request === "compatibility") return undefined

  const request = (() => {
    switch (operation.plan.request.request_framing) {
      case "empty":
        return { statements: "", payload: "nil" }
      case "opaque": {
        const codecs = operation.plan.application_value_codecs
        if (codecs === undefined) {
          throw new Error(
            `operation ${operation.name} opaque request has no application-value codec plan`,
          )
        }
        const input = operation_opaque_field_name(operation, "input", "go")
        const encoded = render_application_value_codec(
          "go",
          codecs,
          `input.${input}`,
          "result.data",
          diagnostic,
          output,
          output_field,
        ).encode
        return { statements: encoded, payload: "wireValue" }
      }
      case "ordered_fields":
        return render_go_field_sequence_request(operation, diagnostic)
    }
  })()

  return {
    statements: request.statements,
    expression: `s.client.invoke(
			ctx,
			${`SmithyOpcode${operation.name}`},
			nil,
			${request.payload},
			SetOptions{},
		)`,
  }
}

function structure_field_name_for(
  contract: Client_Contract,
  structure_name: string,
  field: Operation_Field_Role,
  language: "csharp" | "dart" | "go" | "java" | "kotlin" | "python" | "rust" | "swift" | "typescript",
): string {
  const structure = contract.api.structures.find(
    (candidate) => candidate.name === structure_name,
  )
  const member = structure?.members.find(
    (candidate) => candidate.operation_field_role === field,
  )
  if (member !== undefined) return operation_field_name(member, language)
  // These names belong to shared convenience helpers (SET/policy flag
  // adapters), not to a particular modeled operation.  They must remain
  // renderable when a contract contains only a generic operation or when a
  // future API does not expose the legacy SET shapes.  Operation-specific
  // required roles are validated through the canonical operation plan and
  // `operation_field_name_for`; helper defaults are never used to bind a
  // modeled field silently.
  const fallback: Api_Member = {
    name: lower_camel_case(field),
    required: false,
    type: { kind: "string" },
  }
  return operation_field_name(fallback, language)
}

function operation_structure_field_name_for(
  contract: Client_Contract,
  operation: Managed_Api_Operation,
  direction: "input" | "output",
  parent: Operation_Field_Role,
  field: Operation_Field_Role,
  language: "csharp" | "dart" | "go" | "java" | "kotlin" | "python" | "rust" | "swift" | "typescript",
): string {
  const member = operation.plan.binding[direction][parent]?.[0]
  if (member?.type.kind === "structure" && member.type.name !== undefined) {
    return structure_field_name_for(contract, member.type.name, field, language)
  }
  return operation_field_name_for(operation, direction, field, language)
}

export function managed_operation_constant(
  operation: Api_Operation,
  language: "java" | "kotlin" | "dart",
): string {
  const identifier = snake_case(operation.name).toUpperCase()
  if (language === "dart") {
    return `smithyOperation${pascal_case(snake_case(operation.name))}`
  }
  return `SmithyContract.OPERATION_${identifier}`
}

export function managed_operation_label(operation: Api_Operation): string {
  return snake_case(operation.name).toUpperCase()
}

type Operation_Render_Language =
  | "csharp"
  | "dart"
  | "go"
  | "java"
  | "kotlin"
  | "python"
  | "rust"
  | "swift"
  | "typescript"

type Operation_Result_Renderer<T> = () => T

/**
 * Dispatches one language renderer through the shared result projection plan.
 *
 * The projection set and its error handling live here; language renderers only
 * provide syntax-specific callbacks. This keeps adding a transport-neutral
 * projection from requiring nine copies of the same switch scaffolding.
 */
export function render_operation_result<T>(
  operation: Managed_Api_Operation,
  language: string,
  renderers: Partial<
    Record<
      Operation_Response_Projection_Kind,
      Operation_Result_Renderer<T>
    >
  >,
): T {
  const projection = managed_result_projection(operation).projection
  const renderer = renderers[projection]
  if (renderer === undefined) {
    throw new Error(`unsupported generated ${language} response projection ${projection}`)
  }
  return renderer()
}

/**
 * Renders one native result discriminator for the target language.
 *
 * The semantic name is validated against the shared operation result plan
 * before it is converted to language syntax. This keeps result acceptance in
 * one model table while leaving only identifier spelling to each renderer.
 */
export function operation_result_constant(
  operation: Managed_Api_Operation,
  kind: Operation_Result_Kind,
  language: Operation_Render_Language,
): string {
  const result_projection = managed_result_projection(operation)
  const resolved_kind = result_projection.result_kinds.includes(kind)
    ? kind
    : result_projection.result_kinds.length === 1 &&
        result_projection.result_kinds[0] === "raw"
    ? "raw"
    : undefined
  if (resolved_kind === undefined) {
    throw new Error(
      `operation ${operation.name} result plan does not accept native result kind ${kind}`,
    )
  }
  const suffix = pascal_case(resolved_kind)
  switch (language) {
    case "csharp":
      return `Protocol.FfiResult${suffix}`
    case "dart":
      return `smithyResult${suffix}`
    case "go":
      return `SmithyFFIResult${resolved_kind === "ok" ? "OK" : suffix}`
    case "java":
    case "kotlin":
      return `SmithyContract.RESULT_${resolved_kind.toUpperCase()}`
    case "python":
    case "typescript":
      return `SMITHY_FFI_RESULT_${resolved_kind.toUpperCase()}`
    case "rust":
      return `openkache_client_core::contract::FFI_RESULT_${resolved_kind.toUpperCase()}`
    case "swift":
      return `Smithy_Native_Contract.result${suffix}`
  }
}

/** Resolves the first discriminator declared by an empty response projection. */
export function operation_empty_result_constant(
  operation: Managed_Api_Operation,
  language: Operation_Render_Language,
): string {
  const kind = managed_result_projection(operation).result_kinds[0] ?? "raw"
  return operation_result_constant(operation, kind, language)
}

export function operation_request_value_count(
  operation: Managed_Api_Operation,
): number {
  return operation.plan.operation.input.fields.filter((field) => field.role === "value").length
}

export function operation_request_value_name(
  operation: Managed_Api_Operation,
  language: Operation_Render_Language,
): string | undefined {
  if (operation_request_value_count(operation) === 0) return undefined
  const values = operation.plan.binding.input.value ?? []
  if (values.length !== 1) {
    // Generic field-sequence requests may intentionally carry repeated values
    // (batch mutation/CAS). They are encoded from the canonical ordered plan,
    // so a compact single-value ABI name is neither required nor meaningful.
    if (operation_is_global_field_sequence(operation)) {
      return undefined
    }
    throw new Error(
      `operation ${operation.name} request value role must bind exactly one member`,
    )
  }
  return operation_field_name(values[0]!, language)
}

interface Operation_Convenience_Fields {
  readonly input_condition: string
  readonly input_create_if_missing: string
  readonly input_eviction_mode: string
  readonly input_expected_revision: string
  readonly input_expiration_mode: string
  readonly input_item_id: string
  readonly input_name: string
  readonly input_namespace_id: string
  readonly input_policy: string
  readonly input_ttl_milliseconds: string
  readonly input_value: string
  readonly output_created: string
  readonly output_deleted: string
  readonly output_descriptor: string
  readonly output_json: string
  readonly output_outcome: string
  readonly output_value: string
}

/**
 * Resolves the legacy built-in convenience fields once per operation.
 *
 * Generic field-sequence operations do not consume this projection; their
 * ordered plan is rendered directly. Keeping this map separate prevents a
 * new role from requiring edits in every language renderer.
 */
export function operation_convenience_fields(
  operation: Managed_Api_Operation,
  language: Operation_Render_Language,
): Operation_Convenience_Fields {
  const input = (field: Operation_Field_Role): string =>
    operation_field_name_for(operation, "input", field, language)
  const output = (field: Operation_Field_Role): string =>
    operation_field_name_for(operation, "output", field, language)
  return {
    input_condition: input("condition"),
    input_create_if_missing: input("create_if_missing"),
    input_eviction_mode: input("eviction_mode"),
    input_expected_revision: input("expected_revision"),
    input_expiration_mode: input("expiration_mode"),
    input_item_id: input("item_id"),
    input_name: input("name"),
    input_namespace_id: input("namespace_id"),
    input_policy: input("policy"),
    input_ttl_milliseconds: input("ttl_milliseconds"),
    input_value: input("value"),
    output_created: output("created"),
    output_deleted: output("deleted"),
    output_descriptor: output("descriptor"),
    output_json: output("json"),
    output_outcome: output("outcome"),
    output_value: output("value"),
  }
}

interface Operation_Policy_Fields {
  readonly policy_default_eviction: string
  readonly policy_default_expiration: string
  readonly policy_default_ttl_milliseconds: string
  readonly policy_eviction_override: string
  readonly policy_expiration_override: string
}

interface Operation_Structure_Convenience_Fields extends Operation_Policy_Fields {
  readonly set_condition: string
  readonly set_eviction_mode: string
  readonly set_expiration_mode: string
  readonly set_ttl_milliseconds: string
  readonly set_value: string
}

/**
 * Resolves the remaining compact-v1 structure names once per language.
 *
 * These names are used only by the legacy SET and namespace-policy adapters.
 * Generic operations never depend on `SetInput` or `NamespacePolicy`; keeping
 * this projection here prevents their member naming rules from leaking into
 * the generic operation renderers.
 */
export function structure_convenience_fields(
  contract: Client_Contract,
  language: Operation_Render_Language,
): Operation_Structure_Convenience_Fields {
  const set = (field: Operation_Field_Role): string =>
    structure_field_name_for(contract, "SetInput", field, language)
  const policy = (field: Operation_Field_Role): string =>
    structure_field_name_for(contract, "NamespacePolicy", field, language)
  return {
    set_condition: set("condition"),
    set_expiration_mode: set("expiration_mode"),
    set_ttl_milliseconds: set("ttl_milliseconds"),
    set_eviction_mode: set("eviction_mode"),
    set_value: set("value"),
    policy_default_expiration: policy("default_expiration"),
    policy_default_ttl_milliseconds: policy("default_ttl_milliseconds"),
    policy_expiration_override: policy("expiration_override"),
    policy_default_eviction: policy("default_eviction"),
    policy_eviction_override: policy("eviction_override"),
  }
}

/** Resolves policy leaves for the operation's modeled policy structure. */
export function operation_policy_fields(
  contract: Client_Contract,
  operation: Managed_Api_Operation,
  language: Operation_Render_Language,
): Operation_Policy_Fields {
  const policy = (field: Operation_Field_Role): string =>
    operation_structure_field_name_for(
      contract,
      operation,
      "input",
      "policy",
      field,
      language,
    )
  return {
    policy_default_expiration: policy("default_expiration"),
    policy_default_ttl_milliseconds: policy("default_ttl_milliseconds"),
    policy_expiration_override: policy("expiration_override"),
    policy_default_eviction: policy("default_eviction"),
    policy_eviction_override: policy("eviction_override"),
  }
}
