/** Native FFI projections for the protocol-v1 compatibility surface. */

import { request_transport_plan } from "./compatibility_request_adapters"
import type {
  Ffi_Input_Kind,
  Ffi_Operation_Contract,
} from "./client_contract"
import type {
  Api_Operation,
  Api_Operation_Contract,
} from "./operation_models"

/**
 * Derives the native convenience ABI capabilities for one modeled operation.
 *
 * This is intentionally a compatibility adapter projection. Generic generated
 * operations use their canonical request plan directly and must not be
 * classified by familiar namespace/item/value roles here.
 */
export function compatibility_ffi_operation_contract(
  operation: Api_Operation,
): Ffi_Operation_Contract | undefined {
  const semantic: Api_Operation_Contract | undefined = operation.contract
  if (semantic === undefined) return undefined
  const request = request_transport_plan(semantic)
  const request_role_count = (role: string): number =>
    semantic.request_plan?.filter((field) => field.role === role).length ?? 0
  const request_value_count = request_role_count("value")
  const request_item_count = request_role_count("item_id")
  const accepts_set_options = semantic.request_plan?.some((field) =>
    field.role === "condition" ||
    field.role === "expiration_mode" ||
    field.role === "eviction_mode" ||
    field.role === "ttl_milliseconds"
  ) ?? false
  const generic = (
    input_kind: Ffi_Input_Kind,
    accepts_value: boolean,
    supports_protected: boolean,
    supports_raw: boolean,
    supports_scoped: boolean,
    dedicated_abi: boolean,
    request_item_count = 0,
    allow_set_options = accepts_set_options,
  ): Ffi_Operation_Contract => ({
    input_kind,
    request_item_count,
    accepts_value,
    accepts_set_options: allow_set_options,
    supports_protected,
    supports_raw,
    supports_scoped,
    dedicated_abi,
  })

  if (request.compact_adapter !== undefined) {
    const route = request.compact_adapter?.route
    switch (route) {
      case "namespace_open":
      case "namespace_update_policy":
      case "namespace_delete":
        return generic("none", false, false, false, false, true)
      case "item":
      case "set":
        return generic(
          "item_id",
          request_value_count > 0,
          true,
          true,
          true,
          false,
          request_item_count,
        )
      case "namespace":
        return generic("none", false, true, true, true, false)
      default:
        throw new Error(
          `operation ${operation.name} has unsupported protocol-v1 compact request adapter`,
        )
    }
  }
  switch (request.request_framing) {
    case "opaque":
      return generic("none", true, true, true, false, false)
    case "empty":
      return generic("none", false, true, true, false, false)
    case "ordered_fields": {
      const has_identity = semantic.request_plan?.some(
        (field) => field.role === "namespace_id" || field.role === "item_id",
      ) ?? false
      return generic(
        "none",
        true,
        !has_identity,
        true,
        false,
        false,
        0,
        false,
      )
    }
  }
}
