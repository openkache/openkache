#!/usr/bin/env bun
/** Generates the transport-only Rust wire contract from `protocol/model`. */

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
  smithy_wire_ast,
  type Wire_Contract,
} from "./wire"
import {
  render_protocol_spec_contract_snapshot,
  protocol_spec_contract_snapshot_issues,
  PROTOCOL_SPEC_CONTRACT_SNAPSHOT_END,
  PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START,
} from "./wire_spec"

export { extract_wire_contract, render_rust_wire, smithy_wire_ast } from "./wire"
export type { Wire_Contract, Wire_V1_Contract } from "./wire"
export {
  render_protocol_spec_contract_snapshot,
  protocol_spec_contract_snapshot_issues,
  PROTOCOL_SPEC_CONTRACT_SNAPSHOT_END,
  PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START,
} from "./wire_spec"

const PROTOCOL_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const PUBLIC_ROOT = dirname(PROTOCOL_DIRECTORY)
const GENERATED_OUTPUT_ROOT = resolve(
  process.env.OPENKACHE_GENERATION_OUTPUT_ROOT ?? PUBLIC_ROOT,
)
const GENERATED_WIRE_OUTPUT = process.env.OPENKACHE_RUST_WIRE_OUTPUT ??
  join(GENERATED_OUTPUT_ROOT, "protocol/generated_local/wire_values.rs")
const SPEC_OUTPUT = join(PROTOCOL_DIRECTORY, "SPEC.md")

/** Returns generated outputs that are missing or differ from their source. */
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

function update_protocol_spec(contract: Wire_Contract): void {
  const existing = readFileSync(SPEC_OUTPUT, "utf8")
  const replace_section = (
    source: string,
    start_marker: string,
    end_marker: string,
    rendered: string,
  ): string => {
    const start = source.indexOf(start_marker)
    const end = source.indexOf(end_marker)
    if (start < 0 || end < start) {
      throw new Error(`protocol/SPEC.md is missing generated markers: ${start_marker}`)
    }
    return source.slice(0, start + start_marker.length) +
      `\n${rendered}\n` +
      source.slice(end)
  }
  const updated = replace_section(
    existing,
    PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START,
    PROTOCOL_SPEC_CONTRACT_SNAPSHOT_END,
    render_protocol_spec_contract_snapshot(contract),
  )
  if (updated !== existing) writeFileSync(SPEC_OUTPUT, updated)
}

/** Runs the transport-contract generator. */
export function main(): number {
  try {
    const target = process.env.OPENKACHE_GENERATION_TARGET
    if (target !== undefined && target !== "rust-wire") {
      throw new Error(
        "the protocol generator emits only the transport Rust wire contract; " +
          "request/response codecs and registrations are handwritten in their API modules",
      )
    }
    const contract = extract_wire_contract(smithy_wire_ast())
    const output = render_rust_wire(contract)
    const check_only = process.env.OPENKACHE_GENERATION_CHECK === "1"
    if (process.env.OPENKACHE_GENERATION_UPDATE_SPEC === "1") {
      update_protocol_spec(contract)
    }
    if (check_only) {
      const spec = readFileSync(SPEC_OUTPUT, "utf8")
      const issues = protocol_spec_contract_snapshot_issues(spec, contract)
      if (issues.length > 0) {
        throw new Error(
          "generated protocol documentation is stale:\n" +
            issues.map((issue) => `  - ${issue}`).join("\n") +
            "\nUpdate the marked transport contract block in protocol/SPEC.md.",
        )
      }
    }
    write_output(GENERATED_WIRE_OUTPUT, output, check_only)
    return 0
  } catch (error) {
    console.error(
      `GENERATION_FAILED: ${error instanceof Error ? error.message : String(error)}\n` +
        "  Why: the transport contract must be complete and valid.\n" +
        "  Fix: validate the Smithy model and rerun `./generate.ts`.",
    )
    return 1
  }
}

if (import.meta.main) process.exit(main())
