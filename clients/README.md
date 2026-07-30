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
| Shared core | [`core/`](core/) | Protocol v3 raw and protected engine; protected values still use a pre-v1 format |
| Rust | [`rust/`](rust/) | Protocol v3 end-user SDK over the shared core |
| TypeScript / JavaScript | [`typescript/`](typescript/) | Protocol v3 Node-API SDK for Node.js, Bun, and Deno |
| C# / .NET | [`dotnet/`](dotnet/) | Standalone raw protocol v2 client |
| Python | `python/` | Package scaffold |
| Go | `go/` | Package scaffold |
| Java | `java/` | Package scaffold |
| Kotlin | `kotlin/` | Package scaffold |
| C | `c/` | Package scaffold |
| C++ | `cpp/` | Package scaffold |
| Swift | `swift/` | Package scaffold |
| Dart | `dart/` | Package scaffold |

A scaffold contains registry metadata and a reserved source layout. It does not
connect to OpenKache or expose cache operations.

The [value format](VALUE_FORMAT.md) specifies the planned formatted API v1.
Current Rust and TypeScript protection and envelope bytes predate that
specification and have no compatibility guarantee. Migration status belongs in
this table; byte-level v1 requirements belong only in the value-format
specification.

## Binding architecture

The shared layers have these responsibilities:

- `protocol/` defines and validates server-visible wire frames.
- `core/` handles transport, TLS, retries, raw operations, key derivation,
  compression, encryption, and formatted-value processing.
- implemented language packages convert native values and configuration into
  core types, expose runtime-appropriate asynchronous APIs, and clean up native
  resources.
- raw APIs accept exact protocol item keys and values and bypass formatted-value
  processing.

Language adapters must not implement their own wire framing, retry semantics,
key derivation, compression, encryption, or value containers. Extend the shared
core when a binding needs shared behavior.

The managed .NET client predates this boundary. Do not extend its duplicated
protocol v2 transport; replace it with a core-backed protocol v3 adapter when
the protected .NET API is implemented.

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
| Python | `python -m compileall src && python -m build` | `src/openkache/__init__.py` |
| Swift | `swift build` | `Sources/OpenKache/OpenKache.swift` |

Native linkage, artifact distribution, and runtime integration for scaffolds
are intentionally undefined until each binding is implemented.

## Shared configuration

The TypeScript release package includes Linux x64 and ARM64 Node-API adapters.
See each implemented package README for accepted configuration fields, platform
requirements, and packaging commands.
