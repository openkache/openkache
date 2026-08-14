# OpenKache client core

`openkache-client-core` is the reusable Rust engine behind OpenKache language
bindings.

## Purpose

The core handles connection lifecycle, TLS, retries, stream concurrency,
protocol operations, application-key protection, compression, encryption, and
formatted-value processing. Language adapters convert native types and
delegate to this crate.

The main API layers are:

- `RawClient` and `LocalRawClient`, which accept exact protocol item IDs and
  opaque values;
- `ProtectedClient` and `LocalProtectedClient`, which accept application keys
  and plaintext values;
- `ValueCodec`, which owns value serialization, optional Zstandard compression,
  and formatted-value encryption;
- reusable configuration, key, protection, and value types for binding
  adapters;
- the optional `ffi` feature, which exports the stable C ABI used by C, C++,
  Python, ctypes, and other synchronous native adapters.

The ergonomic Rust SDK lives in [`../rust`](../rust). TypeScript's native
adapter depends on this core directly.

## Related documentation

- The [client index](../README.md) covers binding architecture and
  implementation status.
- The [value-format specification](../VALUE_FORMAT.md) defines formatted value
  bytes and algorithms.
- The [wire protocol specification](../../protocol/SPEC.md) defines framing,
  operations, limits, and retry ambiguity.
- This README covers core crate usage, configuration, and source layout.

This README intentionally does not specify protected-value bytes. Consult the
client index for implemented-format status and the value-format specification
for the v1 contract.

## Commands

From `clients/core`:

```bash
cargo build
cargo check --no-default-features --features quic-compio
cargo check --no-default-features --features quic-quinn
cargo check --no-default-features --features ffi
cargo fmt --check
```

The `ffi` feature builds a dedicated Compio worker around
`LocalProtectedClient`. Protected FFI operations accept exactly one canonical
v1 key item (the CBOR major type is the explicit `Integer`, `Text`, or `Bytes`
discriminator), not raw application bytes. It requires the platform's io_uring driver and exports
`openkache_client_*` symbols from the native library crate outputs. The ABI
supports protected application-key calls, exact-item-ID calls, mutual TLS,
PEM/DER or system trust, compression, both value-encryption profiles, retries,
reconnect, state snapshots, and bounded request lanes. CMake, Go, and Python
package builds regenerate the scoped Smithy-derived client contract as needed.
Reusable ABI
declarations are in `include/openkache/client_abi.h` (with the
`include/openkache_client.h` compatibility include); the generated Smithy
contract header is supplied by each package build.

## Usage

Use the raw layer when the caller supplies the exact protocol item ID and value:

```rust
use openkache_client_core::{ItemId, ItemValue, RawClient, SetOptions};

let client = RawClient::connect("cache.example.com:4433").await?;
let item_id = ItemId::from_bytes([0x42; 32]);
let outcome = client
    .set(item_id, ItemValue::new(b"value".to_vec()), SetOptions::new())
    .await?;
```

`ItemId::from_bytes` preserves the supplied bytes (including an empty or
short ID). `ItemId::from_slice` validates and copies a dynamic buffer. Neither
hashes the supplied bytes. The pre-freeze
v1 contract calls the root secret `client_root_key` and binds the selected
namespace into both Item ID derivation and value AAD. The Rust API retains
`ClientRootKey` as its API name; it is not a separate wire concept.

`ValueCodec` stores the selected transform profile inside the opaque value.
The v1 container is defined by [`VALUE_FORMAT.md`](../VALUE_FORMAT.md):

```text
value_envelope_version:vu128 | selector:u8 | body

selector bits 0..1 = protection identifier
selector bits 2..3 = compression identifier
selector bits 4..5 = payload-format identifier
selector bits 6..7 = reserved (zero in v1)

body = protect(compress(selected payload))
AES-256-SIV-CMAC body = synthetic_iv[16] | ciphertext
AES-256-GCM-SIV body = nonce[12] | ciphertext | tag[16]
```

Payload format `0` is `OpaqueBytes`; payload format `1` is one definite-length
CBOR data item. JSON is an API convenience serialized as canonical RFC 8785
UTF-8 and stored as `OpaqueBytes`. For protected profiles, the exact encoded
version and selector, namespace ID, and variable-length Item ID are
authenticated as specified by `VALUE_FORMAT.md`. Neither the wire protocol nor
the server parses this format.

Use `ProtectedClient` when the core should derive the item ID and transform
plaintext values:

```rust
use openkache_client_core::{ClientRootKey, ProtectedClient};

let key = ClientRootKey::from_base64(&configured_base64_secret)?;
let client = ProtectedClient::connect("cache.example.com:4433", key).await?;
client.set(b"application-key", b"value".to_vec(), Default::default()).await?;
```
## Configuration

`RawClient::connect("host:port")` and `ProtectedClient::connect` use system
trust and derive the TLS server name from the host. Builders accept a
pre-resolved endpoint, explicit trust roots, mutual TLS identity, deadlines,
retry attempts, compression policy, and `max_in_flight`.

`Endpoint` requires a positive port. A pre-resolved socket address also
requires an explicit certificate server name because the network destination
does not provide one.

## Core components

- `src/lib.rs` provides raw lifecycle, retries, operations, and stable errors.
- `src/transport.rs` manages reusable stream lanes and backend-neutral
  deadlines.
- `src/config.rs` provides public transport and TLS configuration wrappers.
- `src/key.rs` handles exact item IDs and client root keys.
- `src/protection.rs` handles application-key and value transformations.
- `src/protected.rs` composes protected operations for bindings.
- `src/value.rs` owns canonical serialization, compression, and authenticated
  encryption.
- `src/value_envelope.rs` contains the adapter-level TypeScript codec envelope
  used by the Node-API adapter; a future thin logical-value adapter may replace it.
- `src/ffi.rs` owns the versioned worker-backed native ABI used by Swift, C,
  C++, Python, and other non-Rust bindings. It exposes both protected
  application-key operations and exact-item-ID raw operations, while the
  worker owns one Compio runtime per native handle. The canonical declarations
  are in [`include/openkache/client_abi.h`](include/openkache/client_abi.h),
  with [`include/openkache_client.h`](include/openkache_client.h) retained as
  a compatibility include. Generated ABI/protocol constants are emitted to
  each package build directory from the client model
  [`../model/openkache.smithy`](../model/openkache.smithy) and the wire model
  [`../../protocol/model/openkache.smithy`](../../protocol/model/openkache.smithy);
  no header is a hand-maintained constants source.
