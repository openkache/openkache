# OpenKache C++ client

This package is the maintained C++20 Gate 0 client.  It is a header-only RAII
adapter over the generated C ABI and exposes exactly five operations:
`connect`, `get`, `set`, `remove` (the C++ spelling of `delete`), and `close`.
The adapter owns no protocol, key-derivation, or value-envelope implementation
outside the generated native boundary.

`openkache::Value` is a lossless tagged model.  It retains `Undefined` versus
`Null`, arbitrary-precision signed integers, Float16/32/64 raw bits,
byte/text distinction, ordered arrays, and ordered maps with scalar,
structurally unique keys.  `encode_structured_value` and
`decode_structured_value` implement one complete
`StructuredValue-CBOR-v1` item; malformed, truncated, trailing, non-UTF-8,
duplicate-key, and resource-limited inputs throw `openkache::Value_Error`.
`get` always returns a tagged `Get_Result`, so a stored `Null` or `Undefined`
cannot be confused with a missing item.

Native failures retain the shared FFI category through `Native_Error::category()`;
resource exhaustion is reported as `Error_Category::Resource_Exhausted`;
an admitted SET or DELETE whose response is lost raises
`Unknown_Mutation_Error`, and callers must not replay it automatically.

Structured values are bounded by `Value_Limits`: the default traversal ceiling
is 128 nested arrays/maps, and `MAX_ALLOWED_VALUE_DEPTH` (1024) is a hard
implementation maximum.  A caller may choose a lower `max_depth`, but larger
limits are rejected before parsing or encoding so recursive native stack usage
stays bounded.

## Build

Build the shared native ABI, then configure this package.  CMake generates the
Smithy contract into the build tree and does not check generated headers into
the source repository:

```bash
cargo build --manifest-path ../../Cargo.toml \
  -p openkache-client-core --no-default-features --features ffi --release
cmake -S . -B target/build \
  -DOPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC=/path/to/libopenkache_client_core.a
cmake --build target/build
```

The native core library is required at configure time; CMake rejects a
header-only or otherwise link-incomplete Gate 0 package. The installed package
includes the selected native library and the exported target carries its
transitive system libraries.

The configure/build pair is the package smoke check: it regenerates the C
contract, compiles the C++20 headers, and validates the imported CMake target.
For an installed-package check, configure a small consumer with
`find_package(OpenKacheClient CONFIG REQUIRED)` as shown below.

Install and consume the C++ target with:

```bash
cmake --install target/build --prefix /path/to/prefix
```

```cmake
find_package(OpenKacheClient CONFIG REQUIRED)
target_link_libraries(app PRIVATE OpenKache::Client)
```

The C++ target requires C++20 through its target usage requirements and does
not change the consuming project's global compiler mode.  The
`OpenKache::ClientCpp` language-specific target remains available; use
`OpenKache::ClientCppShared` or `OpenKache::ClientCppStatic` when linkage must
be explicit.  The C++ adapter remains header-only, with the package-private C
forwarding library and selected native core providing the runtime binary.

## Usage

```cpp
#include <openkache/client.hpp>

using namespace openkache;

Client client = Client::connect("127.0.0.1:4433");
const auto outcome = client.set(
    Typed_Key::text("greeting"),
    Value::text("hello"));
const Get_Result result = client.get(Typed_Key::text("greeting"));
if (result.is_found()) {
    const Value& value = result.value();
    // value remains lossless; Null and Undefined are separate kinds.
}
client.remove(Typed_Key::text("greeting"));
client.close(); // idempotent; the destructor closes as well
```

`Integer` keys are signed `i64`; text keys are valid UTF-8 with an explicit
length; byte keys preserve every byte, including empty and NUL bytes.  Every
operation emits one deterministic StructuredValue-CBOR key item before the
native FFI call.  Floating-point, Boolean, null, collection, invalid-UTF-8,
and out-of-range integer keys are not accepted by the typed-key API.

The Gate 0 profile fixes `NamespaceHash`, lazily resolves and validates the
server-assigned default namespace (ID `1` on a fresh server), the public
development Item-ID root from `KEY_FORMAT.md`, `StructuredValue-CBOR-v1`,
uncompressed/unprotected values, and TLS 1.3 with ALPN `openkache/1` and
`X25519MLKEM768`. An unexpected namespace ID is rejected before Item-ID
derivation.
`DevelopmentTrust` deliberately disables certificate and hostname verification
while retaining TLS encryption and has no plaintext fallback.  This profile is
**development only — do not use this trust profile in production**.  Trust
roots, certificates, mTLS, retries, timeouts, cancellation, TTL, conditional
writes, raw/JSON selectors, Exact Item IDs, compression, and value-protection
controls are intentionally not part of the maintained facade.
