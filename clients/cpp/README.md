# OpenKache C++ client

The C++ package targets C++20 and is a header-only RAII layer over the shared C
ABI. `openkache::Client` is movable, releases its native worker automatically,
and exposes byte-span and `std::string_view` overloads for protected cache
operations. CMake propagates the C++20 requirement only through the imported
target; it does not change the consuming project's global standard.
Projects using C++23 or newer can consume the same target; C++17 is not a
supported minimum because the public API uses `std::span`.
`Connect_Options::transport` selects verified QUIC (the source-compatible
default), verified TLS-over-TCP, or one of the explicit TLS-preserving
insecure variants. Both profiles use TLS 1.3, `openkache/1`, and
`X25519MLKEM768`, with identical v1 frame bytes. Non-default selectors require
the additive `openkache_client_connect_transport` symbol; older native
libraries report a clear unsupported-selector error through the runtime symbol
probe.

## Build

Build the native ABI first, then configure this package. CMake generates the
Smithy C contract into the build tree; generated contract files are not
checked into the source repository:

```bash
cargo build --manifest-path ../../Cargo.toml \
  -p openkache-client-core --no-default-features --features ffi --release
cmake -S . -B target/build \
  -DOPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC=/path/to/libopenkache_client_core.a
cmake --build target/build
```

For a shared build, use
`-DOPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED=/path/to/libopenkache_client_core.so`.
The legacy `OPENKACHE_CLIENT_NATIVE_LIBRARY` option remains accepted for
single-library builds. If Bun or the Smithy CLI is not available, pass a
previously generated header with
`-DOPENKACHE_CLIENT_SMITHY_CONTRACT_HEADER=/path/to/smithy_contract.h`.

Install and consume the package with CMake:

```bash
cmake --install target/build --prefix /path/to/prefix
```

```cmake
find_package(OpenKacheClient CONFIG REQUIRED)
target_link_libraries(app PRIVATE OpenKache::ClientCpp)
```

Use `OpenKache::ClientCppShared` or `OpenKache::ClientCppStatic` when the
linkage mode must be explicit. The C++ targets remain header-only; the shared
Rust/C core is the native binary.

## API

Construct `openkache::Connect_Options` with the server address and, when
protection is wanted, a persistent 32-byte data-protection key, then call
`openkache::Client::connect`. Omitting the key selects unprotected formatted
values while retaining Item ID derivation. A DER/PEM trust
certificate is optional; an empty buffer uses system roots. `get` returns
`std::optional<Bytes>`, `set` returns `Set_Outcome`, and `remove` reports
whether a value existed. `Set_Options` supports conditional writes,
namespace-inherited or explicit expiration, and evictable or
eviction-protected items; a non-empty `ttl_ms` without an explicit mode is
accepted as the convenience `Explicit_Ttl` shorthand. `get_raw`, `set_raw`,
and `remove_raw` expose exact `0..=32`-byte item-ID operations without value
protection. `namespace_open`, `namespace_update_policy`, and
`namespace_delete` expose the server-assigned namespace lifecycle as
transitional out-of-band control-plane shapes with legacy, non-normative
revision fields; they are not stable-v1 data-plane operations. `STATS` and
`SYNC` are likewise transitional experimental maintenance operations and are
disabled by default. Enable `enable_experimental_api = true` explicitly and
coordinate exact revision `draft-2026-08-19.4` out of band as described in
[`protocol/EXPERIMENTAL.md`](../../protocol/EXPERIMENTAL.md) before sending
them; the revision is not negotiated on the wire. Transport and validation
failures throw `openkache::Error`. Logical `std::span` and `std::string_view`
keys cross the ABI with their generated `Bytes` or `Text` discriminator; the
shared core performs canonical key encoding. ABI v1 requests use the
`poll`/`wait`/`free` request lifecycle and preserve
`Unknown_Mutation_Error` or `Canceled_Error` categories. Complete raw SET
policy flags and namespace/scoped operations have no request-handle entry
point, so the adapter drains their synchronous native result at a documented
safe completion boundary.

Formatted writes use automatic level-1 Zstandard compression by default and
retain a completed frame only when it is smaller. Set
`Connect_Options::compression_enabled` to `false` for an explicit
uncompressed opt-out.

The C++ layer does not duplicate protocol or protection logic. Its operation
and outcome values come from the C ABI, whose Smithy-derived constants live in
the shared core include directory.
