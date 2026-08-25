#!/usr/bin/env bun
/** Smoke-tests the staged package archive and the public close retry contract. */

import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"
import { mock } from "bun:test"

const SCRIPT_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const CLIENT_DIRECTORY = dirname(SCRIPT_DIRECTORY)
const PACKAGE_MANIFEST_PATH = join(CLIENT_DIRECTORY, "package.json")
const EXPECTED_ARCHIVE_ENTRIES = [
  "package/dist/index.js",
  "package/dist/index.d.ts",
  "package/dist/native-binding.js",
  "package/target/native/openkache-client.linux-x64-gnu.node",
  "package/target/native/openkache-client.linux-arm64-gnu.node",
  "package/target/native/openkache-client.darwin-arm64.node",
  "package/LICENSE",
  "package/README.md",
] as const

function fail(message: string): never {
  throw new Error(
    `Release smoke failed: ${message}\n` +
      "Why: the published archive must contain the reviewed package outputs and " +
      "retain the documented close retry behavior.\n" +
      "Fix: rebuild the package, inspect the tarball contents, and rerun " +
      "`bun run release:smoke` before creating a release tag.",
  )
}

function run(command: readonly string[], cwd: string): string {
  const result = Bun.spawnSync(command, {
    cwd,
    stderr: "pipe",
    stdout: "pipe",
  })
  if (result.exitCode !== 0) {
    fail(
      `${command.join(" ")} exited with ${result.exitCode}: ` +
        result.stderr.toString().trim(),
    )
  }
  return result.stdout.toString()
}

async function main(): Promise<void> {
  const manifest = JSON.parse(await readFile(PACKAGE_MANIFEST_PATH, "utf8")) as {
    readonly version?: unknown
  }
  if (typeof manifest.version !== "string" || manifest.version.length === 0) {
    fail("package.json does not contain a release version")
  }

  const temporary_directory = await mkdtemp(
    join(tmpdir(), "openkache-typescript-release-smoke-"),
  )
  try {
    const archive_path = join(
      temporary_directory,
      `openkache-${manifest.version}.tgz`,
    )
    run(
      [
        process.execPath,
        "pm",
        "pack",
        "--filename",
        archive_path,
        "--ignore-scripts",
        "--quiet",
      ],
      CLIENT_DIRECTORY,
    )
    const archive_entries = new Set(
      run(["tar", "-tzf", archive_path], temporary_directory)
        .trim()
        .split("\n")
        .filter((entry) => entry.length > 0),
    )
    for (const expected_entry of EXPECTED_ARCHIVE_ENTRIES) {
      if (!archive_entries.has(expected_entry)) {
        fail(`package archive is missing ${expected_entry}`)
      }
    }

    const extracted_directory = join(temporary_directory, "extracted")
    run(["mkdir", "-p", extracted_directory], temporary_directory)
    run(
      ["tar", "-xzf", archive_path, "-C", extracted_directory],
      temporary_directory,
    )

    const native_binding_path = join(
      extracted_directory,
      "package",
      "dist",
      "native-binding.js",
    )
    let close_attempts = 0
    const native_client = {
      get: async (): Promise<null> => null,
      set: async (): Promise<string> => "created",
      delete: async (): Promise<boolean> => false,
      close: async (): Promise<void> => {
        close_attempts += 1
        if (close_attempts === 1) {
          throw new Error("simulated native shutdown failure")
        }
      },
      close_now: (): void => {},
    }
    mock.module(native_binding_path, () => ({
      load_native_module: (): {
        connect: () => Promise<typeof native_client>
      } => ({
        connect: async (): Promise<typeof native_client> => native_client,
      }),
    }))

    const package_entry_path = join(
      extracted_directory,
      "package",
      "dist",
      "index.js",
    )
    const package_module = (await import(
      pathToFileURL(package_entry_path).href
    )) as typeof import("../src/index.js")
    const client = await package_module.OpenKache_Client.connect("127.0.0.1:4433")
    await assert.rejects(client.close(), /simulated native shutdown failure/u)
    if (close_attempts !== 1) {
      fail(`expected one failed close attempt, got ${close_attempts}`)
    }
    await client.close()
    const attempts_after_retry = close_attempts
    if (attempts_after_retry !== 2) {
      fail(
        "expected the second close call to retry native shutdown, got " +
          attempts_after_retry,
      )
    }
    await assert.rejects(client.get("closed"), /client is closed/u)
  } finally {
    await rm(temporary_directory, { force: true, recursive: true })
  }

  console.log(`Release smoke passed for openkache@${manifest.version}.`)
}

await main()
