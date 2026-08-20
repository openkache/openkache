# FAQ

## General

### What is OpenKache?

An open-source, SSD-first cache server. It is designed to use NVMe SSD as
primary storage instead of DRAM. Current cost, latency, and throughput
comparisons are not publication-ready; the maintained server exposes QUIC and
TLS-over-TCP transport profiles.

### How is it different from Redis?

| | Redis | OpenKache |
|---|---|---|
| **Cost per GB** | DRAM (~$3–5/GB) | SSD (~$0.05–0.10/GB) |
| **Transport** | TCP (HOL blocking) | QUIC or TLS-over-TCP (TLS 1.3) |
| **Security** | TLS optional, no client-side value protection | TLS 1.3 transport by default; client-side value protection is optional |
| **Index** | Hash table | BCF53 SIMD filter |

### Is it production-ready?

Core components are under active development. The public QUIC and TLS-over-TCP
server listeners are a preview; the target SSD-backed worker and
restart-recovery contract, plus production hardening, remain in progress.

## SSD

### Does OpenKache wear out SSDs?

The log-structured design is intended to bound write amplification, but device
endurance depends on workload and has not been established by a current
publication-ready report.

### What about latency?

OpenKache has a sub-millisecond latency target, but no current publication-ready
P99 result is being claimed. SIMD-accelerated indexing and QUIC multiplexing are
implementation mechanisms rather than measured guarantees.

## Usage

### How do I install?

See [Getting Started](getting-started.md). Build from source with `cargo build --release`.

### What client languages are available?

Rust, TypeScript and JavaScript on Node.js, Bun, and Deno, .NET, Python, Go,
C, C++, Swift, and the `openkache-cli` command-line client are available.
Java, Kotlin, and Dart currently remain package scaffolds.

### Is end-to-end encryption mandatory?

No. TLS 1.3 protects the transport, while client-side value protection is
optional. Maintained clients derive namespace-bound Item IDs with keyed BLAKE3.
When a client supplies a persistent random 32-byte data-protection key, values
use AES-256-GCM-SIV by default or the explicit AES-SIV-CMAC profile. When the
key is omitted, formatted values are stored unprotected; the Item ID mapping
still applies. The low-level raw API accepts exact Item IDs and encoded values
for callers that intentionally own those transformations.

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
request/reply, and transport channels. `channel-kanal` is enabled by default;
replace it with exactly one of `channel-crossfire` or `channel-flume` in a
`--no-default-features` build. This is a build choice and does not add a runtime
configuration field.
