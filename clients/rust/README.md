# OpenKache Rust client

`openkache-client` provides the ergonomic Rust API over
[`openkache-client-core`](../core).

## Purpose

Rust applications get high-level request builders while advanced callers can
use the re-exported raw core types over the same connection.

- `Client` accepts application keys and plaintext values.
- `RawClient` accepts opaque item IDs of up to 32 bytes and opaque values.
- `LocalClient` and `LocalRawClient` provide equivalent Compio-local layers.
- `Client` and `RawClient` use Tokio and Quinn and are `Clone + Send + Sync`.
- `RawClient` and `LocalRawClient` implement the Smithy-generated
  `smithy::OpenKacheApi` interface, so generated operation inputs and outputs
  use the same service contract as the other language adapters.

Shared SDK status and layering live in the [client index](../README.md).
Formatted value bytes belong to the
[value-format specification](../VALUE_FORMAT.md), and server-visible behavior
belongs to the [wire protocol specification](../../protocol/SPEC.md).

## Commands

From `clients/rust`:

```bash
cargo build
cargo check --no-default-features --features quic-compio
cargo fmt --check
```

Builds require Bun and Smithy CLI on `PATH`. Cargo invokes the client generator
to combine the client model with the wire model, then generates the
`openkache_client::smithy` operation types and API trait before compiling the
crate.

## Connect

The shortest connection path uses system trust and derives the TLS server name
from the endpoint:

```rust
use openkache_client::{Client, DataProtectionKey};

let protection_key = DataProtectionKey::from_base64(&configured_base64_secret)?;
let client = Client::connect("cache.example.com:4433", protection_key).await?;
```

Use the builder for a pre-resolved address, explicit trust root, or mutual TLS:

```rust
use openkache_client::{Certificate, Client, DataProtectionKey, Endpoint};

let endpoint = Endpoint::from_socket_addr("127.0.0.1:4433".parse()?, "localhost")?;
let certificate = Certificate::from_der(certificate_der)?;
let protection_key = DataProtectionKey::from_base64(&configured_base64_secret)?;
let client = Client::builder(endpoint, protection_key)
    .trust_certificate(certificate)
    .connect()
    .await?;
```

`server_name` is separate only for a pre-resolved socket address. Hostname
endpoints derive it automatically.

Production mutual TLS uses client-owned certificate and key types:

```rust
use openkache_client::{Certificate, ClientIdentity, PrivateKey};

let identity = ClientIdentity::new(
    vec![Certificate::from_pem(&client_certificate_pem)?],
    PrivateKey::from_pem(&client_private_key_pem)?,
)?;

let client = Client::builder(endpoint, protection_key)
    .trust_certificate(server_ca)
    .client_identity(identity)
    .connect()
    .await?;
```

## Protect keys and values

`DataProtectionKey` is an application-managed 32-byte random secret. Generate
it with a cryptographically secure random source and store its Base64 form in
secret storage. Do not hash, truncate, or pad a human-readable password into a
key.

Clients must use the same data-protection key to share protected entries.
Rotating it changes derived item IDs, so old entries become unreachable and
must be repopulated.

The [client status table](../README.md#sdk-status) identifies the format
implemented by this release. The
[value-format specification](../VALUE_FORMAT.md) defines the v1 contract.

## Operations

```rust
use openkache_client::{DeleteOutcome, GetOutcome, SetOutcome};

let round_trip_time = client.ping().await?;

assert_eq!(
    client.set(b"greeting", b"hello").await?,
    SetOutcome::Created,
);
assert_eq!(
    client.get(b"greeting").await?,
    GetOutcome::Found(b"hello".to_vec()),
);
assert_eq!(
    client.delete(b"greeting").await?,
    DeleteOutcome::Deleted,
);
```

`Result<T>` represents failure. Outcome enums represent successful cache
results:

```rust
pub enum GetOutcome<T> { Found(T), NotFound }
pub enum SetOutcome { Created, Replaced, NotStored }
pub enum DeleteOutcome { Deleted, NotFound }
```

`Client::get_value` and `Client::set_value` expose the shared logical value
model for canonical JSON in addition to the byte-oriented API:

```rust
use openkache_client::value::{JsonValue, Value};
use openkache_client::SetOptions;

let value = Value::Json(JsonValue::object(vec![
    ("answer".to_owned(), JsonValue::number(42.0)?),
])?);
client
    .set_value(b"document", value, SetOptions::new())
    .await?;
```

Set options are methods on the awaitable request:

```rust
client
    .set(b"lease", b"value")
    .if_absent()
    .expires_after_millis(5_000)
    .await?;
```

Use the raw layer when the application supplies exact protocol values:

```rust
use openkache_client::{ItemId, ItemValue, SetOptions};

let item_id = ItemId::from_bytes([0x42; 32]);
let result = client
    .raw()
    .set(item_id, ItemValue::new(b"value".to_vec()), SetOptions::new())
    .await?;
```

The raw layer bypasses key derivation and formatted-value processing.

For generated service integrations, use the raw client with the Smithy
operation types. The generated interface follows protocol item-ID semantics;
it does not reinterpret `item_id` as an application key:

```rust
use openkache_client::smithy::{GetInput, OpenKacheApi};

let result = <_ as OpenKacheApi>::get(client.raw(), GetInput {
    item_id: item_id.as_bytes().to_vec(),
}).await?;
```

## Configuration and lifecycle

The builder configures explicit trust, mutual TLS, request deadlines, retries
for response-safe operations, `max_in_flight`, and compression.

One client maintains one QUIC connection and lazily opens reusable bidirectional
stream lanes up to `max_in_flight`. One request is active on each lane.
Additional operations wait for a free lane.

```rust
let state = client.connection_state();
client.reconnect().await?;
client.close().await?;
```

`connection_state()` is a best-effort snapshot. `close()` is idempotent and
permanent for that client. Automatic retry and ambiguous mutation outcomes
follow the [wire protocol rules](../../protocol/SPEC.md#retry-and-outcome-rules).

## Core components

- `src/lib.rs` exposes the ergonomic Rust client and request builders.
- `src/ffi.rs` re-exports the versioned C ABI implemented by the shared core
  for native language adapters.
- [`../core`](../core) provides shared transport, protocol, protection, and
  binding behavior.
