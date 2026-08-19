# OpenKache client core

`openkache-client-core` is the reusable Rust engine behind OpenKache language
bindings. The crate currently contains transitional APIs while the draft key,
value, and variable Item ID contracts are being implemented. This README
separates that current surface from the target draft; neither section alone is
a conformance claim for the pre-freeze contracts.

## Purpose

The target core handles connection lifecycle, TLS, transport fallback, lane
concurrency, protocol operations, native key mapping, compression, encryption,
and formatted-value processing. Language adapters convert native types and
delegate to this crate. The current transitional implementation does not yet
provide every target capability described below.

The main API layers are:

- `RawClient` and `LocalRawClient`, which accept exact protocol item IDs and
  raw values;
- `ProtectedClient` and `LocalProtectedClient`, which accept application keys
  and plaintext values;
- `ValueCodec`, which owns value serialization, optional Zstandard compression,
  and formatted-value encryption;
- reusable configuration, key, protection, and value types for binding
  adapters;
- the optional `ffi` feature, which exports the versioned public C ABI used by
  C, C++, Python, ctypes, and other synchronous native adapters.

The ergonomic Rust SDK lives in [`../rust`](../rust). TypeScript's native
adapter depends on this core directly.

## Related documentation

- The [client index](../README.md) covers package inventory and implementation
  status.
- The [client implementation guide](../CLIENT.md) defines the shared boundary
  between this core and maintained language adapters.
- The [key-format specification](../KEY_FORMAT.md) defines typed keys and Item
  ID mapping.
- The [value model](../value/SPEC.md) defines cross-language structured values,
  native mappings, and the initial codec profile.
- The [value-format specification](../VALUE_FORMAT.md) defines the formatted
  value envelope and compression.
- The [value security profiles](../VALUE_SECURITY.md) define the value-key
  schedule, AAD, and cryptographic constructions.
- The [wire protocol specification](../../protocol/SPEC.md) defines framing,
  operations, limits, and ambiguous operation outcomes.
- This README covers core crate usage, configuration, and source layout.

This README intentionally does not specify protected-value bytes. Consult the
client index for implemented-format status and the value-format specification
for the v1 contract.

## What exists today

The current code describes only the transitional implementation; the draft
documents are the target source of truth. Some current
Rust and legacy adapter paths still use fixed-width Item IDs or a legacy value
container. They MUST NOT be treated as evidence that the draft variable-length
wire, key, or value profiles are already implemented.

## Draft target

The target core will expose independent address and value representation
families: mapped or exact Item IDs combined with raw bytes, caller-owned v0
envelopes, or the v1 value envelope. It will support `0..=32`-byte Item IDs,
the key mapping profiles, and the shared lane/request engine described by the
linked specifications. A migration
may change constructors and generated ABI declarations; the public C ABI is
versioned rather than an unqualified promise that every transitional symbol
will remain unchanged.

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
`LocalProtectedClient`. Protected FFI operations accept exactly one complete
canonical v1 key item validated by `KEY_FORMAT.md`, not raw application bytes.
`Integer` includes CBOR major types 0/1 and canonical tags 2/3; `Text` and
`Bytes` use their corresponding CBOR major types. The feature requires the
platform's io_uring driver and exports `openkache_client_*` symbols from the
native library crate outputs. The ABI exposes the shared-core operations and
lifecycle required by native adapters. CMake, Go, and Python package builds
regenerate the scoped Smithy-derived client contract as needed. Reusable ABI
declarations are in
`include/openkache/client_abi.h` (with the
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

`ItemId::from_bytes` preserves the current fixed-array API. `ItemId::from_slice`
validates and copies a dynamic input according to the current implementation.
Neither hashes the supplied bytes. The key-format specification defines the
target empty, short, and 32-byte Exact Item IDs and formatted-key behavior;
this example describes the current Rust API surface until that migration is
complete.

`ValueCodec` composes the value model with the formatted-value envelope. The
value model owns structured-value semantics, the value format owns envelope
and compression bytes, and the value security profiles own protection. Neither
the wire protocol nor the server parses these client-owned formats.

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

The target draft adds QUIC/TLS-over-TCP fallback and deployment-configurable
server-identity verification without changing frame bytes. Both target
transports require TLS 1.3 with `X25519MLKEM768`; plaintext TCP is not
supported. Those target settings are not yet a claim about the transitional
builder surface.

`Endpoint` requires a positive port. A pre-resolved socket address also
requires an explicit certificate server name because the network destination
does not provide one.

## Core components

- `src/lib.rs` provides raw lifecycle, retries, operations, and versioned
  errors.
- `src/operation_request.rs` maps client domain values to generated numeric
  request fields, selects the generated compact layout, and delegates framing
  to the shared protocol encoder.
- `src/request.rs` owns request construction and retry state around the
  protocol-owned `OwnedRequestFrame`. Replayable requests retain one encoded
  frame, while one-shot requests transfer field owners directly into their
  transport attempt.
- `src/protocol.rs` owns client-domain protocol values and semantic response
  projection without redefining request framing.
- `src/transport.rs` manages reusable transport lanes and backend-neutral
  deadlines and writes the ordered segments exposed by `OwnedRequestFrame`.
- `src/config.rs` provides public transport and TLS configuration wrappers.
- `src/key.rs` handles exact item IDs and data-protection keys.
- `src/protection.rs` handles application-key and value transformations.
- `src/protected.rs` composes protected operations for bindings.
- `src/value.rs` owns the current value-model adapter, compression, and
  authenticated encryption.
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
