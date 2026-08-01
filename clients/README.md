# OpenKache client SDKs

OpenKache client packages share one Rust engine for connection management,
protocol operations, key protection, compression, and encryption. Implemented
language packages add only their native API and runtime integration.

## Documentation

Shared client topics are documented here:

| Topic | Reference |
|---|---|
| SDK inventory, implementation status, and binding boundaries | This README |
| Formatted value bytes, serialization, compression, and application-level encryption | [Value format](VALUE_FORMAT.md) |
| QUIC framing, operations, limits, and retry ambiguity | [Wire protocol](../protocol/SPEC.md) |
| Language API, build, packaging, and runtime configuration | The implemented package's README |

Package documentation links to these references instead of restating shared
formats or protocol behavior.

## SDK status

| Package | Path | Implementation |
|---|---|---|
| Shared core | [`core/`](core/) | Protocol v1 raw and protected engine; value format v1 implementation |
| Rust | [`rust/`](rust/) | Protocol v1 end-user SDK; byte APIs use v1 Raw serialization |
| CLI | [`cli/`](cli/) | Bash-friendly one-shot and interactive client binary |
| TypeScript / JavaScript | [`typescript/`](typescript/) | Protocol v1 Node-API SDK with canonical JSON and opaque Raw APIs |
| C# / .NET | [`dotnet/`](dotnet/) | Shared-core C ABI adapter with protected Raw and canonical JSON APIs |
| Python | [`python/`](python/) | Async core-backed SDK; Smithy API and value constants generated from the canonical model |
| Go | [`go/`](go/) | Context-aware protected and raw CGO binding over the shared native ABI |
| Java | [`java/`](java/) | Async FFM client with generated Smithy API |
| Kotlin | [`kotlin/`](kotlin/) | Coroutine client with generated Smithy API |
| C | [`c/`](c/) | Protocol v1 protected C17 ABI over the shared core |
| C++ | [`cpp/`](cpp/) | Protocol v1 C++20 RAII adapter over the C ABI |
| Swift | [`swift/`](swift/) | Actor-based async SDK over the shared native ABI and generated Smithy API |
| Dart | [`dart/`](dart/) | Async `dart:ffi` client with generated contract |

The [value format](VALUE_FORMAT.md) specifies the implemented shared-core
format v1. The core owns Raw and canonical JSON serialization. Every binding's
formatted-value API uses that same canonical representation; Raw APIs preserve
opaque bytes exactly.

## Binding architecture

The shared layers have these responsibilities:

- `protocol/` defines and validates server-visible wire frames.
- `core/` handles transport, TLS, retries, raw operations, key derivation,
  compression, encryption, and formatted-value processing.
- implemented language packages convert native values and configuration into
  core types, expose runtime-appropriate asynchronous APIs, and clean up native
  resources.
- raw APIs accept exact protocol item IDs and values and bypass formatted-value
  processing.

Language adapters must not implement their own wire framing, retry semantics,
key derivation, compression, encryption, or value containers. Extend the shared
core when a binding needs shared behavior.

The .NET package uses the shared native ABI for transport, protocol work, and
protected value operations. Its managed Raw and canonical JSON methods delegate
value derivation and formatting to the shared core.

## Package commands and entry points

Run each command from the listed package directory.

| Package | Validation command | Reserved package surface |
|---|---|---|
| C | `cmake -S . -B target/build && cmake --build target/build` | `include/openkache/client.h` |
| C++ | `cmake -S . -B target/build && cmake --build target/build` | `include/openkache/client.hpp` |
| Dart | `dart analyze` | `lib/openkache.dart`, generated contract |
| Go | `go vet ./... && go test ./... && go build ./...` | Context-aware protected client and generated Smithy API |
| Java | `mvn package` | `src/main/java/io/openkache/client/` |
| Kotlin | `gradle build` | `src/main/kotlin/io/openkache/client/` |
| CLI | `cargo build --release -p openkache-cli` | `openkache-cli` binary |
| Python | `python -m compileall src && python -m build` | `src/openkache/__init__.py`, generated Smithy API under `_generated/` |
| Swift | `swift build` | `Sources/OpenKache/OpenKache.swift`, SwiftPM-generated Smithy API |

Native linkage for C, C++, Python, Swift, Java, Kotlin, and Dart is supplied by
the shared `ffi` native library built from `clients/core`; see each package
README for package-specific linker or packaging configuration. Artifact
distribution for each binding is selected by its package build and release
matrix.

Go, Java, Kotlin, and Dart contract sources are generated build outputs and are
intentionally ignored by Git. Go consumers run `go generate`; Maven and Gradle
invoke the generator for Java and Kotlin, while Dart builds run
`OPENKACHE_GENERATION_TARGET=dart ./generate.ts` first. Release workflows
regenerate the same outputs.

## Shared configuration

The C, C++, Python, and Swift packages use the native ABI exported by
`clients/core`. Their adapters only marshal native values, expose their
language-appropriate lifecycle, and own result handles; protocol, retry, TLS,
value-format, and protection behavior remain in the core. Every operation,
result, state, limit, and value-format identifier is generated from the two
scoped Smithy models for each package's build output, keeping all bindings
aligned without hand-maintained constants. The wire model in
[`../protocol/model/openkache.smithy`](../protocol/model/openkache.smithy)
contains only values the server must understand; the client model in
[`model/openkache.smithy`](model/openkache.smithy) owns adapter defaults, API
shapes, native ABI identifiers, and value-format metadata. The client entry
point [`generate.ts`](generate.ts) owns client extraction/rendering and
combines them for SDK outputs; its only protocol dependency is the wire
contract module [`../protocol/wire.ts`](../protocol/wire.ts).

The TypeScript release package includes Linux x64 and ARM64 Node-API adapters.
See each implemented package README for accepted configuration fields, platform
requirements, and packaging commands.
