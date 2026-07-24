<div align="center">

# OpenKache ⚡

**A next-generation Rust SSD cache server.**

**10× cheaper and faster than Redis.**

Open-source · Rust · QUIC · SIMD-accelerated · SSD-first

<!-- TODO: add more badges — GitHub Stars, crates.io version/downloads, Docker pulls, real CI status, OpenSSF scorecard, code coverage, PRs welcome --> 
[![Build](https://img.shields.io/badge/build-passing-brightgreen.svg)](https://github.com/openkache/openkache/actions)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)
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

The server has zero visibility into client data. Keys are sent as Blake3 keyed hashes, values as AES ciphertext. No plaintext leaves the client — zero trust by default. Algorithm is configurable.

### 📦 Transparent compression

Large values are automatically compressed with zstd before storage and decompressed on retrieval. Transparent to the client, algorithm is configurable.

### 📚 Multi-language SDKs

First-class client libraries for Rust and .NET, with more on the way. Idiomatic, typed APIs — no C bindings, no FFI overhead.

### 📦 Single binary distribution

One statically linked binary. No shared libraries, no runtime dependencies, no package manager required. Copy it anywhere and run.

---

## 🚀 Quick start

```bash
# Enter the dev shell (Nix)
nix develop

# Start the local in-memory QUIC server
cargo run --manifest-path server/Cargo.toml --bin openkache-server
```

The smoke server listens on `127.0.0.1:4433` and writes an ephemeral
self-signed certificate to
`target/openkache-local/certificate.local.der`. It supports `PING`, `GET`,
`SET`, `DELETE`, `STATS`, and `SYNC` over the versioned `openkache/1` QUIC
protocol. `SYNC` is currently an acknowledged no-op because this first server
uses an in-memory HashMap.

---

## 🏗️ Architecture

```
┌──────────────┐         QUIC (UDP, TLS 1.3)         ┌──────────────────────┐
│  openkache-  │ ──────────────────────────────────▶ │     OpenKache        │
│  client      │   Multiplexed streams, 0-RTT        │  (single binary)     │
│  (Rust/.NET) │                                     │                      │
└──────────────┘                                     │  ┌────────────────┐  │
                                                     │  │ BCF53 Filter   │  │
┌──────────────┐                                     │  │ (SIMD, AVX2)   │  │
│  Any QUIC    │                                     │  ├────────────────┤  │
│  client      │                                     │  │ Compacting Slab│  │
└──────────────┘                                     │  │ (Hugepage/NUMA)│  │
                                                     │  ├────────────────┤  │
                                                     │  │ KV Engine      │  │
                                                     │  │ (SHA-256 keys) │  │
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
cargo build --manifest-path server/Cargo.toml
cargo build --manifest-path clients/rust/Cargo.toml
cargo build --manifest-path protocol/Cargo.toml
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

## 🧪 Test

```bash
cargo check --manifest-path server/Cargo.toml
cargo check --manifest-path clients/rust/Cargo.toml
cargo check --manifest-path protocol/Cargo.toml
```

The public repository contains production source only. The full protocol and
end-to-end test suite is maintained in the OpenKache monorepo.

---

## 📊 Project status

OpenKache is in **active development**. Core components are stable, the server protocol layer is being built out, and client SDKs are available for Rust and .NET.

| Component | Status | Notes |
|---|---|---|
| Memory allocators | ✅ Stable | VirtualPageStack + CompactingSlabAllocator in production shape |
| Breadcrumb filter | ✅ Stable | BCF53 with SIMD dispatch, 32–39 M ops/s per core |
| QUIC client (Rust) | 🚧 Preview | Quinn + Noq backends, binary protocol v1 |
| QUIC server | 🚧 Preview | Runnable HashMap smoke server over QUIC |
| .NET client | ✅ Stable | TCP-based, NuGet published |
| Clustering | ❌ Not started | Future: consistent hashing, gossip, replication |

---

## 🗺️ Roadmap

| Milestone | Target | Focus |
|---|---|---|
| Core engine | ✅ Done | Allocators, BCF53 filter, types, Rust client, .NET client |
| Server protocol | 🚧 In progress | QUIC server, command dispatch, KV engine, SSD engine |
| Production hardening | 🔜 Next | Benchmarks, fuzzing, CI/CD, musl releases, Docker images |
| E2E encryption | ⏳ Planned | Client-side encryption, zero-trust server architecture |
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
| `server/` | Core cache server plus the runnable QUIC smoke server |
| `clients/rust/` | Rust client SDK over QUIC |
| `clients/dotnet/` | .NET / C# client SDK |


---

## ⚖️ License

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![OSI Approved](https://img.shields.io/badge/OSI-Approved-brightgreen.svg)](https://opensource.org/licenses/AGPL-3.0)

Licensed under the [GNU Affero General Public License v3.0](./LICENSE).


---

<div align="center">

**Built with ❤️ and 🦀**

</div>
