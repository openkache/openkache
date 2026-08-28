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
- [Contributing](#contributing)
- [License](#license)

---

## Benchmarks

Measured on a 6 vCPU AMD EPYC 7773X host (SSD, kernel 6.8) over loopback, with
32-byte keys and 100-byte values. Each system is driven by a load tool speaking
its own native protocol. Full methodology in [BENCHMARK.md](./BENCHMARK.md).

**GET throughput**

| System | GET throughput | Load tool |
|---|---:|---|
| OpenKache | **97,887 ops/s** | memtier (RESP) |
| PostgreSQL 17.10 | 17,421 ops/s | pgbench |
| MySQL 8.4.11 | 16,295 ops/s | sysbench |

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

---

## Architecture

<div align="center">

<img src="docs/assets/openkache-architecture.png" alt="OpenKache Architecture"/>

</div>

**Why is OpenKache fast?** Because it never hops between cores.

Most servers let a thread pool roam across cores. That is not free — lock
contention, mutexes, context switches, and the synchronization and copy cost
paid every time a cache line bounces from one core to another. The heavier the
load, the more this overhead eats into throughput.

OpenKache takes a **thread-per-core (shared-nothing)** design. Each worker is
pinned to a single core, owns its own data, and shares no state — so there are
no locks. This is the same design TigerBeetle, ScyllaDB, and Redis converged on
to squeeze every drop out of the hardware. The network path and the storage
path each own a core, and they communicate through exactly one **lock-free SPSC
queue**. RESP parsing never blocks disk I/O.

Redis runs commands on a single core. OpenKache keeps the same shared-nothing
principle but shards workers across cores, so throughput scales with the
hardware instead of hitting a single-core ceiling — and with no shared locks,
adding a core adds no contention.

Values live on the SSD; keys live in a compact RAM index (compressed key →
segment offset). And just as a subway moves more people than a car, OpenKache
batches writes from many keys into a single sequential **segment-group** flush
instead of one SSD write per key, using the drive's sequential bandwidth to the
fullest — on Linux, submitting that I/O through `io_uring` to erase even the
system-call overhead.

All of it is written in **Rust**: no GC pauses, data races ruled out at compile
time, C-level control in hand. There is no room for a garbage-collection pause
on the fast path.

See [docs/architecture.md](./docs/architecture.md) for the full design.

---

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

---

## Connect a client

With the server running, connect from your language of choice. All client
guides use `127.0.0.1:4433` as the default local endpoint and list the complete
public API for their language.

Maintained client packages share the same protocol and value-format sources.
See [clients/README.md](./clients/README.md) for the current status of Rust,
TypeScript, Python, .NET, Go, C, C++, Swift, and other bindings.

| Package | Install | Documentation | Source |
|---|---|---|---|
| TypeScript / JavaScript | `npm install openkache` | [npm](https://www.npmjs.com/package/openkache) · [client README](clients/typescript/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/typescript) |
| Python | `python -m pip install openkache` | [PyPI](https://pypi.org/project/openkache/) · [client README](clients/python/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/python) |
| Rust | `cargo add openkache` | [crates.io](https://crates.io/crates/openkache) · [docs.rs](https://docs.rs/openkache/latest/openkache/) · [client README](clients/rust/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/rust) |

The Rust SDK in a nutshell:

```rust
use openkache::{Client, Value};

async fn example() -> openkache::Result<()> {
    let client = Client::connect("127.0.0.1:4433").await?;
    client.set("greeting", Value::text("hello")).await?;
    assert_eq!(
        client.get("greeting").await?.unwrap(),
        Value::text("hello"),
    );
    client.close().await?;
    Ok(())
}
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

---

## Roadmap

| Milestone | Status | Focus |
|---|---|---|
| SSD storage & dual-protocol server | 🚧 In progress | Segment-group writes, RESP/TCP + QUIC Gate 0 |
| Production hardening | 🔜 Next | Restart recovery, reproducible benchmarks, fuzzing, CI/CD |
| Security & correctness | 📅 Planned | Auth/mTLS, value-protection profiles, full protocol surface |
| Scale & reach | 📅 Planned | Clustering, cross-platform servers, general availability |

Full detail in [ROADMAP.md](./ROADMAP.md).

---

## Contributing

- [Contributing guide](./CONTRIBUTING.md)
- [Community guidelines](./COMMUNITY_GUIDELINES.md)
- [Code of conduct](./CODE_OF_CONDUCT.md)

---

## License

Except where otherwise noted, OpenKache is licensed under the
[GNU Affero General Public License v3.0 or later](./LICENSE). Client SDKs
under [`clients/`](./clients/) and the shared protocol under
[`protocol/`](./protocol/) are licensed under the Apache License 2.0; see
the `LICENSE` file in each directory.
