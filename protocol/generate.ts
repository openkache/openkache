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
  extract_wire_contract as extract_generic_wire_contract,
  render_rust_operation_contract,
  render_rust_wire,
  render_rust_server_contract,
  smithy_wire_ast,
  type Wire_Contract,
} from "./wire"
import {
  render_rust_compatibility_contract,
  render_rust_semantic_constants,
} from "./compatibility_v1_renderer"
import {
  derive_wire_compatibility_response_semantics,
  derive_wire_compatibility_retry_mode,
  derive_wire_compatibility_scope,
  extract_compatibility_wire_contract,
  OPTIONAL_VALUES_RESPONSE_FRAMING,
  type Wire_Compatibility_Contract,
} from "./compatibility_v1"
import {
  render_protocol_spec_contract_snapshot as render_protocol_spec_contract_snapshot_generic,
  render_protocol_spec_operation_table as render_protocol_spec_operation_table_generic,
  protocol_spec_contract_snapshot_issues as protocol_spec_contract_snapshot_issues_generic,
  protocol_spec_operation_table_issues as protocol_spec_operation_table_issues_generic,
  PROTOCOL_SPEC_CONTRACT_SNAPSHOT_END,
  PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START,
  PROTOCOL_SPEC_OPERATION_TABLE_END,
  PROTOCOL_SPEC_OPERATION_TABLE_START,
  type Wire_Spec_Adapter,
} from "./wire_spec"

export {
  render_rust_operation_contract,
  render_rust_server_contract,
  render_rust_wire,
} from "./wire"
export {
  render_rust_compatibility_contract,
  render_rust_semantic_constants,
} from "./compatibility_v1_renderer"
export { extract_generic_wire_contract }
export {
  PROTOCOL_SPEC_CONTRACT_SNAPSHOT_END,
  PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START,
  PROTOCOL_SPEC_OPERATION_TABLE_END,
  PROTOCOL_SPEC_OPERATION_TABLE_START,
} from "./wire_spec"
export { smithy_wire_ast } from "./wire"
export type {
  Wire_Contract,
  Wire_Entry,
  Wire_V1_Contract,
} from "./wire"
export type { Wire_Compatibility_Contract } from "./compatibility_v1"

/**
 * Reader-facing metadata for the checked-in protocol-v1 specification.
 *
 * The generic renderer remains unaware of route names, retry policy, and
 * semantic result labels. The protocol generation entry point supplies those
 * compatibility details explicitly when it renders SPEC.md.
 */
const PROTOCOL_V1_SPEC_ADAPTER: Wire_Spec_Adapter = {
  response_payload(operation): string | undefined {
    if (
      operation.contract.response_framing !== OPTIONAL_VALUES_RESPONSE_FRAMING
    ) return undefined
    const value_count = operation.contract.response_plan?.length ?? 0
    return value_count === 1
      ? "one ordered optional field"
      : `${value_count} ordered optional fields`
  },
  scope: (operation) => derive_wire_compatibility_scope(operation.contract),
  retry_mode: (operation) => derive_wire_compatibility_retry_mode(operation.contract),
  response_semantics: (operation) =>
    derive_wire_compatibility_response_semantics(operation.contract),
}

export function render_protocol_spec_operation_table(contract: Wire_Contract): string {
  return render_protocol_spec_operation_table_generic(contract, PROTOCOL_V1_SPEC_ADAPTER)
}

export function protocol_spec_operation_table_issues(
  spec: string,
  contract: Wire_Contract,
): readonly string[] {
  return protocol_spec_operation_table_issues_generic(
    spec,
    contract,
    PROTOCOL_V1_SPEC_ADAPTER,
  )
}

export function render_protocol_spec_contract_snapshot(contract: Wire_Contract): string {
  return render_protocol_spec_contract_snapshot_generic(
    contract,
    PROTOCOL_V1_SPEC_ADAPTER,
  )
}

export function protocol_spec_contract_snapshot_issues(
  spec: string,
  contract: Wire_Contract,
): readonly string[] {
  return protocol_spec_contract_snapshot_issues_generic(
    spec,
    contract,
    PROTOCOL_V1_SPEC_ADAPTER,
  )
}

/**
 * Extracts the checked-in protocol contract with its explicit v1 adapter.
 *
 * The public generation entry point remains compatible with existing callers,
 * while the generic extractor is available from `wire.ts` for future
 * protocols that provide a different adapter (or no compatibility projection).
 */
export function extract_wire_contract(
  ast: unknown,
  strict_operations = false,
): Wire_Compatibility_Contract {
  return extract_compatibility_wire_contract(ast, strict_operations)
}

const PROTOCOL_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const PUBLIC_ROOT = dirname(PROTOCOL_DIRECTORY)
const GENERATED_OUTPUT_ROOT = resolve(
  process.env.OPENKACHE_GENERATION_OUTPUT_ROOT ?? PUBLIC_ROOT,
)
const GENERATED_WIRE_OUTPUT = process.env.OPENKACHE_RUST_WIRE_OUTPUT ??
  join(GENERATED_OUTPUT_ROOT, "protocol/generated_local/wire_values.rs")
const GENERATED_OPERATION_OUTPUT = process.env.OPENKACHE_RUST_OPERATION_OUTPUT ??
  join(GENERATED_OUTPUT_ROOT, "protocol/generated_local/operation_contract.rs")
const GENERATED_COMPATIBILITY_OUTPUT = process.env.OPENKACHE_RUST_COMPATIBILITY_OUTPUT ??
  join(GENERATED_OUTPUT_ROOT, "protocol/generated_local/operation_compatibility.rs")
const GENERATED_SERVER_OUTPUT = process.env.OPENKACHE_RUST_SERVER_OUTPUT ??
  join(GENERATED_OUTPUT_ROOT, "server/generated_local/server_contract.rs")
const SPEC_OUTPUT = join(PROTOCOL_DIRECTORY, "SPEC.md")

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
  const with_table = replace_section(
    existing,
    PROTOCOL_SPEC_OPERATION_TABLE_START,
    PROTOCOL_SPEC_OPERATION_TABLE_END,
    render_protocol_spec_operation_table(contract),
  )
  const updated = replace_section(
    with_table,
    PROTOCOL_SPEC_CONTRACT_SNAPSHOT_START,
    PROTOCOL_SPEC_CONTRACT_SNAPSHOT_END,
    render_protocol_spec_contract_snapshot(contract),
  )
  if (updated !== existing) writeFileSync(SPEC_OUTPUT, updated)
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
    const contract: Wire_Compatibility_Contract =
      extract_compatibility_wire_contract(smithy_wire_ast(), true)
    const check_only = process.env.OPENKACHE_GENERATION_CHECK === "1"
    // Build-script generation must be hermetic: Bazel and Cargo provide only
    // the declared model/generator inputs, not the checked-in documentation.
    // Refresh the human-readable SPEC only from an explicit repository-level
    // generation command.
    if (process.env.OPENKACHE_GENERATION_UPDATE_SPEC === "1") {
      update_protocol_spec(contract)
    }
    if (check_only) {
      const spec_issues = protocol_spec_operation_table_issues(
        readFileSync(SPEC_OUTPUT, "utf8"),
        contract,
      )
      const snapshot_issues = protocol_spec_contract_snapshot_issues(
        readFileSync(SPEC_OUTPUT, "utf8"),
        contract,
      )
      if (spec_issues.length > 0 || snapshot_issues.length > 0) {
        throw new Error(
          "generated protocol documentation is stale:\n" +
            [...spec_issues, ...snapshot_issues]
              .map((path) => `  - ${path}`)
              .join("\n") +
            "\nUpdate the marked generated contract blocks in protocol/SPEC.md.",
        )
      }
    }
    if (target === "rust-server") {
      write_output(GENERATED_SERVER_OUTPUT, render_rust_server_contract(contract), check_only)
    } else {
      write_output(GENERATED_WIRE_OUTPUT, render_rust_wire(contract), check_only)
      write_output(
        GENERATED_OPERATION_OUTPUT,
        render_rust_operation_contract(contract, "crate::codec::CodecKind"),
        check_only,
      )
      write_output(
        GENERATED_COMPATIBILITY_OUTPUT,
        render_rust_compatibility_contract(contract),
        check_only,
      )
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
