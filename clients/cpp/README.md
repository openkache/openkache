# OpenKache C++ client

The C++ package targets C++20 and is a header-only RAII layer over the shared C
ABI. `openkache::Client` is movable, releases its native worker automatically,
and exposes byte-span and `std::string_view` overloads for protected cache
operations. CMake propagates the C++20 requirement only through the imported
target; it does not change the consuming project's global standard.
Projects using C++23 or newer can consume the same target; C++17 is not a
supported minimum because the public API uses `std::span`.

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
If Bun or the Smithy CLI is not available, pass a
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
