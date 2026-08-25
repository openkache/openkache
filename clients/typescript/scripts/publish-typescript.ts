#!/usr/bin/env bun
/** Runs the authenticated, non-interactive TypeScript package publication. */

import { dirname } from "node:path"
import { fileURLToPath } from "node:url"

const CLIENT_DIRECTORY = dirname(dirname(fileURLToPath(import.meta.url)))
const release_environment = {
  ...process.env,
  OPENKACHE_PUBLISH_AUTH: "1",
}
const trusted_publishing_enabled =
  process.env.OPENKACHE_TRUSTED_PUBLISHING === "1"
const release_check =
  process.env.OPENKACHE_RELEASE_ARTIFACTS_READY === "1"
    ? "release:verify"
    : "release:check"

const check = Bun.spawnSync([process.execPath, "run", release_check], {
  cwd: CLIENT_DIRECTORY,
  env: release_environment,
  stderr: "inherit",
  stdout: "inherit",
})
if (check.exitCode !== 0) {
  process.exit(check.exitCode ?? 1)
}

function require_publish_binary(): string {
  if (!trusted_publishing_enabled) return process.execPath
  const publish_binary = process.env.OPENKACHE_NPM_CLI
  if (publish_binary === undefined || publish_binary.length === 0) {
    console.error(
      "Trusted registry publishing is missing its configured CLI.\n" +
        "Why: the release job must provide the OIDC-aware registry client.\n" +
        "Fix: set OPENKACHE_NPM_CLI in the publish job.",
    )
    process.exit(1)
  }
  return publish_binary
}

const publish_binary = require_publish_binary()
const publish = Bun.spawnSync([
  publish_binary,
  "publish",
  "--access",
  "public",
  "--tag",
  "latest",
  "--ignore-scripts",
], {
  cwd: CLIENT_DIRECTORY,
  env: release_environment,
  stderr: "inherit",
  stdout: "inherit",
})
if (publish.exitCode !== 0) {
  process.exit(publish.exitCode ?? 1)
}
