#!/usr/bin/env bun
/** Generates client-owned Smithy contracts and their generated language bindings. */

import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { basename, dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import {
  render_rust_wire as render_protocol_rust_wire,
  type Wire_Contract,
} from "../protocol/wire"
import {
  PROTOCOL_V1_WIRE_ADAPTER,
  extract_compatibility_wire_contract as extract_protocol_wire_contract,
} from "../protocol/compatibility_v1"
export type {
  Api_Contract,
  Api_Enum,
  Api_Enum_Member,
  Api_Member,
  Api_Operation,
  Api_Operation_Contract,
  Api_Structure,
  Api_Type,
  Api_Type_Kind,
  Operation_Field_Role,
} from "./operation_models"
import {
  extract_client_contract,
  type Client_Contract,
} from "./client_contract"
export { extract_client_contract } from "./client_contract"
export type {
  Client_Contract,
  Client_Defaults_Contract,
  Ffi_Contract,
  Ffi_Entry,
  Ffi_Operation_Contract,
  Ffi_Input_Kind,
  Namespace_Descriptor_Field,
  Namespace_Descriptor_Layout,
  Native_Abi_Function,
  Native_Abi_Parameter,
  Native_Abi_Ownership,
  Native_Abi_Lifetime,
  Native_Abi_Structure,
  Native_Abi_Type,
  Value_Envelope_Contract,
  Value_Format_Contract,
} from "./client_contract"
import {
  render_csharp_native_abi,
  render_dart_native_api,
  render_go_native_abi,
  render_java_native_api,
  render_java_native_connect_options,
  render_java_native_descriptor,
  render_java_native_structure,
  native_abi_structure_class_name,
  render_kotlin_native_api,
  render_python_native_abi,
  render_swift_native_abi,
} from "./native_abi_renderers"
export {
  render_csharp_native_abi,
  render_dart_native_api,
  render_go_native_abi,
  render_java_native_api,
  render_java_native_connect_options,
  render_java_native_descriptor,
  render_java_native_structure,
  native_abi_structure_class_name,
  render_kotlin_native_api,
  render_python_native_abi,
  render_swift_native_abi,
} from "./native_abi_renderers"
import {
  render_dart_api,
  render_java_api,
  render_kotlin_api,
  render_typescript_api,
} from "./api_shape_renderers"
export {
  render_dart_api,
  render_java_api,
  render_kotlin_api,
  render_typescript_api,
} from "./api_shape_renderers"
export {
  derive_operation_plan,
  operation_field_count,
  operation_field_requirements,
  validate_operation_field_bindings,
} from "./operation_plans"
export type {
  Operation_Field_Requirement,
  Operation_Field_Plan,
  Operation_Plan,
  Operation_Shape_Plan,
} from "./operation_plans"
import {
  render_c_contract,
  render_rust_client,
  render_rust_operation_constants,
} from "./generator/native_contracts"
export {
  render_c_contract,
  render_rust,
  render_rust_client,
  render_rust_operation_constants,
} from "./generator/native_contracts"
export {
  WIRE_CODEC_REGISTRY,
  field_sequence_framing,
  optional_value_framing,
  type Field_Sequence_Framing,
  type Optional_Value_Framing,
  type Wire_Codec_Registration,
} from "./generator/managed"
import {
  render_dart_contract,
  render_dart_operations,
  render_java_contract,
  render_java_operations,
  render_kotlin_contract,
  render_kotlin_operations,
} from "./generator/renderers/jvm"
import {
  format_go_source,
  render_go_api,
  render_go_contract,
  render_go_operations,
} from "./generator/renderers/go"
export {
  render_go_api,
  render_go_contract,
  render_go_operations,
} from "./generator/renderers/go"
import {
  render_python_api,
  render_python_contract,
  render_python_operations,
} from "./generator/renderers/python"
export {
  render_python_api,
  render_python_contract,
  render_python_operations,
} from "./generator/renderers/python"
import {
  render_swift_api,
  render_swift_operations,
} from "./generator/renderers/swift"
export {
  render_swift_api,
  render_swift_operations,
} from "./generator/renderers/swift"
import {
  render_typescript_operations,
  render_typescript_value_envelope,
  render_typescript_value_format,
} from "./generator/renderers/typescript"
export {
  render_typescript_operations,
  render_typescript_value_envelope,
  render_typescript_value_format,
} from "./generator/renderers/typescript"
import {
  render_csharp,
  render_csharp_api,
  render_csharp_operations,
} from "./generator/renderers/dotnet"
export {
  render_csharp,
  render_csharp_api,
  render_csharp_operations,
} from "./generator/renderers/dotnet"
import {
  render_rust_api,
  render_rust_operations,
} from "./generator/renderers/rust"
export {
  render_rust_api,
  render_rust_operations,
} from "./generator/renderers/rust"
export {
  render_dart_contract,
  render_dart_operations,
  render_java_contract,
  render_java_operations,
  render_kotlin_contract,
  render_kotlin_operations,
} from "./generator/renderers/jvm"

const CLIENTS_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const PUBLIC_ROOT = dirname(CLIENTS_DIRECTORY)
const PROTOCOL_DIRECTORY = join(PUBLIC_ROOT, "protocol")
const MODEL_DIRECTORY = "model"
const SMITHY_EXECUTABLE = process.env.OPENKACHE_SMITHY_EXECUTABLE ?? "smithy"
const SMITHY_USE_SHELL = process.env.OPENKACHE_SMITHY_USE_SHELL === "1"
const GENERATED_OUTPUT_ROOT = resolve(
  process.env.OPENKACHE_GENERATION_OUTPUT_ROOT ?? PUBLIC_ROOT,
)
function generated_path(...segments: string[]): string {
  return join(GENERATED_OUTPUT_ROOT, ...segments)
}

function resolve_smithy_executable(): string {
  if (
    SMITHY_EXECUTABLE.length === 0 ||
    !SMITHY_EXECUTABLE.includes("/") ||
    SMITHY_EXECUTABLE.startsWith("/")
  ) {
    return SMITHY_EXECUTABLE
  }
  let directory = resolve(process.cwd())
  for (;;) {
    if (
      SMITHY_EXECUTABLE.startsWith("external/") &&
      existsSync(resolve(directory, "external"))
    ) {
      return resolve(directory, SMITHY_EXECUTABLE)
    }
    const candidate = resolve(directory, SMITHY_EXECUTABLE)
    if (existsSync(candidate)) return candidate
    const parent = dirname(directory)
    if (parent === directory) return SMITHY_EXECUTABLE
    directory = parent
  }
}
const GENERATED_OUTPUTS = {
  csharp_api: generated_path("clients/dotnet/OpenKache/generated_local/SmithyApi.g.cs"),
  csharp_operations: generated_path(
    "clients/dotnet/OpenKache/generated_local/SmithyGeneratedOperations.g.cs",
  ),
  csharp_wire: generated_path("clients/dotnet/OpenKache/generated_local/WireValues.g.cs"),
  csharp_native_abi: generated_path(
    "clients/dotnet/OpenKache/generated_local/SmithyNativeAbi.g.cs",
  ),
  rust_client: process.env.OPENKACHE_RUST_CLIENT_OUTPUT ??
    generated_path("clients/core/generated_local/client_contract.rs"),
  rust_operation_constants: process.env.OPENKACHE_RUST_OPERATION_CONSTANTS_OUTPUT ??
    generated_path("clients/core/generated_local/operation_constants.rs"),
  rust_api: process.env.OPENKACHE_RUST_API_OUTPUT ??
    generated_path("clients/rust/generated_local/smithy_api.rs"),
  rust_operations: process.env.OPENKACHE_RUST_OPERATIONS_OUTPUT ??
    generated_path("clients/rust/generated_local/smithy_operations.rs"),
  rust_wire: process.env.OPENKACHE_RUST_WIRE_OUTPUT ??
    generated_path("protocol/generated_local/wire_values.rs"),
  typescript_api: generated_path("clients/typescript/src/generated_local/smithy-api.ts"),
  typescript_operations: generated_path(
    "clients/typescript/src/generated_local/smithy-operations.ts",
  ),
  typescript_value_format: generated_path(
    "clients/typescript/src/generated_local/smithy-value-format.ts",
  ),
  typescript_value_envelope: generated_path(
    "clients/typescript/src/generated_local/smithy-value-envelope.ts",
  ),
  python_api: process.env.OPENKACHE_PYTHON_API_OUTPUT ??
    generated_path("clients/python/src/openkache/_generated/smithy_api.py"),
  python_operations: process.env.OPENKACHE_PYTHON_OPERATIONS_OUTPUT ??
    generated_path("clients/python/src/openkache/_generated/smithy_operations.py"),
  python_contract: process.env.OPENKACHE_PYTHON_CONTRACT_OUTPUT ??
    generated_path("clients/python/src/openkache/_generated/smithy_contract.py"),
  python_native_abi: process.env.OPENKACHE_PYTHON_NATIVE_ABI_OUTPUT ??
    generated_path("clients/python/src/openkache/_generated/smithy_native_abi.py"),
  swift_api: process.env.OPENKACHE_SWIFT_API_OUTPUT ??
    generated_path("clients/swift/generated_local/SmithyAPI.swift"),
  swift_operations: process.env.OPENKACHE_SWIFT_OPERATIONS_OUTPUT ??
    generated_path("clients/swift/generated_local/SmithyOperations.swift"),
  swift_native_abi: process.env.OPENKACHE_SWIFT_NATIVE_ABI_OUTPUT ??
    generated_path("clients/swift/generated_local/SmithyNativeABI.swift"),
  c_contract: process.env.OPENKACHE_C_CONTRACT_OUTPUT ??
    generated_path("clients/core/generated_local/smithy_contract.h"),
  go_api: generated_path("clients/go/smithy_api.go"),
  go_contract: generated_path("clients/go/smithy_contract.go"),
  go_operations: generated_path("clients/go/smithy_operations.go"),
  go_native_abi: generated_path("clients/go/generated_local/smithy_native_abi.h"),
  java_api_root: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local",
  ),
  java_contract: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local/SmithyContract.java",
  ),
  java_native_api: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local/SmithyNativeApi.java",
  ),
  java_native_descriptor: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local/SmithyNativeDescriptor.java",
  ),
  java_native_connect_options: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local/SmithyNativeConnectOptions.java",
  ),
  java_operations: generated_path(
    "clients/java/src/main/java/io/openkache/client/generated_local/SmithyGeneratedOperations.java",
  ),
  kotlin_api: generated_path(
    "clients/kotlin/src/main/kotlin/io/openkache/client/generated_local/SmithyApi.kt",
  ),
  kotlin_contract: generated_path(
    "clients/kotlin/src/main/kotlin/io/openkache/client/generated_local/SmithyContract.kt",
  ),
  kotlin_native_api: generated_path(
    "clients/kotlin/src/main/kotlin/io/openkache/client/generated_local/SmithyNativeApi.kt",
  ),
  kotlin_operations: generated_path(
    "clients/kotlin/src/main/kotlin/io/openkache/client/generated_local/SmithyGeneratedOperations.kt",
  ),
  dart_api: generated_path("clients/dart/lib/generated_local/smithy_api.dart"),
  dart_contract: generated_path("clients/dart/lib/generated_local/smithy_contract.dart"),
  dart_native_api: generated_path("clients/dart/lib/generated_local/smithy_native_api.dart"),
  dart_operations: generated_path("clients/dart/lib/generated_local/smithy_operations.dart"),
} as const

function smithy_ast(client_model: boolean): unknown {
  const cwd = client_model ? CLIENTS_DIRECTORY : PROTOCOL_DIRECTORY
  const models = client_model
    ? [join("..", "protocol", MODEL_DIRECTORY), MODEL_DIRECTORY]
    : [MODEL_DIRECTORY]
  const smithy_executable = resolve_smithy_executable()
  const smithy_command =
    SMITHY_USE_SHELL && process.platform !== "win32"
      ? ["sh", smithy_executable, "ast", ...models]
      : [smithy_executable, "ast", ...models]
  const result = Bun.spawnSync(smithy_command, {
    cwd,
    stderr: "pipe",
    stdout: "pipe",
  })
  if (result.exitCode !== 0) {
    const diagnostics = result.stderr.toString().trim()
    throw new Error(
      diagnostics.length === 0
        ? "`smithy ast` exited without diagnostics"
        : `smithy AST generation failed:\n${diagnostics}`,
    )
  }
  try {
    return JSON.parse(result.stdout.toString()) as unknown
  } catch (error) {
    throw new Error(`smithy emitted invalid JSON: ${String(error)}`)
  }
}

type Generation_Target =
  | "all"
  | "c-contract"
  | "dart"
  | "dotnet"
  | "go"
  | "go-contract"
  | "java"
  | "kotlin"
  | "python"
  | "rust-api"
  | "rust-client"
  | "rust-wire"
  | "swift"
  | "typescript"

function generation_target(value: string | undefined): Generation_Target {
  switch (value) {
    case undefined:
      return "all"
    case "all":
      return "all"
    case "c-contract":
      return "c-contract"
    case "dart":
      return "dart"
    case "dotnet":
      return "dotnet"
    case "go":
      return "go"
    case "go-contract":
      return "go-contract"
    case "java":
      return "java"
    case "kotlin":
      return "kotlin"
    case "python":
      return "python"
    case "rust-api":
      return "rust-api"
    case "rust-client":
      return "rust-client"
    case "rust-wire":
      return "rust-wire"
    case "swift":
      return "swift"
    case "typescript":
      return "typescript"
    default:
      throw new Error(`unsupported OPENKACHE_GENERATION_TARGET ${JSON.stringify(value)}`)
  }
}

function expected_wire_outputs(
  contract: Wire_Contract,
  target: "rust-wire",
): Readonly<Record<string, string>> {
  if (target !== "rust-wire") {
    throw new Error(`unsupported wire generation target ${target}`)
  }
  return {
    [GENERATED_OUTPUTS.rust_wire]: render_protocol_rust_wire(contract),
  }
}

function java_native_structure_outputs(
  contract: Client_Contract,
): Readonly<Record<string, string>> {
  const outputs: Record<string, string> = {}
  for (const structure of contract.ffi.native_abi_structures) {
    // These two structures have dedicated renderers because their public
    // names are part of the long-standing Java adapter contract.
    if (
      structure.name === "FfiNamespaceDescriptor" ||
      structure.name === "FfiConnectOptions"
    ) {
      continue
    }
    const class_name = native_abi_structure_class_name(structure.name)
    outputs[join(GENERATED_OUTPUTS.java_api_root, `${class_name}.java`)] =
      render_java_native_structure(structure)
  }
  return outputs
}

function expected_outputs(
  contract: Client_Contract,
  target: Generation_Target,
): Readonly<Record<string, string>> {
  switch (target) {
    case "all":
      return {
        [GENERATED_OUTPUTS.csharp_api]: render_csharp_api(contract),
        [GENERATED_OUTPUTS.csharp_operations]: render_csharp_operations(contract),
        [GENERATED_OUTPUTS.csharp_wire]: render_csharp(contract),
        [GENERATED_OUTPUTS.csharp_native_abi]: render_csharp_native_abi(contract),
        [GENERATED_OUTPUTS.rust_client]: render_rust_client(contract),
        [GENERATED_OUTPUTS.rust_api]: render_rust_api(contract),
        [GENERATED_OUTPUTS.rust_operations]: render_rust_operations(contract),
        [GENERATED_OUTPUTS.rust_wire]: render_protocol_rust_wire(contract),
        [GENERATED_OUTPUTS.typescript_api]: render_typescript_api(contract),
        [GENERATED_OUTPUTS.typescript_operations]:
          render_typescript_operations(contract),
        [GENERATED_OUTPUTS.typescript_value_format]:
          render_typescript_value_format(contract),
        [GENERATED_OUTPUTS.typescript_value_envelope]:
          render_typescript_value_envelope(contract),
        [GENERATED_OUTPUTS.python_api]: render_python_api(contract),
        [GENERATED_OUTPUTS.python_operations]: render_python_operations(contract),
        [GENERATED_OUTPUTS.python_contract]: render_python_contract(contract),
        [GENERATED_OUTPUTS.python_native_abi]: render_python_native_abi(contract),
        [GENERATED_OUTPUTS.swift_api]: render_swift_api(contract),
        [GENERATED_OUTPUTS.swift_operations]: render_swift_operations(contract),
        [GENERATED_OUTPUTS.swift_native_abi]: render_swift_native_abi(contract),
        [GENERATED_OUTPUTS.c_contract]: render_c_contract(contract),
        [GENERATED_OUTPUTS.go_api]: format_go_source(render_go_api(contract)),
        [GENERATED_OUTPUTS.go_contract]: format_go_source(render_go_contract(contract)),
        [GENERATED_OUTPUTS.go_operations]: format_go_source(render_go_operations(contract)),
        [GENERATED_OUTPUTS.go_native_abi]: render_go_native_abi(contract),
        ...render_java_api(contract, GENERATED_OUTPUTS.java_api_root),
        [GENERATED_OUTPUTS.java_contract]: render_java_contract(contract),
        [GENERATED_OUTPUTS.java_native_api]: render_java_native_api(contract),
        [GENERATED_OUTPUTS.java_native_descriptor]: render_java_native_descriptor(contract),
        [GENERATED_OUTPUTS.java_native_connect_options]:
          render_java_native_connect_options(contract),
        ...java_native_structure_outputs(contract),
        [GENERATED_OUTPUTS.java_operations]: render_java_operations(contract),
        [GENERATED_OUTPUTS.kotlin_api]: render_kotlin_api(contract),
        [GENERATED_OUTPUTS.kotlin_contract]: render_kotlin_contract(contract),
        [GENERATED_OUTPUTS.kotlin_native_api]: render_kotlin_native_api(contract),
        [GENERATED_OUTPUTS.kotlin_operations]: render_kotlin_operations(contract),
        [GENERATED_OUTPUTS.dart_api]: render_dart_api(contract),
        [GENERATED_OUTPUTS.dart_contract]: render_dart_contract(contract),
        [GENERATED_OUTPUTS.dart_native_api]: render_dart_native_api(contract),
        [GENERATED_OUTPUTS.dart_operations]: render_dart_operations(contract),
      }
    case "c-contract":
      return {
        [GENERATED_OUTPUTS.c_contract]: render_c_contract(contract),
      }
    case "dart":
      return {
        [GENERATED_OUTPUTS.dart_api]: render_dart_api(contract),
        [GENERATED_OUTPUTS.dart_contract]: render_dart_contract(contract),
        [GENERATED_OUTPUTS.dart_native_api]: render_dart_native_api(contract),
        [GENERATED_OUTPUTS.dart_operations]: render_dart_operations(contract),
      }
    case "dotnet":
      return {
        [GENERATED_OUTPUTS.csharp_api]: render_csharp_api(contract),
        [GENERATED_OUTPUTS.csharp_operations]: render_csharp_operations(contract),
        [GENERATED_OUTPUTS.csharp_wire]: render_csharp(contract),
        [GENERATED_OUTPUTS.csharp_native_abi]: render_csharp_native_abi(contract),
      }
    case "go":
      return {
        [GENERATED_OUTPUTS.go_api]: format_go_source(render_go_api(contract)),
        [GENERATED_OUTPUTS.go_contract]: format_go_source(render_go_contract(contract)),
        [GENERATED_OUTPUTS.go_operations]: format_go_source(render_go_operations(contract)),
        [GENERATED_OUTPUTS.go_native_abi]: render_go_native_abi(contract),
      }
    case "go-contract":
      return {
        [GENERATED_OUTPUTS.go_api]: format_go_source(render_go_api(contract)),
        [GENERATED_OUTPUTS.go_contract]: format_go_source(render_go_contract(contract)),
      }
    case "java":
      return {
        ...render_java_api(contract, GENERATED_OUTPUTS.java_api_root),
        [GENERATED_OUTPUTS.java_contract]: render_java_contract(contract),
        [GENERATED_OUTPUTS.java_native_api]: render_java_native_api(contract),
        [GENERATED_OUTPUTS.java_native_descriptor]: render_java_native_descriptor(contract),
        [GENERATED_OUTPUTS.java_native_connect_options]:
          render_java_native_connect_options(contract),
        ...java_native_structure_outputs(contract),
        [GENERATED_OUTPUTS.java_operations]: render_java_operations(contract),
      }
    case "kotlin":
      return {
        [GENERATED_OUTPUTS.kotlin_api]: render_kotlin_api(contract),
        [GENERATED_OUTPUTS.kotlin_contract]: render_kotlin_contract(contract),
        [GENERATED_OUTPUTS.kotlin_native_api]: render_kotlin_native_api(contract),
        [GENERATED_OUTPUTS.kotlin_operations]: render_kotlin_operations(contract),
      }
    case "rust-api":
      return {
        [GENERATED_OUTPUTS.rust_api]: render_rust_api(contract),
        [GENERATED_OUTPUTS.rust_operations]: render_rust_operations(contract),
      }
    case "rust-client":
      return {
        [GENERATED_OUTPUTS.rust_client]: render_rust_client(contract),
        [GENERATED_OUTPUTS.rust_operation_constants]:
          render_rust_operation_constants(contract),
      }
    case "rust-wire":
      return {
        [GENERATED_OUTPUTS.rust_wire]: render_protocol_rust_wire(contract),
      }
    case "typescript":
      return {
        [GENERATED_OUTPUTS.typescript_api]: render_typescript_api(contract),
        [GENERATED_OUTPUTS.typescript_operations]:
          render_typescript_operations(contract),
        [GENERATED_OUTPUTS.typescript_value_format]:
          render_typescript_value_format(contract),
        [GENERATED_OUTPUTS.typescript_value_envelope]:
          render_typescript_value_envelope(contract),
      }
    case "python":
      return {
        [GENERATED_OUTPUTS.python_api]: render_python_api(contract),
        [GENERATED_OUTPUTS.python_operations]: render_python_operations(contract),
        [GENERATED_OUTPUTS.python_contract]: render_python_contract(contract),
        [GENERATED_OUTPUTS.python_native_abi]: render_python_native_abi(contract),
      }
    case "swift":
      return {
        [GENERATED_OUTPUTS.swift_api]: render_swift_api(contract),
        [GENERATED_OUTPUTS.swift_operations]: render_swift_operations(contract),
        [GENERATED_OUTPUTS.swift_native_abi]: render_swift_native_abi(contract),
      }
  }
}

/** Directory whose files are fully owned by one generated target. */
export interface Generated_Output_Scope {
  readonly directory: string
  readonly extensions?: readonly string[]
}

function generated_scope_files(scope: Generated_Output_Scope): readonly string[] {
  const files: string[] = []
  const extensions = scope.extensions === undefined ? undefined : new Set(scope.extensions)
  const visit = (directory: string): void => {
    let entries
    try {
      entries = readdirSync(directory, { withFileTypes: true })
    } catch {
      return
    }
    for (const entry of entries) {
      const path = join(directory, entry.name)
      if (entry.isDirectory()) {
        visit(path)
      } else if (
        entry.isFile() &&
        (extensions === undefined || extensions.has(entry.name.slice(entry.name.lastIndexOf("."))))
      ) {
        files.push(path)
      }
    }
  }
  visit(scope.directory)
  return files.sort()
}

function generated_scope_obsolete_files(
  outputs: Readonly<Record<string, string>>,
  scopes: readonly Generated_Output_Scope[],
): readonly string[] {
  const expected_paths = new Set(Object.keys(outputs).map((path) => resolve(path)))
  const obsolete = new Set<string>()
  for (const scope of scopes) {
    for (const path of generated_scope_files(scope)) {
      if (!expected_paths.has(resolve(path))) obsolete.add(path)
    }
  }
  return [...obsolete].sort()
}

/** Returns generated outputs that are missing or differ from the contract. */
export function generated_output_issues(
  outputs: Readonly<Record<string, string>>,
  scopes: readonly Generated_Output_Scope[] = [],
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
  for (const output_path of generated_scope_obsolete_files(outputs, scopes)) {
    mismatches.push(`${output_path} (obsolete)`)
  }
  return mismatches
}

function generated_output_scopes(target: Generation_Target): readonly Generated_Output_Scope[] {
  switch (target) {
    case "all":
    case "java":
      return [{ directory: GENERATED_OUTPUTS.java_api_root, extensions: [".java"] }]
    default:
      return []
  }
}

function write_outputs(
  outputs: Readonly<Record<string, string>>,
  check_only: boolean,
  scopes: readonly Generated_Output_Scope[],
): void {
  if (check_only) {
    const mismatches = generated_output_issues(outputs, scopes)
    if (mismatches.length > 0) {
      throw new Error(
        "generated contract outputs are stale:\n" +
          mismatches.map((output_path) => `  - ${output_path}`).join("\n") +
          "\nRun `just generate-protocol-contract` to regenerate them.",
      )
    }
    return
  }
  for (const output_path of generated_scope_obsolete_files(outputs, scopes)) {
    rmSync(output_path, { force: true })
    console.log(`Removed obsolete generated output ${output_path}`)
  }
  for (const [output_path, content] of Object.entries(outputs)) {
    const output_directory = dirname(output_path)
    mkdirSync(output_directory, { recursive: true })
    // Parallel build recipes may generate overlapping targets; rename a complete
    // temporary file so readers never observe a partially written contract.
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
}

/** Runs the protocol contract generator CLI.
 *
 * @returns Process exit code.
 */
export function main(): number {
  try {
    const target = generation_target(process.env.OPENKACHE_GENERATION_TARGET)
    const outputs =
      target === "rust-wire"
        ? expected_wire_outputs(extract_protocol_wire_contract(smithy_ast(false)), target)
        : expected_outputs(
          extract_client_contract(smithy_ast(true), true, PROTOCOL_V1_WIRE_ADAPTER),
          target,
        )
    write_outputs(
      outputs,
      process.env.OPENKACHE_GENERATION_CHECK === "1",
      generated_output_scopes(target),
    )
    return 0
  } catch (error) {
    console.error(
      `GENERATION_FAILED: ${error instanceof Error ? error.message : String(error)}\n` +
        "  Why: client language and ABI values can only be generated from valid, complete wire and client Smithy contracts.\n" +
        "  Fix: Run `smithy validate model` for the protocol and client models, correct the reported model or generator error, then rerun `./generate.ts` from the clients directory.",
    )
    return 1
  }
}

if (import.meta.main) process.exit(main())
