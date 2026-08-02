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
  readonly previous_data_protection_keys?: readonly Uint8Array[]
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

export interface Native_Client {
  next_request_id(): number
  cancel(request_id: number): boolean
  ping(request_id?: number): Promise<void>
  get(key: Uint8Array, request_id?: number): Promise<Uint8Array | null>
  get_json(key: Uint8Array, request_id?: number): Promise<string | null>
  set(
    key: Uint8Array,
    value: Uint8Array,
    condition?: "if_absent" | "if_present",
    ttl_ms?: number,
    mutation_id?: Uint8Array,
    request_id?: number,
  ): Promise<string>
  set_json(
    key: Uint8Array,
    value: unknown,
    condition?: "if_absent" | "if_present",
    ttl_ms?: number,
    mutation_id?: Uint8Array,
    request_id?: number,
  ): Promise<string>
  delete(
    key: Uint8Array,
    mutation_id?: Uint8Array,
    request_id?: number,
  ): Promise<boolean>
  stats(request_id?: number): Promise<string>
  sync(request_id?: number): Promise<void>
  close(): Promise<void>
  close_now(): void
  connection_state(): string
  reconnect(request_id?: number): Promise<void>
  raw_get(item_id: Uint8Array, request_id?: number): Promise<Uint8Array | null>
  raw_set(
    item_id: Uint8Array,
    value: Uint8Array,
    condition?: "if_absent" | "if_present",
    ttl_ms?: number,
    mutation_id?: Uint8Array,
    request_id?: number,
  ): Promise<string>
  raw_delete(
    item_id: Uint8Array,
    mutation_id?: Uint8Array,
    request_id?: number,
  ): Promise<boolean>
  metrics_snapshot(): Native_Metrics_Snapshot
}

export interface Native_Metrics_Snapshot {
  readonly requests: number
  readonly hits: number
  readonly misses: number
  readonly retries: number
  readonly reconnects: number
  readonly cancellations: number
  readonly transport_errors: number
  readonly protocol_errors: number
  readonly bytes_sent: number
  readonly bytes_received: number
  readonly active_lanes: number
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
