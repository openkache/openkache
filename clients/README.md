# OpenKache client SDKs

OpenKache keeps connection management, protocol operations, application-key derivation,
compression, and encryption in `core/`. Language packages remain thin adapters over that shared
implementation instead of composing transport and protection themselves.

## Status

| Language | Path | Status |
|---|---|---|
| Shared core | `core/` | Implemented raw and protected client engine |
| Rust | `rust/` | Implemented ergonomic end-user SDK |
| TypeScript / JavaScript | `typescript/` | Node.js, Bun, and Deno Node-API SDK |
| C# / .NET | `dotnet/` | Implemented raw managed client; protected core adapter deferred |
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

Do not duplicate connection lifecycle, retries, cache-operation semantics, QUIC, wire framing,
keyed derivation, compression, encryption, or certificate behavior outside `core/`. Extend the
shared core and add only the smallest runtime- or ABI-specific adapter when a binding needs new
behavior.

The existing raw managed .NET client predates this boundary. Do not extend its duplicated
transport or protocol implementation; replace it with a thin core-backed adapter when the
protected .NET API is implemented.

The next formatted API revision is specified by the shared
[OpenKache value format](VALUE_FORMAT.md).
Low-level raw APIs remain available for callers that already own exact protocol values.

Each scaffold README lists its package-manager validation command. Those
commands validate package structure only until the corresponding binding is
implemented.

## Configuration

The TypeScript release package includes Linux x64 and ARM64 Node-API adapters
for Node.js, Bun, and Deno under `target/native/`. Other scaffolded clients
choose their runtime integration when implemented.
