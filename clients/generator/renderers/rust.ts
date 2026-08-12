/** Rust API and operation renderers. */

import type { Api_Type } from "../../operation_models"
import type { Client_Contract } from "../../client_contract"
import { snake_case } from "../../generator_names"
import { operation_field_count } from "../../operation_plans"
import type { Operation_Result_Kind } from "../../compatibility_result_projections"
import { operation_uses_optional_value_layout } from "../../compatibility_response_framing"
import { is_packed_f64_type, rust_string_literal } from "../rendering"
import {
  has_application_value_codec,
  managed_operation_entries,
  managed_operation_label,
  managed_result_projection,
  operation_composite_field_codec,
  operation_composite_fields,
  operation_composite_value_count,
  operation_convenience_fields,
  operation_field_name,
  operation_fields,
  operation_is_global_empty,
  operation_is_global_field_sequence,
  operation_is_global_opaque,
  operation_item_fields,
  operation_opaque_field_name,
  operation_policy_fields,
  operation_request_is_opaque,
  operation_request_value_count,
  operation_request_value_name,
  operation_result_constant,
  operation_uses_compact_item_request,
  operation_uses_compact_namespace_request,
  operation_uses_compact_request_route,
  operation_uses_field_sequence_helpers,
  operation_uses_item_id_helpers,
  render_application_value_codec,
  render_composite_field_decode,
  render_composite_output,
  render_field_sequence_response_decode,
  render_operation_result,
  render_rust_container_helpers,
  render_rust_field_sequence_helpers,
  render_rust_generic_invocation,
  type Managed_Api_Operation,
} from "../managed"

function rust_api_type(type: Api_Type, required: boolean): string {
  let rendered: string
  switch (type.kind) {
    case "blob":
      rendered = "Vec<u8>"
      break
    case "boolean":
      rendered = "bool"
      break
    case "double":
      rendered = "f64"
      break
    case "enum":
      if (type.name === undefined) throw new Error("enum API type has no name")
      rendered = type.name
      break
    case "integer":
      rendered = "i32"
      break
    case "list":
      rendered = is_packed_f64_type(type)
        ? "Vec<f64>"
        : `Vec<${rust_api_type(type.member ?? { kind: "blob" }, true)}>`
      break
    case "map":
      rendered = `std::collections::BTreeMap<${rust_api_type(
        type.key ?? { kind: "string" },
        true,
      )}, ${rust_api_type(type.value ?? { kind: "blob" }, true)}>`
      break
    case "long":
      rendered = "i64"
      break
    case "structure":
      if (type.name === undefined) throw new Error("structure API type has no name")
      rendered = type.name
      break
    case "string":
      rendered = "String"
      break
    case "union":
      rendered = "Vec<u8>"
      break
    case "unsigned_long":
      rendered = "u64"
      break
  }
  return required ? rendered : `Option<${rendered}>`
}

function rust_api_type_supports_eq(
  contract: Client_Contract,
  type: Api_Type,
  visited = new Set<string>(),
): boolean {
  switch (type.kind) {
    case "double":
      return false
    case "list":
      return type.member === undefined
        ? true
        : rust_api_type_supports_eq(contract, type.member, visited)
    case "map":
      return rust_api_type_supports_eq(contract, type.key ?? { kind: "string" }, visited) &&
        rust_api_type_supports_eq(contract, type.value ?? { kind: "blob" }, visited)
    case "structure": {
      if (type.name === undefined || visited.has(type.name)) return true
      const structure = contract.api.structures.find(
        (candidate) => candidate.name === type.name,
      )
      if (structure === undefined) return true
      const next_visited = new Set(visited)
      next_visited.add(type.name)
      return structure.members.every((member) =>
        rust_api_type_supports_eq(contract, member.type, next_visited)
      )
    }
    default:
      return true
  }
}

/** Renders Smithy operation types and an API trait for Rust.
 *
 * @param contract - Validated language-neutral wire and API contract.
 * @returns Deterministic Rust source with a trailing newline.
 */
export function render_rust_api(contract: Client_Contract): string {
  const enums = contract.api.enums.map((enum_) => {
    const members = enum_.members
      .map((member) => `    /// Smithy ${member.value} value.
    ${member.name},`)
      .join("\n")
    return `/// Values defined by the Smithy ${enum_.name} shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ${enum_.name} {
${members}
}

impl ${enum_.name} {
    pub fn smithy_value(self) -> &'static str {
        match self {
${enum_.members.map((member) => `            Self::${member.name} => ${rust_string_literal(member.value)},`).join("\n")}
        }
    }

    pub fn from_smithy_value(value: &str) -> Option<Self> {
        match value {
${enum_.members.map((member) => `            ${rust_string_literal(member.value)} => Some(Self::${member.name}),`).join("\n")}
            _ => None,
        }
    }
}`
  })
  const structures = contract.api.structures.map((structure) => {
    if (structure.members.length === 0) {
      return `/// Smithy ${structure.name} structure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ${structure.name};`
    }
    const equality_derives = structure.members.every((member) =>
      rust_api_type_supports_eq(contract, member.type)
    )
      ? "Eq, PartialEq"
      : "PartialEq"
    const members = structure.members.map(
      (member) =>
        `    /// Smithy ${member.name} member.
    pub ${snake_case(member.name)}: ${rust_api_type(member.type, member.required)},`,
    )
    return `/// Smithy ${structure.name} structure.
#[derive(Clone, Debug, ${equality_derives})]
pub struct ${structure.name} {
${members.join("\n")}
}`
  })
  const operations = contract.api.operations.map(
    (operation) =>
      `    /// Invokes the Smithy ${operation.name} operation.
    fn ${snake_case(operation.name)}(
        &self,
        input: ${operation.input},
    ) -> impl core::future::Future<
        Output = core::result::Result<${operation.output}, Self::Error>,
    >;`,
  )
  return `// Generated from the OpenKache Smithy contract. Do not edit.

${[...enums, ...structures].join("\n\n")}

/// Operations defined by the OpenKache Smithy service.
///
/// The trait does not require Send futures because the Rust client exposes
/// both Tokio/Quinn and runtime-local Compio implementations. Callers that
/// need cross-thread scheduling can add the bound to the concrete client.
pub trait OpenKacheApi {
    /// Error returned by an operation.
    type Error;

${operations.join("\n\n")}
}
`
}

function render_rust_operation_method(
  contract: Client_Contract,
  operation: Managed_Api_Operation,
): string {
  const method_name = snake_case(operation.name)
  const operation_label = managed_operation_label(operation)
  const result_constant = (kind: Operation_Result_Kind): string =>
    operation_result_constant(operation, kind, "rust")
  const {
    input_condition,
    input_create_if_missing,
    input_eviction_mode,
    input_expected_revision,
    input_expiration_mode,
    input_item_id,
    input_name,
    input_namespace_id,
    input_policy,
    input_ttl_milliseconds,
    input_value,
    output_created,
    output_deleted,
    output_descriptor,
    output_json,
    output_outcome,
    output_value,
  } = operation_convenience_fields(operation, "rust")
  const input_item_ids = operation_item_fields(operation).map(
    (member) => operation_field_name(member, "rust"),
  )
  const input_item_id_expression = input_item_ids.length === 0
    ? "Vec::new()"
    : input_item_ids.length <= 1
    ? `input.${input_item_id}`
    : `smithy_concat_item_ids(&[${input_item_ids.map((name) => `&input.${name}`).join(", ")}])?`
  const {
    policy_default_eviction,
    policy_default_expiration,
    policy_default_ttl_milliseconds,
    policy_eviction_override,
    policy_expiration_override,
  } = operation_policy_fields(contract, operation, "rust")
  const application_value_codecs = operation.plan.application_value_codecs
  const input_request_value = operation_request_value_name(operation, "rust")
    ?? "Vec::new()"
  return render_operation_result(operation, "Rust", {
    raw_payload: () => {
      const output_payload = operation_opaque_field_name(operation, "output", "rust")
      const invocation = render_rust_generic_invocation(
        operation,
        `"${operation_label}"`,
      ) ??
        `$client::execute_unary(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    Vec::new(),
                )
                    .await?`
      return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let _ = &input;
                let result = ${invocation};
                smithy_require_kind(
                    &result,
                    &[${result_constant("ok")}],
                    "${operation_label}",
                )?;
                Ok(smithy::${operation.output} { ${output_payload}: result.payload })
            }`
    },
    opaque: () => {
        const input_payload = operation_request_is_opaque(operation)
          ? operation_opaque_field_name(operation, "input", "rust")
          : undefined
        const output_payload = operation_opaque_field_name(operation, "output", "rust")
        const codec = render_application_value_codec(
          "rust",
          application_value_codecs!,
          input_payload === undefined ? "Vec::new()" : `input.${input_payload}`,
          "result.payload",
          `"${operation_label}"`,
        )
        const decoded_payload = codec.decode
        const invocation = render_rust_generic_invocation(
          operation,
          `"${operation_label}"`,
        ) ??
          (operation_is_global_empty(operation)
            ? `$client::execute_unary(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    Vec::new(),
                )
                    .await?`
            : `$client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    input.${input_namespace_id},
                    ${input_item_id_expression},
                    ${input_request_value},
                    openkache_client_core::SetOptions::new(),
                )
                    .await?`)
        return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let result = ${invocation};
                smithy_require_kind(
                    &result,
                    &[${result_constant("value")}],
                    "${operation_label}",
                )?;
                let payload = ${decoded_payload};
                Ok(smithy::${operation.output} { ${output_payload}: payload })
            }`
    },
    field_sequence: () => {
        const decoded_fields = operation_composite_fields(operation)
          .map((field, index) => ({
            name: `decoded_value_${index}`,
            expression: render_composite_field_decode(
              "rust",
              operation_composite_field_codec(operation, field),
              "values.remove(0)",
              `"${operation_label}"`,
              field.required,
              field.type,
            ),
          }))
        const decoded_statements = decoded_fields
          .map((field) => `                let ${field.name} = ${field.expression};`)
          .join("\n")
        const output_expression = render_composite_output(
          operation,
          "rust",
          decoded_fields.map((field) => field.name),
        )
        const response_values = render_field_sequence_response_decode(
          "rust",
          operation,
          "&result.payload",
          `"${operation_label}"`,
        )
        const invocation = render_rust_generic_invocation(
          operation,
          `"${operation_label}"`,
        ) ??
          (operation_is_global_empty(operation)
            ? `$client::execute_unary(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    Vec::new(),
                )
                    .await?`
            : `$client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    input.${input_namespace_id},
                    ${input_item_id_expression},
                    ${input_request_value === "Vec::new()" ? "Vec::new()" : `input.${input_request_value}`},
                    SetOptions::new(),
                )
                    .await?`)
        return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let result = ${invocation};
                smithy_require_kind(
                    &result,
                    &[${result_constant("value")}],
                    "${operation_label}",
                )?;
                let mut values = ${response_values};
${decoded_statements}
                Ok(${output_expression})
            }`
    },
    optional_payload: () => {
      if (operation_field_count(operation.plan.operation, "output", "value") > 1) {
        const output_values = operation_fields(operation, "output", "value")
          .map((member) =>
            `                    ${operation_field_name(member, "rust")}: values.remove(0),`)
          .join("\n")
        return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    input.${input_namespace_id},
                    ${input_item_id_expression},
                    ${input_request_value === "Vec::new()" ? "Vec::new()" : `input.${input_request_value}`},
                    SetOptions::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[${result_constant("value")}],
                    "${operation_label}",
                )?;
                let mut values = smithy_decode_optional_values(
                    &result.payload,
                    ${operation_field_count(operation.plan.operation, "output", "value")},
                    "${operation_label}",
                )?;
                Ok(smithy::${operation.output} {
${output_values}
                })
            }`
      }
      return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    input.${input_namespace_id},
                    ${input_item_id_expression},
                    ${input_request_value === "Vec::new()" ? "Vec::new()" : `input.${input_request_value}`},
                    SetOptions::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[
                        ${result_constant("value")},
                        ${result_constant("not_found")},
                    ],
                    "${operation_label}",
                )?;
                let value = if result.kind
                    == ${result_constant("not_found")}
                {
                    None
                } else {
                    Some(result.payload)
                };
                Ok(smithy::${operation.output} { ${output_value}: value })
            }`
    },
    status_outcome: () => {
      return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let options = smithy_set_options(
                    input.${input_condition},
                    input.${input_expiration_mode},
                    input.${input_ttl_milliseconds},
                    input.${input_eviction_mode},
                )?;
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    input.${input_namespace_id},
                    ${input_item_id_expression},
                    input.${input_value},
                    options,
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[
                        ${result_constant("created")},
                        ${result_constant("replaced")},
                        ${result_constant("not_stored")},
                    ],
                    "${operation_label}",
                )?;
                let outcome = match result.kind {
                    ${result_constant("created")} => {
                        smithy::SetOutcome::Created
                    }
                    ${result_constant("replaced")} => {
                        smithy::SetOutcome::Replaced
                    }
                    ${result_constant("not_stored")} => {
                        smithy::SetOutcome::NotStored
                    }
                    _ => unreachable!("smithy_require_kind validated SET result"),
                };
                Ok(smithy::${operation.output} { ${output_outcome}: outcome })
            }`
    },
    boolean_outcome: () => {
      return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    input.${input_namespace_id},
                    ${input_item_id_expression},
                    [],
                    SetOptions::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[
                        ${result_constant("deleted")},
                        ${result_constant("not_deleted")},
                    ],
                    "${operation_label}",
                )?;
                let deleted =
                    result.kind == ${result_constant("deleted")};
                Ok(smithy::${operation.output} { ${output_deleted}: deleted })
            }`
    },
    text_payload: () => {
      return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    input.${input_namespace_id},
                    [],
                    [],
                    SetOptions::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[${result_constant("value")}],
                    "${operation_label}",
                )?;
                let json = String::from_utf8(result.payload).map_err(|error| {
                    Error::Protocol(format!("${operation_label} response is not UTF-8: {error}"))
                })?;
                Ok(smithy::${operation.output} { ${output_json}: json })
            }`
    },
    empty: () => {
      if (operation_is_global_empty(operation)) {
        return `            async fn ${method_name}(
                &self,
                _input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let result = $client::execute_raw(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    [],
                    [],
                    SetOptions::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[${result_constant("ok")}],
                    "${operation_label}",
                )?;
                Ok(smithy::${operation.output})
            }`
      }
      if (
        operation_is_global_opaque(operation) ||
        operation_is_global_field_sequence(operation)
      ) {
        const invocation = render_rust_generic_invocation(
          operation,
          `"${operation_label}"`,
        )!
        return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let result = ${invocation};
                smithy_require_kind(
                    &result,
                    &[${result_constant(managed_result_projection(operation).result_kinds[0] ?? "raw")}],
                    "${operation_label}",
                )?;
                Ok(smithy::${operation.output})
            }`
      }
      if (operation_uses_compact_item_request(operation)) {
        const has_request_value = operation_request_value_count(operation) > 0
        if (has_request_value) {
          return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let options = smithy_set_options(
                    input.${input_condition},
                    input.${input_expiration_mode},
                    input.${input_ttl_milliseconds},
                    input.${input_eviction_mode},
                )?;
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    input.${input_namespace_id},
                    ${input_item_id_expression},
                    input.${input_value},
                    options,
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[${result_constant("ok")}],
                    "${operation_label}",
                )?;
                Ok(smithy::${operation.output})
            }`
        }
        return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    input.${input_namespace_id},
                    ${input_item_id_expression},
                    [],
                    SetOptions::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[${result_constant("ok")}],
                    "${operation_label}",
                )?;
                Ok(smithy::${operation.output})
            }`
      }
      if (operation_uses_compact_namespace_request(operation)) {
        return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let result = $client::execute_scoped(
                    self,
                    openkache_client_core::Opcode::${operation.name},
                    input.${input_namespace_id},
                    [],
                    [],
                    SetOptions::new(),
                )
                    .await?;
                smithy_require_kind(
                    &result,
                    &[${result_constant("ok")}],
                    "${operation_label}",
                )?;
                Ok(smithy::${operation.output})
            }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_delete")) {
        return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                $client::namespace_delete(self, input.${input_namespace_id}, input.${input_expected_revision})
                    .await?;
                Ok(smithy::${operation.output})
            }`
      }
      throw new Error(`unsupported generated Rust empty operation ${operation.name}`)
    },
    descriptor: () => {
      if (operation_uses_compact_request_route(operation, "namespace_open")) {
        return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let policy = input
                    .${input_policy}
                    .map(|policy| smithy_namespace_policy(
                        policy.${policy_default_expiration},
                        policy.${policy_default_ttl_milliseconds},
                        policy.${policy_expiration_override},
                        policy.${policy_default_eviction},
                        policy.${policy_eviction_override},
                    ))
                    .transpose()?;
                let (descriptor, created) = $client::namespace_open_with_outcome(
                    self,
                    input.${input_name}.into_bytes(),
                    input.${input_create_if_missing},
                    policy,
                )
                .await?;
                Ok(smithy::${operation.output} {
                    ${output_descriptor}: smithy_namespace_descriptor(descriptor),
                    ${output_created}: created,
                })
            }`
      }
      if (operation_uses_compact_request_route(operation, "namespace_update_policy")) {
        return `            async fn ${method_name}(
                &self,
                input: smithy::${operation.input},
            ) -> std::result::Result<smithy::${operation.output}, Self::Error> {
                let policy = smithy_namespace_policy(
                    input.${input_policy}.${policy_default_expiration},
                    input.${input_policy}.${policy_default_ttl_milliseconds},
                    input.${input_policy}.${policy_expiration_override},
                    input.${input_policy}.${policy_default_eviction},
                    input.${input_policy}.${policy_eviction_override},
                )?;
                let descriptor = $client::namespace_update_policy(
                    self,
                    input.${input_namespace_id},
                    input.${input_expected_revision},
                    policy,
                )
                .await?;
                Ok(smithy::${operation.output} {
                    ${output_descriptor}: smithy_namespace_descriptor(descriptor),
                })
            }`
      }
      throw new Error(`unsupported generated Rust namespace operation ${operation.name}`)
    },
  })
}

/** Renders generated Rust operation implementations backed by the shared client core. */
export function render_rust_operations(contract: Client_Contract): string {
  const managed_operations = managed_operation_entries(contract)
  const field_sequence_helpers = managed_operations.some(
    operation_uses_field_sequence_helpers,
  )
    ? render_rust_field_sequence_helpers()
    : ""
  const methods = managed_operations
    .map((operation) => render_rust_operation_method(contract, operation))
    .join("\n\n")
  const container_helpers = render_rust_container_helpers(
    contract.max_value_bytes,
    methods,
  )
  const f64_array_helpers = has_application_value_codec(
    managed_operations,
    "packed_f64_be",
  )
    ? `fn smithy_encode_f64_array(values: &[f64]) -> std::result::Result<Vec<u8>, Error> {
    let mut payload = Vec::with_capacity(values.len() * 8);
    for value in values {
        if !value.is_finite() {
            return Err(Error::Protocol(
                "binary64 array input must contain finite values".into(),
            ));
        }
        payload.extend_from_slice(&value.to_be_bytes());
    }
    Ok(payload)
}

fn smithy_decode_f64_array(
    payload: &[u8],
    operation: &str,
) -> std::result::Result<Vec<f64>, Error> {
    if payload.len() % 8 != 0 {
        return Err(Error::Protocol(format!(
            "{operation} response has a malformed binary64 array length",
        )));
    }
    let mut values = Vec::with_capacity(payload.len() / 8);
    for chunk in payload.chunks_exact(8) {
        let value = f64::from_be_bytes(chunk.try_into().map_err(|_| {
            Error::Protocol(format!("{operation} response has an invalid binary64 value"))
        })?);
        if !value.is_finite() {
            return Err(Error::Protocol(format!(
                "{operation} response contains a non-finite binary64 value",
            )));
        }
        values.push(value);
    }
    Ok(values)
}
`
    : ""
  const item_id_helpers = managed_operations.some(
    operation_uses_item_id_helpers,
  )
    ? `fn smithy_concat_item_ids(
    item_ids: &[&[u8]],
) -> std::result::Result<Vec<u8>, Error> {
    let mut combined = Vec::with_capacity(
        item_ids.len() * openkache_client_core::ITEM_ID_BYTES,
    );
    for item_id in item_ids {
        if item_id.len() != openkache_client_core::ITEM_ID_BYTES {
            return Err(Error::Protocol(format!(
                "item IDs must contain exactly {} bytes",
                openkache_client_core::ITEM_ID_BYTES,
            )));
        }
        combined.extend_from_slice(item_id);
    }
    Ok(combined)
}
`
    : ""
  const optional_values_helpers = managed_operations.some(
    operation_uses_optional_value_layout,
  )
    ? `fn smithy_decode_optional_values(
    payload: &[u8],
    value_count: usize,
    _operation: &str,
) -> std::result::Result<Vec<Option<Vec<u8>>>, Error> {
    openkache_client_core::decode_optional_values(payload, value_count)
        .map_err(|error| Error::Protocol(error.to_string()))
}
`
    : ""
  const compatibility_helpers = `${item_id_helpers}${optional_values_helpers}`
  return `// Generated from the OpenKache Smithy client contract. Do not edit.

fn smithy_require_kind(
    result: &openkache_client_core::OperationResult,
    expected: &[u32],
    operation: &str,
) -> std::result::Result<(), Error> {
    if expected.contains(&result.kind) {
        return Ok(());
    }
    Err(Error::Protocol(format!(
        "{operation} returned unexpected result kind {}",
        result.kind,
    )))
}

${f64_array_helpers}

${container_helpers}
${field_sequence_helpers}
${compatibility_helpers}

macro_rules! impl_smithy_api {
    ($client:ident) => {
        impl smithy::OpenKacheApi for $client {
            type Error = Error;

${methods}
        }
    };
}

#[cfg(feature = "quic-quinn")]
impl_smithy_api!(RawClient);

#[cfg(feature = "quic-compio")]
impl_smithy_api!(LocalRawClient);
`
}
