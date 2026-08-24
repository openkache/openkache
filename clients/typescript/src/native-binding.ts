/**
 * Private Node-API boundary for the Gate 0 client.
 *
 * This module is intentionally not re-exported from the package entry point.
 * The native adapter owns transport, retries, TLS, and value-envelope details;
 * the public TypeScript facade exposes only the five Gate 0 operations.
 */

import { createRequire } from "node:module"
import { fileURLToPath } from "node:url"

/** Options consumed by the private native connector. */
export interface Native_Client_Options {
  readonly address: string
}

/** The private native operation surface used by the TypeScript facade. */
export interface Native_Client {
  get(key: Uint8Array): Promise<Uint8Array | null>
  set(key: Uint8Array, value: Uint8Array): Promise<string>
  delete(key: Uint8Array): Promise<boolean>
  close(): Promise<void>
  close_now(): void
}

interface Native_Module {
  connect(options: Native_Client_Options): Promise<Native_Client>
}

const REQUIRE = createRequire(import.meta.url)

/**
 * Loads the packaged Node-API adapter for the current platform.
 *
 * The loader remains private so callers cannot select a transport, trust
 * policy, certificate, retry policy, or native artifact through the Gate 0
 * facade.
 *
 * @returns The private native module.
 * @throws {Error} When the platform is unsupported or the addon cannot load.
 */
export function load_native_module(): Native_Module {
  return REQUIRE(default_native_path()) as Native_Module
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
        `got ${process.platform} ${process.arch}`,
    )
  }
  return fileURLToPath(
    new URL(`../target/native/${artifact_name}`, import.meta.url),
  )
}
