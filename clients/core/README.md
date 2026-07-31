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
- `ValueCodec`, which owns Raw and RFC 8785 JSON serialization, optional
  Zstandard compression, and formatted-value encryption;
- reusable configuration, key, protection, and value types for binding
  adapters.

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
cargo fmt --check
```

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
and copies a dynamic buffer. Neither hashes the supplied bytes. Use
`DataProtectionKey::derive_item_id` or `DataProtection::item_id` for the
shared BLAKE3 derivation.

`ValueCodec` stores its metadata inside the opaque value:

```text
version:vu128 | format:u8 | body

format bits 0..3 = compression identifier
format bits 4..7 = encryption identifier

Compact body = AES-256-SIV-CMAC synthetic_iv[16] | ciphertext
Robust body  = nonce[12] | AES-256-GCM-SIV ciphertext | tag[16]
```

The encrypted serialization identifier and body are authenticated with the
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
- `src/ffi.rs` exposes the shared versioned C ABI used by native language
  adapters; [`include/openkache/client_abi.h`](include/openkache/client_abi.h)
  contains its declarations and includes the generated Smithy contract at
  [`include/openkache/smithy_contract.h`](include/openkache/smithy_contract.h).
  The root [`include/openkache_client.h`](include/openkache_client.h) remains
  as a compatibility include for existing bindings.
- `src/value.rs` owns canonical serialization, compression, and authenticated
  encryption.
- `src/value_envelope.rs` contains the adapter-level TypeScript codec envelope
  used by the Node-API adapter; a future thin logical-value adapter may replace it.
