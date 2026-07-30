# OpenKache Rust client

`openkache-client` provides the ergonomic Rust end-user API over the reusable
[`openkache-client-core`](../core) transport and protocol implementation.

## Purpose

Rust applications get ergonomic defaults while language adapters and advanced callers can depend
on `openkache-client-core` directly for every protocol value pattern. The high-level crate
re-exports the core's stable public types so callers can still reach the raw layer without a second
connection.

- `Client` accepts application keys and plaintext values. It derives 32-byte item keys and applies
  mandatory key hiding and value encryption plus optional compression.
- `RawClient`, implemented in `clients/core`, accepts an exact 32-byte `ItemKey` and an
  `ItemValue`. It does not hash keys or transform values.
- `LocalClient` and `LocalRawClient` provide the same layers for a Compio runtime. The primary
  `Client` and `RawClient` use Tokio and Quinn and are `Clone + Send + Sync`.

## Commands

From `clients/rust`:

```bash
cargo build
cargo check --no-default-features --features quic-compio
cargo fmt --check
```

## Core components

- `src/lib.rs` exposes ergonomic Rust request builders over core's shared protected client.
- `src/ffi.rs` adapts the high-level Compio client to the versioned C ABI.
- `../core` owns transport, TLS configuration, raw operations, key derivation, compression,
  encryption, protected cache operations, and the cross-language value envelope.

## Configuration

The shortest connection accepts one `host:port` string plus the mandatory data protection key. It
uses system trust and defaults for deadlines, retries, compression, and stream concurrency. The
builder configures self-signed trust, mutual TLS, request deadlines, retry-safe attempts,
`max_in_flight`, and compression.

## Connect

The shortest connection path resolves the hostname, derives its TLS server name, and loads the
operating system trust store:

```rust
use openkache_client::{Client, DataProtectionKey};

let protection_key = DataProtectionKey::from_base64(&configured_base64_secret)?;
let client = Client::connect("cache.example.com:4433", protection_key).await?;
```

Explicitly trust a development or self-signed certificate with the builder. The stable client API
owns its certificate types and does not expose rustls:

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

`server_name` is needed only when a pre-resolved socket address cannot supply the certificate name.
`Endpoint::new("cache.example.com", 4433)` and the one-string connection path derive it
automatically.

Production mutual TLS uses client-owned types:

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

`DataProtectionKey` is the application-managed 32-byte master secret for both key hiding and value
encryption. Generate 32 random bytes and store their Base64 representation. Do not hash, truncate,
or pad an arbitrary UTF-8 string into a key.

```rust
use openkache_client::{Client, DataProtectionKey};
use openkache_client::value::{Compression, ZstandardOptions};

let protection_key = DataProtectionKey::from_base64(&configured_base64_secret)?;
let client = Client::builder(endpoint, protection_key)
    .compression(Compression::Zstandard(ZstandardOptions::default()))
    .connect()
    .await?;
```

The client derives independent HKDF-SHA-256 subkeys. Application keys become deterministic
HMAC-SHA-256 item keys, while values use XChaCha20-Poly1305 with a fresh nonce and the item key as
authenticated data. The high-level client always requires this protection. A human passphrase API
using Argon2id and an explicit salt is deferred.

Clients must use the same data protection key to share entries. Rotating the key changes the
derived item keys, so previously stored entries become unreachable and must be repopulated. The
client does not retain old keys or perform dual-key reads.

Every binary-protocol key is exactly 32 bytes:

```rust
use openkache_client::ItemKey;

let exact = ItemKey::from_bytes([0x42; 32]);
```

`ItemKey::from_slice` provides the same exact-byte behavior for a dynamic buffer and rejects every
other length. `RawClient` sends the resulting key without deriving or hashing it. Language adapters
should depend on `openkache-client-core` directly; Rust applications can use the re-exported raw
types when they already own a 32-byte item key.

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

`Result<T>` represents client, transport, protocol, or server failure. Outcome enums represent
successful domain results:

```rust
pub enum GetOutcome<T> { Found(T), NotFound }
pub enum SetOutcome { Created, Replaced, NotStored }
pub enum DeleteOutcome { Deleted, NotFound }
```

`Error` is a non-exhaustive client-owned enum. `Backend` and `Operation` provide stable identifiers
for runtime, transport, timeout, and mutation errors; backend library errors remain diagnostic
messages rather than leaked Quinn, Compio, rustls, or protocol types. `AmbiguousOutcome` retains its
structured underlying `Error` as `cause`.

Set options are methods on the awaitable request rather than separate `with_options` functions:

```rust
client
    .set(b"lease", b"value")
    .if_absent()
    .expires_after_millis(5_000)
    .await?;
```

TTL is an optional positive `u64` millisecond value. Zero is invalid. It is not a `Duration`
because the wire format has an exact millisecond unit.

The raw signatures are:

```rust
async fn get(&self, key: ItemKey) -> Result<GetOutcome<ItemValue>>;
async fn set(
    &self,
    key: ItemKey,
    value: ItemValue,
    options: SetOptions,
) -> Result<SetOutcome>;
async fn delete(&self, key: ItemKey) -> Result<DeleteOutcome>;
```

`ItemValue::from_parts` preserves exact opaque bytes and client-owned compression/encryption
metadata without exposing protocol-crate types.

## Concurrency and connection lifecycle

One client owns one QUIC connection. It lazily creates up to 256 reusable bidirectional stream
lanes by default, with one untagged request in flight per lane. A free lane is reused immediately;
otherwise another lane is opened until `max_in_flight` is reached. Additional requests wait for a
lane. Configure the bound with `.max_in_flight(n)`.

Multiple in-flight requests are not pipelined on one busy lane. The protocol has no request IDs, so
doing so would make cancellation and response recovery ambiguous. Applications requiring more
parallelism than one connection should use multiple clients until a multi-connection pool is
provided.

```rust
let state = client.connection_state();
client.reconnect().await?;
client.close().await?;
```

`connection_state()` is a best-effort snapshot and does not guarantee the next request will
succeed. Response-safe operations (`PING`, `GET`, and `STATS`) may reconnect and retry according to
`RetryPolicy`. Mutations are never replayed after request transmission when the response cannot be
confirmed because the server may already have applied them; the operation returns
`Error::AmbiguousOutcome`. Connection failures mark the client disconnected, and the next operation
reconnects before sending its own request. A timeout while waiting for an unused lane is not
ambiguous because no request was sent and does not invalidate a healthy connection.

## Deferred capabilities

Telemetry hooks, batch and multi-key operations, multi-connection pooling, DNS refresh, advanced
TLS controls, connection-state subscriptions, passphrase derivation, and codec
registry/schema-negotiation are intentionally deferred.
