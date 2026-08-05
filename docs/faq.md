# FAQ

## General

### What is OpenKache?

An open-source, SSD-first cache server. It uses NVMe SSD as primary storage instead of DRAM, making it 10× cheaper than Redis while delivering competitive latency through SIMD-accelerated indexing and QUIC transport.

### How is it different from Redis?

| | Redis | OpenKache |
|---|---|---|
| **Cost per GB** | DRAM (~$3–5/GB) | SSD (~$0.05–0.10/GB) |
| **Transport** | TCP (HOL blocking) | QUIC (TLS 1.3, multiplexed) |
| **Security** | TLS optional | E2E encrypted by default |
| **Index** | Hash table | BCF53 SIMD filter |

### Is it production-ready?

Core components are under active development. The QUIC server dispatches requests to the SSD-backed worker runtime; recovery and production hardening remain in progress.

## SSD

### Does OpenKache wear out SSDs?

Write amplification is minimal due to log-structured design. Modern NVMe SSDs are rated for 1 PBW. A typical cache workload writes under 100 TB/year, giving over 10 years of life.

### What about latency?

OpenKache targets <1ms p99, competitive with Redis. SIMD-accelerated indexing and QUIC multiplexing offset the SSD latency gap.

## Usage

### How do I install?

See [Getting Started](getting-started.md). Build from source with `cargo build --release`.

### What client languages are available?

Rust, TypeScript and JavaScript on Node.js, Bun, and Deno, .NET, and the
`openkache-cli` command-line client are available.

### Is end-to-end encryption mandatory?

High-level Rust and TypeScript clients require a random 32-byte data protection
master key. The shared client core derives HMAC-SHA-256 item IDs and an
independent XChaCha20-Poly1305 value key. The server sees deterministic 32-byte
item IDs and encoded value sizes, but not application keys or value plaintext.
The low-level raw API accepts exact item IDs and encoded values for callers
that intentionally own those transformations.

### Can I use my own QUIC implementation?

The server has a backend-independent connection and stream boundary. The
default `noq` backend and the optional `quinn` backend retain their Compio-native
UDP I/O and timers, so they require `network-runtime-compio`. The `quiche`
backend uses the runtime-neutral network adapter and can run with Compio,
Monoio, Glommio, or Kimojio. Select a backend with `--quic-backend` or
`[quic].backend`; enable it with `quic-noq`, `quic-quinn`, or `quic-quiche`.
A build with exactly one backend selects it automatically, while a build with
multiple backends requires an explicit selection. Mozilla neqo is not currently
available: its official transport is not published as a standalone crate and
its server API requires NSS certificate-database integration.

### Can I select a different in-process channel implementation?

Yes. The server uses one compile-time channel backend for its worker,
request/reply, and transport channels. `channel-crossfire` is enabled by
default; replace it with exactly one of `channel-flume` or `channel-kanal` in a
`--no-default-features` build. This is a build choice and does not add a runtime
configuration field.
