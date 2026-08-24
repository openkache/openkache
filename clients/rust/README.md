# OpenKache Rust client

`openkache-client` is the maintained asynchronous Rust SDK for the
[OpenKache](https://openkache.com) cache server. It provides one reusable,
TLS-protected connection, bounded concurrency, typed application keys,
automatic value protection, and an exact Item ID escape hatch.

The crate is published as `openkache-client` and is imported as
`openkache_client`. The shared low-level engine is published separately as
[`openkache-client-core`](https://docs.rs/openkache-client-core).

The short crates.io name `openkache` is already occupied and is also the
workspace server package name, so this client deliberately keeps the
unambiguous `openkache-client` registry name. Publishing the SDK as
`openkache` requires a separate crates.io ownership/name-change decision.

## Install

The default feature uses Tokio and Quinn:

```toml
[dependencies]
openkache-client = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

`openkache-client` does not create an async runtime. Applications choose and
own their runtime; Tokio is listed above because the default Quinn transport
needs a Tokio runtime to make progress.

## Quick start

The server name in the endpoint is also used for TLS certificate verification.
The protection key must be a randomly generated 32-byte secret, encoded as
padded or unpadded Base64.

```rust,no_run
use openkache_client::{Client, DataProtectionKey, DeleteOutcome, GetOutcome, SetOutcome};

#[tokio::main]
async fn main() -> openkache_client::Result<()> {
    let protection_key = DataProtectionKey::from_base64(
        &std::env::var("OPENKACHE_DATA_PROTECTION_KEY")
            .expect("set OPENKACHE_DATA_PROTECTION_KEY"),
    )?;
    let client = Client::connect("cache.example.com:4433", protection_key).await?;

    client.ping().await?;
    assert!(matches!(
        client.set(b"greeting", b"hello").await?,
        SetOutcome::Created | SetOutcome::Replaced
    ));

    match client.get(b"greeting").await? {
        GetOutcome::Found(value) => assert_eq!(value, b"hello"),
        GetOutcome::NotFound => panic!("the value was not stored"),
    }

    assert_eq!(client.delete(b"greeting").await?, DeleteOutcome::Deleted);
    client.close().await
}
```

The same round trip is available as a runnable example. It expects an
endpoint trusted by the local system certificate store:

```bash
export OPENKACHE_ENDPOINT=cache.example.com:4433
export OPENKACHE_DATA_PROTECTION_KEY='base64-secret'
cargo run --example basic
```

For a self-signed development server, use an explicit trust root:

```rust,no_run
use openkache_client::{Certificate, Client, DataProtectionKey, Endpoint};

# async fn connect() -> openkache_client::Result<()> {
let endpoint = Endpoint::from_socket_addr("127.0.0.1:4433".parse()?, "localhost")?;
let server_certificate = Certificate::from_der(std::fs::read("server-ca.der")?)?;
let key = DataProtectionKey::from_base64(
    &std::env::var("OPENKACHE_DATA_PROTECTION_KEY").unwrap(),
)?;
let client = Client::builder(endpoint, key)
    .trust_certificate(server_certificate)
    .connect()
    .await?;
# let _ = client;
# Ok(())
# }
```

Certificate verification is always enabled. Mutual TLS uses
`ClientIdentity::new` with the client certificate chain and private key:

```rust,no_run
use openkache_client::{Certificate, ClientIdentity, PrivateKey};

# fn build_identity(
#     certificate_pem: &[u8],
#     private_key_pem: &[u8],
# ) -> openkache_client::Result<ClientIdentity> {
let identity = ClientIdentity::new(
    vec![Certificate::from_pem(certificate_pem)?],
    PrivateKey::from_pem(private_key_pem)?,
)?;
# Ok(identity)
# }
```

## Operations

`Result<T>` is reserved for client, protocol, TLS, and server failures.
Successful cache outcomes are explicit enums:

```rust
pub enum GetOutcome<T> {
    Found(T),
    NotFound,
}
pub enum SetOutcome {
    Created,
    Replaced,
    NotStored,
}
pub enum DeleteOutcome {
    Deleted,
    NotFound,
}
```

`set` returns an awaitable request, so conditions and expiration remain part
of the same operation:

```rust,no_run
# async fn write(client: &openkache_client::Client) -> openkache_client::Result<()> {
client
    .set(b"session", b"value")
    .if_absent()
    .expires_after_millis(30_000)
    .await?;
# Ok(())
# }
```

Use `set_with_options` when options are assembled by another layer:

```rust,no_run
# async fn write(client: &openkache_client::Client) -> openkache_client::Result<()> {
use openkache_client::SetOptions;

client
    .set_with_options(b"session", b"value", SetOptions::new().if_present())
    .await?;
# Ok(())
# }
```

The logical value helpers support the shared cross-language value model,
canonical JSON, and StructuredValue-CBOR-v1:

```rust,no_run
use openkache_client::value::{JsonValue, Value};
use openkache_client::{Client, SetOptions};

# async fn write(client: &Client) -> openkache_client::Result<()> {
let value = Value::Json(JsonValue::object(vec![
    ("answer".to_owned(), JsonValue::number(42.0)?),
])?);
client
    .set_value(b"document", value, SetOptions::new())
    .await?;
    client
    .set_json(
        b"metadata",
        JsonValue::String("canonical JSON".into()),
        SetOptions::new(),
    )
    .await?;
# Ok(())
# }
```

## Keys and the raw layer

High-level methods accept `TypedKey` conversions for byte strings, text, and
signed integer keys. The default `NamespaceHash` profile canonicalizes the key
and binds its Item ID to the selected namespace and client root.

When an application already owns the protocol identity, use the raw client:

```rust,no_run
use openkache_client::{ItemId, ItemValue, SetOptions};

# async fn write(client: &openkache_client::Client) -> openkache_client::Result<()> {
let item_id = ItemId::from_slice(&[0x42; 32])?;
client
    .raw()
    .set(item_id, ItemValue::new(b"value".to_vec()), SetOptions::new())
    .await?;
# Ok(())
# }
```

Raw operations do not derive keys or interpret formatted values. The raw
Tokio, Compio, and TLS-over-TCP clients implement the generated
`smithy::OpenKacheApi` trait for integrations that need exact protocol
inputs and outputs.

`DataProtectionKey` couples the convenience key derivation and value
protection profiles. Use `Client::builder_with_keyring` when Item ID identity
and value-key rotation must be independent. Rotating the Item ID root makes
old entries unreachable; rotate value keys with `ValueKeyring` when reads of
old protected values are required.

## Configuration and lifecycle

The builder exposes:

- system or explicit server trust roots and mutual TLS;
- request and connection deadlines;
- retries for response-safe operations;
- bounded `max_in_flight` lanes and aggregate in-flight bytes;
- automatic level-1 Zstandard compression (keeps the compressed frame only
  when it is smaller);
- authenticated-encryption profile and value-keyring selection;
- namespace ID/name and policy selection.

```rust,no_run
use openkache_client::{Client, DataProtectionKey, RetryPolicy};

# async fn connect(endpoint: openkache_client::Endpoint, key: DataProtectionKey)
#     -> openkache_client::Result<Client> {
let client = Client::builder(endpoint, key)
    .max_in_flight(64)
    .retry_policy(RetryPolicy::default())
    .connect()
    .await?;
client.reconnect().await?;
client.close().await?;
# Ok(client)
# }
```

`connection_state()` is a best-effort snapshot, not a health guarantee.
`close()` is idempotent and permanently closes that client instance.
Mutations that were transmitted but whose response cannot be confirmed return
`Error::AmbiguousOutcome`; the client never silently replays them.

## Transport features

| Feature | Client type | Runtime |
| --- | --- | --- |
| `quic-quinn` (default) | `Client`, `RawClient` | Tokio |
| `quic-compio` | `LocalClient`, `LocalRawClient` | Compio, local-only |
| `tls-tcp` | `TlsTcpClient`, `TlsTcpRawClient` | Tokio |
| `ffi` | Versioned native ABI under `ffi` | Enables the shared FFI transports |
| `runtime-tokio` (default) | Tokio-based examples | Adds Tokio only for examples |

Disable defaults when selecting a different transport:

```bash
cargo check --no-default-features --features quic-compio
cargo check --no-default-features --features tls-tcp
```

## Publishing and docs.rs

The package metadata, README, license, generated contract snapshots, and
registry-compatible dependency versions are included in the crate tarball.
The source checkout regenerates contracts from the canonical Smithy inputs;
the published package uses its checked-in snapshot and therefore does not
require Bun or Smithy.

`cargo publish` automatically asks docs.rs to build documentation after the
crate reaches crates.io. There is no separate docs.rs upload command. The
`all-features` docs.rs configuration documents every transport and the
generated Smithy API.

From the monorepo root, publish the protocol, value, core, and high-level
client crates in dependency order with one command:

```bash
nix develop -c just publish-rust-client
```

This command requires a crates.io token (`cargo login` or `CARGO_REGISTRY_TOKEN`)
and a clean, reviewed worktree. It excludes the server and CLI packages.
Before publishing, run the same package checks locally:

```bash
nix develop -c just rust-client-package-check
```

## Development

From the public repository checkout:

```bash
cargo fmt --check
cargo check -p openkache-client
cargo doc -p openkache-client --no-deps
```

The private OpenKache monorepo owns integration tests and benchmarks. The
client package itself intentionally contains only production source,
publication metadata, and user-facing examples.
