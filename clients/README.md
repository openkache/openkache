# OpenKache client SDKs

OpenKache keeps transport, protocol framing, compression, and encryption in the
Rust client. Language packages should remain thin adapters over its stable C
ABI unless a platform cannot load the native library.

## Status

| Language | Path | Status |
|---|---|---|
| Rust | `rust/` | Implemented core client |
| TypeScript | `typescript/` | Implemented Bun binding |
| C# / .NET | `dotnet/` | Implemented managed client |
| Python | `python/` | Package scaffold |
| JavaScript | `javascript/` | Package scaffold |
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

- native library discovery and loading;
- conversion between language values and ABI byte buffers;
- asynchronous execution appropriate to the runtime;
- deterministic client and result cleanup;
- idiomatic errors and package-level API names.

Do not duplicate QUIC, wire framing, hashing, compression, encryption, or
certificate behavior outside `rust/`. Extend the Rust ABI first when a binding
needs new core behavior.

## Commands

Build the Rust shared library from this directory:

```bash
cargo build --manifest-path rust/Cargo.toml --features ffi --release
```

Each scaffold README lists its package-manager validation command. Those
commands validate package structure only until the corresponding binding is
implemented.

## Configuration

Implemented native bindings require the Rust shared library for the target
platform. Packaging and library-discovery conventions are intentionally left
open in the scaffolds so they can be chosen with each ecosystem's release
workflow.
