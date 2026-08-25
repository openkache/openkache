# Getting started

## Requirements

- Linux with `io_uring`
- two distinct CPUs available to the process
- Rust and the native build tools required by the workspace dependencies

## Run the server

From the repository root:

```bash
cargo run --locked --package openkache-server --bin openkache-server
```

The default command binds `127.0.0.1:4433`, pins networking to CPU 0, and pins
storage to CPU 1. TCP accepts RESP commands while UDP accepts the OpenKache
Gate 0 protocol over QUIC.

Choose another address and CPU pair with positional arguments:

```bash
cargo run --locked --package openkache-server --bin openkache-server -- \
  0.0.0.0:4433 2 3
```

The process creates a fixed 16 GiB `openkache.data` file in its working
directory. The current preview truncates that file on every start.

## Use the Rust client

```rust
use openkache::{Client, GetResult, Value};

# async fn example() -> openkache::Result<()> {
let client = Client::connect("127.0.0.1:4433").await?;
client.set("greeting", Value::text("hello")).await?;
assert_eq!(
    client.get("greeting").await?,
    GetResult::Found(Value::text("hello")),
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
