# Getting started

## Requirements

- Linux with `io_uring`, or Apple Silicon macOS
- Linux with two distinct CPU IDs; Apple Silicon macOS delegates thread
  placement to the scheduler
- Rust and the native build tools required by the workspace dependencies when
  building from source

Prebuilt archives are available for [Linux x86_64, Linux aarch64, and Apple
Silicon macOS](../README.md#download-server-binaries).

## Run the server

From the repository root:

```bash
cargo run --locked --package openkache-server --bin openkache-server
```

The default command binds `127.0.0.1:4433`. Linux pins networking and storage
to CPU 0 and CPU 1; macOS delegates thread placement to the scheduler. TCP
accepts RESP commands while UDP accepts the OpenKache Gate 0 protocol over
QUIC.

On Linux, choose another address and CPU pair with positional arguments:

```bash
cargo run --locked --package openkache-server --bin openkache-server -- \
  0.0.0.0:4433 2 3
```

On macOS, choose another address without CPU arguments:

```bash
cargo run --locked --package openkache-server --bin openkache-server -- \
  0.0.0.0:4433
```

The process creates a fixed 16 GiB `openkache.data` file in its working
directory. The current preview truncates that file on every start.

On Linux the server uses `io_uring`; the Apple Silicon build uses Tokio's
native polling fallback.

## Use the Rust client

```rust
use openkache::{Client, Value};

# async fn example() -> openkache::Result<()> {
let client = Client::connect("127.0.0.1:4433").await?;
client.set("greeting", Value::text("hello")).await?;
assert_eq!(
    client.get("greeting").await?.unwrap(),
    Value::text("hello"),
);
client.delete("greeting").await?;
client.close().await?;
# Ok(())
# }
```

The Gate 0 profile is for local development and deliberately skips server
certificate verification. The current server also accepts plaintext RESP on
the TCP port.

## Verify

```bash
cargo check --locked
cargo test --locked --package openkache-server
```

See [the server README](../server/README.md) for the implemented operation
subset and [the client status](../clients/README.md) for language packages.
