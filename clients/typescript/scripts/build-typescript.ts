#!/usr/bin/env bun
/** Removes stale generated output and compiles the TypeScript package. */

import { rmSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

const SCRIPT_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const CLIENT_DIRECTORY = dirname(SCRIPT_DIRECTORY)
const OUTPUT_DIRECTORY = join(CLIENT_DIRECTORY, "dist")
const PROTOCOL_GENERATOR = join(
  CLIENT_DIRECTORY,
  "..",
  "..",
  "protocol",
  "generate.ts",
)
const TYPESCRIPT_CLI = join(
  CLIENT_DIRECTORY,
  "node_modules",
  "typescript",
  "bin",
  "tsc",
)

rmSync(OUTPUT_DIRECTORY, { force: true, recursive: true })
const generation = Bun.spawnSync([process.execPath, PROTOCOL_GENERATOR], {
  env: {
    ...process.env,
    OPENKACHE_GENERATION_TARGET: "typescript",
  },
  stderr: "inherit",
  stdout: "inherit",
})
if (generation.exitCode !== 0) {
  throw new Error(
    `Smithy API generation failed with exit code ${generation.exitCode}. ` +
      "Install Smithy CLI and ensure it is on PATH before building.",
  )
}
const build = Bun.spawnSync([
  process.execPath,
  TYPESCRIPT_CLI,
  "--project",
  join(CLIENT_DIRECTORY, "tsconfig.json"),
], {
  cwd: CLIENT_DIRECTORY,
  stdout: "inherit",
  stderr: "inherit",
})
if (build.exitCode !== 0) {
  throw new Error(
    `TypeScript build failed with exit code ${build.exitCode}. ` +
      "Run bun install --frozen-lockfile before building.",
  )
}
