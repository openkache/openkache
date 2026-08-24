//! Paths and Smithy identifiers shared by the client generator.

import { existsSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

export const CLIENTS_DIRECTORY = dirname(dirname(fileURLToPath(import.meta.url)))
export const PUBLIC_ROOT = dirname(CLIENTS_DIRECTORY)
export const PROTOCOL_DIRECTORY = join(PUBLIC_ROOT, "protocol")
export const MODEL_DIRECTORY = "model"
export const SMITHY_EXECUTABLE = process.env.OPENKACHE_SMITHY_EXECUTABLE ?? "smithy"
export const SMITHY_USE_SHELL = process.env.OPENKACHE_SMITHY_USE_SHELL === "1"
export const SERVICE_SHAPE_ID = "openkache.protocol#OpenKache"
export const CLIENT_SERVICE_SHAPE_ID = "openkache.client#OpenKacheClient"
export const FFI_CONTRACT_TRAIT_ID = "openkache.client#ffiContract"
export const CLIENT_DEFAULTS_TRAIT_ID = "openkache.client#clientDefaults"
export const VALUE_FORMAT_TRAIT_ID = "openkache.client#valueFormat"
export const VALUE_ENVELOPE_TRAIT_ID = "openkache.client#valueEnvelope"
export const UNSIGNED_LONG_TRAIT_ID = "openkache.client#unsignedLong"

/**
 * Gate 0 defaults used only when parsing a pre-Gate-0 synthetic Smithy AST.
 *
 * The checked-in client model always supplies these members through the
 * `clientDefaults` trait; retaining a compatibility fallback keeps the
 * generator's historical fixture helpers useful without weakening validation
 * of the maintained model.
 */
export const LEGACY_GATE0_DEFAULTS = {
  alpn_version: 1,
  compression: 0,
  encryption: 0,
  item_id_root_key_hex:
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
  namespace_id: 1,
  value_selector: 0x10,
} as const

export const FFI_ENUMS = {
  operations: { name: "FfiOperation", kind: "FFI operation" },
  transports: { name: "FfiTransport", kind: "FFI transport" },
  result_kinds: { name: "FfiResultKind", kind: "FFI result" },
  connection_states: { name: "FfiConnectionState", kind: "FFI connection state" },
  set_conditions: { name: "FfiSetCondition", kind: "FFI SET condition" },
  namespace_descriptor_decode_statuses: {
    name: "FfiNamespaceDescriptorDecodeStatus",
    kind: "FFI namespace descriptor decode status",
  },
  namespace_default_expirations: {
    name: "FfiNamespaceDefaultExpiration",
    kind: "FFI namespace default expiration",
  },
  namespace_default_evictions: {
    name: "FfiNamespaceDefaultEviction",
    kind: "FFI namespace default eviction",
  },
  namespace_override_policies: {
    name: "FfiNamespaceOverridePolicy",
    kind: "FFI namespace override policy",
  },
} as const
const GENERATED_OUTPUT_ROOT = resolve(
  process.env.OPENKACHE_GENERATION_OUTPUT_ROOT ?? PUBLIC_ROOT,
)
function generated_path(...segments: string[]): string {
  return join(GENERATED_OUTPUT_ROOT, ...segments)
}

export function resolve_smithy_executable(): string {
  if (
    SMITHY_EXECUTABLE.length === 0 ||
    !SMITHY_EXECUTABLE.includes("/") ||
    SMITHY_EXECUTABLE.startsWith("/")
  ) {
    return SMITHY_EXECUTABLE
  }
  let directory = resolve(process.cwd())
  for (;;) {
    if (
      SMITHY_EXECUTABLE.startsWith("external/") &&
      existsSync(resolve(directory, "external"))
    ) {
      return resolve(directory, SMITHY_EXECUTABLE)
    }
    const candidate = resolve(directory, SMITHY_EXECUTABLE)
    if (existsSync(candidate)) return candidate
    const parent = dirname(directory)
    if (parent === directory) return SMITHY_EXECUTABLE
    directory = parent
  }
}
export const GENERATED_OUTPUTS = {
  csharp_api: generated_path("clients/dotnet/OpenKache/generated_local/SmithyApi.g.cs"),
  csharp_wire: generated_path("clients/dotnet/OpenKache/generated_local/WireValues.g.cs"),
  rust_client: process.env.OPENKACHE_RUST_CLIENT_OUTPUT ??
    generated_path("clients/core/generated_local/client_contract.rs"),
  rust_api: process.env.OPENKACHE_RUST_API_OUTPUT ??
    generated_path("clients/rust/generated_local/smithy_api.rs"),
  rust_wire: process.env.OPENKACHE_RUST_WIRE_OUTPUT ??
    generated_path("protocol/generated_local/wire_values.rs"),
  typescript_api: generated_path("clients/typescript/src/generated_local/smithy-api.ts"),
  typescript_value_format: generated_path(
    "clients/typescript/src/generated_local/smithy-value-format.ts",
  ),
  typescript_value_envelope: generated_path(
    "clients/typescript/src/generated_local/smithy-value-envelope.ts",
  ),
  python_api: process.env.OPENKACHE_PYTHON_API_OUTPUT ??
    generated_path("clients/python/src/openkache/_generated/smithy_api.py"),
  python_contract: process.env.OPENKACHE_PYTHON_CONTRACT_OUTPUT ??
    generated_path("clients/python/src/openkache/_generated/smithy_contract.py"),
  swift_api: process.env.OPENKACHE_SWIFT_API_OUTPUT ??
    generated_path("clients/swift/generated_local/SmithyAPI.swift"),
  c_contract: process.env.OPENKACHE_C_CONTRACT_OUTPUT ??
    generated_path("clients/core/generated_local/smithy_contract.h"),
  go_api: generated_path("clients/go/smithy_api.go"),
  go_contract: generated_path("clients/go/smithy_contract.go"),
} as const
