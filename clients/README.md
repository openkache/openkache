# OpenKache client SDKs

OpenKache keeps transport, protocol framing, compression, and encryption in the
Rust client. Language packages should remain thin adapters over the Rust
transport instead of implementing the wire or security protocols again.

## Status

| Language | Path | Status |
|---|---|---|
| Rust | `rust/` | Implemented core client |
| TypeScript / JavaScript | `typescript/` | Implemented Node.js SDK |
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

- native helper discovery and lifecycle;
- conversion between language values and helper byte buffers;
- asynchronous execution appropriate to the runtime;
- deterministic client and helper cleanup;
- idiomatic errors and package-level API names.

Do not duplicate QUIC, wire framing, hashing, compression, encryption, or
certificate behavior outside `rust/`. Extend the Rust transport helper when a
binding needs new core behavior.

Object APIs use the shared [OpenKache value envelope](VALUE_FORMAT.md).
Raw byte APIs remain available for application-owned cross-language formats.

## Commands

Build the Rust helper from this directory:

```bash
cargo build --manifest-path rust/Cargo.toml --bin openkache-client-helper --release
```

Each scaffold README lists its package-manager validation command. Those
commands validate package structure only until the corresponding binding is
implemented.

## Configuration

The TypeScript release package includes its statically linked Rust helper under
`target/native/`. Other scaffolded clients choose their runtime integration
when implemented.
