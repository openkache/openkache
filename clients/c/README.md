# OpenKache native C ABI

This directory packages the maintained Gate 0 C17 ABI used by the native
adapters.  It is intentionally a thin FFI boundary: transport, request
admission, key mapping, value envelopes, and result ownership remain in the
shared Rust core, while the C++ facade supplies typed-key and
StructuredValue-CBOR-v1 conversion.

The installed C ABI exposes `openkache_client_gate0_connect`,
`openkache_client_gate0_get`, `openkache_client_gate0_set`,
`openkache_client_gate0_delete_value`, and `openkache_client_gate0_close`,
plus result ownership helpers.  The generated shared-core contract is used
only to build the package-private forwarding library and is not installed.
Callers must not invoke raw, JSON, Exact Item ID, namespace, experimental,
cancellation, TTL, retry, or certificate controls.

Pointers and lengths are explicit; result payloads are borrowed until
`openkache_client_gate0_result_free`, and a connected handle is transferred
with `openkache_client_gate0_result_take_client` before
`openkache_client_gate0_close`.

## Build and install

Build the native core with the `ffi` feature and pass either an explicit static
or shared library to CMake:

```bash
cargo build --manifest-path ../../Cargo.toml \
  -p openkache-client-core --no-default-features --features ffi --release
cmake -S . -B target/build \
  -DOPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC=/path/to/libopenkache_client_core.a
cmake --build target/build
cmake --install target/build --prefix /path/to/prefix
```

The configure/build pair is the C17 package smoke check.  It regenerates the
private Smithy contract, compiles the forwarding library, and validates the
imported C target; generated declarations are not copied to the install tree.
An installed consumer can use the `find_package` snippet below.

If Bun or the Smithy CLI is unavailable, set
`OPENKACHE_CLIENT_SMITHY_CONTRACT_HEADER` to a generated
`smithy_contract.h`.  Downstream CMake projects use:

```cmake
find_package(OpenKacheClient CONFIG REQUIRED)
target_link_libraries(app PRIVATE OpenKache::ClientC)
```

The CMake package also installs `openkache-client.pc` metadata when a shared
or static native library is supplied.

## Development profile

The maintained C++ facade fixes TLS 1.3, ALPN `openkache/1`,
`X25519MLKEM768`, and the `DevelopmentTrust` profile.  DevelopmentTrust
disables certificate and hostname verification but retains TLS encryption and
does not permit plaintext fallback.  It is **development only — do not use
this trust profile in production**.  Production trust roots and certificate
configuration require a future facade revision.
