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
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [License](#license)
- [Third-party attributions](#third-party-attributions)

---

## Benchmarks

Benchmarks ran over loopback on
[serveroptima1](./benchmark/BENCHMARK.md#test-environment).

All three systems were benchmarked with kvbench using their native protocols.
Each database uses a different protocol, so we built kvbench to measure them
consistently. See the [full methodology and kvbench
details](./benchmark/BENCHMARK.md).

**GET throughput**

| System | GET throughput | Load tool |
|---|---:|---|
| OpenKache | **97,887 ops/s (1×)** | kvbench (RESP) |
| PostgreSQL 17.10 | 17,421 ops/s (0.18×) | kvbench (PostgreSQL wire) |
| MySQL 8.4.11 | 16,295 ops/s (0.17×) | kvbench (MySQL wire) |

OpenKache reaches 76% of the hardware limit (128,820 IOPS, measured with
[fio](https://github.com/axboe/fio)).

**GET latency (one request at a time)**

| System | avg | p50 | p99 | p99.9 |
|---|---:|---:|---:|---:|
| OpenKache | **238.7 µs (1×)** | 229 µs (1×) | 386 µs (1×) | 1376 µs (1×) |
| MySQL 8.4.11 | 385.7 µs (1.6×) | 410 µs (1.8×) | 1169 µs (3.0×) | 2207 µs (1.6×) |
| PostgreSQL 17.10 | 558.0 µs (2.3×) | 510 µs (2.2×) | 1263 µs (3.3×) | 3342 µs (2.4×) |

---

## Architecture

<div align="center">

<img src="docs/assets/openkache-architecture.png" alt="OpenKache Architecture"/>

</div>

**Why is OpenKache fast?** Because it never hops between cores.

Most servers let a thread pool roam across cores. That is not free: lock
contention, mutexes, context switches, and the synchronization and copy cost
paid every time a cache line bounces from one core to another. The heavier the
load, the more this overhead eats into throughput.

OpenKache takes a **thread-per-core (shared-nothing)** design. Each worker is
pinned to a single core, owns its own data, and shares no state, so there are
no locks. This is the same design TigerBeetle, ScyllaDB, and Redis converged on
to squeeze every drop out of the hardware. The network path and the storage
path each own a core, and they communicate through exactly one **lock-free SPSC
queue**. RESP parsing never blocks disk I/O.

Redis runs commands on a single core. OpenKache keeps the same shared-nothing
principle but shards workers across cores, so throughput scales with the
hardware instead of hitting a single-core ceiling. With no shared locks,
adding a core adds no contention.

Values live on the SSD; keys live in a compact RAM index (compressed key →
segment offset). And just as a subway moves more people than a car, OpenKache
batches writes from many keys into a single sequential **segment-group** flush
instead of one SSD write per key, using the drive's sequential bandwidth to the
fullest. On Linux, it submits that I/O through `io_uring` to erase even the
system-call overhead.

All of it is written in **Rust**: no GC pauses, data races ruled out at compile
time, C-level control in hand. There is no room for a garbage-collection pause
on the fast path.

See [docs/architecture.md](./docs/architecture.md) for the full design.

---

## Quick start

The installer downloads the latest tagged release, selects the correct archive
for Linux x86-64, Linux ARM64, or Apple Silicon macOS, verifies its SHA-256
checksum, and installs both `openkache-server` and `openkache-cli` to
`~/.local/bin`. Windows users can run the Linux release in WSL2.

### Install OpenKache

```bash
curl -fsSL https://github.com/openkache/openkache/raw/main/install.sh | sh
```

You can [review the installer](./install.sh) before running it.

If your shell reports `command not found`, add the installation directory to
`PATH` for the current terminal:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Linux requires `io_uring` and two available CPUs. The Apple Silicon macOS
binary uses the native functional-development path; performance claims apply
only to Linux.

### Terminal 1: start the server

```bash
openkache-server
```

Keep this terminal open while you use OpenKache.

### Terminal 2: verify the server

Open a second terminal and run:

```bash
openkache-cli ping
# PONG

openkache-cli set hello "from CLI"
# CREATED

openkache-cli get hello
# from CLI
```

Return to Terminal 1 and press <kbd>Ctrl</kbd>+<kbd>C</kbd> to stop the server.

---

## Connect a client

With the server running, connect from your language of choice. All client
guides use `127.0.0.1:4433` as the default local endpoint and list the complete
public API for their language.

OpenKache publishes separate client packages for TypeScript/JavaScript, Python,
and Rust. All three share the same protocol and value-format sources:

| Package | Install | Documentation | Source |
|---|---|---|---|
| TypeScript / JavaScript | `npm install openkache` | [npm](https://www.npmjs.com/package/openkache) · [client README](clients/typescript/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/typescript) |
| Python | `python -m pip install openkache` | [PyPI](https://pypi.org/project/openkache/) · [client README](clients/python/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/python) |
| Rust | `cargo add openkache` | [crates.io](https://crates.io/crates/openkache) · [docs.rs](https://docs.rs/openkache/latest/openkache/) · [client README](clients/rust/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/rust) |

See [clients/README.md](./clients/README.md) for the status of additional
bindings, including .NET, Go, C, C++, and Swift.

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

The [`openkache-cli`](clients/cli/README.md) binary included in every tagged
server release is the Bash-friendly option. It uses the same fixed Gate 0
profile by default; see the [quick start](#quick-start) for a complete example.

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

---

## Third-party attributions

Release archives, container images, and client packages include a generated
`THIRD-PARTY-NOTICES.txt` derived from the locked Cargo dependency graph. It
preserves upstream license, notice, and attribution text; OpenKache's own
license remains in the adjacent `LICENSE` file.

Entries without a separate upstream license file are flagged for maintainer
review. Do not redistribute an artifact while its notice contains
`LEGAL REVIEW REQUIRED`. See [`RELEASING.md`](./RELEASING.md) for release
checks and artifact-specific instructions.
