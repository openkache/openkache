/**
 * Node.js, Bun, and Deno loading contract for the packaged Node-API adapter.
 */

import { createRequire } from "node:module"
import { fileURLToPath } from "node:url"

export interface Native_Identity {
  readonly certificate_chain: readonly Uint8Array[]
  readonly private_key: Uint8Array
}

export interface Native_Client_Options {
  readonly address: string
  readonly server_name: string
  readonly certificate: Uint8Array
  readonly identity?: Native_Identity
  readonly data_protection_key: Uint8Array
  readonly compression_enabled: boolean
  readonly compression_level?: number
  readonly minimum_input_size?: number
  readonly minimum_savings?: number
  readonly connect_timeout_ms?: number
  readonly request_timeout_ms?: number
  readonly retry_max_attempts?: number
  readonly max_in_flight?: number
  readonly encryption?: "compact" | "robust"
}

export interface Native_Namespace_Policy {
  readonly default_expiration: "no_expiry" | "fixed_ttl"
  readonly default_ttl_milliseconds?: bigint
  readonly expiration_override: "allowed" | "disallowed"
  readonly default_eviction: "evictable" | "eviction_protected"
  readonly eviction_override: "allowed" | "disallowed"
}

export interface Native_Namespace_Descriptor {
  readonly namespace_id: bigint
  readonly revision: bigint
  readonly policy: Native_Namespace_Policy
}

export interface Native_Namespace_Open_Output {
  readonly descriptor: Native_Namespace_Descriptor
  readonly created: boolean
}

interface Native_Value_Envelope {
  readonly encoding: string
  readonly type_name: string
  readonly payload: Uint8Array
}

export interface Native_Client {
  ping(): Promise<void>
  get(key: Uint8Array): Promise<Uint8Array | null>
  get_value(key: Uint8Array): Promise<Native_Value_Envelope | null>
  get_json(key: Uint8Array): Promise<string | null>
  set(
    key: Uint8Array,
    value: Uint8Array,
    condition?: "any" | "if_absent" | "if_present",
    expiration_mode?: "inherit" | "no_expiry" | "explicit_ttl",
    eviction_mode?: "inherit" | "evictable" | "eviction_protected",
    ttl_ms?: number,
  ): Promise<string>
  set_value(
    key: Uint8Array,
    encoding: string,
    type_name: string,
    payload: Uint8Array,
    condition?: "any" | "if_absent" | "if_present",
    expiration_mode?: "inherit" | "no_expiry" | "explicit_ttl",
    eviction_mode?: "inherit" | "evictable" | "eviction_protected",
    ttl_ms?: number,
  ): Promise<string>
  set_json(
    key: Uint8Array,
    value: unknown,
    condition?: "any" | "if_absent" | "if_present",
    expiration_mode?: "inherit" | "no_expiry" | "explicit_ttl",
    eviction_mode?: "inherit" | "evictable" | "eviction_protected",
    ttl_ms?: number,
  ): Promise<string>
  delete(key: Uint8Array): Promise<boolean>
  stats(): Promise<string>
  sync(): Promise<void>
  close(): Promise<void>
  close_now(): void
  connection_state(): string
  reconnect(): Promise<void>
  raw_get(item_id: Uint8Array): Promise<Uint8Array | null>
  raw_get_in_namespace(
    namespace_id: bigint,
    item_id: Uint8Array,
  ): Promise<Uint8Array | null>
  raw_set(
    item_id: Uint8Array,
    value: Uint8Array,
    condition?: "any" | "if_absent" | "if_present",
    expiration_mode?: "inherit" | "no_expiry" | "explicit_ttl",
    eviction_mode?: "inherit" | "evictable" | "eviction_protected",
    ttl_ms?: number,
  ): Promise<string>
  raw_set_in_namespace(
    namespace_id: bigint,
    item_id: Uint8Array,
    value: Uint8Array,
    condition?: "any" | "if_absent" | "if_present",
    expiration_mode?: "inherit" | "no_expiry" | "explicit_ttl",
    eviction_mode?: "inherit" | "evictable" | "eviction_protected",
    ttl_ms?: bigint,
  ): Promise<string>
  raw_delete(item_id: Uint8Array): Promise<boolean>
  raw_delete_in_namespace(
    namespace_id: bigint,
    item_id: Uint8Array,
  ): Promise<boolean>
  namespace_open(
    name: string,
    create_if_missing: boolean,
    policy?: Native_Namespace_Policy,
  ): Promise<Native_Namespace_Open_Output>
  namespace_update_policy(
    namespace_id: bigint,
    expected_revision: bigint,
    policy: Native_Namespace_Policy,
  ): Promise<Native_Namespace_Descriptor>
  namespace_delete(namespace_id: bigint, expected_revision: bigint): Promise<void>
  stats_in_namespace(namespace_id: bigint): Promise<string>
  sync_in_namespace(namespace_id: bigint): Promise<void>
}

interface Native_Module {
  connect(options: Native_Client_Options): Promise<Native_Client>
}

const REQUIRE = createRequire(import.meta.url)

/**
 * Loads the Node.js, Bun, or Deno Node-API adapter for the current platform.
 *
 * @param native_path - Optional custom `.node` adapter path.
 * @returns The semantic native client module.
 * @throws {Error} When the platform is unsupported or the addon cannot load.
 */
export function load_native_module(native_path?: string): Native_Module {
  const module_path = native_path ?? default_native_path()
  return REQUIRE(module_path) as Native_Module
}

function default_native_path(): string {
  let artifact_name: string | undefined
  if (process.platform === "linux") {
    if (process.arch === "x64") {
      artifact_name = "openkache-client.linux-x64-gnu.node"
    } else if (process.arch === "arm64") {
      artifact_name = "openkache-client.linux-arm64-gnu.node"
    }
  } else if (process.platform === "darwin" && process.arch === "arm64") {
    artifact_name = "openkache-client.darwin-arm64.node"
  }
  if (artifact_name === undefined) {
    throw new Error(
      `packaged native adapter supports Linux x64/ARM64 and Apple Silicon macOS, ` +
        `got ${process.platform} ${process.arch}; provide native_path for a custom build`,
    )
  }
  return fileURLToPath(
    new URL(`../target/native/${artifact_name}`, import.meta.url),
  )
}
