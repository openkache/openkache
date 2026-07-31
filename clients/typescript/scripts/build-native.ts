#!/usr/bin/env bun
/** Builds the host's supported Node-API adapters and stages package artifacts. */

import { copyFileSync, mkdirSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

const SCRIPT_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const CLIENT_DIRECTORY = dirname(SCRIPT_DIRECTORY)
const BUILD_DIRECTORY = join(CLIENT_DIRECTORY, "target", "native-build")
const OUTPUT_DIRECTORY = join(CLIENT_DIRECTORY, "target", "native")
interface Build_Target {
  readonly target_triple: string
  readonly artifact_name: string
  readonly library_name: string
  readonly command: readonly string[]
}

const LINUX_BUILD_TARGETS: readonly Build_Target[] = [
  {
    target_triple: "x86_64-unknown-linux-gnu",
    artifact_name: "openkache-client.linux-x64-gnu.node",
    library_name: "libopenkache_client_napi.so",
    command: ["cargo", "zigbuild", "--target", "x86_64-unknown-linux-gnu.2.17"],
  },
  {
    target_triple: "aarch64-unknown-linux-gnu",
    artifact_name: "openkache-client.linux-arm64-gnu.node",
    library_name: "libopenkache_client_napi.so",
    command: ["cargo", "zigbuild", "--target", "aarch64-unknown-linux-gnu.2.17"],
  },
] as const

const DARWIN_BUILD_TARGETS: readonly Build_Target[] = [
  {
    target_triple: "aarch64-apple-darwin",
    artifact_name: "openkache-client.darwin-arm64.node",
    library_name: "libopenkache_client_napi.dylib",
    command: ["cargo", "build", "--target", "aarch64-apple-darwin"],
  },
] as const

function host_build_targets(): readonly Build_Target[] {
  if (process.platform === "linux") return LINUX_BUILD_TARGETS
  if (process.platform === "darwin" && process.arch === "arm64") {
    return DARWIN_BUILD_TARGETS
  }
  throw new Error(
    `Native adapter builds support Linux x64/ARM64 and Apple Silicon macOS, ` +
      `got ${process.platform} ${process.arch}.`,
  )
}

const cargo_environment = { ...process.env }
delete cargo_environment.CARGO_BUILD_TARGET
mkdirSync(OUTPUT_DIRECTORY, { recursive: true })
for (const build_target of host_build_targets()) {
  const build = Bun.spawnSync([
    ...build_target.command,
    "--locked",
    "--manifest-path",
    join(CLIENT_DIRECTORY, "native", "Cargo.toml"),
    "--release",
    "--target-dir",
    BUILD_DIRECTORY,
  ], {
    cwd: CLIENT_DIRECTORY,
    env: cargo_environment,
    stdout: "inherit",
    stderr: "inherit",
  })
  if (build.exitCode !== 0) {
    throw new Error(
      `Native adapter build failed for ${build_target.target_triple} with exit code ` +
        `${build.exitCode}. Run this command inside the repository Nix development shell.`,
    )
  }

  const source_path = join(
    BUILD_DIRECTORY,
    build_target.target_triple,
    "release",
    build_target.library_name,
  )
  copyFileSync(
    source_path,
    join(OUTPUT_DIRECTORY, build_target.artifact_name),
  )
  console.log(`Staged ${build_target.artifact_name}`)
}
