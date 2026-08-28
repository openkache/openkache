<div align="center">

# OpenKache ⚡

**OpenKache is a high-performance cache server designed from the ground up for modern SSDs.**

Open source · RESP/TCP · OpenKache/QUIC · Linux `io_uring`

[![Build](https://img.shields.io/badge/build-preview-orange.svg)](https://github.com/openkache/openkache/actions)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org/)

**English** · [한국어](./README.ko.md) · [中文](./README.zh.md)

</div>

## Table of contents

- [Benchmarks](#benchmarks)
- [Architecture](#architecture)
- [Quick start](#quick-start)
- [Connect a client](#connect-a-client)
- [Container image](#container-image)
- [Roadmap](#roadmap)
- [Build and verify](#build-and-verify)
- [Client packages](#client-packages)
- [Repository layout](#repository-layout)
- [Project status](#project-status)
- [Contributing](#contributing)
- [License](#license)
- [Third-party attributions](#third-party-attributions)

## Benchmarks

Measured on a 6 vCPU AMD EPYC 7773X host (SSD, kernel 6.8) over loopback, with
32-byte keys and 100-byte values. Each system is driven by a load tool speaking
its own native protocol. Full methodology in [BENCHMARK.md](./BENCHMARK.md).

**GET throughput**

| System | GET throughput | Load tool |
|---|---:|---|
| OpenKache | **97,887 ops/s** | kvbench (RESP) |
| PostgreSQL 17.10 | 17,421 ops/s | kvbench (PostgreSQL wire) |
| MySQL 8.4.11 | 16,295 ops/s | kvbench (MySQL wire) |

OpenKache is 5.6× faster than PostgreSQL and 6.0× faster than MySQL, reaching
76% of the machine's single-core 4 KiB random-read ceiling (128,820 IOPS) with
a single storage core.

**GET latency (one request at a time)**

| System | avg | p50 | p99 | p99.9 |
|---|---:|---:|---:|---:|
| OpenKache | **238.7 µs** | 229 µs | 386 µs | 1376 µs |
| MySQL 8.4.11 | 385.7 µs | 410 µs | 1169 µs | 2207 µs |
| PostgreSQL 17.10 | 558.0 µs | 510 µs | 1263 µs | 3342 µs |

Average GET latency is 1.6× lower than MySQL and 2.3× lower than PostgreSQL; at
p99 it is 3.0× and 3.3× lower.

## Architecture

<div align="center">

<img src="docs/assets/openkache-architecture.png" alt="OpenKache Architecture"/>

</div>

**Why is OpenKache fast? Because a request never leaves the core it landed on.**

Most servers scatter work across a thread pool that roams between cores. That is
not free. Every time a request hops cores you pay for it: lock contention,
mutexes, context switches, and cache lines ping-ponging between caches, each one
forcing a synchronization and a copy. Under load it is this coordination
overhead, not the actual work, that caps throughput.

OpenKache refuses to pay. Each decision below deletes an entire class of that cost:

| Design choice | The cost it deletes | Grounded in |
|---|---|---|
| **Thread-per-core, shared-nothing** | Lock contention, context switches, cross-core cache-line bouncing | [KVell (SOSP '19)](https://dl.acm.org/doi/10.1145/3341301.3359628) |
| **Segment-group sequential writes** | Random-write IOPS, flash write amplification | [FairyWren (OSDI '24)](https://www.usenix.org/conference/osdi24/presentation/mcallister) |
| **`io_uring` + direct I/O** | System-call overhead, kernel page-cache copies | — |
| **Written in Rust** | GC pauses; data races caught by the compiler, not in production | — |

KVell's lesson is blunt: on a modern SSD the bottleneck is no longer the disk.
It is the CPU. So OpenKache pins each worker to its own core, hands it its own
data, and lets it run without a single lock. The network worker and the storage
worker each own a core and speak through one lock-free SPSC queue, so parsing
never blocks disk I/O and disk I/O never blocks parsing.

FairyWren makes the second point: a flash cache burns its lifespan on small
random writes. So OpenKache never issues one. It batches many keys into a single
sequential **segment-group** flush, a subway carrying a full train instead of
one car per passenger, riding the SSD's sequential bandwidth to the ceiling.

**Rust makes this safe to push to the limit: bare-metal control with no garbage
collector to stall the hot path, and data races caught by the compiler, not in
production.**

See [docs/architecture.md](./docs/architecture.md) for the full design.

## Quick start

OpenKache is optimized and benchmarked for **Linux**. Its high-performance
`io_uring` network frontend, direct-I/O storage path, and CPU-pinned runtime are
Linux-only. Apple Silicon macOS has a deliberately unoptimized portability
preview that uses Tokio polling and buffered file I/O; it is intended for
functional development, not performance comparisons. Windows has no native
server; WSL2 uses the Linux build and requires a kernel that permits `io_uring`.

Start the server before connecting a client. Choose the first option that fits
your environment: container image, Homebrew or APT package, release archive,
or Cargo.

Linux requirements:

- Linux with `io_uring`
- two distinct CPUs available to the process

### Container image

Run the published preview image without authenticating to GHCR:

```bash
podman run --rm \
  --security-opt seccomp=unconfined \
  --publish 4433:4433/tcp \
  --publish 4433:4433/udp \
  ghcr.io/openkache/openkache:edge
```

`edge` follows the latest successful build from `main`. For reproducible
deployments, pin the multi-platform manifest by its `sha256` digest instead of
using the rolling tag.

To build the image locally, run from the repository root:

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

### Homebrew (Apple Silicon macOS)

Download the formula attached to the matching release, then let Homebrew
install the server and CLI. The formula test starts the server and exercises
`PING`, `SET`, `GET`, and `DELETE` through `openkache-cli`.

```bash
VERSION="${SERVER_VERSION:-0.1.0}"
BASE="https://github.com/openkache/openkache/releases/download/server-v${VERSION}"
curl --fail --location --remote-name "${BASE}/openkache.rb"
brew install --formula ./openkache.rb
openkache-server
```

The macOS server is intentionally unoptimized. It preserves the protocol for
local functional development, but every published performance claim refers to
the Linux server.

### APT package (Ubuntu, Debian, and WSL2)

Download the package for the machine's Debian architecture and install it with
APT. The package includes the server, CLI, configuration, and a loopback-only
systemd unit; the service is not enabled automatically.

```bash
VERSION="${SERVER_VERSION:-0.1.0}"
ARCH="$(dpkg --print-architecture)"
BASE="https://github.com/openkache/openkache/releases/download/server-v${VERSION}"
PACKAGE="openkache_${VERSION}_${ARCH}.deb"
curl --fail --location --remote-name "${BASE}/${PACKAGE}"
sudo apt install "./${PACKAGE}"
openkache-server
```

On a systemd host, start the optional service with
`sudo systemctl enable --now openkache`. On WSL2 without systemd, run
`openkache-server` in the foreground.

### Linux release archive

When a `server-v<version>` release is available, download the matching Linux
archive from [GitHub Releases](https://github.com/openkache/openkache/releases).
Set `SERVER_VERSION` to the published version, then verify and run the archive:

```bash
VERSION="${SERVER_VERSION:-0.1.0}"
PLATFORM="linux-x86_64-musl" # use linux-aarch64-musl on arm64
BASE="https://github.com/openkache/openkache/releases/download/server-v${VERSION}"
ARCHIVE="openkache-server-${VERSION}-${PLATFORM}.tar.gz"
curl --fail --location --remote-name "${BASE}/${ARCHIVE}"
curl --fail --location --remote-name "${BASE}/SHA256SUMS"
grep -F " ${ARCHIVE}" SHA256SUMS | sha256sum --check
tar -xzf "${ARCHIVE}"
"./openkache-server-${VERSION}-${PLATFORM}/openkache-server"
```

### Cargo

Building from source also requires Rust and the native toolchain used by the
workspace dependencies. Run the server on `127.0.0.1:4433` using CPUs 0 and 1:

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

## Connect a client

With the server running, connect from your language of choice. All client
guides use `127.0.0.1:4433` as the default local endpoint and list the complete
public API for their language.

| Package | Install | Documentation | Source |
|---|---|---|---|
| TypeScript / JavaScript | `npm install openkache` | [npm](https://www.npmjs.com/package/openkache) · [client README](clients/typescript/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/typescript) |
| Python | `python -m pip install openkache` | [PyPI](https://pypi.org/project/openkache/) · [client README](clients/python/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/python) |
| Rust | `cargo add openkache` | [crates.io](https://crates.io/crates/openkache) · [docs.rs](https://docs.rs/openkache/latest/openkache/) · [client README](clients/rust/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/rust) |

The Rust SDK in a nutshell:

```rust
use openkache::{Client, Value};

# async fn example() -> openkache::Result<()> {
let client = Client::connect("127.0.0.1:4433").await?;
client.set("greeting", Value::text("hello")).await?;
assert_eq!(
    client.get("greeting").await?.unwrap(),
    Value::text("hello"),
);
client.close().await?;
# Ok(())
# }
```

The source-built [`openkache-cli`](clients/cli/README.md) is the Bash-friendly
option, using the same fixed Gate 0 profile by default:

```bash
openkache-cli set hello "from cli"
openkache-cli get hello
```

Use `openkache-cli --profile configured` when certificate roots, mutual TLS,
client-side value protection, or compatibility-only TTL/conditional writes are
required.

> **Local development trust profile.** The default Gate 0 profile is for local
> development: it uses TLS 1.3 over QUIC and never falls back to plaintext, but
> it does not verify the server certificate. Do not reuse this trust profile for
> production traffic.

## Roadmap

| Milestone | Status | Focus |
|---|---|---|
| SSD storage & dual-protocol server | 🚧 In progress | Segment-group writes, RESP/TCP + QUIC Gate 0 |
| Production hardening | 🔜 Next | Restart recovery, reproducible benchmarks, fuzzing, CI/CD |
| Security & correctness | 📅 Planned | Auth/mTLS, value-protection profiles, full protocol surface |
| Scale & reach | 📅 Planned | Clustering, cross-platform servers, general availability |

Full detail in [ROADMAP.md](./ROADMAP.md).

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
[GNU Affero General Public License v3.0 or later](./LICENSE). Client SDKs
under [`clients/`](./clients/) and the shared protocol under
[`protocol/`](./protocol/) are licensed under the Apache License 2.0; see
the `LICENSE` file in each directory.

## Third-party attributions

Release archives, container images, and client packages include a generated
`THIRD-PARTY-NOTICES.txt` derived from the locked Cargo dependency graph. It
preserves upstream license, notice, and attribution text; OpenKache's own
license remains in the adjacent `LICENSE` file.

Entries without a separate upstream license file are flagged for maintainer
review. Do not redistribute an artifact while its notice contains
`LEGAL REVIEW REQUIRED`. See [`RELEASING.md`](./RELEASING.md) for release
checks and artifact-specific instructions.
