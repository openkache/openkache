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
  readonly compression_level: number
  readonly minimum_input_size: number
  readonly minimum_savings: number
  readonly connect_timeout_ms: number
  readonly request_timeout_ms: number
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
  set(
    key: Uint8Array,
    value: Uint8Array,
    condition?: "if_absent" | "if_present",
    ttl_ms?: number,
  ): Promise<string>
  set_value(
    key: Uint8Array,
    encoding: string,
    type_name: string,
    payload: Uint8Array,
    condition?: "if_absent" | "if_present",
    ttl_ms?: number,
  ): Promise<string>
  delete(key: Uint8Array): Promise<boolean>
  stats(): Promise<string>
  sync(): Promise<void>
  close(): void
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
  if (process.platform !== "linux") {
    throw new Error(
      `packaged native adapter supports Linux, got ${process.platform} ${process.arch}; ` +
        "provide native_path for a custom build",
    )
  }
  let artifact_name: string
  switch (process.arch) {
    case "x64":
      artifact_name = "openkache-client.linux-x64-gnu.node"
      break
    case "arm64":
      artifact_name = "openkache-client.linux-arm64-gnu.node"
      break
    default:
      throw new Error(
        `packaged native adapter supports Linux x64 and ARM64, got ${process.arch}; ` +
          "provide native_path for a custom build",
      )
  }
  return fileURLToPath(
    new URL(`../target/native/${artifact_name}`, import.meta.url),
  )
}
