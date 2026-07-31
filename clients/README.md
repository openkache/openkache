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
| TypeScript / JavaScript | [`typescript/`](typescript/) | Protocol v1 Node-API SDK; canonical JSON uses the shared core and a legacy envelope remains available for compatibility |
| C# / .NET | [`dotnet/`](dotnet/) | Standalone raw protocol v1 client |
| Python | [`python/`](python/) | Async core-backed SDK; Smithy API and value constants generated from the canonical model |
| Go | `go/` | Package scaffold |
| Java | `java/` | Package scaffold |
| Kotlin | `kotlin/` | Package scaffold |
| C | [`c/`](c/) | Protocol v1 protected C17 ABI over the shared core |
| C++ | [`cpp/`](cpp/) | Protocol v1 C++20 RAII adapter over the C ABI |
| Swift | [`swift/`](swift/) | Actor-based async SDK over the shared native ABI and generated Smithy API |
| Dart | `dart/` | Package scaffold |

Go, Java, Kotlin, and Dart currently contain registry metadata and reserved
source layouts only. They do not connect to OpenKache or expose cache
operations yet.

The [value format](VALUE_FORMAT.md) specifies the implemented shared-core
format v1. The core owns Raw and canonical JSON serialization; language
adapters convert native values into that shared logical model. TypeScript's
legacy metadata envelope remains a package-level compatibility detail; new
cross-language values should use its `set_json`/`get_json` API.

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

The managed .NET client predates this boundary. Do not extend its duplicated
protocol v1 transport; replace it with a core-backed adapter when the protected
.NET API is implemented.

## Scaffold commands and entry points

Run each command from the listed package directory. These commands validate
package structure only.

| Package | Validation command | Reserved package surface |
|---|---|---|
| C | `cmake -S . -B target/build && cmake --build target/build` | `include/openkache/client.h` |
| C++ | `cmake -S . -B target/build && cmake --build target/build` | `include/openkache/client.hpp` |
| Dart | `dart analyze` | `lib/openkache.dart` |
| Go | `go vet ./... && go build ./...` | `doc.go` |
| Java | `mvn package` | `src/main/java/io/openkache/client/package-info.java` |
| Kotlin | `gradle build` | `src/main/kotlin/io/openkache/client/OpenKache.kt` |
| CLI | `cargo build --release -p openkache-cli` | `openkache-cli` binary |
| Python | `python -m compileall src && python -m build` | `src/openkache/__init__.py`, generated Smithy API under `_generated/` |
| Swift | `swift build` | `Sources/OpenKache/OpenKache.swift`, generated Smithy API |

Native linkage for C, C++, Python, and Swift is supplied by the shared `ffi`
native library built from `clients/core`; see each package README for the
package-specific linker or packaging configuration. Artifact distribution for
the remaining scaffolds is intentionally undefined until those bindings are
implemented.

## Shared configuration

The C, C++, Python, and Swift packages use the native ABI exported by
`clients/core`. Their adapters only marshal native values, expose their
language-appropriate lifecycle, and own result handles; protocol, retry, TLS,
value-format, and protection behavior remain in the core. Every operation,
result, state, limit, and value-format identifier is generated from the
Smithy model for each package's build output, keeping all bindings aligned
without hand-maintained constants.

The TypeScript release package includes Linux x64 and ARM64 Node-API adapters.
See each implemented package README for accepted configuration fields, platform
requirements, and packaging commands.
