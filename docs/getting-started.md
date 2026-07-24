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