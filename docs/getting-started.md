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
`target/openkache-local/certificate.local.der`. It automatically sizes itself
from process CPU affinity, available or cgroup-limited RAM, and available
filesystem space. Use `--port <port>` only to override the default port, or
`--config <path>` to load an explicit TOML cache configuration.

To inspect the automatic result and optionally override individual inputs, run:

```bash
cargo run --manifest-path server/Cargo.toml --bin openkache-server -- \
  --cpus 4 \
  --memory-gib 32 \
  --storage-gb 2500 \
  --profile balanced \
  --directory ./openkache-data \
  --plan
```

`balanced` is the default and models 1 KiB encoded values. `light` models
100-byte inline values, while `heavy` models 2 KiB Blob values. Every sizing
argument is optional. Remove `--plan` to open the storage files and start
serving with the calculated configuration.

The calculated limits are advisory. The planner detects standard Linux cgroup
memory limits and filesystem availability but not filesystem quotas, SSD type,
or device throughput. Its memory estimate covers the packed Table rather than
whole-process peak RSS. `--cpus` selects worker threads but does not impose a
process CPU quota. `light` and `balanced` accept individual values up to their
1 MiB Blob Segment size; `heavy` accepts up to 64 MiB. Existing storage must be
reopened with the same worker count and Segment layout.

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
