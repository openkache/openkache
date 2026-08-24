#!/usr/bin/env bun
/** Runs the authenticated, non-interactive TypeScript package publication. */

import { dirname } from "node:path"
import { fileURLToPath } from "node:url"

const CLIENT_DIRECTORY = dirname(dirname(fileURLToPath(import.meta.url)))
const release_environment = {
  ...process.env,
  OPENKACHE_PUBLISH_AUTH: "1",
}
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

const publish = Bun.spawnSync([
  process.execPath,
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
