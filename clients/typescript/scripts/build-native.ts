#!/usr/bin/env bun
/** Builds the supported Linux Node-API adapters and stages their package artifacts. */

import { copyFileSync, mkdirSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

const SCRIPT_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const CLIENT_DIRECTORY = dirname(SCRIPT_DIRECTORY)
const BUILD_DIRECTORY = join(CLIENT_DIRECTORY, "target", "native-build")
const OUTPUT_DIRECTORY = join(CLIENT_DIRECTORY, "target", "native")
const BUILD_TARGETS = [
  {
    target_triple: "x86_64-unknown-linux-gnu",
    cargo_target: "x86_64-unknown-linux-gnu.2.17",
    artifact_name: "openkache-client.linux-x64-gnu.node",
  },
  {
    target_triple: "aarch64-unknown-linux-gnu",
    cargo_target: "aarch64-unknown-linux-gnu.2.17",
    artifact_name: "openkache-client.linux-arm64-gnu.node",
  },
] as const

const cargo_environment = { ...process.env }
delete cargo_environment.CARGO_BUILD_TARGET
mkdirSync(OUTPUT_DIRECTORY, { recursive: true })
for (const build_target of BUILD_TARGETS) {
  const build = Bun.spawnSync([
    "cargo",
    "zigbuild",
    "--locked",
    "--manifest-path",
    join(CLIENT_DIRECTORY, "native", "Cargo.toml"),
    "--release",
    "--target",
    build_target.cargo_target,
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
      `Native adapter build failed for ${build_target.cargo_target} with exit code ` +
        `${build.exitCode}. Run this command inside the repository Nix development shell.`,
    )
  }

  const source_path = join(
    BUILD_DIRECTORY,
    build_target.target_triple,
    "release",
    "libopenkache_client_napi.so",
  )
  copyFileSync(
    source_path,
    join(OUTPUT_DIRECTORY, build_target.artifact_name),
  )
  console.log(`Staged ${build_target.artifact_name}`)
}
