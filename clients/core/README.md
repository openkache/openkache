# OpenKache client core

`openkache-client-core` is the reusable low-level Rust implementation of the OpenKache QUIC
transport and binary protocol client.

## Purpose

The core gives language adapters and advanced Rust callers exact control over protocol item keys,
item values, request options, connection lifecycle, retries, and stream concurrency. It does not
choose an application key or value codec.

- `RawClient` uses Tokio and Quinn and is `Clone + Send + Sync`.
- `LocalRawClient` exposes the same contract on a caller-owned Compio runtime.
- `ItemKey` is the exact 32-byte item identifier sent over the wire.
- `ItemValue` carries exact bytes plus compression and encryption metadata.
- `DataProtectionKey`, `ValueCodec`, and `value_envelope` are optional tools that higher layers can
  compose consistently across language adapters.

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

`ItemKey::derive(application_key)` is the shared SHA-256 convenience path. `ItemKey::from_bytes`
preserves caller-supplied bytes exactly.

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
- `src/value.rs` owns optional compression and authenticated encryption tools.
- `src/value_envelope.rs` owns the cross-language self-describing value frame.
