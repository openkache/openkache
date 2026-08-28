# OpenKache roadmap

OpenKache is an SSD-first cache server: it treats the SSD as the primary
capacity tier instead of a swap target for an in-memory index. This roadmap
describes what exists today, what is actively being built, and the direction
toward a production-ready release.

Status legend: ✅ done · 🚧 in progress · 🔜 next · 📅 planned

## Where we are today (preview)

The current preview runs one server process that speaks Redis-compatible
`GET`/`SET`/`DEL` over RESP/TCP and the native OpenKache Gate 0 protocol over
QUIC/UDP. On Linux it uses an `io_uring` network frontend and a direct-I/O
storage path; writes from many keys are aggregated into sequential
segment-group writes to the SSD.

| Area | Status | Notes |
|---|---|---|
| SSD storage engine | 🚧 | Segment-group write aggregation, direct I/O on Linux |
| RESP/TCP frontend | 🚧 | Redis-compatible `GET`/`SET`/`DEL` |
| OpenKache/QUIC Gate 0 | 🚧 | TLS 1.3 over QUIC; local-development trust profile |
| Client SDKs | 🚧 | Rust, TypeScript, Python from one generated contract |
| Single-binary deploy | ✅ | Static musl binary and Linux amd64/arm64 container |

## Next: production hardening 🔜

The nearest milestone turns the preview into something an operator can trust
with real data.

- **Restart recovery.** Persist and reload the cache across restarts instead of
  truncating `openkache.data` on every start.
- **Reproducible benchmarks.** A committed harness that anyone can run to
  reproduce the throughput and latency numbers, plus mixed GET/SET workloads.
- **Operational hardening.** Fuzzing on the RESP and wire parsers, CI/CD that
  gates every merge, and capacity-sizing guidance for a given SSD.

## Then: security and correctness 📅

- **Production authentication.** Client authentication and mutual TLS beyond the
  local-development Gate 0 profile, with a real certificate trust chain.
- **Value protection profiles.** Client-side value protection
  (AES-256-GCM-SIV / AES-256-SIV-CMAC) layered on top of transport TLS.
- **Full protocol surface.** TTL overrides, conditional writes, namespace
  administration, and statistics beyond the current Gate 0 operation subset.

## Later: scale and reach 📅

- **Clustering.** Consistent hashing, membership/gossip, replication, and
  failover so a deployment can outgrow a single node.
- **Cross-platform servers.** A portable macOS build for development and a
  Windows/WSL path, so contributors are not limited to Linux. Peak throughput
  stays a Linux `io_uring` story; other platforms target correctness and
  developer ergonomics.
- **General availability.** A stable API, published packages across languages,
  and production operations documentation.

## How the pieces fit

The generated protocol contract means a new operation is defined once and flows
out to every language client, so protocol work and client work move together
rather than drifting. See [docs/architecture.md](./docs/architecture.md) for
how the network, storage, and protocol layers are separated, and
[CONTRIBUTING.md](./CONTRIBUTING.md) for how to pick up a piece of this roadmap.
