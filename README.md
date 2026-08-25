<div align="center">

# OpenKache ⚡

**An experimental Rust SSD-backed cache server.**

Open source · RESP/TCP · OpenKache/QUIC · Linux `io_uring`

[![Build](https://img.shields.io/badge/build-preview-orange.svg)](https://github.com/openkache/openkache/actions)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org/)

</div>

Unless a section is explicitly marked as a target or draft, this README
describes the current public preview. The protocol, client-format, security,
and storage design documents are target contracts; their implementations may
temporarily lag during the migration.

## What works today

The current server is a two-thread SSD cache prototype:

- RESP `GET`, `SET`, and `DEL` over TCP
- Gate 0 `PING`, `GET`, `SET`, and `DELETE` over OpenKache/QUIC
- one network thread and one storage thread pinned to distinct CPUs
- a fixed 16 GiB `openkache.data` file backed by `io_uring`
- Rust and multi-language SDKs built on the shared client core
- `linux/amd64` and `linux/arm64` container publication

This is not a production release. The server recreates its cache file on
startup, generates an ephemeral self-signed certificate, and does not
authenticate clients. TTL overrides, conditional writes, namespace
administration, statistics, synchronization, clustering, and restart recovery
are not implemented by the current server.

## Quick start

Requirements:

- Linux with `io_uring`
- two distinct CPUs available to the process
- Rust plus the native toolchain required by the workspace dependencies

Run on `127.0.0.1:4433` using CPUs 0 and 1:

```bash
cargo run --locked --package openkache-server --bin openkache-server
```

The server uses the same numeric address for RESP/TCP and OpenKache/QUIC. To
select a different address and CPU pair:

```bash
cargo run --locked --package openkache-server --bin openkache-server -- \
  0.0.0.0:4433 2 3
```

The cache file is created in the process working directory and truncated each
time the server starts.

### Use the Rust SDK

```rust
use openkache::{Client, GetResult, Value};

# async fn example() -> openkache::Result<()> {
let client = Client::connect("127.0.0.1:4433").await?;
client.set("greeting", Value::text("hello")).await?;
assert_eq!(
    client.get("greeting").await?,
    GetResult::Found(Value::text("hello")),
);
client.close().await?;
# Ok(())
# }
```

The Gate 0 SDK intentionally disables certificate verification for local
development. It still uses TLS 1.3 over QUIC and never falls back to plaintext.

### Try a maintained client

The three maintained packages use the local development TLS profile. It
disables certificate verification, so use these examples only against a local
development server; do not reuse this trust profile for production traffic.

| Package | Install | Documentation |
|---|---|---|
| TypeScript / JavaScript | `npm install openkache` or `bun add openkache` | [npm](https://www.npmjs.com/package/openkache) · [README](clients/typescript/README.md) |
| Python | `python -m pip install openkache` | [PyPI](https://pypi.org/project/openkache/) · [README](clients/python/README.md) |
| Rust | `cargo add openkache` | [crates.io](https://crates.io/crates/openkache) · [README](clients/rust/README.md) |

All three clients can connect to the default local endpoint:
`127.0.0.1:4433`.

TypeScript / JavaScript:

```typescript
import { OpenKache_Client } from "openkache"

const client = await OpenKache_Client.connect("127.0.0.1:4433")
try {
  console.log(await client.set("hello", { from: "javascript" }))
  console.log(await client.get("hello"))
  console.log(await client.delete("hello"))
} finally {
  await client.close()
}
```

Python:

```python
from openkache import Client

client = Client.connect("127.0.0.1:4433")
try:
    print(client.set("hello", {"from": "python"}))
    print(client.get("hello"))
    print(client.delete("hello"))
finally:
    client.close()
```

Rust:

```rust
use openkache::{Client, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::connect("127.0.0.1:4433").await?;
    client.set("hello", Value::text("from rust")).await?;
    println!("{:?}", client.get("hello").await?);
    client.delete("hello").await?;
    client.close().await?;
    Ok(())
}
```

Each package README contains a complete runnable example, result semantics,
supported key/value types, and package-specific build requirements.

### Container image

Build locally from the repository root:

```bash
docker build --file server/Dockerfile --tag localhost/openkache:dev .
docker run --rm \
  --security-opt seccomp=unconfined \
  --publish 4433:4433/tcp \
  --publish 4433:4433/udp \
  --volume openkache-data:/var/lib/openkache \
  localhost/openkache:dev
```

The default container command pins the network thread to CPU 0 and the storage
thread to CPU 1. Override the command when the container CPU set uses different
IDs. See the [container guide](./docs/container-image.md) for details.

## Build and verify

```bash
cargo check --locked
cargo test --locked --package openkache-server
cargo server-build
```

The root Cargo workspace owns the protocol, server, shared client core, Rust
SDK, CLI, and native TypeScript adapter under one lockfile.

Server allocator experiments are available as opt-in features:

```bash
cargo server-build --features alloc-jemalloc
cargo server-build --features alloc-mimalloc
```

Do not enable both allocator features at once.

## Client packages

Maintained client packages share the same protocol and value-format sources.
See [clients/README.md](./clients/README.md) for the current status of Rust,
TypeScript, Python, .NET, Go, C, C++, Swift, and other bindings.

The current server compatibility frontend supports only the Gate 0 operation
subset listed above. Broader APIs described by target contracts may be present
in generated clients before the server implements them.

## Repository layout

| Path | Contents |
| --- | --- |
| `server/` | Current SSD cache server and container definition |
| `protocol/` | Shared wire model, generated contracts, and codecs |
| `clients/` | Client SDKs and native adapters |
| `docs/` | Current usage guides and explicitly identified target documents |

The current server implementation lives in [server/README.md](./server/README.md).
Protocol details live in [protocol/README.md](./protocol/README.md).

## Project status

| Component | Status |
| --- | --- |
| RESP/TCP server | Preview |
| OpenKache/QUIC Gate 0 server | Preview |
| SSD storage and deletion | Preview |
| Restart recovery | Not implemented |
| Production authentication | Not implemented |
| Client SDKs | Preview; see package status |
| Container image | Available for Linux amd64/arm64 |
| Clustering | Not started |

## Contributing

- [Contributing guide](./CONTRIBUTING.md)
- [Community guidelines](./COMMUNITY_GUIDELINES.md)
- [Code of conduct](./CODE_OF_CONDUCT.md)

## License

Except where otherwise noted, OpenKache is licensed under the
[GNU Affero General Public License v3.0 or later](./LICENSE). Client SDKs under
`clients/` and the shared protocol under `protocol/` use the Apache License 2.0
as documented in their package directories.
