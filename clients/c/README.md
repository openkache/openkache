# OpenKache native C ABI

This directory packages the maintained Gate 0 C17 ABI used by native
adapters. It is intentionally a thin FFI boundary: transport, request
admission, key mapping, value envelopes, and result ownership remain in the
shared Rust core, while the C++ facade supplies typed-key and
StructuredValue-CBOR-v1 conversion.

The installed C ABI exposes `openkache_client_gate0_connect`,
`openkache_client_gate0_get`, `openkache_client_gate0_set`,
`openkache_client_gate0_delete_value`, and `openkache_client_gate0_close`,
plus result ownership helpers.  The generated shared-core contract is used
to keep the maintained declarations and forwarding library aligned with the
Smithy source of truth; CMake installs that projection as
`openkache/smithy_contract.h` alongside the maintained headers.
Callers must not invoke raw, JSON, Exact Item ID, namespace, experimental,
cancellation, TTL, retry, or certificate controls.

Pointers and lengths are explicit; result payloads are borrowed until
`openkache_client_gate0_result_free`, and a connected handle is transferred
with `openkache_client_gate0_result_take_client` before
`openkache_client_gate0_close`.

## Client lifetime

Call `openkache_client_gate0_close` explicitly after the last operation and
ensure no operation is using the handle when it runs. The close function frees
the native worker; discard the handle afterwards and do not call it again.
Explicit close is the normative lifecycle boundary for C. C has no destructor
or finalizer, so an abandoned handle has no automatic best-effort fallback and
leaks native resources until process teardown.

`gate0_get`, `gate0_set`, and `gate0_delete_value` accept one canonical
StructuredValue-CBOR key item, so integer, text, and byte keys use the same
wire representation across native adapters.  A mutating timeout after worker
admission is returned with the generated `UNKNOWN_MUTATION` result and
error-category discriminators; callers must not replay that operation.

Before each operation, the forwarding layer lazily opens the empty default
namespace and checks its descriptor against the generated Gate 0 namespace
identity.  An unexpected server-assigned ID is rejected before any
StructuredValue key is converted into an Item ID.  The mismatch is returned
as one owned `ERROR` result with the generated `PROTOCOL` error category; no
GET, SET, or DELETE is dispatched, so the mismatch cannot be reported as an
item-level `NOT_FOUND`.

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

The native core library is required at configure time; CMake rejects a
header-only or otherwise link-incomplete Gate 0 package. The installed package
includes the selected native library and forwards its transitive system
libraries through the exported target.

Release installations also include `THIRD-PARTY-NOTICES.txt` under
`share/doc/OpenKacheClient`. When building from a checkout, stage it before
installing (run these commands from the repository root):

```bash
cargo fetch --locked
bun install --cwd scripts --frozen-lockfile --production
./scripts/generate-third-party-notices.ts \
  --artifact cmake \
  --output clients/THIRD-PARTY-NOTICES.txt
```

The configure/build pair is the C17 package smoke check.  It regenerates the
Smithy projection, compiles the forwarding library, and validates the imported
C target; the generated projection is copied into the install tree so the
installed maintained headers remain self-contained.  An installed consumer
can use the `find_package` snippet below.

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

The maintained C facade fixes TLS 1.3, ALPN `openkache/1`,
`X25519MLKEM768`, and the `DevelopmentTrust` profile.  DevelopmentTrust
disables certificate and hostname verification but retains TLS encryption and
does not permit plaintext fallback.  It is **development only — do not use
this trust profile in production**.  Production trust roots and certificate
configuration require a future facade revision.
