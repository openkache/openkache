# OpenKache Protocol Handover

## Current status

Protocol v2 defines binary frames for `PING`, `GET`, `SET`, `DELETE`, `STATS`,
and `SYNC`. `SET` supports an optional millisecond TTL and the atomic
`if_absent` and `if_present` existence conditions. The Rust client and server
share these types directly.

## Architecture

Each QUIC bidirectional stream carries exactly one request and one response.
Fixed-width big-endian lengths make framing deterministic, while explicit
status codes keep cache misses separate from protocol or server errors.

`SET` encodes its options in the high bits of the request value-length word.
When the TTL bit is present, an unsigned 8-byte relative TTL in milliseconds
appears after the key digest and before the value. A zero TTL is invalid.
`if_absent` stores only when the key is absent, while `if_present` stores only
when the key is present. An expired key is absent for both conditions.

The SSD item format is version 2. Persistent items keep their existing layout;
expiring items use a distinct kind and store an 8-byte absolute Unix
millisecond deadline immediately before the stored value. Version 1 cache files
must be repopulated.

## Known limitations

- Requests do not carry IDs because QUIC stream identity provides correlation.
- TTL precision is milliseconds; expiry is lazy, so expired SSD bytes are
  reclaimed when their Segment Group is reused.
- There is no protocol negotiation beyond the `openkache/2` ALPN identifier.

## Next steps

1. Define production durability guarantees for `SYNC`.
2. Add version-negotiation guidance before introducing a future protocol
   version.
