# OpenKache Rust client

`openkache` is the maintained Rust binding for the OpenKache v1 Gate 0
(`v1-gate0`) client contract. It is one publishable crate; the transport,
protocol, key, and value implementation crates are private workspace
implementation details. The server is released separately as the
[`openkache-server`](../../server/) package. Release this crate from a
reviewed `client-v<version>` tag through the public
`Release OpenKache Rust crate` workflow with `package=client`; the workflow
packages only `clients/rust/Cargo.toml` and never publishes internal workspace
crates.

## Install

```toml
[dependencies]
openkache = "0.1"
```

The facade exposes exactly five operations:

- `Client::connect(endpoint)` establishes the fixed development TLS profile.
- `Client::get(key)` returns `GetResult::Missing` or `GetResult::Found(Value)`.
- `Client::set(key, value)` returns `SetOutcome::Created` or `Replaced`.
- `Client::delete(key)` returns `DeleteOutcome::Deleted` or `NotFound`.
- `Client::close()` is idempotent and waits for admitted operations to settle
  before releasing the transport.

`Value` is the lossless cross-language model encoded as
`StructuredValue-CBOR-v1`. It preserves undefined versus null, arbitrary
integer magnitude, float width and raw bits, byte/text identity, ordered
containers, and scalar map-key equality. `TypedKey` accepts only signed `i64`,
UTF-8 text, or exact bytes.

Gate 0 fixes NamespaceHash mapping, resolves the server-assigned default
namespace lazily (ID `1` on a fresh server), selector `0x10` (uncompressed,
unprotected, StructuredValue-CBOR-v1), ALPN `openkache/1`, and the development
trust profile. The development profile disables certificate verification while
retaining TLS encryption; **development only — do not use this trust profile
in production**.
The facade has no certificate, cancellation, retry, timeout, compression,
protection, raw-byte, Exact Item ID, conditional-write, or policy options.

## Quick start

```rust
use openkache::{Client, DeleteOutcome, GetResult, SetOutcome, Value};

# async fn example() -> openkache::Result<()> {
let client = Client::connect("127.0.0.1:4433").await?;
assert_eq!(client.set("profile", Value::text("OpenKache")).await?, SetOutcome::Created);
assert_eq!(
    client.get("profile").await?,
    GetResult::Found(Value::text("OpenKache")),
);
assert_eq!(client.delete("profile").await?, DeleteOutcome::Deleted);
client.close().await?;
# Ok(())
# }
```

If a mutation crosses admission but its response is lost, the client returns
`Error::UnknownMutation` and never replays it.

## Commands

Check the facade and run its package-local build, test, and documentation commands:

```bash
cargo check --locked
cargo test --locked
cargo doc --locked --no-deps
```

The same commands are the package-local dry run used before creating a
`client-v<version>` release tag. The immutable publication workflow rebuilds
and checksums the exact tagged archive before asking the protected
`crates-io-release` environment for approval.

Run the checked-in example against a local development server:

```bash
cargo run --example basic
```
