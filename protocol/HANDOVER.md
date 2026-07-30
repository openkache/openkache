# OpenKache Protocol Handover

Normative wire requirements live in [SPEC.md](./SPEC.md). This handover records
implementation status, architectural context, and follow-up work.

## Current status

Protocol v3 defines binary frames for `PING`, `GET`, `SET`, `DELETE`, `STATS`,
and `SYNC`. `SET` supports an optional millisecond TTL and the atomic
`if_absent` and `if_present` existence conditions. The Rust client and server
share these types directly.

## Architecture

Each QUIC bidirectional stream is a reusable lane with one request in flight.
Canonical unsigned `vu128` fields keep small lengths compact, while explicit
status codes keep cache misses separate from protocol or server errors.

Every request starts with an opcode and an operation-specific flags byte,
followed by canonical `key_len` and `value_len` fields. `SET` flag bits encode
TTL presence, `if_absent`, and `if_present`. When the TTL bit is present, an
unsigned `vu128` relative TTL in milliseconds
appears after the exact 32-byte item key and before the value. A zero TTL is
invalid.
`if_absent` stores only when the key is absent, while `if_present` stores only
when the key is present. An expired key is absent for both conditions.

Values are opaque. Wire framing and SSD metadata contain no compression or
application-encryption flags. A client that transforms values owns an envelope
inside its value bytes.

## Known limitations

- Requests do not carry IDs because QUIC stream identity provides correlation.
- TTL precision is milliseconds; expiry is lazy, so expired SSD bytes are
  reclaimed when their Segment Group is reused.
- The version is negotiated only through `openkache/3` ALPN; frames carry no
  redundant version field.

## Next steps

1. Define production durability guarantees for `SYNC`.
