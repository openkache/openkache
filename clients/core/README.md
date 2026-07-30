# OpenKache client core

`openkache-client-core` is the reusable Rust client engine behind OpenKache language bindings.

## Purpose

The core owns connection lifecycle, retries, stream concurrency, binary protocol operations,
application-key hiding, compression, and encryption. Language adapters convert native types and
delegate to this engine. Raw clients remain available for callers that already own exact protocol
keys and values.

- `RawClient` uses Tokio and Quinn and is `Clone + Send + Sync`.
- `LocalRawClient` exposes the same contract on a caller-owned Compio runtime.
- `ProtectedClient` and `LocalProtectedClient` compose raw operations with mandatory
  application-key hiding and value encryption.
- `ItemKey` is the exact 32-byte item identifier sent over the wire.
- `ItemValue` carries exact opaque bytes. Compression and encryption metadata, when used, live
  inside the client-owned value envelope.
- `DataProtection`, `DataProtectionKey`, `ValueCodec`, and `value_envelope` remain reusable
  primitives for custom low-level integrations.

The ergonomic Rust SDK lives in [`../rust`](../rust). TypeScript's native adapter depends on this
core directly instead of depending on the Rust end-user layer.

## Commands

From `clients/core`:

```bash
cargo build
cargo check --no-default-features --features quic-compio
cargo check --no-default-features --features quic-quinn
cargo fmt --check
```

## Usage

```rust
use openkache_client_core::{ItemKey, ItemValue, RawClient, SetOptions};

let client = RawClient::connect("cache.example.com:4433").await?;
let key = ItemKey::from_bytes([0x42; 32]);
let outcome = client
    .set(key, ItemValue::plaintext(b"value".to_vec()), SetOptions::new())
    .await?;
```

`ItemKey::from_bytes` preserves a Rust array directly, while `ItemKey::from_slice` validates and
copies a dynamic language-binding buffer. Neither constructor hashes the supplied bytes. Use
`DataProtectionKey::derive_item_key` or `DataProtection::item_key` when a language adapter needs
the shared HMAC-SHA-256 derivation.

Plaintext `ValueCodec` mode is exact passthrough. When compression or encryption
is configured, the codec stores its metadata inside the opaque value:

```text
"OKT\x01" | flags:u8 | body

flags bit 0 = compressed
flags bit 1 = encrypted
encrypted body = nonce[24] | ciphertext | authentication_tag[16]
```

For encrypted values, the header and flags are authenticated with the item key.
Neither the wire protocol nor the server parses this envelope.

## Configuration

`RawClient::connect("host:port")` uses system trust and derives the TLS server name from the host.
Use `Endpoint`, `Certificate`, `ClientIdentity`, and a builder for a pre-resolved address,
self-signed trust, mutual TLS, deadlines, retry attempts, or a custom `max_in_flight` bound.

`Endpoint` parsing uses the validated URI authority parser from the `http` crate and then enforces
the OpenKache-specific requirement that a positive port is present. `server_name` remains explicit
only for pre-resolved socket addresses because TLS must verify a certificate identity separately
from the network destination.

## Core components

- `src/lib.rs` owns lifecycle, retries, structured errors, raw operations, and runtime-specific
  client types.
- `src/transport.rs` owns lazy reusable stream lanes and backend-neutral deadlines.
- `src/config.rs` owns stable TLS-independent public configuration wrappers.
- `src/key.rs` owns exact item keys and domain-separated data-protection derivation.
- `src/protection.rs` composes mandatory keyed derivation and encrypted values for language layers.
- `src/protected.rs` owns shared application-key and plaintext-value operations for bindings.
- `src/value.rs` owns optional compression and authenticated encryption tools.
- `src/value_envelope.rs` owns the cross-language self-describing value frame.
