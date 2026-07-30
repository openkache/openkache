<div align="center">

# OpenKache ⚡

**A next-generation Rust SSD cache server.**

**10× cheaper and faster than Redis.**

Open-source · Rust · QUIC · SIMD-accelerated · SSD-first

<!-- TODO: add more badges — GitHub Stars, crates.io version/downloads, Docker pulls, real CI status, OpenSSF scorecard, code coverage, PRs welcome --> 
[![Build](https://img.shields.io/badge/build-passing-brightgreen.svg)](https://github.com/openkache/openkache/actions)
[![Rust](https://img.shields.io/badge/rust-nightly-orange.svg)](https://www.rust-lang.org/)
[![SIMD](https://img.shields.io/badge/simd-AVX2%20|%20AVX--512%20|%20NEON%20|%20SVE2-blueviolet)](#index-simd-accelerated-bcf53-breadcrumb-filter)

</div>


## ✨ Features

### 💾 SSD-first architecture

OpenKache uses **SSD as primary storage**, not a secondary tier. This eliminates the DRAM cost bottleneck while delivering higher throughput and lower latency than Redis.

### 🔍 Index: SIMD-accelerated BCF53 Breadcrumb Filter

The first production-grade Rust implementation of the state-of-the-art BCF53 Breadcrumb Filter.

- **Runtime SIMD dispatch**: Detects the best available ISA at startup — AVX-512BW → AVX2 → SVE2 → SVE → NEON → scalar fallback.
- **Compact bit packing**: 53 mini-buckets, 8-bit tags — 51 single-choice + 35 two-choice entries per cacheline.
- **32+ million ops/sec per core** (AVX2).

### 📡 Transport: QUIC, not TCP

- **Multiplexed, connection-oriented**: Hundreds of concurrent streams over a single connection. No connection pool needed, no head-of-line blocking.
- **TLS 1.3 baked in**: Every connection is encrypted by default.
- **Pluggable backends**: Choose between Quinn, Noq, or Quiche — swap with a feature flag.

### 🔒 End-to-end encryption

Secure clients compress values and encrypt them with XChaCha20-Poly1305 before
transmission. The compact authenticated value is bound to its exact 32-byte wire
key, so moving ciphertext to another cache key fails authentication. Protected
clients hide application keys behind HMAC-SHA-256 item keys. The server observes
deterministic item keys and encoded sizes, but not application keys or value
plaintext.

### 📦 Transparent compression

Large values are automatically compressed with zstd before storage and decompressed on retrieval. Transparent to the client, algorithm is configurable.

### 📚 Multi-language SDKs

Implemented client libraries are available for Rust, TypeScript and JavaScript
on Node.js, Bun, and Deno, and .NET. JavaScript runtimes share the TypeScript
package, which calls the shared low-level client core through Node-API so transport,
compression, and encryption behavior stay identical. Package scaffolds for
Python, Go, Java, Kotlin, C, C++, Swift, and Dart are available under
[`clients/`](./clients/README.md) for future Rust-backed bindings.

### 📦 Single binary distribution

One statically linked binary. No shared libraries, no runtime dependencies, no package manager required. Copy it anywhere and run.

---

## 🚀 Quick start

```bash
# Start the local SSD-backed QUIC server
cargo run --manifest-path server/Cargo.toml --bin openkache-server
```

The server listens on `127.0.0.1:4433`, stores shard files under
`target/kvkache-v1`, and writes an ephemeral
self-signed certificate to
`target/openkache-local/certificate.local.der`. It supports `PING`, `GET`,
`SET`, `DELETE`, `STATS`, and `SYNC` over the versioned `openkache/3` QUIC
protocol. `SET` accepts an optional millisecond TTL and atomic `if_absent` or
`if_present` existence condition. Expired values are treated as absent
immediately, while their SSD space is reclaimed when the containing Segment
Group is reused. `SYNC` flushes each SSD worker before acknowledging the
request. Pass `--port <port>` only when overriding the default port, or pass
`--config <path>` to load an explicit TOML cache configuration.
The complete byte-level contract is the
[wire protocol v3 specification](./protocol/SPEC.md).

The default loopback endpoint accepts unauthenticated clients and grants them
administrative commands for local development. `--insecure-development` is
required to extend that behavior to an explicitly selected non-loopback
address. Production non-loopback startup requires a stable server certificate
and private key, a client CA for mTLS, and an administrator certificate
allowlist. See the [production TLS guide](#production-tls).

### Production TLS

Create a small internal PKI without OpenSSL:

```bash
openkache-server pki init
openkache-server pki issue-server --dns cache.example.com --ip 10.0.0.10
openkache-server pki issue-client application-01
openkache-server pki issue-admin operator-01
openkache-server pki list
```

The default workspace is `_local/openkache-pki`. Keep
`authority/ca.key` offline, deploy only the generated `server/` directory, and
start with `--pki-directory /etc/openkache/pki`. Client and administrator
directories are portable bundles containing the trusted CA, leaf certificate,
and private key.

Configure PEM or DER paths in the server TOML:

```toml
[tls]
certificate_chain = "/etc/openkache/tls/server-chain.pem"
private_key = "/etc/openkache/tls/server-key.pem"
client_ca = "/etc/openkache/tls/client-ca-bundle.pem"
admin_client_certificates = [
  "/etc/openkache/tls/operators/admin-2026.pem",
]
```

The server certificate must contain every client-facing DNS name and IP
address in its SANs. `client_ca` authenticates all clients. Only authenticated
clients whose exact leaf certificate appears in
`admin_client_certificates` may run `STATS` or `SYNC`.

TLS files are loaded at startup. For server identity rotation, deploy client
trust for both issuers first, place the replacement chain and key at new paths,
and roll servers one at a time. For client-CA rotation, temporarily put both
CAs in `client_ca` before reissuing clients. For administrator rotation, list
both old and new leaf certificates until the new identity has been verified,
then remove the old entry and roll again. Keep private keys readable only by
the service account.

Without TOML, the server uses all CPUs permitted by process affinity, the
smaller of available host RAM and remaining common cgroup headroom, and
available space on the storage directory's filesystem. The automatic process
budget preserves at least 20% of currently available RAM, then limits the
packed Table to half of that budget. It starts with the default `balanced`
profile for 1 KiB values. `light` models 100-byte values and `heavy` models
2 KiB values.

```bash
cargo run --manifest-path server/Cargo.toml --bin openkache-server -- \
  --port 6380 \
  --cpus 4 \
  --memory-gib 32 \
  --storage-gb 2500 \
  --directory ./openkache-data \
  --plan
```

Every argument in this example is optional. `--plan` prints the automatically
detected or overridden sizing result without starting. The planner reserves 5%
of available SSD space, limits the packed Table to 50% of the safe process RAM
budget, and targets 75% of theoretical SG key capacity so updates and
Tombstones have room.

Sizing is a deterministic capacity estimate, not an adaptive benchmark. It
detects the standard Linux cgroup limits and current usage plus filesystem
availability, but does not detect filesystem quotas, SSD type, or NVMe
performance. At runtime, workers reserve each Segment generation immediately
before writing it instead of preallocating the whole sparse file. Memory or
storage pressure temporarily rejects `SET` with an overloaded response while
reads, deletes, and recovery remain available. `STATS` reports
the memory and storage stop/resume thresholds, `memory_stop_writes`,
`storage_stop_writes`, and `rejected_writes`.

`--cpus` selects worker threads but does not impose a process CPU quota; use
deployment affinity or cgroups for that boundary. `light` and `balanced` use
1 MiB Blob Segments, so one value cannot exceed 1 MiB; `heavy` raises that
limit to 64 MiB. Reopen existing storage with the same automatically detected
layout or explicit sizing overrides because worker count and Segment layout
changes require cache recreation.

For a resource-sized configuration without TOML, provide the worker CPU count,
RAM limit, SSD limit, and storage directory. The default `balanced` profile
models 1 KiB values; `light` models 100-byte values and `heavy` models 2 KiB
values.

```bash
cargo run --manifest-path server/Cargo.toml --bin openkache-server -- \
  --cpus 4 \
  --memory-gib 32 \
  --storage-gb 2500 \
  --directory ./openkache-data \
  --plan
```

Remove `--plan` to start the server with the displayed sizing result. The
planner reserves 5% of available SSD space, keeps at least 20% of currently
available RAM outside the process budget, limits the packed Table to half of
that budget, and targets 75% of theoretical SG key capacity so updates and
Tombstones have room.

Sizing is a deterministic capacity estimate, not an adaptive benchmark. It
inspects cgroup memory headroom and filesystem free space, but does not detect
filesystem quotas or NVMe performance. Runtime Segment reservation and
stop-writes thresholds protect the remaining headroom from concurrent resource
use. `--cpus` selects worker threads but does not impose a process CPU quota;
use deployment affinity or cgroups for that boundary. `light` and `balanced`
use 1 MiB Blob Segments, so one value cannot exceed 1 MiB; `heavy` raises that
limit to 64 MiB. Reuse the same sizing arguments when reopening existing
storage because worker count and Segment layout changes require cache
recreation.

---

## 🏗️ Architecture

```
┌──────────────┐         QUIC (UDP, TLS 1.3)         ┌──────────────────────┐
│  openkache-  │ ──────────────────────────────────▶ │     OpenKache        │
│  client      │   Multiplexed streams, 0-RTT        │  (single binary)     │
│ (Rust/TS/.NET)│                                    │                      │
└──────────────┘                                     │  ┌────────────────┐  │
                                                     │  │ BCF53 Filter   │  │
┌──────────────┐                                     │  │ (SIMD, AVX2)   │  │
│  Any QUIC    │                                     │  ├────────────────┤  │
│  client      │                                     │  │ Compacting Slab│  │
└──────────────┘                                     │  │ (Hugepage/NUMA)│  │
                                                     │  ├────────────────┤  │
                                                     │  │ KV Engine      │  │
                                                     │  │ (32-byte keys) │  │
                                                     │  ├────────────────┤  │
                                                     │  │ SSD Engine     │  │
                                                     │  └────────────────┘  │
                                                     └──────────────────────┘
```

---

## ⚔️ Comparison

| | Redis | OpenKache |
|---|---|---|
| **Cost per GB** | DRAM (~$3–5/GB) | SSD (~$0.05–0.10/GB) |
| **P99 latency** | <1 ms | <1 ms |
| **Throughput** | ~100K ops/s (single node) | **1M+ ops/s** (single node) |
| **Transport** | TCP (head-of-line blocking) | QUIC (TLS 1.3, multiplexed, 0-RTT) |
| **Security** | TLS optional, no E2E | **E2E encrypted by default**, zero trust |

---

## 🛠️ Build

### Native (fast iteration)

```bash
cargo build --locked
bun run --cwd clients/typescript build:native
```

The root Cargo workspace owns the server, protocol, shared client core, Rust
client, and Node-API adapter under one `Cargo.lock`. The default build omits
only the Node-API adapter, which the TypeScript build stages separately.

### Server allocator

The server uses jemalloc by default. Select the system allocator, mimalloc, or
snmalloc by disabling default features and enabling exactly one allocator
feature:

```bash
cargo build --release --manifest-path server/Cargo.toml \
  --bin openkache-server \
  --no-default-features \
  --features allocator-mimalloc,channel-crossfire,quic-noq
```

The allocator features are mutually exclusive. The server reports the selected
allocator during startup.

### Server channel

The server uses Crossfire by default for worker, request/reply, and transport
channels. Select exactly one of `channel-crossfire`, `channel-flume`, or
`channel-kanal` at compile time:

```bash
cargo build --release --manifest-path server/Cargo.toml \
  --bin openkache-server \
  --no-default-features \
  --features allocator-system,channel-flume,quic-noq
```

### Static musl (x86_64 / aarch64)

```bash
cargo zigbuild --target x86_64-unknown-linux-musl
cargo zigbuild --target aarch64-unknown-linux-musl
```

### Both architectures

```bash
cargo release-all
```

---

## ✅ Verify the build

```bash
cargo check --locked
```

---

## 📊 Project status

OpenKache is in **active development**. Core components are stable, the server protocol layer is being built out, and client SDKs are available for Rust, TypeScript, and .NET.

| Component | Status | Notes |
|---|---|---|
| Memory allocators | ✅ Stable | VirtualPageStack + CompactingSlabAllocator in production shape |
| Breadcrumb filter | ✅ Stable | BCF53 with SIMD dispatch, 32–39 M ops/s per core |
| QUIC client (Rust) | 🚧 Preview | Shared Rust core, binary protocol v3, secure value codec |
| QUIC client (TypeScript) | 🚧 Preview | Node.js, Bun, and Deno-compatible Node-API SDK |
| QUIC server | 🚧 Preview | SSD-backed worker shards over multiplexed QUIC streams |
| QUIC client (.NET) | 🚧 Preview | Managed `System.Net.Quic`, binary protocol v2 |
| Clustering | ❌ Not started | Future: consistent hashing, gossip, replication |

---

## 🗺️ Roadmap

| Milestone | Target | Focus |
|---|---|---|
| Core engine | ✅ Done | Allocators, BCF53 filter, types, and client foundations |
| Server protocol | 🚧 In progress | Recovery, operational hardening, and stable configuration |
| Production hardening | 🔜 Next | Benchmarks, fuzzing, CI/CD, musl releases, Docker images |
| E2E encryption | ✅ Done | Zstandard then compact XChaCha20-Poly1305 values |
| Clustering | 📅 Future | Consistent hashing, gossip protocol, replication, failover |
| General availability | 🎯 Future | Stable API, cross-platform packages, production docs |

---

## 🤖 AI coding agents

OpenKache provides [`/llms.txt`](./llms.txt) and [`/llms-full.txt`](./llms-full.txt) for LLM-friendly documentation.

---

## 📁 Project structure

| Path | Contents |
|---|---|
| `protocol/` | Shared binary request, response, opcode, and status definitions |
| `server/` | SSD cache engine plus the runnable QUIC server |
| `clients/` | Implemented SDKs and thin-binding package scaffolds |
| `clients/core/` | Low-level QUIC client core shared by language adapters |
| `clients/rust/` | Ergonomic Rust end-user SDK over the client core |
| `clients/typescript/` | Node.js, Bun, and Deno client backed by Rust through Node-API |
| `clients/dotnet/` | Managed .NET client over QUIC |


---

## ⚖️ License

Except where otherwise noted, OpenKache is licensed under the
[GNU Affero General Public License v3.0 or later](./LICENSE). Client SDKs
under [`clients/`](./clients/) and the shared protocol under
[`protocol/`](./protocol/) are licensed under the Apache License 2.0; see
the `LICENSE` file in each directory.


---

<div align="center">

**Built with ❤️ and 🦀**

</div>
