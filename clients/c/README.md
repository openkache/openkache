# OpenKache C client

The C package is a small C17 ABI over the shared Rust client core. It provides
protected `PING`, `GET`, `SET`, `DELETE`, `STATS`, and `SYNC` operations while
the core owns QUIC, TLS, retries, framing, compression, encryption, and the
worker lifecycle.

## Build

Build the native ABI from the workspace with the `ffi` feature, then point
CMake at the resulting library:

```bash
cargo build --manifest-path ../../Cargo.toml \
  -p openkache-client-core --no-default-features --features ffi --release
cmake -S . -B target/build \
  -DOPENKACHE_CLIENT_NATIVE_LIBRARY=/path/to/libopenkache_client_core.a
cmake --build target/build
```

Without `OPENKACHE_CLIENT_NATIVE_LIBRARY`, the CMake target still validates and
installs the headers. Supplying the library adds native linkage to
`OpenKache::ClientC`.

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
An empty trust buffer selects system roots. `openkache_client_execute` derives
protected item IDs from application keys, while `openkache_client_execute_raw`
requires a 32-byte item ID and sends opaque values unchanged.

Operation and value-format constants in
`clients/core/include/openkache/smithy_contract.h` are generated from the
canonical Smithy model. C and C++ adapters therefore share the same operation
numbers, limits, and value-format identifiers.
