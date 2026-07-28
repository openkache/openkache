# Getting Started

## Prerequisites

- Linux x86_64 or aarch64
- Rust toolchain (if building from source)
- NVMe SSD recommended for production

## Install

### From source

```bash
git clone https://github.com/openkache/openkache
cd openkache
cargo build --release
```

## Quickstart

### Run the SSD-backed QUIC server

```bash
cargo run --manifest-path server/Cargo.toml --bin openkache-server
```

The server listens on `127.0.0.1:4433`, stores shard files under
`target/kvkache-v1`, and writes its generated certificate to
`target/openkache-local/certificate.local.der`. Use `--config <path>` to load
an explicit TOML cache configuration.

### Run the BCF53 benchmark

```bash
cargo run --package openkache --bin breadcrumb --release
```

### Use the SDK (Rust)

```rust
use openkache_client::value::{Compression, ValueCodec, ZstandardOptions};
use openkache_client::{Client, ClientOptions};

let certificate = std::fs::read(
    "target/openkache-local/certificate.local.der",
)?;
let client = Client::connect_with_options(
    "127.0.0.1:4433".parse()?,
    "localhost",
    &certificate,
    ClientOptions {
        value_codec: ValueCodec::encrypted(
            encryption_key,
            Compression::Zstandard(ZstandardOptions::default()),
        )?,
    },
)
.await?;
client.set(b"mykey", b"myvalue").await?;
let value = client.get(b"mykey").await?;
```

## Next steps

- See [Architecture](../README.md#-architecture) for how OpenKache works
- See the Rust client SDK under `clients/rust/`
- See the TypeScript client SDK under `clients/typescript/`
- See the .NET client SDK under `clients/dotnet/`
