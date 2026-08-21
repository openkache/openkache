# OpenKache C client

The C package is a small C17 ABI over the shared Rust client core. It provides
protected `PING`, `GET`, `SET`, `DELETE`, and transitional administrative
operations while
the core owns QUIC-over-TLS, retries, framing, compression, encryption, and the
worker lifecycle. The current binding is QUIC-only; TLS-over-TCP is part of the
target maintained-client contract.

`STATS` and `SYNC` are transitional experimental maintenance operations and are
disabled by default. Enable `enable_experimental_api = true` explicitly and
coordinate exact revision `draft-2026-08-19.4` out of band as described in
[`protocol/EXPERIMENTAL.md`](../../protocol/EXPERIMENTAL.md) before sending
them; the revision is not negotiated on the wire. The generated
namespace-management functions are out-of-band WIP control-plane shapes; they
do not reserve stable v1 opcodes or public data-plane routes.

## Build

Build the native ABI from the workspace with the `ffi` feature, then point
CMake at the resulting library. CMake generates the Smithy C contract into
the build tree; it is intentionally not checked into the source repository:

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

Installable CMake and `pkg-config` metadata are produced when a native library
is supplied:

```bash
cmake --install target/build --prefix /path/to/prefix
pkg-config --cflags --libs openkache-client
pkg-config --static --cflags --libs openkache-client-static
```

Downstream CMake projects can use the installed package with
`find_package(OpenKacheClient CONFIG REQUIRED)` and
`target_link_libraries(app PRIVATE OpenKache::ClientC)`. Select
`OpenKache::ClientCShared` or `OpenKache::ClientCStatic` when the linkage mode
must be explicit.

## API

`include/openkache/client.h` includes the canonical
`clients/core/include/openkache/client_abi.h`, which defines an opaque client
handle, result ownership rules, operation and outcome discriminators, and
buffer-based protected and exact-item-ID calls. Result payloads are borrowed until
`openkache_client_result_free`; copy them before freeing the result. A
connected handle is transferred with `openkache_client_result_take_client` and
released with `openkache_client_free`.

`openkache_client_connect_ex` is the flat generated-binding entry point;
`openkache_client_connect_with_options` is a named-field convenience wrapper.
An empty trust buffer selects system roots. `openkache_client_execute` accepts
one complete canonical v1 key item and derives its protected Item ID, while
`openkache_client_execute_raw` accepts `0..=32`-byte exact item IDs and sends
opaque values unchanged. C callers that construct protected keys directly should use
the `Integer`, `Text`, and `Bytes` rules in
[`../KEY_FORMAT.md`](../KEY_FORMAT.md).

The ABI v6 connect functions keep their historical coupled
`data_protection_key` semantics. Bindings that need publicly derivable Item IDs
with protected values can probe `openkache_client_abi_version_v7()` and, when
it returns `7`, call `openkache_client_connect_with_options_v7`. The v7 options
reference the v6 transport settings but require an explicit Item-ID root and
an immutable array of value-key records; the legacy data-protection field must
be empty and is never reinterpreted. A zero-length Item-ID root selects the
public all-zero root, while value-key IDs and key material remain independent.

`openkache_client_namespace_open`, `openkache_client_namespace_update_policy`,
and `openkache_client_namespace_delete` expose those transitional
control-plane shapes when a private adapter enables them. Namespace results
carry the canonical descriptor payload; use
`openkache_client_namespace_descriptor_decode` to obtain a typed descriptor
without reimplementing the wire parser in the application.

Operation and value-format constants in the generated
`openkache/smithy_contract.h` are sourced at build/package time from the client
model [`../model/openkache.smithy`](../model/openkache.smithy) and wire model
[`../../protocol/model/openkache.smithy`](../../protocol/model/openkache.smithy).
C and C++ adapters therefore share the same operation numbers, limits, and
value-format identifiers without checking generated files into the repository.
