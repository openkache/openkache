//! Top-level client contract extraction.

import {
  extract_compatibility_wire_contract as extract_protocol_wire_contract,
} from "../../protocol/compatibility_v1"
import {
  CLIENT_DEFAULTS_TRAIT_ID,
  CLIENT_SERVICE_SHAPE_ID,
  FFI_CONTRACT_TRAIT_ID,
  SERVICE_SHAPE_ID,
  VALUE_ENVELOPE_TRAIT_ID,
  VALUE_FORMAT_TRAIT_ID,
} from "./config"
import {
  api_contract,
  object_member,
  object_value,
  trait_value_any,
} from "./extract_ast"
import { ffi_contract } from "./extract_ffi"
import {
  client_defaults_contract,
  value_envelope_contract,
  value_format_contract,
} from "./extract_values"
import type { Client_Contract } from "./model"

export function extract_client_contract(ast: unknown): Client_Contract {
  const ast_object = object_value(ast, "Smithy AST")
  const shapes = object_member(ast_object, "shapes", "Smithy AST")
  const wire = extract_protocol_wire_contract(ast)
  const client_service_id = shapes[CLIENT_SERVICE_SHAPE_ID] === undefined
    ? SERVICE_SHAPE_ID
    : CLIENT_SERVICE_SHAPE_ID
  const client_namespace = client_service_id.slice(0, client_service_id.lastIndexOf("#"))
  const service = object_member(shapes, client_service_id, "Smithy AST.shapes")
  const location = `Smithy AST.shapes.${client_service_id}`
  const trait_ids = (trait_id: string): readonly string[] =>
    client_service_id === SERVICE_SHAPE_ID
      ? [trait_id, trait_id.replace("openkache.client#", "openkache.protocol#")]
      : [trait_id]
  const value_format_trait = trait_value_any(service, trait_ids(VALUE_FORMAT_TRAIT_ID), location)
  const value_envelope_trait = trait_value_any(service, trait_ids(VALUE_ENVELOPE_TRAIT_ID), location)
  const client_defaults_trait = trait_value_any(service, trait_ids(CLIENT_DEFAULTS_TRAIT_ID), location)
  const ffi_trait = trait_value_any(service, trait_ids(FFI_CONTRACT_TRAIT_ID), location)
  const parsed_api = api_contract(shapes, client_service_id, client_namespace)
  const api = {
    ...parsed_api,
    // Smithy AST output is not required to preserve service-operation order.
    // Use the protocol assignments so every generated API presents operations
    // in the same stable order as the wire contract.
    operations: [...parsed_api.operations].sort(
      (left, right) =>
        (wire.opcodes.find((entry) => entry.name === left.name)?.value ?? 0) -
        (wire.opcodes.find((entry) => entry.name === right.name)?.value ?? 0),
    ),
  }
  const opcode_names = new Set(wire.opcodes.map((entry) => entry.name))
  const api_operation_names = new Set<string>()
  for (const operation of api.operations) {
    if (api_operation_names.has(operation.name)) {
      throw new Error(`duplicate client operation ${operation.name}`)
    }
    api_operation_names.add(operation.name)
    if (!opcode_names.has(operation.name)) {
      throw new Error(
        `client operation ${operation.name} has no matching protocol opcode`,
      )
    }
  }
  const ffi = ffi_contract(ffi_trait, shapes, client_namespace)
  const opcode_values = new Set(wire.opcodes.map((entry) => entry.value))
  for (const entry of ffi.operations) {
    if (opcode_values.has(entry.value)) {
      throw new Error(
        `FFI operation ${entry.name} wire value ${entry.value} overlaps a protocol opcode`,
      )
    }
  }
  return {
    ...wire,
    api,
    client_defaults: client_defaults_contract(client_defaults_trait),
    ffi,
    value_envelope: value_envelope_contract(value_envelope_trait),
    value_format: value_format_contract(value_format_trait),
  }
}
