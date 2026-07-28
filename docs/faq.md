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

Rust, TypeScript on Bun, and .NET SDKs are available.

### Is end-to-end encryption mandatory?

The TypeScript client requires a 32-byte XChaCha20-Poly1305 key. Rust callers
can select the same secure codec or use the backwards-compatible plaintext
mode. Cache keys are sent as SHA-256 digests. With the secure codec, the server
sees deterministic key digests and encrypted value sizes, but not value
plaintext.

### Can I use my own QUIC implementation?

The server has a backend-independent connection and stream boundary. The
default `noq` backend and the optional `quinn` and `quiche` backends use the
same Compio UDP I/O and timers and are selected with `--quic-backend` or
`[quic].backend`. Enable them with the `quic-noq`, `quic-quinn`, and
`quic-quiche` Cargo features; at least one is required. Mozilla neqo is not
currently available: its official transport is not published as a standalone
crate and its server API requires NSS certificate-database integration.
