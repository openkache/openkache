#!/usr/bin/env bun
/** Validates the complete TypeScript package before a registry publication. */

import { existsSync, readFileSync, statSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

const SCRIPT_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const CLIENT_DIRECTORY = dirname(SCRIPT_DIRECTORY)
const PACKAGE_MANIFEST_PATH = join(CLIENT_DIRECTORY, "package.json")
const REGISTRY_URL = "https://registry.npmjs.org"
const EXPECTED_PACKAGE_NAME = "openkache"
const EXPECTED_FILES = [
  "dist/index.js",
  "dist/index.d.ts",
  "dist/native-binding.js",
  "dist/native-binding.d.ts",
  "dist/gate0-contract.js",
  "dist/gate0-contract.d.ts",
  "dist/value-codec.js",
  "dist/value-codec.d.ts",
  "dist/generated_local/smithy-api.js",
  "dist/generated_local/smithy-api.d.ts",
  "target/native/openkache-client.linux-x64-gnu.node",
  "target/native/openkache-client.linux-arm64-gnu.node",
  "target/native/openkache-client.darwin-arm64.node",
  "LICENSE",
  "README.md",
] as const

type Package_Manifest = Readonly<Record<string, unknown>>

function fail(message: string): never {
  console.error(`ERROR: ${message}`)
  process.exit(1)
}

function read_manifest(): Package_Manifest {
  let manifest_text: string
  try {
    manifest_text = readFileSync(PACKAGE_MANIFEST_PATH, "utf8")
  } catch (error) {
    fail(
      `Could not read ${PACKAGE_MANIFEST_PATH}: ${error instanceof Error ? error.message : String(error)}.\n` +
        "Why: the release check must inspect the exact package manifest being published.\n" +
        "Fix: run this command from clients/typescript or use `bun run release:check`.",
    )
  }

  try {
    return JSON.parse(manifest_text) as Package_Manifest
  } catch (error) {
    fail(
      `Could not parse ${PACKAGE_MANIFEST_PATH}: ${error instanceof Error ? error.message : String(error)}.\n` +
        "Why: registry metadata is read from package.json and must be valid JSON.\n" +
        "Fix: repair package.json, then run `bun pm pkg fix` and repeat the check.",
    )
  }
}

function require_string(
  manifest: Package_Manifest,
  key: string,
): string {
  const value = manifest[key]
  if (typeof value !== "string" || value.length === 0) {
    fail(
      `package.json field "${key}" must be a non-empty string.\n` +
        "Why: the registry release metadata is incomplete.\n" +
        "Fix: set the field in package.json before publishing.",
    )
  }
  return value
}

function require_public_access(manifest: Package_Manifest): void {
  const publish_config = manifest.publishConfig
  if (
    typeof publish_config !== "object" ||
    publish_config === null ||
    (publish_config as Record<string, unknown>).access !== "public"
  ) {
    fail(
      'package.json must set publishConfig.access to "public".\n' +
        "Why: openkache is the public package and must not be published privately.\n" +
        'Fix: add `"publishConfig": { "access": "public" }`.',
    )
  }
}

function require_packaged_files(manifest: Package_Manifest): void {
  const files = manifest.files
  if (!Array.isArray(files)) {
    fail(
      "package.json must declare a files array.\n" +
        "Why: the release must be limited to reviewed runtime and documentation files.\n" +
        'Fix: add the package output paths to the manifest "files" array.',
    )
  }

  const declared_files = new Set(files.filter((file): file is string => typeof file === "string"))
  const missing_declarations = EXPECTED_FILES.filter((file) => {
    if (declared_files.has(file)) return false
    const directory = file.split("/")[0]
    return !declared_files.has(directory)
  })
  if (missing_declarations.length > 0) {
    fail(
      `package.json files is missing: ${missing_declarations.join(", ")}.\n` +
        "Why: the published tarball must contain declarations, JavaScript, and every supported native adapter.\n" +
        'Fix: add the missing paths to package.json "files".',
    )
  }
}

function require_stable_version(version: string): void {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u.test(version)) {
    fail(
      `Version "${version}" is not a stable SemVer release.\n` +
        "Why: this workflow reserves the latest dist-tag for complete stable packages.\n" +
        "Fix: set package.json version to a three-part version such as 0.1.0.",
    )
  }
}

function require_clean_worktree(): void {
  const status = Bun.spawnSync(
    ["git", "status", "--porcelain=v1", "--untracked-files=no"],
    {
      cwd: CLIENT_DIRECTORY,
      stderr: "pipe",
      stdout: "pipe",
    },
  )
  if (status.exitCode !== 0) {
    fail(
      "Could not inspect the public repository worktree.\n" +
        "Why: publishing from an unknown Git state makes the registry artifact unreproducible.\n" +
        "Fix: run the release check from a Git checkout with Git available.",
    )
  }
  const changes = status.stdout.toString().trim()
  if (changes.length > 0) {
    fail(
      `The public repository has tracked changes:\n${changes}\n` +
        "Why: a published package must correspond to one committed source snapshot.\n" +
        "Fix: commit and merge the package changes, then rerun the release from that clean checkout.",
    )
  }
}

function require_build_outputs(): void {
  const missing_outputs: string[] = []
  for (const relative_path of EXPECTED_FILES) {
    const file_path = join(CLIENT_DIRECTORY, relative_path)
    if (!existsSync(file_path) || statSync(file_path).size === 0) {
      missing_outputs.push(relative_path)
    }
  }
  if (missing_outputs.length > 0) {
    fail(
      `Required package outputs are missing or empty: ${missing_outputs.join(", ")}.\n` +
        "Why: a partial native build would publish a package that fails on a supported platform.\n" +
        "Fix: build Linux artifacts on Linux and the Darwin artifact on Apple Silicon, stage all outputs under target/native, then rerun `bun run release:check`.",
    )
  }
}

function require_release_tag(version: string): void {
  if (process.env.GITHUB_REF_TYPE !== "tag") return
  const expected_tag = `typescript-v${version}`
  if (process.env.GITHUB_REF_NAME !== expected_tag) {
    fail(
      `GitHub tag "${process.env.GITHUB_REF_NAME ?? "unknown"}" does not match ${expected_tag}.\n` +
        "Why: the release tag must identify the exact package version in package.json.\n" +
        `Fix: create and push tag ${expected_tag} from the committed public main snapshot.`,
    )
  }
}

function require_publish_source(version: string): void {
  if (process.env.OPENKACHE_PUBLISH_AUTH !== "1") return
  if (process.env.GITHUB_REF_TYPE === "tag") {
    require_release_tag(version)
    return
  }

  const branch = Bun.spawnSync(["git", "branch", "--show-current"], {
    cwd: CLIENT_DIRECTORY,
    stderr: "pipe",
    stdout: "pipe",
  })
  const branch_name = branch.stdout.toString().trim()
  if (branch.exitCode !== 0 || branch_name !== "main") {
    fail(
      `Authenticated publication must run from public main, got "${branch_name || "detached HEAD"}".\n` +
        "Why: a local release must correspond to the merged public source snapshot rather than a feature branch.\n" +
        "Fix: merge the release commit, switch to public main, and rerun `bun run release:publish`; CI may publish the matching typescript-v<version> tag.",
    )
  }
}

function require_trusted_publishing_oidc(): void {
  if (
    process.env.ACTIONS_ID_TOKEN_REQUEST_URL === undefined ||
    process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN === undefined
  ) {
    fail(
      "GitHub Actions OIDC authentication is unavailable.\n" +
        "Why: npm Trusted Publishing requires an OIDC token minted with the publish job's id-token: write permission.\n" +
        "Fix: grant id-token: write to this publish job and run it on a GitHub-hosted runner.",
    )
  }
}

async function require_registry_version_is_free(
  package_name: string,
  version: string,
): Promise<void> {
  const package_url = `${REGISTRY_URL}/${encodeURIComponent(package_name)}/${version}`
  let response: Response
  try {
    response = await fetch(package_url, {
      headers: { accept: "application/json" },
    })
  } catch (error) {
    fail(
      `Could not query the package registry: ${error instanceof Error ? error.message : String(error)}.\n` +
        "Why: the release check must prevent an accidental republish and cannot prove that the version is unused offline.\n" +
        "Fix: restore registry access and rerun the check; do not bypass this step.",
    )
  }

  if (response.status === 404) return
  if (response.ok) {
    fail(
      `${package_name}@${version} already exists on the package registry.\n` +
        "Why: registry versions are immutable and a second publication cannot replace the existing artifact.\n" +
        "Fix: choose the next version in package.json or verify the intended release before continuing.",
    )
  }
  fail(
    `The package registry returned HTTP ${response.status} for ${package_name}@${version}.\n` +
      "Why: the release check could not prove that the version is available.\n" +
      "Fix: investigate registry access and rerun the check; do not publish on an inconclusive response.",
  )
}

async function main(): Promise<void> {
  const manifest = read_manifest()
  const package_name = require_string(manifest, "name")
  const version = require_string(manifest, "version")
  if (package_name !== EXPECTED_PACKAGE_NAME) {
    fail(
      `package.json name is "${package_name}", expected "${EXPECTED_PACKAGE_NAME}".\n` +
        "Why: the release command targets the OpenKache TypeScript package.\n" +
        "Fix: correct the package name or use a package-specific release workflow.",
    )
  }

  require_stable_version(version)
  require_public_access(manifest)
  require_packaged_files(manifest)
  require_clean_worktree()
  require_release_tag(version)
  require_publish_source(version)
  require_build_outputs()
  await require_registry_version_is_free(package_name, version)

  if (process.env.OPENKACHE_PUBLISH_AUTH === "1") {
    if (process.env.OPENKACHE_TRUSTED_PUBLISHING === "1") {
      require_trusted_publishing_oidc()
    } else {
      const identity = Bun.spawnSync([process.execPath, "pm", "whoami"], {
        cwd: CLIENT_DIRECTORY,
        stderr: "pipe",
        stdout: "pipe",
      })
      if (identity.exitCode !== 0) {
        fail(
          "Registry authentication is not configured.\n" +
            "Why: release:publish must prove the registry identity before it can mutate the registry.\n" +
            "Fix: configure NPM_CONFIG_TOKEN or a user .npmrc, then verify with `bun pm whoami`.",
        )
      }
      console.log(`Registry identity verified: ${identity.stdout.toString().trim()}`)
    }
  }

  console.log(`Release preflight passed for ${package_name}@${version}.`)
}

await main()
