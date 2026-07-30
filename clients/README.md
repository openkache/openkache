# OpenKache client SDKs

OpenKache keeps transport, protocol framing, compression, and encryption tools in
`core/`. Language packages should remain thin adapters over that low-level crate
instead of implementing the wire or security protocols again.

## Status

| Language | Path | Status |
|---|---|---|
| Shared core | `core/` | Implemented low-level QUIC and protocol client |
| Rust | `rust/` | Implemented ergonomic end-user SDK |
| TypeScript / JavaScript | `typescript/` | Node.js, Bun, and Deno Node-API SDK |
| C# / .NET | `dotnet/` | Implemented managed client |
| Python | `python/` | Package scaffold |
| Go | `go/` | Package scaffold |
| Java | `java/` | Package scaffold |
| Kotlin | `kotlin/` | Package scaffold |
| C | `c/` | Package scaffold |
| C++ | `cpp/` | Package scaffold |
| Swift | `swift/` | Package scaffold |
| Dart | `dart/` | Package scaffold |

A package scaffold contains registry metadata and the conventional source
layout only. It does not connect to OpenKache or expose cache operations yet.

## Binding architecture

Future bindings should own only language-native concerns:

- native adapter discovery and lifecycle;
- conversion between language values and Rust byte buffers;
- asynchronous execution appropriate to the runtime;
- deterministic client cleanup;
- idiomatic errors and package-level API names.

Do not duplicate QUIC, wire framing, hashing, compression, encryption, or
certificate behavior outside `core/`. Extend the shared core and add only the
smallest runtime-specific adapter when a binding needs new behavior.

Object APIs use the shared [OpenKache value envelope](VALUE_FORMAT.md).
Raw byte APIs remain available for application-owned cross-language formats.

Each scaffold README lists its package-manager validation command. Those
commands validate package structure only until the corresponding binding is
implemented.

## Configuration

The TypeScript release package includes Linux x64 and ARM64 Node-API adapters
for Node.js, Bun, and Deno under `target/native/`. Other scaffolded clients
choose their runtime integration when implemented.
