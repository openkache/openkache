# Getting Started

## Prerequisites

- Linux x86_64/aarch64 or Apple Silicon macOS
- Rust toolchain (if building from source)
- NVMe SSD recommended for production (the server warns but does not reject other
  storage)

The server runtime contract is limited to Linux x86_64/aarch64 and Apple
Silicon macOS. On Linux, the selected native network runtime must provide
`io_uring`; the required `io_uring_setup`, `io_uring_enter`, and
`io_uring_register` syscalls must be available to the process. If a container
seccomp profile denies them, startup fails with a diagnostic that points to the
profile and `/proc/sys/kernel/io_uring_disabled`.

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
from the CPUs available to the process, host-available or cgroup-limited RAM,
and available filesystem space. Use `--port <port>` only to override the
default port, or `--config <path>` to load an explicit TOML cache configuration.

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
memory limits or macOS host memory pressure plus filesystem availability, but
not filesystem quotas, SSD type, or device throughput. After storage workers
open their data files, OpenKache best-effort checks those opened files and
emits a non-fatal warning when any device is non-NVMe or cannot be identified.
NVMe is recommended for the intended latency and throughput profile, but it is
not a hard requirement. Its memory estimate covers the packed Table rather than
whole-process peak RSS. `--cpus` selects
worker threads but does not impose a process CPU quota. `light` and `balanced`
accept individual values up to their 1 MiB Blob Segment size; `heavy` accepts
up to 64 MiB. Existing storage must be reopened with the same worker count and
Segment layout.

To calculate a configuration from resource budgets instead, run:

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
100-byte inline values, while `heavy` models 2 KiB Blob values. Remove
`--plan` to open the storage files and start serving with the calculated
configuration.

The calculated limits are advisory. The planner does not inspect cgroup
limits, filesystem free space, or device throughput, and its memory estimate
covers the packed Table rather than whole-process peak RSS. `--cpus` selects
worker threads but does not impose a process CPU quota. `light` and `balanced`
accept individual values up to their 1 MiB Blob Segment size; `heavy` accepts
up to 64 MiB. Existing storage must be reopened with the same worker count and
Segment layout.

### Run the BCF53 benchmark

```bash
cargo run --package openkache --bin breadcrumb --release
```

### Use the SDK (Rust)

```rust
use openkache_client::value::{Compression, ZstandardOptions};
use openkache_client::{Certificate, Client, ClientRootKey, Endpoint};

let certificate = std::fs::read(
    "target/openkache-local/certificate.local.der",
)?;
let endpoint = Endpoint::from_socket_addr("127.0.0.1:4433".parse()?, "localhost")?;
let certificate = Certificate::from_der(certificate)?;
let client_root_key = ClientRootKey::from_base64(configured_base64_secret)?;
let client = Client::builder(endpoint, client_root_key)
    .trust_certificate(certificate)
    .compression(Compression::Zstandard(ZstandardOptions::default()))
    .connect()
    .await?;
client.set(b"mykey", b"myvalue").await?;
let value = client.get(b"mykey").await?;
```

## Next steps

- See [Architecture](../README.md#-architecture) for how OpenKache works
- See the client status and binding architecture in `clients/README.md`
- See the low-level shared client core under `clients/core/`
- See the Rust client SDK under `clients/rust/`
- See the Bash-friendly CLI under `clients/cli/`
- See the TypeScript client SDK under `clients/typescript/`
- See the .NET client SDK under `clients/dotnet/`
