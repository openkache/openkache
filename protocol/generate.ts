#!/usr/bin/env bun
/** Generates the server-visible Rust wire contract from `protocol/model`. */

import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { basename, dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import {
  extract_wire_contract,
  render_rust_wire,
  render_rust_semantic_constants,
  render_rust_server_contract,
  render_protocol_spec_operation_table,
  smithy_wire_ast,
  type Wire_Contract,
} from "./wire"

export {
  extract_wire_contract,
  render_rust_semantic_constants,
  render_rust_server_contract,
  render_rust_wire,
  render_protocol_spec_operation_table,
  smithy_wire_ast,
} from "./wire"
export type {
  Wire_Contract,
  Wire_Entry,
  Wire_Operation_Field,
  Wire_V1_Contract,
} from "./wire"

const PROTOCOL_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const PUBLIC_ROOT = dirname(PROTOCOL_DIRECTORY)
const GENERATED_OUTPUT_ROOT = resolve(
  process.env.OPENKACHE_GENERATION_OUTPUT_ROOT ?? PUBLIC_ROOT,
)
const GENERATED_WIRE_OUTPUT = process.env.OPENKACHE_RUST_WIRE_OUTPUT ??
  join(GENERATED_OUTPUT_ROOT, "protocol/generated_local/wire_values.rs")
const GENERATED_SERVER_OUTPUT = process.env.OPENKACHE_RUST_SERVER_OUTPUT ??
  join(GENERATED_OUTPUT_ROOT, "server/generated_local/server_contract.rs")

/** Returns generated outputs that are missing or differ from the wire contract. */
export function generated_output_issues(
  outputs: Readonly<Record<string, string>>,
): readonly string[] {
  const mismatches: string[] = []
  for (const [output_path, content] of Object.entries(outputs)) {
    let existing: string
    try {
      existing = readFileSync(output_path, "utf8")
    } catch {
      mismatches.push(`${output_path} (missing)`)
      continue
    }
    if (existing !== content) mismatches.push(output_path)
  }
  return mismatches
}

function write_output(output_path: string, content: string, check_only: boolean): void {
  if (check_only) {
    const mismatches = generated_output_issues({ [output_path]: content })
    if (mismatches.length > 0) {
      throw new Error(
        "generated wire output is stale:\n" +
          mismatches.map((path) => `  - ${path}`).join("\n") +
          "\nRun `./generate.ts` from the protocol directory to regenerate it.",
      )
    }
    return
  }

  const output_directory = dirname(output_path)
  mkdirSync(output_directory, { recursive: true })
  const temporary_directory = mkdtempSync(join(output_directory, "generate.local."))
  const temporary_path = join(temporary_directory, basename(output_path))
  try {
    writeFileSync(temporary_path, content)
    renameSync(temporary_path, output_path)
    console.log(`Generated ${output_path}`)
  } finally {
    rmSync(temporary_directory, { force: true, recursive: true })
  }
}

/** Runs the protocol wire-contract generator.
 *
 * @returns Process exit code.
 */
export function main(): number {
  try {
    const target = process.env.OPENKACHE_GENERATION_TARGET
    if (target !== undefined && target !== "rust-wire" && target !== "rust-server") {
      throw new Error(
        "the protocol entry point emits only Rust wire or server contracts; " +
          "run `../clients/generate.ts` for client language or ABI outputs",
      )
    }
    const contract: Wire_Contract = extract_wire_contract(smithy_wire_ast(), true)
    const check_only = process.env.OPENKACHE_GENERATION_CHECK === "1"
    if (target === "rust-server") {
      write_output(GENERATED_SERVER_OUTPUT, render_rust_server_contract(contract), check_only)
    } else {
      write_output(GENERATED_WIRE_OUTPUT, render_rust_wire(contract), check_only)
    }
    return 0
  } catch (error) {
    console.error(
      `GENERATION_FAILED: ${error instanceof Error ? error.message : String(error)}\n` +
        "  Why: server framing values must come from a complete, valid wire Smithy contract.\n" +
        "  Fix: Run `smithy validate model` from the protocol directory, correct the model or generator error, then rerun `./generate.ts`.",
    )
    return 1
  }
}

if (import.meta.main) process.exit(main())
