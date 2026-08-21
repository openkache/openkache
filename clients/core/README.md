# OpenKache client core

`openkache-client-core` is the reusable Rust engine behind OpenKache language
bindings. This README documents the current transitional crate. The
[target design](TARGET.md) describes the pre-freeze destination.

## Purpose

The core centralizes connection lifecycle, protocol operations, key mapping,
value processing, and the native ABI used by maintained bindings. Language
adapters convert native types and delegate shared behavior to this crate.

The main API layers are:

- raw and protected convenience clients for the current implementation;
- the shared address/value operations described by `CLIENT.md`;
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
- The [security model](../../SECURITY_MODEL.md) defines the threat model,
  value-key schedule, AAD, and cryptographic constructions.
- The [wire protocol specification](../../protocol/SPEC.md) defines framing,
  operations, limits, and ambiguous operation outcomes.
- This README covers core crate usage, configuration, and source layout.
- The [target design](TARGET.md) summarizes the post-migration core contract.

This README intentionally does not specify protected-value bytes. Consult the
client index for implemented-format status and the value-format specification
for the v1 contract.

## What exists today

The current core implements the draft key boundary: `TypedKey` produces one
canonical deterministic-CBOR item, `NamespaceHash` is the default
root/namespace-bound profile, and `PublicKeyOrHash` is explicit. `ItemId`
accepts exact `0..=32` bytes; raw clients preserve those bytes without
derivation or value protection. The deprecated `Hash` and `ByteKeyOrHash`
profiles remain explicit compatibility paths with their historical framing;
they are never selected by the v1 default.

## Commands

From `clients/core`:

```bash
cargo build
cargo check --no-default-features --features quic-compio
cargo check --no-default-features --features quic-quinn
cargo check --no-default-features --features tls-tcp
cargo check --no-default-features --features ffi
cargo fmt --check
```

The `ffi` feature builds a dedicated Compio worker around
`LocalProtectedClient`. It requires the platform's io_uring driver and exports
`openkache_client_*` symbols from the native library crate outputs. The ABI
exposes the shared-core operations and lifecycle required by native adapters.
CMake, Go, and Python package builds regenerate the scoped Smithy-derived
client contract as needed. Reusable ABI declarations are in
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

`ItemId::from_bytes` preserves the maximum-width compatibility constructor.
`ItemId::from_slice` and `ItemId::exact` validate and copy exact `0..=32`-byte
inputs; neither hashes or pads the supplied bytes. Use the raw client for
Exact Item ID operations and the protected client for mapped `TypedKey`
operations.

`ValueCodec` composes the value model with the formatted-value envelope. The
value model owns structured-value semantics, the value format owns envelope
and compression bytes, and the security model owns protection. Neither
the wire protocol nor the server parses these client-owned formats.

Use `ProtectedClient` when the core should derive the item ID and transform
plaintext values:

```rust
use openkache_client_core::{ClientRootKey, ProtectedClient};

let key = ClientRootKey::from_base64(&configured_base64_secret)?;
let client = ProtectedClient::connect("cache.example.com:4433", key).await?;
client.set(b"application-key", b"value".to_vec(), Default::default()).await?;
```

When Item-ID identity must remain public while values are protected, use the
explicit keyring builder:

```rust
use openkache_client_core::{ClientRootKey, ProtectedClient, ValueKeyring};

let value_keys = ValueKeyring::single(1, [0x24; 32])?;
let client = ProtectedClient::builder_with_keyring(
    "cache.example.com:4433".parse()?,
    ClientRootKey::public(),
    value_keys,
);
```

`ClientRootKey::public()` (equivalent to `zero()`) deliberately makes mapped
Item IDs publicly derivable; it never claims application-key confidentiality.
The original `ProtectedClient::builder` remains source-compatible and keeps
its explicit secret-root-plus-derived-value-key behavior.

## Configuration

`RawClient::connect("host:port")` and `ProtectedClient::connect` use system
trust and derive the TLS server name from the host. Builders accept a
pre-resolved endpoint, explicit trust roots, mutual TLS identity, deadlines,
retry attempts, compression policy, and `max_in_flight`.
`max_in_flight_bytes` additionally bounds aggregate bytes retained across
transport and value-protection work.

Target transport and security settings are documented in
[`TARGET.md`](TARGET.md); they are not claims about the transitional builder
surface.

`Endpoint` requires a positive port. A pre-resolved socket address also
requires an explicit certificate server name because the network destination
does not provide one.

The native ABI v6 connection functions retain their historical
`data_protection_key` coupling. ABI v7 adds
`openkache_client_connect_with_options_v7`, which references v6 transport
options while accepting an explicit Item-ID root and an immutable value-key
array; callers must check `openkache_client_abi_version_v7()` before use.

## Request-engine migration

The core exposes a transport-neutral `RequestEngine` for multiplexed lanes.
Callers reserve an ID and one aggregate request/response byte permit with
`admit`, encode that ID through the generated request codec, and submit the
resulting opaque frame. `RequestHandle` dispatches a response by echoed request
ID, validates the operation's generated status set, retains exact
request/response bytes, and distinguishes local rejection, transport failure,
and unknown mutation outcomes. Transport adapters implement `TransportLane`;
`QuinnTransportConnection` provides bounded multiplexed QUIC lanes and
`TlsTcpTransport` provides one ordered TLS-over-TCP lane. Both enforce TLS 1.3,
ALPN `openkache/1`, and the singleton `X25519MLKEM768` hybrid key exchange;
neither adapter exposes plaintext or classical fallback. The engine does not
decode server values or impose a value representation.

Existing convenience clients remain source-compatible during this migration.
New adapters should use the engine boundary rather than assuming one request
per lane or response order, and should call `shutdown`/`drain` exactly once
when owning a connection lifecycle.

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
