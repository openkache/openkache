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

### Using Nix

```bash
nix develop  # enter dev shell
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
use openkache::prelude::*;

let client = OpenKacheClient::new("localhost:9000").await?;
client.set("mykey", b"myvalue").await?;
let value = client.get("mykey").await?;
```

## Next steps

- See [Architecture](../README.md#-architecture) for how OpenKache works
- See the Rust client SDK under `clients/rust/`
- See the .NET client SDK under `clients/dotnet/`
