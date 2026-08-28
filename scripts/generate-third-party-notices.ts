#!/usr/bin/env bun
/**
 * Generates the third-party license bundle for a release artifact.
 *
 * The bundle is intentionally produced in release staging (or `target/`) and
 * is not checked into the repository. Cargo metadata supplies the exact
 * locked dependency graph, while the registry source trees supply the
 * upstream license and notice files.
 */

import {
  existsSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs"
import { dirname, isAbsolute, join, relative, resolve } from "node:path"

import yargs from "yargs"
import { hideBin } from "yargs/helpers"

/** Names of release dependency graphs supported by the notice generator. */
export type Artifact_Name =
  | "server"
  | "client"
  | "python"
  | "typescript"
  | "cmake"
  | "rust"
  | "dotnet"
  | "cli"
  | "all"

type Cargo_Metadata = {
  readonly packages: readonly Cargo_Package[]
  readonly resolve?: Cargo_Resolve
}

type Cargo_Package = {
  readonly id: string
  readonly name: string
  readonly version: string
  readonly license?: string | null
  readonly license_file?: string | null
  readonly authors?: readonly string[] | null
  readonly repository?: string | null
  readonly source?: string | null
  readonly manifest_path: string
}

type Cargo_Resolve = {
  readonly nodes: readonly Cargo_Node[]
}

type Cargo_Node = {
  readonly id: string
  readonly deps?: readonly Cargo_Dependency[]
}

type Cargo_Dependency_Kind = {
  readonly kind?: string
}

type Cargo_Dependency = {
  readonly pkg: string
  readonly dep_kinds?: readonly Cargo_Dependency_Kind[]
}

type Root_Spec = {
  readonly manifest: string
  readonly package: string
}

type Package_License_File = {
  readonly path: string
  readonly content: string
}

/** Dependency metadata and reproduced legal files rendered in a notice. */
export type Notice_Package = {
  readonly name: string
  readonly version: string
  readonly license: string
  readonly license_file?: string
  readonly authors: readonly string[]
  readonly repository?: string
  readonly source: string
  readonly license_files: readonly Package_License_File[]
}

const ARTIFACT_NAMES: readonly Artifact_Name[] = [
  "server",
  "client",
  "python",
  "typescript",
  "cmake",
  "rust",
  "dotnet",
  "cli",
  "all",
] as const

const ARTIFACT_ROOTS: Readonly<
  Record<Exclude<Artifact_Name, "all">, readonly Root_Spec[]>
> = {
  server: [
    { manifest: "server/Cargo.toml", package: "openkache-server" },
  ],
  client: [
    { manifest: "clients/core/Cargo.toml", package: "openkache-client-core" },
    {
      manifest: "clients/typescript/native/Cargo.toml",
      package: "openkache-client-napi",
    },
    {
      manifest: "clients/python/native/Cargo.toml",
      package: "openkache-client-python-native",
    },
    { manifest: "clients/rust/Cargo.toml", package: "openkache" },
  ],
  python: [
    {
      manifest: "clients/python/native/Cargo.toml",
      package: "openkache-client-python-native",
    },
  ],
  typescript: [
    {
      manifest: "clients/typescript/native/Cargo.toml",
      package: "openkache-client-napi",
    },
  ],
  cmake: [
    { manifest: "clients/core/Cargo.toml", package: "openkache-client-core" },
  ],
  rust: [
    { manifest: "clients/rust/Cargo.toml", package: "openkache" },
  ],
  dotnet: [
    { manifest: "clients/core/Cargo.toml", package: "openkache-client-core" },
  ],
  cli: [
    { manifest: "clients/cli/Cargo.toml", package: "openkache-cli" },
  ],
}

/**
 * Keep a deliberate record of the permissive option used for dual-licensed
 * dependencies. The complete upstream files are still reproduced below, but
 * this map documents the term selected for the OpenKache build.
 */
const LICENSE_SELECTIONS: Readonly<Record<string, string>> = {
  blake3: "Apache-2.0",
  "r-efi": "MIT",
  "ryu-js": "Apache-2.0",
  "zstd-pure-rs": "BSD-3-Clause",
}

/**
 * Conventional names used for license, notice, copyright, and patent files.
 *
 * SPDX/reuse projects often use short names inside `LICENSES/`, which are
 * handled separately below.  The prefix also covers common aggregate files
 * such as `THIRD-PARTY-NOTICES` and `THIRD_PARTY` without treating unrelated
 * source files as legal text.
 */
const LICENSE_FILE_NAME =
  /^(?:(?:third[._ -]?party[._ -]?)?(?:licenses?|licences?|copying(?:[0-9]+)?|notices?|authors?|copyrights?|unlicenses?|patents?|third[._ -]?party))(?:[._-].*)?$/iu
const CRATES_IO_SOURCE_PREFIX =
  "registry+https://github.com/rust-lang/crates.io-index"

function compare_strings(left: string, right: string): number {
  if (left < right) return -1
  if (left > right) return 1
  return 0
}

function fail(message: string): never {
  throw new Error(
    `${message}\n` +
      "Why: release artifacts must carry the license and notice material for " +
      "the exact locked dependency graph they distribute.\n" +
      "Fix: run `cargo fetch --locked` in the public checkout, then rerun the " +
      "notice generator from a supported Nix/Bun environment.",
  )
}

function normalize_text(content: string): string {
  return content.replace(/\r\n?/gu, "\n")
}

function decode_license_text(bytes: Uint8Array, file_path: string): string {
  try {
    return new TextDecoder("utf-8", {
      fatal: true,
      // Preserve a UTF-8 BOM as part of the upstream text. The generator only
      // normalizes line endings and must not silently remove legal content.
      ignoreBOM: true,
    }).decode(bytes)
  } catch (error) {
    fail(
      `License or notice file ${file_path} is not valid UTF-8: ${
        error instanceof Error ? error.message : String(error)
      }.`,
    )
  }
}

/**
 * Chooses a fenced Markdown delimiter that cannot occur as a complete line
 * inside the reproduced upstream text.
 *
 * License files are copied as data, not parsed as Markdown. A dependency can
 * nevertheless contain backtick runs (for example, a notice with a Markdown
 * example), so a fixed three-backtick fence could terminate the rendered
 * block early. One extra backtick beyond the longest run keeps the text
 * faithful while preserving a readable fenced block.
 *
 * @param content - Normalized upstream license or notice text.
 * @returns A backtick fence of safe length for the content.
 */
function markdown_fence(content: string): string {
  let longest_run = 0
  for (const match of content.matchAll(/`+/gu)) {
    longest_run = Math.max(longest_run, match[0].length)
  }
  return "`".repeat(Math.max(3, longest_run + 1))
}

function license_expression_offers(
  expression: string,
  selected_license: string,
): boolean {
  return expression
    .replace(/[()]/gu, "")
    .split(/\s+(?:AND|OR)\s+/u)
    .some((term) => term.trim() === selected_license)
}

function parse_metadata(output: string, manifest: string): Cargo_Metadata {
  const json_start = output.indexOf("{")
  const json_end = output.lastIndexOf("}")
  if (json_start < 0 || json_end <= json_start) {
    fail(
      `cargo metadata for ${manifest} did not produce JSON.\n` +
        `Cargo output: ${output.trim() || "<empty>"}`,
    )
  }
  try {
    return JSON.parse(output.slice(json_start, json_end + 1)) as Cargo_Metadata
  } catch (error) {
    fail(
      `cargo metadata for ${manifest} returned invalid JSON: ${
        error instanceof Error ? error.message : String(error)
      }.\nCargo output: ${output.slice(json_start, json_end + 1).slice(0, 500)}`,
    )
  }
}

function cargo_metadata(public_root: string, root_spec: Root_Spec): Cargo_Metadata {
  const result = Bun.spawnSync(
    [
      "cargo",
      "metadata",
      "--manifest-path",
      root_spec.manifest,
      "--format-version",
      "1",
      "--locked",
      "--all-features",
    ],
    {
      cwd: public_root,
      stdout: "pipe",
      stderr: "pipe",
    },
  )
  if (result.exitCode !== 0) {
    fail(
      `cargo metadata failed for ${root_spec.manifest} with exit code ${result.exitCode}.\n` +
        `Cargo stderr: ${result.stderr.toString().trim() || "<empty>"}`,
    )
  }
  return parse_metadata(result.stdout.toString(), root_spec.manifest)
}

function package_by_root(
  metadata: Cargo_Metadata,
  public_root: string,
  root_spec: Root_Spec,
): Cargo_Package {
  const expected_manifest = resolve(public_root, root_spec.manifest)
  const candidate_package = metadata.packages.find(
    (candidate) =>
      resolve(candidate.manifest_path) === expected_manifest &&
      candidate.name === root_spec.package,
  )
  if (candidate_package === undefined) {
    fail(
      `Cargo metadata did not contain ${root_spec.package} at ${root_spec.manifest}.`,
    )
  }
  return candidate_package
}

function dependency_is_runtime_or_build(
  dependency: Cargo_Dependency,
): boolean {
  const dependency_kinds = dependency.dep_kinds ?? []
  return dependency_kinds.length === 0 ||
    dependency_kinds.some((kind) => kind.kind !== "dev")
}

function dependency_closure(
  metadata: Cargo_Metadata,
  root_package: Cargo_Package,
): ReadonlyMap<string, Cargo_Package> {
  const resolve_graph = metadata.resolve
  if (resolve_graph === undefined) {
    fail(`Cargo metadata for ${root_package.name} did not include a resolve graph.`)
  }
  const packages_by_id = new Map(
    metadata.packages.map((candidate) => [candidate.id, candidate]),
  )
  const nodes_by_id = new Map(
    resolve_graph.nodes.map((node) => [node.id, node]),
  )
  const seen_ids = new Set<string>()
  const pending_ids = [root_package.id]
  while (pending_ids.length > 0) {
    const package_id = pending_ids.pop()
    if (package_id === undefined || seen_ids.has(package_id)) continue
    seen_ids.add(package_id)
    const node = nodes_by_id.get(package_id)
    if (node === undefined) {
      fail(
        `Cargo resolve graph is missing node ${package_id} while walking ${root_package.name}.`,
      )
    }
    for (const dependency of node.deps ?? []) {
      if (dependency_is_runtime_or_build(dependency)) {
        pending_ids.push(dependency.pkg)
      }
    }
  }
  return new Map(
    [...seen_ids]
      .map((package_id) => packages_by_id.get(package_id))
      .filter((candidate): candidate is Cargo_Package =>
        candidate !== undefined && typeof candidate.source === "string",
      )
      .map((candidate) => [candidate.id, candidate]),
  )
}

/**
 * Collects textual license and notice material from a dependency source tree.
 *
 * SPDX-oriented crates sometimes keep complete texts in a `LICENSES/`
 * directory under short names such as `MIT.txt`; those names do not carry a
 * conventional `LICENSE` prefix, so every textual file in a license directory
 * is included as well.
 *
 * @param package_root - Root of the unpacked dependency source tree.
 * @returns Deterministically ordered license and notice files.
 * @throws {Error} When a legal file is invalid UTF-8, binary, or escapes the
 * package source tree through a symlink.
 */
export function collect_license_files(
  package_root: string,
): readonly Package_License_File[] {
  const files: Package_License_File[] = []
  const real_package_root = realpathSync(package_root)
  const pending_directories = [package_root]
  while (pending_directories.length > 0) {
    const directory = pending_directories.pop()
    if (directory === undefined) continue
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (entry.name.startsWith(".")) continue
      const entry_path = join(directory, entry.name)
      if (entry.isDirectory()) {
        pending_directories.push(entry_path)
        continue
      }
      const relative_path = relative(package_root, entry_path)
        .split("\\")
        .join("/")
      const in_license_directory = relative_path
        .split("/")
        .slice(0, -1)
        .some((segment) => /^licenses?$/iu.test(segment))
      if (!in_license_directory && !LICENSE_FILE_NAME.test(entry.name)) continue
      const entry_stats = lstatSync(entry_path)
      let content_path = entry_path
      if (entry_stats.isSymbolicLink()) {
        let real_content_path: string
        try {
          real_content_path = realpathSync(entry_path)
        } catch (error) {
          fail(
            `License or notice file ${relative_path} is a broken symlink: ${
              error instanceof Error ? error.message : String(error)
            }.`,
          )
        }
        const real_relative_path = relative(
          real_package_root,
          real_content_path,
        )
        if (
          isAbsolute(real_relative_path) ||
          real_relative_path === ".." ||
          real_relative_path.startsWith("../") ||
          real_relative_path.startsWith("..\\")
        ) {
          fail(
            `License or notice file ${relative_path} points outside the package source tree.`,
          )
        }
        content_path = real_content_path
      } else if (!entry_stats.isFile()) {
        fail(
          `License or notice path ${relative_path} is not a regular file.`,
        )
      }
      const file_stats = statSync(content_path)
      if (!file_stats.isFile()) {
        fail(
          `License or notice path ${relative_path} does not resolve to a regular file.`,
        )
      }
      if (file_stats.size === 0) continue
      const bytes = readFileSync(content_path)
      if (bytes.includes(0)) {
        fail(
          `License or notice file ${relative_path} contains NUL bytes and cannot be reproduced as text.`,
        )
      }
      files.push({
        path: relative_path,
        content: normalize_text(decode_license_text(bytes, relative_path)),
      })
    }
  }
  return files.sort((left, right) => compare_strings(left.path, right.path))
}

function package_notice(candidate: Cargo_Package): Notice_Package {
  const package_root = dirname(candidate.manifest_path)
  if (!existsSync(package_root)) {
    fail(
      `Cargo source directory for ${candidate.name}@${candidate.version} is missing: ${package_root}.`,
    )
  }
  if (typeof candidate.license !== "string" || candidate.license.length === 0) {
    fail(
      `${candidate.name}@${candidate.version} has no SPDX license expression in Cargo metadata.`,
    )
  }
  const license_files = [...collect_license_files(package_root)]
  if (typeof candidate.license_file === "string") {
    const declared_license_path = resolve(package_root, candidate.license_file)
    const declared_license_relative_path = relative(
      package_root,
      declared_license_path,
    ).split("\\").join("/")
    if (
      isAbsolute(declared_license_relative_path) ||
      declared_license_relative_path === ".." ||
      declared_license_relative_path.startsWith("../")
    ) {
      fail(
        `${candidate.name}@${candidate.version} declares a license file outside its source tree: ${candidate.license_file}.`,
      )
    }
    if (
      !existsSync(declared_license_path) ||
      !statSync(declared_license_path).isFile()
    ) {
      fail(
        `${candidate.name}@${candidate.version} declares a missing license file: ${candidate.license_file}.`,
      )
    }
    const real_package_root = realpathSync(package_root)
    const real_declared_license_path = realpathSync(declared_license_path)
    const real_declared_relative_path = relative(
      real_package_root,
      real_declared_license_path,
    )
    if (
      isAbsolute(real_declared_relative_path) ||
      real_declared_relative_path === ".." ||
      real_declared_relative_path.startsWith("../") ||
      real_declared_relative_path.startsWith("..\\")
    ) {
      fail(
        `${candidate.name}@${candidate.version} declares a license file that points outside its source tree: ${candidate.license_file}.`,
      )
    }
    if (!statSync(real_declared_license_path).isFile()) {
      fail(
        `${candidate.name}@${candidate.version} declares a license path that is not a regular file: ${candidate.license_file}.`,
      )
    }
    if (!license_files.some((file) => file.path === declared_license_relative_path)) {
      const declared_license_bytes = readFileSync(real_declared_license_path)
      if (declared_license_bytes.length === 0) {
        fail(
          `${candidate.name}@${candidate.version} declares an empty license file: ${candidate.license_file}.`,
        )
      }
      if (declared_license_bytes.includes(0)) {
        fail(
          `${candidate.name}@${candidate.version} declares a binary license file: ${candidate.license_file}.`,
        )
      }
      license_files.push({
        path: declared_license_relative_path,
        content: normalize_text(
          decode_license_text(
            declared_license_bytes,
            declared_license_relative_path,
          ),
        ),
      })
      license_files.sort((left, right) => compare_strings(left.path, right.path))
    }
  }
  return {
    name: candidate.name,
    version: candidate.version,
    license: candidate.license,
    ...(typeof candidate.license_file !== "string"
      ? {}
      : { license_file: candidate.license_file }),
    authors: Array.isArray(candidate.authors) ? candidate.authors : [],
    ...(typeof candidate.repository !== "string"
      ? {}
      : { repository: candidate.repository }),
    source: candidate.source ?? "<unknown source>",
    license_files,
  }
}

function unique_packages(
  public_root: string,
  artifact_name: Exclude<Artifact_Name, "all">,
): readonly Notice_Package[] {
  const package_by_id = new Map<string, Cargo_Package>()
  for (const root_spec of ARTIFACT_ROOTS[artifact_name]) {
    const metadata = cargo_metadata(public_root, root_spec)
    const root_package = package_by_root(metadata, public_root, root_spec)
    for (const [package_id, candidate_package] of dependency_closure(
      metadata,
      root_package,
    )) {
      package_by_id.set(package_id, candidate_package)
    }
  }
  return [...package_by_id.values()]
    .map(package_notice)
    .sort((left, right) =>
      compare_strings(
        `${left.name}\u0000${left.version}\u0000${left.source}`,
        `${right.name}\u0000${right.version}\u0000${right.source}`,
      ))
}

function packages_for_artifact(
  public_root: string,
  artifact_name: Artifact_Name,
): readonly Notice_Package[] {
  if (artifact_name !== "all") {
    return unique_packages(public_root, artifact_name)
  }
  const package_by_key = new Map<string, Notice_Package>()
  for (const child_artifact of ARTIFACT_NAMES) {
    if (child_artifact === "all") continue
    for (const package_ of unique_packages(public_root, child_artifact)) {
      const key = `${package_.name}\u0000${package_.version}\u0000${package_.source}`
      package_by_key.set(key, package_)
    }
  }
  return [...package_by_key.values()].sort((left, right) =>
    compare_strings(
      `${left.name}\u0000${left.version}\u0000${left.source}`,
      `${right.name}\u0000${right.version}\u0000${right.source}`,
    ))
}

function render_package(package_: Notice_Package): string {
  const selection = LICENSE_SELECTIONS[package_.name]
  if (
    selection !== undefined &&
    !license_expression_offers(package_.license, selection)
  ) {
    fail(
      `${package_.name}@${package_.version} no longer offers the selected ` +
        `${selection} terms in its SPDX expression ${package_.license}.`,
    )
  }
  const source =
    package_.repository ??
    (package_.source.startsWith(CRATES_IO_SOURCE_PREFIX)
      ? `https://crates.io/crates/${package_.name}/${package_.version}`
      : package_.source)
  const lines = [
    "--------------------------------------------------------------------------------",
    `## ${package_.name} ${package_.version}`,
    "",
    `- License expression: ${package_.license}`,
    ...(selection === undefined
      ? []
      : [`- OpenKache selected terms: ${selection}`]),
    `- Cargo source: ${package_.source}`,
    `- Source: ${source}`,
    ...(package_.license_file === undefined
      ? []
      : [`- Declared license file: ${package_.license_file}`]),
    ...(package_.authors.length === 0
      ? []
      : [`- Authors: ${package_.authors.join("; ")}`]),
    "",
  ]
  if (package_.license_files.length === 0) {
    lines.push(
      "The dependency source tree does not contain a separate license or " +
        "notice file. The Cargo license expression above is the package's " +
        "published license declaration; retain this attribution and consult " +
        "the upstream repository before redistributing modified source.",
      "",
    )
    return `${lines.join("\n")}\n`
  }
  lines.push(
    "The following license and notice files are reproduced verbatim with " +
      "line endings normalized:",
  )
  for (const license_file of package_.license_files) {
    const fence = markdown_fence(license_file.content)
    lines.push(
      "",
      `### ${license_file.path}`,
      "",
      `${fence}text`,
      license_file.content,
      fence,
    )
  }
  return `${lines.join("\n")}\n`
}

/**
 * Renders a deterministic third-party notice for one artifact graph.
 *
 * @param artifact_name - Artifact whose dependency graph is represented.
 * @param packages - Sorted dependency metadata and upstream legal files.
 * @returns Complete notice text with attribution headers and package sections.
 * @throws {Error} When a deliberate OR-license selection is no longer offered
 * by a dependency's SPDX expression.
 */
export function render_notice(
  artifact_name: Artifact_Name,
  packages: readonly Notice_Package[],
): string {
  const packages_without_license_files = packages.filter(
    (package_) => package_.license_files.length === 0,
  ).length
  const header = [
    "THIRD-PARTY-NOTICES",
    "===================",
    "",
    "This file is generated from the locked Cargo dependency graph for the",
    `OpenKache ${artifact_name} artifact. Do not edit it by hand.`,
    "",
    "It covers runtime and build dependencies reachable from the artifact's",
    "production Cargo roots. OpenKache's own license is distributed separately",
    "in LICENSE. Each package below retains its published license expression",
    "and upstream source attribution. For an OR expression, the selected terms",
    "are recorded when OpenKache has a deliberate build-policy choice.",
    "",
    `Dependency package count: ${packages.length}`,
    `Packages without separate license files: ${packages_without_license_files}`,
    ...(packages_without_license_files === 0
      ? []
      : [
        "LEGAL REVIEW REQUIRED: affected entries retain SPDX declarations and",
        "upstream URLs; verify full license text against the upstream source " +
          "before redistribution.",
      ]),
    "",
  ]
  return `${header.join("\n")}${packages.map(render_package).join("\n")}`
}

function output_notice(output_path: string, content: string): void {
  if (output_path === "-") {
    process.stdout.write(content)
    return
  }
  mkdirSync(dirname(output_path), { recursive: true })
  writeFileSync(output_path, content, "utf8")
}

/**
 * Parses CLI arguments and writes one generated notice bundle.
 *
 * @returns A zero exit code after successful generation.
 * @throws {Error} When Cargo metadata, dependency sources, or output handling
 * cannot complete safely.
 */
export async function main(): Promise<number> {
  const parsed_arguments = await yargs(hideBin(Bun.argv))
    .scriptName("generate-third-party-notices")
    .option("artifact", {
      type: "string",
      choices: [...ARTIFACT_NAMES],
      default: "all",
      describe: "release artifact dependency graph to include",
    })
    .option("output", {
      type: "string",
      demandOption: true,
      describe: "staging path for the generated notice, or - for stdout",
    })
    .option("root", {
      type: "string",
      describe: "public OpenKache repository root (defaults to this checkout)",
    })
    .strict()
    .help()
    .parse()
  const artifact_name = parsed_arguments.artifact as Artifact_Name
  const public_root = resolve(
    parsed_arguments.root ?? join(import.meta.dir, ".."),
  )
  if (!existsSync(join(public_root, "Cargo.toml"))) {
    fail(`Public repository root does not contain Cargo.toml: ${public_root}.`)
  }
  const packages = packages_for_artifact(public_root, artifact_name)
  if (packages.length === 0) {
    fail(
      `No external registry or git dependencies were found for the ${artifact_name} artifact.`,
    )
  }
  output_notice(
    parsed_arguments.output === "-"
      ? parsed_arguments.output
      : resolve(parsed_arguments.output),
    render_notice(artifact_name, packages),
  )
  if (parsed_arguments.output !== "-") {
    console.error(
      `Generated ${artifact_name} third-party notice with ${packages.length} packages at ${parsed_arguments.output}.`,
    )
  }
  return 0
}

if (import.meta.main) {
  try {
    process.exit(await main())
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    process.exit(1)
  }
}
