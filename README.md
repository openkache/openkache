<div align="center">

# OpenKache ⚡

**An experimental Rust SSD-first cache server.**

Open-source · Rust · QUIC/TLS-over-TCP · SIMD-accelerated · SSD-first

<!-- TODO: add more badges — GitHub Stars, crates.io version/downloads, Docker pulls, real CI status, OpenSSF scorecard, code coverage, PRs welcome -->
[![Build](https://img.shields.io/badge/build-passing-brightgreen.svg)](https://github.com/openkache/openkache/actions)
[![Rust](https://img.shields.io/badge/rust-nightly-orange.svg)](https://www.rust-lang.org/)
[![SIMD](https://img.shields.io/badge/simd-AVX2%20|%20AVX--512%20|%20NEON%20|%20SVE2-blueviolet)](#index-simd-accelerated-bcf53-breadcrumb-filter)
[![npm](https://img.shields.io/npm/v/openkache?logo=npm)](https://www.npmjs.com/package/openkache)
[![crates.io](https://img.shields.io/crates/v/openkache?logo=rust)](https://crates.io/crates/openkache)
[![PyPI](https://img.shields.io/pypi/v/openkache?logo=pypi)](https://pypi.org/project/openkache/)

</div>

Unless a section is explicitly marked as a target or draft, this README
describes the current public preview. The protocol, client-format, security,
and storage design documents are target contracts; their implementations may
temporarily lag during the migration.

## ✨ Features

### 💾 SSD-first architecture

OpenKache is designed to use **SSD as primary storage**, not a secondary tier.
The current server is a preview; restart recovery and publication-quality
capacity/throughput comparisons are still pending.

### 🔍 Index: SIMD-accelerated BCF53 Breadcrumb Filter

The repository contains a Rust implementation of the BCF53 Breadcrumb Filter.

- **Runtime SIMD dispatch**: Detects the best available ISA at startup — AVX-512BW → AVX2 → SVE2 → SVE → NEON → scalar fallback.
- **Compact bit packing**: 53 mini-buckets, 8-bit tags — 51 single-choice + 35 two-choice entries per cacheline.
- **Performance status**: no publication-ready cross-system performance claim
  is made for the current preview.

### 📡 Transport: QUIC and TLS-over-TCP

- **Connection-oriented**: QUIC multiplexes hundreds of concurrent streams;
  TLS-over-TCP uses one ordered lane per connection.
- **TLS 1.3 baked in**: Every connection is encrypted by default.
- **Pluggable backends**: Choose between Quinn, Noq, or Quiche — swap with a feature flag.
- **Strict hybrid key exchange**: Conforming profiles require
  `X25519MLKEM768`; plaintext and classical-only fallback are rejected.

The maintained server listener exposes both profiles: QUIC over UDP and
TLS-over-TCP with one lane per TLS connection. By default the TCP listener
reuses the QUIC bind address (TCP and UDP may share a port); set
`[tcp].listen` when the profiles need separate addresses. See the
[transport security profile](docs/transport-security.md) for close handling,
bounded reads, backend conformance, and deployment limitations.

The server's network executor is selected at compile time. The four network
runtime features are equal first-class options; `network-runtime-compio` is the
default for compatibility with the standard build.

| Network runtime feature | QUIC backends |
| --- | --- |
| `network-runtime-compio` | `quic-noq`, `quic-quinn`, `quic-quiche` |
| `network-runtime-monoio` | `quic-quiche` |
| `network-runtime-glommio` | `quic-quiche` |
| `network-runtime-kimojio` | `quic-quiche` |

Enable exactly one `network-runtime-*` feature. `quiche` is runtime-neutral:
its UDP, timer, and task integration uses the network-runtime adapter.
`quinn` and `noq` retain their Compio-native stream implementations and
therefore require `network-runtime-compio`. The existing `quic.backend`
configuration (or `--quic-backend` selection) remains unchanged.

Network and storage workers share a combined worker and its io_uring ring only
when the selected runtime names match and both runtimes support combined roles.
Compio, Monoio, and Kimojio can share a worker when the same runtime is selected
for both roles. Simulated storage uses its own runtime name, so overlapping
network/storage CPUs are rejected rather than shared.

### 🔒 End-to-end encryption

Secure clients may compress values and encrypt them with the v1
AES-256-SIV-CMAC or AES-256-GCM-SIV profiles before transmission. The
authenticated value is bound to its exact wire Item ID bytes and namespace,
so moving ciphertext to another cache item or namespace fails authentication.
Protected clients hide typed application keys behind namespace-bound BLAKE3
item IDs. Mapped `NamespaceHash` operations derive 32-byte Item IDs, while
Exact operations preserve caller-supplied `0..=32`-byte IDs. The server observes
deterministic item IDs and encoded sizes, but not application keys or value
plaintext. The complete threat model and protection matrix are documented in
[`SECURITY_MODEL.md`](SECURITY_MODEL.md).

Maintained Gate 0 clients use the frozen `StructuredValue-CBOR-v1` value
profile. Every successful mapped `get` and `set` uses selector `0x10`
(`Unprotected | Uncompressed | StructuredValue-CBOR-v1`), preserving the
lossless value model across bindings. Gate 0 does not expose caller-selected
compression or value protection; see [`clients/VALUE_FORMAT.md`](clients/VALUE_FORMAT.md)
for the complete envelope and future-profile rules.

### 📚 Multi-language SDKs

Registry packages are available for
[Rust](https://crates.io/crates/openkache),
[TypeScript and JavaScript](https://www.npmjs.com/package/openkache), and
[Python](https://pypi.org/project/openkache/). They all provide the same
basic workflow: connect, get, set, delete, and close. Choose the package for
your language; each client README contains installation options, a complete
example, and a public API reference. The [client package
index](./clients/README.md) lists compatibility adapters and language
scaffolds separately.

### 📦 Single binary distribution

One statically linked binary. No shared libraries, no runtime dependencies, no package manager required. Copy it anywhere and run.

---

## 🚀 Quick start

```bash
# Start the local QUIC and TLS-over-TCP preview server
cargo run --manifest-path server/Cargo.toml --bin openkache-server
```

The server listens on `127.0.0.1:4433`, stores shard files under
`target/kvkache-v1`, and writes an ephemeral self-signed certificate to
`target/openkache-local/certificate.local.der`. By default it supports
`PING`, `GET`, `SET`, and `DELETE` over the versioned `openkache/1` QUIC and
TLS-over-TCP transport profiles. `SET` accepts an optional millisecond TTL and
atomic `if_absent` or `if_present` existence condition. Expired values are
treated as absent immediately, while their SSD space is reclaimed when the
containing Segment Group is reused. `EXPERIMENTAL_STATS` and
`EXPERIMENTAL_SYNC` are experimental administrative operations and are
disabled by default. To enable them, set
both `enable_experimental_api = true` and
`experimental_api_revision = "draft-2026-08-19.4"` in the server
configuration, then coordinate that exact revision with the client; the
revision is not negotiated on the wire. When enabled, `EXPERIMENTAL_SYNC`
flushes each SSD worker for the current process. Clean shutdown checkpoint replay is supported,
but crash recovery and broader durability guarantees remain outside the current
preview. Pass `--port <port>` only when overriding the default port, or pass
`--config <path>` to load an explicit TOML cache configuration.
The complete byte-level contract is the
[wire protocol v1 specification](./protocol/SPEC.md).

The default loopback endpoint accepts unauthenticated clients and grants them
administrative commands for local development. `--insecure-development` is
required to extend that behavior to an explicitly selected non-loopback
address. Production non-loopback startup requires a stable server certificate
and private key, a client CA for mTLS, and an administrator certificate
allowlist. See the [production TLS guide](#production-tls).

### Try a client

The examples in the client READMEs use the local development TLS profile. It
does not verify the server certificate, so use it only with a local development
server; do not reuse this trust profile for production traffic.

| Package | Install | Documentation | Source |
|---|---|---|---|
| TypeScript / JavaScript | `npm install openkache` | [npm](https://www.npmjs.com/package/openkache) · [client README](clients/typescript/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/typescript) |
| Python | `python -m pip install openkache` | [PyPI](https://pypi.org/project/openkache/) · [client README](clients/python/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/python) |
| Rust | `cargo add openkache` | [crates.io](https://crates.io/crates/openkache) · [docs.rs](https://docs.rs/openkache/latest/openkache/) · [client README](clients/rust/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/rust) |

All three client guides use `127.0.0.1:4433` as the default local endpoint.
They also list alternative package managers and the complete public API for
their language.

### Container image

The server is published for Linux `amd64` and `arm64` as
`ghcr.io/openkache/openkache`. The image runs as a non-root user, keeps cache
data in `/var/lib/openkache`, and requires a mounted PKI bundle for its
production default command. See the [container image guide](./docs/container-image.md)
for secure mTLS deployment and the explicit isolated-development command.

The server runtime supports Linux `x86_64`/`aarch64` and Apple Silicon macOS.
Linux startup requires the selected native network runtime to provide `io_uring`
and access to the `io_uring_setup`, `io_uring_enter`, and `io_uring_register`
syscalls; unsupported targets fail with an explicit platform error. NVMe SSD is
the intended storage medium but is not mandatory. After storage workers open
their data files, startup warns when any opened file is on known non-NVMe
storage or its device cannot be identified.

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
`admin_client_certificates` may run `EXPERIMENTAL_STATS` or `EXPERIMENTAL_SYNC`.

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
detects standard Linux cgroup limits and current usage or macOS host memory
pressure plus filesystem availability, but does not detect filesystem quotas,
SSD type, or NVMe performance. At runtime, workers reserve each Segment
generation immediately before writing it instead of preallocating the whole
sparse file. Memory or storage pressure temporarily rejects `SET` with an
overloaded response while reads, deletes, and recovery remain available.
`EXPERIMENTAL_STATS` reports
the memory and storage stop/resume thresholds, `memory_stop_writes`,
`storage_stop_writes`, and `rejected_writes`.

`--cpus` selects worker threads but does not impose a process CPU quota; use
deployment affinity or cgroups for that boundary. `light` and `balanced` use
1 MiB Blob Segments, so one encoded item cannot exceed 1 MiB; `heavy` uses
64 MiB Blob Segments but caps one encoded item at 16 MiB. Reopen existing
storage with the same automatically detected layout or explicit sizing
overrides. Format-v1 permits a supported increase in bucket choice count;
worker/Segment geometry changes and unsupported choice-count changes require
cache recreation.

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
use 1 MiB Blob Segments, so one encoded item cannot exceed 1 MiB; `heavy` uses
64 MiB Blob Segments but caps one encoded item at 16 MiB. Reuse the same sizing
arguments when reopening existing storage. Format-v1 permits a supported
increase in bucket choice count; worker/Segment geometry changes and
unsupported choice-count changes require cache recreation.

---

## 🏗️ Architecture

```
┌──────────────┐   QUIC (UDP) / TLS-over-TCP (TLS 1.3)   ┌──────────────────┐
│  openkache-  │ ──────────────────────────────────▶ │     OpenKache        │
│  client      │   Multiplexed streams / one lane    │  (single binary)     │
│ (Rust/TS/.NET)│                                    │                      │
└──────────────┘                                     │  ┌────────────────┐  │
                                                     │  │ BCF53 Filter   │  │
┌──────────────┐                                     │  │ (SIMD, AVX2)   │  │
│  Any v1      │                                     │  ├────────────────┤  │
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

There are no current, publication-ready cross-system performance results.
Archived or diagnostic measurements must not be read as current throughput or
latency guarantees.

| | Redis | OpenKache |
|---|---|---|
| **Cost per GB** | DRAM (~$3–5/GB) | SSD (~$0.05–0.10/GB) |
| **P99 latency** | varies by workload | not published |
| **Throughput** | varies by workload | not published |
| **Transport** | TCP (head-of-line blocking) | QUIC or TLS-over-TCP (TLS 1.3) |
| **Security** | TLS optional, no client-side value protection | TLS 1.3 transport by default; client-side value protection is optional |

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

For the canonical locked release build of only the server binary, use the
shared Cargo alias:

```bash
cargo server-build
```

The container builder invokes the same alias and adds its target triple:

```bash
cargo server-build --target x86_64-unknown-linux-musl
```

### Server allocator

The server uses jemalloc by default. Select the system allocator, mimalloc, or
snmalloc by disabling default features and enabling exactly one allocator
feature:

```bash
cargo server-build \
  --no-default-features \
  --features allocator-mimalloc,channel-crossfire,network-runtime-compio,quic-noq,storage-runtime-compio
```

The supported choices are `allocator-jemalloc` (the default),
`allocator-system`, `allocator-mimalloc`, and `allocator-snmalloc`. The
features are mutually exclusive, and a server binary must select exactly one.
The server reports the selected allocator during startup.

### Server channel

The server uses Kanal by default for worker, request/reply, and transport
channels. Select exactly one of `channel-kanal`, `channel-crossfire`, or
`channel-flume` at compile time:

```bash
cargo server-build \
  --no-default-features \
  --features allocator-system,channel-flume,network-runtime-compio,quic-noq,storage-runtime-compio
```

### Static musl (x86_64 / aarch64)

```bash
cargo zigbuild --target x86_64-unknown-linux-musl
cargo zigbuild --target aarch64-unknown-linux-musl
```

### Both architectures

```bash
cargo server-build --target x86_64-unknown-linux-musl
cargo server-build --target aarch64-unknown-linux-musl
```

### Container image

Build the server image from the public repository root with Docker:

```bash
docker build --file server/Dockerfile --tag localhost/openkache:dev .
```

Podman is also supported with the same Dockerfile and build context:

```bash
podman build --format docker --file server/Dockerfile --tag localhost/openkache:dev .
```

The CI workflow builds and publishes the multi-architecture image to GHCR.
Container builds run the same locked Smithy/Bun protocol generation as source
builds; see the [container image guide](./docs/container-image.md) for image
tags, storage, and deployment commands.

---

## ✅ Verify the build

```bash
cargo check --locked
```

---

## 📊 Project status

OpenKache is in **active development**. Core components are stable, the server
protocol layer is being built out, and client packages are available in the
languages listed below; package status details live in
[`clients/README.md`](./clients/README.md).

| Component | Status | Notes |
|---|---|---|
| Memory allocators | ✅ Stable | VirtualPageStack + CompactingSlabAllocator in production shape |
| Breadcrumb filter | ✅ Stable | BCF53 with runtime SIMD dispatch |
| Rust client | 🚧 Preview | Async client package |
| Command-line client | 🚧 Preview | `openkache-cli` for Bash scripts and interactive shell use |
| TypeScript / JavaScript client | 🚧 Preview | Node.js, Bun, and Deno package |
| Python client | 🚧 Preview | Python package |
| Go client | 🚧 Preview | Context-aware shared-core native ABI binding |
| C client | 🚧 Preview | C17 shared-core ABI |
| C++ client | 🚧 Preview | C++20 adapter over the C ABI |
| Swift client | 🚧 Preview | Async shared-core SDK with generated Smithy operations |
| QUIC/TLS-over-TCP server | 🚧 Preview | Current protocol preview; target SSD-backed worker shards and restart recovery |
| Container image | ✅ Available | Non-root `linux/amd64` and `linux/arm64` image on GHCR |
| QUIC client (.NET) | 🚧 Preview | Shared-core C ABI adapter, raw Smithy API, binary protocol v1 |
| Clustering | ❌ Not started | Future: consistent hashing, gossip, replication |

---

## 🗺️ Roadmap

| Milestone | Target | Focus |
|---|---|---|
| Core engine | ✅ Done | Allocators, BCF53 filter, types, and client foundations |
| Server protocol | 🚧 In progress | Recovery, operational hardening, and stable configuration |
| Production hardening | 🔜 Next | Benchmarks, fuzzing, CI/CD, musl release artifacts, and capacity guidance |
| Value protection profiles | 🚧 Future | AES-256-SIV-CMAC/AES-256-GCM-SIV profiles beyond the unprotected Gate 0 profile |
| Clustering | 📅 Future | Consistent hashing, gossip protocol, replication, failover |
| General availability | 🎯 Future | Stable API, cross-platform packages, production docs |

---

## 🤝 Community and contributing

OpenKache is a systems project focused on making caching less expensive and
easier to operate. There is useful work at every level, from clear bug reports
and documentation to changes that improve reliability and ease of operation.
If you like systems work where a small change can matter, start with the
[roadmap](./README.md#-roadmap) or
[open issues](https://github.com/openkache/openkache/issues).

- [Contributing](./CONTRIBUTING.md) — how to propose, check, and review changes.
- [Community Guidelines](./COMMUNITY_GUIDELINES.md) — how we work together and
  raise concerns.
- [Code of Conduct](./CODE_OF_CONDUCT.md) — the project’s conduct and reporting
  policy.

---

## 🤖 AI coding agents

OpenKache provides [`/llms.txt`](./llms.txt) and [`/llms-full.txt`](./llms-full.txt) for LLM-friendly documentation.

---

## 📁 Project structure

| Path | Contents |
|---|---|
| `protocol/` | Shared binary request, response, opcode, and status definitions |
| `server/` | SSD cache engine plus the runnable QUIC/TLS-over-TCP server |
| `clients/` | Implemented SDKs and thin-binding package scaffolds |
| `clients/core/` | Low-level QUIC client core shared by language adapters |
| `clients/rust/` | Ergonomic Rust end-user SDK over the client core |
| `clients/cli/` | Bash-friendly one-shot and interactive CLI binary |
| `clients/typescript/` | Node.js, Bun, and Deno client backed by Rust through Node-API |
| `clients/dotnet/` | Managed .NET raw Smithy adapter over the shared core C ABI |


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
