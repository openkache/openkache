# OpenKache client SDKs

OpenKache client packages implement the frozen Gate 0 (`v1-gate0`) client
contract. The published Rust client is the [`openkache`](rust/) crate; it
provides the maintained `connect`/`get`/`set`/`delete`/`close` facade and
`StructuredValue-CBOR-v1` values. The [`openkache-server`](../server/) package
is a separate release artifact. The other Rust crates in this directory are
internal implementation crates used to build the published packages and are
not end-user dependencies. Release the client from a reviewed
`client-v<version>` tag through the `Release OpenKache Rust crate` workflow with
`package=client`; do not publish the internal core, value, CLI, or native
adapter crates.

## Install a maintained SDK

These are the three supported registry packages for the maintained Gate 0
facade. They share the same logical operations and value model, but retain
idiomatic language APIs.

| Language | Registry | Install | API style |
|---|---|---|---|
| TypeScript / JavaScript | [npm](https://www.npmjs.com/package/openkache) | `npm install openkache` or `bun add openkache` | Promise-based |
| Python | [PyPI](https://pypi.org/project/openkache/) | `python -m pip install openkache` | Synchronous |
| Rust | [crates.io](https://crates.io/crates/openkache) | `cargo add openkache` | Async |

Start the local preview server from the
[server quick start](../README.md#-quick-start), then use
`127.0.0.1:4433` as the endpoint. The fixed development TLS profile disables
certificate verification and is intended only for local development.

Each package README is the source for its language-facing API:

- [TypeScript / JavaScript](typescript/README.md) — Node.js, Bun, and Deno
- [Python](python/README.md) — synchronous Gate 0 facade
- [Rust](rust/README.md) — published async crate
- [CLI](cli/README.md) — source-built Gate 0 command-line client with a
  configurable compatibility profile

## Documentation

Shared client topics are documented here:

| Topic | Reference |
|---|---|
| SDK inventory, implementation status, and binding boundaries | This README |
| Maintained binding architecture, request engine, native conversion, and local policies | [Client implementation guide](CLIENT.md) |
| Typed keys, namespace hashing, and public key mapping | [Key format](KEY_FORMAT.md) |
| Formatted value envelope and compression selection | [Value format](VALUE_FORMAT.md) |
| Security goals, threat model, value protection, and key lifecycle | [Security model](../SECURITY_MODEL.md) |
| Public boundary fixtures for v1 interoperability | [`fixtures/`](fixtures/) |
| Cross-language value model, native mappings, and structured-value profiles | [Value project](value/SPEC.md) |
| Wire framing, operations, transport profiles, limits, and ambiguous outcomes | [Wire protocol](../protocol/SPEC.md) |
| Language API, build, packaging, and runtime configuration | The implemented package's README |

Package documentation links to these references instead of restating shared
formats or protocol behavior.

## SDK status

| Package | Path | Implementation |
|---|---|---|
| Shared core | [`core/`](core/) | Internal Rust engine and native ABI; not an end-user dependency |
| Value model | [`value/`](value/) | Internal `Value` algebra and bounded structured payload codec |
| Rust | [`rust/`](rust/) | Published `openkache` crate with the maintained five-operation facade |
| CLI | [`cli/`](cli/) | Bash-friendly Gate 0 client by default, with a configurable compatibility profile |
| TypeScript / JavaScript | [`typescript/`](typescript/) | Node-API SDK with typed keys and a structured `set`/`get` facade |
| C# / .NET | [`dotnet/`](dotnet/) | Compatibility adapter over the internal native ABI; not the Gate 0 Rust facade |
| Python | [`python/`](python/) | Maintained synchronous five-operation Gate 0 facade |
| Go | [`go/`](go/) | Compatibility adapter over the internal native ABI; not the Gate 0 Rust facade |
| Java | `java/` | Package scaffold |
| Kotlin | `kotlin/` | Package scaffold |
| C | [`c/`](c/) | Maintained Gate 0 C17 ABI over the internal native core |
| C++ | [`cpp/`](cpp/) | Maintained Gate 0 C++20 adapter over the C ABI |
| Swift | [`swift/`](swift/) | Compatibility adapter over the internal native ABI; not the Gate 0 Rust facade |
| Dart | `dart/` | Package scaffold |

Java, Kotlin, and Dart currently contain registry metadata and scaffold source
layouts only. They do not connect to OpenKache or expose cache operations yet.

The [key](KEY_FORMAT.md), [value model](value/SPEC.md), and [value
envelope](VALUE_FORMAT.md) documents, together with the
[security model](../SECURITY_MODEL.md), define the language-independent v1
client contracts. The [client implementation guide](CLIENT.md) describes how
maintained bindings share one internal engine without making implementation
crate APIs a requirement for third-party clients. The value envelope carries
the selected value profile opaquely; the server does not interpret it. The
maintained Gate 0 facade uses the common structured value model and exposes no
JSON, raw-byte, Exact Item ID, or caller-owned-v0 operation.

Package documentation intentionally describes the logical value contract
rather than the shared core's concrete payload profile or selector assignment.
Those details remain in the protocol and value-profile references for
interoperability implementers.

The documents above are the frozen v1 contract. Package READMEs describe
whether each package implements the maintained Gate 0 facade, is a compatibility
adapter, or is an internal implementation crate. The CLI README also documents
the profile boundary: its default `gate0` mode shares keys and structured
values with the Rust client and the prototype's native QUIC frontend, while
`configured` keeps the broader raw-value and TLS controls.

`EXPERIMENTAL_STATS` and `EXPERIMENTAL_SYNC` are out-of-band experimental
maintenance operations and are disabled by default. A server exposes them only when configured with
`enable_experimental_api = true` and
`experimental_api_revision = "draft-2026-08-19.4"`. Clients and servers must
coordinate that exact revision out of band before sending them; the revision is
not negotiated on the wire. See
[`protocol/EXPERIMENTAL.md`](../protocol/EXPERIMENTAL.md). Namespace-open,
policy-update, and namespace-delete methods that appear in generated bindings
are out-of-band control-plane shapes only; they are not stable v1 data-plane
operations. Package examples must label those methods as experimental and must
not treat generated Smithy metadata as a stable opcode or status registry.

| Area | v1 contract | Implementation | Status |
|---|---|---|---|
| Item ID | `0..=32` bytes | Exact `0..=32` bytes in core/FFI and maintained adapters | Implemented |
| Integer key | Signed `i64` deterministic-CBOR subset | Core and maintained adapters enforce signed `i64`; CBOR bignum tags are rejected | Implemented |
| `NamespaceHash` input | Domain string \| namespace ID \| canonical key bytes | Namespace-bound BLAKE3 uses the documented domain, namespace ID, and canonical key bytes | Implemented |
| Item ID/value keys | Independent Item ID root and value-key rotation | `builder_with_keyring`/`with_item_id_root_and_keyring` keep identity and value keys independent; legacy root builders remain coupled for compatibility | Implemented |
| Item ID mapping | `NamespaceHash` plus explicit `PublicKeyOrHash` | Published facade uses `NamespaceHash`; internal adapters retain explicit raw/Exact compatibility paths | Implemented |
| Public Item ID root + protected value | Zero/public root may be paired with a separate value keyring | Explicit keyring builders and the ABI v1 keyring entry point accept a public zero root; the base ABI v1 path keeps its coupled semantics | Implemented |
| Key API shape | Exact Item ID accepts `0..=32` bytes; mapped profiles choose the output length | Core and maintained adapters accept the complete `0..=32` Item ID range | Implemented |
| Structured values | Common cross-language value model | Maintained mapped `set`/`get` use `StructuredValue-CBOR-v1` | Implemented |
| Maintained compression | Gate 0 fixes `Uncompressed` (`gate0Compression: 0`); internal and compatibility adapters use automatic level-1 Zstandard | Gate 0 packages emit selector `0x10` (`Uncompressed`, `Unprotected`, `StructuredValue-CBOR-v1`) with no compression option; compatibility adapters retain explicit compression controls | Implemented |
| Value-format `maxVu128Bytes` | Unsigned 64-bit values, at most 9 bytes | Smithy client model/generator emits 9-byte metadata | Implemented |

The frozen specifications are the source of truth for the v1 contract.
Generated Smithy enums and wire metadata may contain compatibility or
out-of-band operations and legacy status values; they are not evidence of
stable-v1 opcode/status assignments. Stable-v1 validators and emitters MUST
follow [`protocol/SPEC.md`](../protocol/SPEC.md). Package-specific generated
models remain the source of a compatibility adapter's ABI, not the v1 wire,
key, or value contract.

## Binding architecture

Maintained language packages convert native values and runtime behavior at the
edge while delegating transport, protocol, retry classification, key mapping,
and formatted-value processing to `clients/core`. The [client implementation
guide](CLIENT.md) defines that shared boundary. The [core README](core/README.md)
documents the current Rust crate and native ABI; the
[core target design](core/TARGET.md) records internal implementation details;
it is not an end-user API.
Each package README documents its language-facing API and platform integration.

## Scaffold commands and entry points

Run each command from the listed package directory. These commands validate
package structure and buildability.

The C and C++ commands require an `openkache-client-core` native library built
with the `ffi` feature; pass its static or shared path through
`OPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC` or
`OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED` as shown in the package READMEs.

| Package | Validation command | Reserved package surface |
|---|---|---|
| C | `cmake -S . -B target/build -DOPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC=/path/to/libopenkache_client_core.a && cmake --build target/build` | `include/openkache/client.h` |
| C++ | `cmake -S . -B target/build -DOPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC=/path/to/libopenkache_client_core.a && cmake --build target/build` | `include/openkache/client.hpp` |
| Dart | `dart analyze` | `lib/openkache.dart` |
| Go | `go generate && go vet ./... && go build ./...` | Context-aware protected client and generated Smithy API |
| Java | `mvn package` | `src/main/java/io/openkache/client/package-info.java` |
| Kotlin | `gradle build` | `src/main/kotlin/io/openkache/client/OpenKache.kt` |
| CLI | `cargo build --release -p openkache-cli` | `openkache-cli` binary |
| Python | `python -m compileall src && python -m build` | `src/openkache/__init__.py`, generated Smithy API under `_generated/` |
| Swift | `swift build` | `Sources/OpenKache/OpenKache.swift`, SwiftPM-generated Smithy API |

Native linkage for C, C++, Python, and Swift is supplied by the shared `ffi`
native library built from `clients/core`; see each package README for the
package-specific linker or packaging configuration. Artifact distribution for
the remaining scaffolds is intentionally undefined until those bindings are
implemented.

## Generated client contract

Operations, results, states, limits, maintained defaults, and format
identifiers for compatibility adapters are generated from two scoped Smithy
models. Generated output may include compatibility or out-of-band operations;
the frozen client and protocol specifications remain normative for Gate 0. The
wire model in
[`../protocol/model/openkache.smithy`](../protocol/model/openkache.smithy)
contains only values the server must understand; the client model in
[`model/openkache.smithy`](model/openkache.smithy) owns adapter defaults, API
shapes, native ABI identifiers, and value-format metadata. The stable
[`generate.ts`](generate.ts) entry point selects outputs and performs atomic
writes. Modules under [`generator/`](generator/) separately own Smithy
extraction, shared contract types, literal rendering, and language-specific
renderers. This keeps adding or changing one SDK renderer independent of the
other language backends while preserving one generated contract. Native ABI
and package responsibilities are specified by the [client implementation
guide](CLIENT.md).

The TypeScript release package includes Linux x64 and ARM64 Node-API adapters.
See each implemented package README for accepted configuration fields, platform
requirements, and packaging commands.
