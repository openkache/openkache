# OpenKache C client

The C package is a small C17 ABI over the shared Rust client core. It provides
protected `PING`, `GET`, `SET`, `DELETE`, `STATS`, `SYNC`, and namespace
management operations while
the core owns QUIC, TLS, retries, framing, compression, encryption, and the
worker lifecycle.

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

`include/openkache/client.h` includes the compatibility
`clients/core/include/openkache/client_abi.h`, which consumes the generated
Smithy ABI header and retains source-compatible aliases for the opaque client
handle, result ownership rules, operation and outcome discriminators, and
buffer-based protected and exact-item-ID calls. Result payloads are borrowed until
`openkache_client_result_free`; copy them before freeing the result. A
connected handle is transferred with `openkache_client_result_take_client` and
released with `openkache_client_free`.

`openkache_client_connect_ex` is the flat generated-binding entry point;
`openkache_client_connect_with_options` is a named-field convenience wrapper.
An empty trust buffer selects system roots. `openkache_client_execute` accepts
one complete canonical v1 key item and derives its protected Item ID, while
`openkache_client_execute_raw` accepts an item ID of up to 32 bytes and sends opaque
values unchanged. C callers that construct protected keys directly should use
the `Integer`, `Text`, and `Bytes` rules in
[`../KEY_FORMAT.md`](../KEY_FORMAT.md).

`openkache_client_namespace_open`, `openkache_client_namespace_update_policy`,
and `openkache_client_namespace_delete` manage server-assigned namespaces.
Namespace results carry the canonical descriptor payload; use
`openkache_client_namespace_descriptor_decode` to obtain a typed descriptor
without reimplementing the wire parser in the application.

Operation and value-format constants in the generated
`openkache/smithy_contract.h` are sourced at build/package time from the client
model [`../model/openkache.smithy`](../model/openkache.smithy) and wire model
[`../../protocol/model/openkache.smithy`](../../protocol/model/openkache.smithy).
C and C++ adapters therefore share the same operation numbers, limits, and
value-format identifiers without checking generated files into the repository.
