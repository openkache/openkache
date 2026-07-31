# OpenKache C++ client

The C++ package is a C++20 RAII layer over the shared C ABI. `openkache::Client`
is movable, releases its native worker automatically, and exposes byte-span
and `std::string_view` overloads for protected cache operations.

## Build

Build the shared native ABI first, then configure this package:

```bash
cargo build --manifest-path ../../Cargo.toml \
  -p openkache-client-core --no-default-features --features ffi --release
cmake -S . -B target/build \
  -DOPENKACHE_CLIENT_NATIVE_LIBRARY=/path/to/libopenkache_client_core.a
cmake --build target/build
```

The CMake target validates headers without a native library. Supplying
`OPENKACHE_CLIENT_NATIVE_LIBRARY` propagates the core linkage through
`OpenKache::ClientCpp`.

## API

Construct `openkache::Connect_Options` with the server address and 32-byte
data-protection key, then call `openkache::Client::connect`. A DER/PEM trust
certificate is optional; an empty buffer uses system roots. `get` returns
`std::optional<Bytes>`, `set` returns `Set_Outcome`, and `remove` reports
whether a value existed. `get_raw`, `set_raw`, and `remove_raw` expose exact
32-byte item-ID operations without value protection. Transport and validation
failures throw `openkache::Error`.

The C++ layer does not duplicate protocol or protection logic. Its operation
and outcome values come from the C ABI, whose Smithy-derived constants live in
the shared core include directory.
