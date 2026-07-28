# OpenKache Protocol Handover

## Current status

Protocol v2 defines binary frames for `PING`, `GET`, `SET`, `DELETE`, `STATS`,
and `SYNC`. The Rust client and the in-memory QUIC smoke server share these
types directly.

## Architecture

Each QUIC bidirectional stream carries exactly one request and one response.
Fixed-width big-endian lengths make framing deterministic, while explicit
status codes keep cache misses separate from protocol or server errors.

## Known limitations

- `SET` has no expiry or TTL field.
- Requests do not carry IDs because QUIC stream identity provides correlation.
- `SYNC` semantics depend on the storage backend; the memory backend treats it
  as an acknowledged no-op.
- There is no protocol negotiation beyond the `openkache/2` ALPN identifier.

## Next steps

1. Specify expiry semantics before adding TTL to `SET`.
2. Define production durability guarantees for `SYNC`.
3. Add version-negotiation guidance before introducing protocol v2.
