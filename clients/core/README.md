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
- `ValueCodec`, which will own Raw and deterministic CBOR serialization, optional
  Zstandard compression, and formatted-value encryption;
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
`LocalProtectedClient`. It requires the platform's io_uring driver and exports
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

`ItemId::from_bytes` preserves a fixed array. `ItemId::from_slice` validates
and copies a dynamic buffer. Neither hashes the supplied bytes. The pre-freeze
v1 contract renames the root secret to `ClientRootKey` and binds the selected
namespace into both item-ID derivation and value AAD. The checked-in
implementation still exposes the previous `DataProtectionKey` surface until
that migration is completed.

`ValueCodec` stores its metadata inside the opaque value:

```text
version:vu128 | flags:u8 | body

flags bits 0..1 = encryption identifier
flags bits 2..3 = compression identifier
flags bits 4..5 = codec identifier
flags bits 6..7 = reserved (zero in v1)

body = encrypt(compress(selected codec payload))
Compact body = AES-256-SIV-CMAC synthetic_iv[16] | ciphertext
Robust body  = nonce[12] | AES-256-GCM-SIV ciphertext | tag[16]
```

For protected profiles, the packed flags and body are authenticated with the
exact item ID and container header. Neither the wire protocol nor the server
parses this format.

Use `ProtectedClient` when the core should derive the item ID and transform
plaintext values:

```rust
use openkache_client_core::{DataProtectionKey, ProtectedClient};

let key = DataProtectionKey::from_base64(&configured_base64_secret)?;
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
- `src/key.rs` handles exact item IDs and data-protection keys.
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
