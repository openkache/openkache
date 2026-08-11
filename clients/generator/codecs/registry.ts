/** Shared operation plans, codecs, and framing used by language renderers. */

import {
  WIRE_CODEC_NAMES,
} from "../../../protocol/wire"
import type {
  Api_Operation,
  Api_Operation_Contract,
  Api_Type,
} from "../../operation_models"
import { typescript_name } from "../../generator_names"
import type { Operation_Field_Plan } from "../../operation_plans"
import { typescript_api_name } from "../../api_shape_renderers"

import {
  go_api_name,
  go_api_type,
  python_api_name,
} from "./contract"
import type { Rendered_Go_Composite_Field } from "./framing"

export type Application_Value_Codec = string

export interface Application_Value_Codec_Pair {
  /**
   * Request and response framing are independent. An operation may carry an
   * opaque request with a status-only response, or an empty request with an
   * opaque response, so either side can legitimately have no codec.
   */
  readonly input: Application_Value_Codec
  readonly input_type: Api_Type
  readonly output: Application_Value_Codec
  readonly output_type: Api_Type
}

export type Application_Value_Language =
  | "java"
  | "kotlin"
  | "dart"
  | "typescript"
  | "go"
  | "python"
  | "swift"
  | "csharp"
  | "rust"

interface Rendered_Application_Value_Codec {
  readonly encode: string
  readonly decode: string
}

/** One codec registration shared by every generated language renderer. */
export interface Wire_Codec_Registration {
  readonly name: string
  readonly matches: (type: Api_Type) => boolean
  /**
   * Language-specific syntax is owned by the codec registration itself.
   * Operation renderers only ask the registry for encode/decode expressions.
   */
  readonly render: (
    language: Application_Value_Language,
    input: string,
    payload: string,
    diagnostic: string,
    output?: string,
    output_field?: string,
    type?: Api_Type,
  ) => Rendered_Application_Value_Codec
  readonly render_optional: (
    language: Application_Value_Language,
    payload: string,
    diagnostic: string,
    type?: Api_Type,
  ) => string
  readonly render_go_optional: (
    payload: string,
    decoded: string,
    diagnostic: string,
    output: string,
    type?: Api_Type,
  ) => Rendered_Go_Composite_Field
}

/**
 * Application payload codecs are registered by wire shape, not operation.
 * Adding a new representation requires one registration and one set of
 * language helper templates, never another operation-name branch.
 */
export const WIRE_CODEC_REGISTRY: readonly Wire_Codec_Registration[] = [
  {
    name: "utf8",
    matches: (type) => type.kind === "string",
    render: render_utf8_codec,
    render_optional: render_optional_utf8_codec,
    render_go_optional: render_go_optional_utf8_codec,
  },
  {
    name: "packed_f64_be",
    matches: (type) => type.kind === "list" && type.member?.kind === "double",
    render: render_f64_array_codec,
    render_optional: render_optional_f64_array_codec,
    render_go_optional: render_go_optional_f64_array_codec,
  },
  {
    name: "u64_be",
    matches: (type) => type.kind === "long" || type.kind === "unsigned_long",
    render: render_u64_codec,
    render_optional: render_optional_u64_codec,
    render_go_optional: render_go_optional_u64_codec,
  },
  {
    name: "bool_u8",
    matches: (type) => type.kind === "boolean",
    render: render_bool_codec,
    render_optional: render_optional_bool_codec,
    render_go_optional: render_go_optional_bool_codec,
  },
  {
    name: "f64_be",
    matches: (type) => type.kind === "double",
    render: render_f64_codec,
    render_optional: render_optional_f64_codec,
    render_go_optional: render_go_optional_f64_codec,
  },
  {
    name: "i32_be",
    matches: (type) => type.kind === "integer",
    render: render_i32_codec,
    render_optional: render_optional_i32_codec,
    render_go_optional: render_go_optional_i32_codec,
  },
  {
    name: "raw_bytes",
    matches: (type) => type.kind === "blob",
    render: render_raw_bytes_codec,
    render_optional: render_optional_raw_bytes_codec,
    render_go_optional: render_go_optional_raw_bytes_codec,
  },
  {
    name: "enum",
    matches: (type) => type.kind === "enum",
    render: render_enum_codec,
    render_optional: render_optional_enum_codec,
    render_go_optional: render_go_optional_enum_codec,
  },
  {
    name: "list",
    matches: (type) => type.kind === "list",
    render: render_list_codec,
    render_optional: render_optional_list_codec,
    render_go_optional: render_go_optional_list_codec,
  },
  {
    name: "map",
    matches: (type) => type.kind === "map",
    render: render_map_codec,
    render_optional: render_optional_map_codec,
    render_go_optional: render_go_optional_map_codec,
  },
  {
    name: "union",
    matches: (type) => type.kind === "union",
    render: render_union_codec,
    render_optional: render_optional_union_codec,
    render_go_optional: render_go_optional_union_codec,
  },
]

const registered_wire_codec_names = WIRE_CODEC_REGISTRY.map(
  (registration) => registration.name,
)
const duplicate_wire_codec_renderers = registered_wire_codec_names.filter(
  (name, index) => registered_wire_codec_names.indexOf(name) !== index,
)
const missing_wire_codec_renderers = WIRE_CODEC_NAMES.filter(
  (name) => !registered_wire_codec_names.includes(name),
)
const extra_wire_codec_renderers = registered_wire_codec_names.filter(
  (name) => !WIRE_CODEC_NAMES.some((candidate) => candidate === name),
)
if (
  duplicate_wire_codec_renderers.length > 0 ||
  missing_wire_codec_renderers.length > 0 ||
  extra_wire_codec_renderers.length > 0
) {
  throw new Error(
    [
      "wire codec registry must exactly match the canonical codec names",
      duplicate_wire_codec_renderers.length > 0
        ? `duplicates: ${[...new Set(duplicate_wire_codec_renderers)].join(", ")}`
        : undefined,
      missing_wire_codec_renderers.length > 0
        ? `missing: ${missing_wire_codec_renderers.join(", ")}`
        : undefined,
      extra_wire_codec_renderers.length > 0
        ? `extra: ${extra_wire_codec_renderers.join(", ")}`
        : undefined,
    ]
      .filter((message): message is string => message !== undefined)
      .join("; "),
  )
}

/**
 * Renders the language-specific edge of a registered payload codec.
 *
 * Operation renderers only provide the input/output expressions and consume
 * this pair. Codec selection therefore remains a single registry decision,
 * while the unavoidable ABI syntax for each language lives in one place.
 */
function render_utf8_codec(
  language: Application_Value_Language,
  input: string,
  payload: string,
  diagnostic: string,
  output?: string,
  output_field?: string,
): Rendered_Application_Value_Codec {
  switch (language) {
    case "java":
      return {
        encode: `${input}.getBytes(StandardCharsets.UTF_8)`,
        decode: `smithyDecodeUtf8(${payload}, ${diagnostic})`,
      }
    case "kotlin":
      return {
        encode: `${input}.toByteArray()`,
        decode: `smithyDecodeUtf8(${payload}, ${diagnostic})`,
      }
    case "dart":
      return {
        encode: `utf8.encode(${input})`,
        decode: `_smithyDecodeUtf8(${payload}, ${diagnostic})`,
      }
    case "typescript":
      return {
        encode: `new TextEncoder().encode(${input})`,
        decode: `this.#transport.decode_utf8(${payload}, ${diagnostic})`,
      }
    case "go":
      return {
        encode: `wireValue := []byte(${input})`,
        decode: `return ${output ?? "struct{}"}{${output_field ?? "Payload"}: string(${payload})}, nil`,
      }
    case "python":
      return {
        encode: `${input}.encode("utf-8")`,
        decode: `self._smithy_transport.decode_utf8(${payload}, ${diagnostic})`,
      }
    case "swift":
      return {
        encode: `Data(${input}.utf8)`,
        decode: `try { () throws -> String in
      guard let value = String(data: ${payload}, encoding: .utf8) else {
      throw OpenKacheError(${diagnostic} + " response is not valid UTF-8")
      }
      return value
    }()`,
      }
    case "csharp":
      return {
        encode: `ValidateValue(Encoding.UTF8.GetBytes(${input}))`,
        decode: `new UTF8Encoding(false, true).GetString(${payload})`,
      }
    case "rust":
      return {
        encode: `${input}.into_bytes()`,
        decode: `String::from_utf8(${payload}).map_err(|error| {
                    Error::Protocol(format!("{} response is not UTF-8: {error}", ${diagnostic}))
                })?`,
      }
  }
}

function render_raw_bytes_codec(
  language: Application_Value_Language,
  input: string,
  payload: string,
  _diagnostic: string,
  output?: string,
  output_field?: string,
): Rendered_Application_Value_Codec {
  switch (language) {
    case "java":
    case "kotlin":
    case "dart":
    case "typescript":
    case "python":
      return { encode: input, decode: payload }
    case "go":
      return {
        encode: `wireValue := ${input}`,
        decode: `return ${output ?? "struct{}"}{${output_field ?? "Payload"}: ${payload}}, nil`,
      }
    case "swift":
      return { encode: input, decode: payload }
    case "csharp":
      return { encode: `ValidateValue(${input})`, decode: payload }
    case "rust":
      return { encode: input, decode: payload }
  }
}

function wire_codec_for_type(type: Api_Type): Wire_Codec_Registration {
  const declared = type.wire_codec
  const registration = declared === undefined
    ? WIRE_CODEC_REGISTRY.find((candidate) => candidate.matches(type))
    : WIRE_CODEC_REGISTRY.find((candidate) => candidate.name === declared)
  if (registration === undefined || !registration.matches(type)) {
    throw new Error(
      `unsupported nested wire codec ${JSON.stringify(declared ?? type.kind)}`,
    )
  }
  return registration
}

/**
 * Resolves one operation field through the canonical protocol plan.
 *
 * The Smithy type remains the language projection, but the wire codec belongs
 * to the shared operation descriptor. When both are present, require them to
 * agree so a future generator change cannot silently create a second codec
 * decision path.
 */
export function wire_codec_for_field(
  operation: Api_Operation & { readonly contract: Api_Operation_Contract },
  field: Operation_Field_Plan,
): Wire_Codec_Registration {
  const declared = field.codecs?.[0]
  if (declared === undefined) {
    return wire_codec_for_type(field.type)
  }
  const registration = WIRE_CODEC_REGISTRY.find(
    (candidate) => candidate.name === declared,
  )
  if (registration === undefined) {
    throw new Error(
      `operation ${operation.name} field ${field.path.join(".")} names unsupported wire codec ${JSON.stringify(declared)}`,
    )
  }
  if (!registration.matches(field.type)) {
    throw new Error(
      `operation ${operation.name} field ${field.path.join(".")} codec ${JSON.stringify(declared)} does not match its Smithy type ${field.type.kind}`,
    )
  }
  return registration
}

function render_nested_codec(
  language: Application_Value_Language,
  type: Api_Type,
  input: string,
  payload: string,
  diagnostic: string,
): Rendered_Application_Value_Codec {
  return wire_codec_for_type(type).render(
    language,
    input,
    payload,
    diagnostic,
    undefined,
    undefined,
    type,
  )
}

function enum_type_name(
  language: Application_Value_Language,
  type: Api_Type,
): string {
  if (type.name === undefined) throw new Error("enum codec has no shape name")
  switch (language) {
    case "typescript":
      return typescript_api_name(type.name)
    case "python":
      return python_api_name(type.name)
    case "swift":
      return `Smithy_${typescript_name(type.name)}`
    case "go":
      return go_api_name(type.name)
    default:
      return type.name
  }
}

function render_enum_codec(
  language: Application_Value_Language,
  input: string,
  payload: string,
  diagnostic: string,
  _output?: string,
  _output_field?: string,
  type?: Api_Type,
): Rendered_Application_Value_Codec {
  if (type?.kind !== "enum") {
    return render_raw_bytes_codec(language, input, payload, diagnostic, _output, _output_field)
  }
  const name = enum_type_name(language, type)
  switch (language) {
    case "java":
      return {
        encode: `${input}.smithyValue().getBytes(StandardCharsets.UTF_8)`,
        decode: `${name}.fromSmithyValue(smithyDecodeUtf8(${payload}, ${diagnostic}))`,
      }
    case "kotlin":
      return {
        encode: `${input}.smithyValue.toByteArray()`,
        decode: `${name}.fromSmithyValue(smithyDecodeUtf8(${payload}, ${diagnostic}))`,
      }
    case "dart":
      return {
        encode: `utf8.encode(${input}.smithyValue)`,
        decode: `${name}.fromSmithyValue(_smithyDecodeUtf8(${payload}, ${diagnostic}))`,
      }
    case "typescript":
      return {
        encode: `new TextEncoder().encode(${input})`,
        decode: `smithy_decode_enum(${payload}, ${JSON.stringify(type.enum_values ?? [])}, ${diagnostic}) as ${name}`,
      }
    case "go":
      return {
        encode: `wireValue, err := smithyEncodeEnum(string(${input}), []string{${(type.enum_values ?? []).map((value) => JSON.stringify(value)).join(", ")}})
\tif err != nil {
\t\treturn ${_output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}`,
        decode: `value, err := smithyDecodeEnum(${payload}, []string{${(type.enum_values ?? []).map((value) => JSON.stringify(value)).join(", ")}})
\tif err != nil {
\t\treturn ${_output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}
\treturn ${_output ?? "struct{}"}{${_output_field ?? "Payload"}: ${name}(value)}, nil`,
      }
    case "python":
      return {
        encode: `${input}.value.encode("utf-8")`,
        decode: `${name}(${payload}.decode("utf-8"))`,
      }
    case "swift":
      return {
        encode: `Data(${input}.rawValue.utf8)`,
        decode: `try smithyDecodeEnum(${payload}, ${name}.self, ${diagnostic})`,
      }
    case "csharp":
      return {
        encode: `Encoding.UTF8.GetBytes(Smithy.Smithy${name}Wire.ToValue(${input}))`,
        decode: `Smithy.Smithy${name}Wire.FromValue(Encoding.UTF8.GetString(${payload}))`,
      }
    case "rust":
      return {
        encode: `${input}.smithy_value().as_bytes().to_vec()`,
        decode: `${name}::from_smithy_value(std::str::from_utf8(&${payload}).map_err(|error| {
                    Error::Protocol(format!("{} response is not UTF-8: {error}", ${diagnostic}))
                })?).ok_or_else(|| Error::Protocol(format!(
                    "{} response contains an unknown enum value", ${diagnostic})))?`,
      }
  }
}

function render_optional_enum_codec(
  language: Application_Value_Language,
  payload: string,
  diagnostic: string,
  type?: Api_Type,
): string {
  if (type?.kind !== "enum") {
    return render_optional_raw_bytes_codec(language, payload, diagnostic)
  }
  const rendered = render_enum_codec(
    language,
    "value",
    payload,
    diagnostic,
    undefined,
    undefined,
    type,
  ).decode
  switch (language) {
    case "java":
      return `(${payload} == null ? null : ${rendered})`
    case "kotlin":
      return `${payload}?.let { ${render_enum_codec("kotlin", "value", "it", diagnostic, undefined, undefined, type).decode} }`
    case "dart":
      return `${payload} == null ? null : ${render_enum_codec("dart", "value", `${payload}!`, diagnostic, undefined, undefined, type).decode}`
    case "typescript":
      return `${payload} === undefined ? undefined : ${render_enum_codec("typescript", "value", `${payload}!`, diagnostic, undefined, undefined, type).decode}`
    case "go":
      return `smithyDecodeOptionalEnum(${payload}, ${JSON.stringify(type?.name ?? "enum")})`
    case "python":
      return `${payload} if ${payload} is None else ${rendered}`
    case "swift":
      return `try ${payload}.map { data in ${render_enum_codec("swift", "value", "data", diagnostic, undefined, undefined, type).decode} }`
    case "csharp":
      return `${payload} is null ? null : ${rendered}`
    case "rust":
      return `${payload}.map(|value| ${render_enum_codec("rust", "value", "value", diagnostic, undefined, undefined, type).decode}).transpose()?`
  }
}

function render_go_optional_enum_codec(
  payload: string,
  decoded: string,
  diagnostic: string,
  output: string,
  type?: Api_Type,
): Rendered_Go_Composite_Field {
  if (type?.kind !== "enum") {
    return render_go_optional_raw_bytes_codec(payload, decoded, diagnostic, output)
  }
  const values = type?.enum_values ?? []
  const api_type = go_api_type(type, true)
  return {
    expression: decoded,
    statements: `\t\tvar ${decoded} *${api_type}
\t\tif ${payload} != nil {
\t\t\tvalue, err := smithyDecodeEnum(*${payload}, []string{${values.map((value) => JSON.stringify(value)).join(", ")}})
\t\t\tif err != nil {
\t\t\t\treturn ${output}{}, operationError(${diagnostic}, err)
\t\t\t}
\t\t\tconverted := ${api_type}(value)
\t\t\t${decoded} = &converted
\t\t}`,
  }
}

function render_list_codec(
  language: Application_Value_Language,
  input: string,
  payload: string,
  diagnostic: string,
  output?: string,
  output_field?: string,
  type?: Api_Type,
): Rendered_Application_Value_Codec {
  if (type?.kind !== "list" || type.member === undefined) {
    return render_raw_bytes_codec(language, input, payload, diagnostic, output, output_field)
  }
  const member = type.member
  const nested_encode = (value: string): string =>
    render_nested_codec(language, member, value, "value", diagnostic).encode
  const nested_decode = (value: string): string =>
    render_nested_codec(language, member, "value", value, diagnostic).decode
  switch (language) {
    case "java":
      return {
        encode: `smithyEncodeList(${input}.stream().map(value -> ${nested_encode("value")}).toArray(byte[][]::new))`,
        decode: `java.util.Arrays.stream(smithyDecodeList(${payload}, ${diagnostic})).map(value -> ${nested_decode("value")}).toList()`,
      }
    case "kotlin":
      return {
        encode: `smithyEncodeList(${input}.map { value -> ${nested_encode("value")} })`,
        decode: `smithyDecodeList(${payload}, ${diagnostic}).map { value -> ${nested_decode("value")} }`,
      }
    case "dart":
      return {
        encode: `_smithyEncodeList(${input}.map((value) => ${nested_encode("value")}).toList())`,
        decode: `_smithyDecodeList(${payload}, ${diagnostic}).map((value) => ${nested_decode("value")}).toList()`,
      }
    case "typescript":
      return {
        encode: `smithy_encode_list(${input}.map((value) => ${nested_encode("value")}))`,
        decode: `smithy_decode_list(${payload}, ${diagnostic}).map((value) => ${nested_decode("value")})`,
      }
    case "python":
      return {
        encode: `_smithy_encode_list([${nested_encode("value")} for value in ${input}])`,
        decode: `[${nested_decode("value")} for value in _smithy_decode_list(${payload}, ${diagnostic})]`,
      }
    case "swift":
      return {
        encode: `try smithyEncodeList(${input}.map { value in ${nested_encode("value")} })`,
        decode: `try smithyDecodeList(${payload}, operation: ${diagnostic}).map { value in ${nested_decode("value")} }`,
      }
    case "csharp":
      return {
        encode: `EncodeList(${input}.Select(value => ${nested_encode("value")}).ToArray())`,
        decode: `DecodeList(${payload}, ${diagnostic}).Select(value => ${nested_decode("value")}).ToArray()`,
      }
    case "rust":
      return {
        encode: `smithy_encode_list(&${input}.iter().map(|value| -> std::result::Result<Vec<u8>, Error> { Ok(${nested_encode("value")}) }).collect::<std::result::Result<Vec<_>, _>>()?)?`,
        decode: `smithy_decode_list(&${payload}, ${diagnostic})?.into_iter().map(|value| -> std::result::Result<_, Error> { Ok(${nested_decode("value")}) }).collect::<std::result::Result<Vec<_>, _>>()?`,
      }
    case "go": {
      const nested = render_go_nested_encode(member, "value", diagnostic)
      return {
        encode: `wireValues := make([][]byte, len(${input}))
\tfor index, value := range ${input} {
\t\twireValues[index] = ${nested}
\t}
\twireValue, err := smithyEncodeList(wireValues)
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}`,
        decode: `values, err := smithyDecodeList(${payload})
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}
\tdecoded := make([]${go_api_type(member, true)}, len(values))
\tfor index, value := range values {
\t\tdecoded[index] = ${render_go_nested_decode(member, "value", diagnostic)}
\t}
\treturn ${output ?? "struct{}"}{${output_field ?? "Payload"}: decoded}, nil`,
      }
    }
  }
}

function render_optional_list_codec(
  language: Application_Value_Language,
  payload: string,
  diagnostic: string,
  type?: Api_Type,
): string {
  if (type?.kind !== "list") {
    return render_optional_raw_bytes_codec(language, payload, diagnostic)
  }
  const decoded = render_list_codec(
    language,
    "value",
    payload,
    diagnostic,
    undefined,
    undefined,
    type,
  ).decode
  switch (language) {
    case "java":
      return `(${payload} == null ? null : ${decoded})`
    case "kotlin":
      return `${payload}?.let { ${render_list_codec("kotlin", "value", "it", diagnostic, undefined, undefined, type).decode} }`
    case "dart":
      return `${payload} == null ? null : ${render_list_codec("dart", "value", `${payload}!`, diagnostic, undefined, undefined, type).decode}`
    case "typescript":
      return `${payload} === undefined ? undefined : ${render_list_codec("typescript", "value", `${payload}!`, diagnostic, undefined, undefined, type).decode}`
    case "go":
      return `smithyDecodeOptionalList(${payload})`
    case "python":
      return `${payload} if ${payload} is None else ${decoded}`
    case "swift":
      return `try ${payload}.map { data in ${render_list_codec("swift", "value", "data", diagnostic, undefined, undefined, type).decode} }`
    case "csharp":
      return `${payload} is null ? null : ${decoded}`
    case "rust":
      return `${payload}.map(|value| ${render_list_codec("rust", "value", "value", diagnostic, undefined, undefined, type).decode}).transpose()?`
  }
}

function render_go_optional_list_codec(
  payload: string,
  decoded: string,
  diagnostic: string,
  output: string,
  type?: Api_Type,
): Rendered_Go_Composite_Field {
  const member = type?.kind === "list" ? type.member : undefined
  if (member === undefined) {
    return render_go_optional_raw_bytes_codec(payload, decoded, diagnostic, output)
  }
  const member_type = go_api_type(member, true)
  const nested_decode = render_go_nested_decode(member, "value", diagnostic)
  return {
    expression: decoded,
    statements: `\t\tvar ${decoded} *[]${member_type}
\t\tif ${payload} != nil {
\t\t\trawValues, err := smithyDecodeList(*${payload})
\t\t\tif err != nil {
\t\t\t\treturn ${output}{}, operationError(${diagnostic}, err)
\t\t\t}
\t\t\tconverted := make([]${member_type}, len(rawValues))
\t\t\tfor index, value := range rawValues {
\t\t\t\tconverted[index] = ${nested_decode}
\t\t\t}
\t\t\t${decoded} = &converted
\t\t}`,
  }
}

function render_map_codec(
  language: Application_Value_Language,
  input: string,
  payload: string,
  diagnostic: string,
  output?: string,
  output_field?: string,
  type?: Api_Type,
): Rendered_Application_Value_Codec {
  if (type?.kind !== "map" || type.key === undefined || type.value === undefined) {
    return render_raw_bytes_codec(language, input, payload, diagnostic, output, output_field)
  }
  const key_encode = (value: string): string =>
    render_nested_codec(language, type.key!, value, "key", diagnostic).encode
  const value_encode = (value: string): string =>
    render_nested_codec(language, type.value!, value, "value", diagnostic).encode
  const key_decode = (value: string): string =>
    render_nested_codec(language, type.key!, "key", value, diagnostic).decode
  const value_decode = (value: string): string =>
    render_nested_codec(language, type.value!, "value", value, diagnostic).decode
  switch (language) {
    case "java":
      return {
        encode: `smithyEncodeMap(${input}.entrySet().stream().map(entry -> new byte[][] { ${key_encode("entry.getKey()")}, ${value_encode("entry.getValue()")} }).toArray(byte[][][]::new))`,
        decode: `smithyDecodeMap(${payload}, ${diagnostic}).stream().collect(java.util.stream.Collectors.toMap(entry -> ${key_decode("entry[0]")}, entry -> ${value_decode("entry[1]")}))`,
      }
    case "kotlin":
      return {
        encode: `smithyEncodeMap(${input}.entries.map { (key, value) -> listOf(${key_encode("key")}, ${value_encode("value")}) })`,
        decode: `smithyDecodeMap(${payload}, ${diagnostic}).associate { (key, value) -> ${key_decode("key")} to ${value_decode("value")} }`,
      }
    case "dart":
      return {
        encode: `_smithyEncodeMap(${input}.entries.map((entry) => <List<int>>[${key_encode("entry.key")}, ${value_encode("entry.value")}]).toList())`,
        decode: `Map.fromEntries(_smithyDecodeMap(${payload}, ${diagnostic}).map((entry) => MapEntry(${key_decode("entry[0]")}, ${value_decode("entry[1]")})))`,
      }
    case "typescript":
      return {
        encode: `smithy_encode_map([...${input}.entries()].map(([key, value]) => [${key_encode("key")}, ${value_encode("value")}]))`,
        decode: `new Map(smithy_decode_map(${payload}, ${diagnostic}).map(([key, value]) => [${key_decode("key")}, ${value_decode("value")}]))`,
      }
    case "python":
      return {
        encode: `_smithy_encode_map([(${key_encode("key")}, ${value_encode("value")}) for key, value in ${input}.items()])`,
        decode: `{${key_decode("key")}: ${value_decode("value")} for key, value in _smithy_decode_map(${payload}, ${diagnostic})}`,
      }
    case "swift":
      return {
        encode: `try smithyEncodeMap(${input}.map { key, value in (${key_encode("key")}, ${value_encode("value")}) })`,
        decode: `try Dictionary(uniqueKeysWithValues: smithyDecodeMap(${payload}, operation: ${diagnostic}).map { key, value in (${key_decode("key")}, ${value_decode("value")}) })`,
      }
    case "csharp":
      return {
        encode: `EncodeMap(${input}.Select(entry => new[] { ${key_encode("entry.Key")}, ${value_encode("entry.Value")} }).ToArray())`,
        decode: `DecodeMap(${payload}, ${diagnostic}).ToDictionary(entry => ${key_decode("entry.Key")}, entry => ${value_decode("entry.Value")})`,
      }
    case "rust":
      return {
        encode: `smithy_encode_map(&${input}.iter().map(|(key, value)| -> std::result::Result<(Vec<u8>, Vec<u8>), Error> { Ok((${key_encode("key")}, ${value_encode("value")})) }).collect::<std::result::Result<Vec<_>, _>>()?)?`,
        decode: `smithy_decode_map(&${payload}, ${diagnostic})?.into_iter().map(|(key, value)| -> std::result::Result<_, Error> { Ok((${key_decode("key")}, ${value_decode("value")})) }).collect::<std::result::Result<std::collections::BTreeMap<_, _>, _>>()?`,
      }
    case "go":
      return {
        encode: `wireEntries := make([][2][]byte, 0, len(${input}))
\tfor key, value := range ${input} {
\t\twireEntries = append(wireEntries, [2][]byte{${render_go_nested_encode(type.key!, "key", diagnostic)}, ${render_go_nested_encode(type.value!, "value", diagnostic)}})
\t}
\twireValue, err := smithyEncodeMap(wireEntries)
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}`,
        decode: `entries, err := smithyDecodeMap(${payload})
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}
\tdecoded := make(map[${go_api_type(type.key, true)}]${go_api_type(type.value, true)}, len(entries))
\tfor _, entry := range entries {
\t\tdecoded[${render_go_nested_decode(type.key, "entry[0]", diagnostic)}] = ${render_go_nested_decode(type.value, "entry[1]", diagnostic)}
\t}
\treturn ${output ?? "struct{}"}{${output_field ?? "Payload"}: decoded}, nil`,
      }
    }
  }

function render_optional_map_codec(
  language: Application_Value_Language,
  payload: string,
  diagnostic: string,
  type?: Api_Type,
): string {
  if (type?.kind !== "map") {
    return render_optional_raw_bytes_codec(language, payload, diagnostic)
  }
  const decoded = render_map_codec(
    language,
    "value",
    payload,
    diagnostic,
    undefined,
    undefined,
    type,
  ).decode
  switch (language) {
    case "java":
      return `(${payload} == null ? null : ${decoded})`
    case "kotlin":
      return `${payload}?.let { ${render_map_codec("kotlin", "value", "it", diagnostic, undefined, undefined, type).decode} }`
    case "dart":
      return `${payload} == null ? null : ${render_map_codec("dart", "value", `${payload}!`, diagnostic, undefined, undefined, type).decode}`
    case "typescript":
      return `${payload} === undefined ? undefined : ${render_map_codec("typescript", "value", `${payload}!`, diagnostic, undefined, undefined, type).decode}`
    case "go":
      return `smithyDecodeOptionalMap(${payload})`
    case "python":
      return `${payload} if ${payload} is None else ${decoded}`
    case "swift":
      return `try ${payload}.map { data in ${render_map_codec("swift", "value", "data", diagnostic, undefined, undefined, type).decode} }`
    case "csharp":
      return `${payload} is null ? null : ${decoded}`
    case "rust":
      return `${payload}.map(|value| ${render_map_codec("rust", "value", "value", diagnostic, undefined, undefined, type).decode}).transpose()?`
  }
}

function render_go_optional_map_codec(
  payload: string,
  decoded: string,
  diagnostic: string,
  output: string,
  type?: Api_Type,
): Rendered_Go_Composite_Field {
  const key = type?.kind === "map" ? type.key : undefined
  const value = type?.kind === "map" ? type.value : undefined
  if (key === undefined || value === undefined) {
    return render_go_optional_raw_bytes_codec(payload, decoded, diagnostic, output)
  }
  const key_type = go_api_type(key, true)
  const value_type = go_api_type(value, true)
  const key_decode = render_go_nested_decode(key, "entry[0]", diagnostic)
  const value_decode = render_go_nested_decode(value, "entry[1]", diagnostic)
  return {
    expression: decoded,
    statements: `\t\tvar ${decoded} *map[${key_type}]${value_type}
\t\tif ${payload} != nil {
\t\t\trawEntries, err := smithyDecodeMap(*${payload})
\t\t\tif err != nil {
\t\t\t\treturn ${output}{}, operationError(${diagnostic}, err)
\t\t\t}
\t\t\tconverted := make(map[${key_type}]${value_type}, len(rawEntries))
\t\t\tfor _, entry := range rawEntries {
\t\t\t\tconverted[${key_decode}] = ${value_decode}
\t\t\t}
\t\t\t${decoded} = &converted
\t\t}`,
  }
}

function render_union_codec(
  language: Application_Value_Language,
  input: string,
  payload: string,
  diagnostic: string,
  output?: string,
  output_field?: string,
  _type?: Api_Type,
): Rendered_Application_Value_Codec {
  switch (language) {
    case "java":
      return { encode: `smithyEncodeUnion(${input}, ${diagnostic})`, decode: `smithyDecodeUnion(${payload}, ${diagnostic})` }
    case "kotlin":
      return { encode: `smithyEncodeUnion(${input}, ${diagnostic})`, decode: `smithyDecodeUnion(${payload}, ${diagnostic})` }
    case "dart":
      return { encode: `_smithyEncodeUnion(${input}, ${diagnostic})`, decode: `_smithyDecodeUnion(${payload}, ${diagnostic})` }
    case "typescript":
      return { encode: `smithy_encode_union(${input}, ${diagnostic})`, decode: `smithy_decode_union(${payload}, ${diagnostic})` }
    case "python":
      return { encode: `_smithy_encode_union(${input}, ${diagnostic})`, decode: `_smithy_decode_union(${payload}, ${diagnostic})` }
    case "swift":
      return { encode: `try smithyEncodeUnion(${input}, operation: ${diagnostic})`, decode: `try smithyDecodeUnion(${payload}, operation: ${diagnostic})` }
    case "csharp":
      return { encode: `EncodeUnion(${input}, ${diagnostic})`, decode: `DecodeUnion(${payload}, ${diagnostic})` }
    case "rust":
      return { encode: `smithy_encode_union(&${input}, ${diagnostic})?`, decode: `smithy_decode_union(&${payload}, ${diagnostic})?` }
    case "go":
      return {
        encode: `wireValue, err := smithyEncodeUnion(${input})
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}`,
        decode: `value, err := smithyDecodeUnion(${payload})
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}
\treturn ${output ?? "struct{}"}{${output_field ?? "Payload"}: value}, nil`,
      }
  }
}

function render_optional_union_codec(
  language: Application_Value_Language,
  payload: string,
  diagnostic: string,
): string {
  switch (language) {
    case "java":
      return `(${payload} == null ? null : smithyDecodeUnion(${payload}, ${diagnostic}))`
    case "kotlin":
      return `${payload}?.let { smithyDecodeUnion(it, ${diagnostic}) }`
    case "dart":
      return `${payload} == null ? null : _smithyDecodeUnion(${payload}!, ${diagnostic})`
    case "typescript":
      return `${payload} === undefined ? undefined : smithy_decode_union(${payload}!, ${diagnostic})`
    case "python":
      return `${payload} if ${payload} is None else _smithy_decode_union(${payload}, ${diagnostic})`
    case "swift":
      return `try ${payload}.map { try smithyDecodeUnion($0, operation: ${diagnostic}) }`
    case "csharp":
      return `${payload} is null ? null : DecodeUnion(${payload}!, ${diagnostic})`
    case "rust":
      return `${payload}.map(|value| smithy_decode_union(&value, ${diagnostic})).transpose()?`
    case "go":
      return `smithyDecodeOptionalUnion(${payload})`
  }
}

function render_go_optional_union_codec(
  payload: string,
  decoded: string,
  diagnostic: string,
  output: string,
): Rendered_Go_Composite_Field {
  return {
    expression: decoded,
    statements: `\t\t${decoded}, err := smithyDecodeOptionalUnion(${payload})
\t\tif err != nil {
\t\t\treturn ${output}{}, operationError(${diagnostic}, err)
\t\t}`,
  }
}

/** Renders a byte-producing Go expression for a nested container member. */
function render_go_nested_encode(
  type: Api_Type,
  input: string,
  _diagnostic: string,
): string {
  const codec = wire_codec_for_type(type).name
  switch (codec) {
    case "utf8":
      return `[]byte(${input})`
    case "enum":
      return `func() []byte {
\t\tencoded, err := smithyEncodeEnum(string(${input}), []string{${(type.enum_values ?? []).map((value) => JSON.stringify(value)).join(", ")} })
\t\tif err != nil { panic(err) }
\t\treturn encoded
\t}()`
    case "raw_bytes":
      return input
    case "u64_be":
      return `smithyEncodeU64(${input})`
    case "i32_be":
      return `smithyEncodeI32(${input})`
    case "f64_be":
      return `smithyEncodeF64(${input})`
    case "bool_u8":
      return `smithyEncodeBool(${input})`
    case "packed_f64_be":
      return `func() []byte { encoded, err := smithyEncodeF64Array(${input}); if err != nil { panic(err) }; return encoded }()`
    case "union":
      return `func() []byte { encoded, err := smithyEncodeUnion(${input}); if err != nil { panic(err) }; return encoded }()`
    case "list":
      if (type.member === undefined) throw new Error("list codec requires list member metadata")
      return `func() []byte {
\t\tvalues := make([][]byte, len(${input}))
\t\tfor index, value := range ${input} {
\t\t\tvalues[index] = ${render_go_nested_encode(type.member, "value", _diagnostic)}
\t\t}
\t\tencoded, err := smithyEncodeList(values)
\t\tif err != nil { panic(err) }
\t\treturn encoded
\t}()`
    case "map":
      if (type.key === undefined || type.value === undefined) {
        throw new Error("map codec requires key/value metadata")
      }
      return `func() []byte {
\t\tentries := make([][2][]byte, 0, len(${input}))
\t\tfor key, value := range ${input} {
\t\t\tentries = append(entries, [2][]byte{
\t\t\t\t${render_go_nested_encode(type.key, "key", _diagnostic)},
\t\t\t\t${render_go_nested_encode(type.value, "value", _diagnostic)},
\t\t\t})
\t\t}
\t\tencoded, err := smithyEncodeMap(entries)
\t\tif err != nil { panic(err) }
\t\treturn encoded
\t}()`
    default:
      return input
  }
}

/** Renders a typed Go expression from one length-delimited member. */
function render_go_nested_decode(
  type: Api_Type,
  payload: string,
  _diagnostic: string,
): string {
  const codec = wire_codec_for_type(type).name
  switch (codec) {
    case "utf8":
      return `string(${payload})`
    case "raw_bytes":
      return payload
    case "u64_be":
      return `func() uint64 { value, err := smithyDecodeU64(${payload}); if err != nil { panic(err) }; return value }()`
    case "i32_be":
      return `func() int32 { value, err := smithyDecodeI32(${payload}); if err != nil { panic(err) }; return value }()`
    case "f64_be":
      return `func() float64 { value, err := smithyDecodeF64(${payload}); if err != nil { panic(err) }; return value }()`
    case "bool_u8":
      return `func() bool { value, err := smithyDecodeBool(${payload}); if err != nil { panic(err) }; return value }()`
    case "packed_f64_be":
      return `func() []float64 { value, err := smithyDecodeF64Array(${payload}); if err != nil { panic(err) }; return value }()`
    case "union":
      return `func() []byte { value, err := smithyDecodeUnion(${payload}); if err != nil { panic(err) }; return value }()`
    case "enum":
      return `func() ${go_api_type(type, true)} {
		value, err := smithyDecodeEnum(${payload}, []string{${(type.enum_values ?? []).map((value) => JSON.stringify(value)).join(", ")} })
		if err != nil { panic(err) }
		return ${go_api_type(type, true)}(value)
	}()`
    case "list": {
      if (type.member === undefined) return payload
      const member_type = go_api_type(type.member, true)
      const nested = render_go_nested_decode(type.member, "value", _diagnostic)
      return `func() ${go_api_type(type, true)} {
		rawValues, err := smithyDecodeList(${payload})
		if err != nil { panic(err) }
		decoded := make([]${member_type}, len(rawValues))
		for index, value := range rawValues {
			decoded[index] = ${nested}
		}
		return decoded
	}()`
    }
    case "map": {
      if (type.key === undefined || type.value === undefined) return payload
      const key_type = go_api_type(type.key, true)
      const value_type = go_api_type(type.value, true)
      const key_nested = render_go_nested_decode(type.key, "entry[0]", _diagnostic)
      const value_nested = render_go_nested_decode(type.value, "entry[1]", _diagnostic)
      return `func() ${go_api_type(type, true)} {
		rawEntries, err := smithyDecodeMap(${payload})
		if err != nil { panic(err) }
		decoded := make(map[${key_type}]${value_type}, len(rawEntries))
		for _, entry := range rawEntries {
			decoded[${key_nested}] = ${value_nested}
		}
		return decoded
	}()`
    }
    default:
      return payload
  }
}

function render_f64_array_codec(
  language: Application_Value_Language,
  input: string,
  payload: string,
  diagnostic: string,
  output?: string,
  output_field?: string,
): Rendered_Application_Value_Codec {
  switch (language) {
    case "java":
      return {
        encode: `smithyEncodeF64Array(${input})`,
        decode: `smithyDecodeF64Array(${payload}, ${diagnostic})`,
      }
    case "kotlin":
      return {
        encode: `smithyEncodeF64Array(${input})`,
        decode: `smithyDecodeF64Array(${payload}, ${diagnostic})`,
      }
    case "dart":
      return {
        encode: `_smithyEncodeF64Array(${input})`,
        decode: `_smithyDecodeF64Array(${payload}, ${diagnostic})`,
      }
    case "typescript":
      return {
        encode: `smithy_encode_f64_array(${input})`,
        decode: `smithy_decode_f64_array(${payload}, ${diagnostic})`,
      }
    case "go":
      return {
        encode: `wireValue, err := smithyEncodeF64Array(${input})
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, err
\t}`,
        decode: `values, err := smithyDecodeF64Array(${payload})
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}
\treturn ${output ?? "struct{}"}{${output_field ?? "Payload"}: values}, nil`,
      }
    case "python":
      return {
        encode: `_smithy_encode_f64_array(${input})`,
        decode: `_smithy_decode_f64_array(${payload}, ${diagnostic})`,
      }
    case "swift":
      return {
        encode: `try smithyEncodeF64Array(${input})`,
        decode: `try smithyDecodeF64Array(
      ${payload},
      operation: ${diagnostic}
    )`,
      }
    case "csharp":
      return {
        encode: `EncodeF64Array(${input})`,
        decode: `DecodeF64Array(${payload}, ${diagnostic})`,
      }
    case "rust":
      return {
        encode: `smithy_encode_f64_array(&${input})?`,
        decode: `smithy_decode_f64_array(&${payload}, ${diagnostic})?`,
      }
  }
}

function render_u64_codec(
  language: Application_Value_Language,
  input: string,
  payload: string,
  diagnostic: string,
  output?: string,
  output_field?: string,
): Rendered_Application_Value_Codec {
  switch (language) {
    case "java":
    case "kotlin":
      return {
        encode: `smithyEncodeU64(${input})`,
        decode: `smithyDecodeU64(${payload}, ${diagnostic})`,
      }
    case "dart":
      return {
        encode: `_smithyEncodeU64(${input})`,
        decode: `_smithyDecodeU64(${payload}, ${diagnostic})`,
      }
    case "typescript":
      return {
        encode: `smithy_encode_u64(${input})`,
        decode: `smithy_decode_u64(${payload}, ${diagnostic})`,
      }
    case "go":
      return {
        encode: `wireValue := smithyEncodeU64(${input})`,
        decode: `value, err := smithyDecodeU64(${payload})
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}
\treturn ${output ?? "struct{}"}{${output_field ?? "Payload"}: value}, nil`,
      }
    case "python":
      return {
        encode: `_smithy_encode_u64(${input})`,
        decode: `_smithy_decode_u64(${payload}, ${diagnostic})`,
      }
    case "swift":
      return {
        encode: `smithyEncodeU64(${input})`,
        decode: `try smithyDecodeU64(${payload}, operation: ${diagnostic})`,
      }
    case "csharp":
      return {
        encode: `EncodeU64(${input})`,
        decode: `DecodeU64(${payload}, ${diagnostic})`,
      }
    case "rust":
      return {
        encode: `${input}.to_be_bytes().to_vec()`,
        decode: `u64::from_be_bytes(${payload}.try_into().map_err(|_| {
                    Error::Protocol(format!("{} response has an invalid u64 field", ${diagnostic}))
                })?)`,
      }
  }
}

function render_bool_codec(
  language: Application_Value_Language,
  input: string,
  payload: string,
  diagnostic: string,
  output?: string,
  output_field?: string,
): Rendered_Application_Value_Codec {
  switch (language) {
    case "java":
    case "kotlin":
      return {
        encode: `smithyEncodeBool(${input})`,
        decode: `smithyDecodeBool(${payload}, ${diagnostic})`,
      }
    case "dart":
      return {
        encode: `_smithyEncodeBool(${input})`,
        decode: `_smithyDecodeBool(${payload}, ${diagnostic})`,
      }
    case "typescript":
      return {
        encode: `smithy_encode_bool(${input})`,
        decode: `smithy_decode_bool(${payload}, ${diagnostic})`,
      }
    case "go":
      return {
        encode: `wireValue := smithyEncodeBool(${input})`,
        decode: `value, err := smithyDecodeBool(${payload})
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}
\treturn ${output ?? "struct{}"}{${output_field ?? "Payload"}: value}, nil`,
      }
    case "python":
      return {
        encode: `_smithy_encode_bool(${input})`,
        decode: `_smithy_decode_bool(${payload}, ${diagnostic})`,
      }
    case "swift":
      return {
        encode: `smithyEncodeBool(${input})`,
        decode: `try smithyDecodeBool(${payload}, operation: ${diagnostic})`,
      }
    case "csharp":
      return {
        encode: `EncodeBool(${input})`,
        decode: `DecodeBool(${payload}, ${diagnostic})`,
      }
    case "rust":
      return {
        encode: `vec![if ${input} { 1 } else { 0 }]`,
        decode: `match ${payload}.as_slice() {
                    [0] => false,
                    [1] => true,
                    _ => return Err(Error::Protocol(format!(
                        "{} response has an invalid boolean field", ${diagnostic}))),
                }`,
      }
  }
}

function render_f64_codec(
  language: Application_Value_Language,
  input: string,
  payload: string,
  diagnostic: string,
  output?: string,
  output_field?: string,
): Rendered_Application_Value_Codec {
  switch (language) {
    case "java":
    case "kotlin":
      return {
        encode: `smithyEncodeF64(${input})`,
        decode: `smithyDecodeF64(${payload}, ${diagnostic})`,
      }
    case "dart":
      return {
        encode: `_smithyEncodeF64(${input})`,
        decode: `_smithyDecodeF64(${payload}, ${diagnostic})`,
      }
    case "typescript":
      return {
        encode: `smithy_encode_f64(${input})`,
        decode: `smithy_decode_f64(${payload}, ${diagnostic})`,
      }
    case "go":
      return {
        encode: `wireValue := smithyEncodeF64(${input})`,
        decode: `value, err := smithyDecodeF64(${payload})
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}
\treturn ${output ?? "struct{}"}{${output_field ?? "Payload"}: value}, nil`,
      }
    case "python":
      return {
        encode: `_smithy_encode_f64(${input})`,
        decode: `_smithy_decode_f64(${payload}, ${diagnostic})`,
      }
    case "swift":
      return {
        encode: `smithyEncodeF64(${input})`,
        decode: `try smithyDecodeF64(${payload}, operation: ${diagnostic})`,
      }
    case "csharp":
      return {
        encode: `EncodeF64(${input})`,
        decode: `DecodeF64(${payload}, ${diagnostic})`,
      }
    case "rust":
      return {
        encode: `${input}.to_be_bytes().to_vec()`,
        decode: `f64::from_be_bytes(${payload}.try_into().map_err(|_| {
                    Error::Protocol(format!("{} response has an invalid f64 field", ${diagnostic}))
                })?)`,
      }
  }
}

function render_i32_codec(
  language: Application_Value_Language,
  input: string,
  payload: string,
  diagnostic: string,
  output?: string,
  output_field?: string,
): Rendered_Application_Value_Codec {
  switch (language) {
    case "java":
    case "kotlin":
      return {
        encode: `smithyEncodeI32(${input})`,
        decode: `smithyDecodeI32(${payload}, ${diagnostic})`,
      }
    case "dart":
      return {
        encode: `_smithyEncodeI32(${input})`,
        decode: `_smithyDecodeI32(${payload}, ${diagnostic})`,
      }
    case "typescript":
      return {
        encode: `smithy_encode_i32(${input})`,
        decode: `smithy_decode_i32(${payload}, ${diagnostic})`,
      }
    case "go":
      return {
        encode: `wireValue := smithyEncodeI32(${input})`,
        decode: `value, err := smithyDecodeI32(${payload})
\tif err != nil {
\t\treturn ${output ?? "struct{}"}{}, operationError(${diagnostic}, err)
\t}
\treturn ${output ?? "struct{}"}{${output_field ?? "Payload"}: value}, nil`,
      }
    case "python":
      return {
        encode: `_smithy_encode_i32(${input})`,
        decode: `_smithy_decode_i32(${payload}, ${diagnostic})`,
      }
    case "swift":
      return {
        encode: `smithyEncodeI32(${input})`,
        decode: `try smithyDecodeI32(${payload}, operation: ${diagnostic})`,
      }
    case "csharp":
      return {
        encode: `EncodeI32(${input})`,
        decode: `DecodeI32(${payload}, ${diagnostic})`,
      }
    case "rust":
      return {
        encode: `${input}.to_be_bytes().to_vec()`,
        decode: `i32::from_be_bytes(${payload}.try_into().map_err(|_| {
                    Error::Protocol(format!("{} response has an invalid i32 field", ${diagnostic}))
                })?)`,
      }
  }
}

function render_optional_utf8_codec(
  language: Application_Value_Language,
  payload: string,
  diagnostic: string,
): string {
  switch (language) {
    case "java":
      return `(${payload} == null ? null : smithyDecodeUtf8(${payload}, ${diagnostic}))`
    case "kotlin":
      return `${payload}?.let { smithyDecodeUtf8(it, ${diagnostic}) }`
    case "dart":
      return `${payload} == null ? null : _smithyDecodeUtf8(${payload}!, ${diagnostic})`
    case "typescript":
      return `${payload} === undefined ? undefined : this.#transport.decode_utf8(${payload}!, ${diagnostic})`
    case "go":
      return `smithyDecodeOptionalUTF8(${payload})`
    case "python":
      return `${payload} if ${payload} is None else self._smithy_transport.decode_utf8(${payload}, ${diagnostic})`
    case "swift":
      return `try ${payload}.map { data in
      guard let value = String(data: data, encoding: .utf8) else {
        throw OpenKacheError(${diagnostic} + " response is not valid UTF-8")
      }
      return value
    }`
    case "csharp":
      return `${payload} is null ? null : new UTF8Encoding(false, true).GetString(${payload}!)`
    case "rust":
      return `${payload}.map(|value| String::from_utf8(value).map_err(|error| {
                    Error::Protocol(format!("{} response is not UTF-8: {error}", ${diagnostic}))
                })).transpose()?`
  }
}

function render_optional_f64_array_codec(
  language: Application_Value_Language,
  payload: string,
  diagnostic: string,
): string {
  switch (language) {
    case "java":
      return `(${payload} == null ? null : smithyDecodeF64Array(${payload}, ${diagnostic}))`
    case "kotlin":
      return `${payload}?.let { smithyDecodeF64Array(it, ${diagnostic}) }`
    case "dart":
      return `${payload} == null ? null : _smithyDecodeF64Array(${payload}!, ${diagnostic})`
    case "typescript":
      return `${payload} === undefined ? undefined : smithy_decode_f64_array(${payload}!, ${diagnostic})`
    case "go":
      return `smithyDecodeOptionalF64Array(${payload}, ${diagnostic})`
    case "python":
      return `${payload} if ${payload} is None else _smithy_decode_f64_array(${payload}, ${diagnostic})`
    case "swift":
      return `try ${payload}.map { try smithyDecodeF64Array($0, operation: ${diagnostic}) }`
    case "csharp":
      return `${payload} is null ? null : DecodeF64Array(${payload}!, ${diagnostic})`
    case "rust":
      return `${payload}.map(|value| smithy_decode_f64_array(&value, ${diagnostic})).transpose()?`
  }
}

function render_optional_raw_bytes_codec(
  _language: Application_Value_Language,
  payload: string,
  _diagnostic: string,
): string {
  return payload
}

function render_optional_u64_codec(
  language: Application_Value_Language,
  payload: string,
  diagnostic: string,
): string {
  switch (language) {
    case "java":
      return `(${payload} == null ? null : smithyDecodeU64(${payload}, ${diagnostic}))`
    case "kotlin":
      return `${payload}?.let { smithyDecodeU64(it, ${diagnostic}) }`
    case "dart":
      return `${payload} == null ? null : _smithyDecodeU64(${payload}!, ${diagnostic})`
    case "typescript":
      return `${payload} === undefined ? undefined : smithy_decode_u64(${payload}!, ${diagnostic})`
    case "go":
      return `smithyDecodeOptionalU64(${payload})`
    case "python":
      return `${payload} if ${payload} is None else _smithy_decode_u64(${payload}, ${diagnostic})`
    case "swift":
      return `try ${payload}.map { try smithyDecodeU64($0, operation: ${diagnostic}) }`
    case "csharp":
      return `${payload} is null ? null : DecodeU64(${payload}!, ${diagnostic})`
    case "rust":
      return `${payload}.map(|value| u64::from_be_bytes(value.try_into().map_err(|_| {
                    Error::Protocol(format!("{} response has an invalid u64 field", ${diagnostic}))
                })?)).transpose()?`
  }
}

function render_optional_bool_codec(
  language: Application_Value_Language,
  payload: string,
  diagnostic: string,
): string {
  switch (language) {
    case "java":
      return `(${payload} == null ? null : smithyDecodeBool(${payload}, ${diagnostic}))`
    case "kotlin":
      return `${payload}?.let { smithyDecodeBool(it, ${diagnostic}) }`
    case "dart":
      return `${payload} == null ? null : _smithyDecodeBool(${payload}!, ${diagnostic})`
    case "typescript":
      return `${payload} === undefined ? undefined : smithy_decode_bool(${payload}!, ${diagnostic})`
    case "go":
      return `smithyDecodeOptionalBool(${payload})`
    case "python":
      return `${payload} if ${payload} is None else _smithy_decode_bool(${payload}, ${diagnostic})`
    case "swift":
      return `try ${payload}.map { try smithyDecodeBool($0, operation: ${diagnostic}) }`
    case "csharp":
      return `${payload} is null ? null : DecodeBool(${payload}!, ${diagnostic})`
    case "rust":
      return `${payload}.map(|value| match value.as_slice() {
                    [0] => Ok(false),
                    [1] => Ok(true),
                    _ => Err(Error::Protocol(format!("{} response has an invalid boolean field", ${diagnostic}))),
                }).transpose()?`
  }
}

function render_optional_f64_codec(
  language: Application_Value_Language,
  payload: string,
  diagnostic: string,
): string {
  switch (language) {
    case "java":
      return `(${payload} == null ? null : smithyDecodeF64(${payload}, ${diagnostic}))`
    case "kotlin":
      return `${payload}?.let { smithyDecodeF64(it, ${diagnostic}) }`
    case "dart":
      return `${payload} == null ? null : _smithyDecodeF64(${payload}!, ${diagnostic})`
    case "typescript":
      return `${payload} === undefined ? undefined : smithy_decode_f64(${payload}!, ${diagnostic})`
    case "go":
      return `smithyDecodeOptionalF64(${payload})`
    case "python":
      return `${payload} if ${payload} is None else _smithy_decode_f64(${payload}, ${diagnostic})`
    case "swift":
      return `try ${payload}.map { try smithyDecodeF64($0, operation: ${diagnostic}) }`
    case "csharp":
      return `${payload} is null ? null : DecodeF64(${payload}!, ${diagnostic})`
    case "rust":
      return `${payload}.map(|value| f64::from_be_bytes(value.try_into().map_err(|_| {
                    Error::Protocol(format!("{} response has an invalid f64 field", ${diagnostic}))
                })?)).transpose()?`
  }
}

function render_optional_i32_codec(
  language: Application_Value_Language,
  payload: string,
  diagnostic: string,
): string {
  switch (language) {
    case "java":
      return `(${payload} == null ? null : smithyDecodeI32(${payload}, ${diagnostic}))`
    case "kotlin":
      return `${payload}?.let { smithyDecodeI32(it, ${diagnostic}) }`
    case "dart":
      return `${payload} == null ? null : _smithyDecodeI32(${payload}!, ${diagnostic})`
    case "typescript":
      return `${payload} === undefined ? undefined : smithy_decode_i32(${payload}!, ${diagnostic})`
    case "go":
      return `smithyDecodeOptionalI32(${payload})`
    case "python":
      return `${payload} if ${payload} is None else _smithy_decode_i32(${payload}, ${diagnostic})`
    case "swift":
      return `try ${payload}.map { try smithyDecodeI32($0, operation: ${diagnostic}) }`
    case "csharp":
      return `${payload} is null ? null : DecodeI32(${payload}!, ${diagnostic})`
    case "rust":
      return `${payload}.map(|value| i32::from_be_bytes(value.try_into().map_err(|_| {
                    Error::Protocol(format!("{} response has an invalid i32 field", ${diagnostic}))
                })?)).transpose()?`
  }
}

function render_go_optional_utf8_codec(
  payload: string,
  decoded: string,
  _diagnostic: string,
  _output: string,
): Rendered_Go_Composite_Field {
  return {
    expression: decoded,
    statements: `\t\t${decoded} := smithyDecodeOptionalUTF8(${payload})`,
  }
}

function render_go_optional_f64_array_codec(
  payload: string,
  decoded: string,
  diagnostic: string,
  output: string,
): Rendered_Go_Composite_Field {
  return {
    expression: decoded,
    statements: `\t\t${decoded}, err := smithyDecodeOptionalF64Array(${payload}, ${diagnostic})
\t\tif err != nil {
\t\t\treturn ${output}{}, operationError(${diagnostic}, err)
\t\t}`,
  }
}

function render_go_optional_raw_bytes_codec(
  payload: string,
  decoded: string,
  _diagnostic: string,
  _output: string,
): Rendered_Go_Composite_Field {
  return {
    expression: decoded,
    statements: `\t\t${decoded} := ${payload}`,
  }
}

function render_go_optional_u64_codec(
  payload: string,
  decoded: string,
  diagnostic: string,
  output: string,
): Rendered_Go_Composite_Field {
  return {
    expression: decoded,
    statements: `\t\t${decoded}, err := smithyDecodeOptionalU64(${payload})
\t\tif err != nil {
\t\t\treturn ${output}{}, operationError(${diagnostic}, err)
\t\t}`,
  }
}

function render_go_optional_bool_codec(
  payload: string,
  decoded: string,
  diagnostic: string,
  output: string,
): Rendered_Go_Composite_Field {
  return {
    expression: decoded,
    statements: `\t\t${decoded}, err := smithyDecodeOptionalBool(${payload})
\t\tif err != nil {
\t\t\treturn ${output}{}, operationError(${diagnostic}, err)
\t\t}`,
  }
}

function render_go_optional_f64_codec(
  payload: string,
  decoded: string,
  diagnostic: string,
  output: string,
): Rendered_Go_Composite_Field {
  return {
    expression: decoded,
    statements: `\t\t${decoded}, err := smithyDecodeOptionalF64(${payload})
\t\tif err != nil {
\t\t\treturn ${output}{}, operationError(${diagnostic}, err)
\t\t}`,
  }
}

function render_go_optional_i32_codec(
  payload: string,
  decoded: string,
  diagnostic: string,
  output: string,
): Rendered_Go_Composite_Field {
  return {
    expression: decoded,
    statements: `\t\t${decoded}, err := smithyDecodeOptionalI32(${payload})
\t\tif err != nil {
\t\t\treturn ${output}{}, operationError(${diagnostic}, err)
\t\t}`,
  }
}

/**
 * Resolves a model codec once and delegates rendering to its registration.
 * No operation renderer needs a codec-name switch.
 */
export function render_application_value_codec(
  language: Application_Value_Language,
  codecs: Application_Value_Codec_Pair,
  input: string,
  payload: string,
  diagnostic: string,
  output?: string,
  output_field?: string,
): Rendered_Application_Value_Codec {
  const input_registration = WIRE_CODEC_REGISTRY.find((candidate) =>
    candidate.name === codecs.input
  )
  const output_registration = WIRE_CODEC_REGISTRY.find((candidate) =>
    candidate.name === codecs.output
  )
  if (input_registration === undefined) {
    throw new Error(
      `wire codec ${JSON.stringify(codecs.input)} has no renderer registration`,
    )
  }
  if (output_registration === undefined) {
    throw new Error(
      `wire codec ${JSON.stringify(codecs.output)} has no renderer registration`,
    )
  }
  const encoded = input_registration.render(
    language,
    input,
    payload,
    diagnostic,
    output,
    output_field,
    codecs.input_type,
  )
  const decoded = output_registration.render(
    language,
    input,
    payload,
    diagnostic,
    output,
    output_field,
    codecs.output_type,
  )
  return { encode: encoded.encode, decode: decoded.decode }
}
