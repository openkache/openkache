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
protection is wanted, a persistent 32-byte client root key, then call
`openkache::Client::connect`. Omitting both the key and `encryption` selects
unprotected formatted values; omitting only `encryption` selects Robust when
the key is present. An explicit Compact or Robust profile without a key is
rejected. A DER/PEM trust
certificate is optional; an empty buffer uses system roots. `get` returns
`std::optional<Bytes>`, `set` returns `Set_Outcome`, and `remove` reports
whether a value existed. `Set_Options` supports conditional writes,
namespace-inherited or explicit expiration, and evictable or
eviction-protected items; a non-empty `ttl_ms` without an explicit mode is
accepted as the convenience `Explicit_Ttl` shorthand. `get_raw`, `set_raw`,
and `remove_raw` expose exact opaque item-ID operations (up to the 32-byte
wire maximum) without value
protection. `namespace_open`, `namespace_update_policy`, and
`namespace_delete` expose the server-assigned namespace lifecycle and
optimistic revisions. Transport and validation failures throw
`openkache::Error`.

`Connect_Options::key_format` selects the client-local application-key mapping.
`Key_Format::Hash` is the default. `Key_Format::Byte_Key_Or_Hash` applies only
to byte-span overloads: it preserves byte keys up to the wire Item ID limit and
hashes longer keys. Calling a `std::string_view` Text overload with that format
is invalid and throws `openkache::Error`.

The C++ layer does not duplicate protocol or protection logic. Its operation
and outcome values come from the C ABI, whose Smithy-derived constants live in
the shared core include directory.

Set `Connect_Options::key_format` to `Key_Format::Byte_Key_Or_Hash` to preserve
byte keys up to 32 bytes as Item IDs and hash longer byte keys.
