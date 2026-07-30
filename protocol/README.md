# OpenKache Protocol

`openkache-protocol` defines the shared binary wire format used by OpenKache
QUIC clients and servers.

## Purpose

The crate keeps opcode, status, framing, and size validation in one place so
clients and servers cannot silently drift onto incompatible formats.

## Commands

From this directory:

```bash
cargo build
cargo check
cargo fmt --check
```

## Wire format

Protocol v2 uses the QUIC ALPN identifier `openkache/2`. Each bidirectional
stream is a reusable sequential lane carrying any number of request/response
pairs. A lane has at most one in-flight request, so responses need no request
identifier.

```text
request  = opcode:u8 | key_len:u32be | value_len_and_flags:u32be |
           client_key_digest | [ttl_ms:u64be] | value
response = status:u8 | payload_len:u32be | payload
```

Supported operations are `PING`, `GET`, `SET`, `DELETE`, `STATS`, and `SYNC`.
Clients encode KV keys as the 32-byte SHA-256 digest of the exact user-key
bytes. The server rejects every other key length. Values and response payloads
are limited to 64 MiB. Servers may enforce a smaller operational item limit.
`SET` uses request length flag bits for an optional positive millisecond TTL
and the mutually exclusive `if_absent` and `if_present` conditions.
`STATS` and `SYNC` return `Forbidden` when the authenticated client lacks
administrator authorization.
The 8-byte relative TTL appears immediately before the value when present.

### Compact-length primitive

The crate also defines a canonical byte-aligned integer encoding for lengths in
the inclusive range `0..=4096`. It is available for future wire protocols and
storage formats; protocol v2 continues to use the fixed headers above.

Let `x` be the unsigned length:

```text
0 <= x < 240:
    octet[0] = x

240 <= x <= 4096:
    y = x - 240
    octet[0] = 0xf0 | (y >> 8)
    octet[1] = y & 0xff
```

The first-byte range `0x00..=0xef` therefore contains complete one-byte values.
The range `0xf0..=0xff` is a two-byte tag whose low nibble and following byte
form a 12-bit big-endian offset from 240.

| Value | Canonical bytes |
|---:|:---|
| 0 | `00` |
| 239 | `ef` |
| 240 | `f0 00` |
| 255 | `f0 0f` |
| 256 | `f0 10` |
| 4095 | `ff 0f` |
| 4096 | `ff 10` |

Every value has exactly one representation. Decoders must reject an incomplete
two-byte value and encodings `ff 11` through `ff ff`, which resolve above the
4 KiB ceiling. Trailing bytes belong to the enclosing format and are not
consumed by this primitive.

This split maximizes the number of one-byte values in any prefix-free,
byte-aligned format limited to two bytes that covers `0..=4096`. If `s`
first-byte values are complete and the other `256 - s` values introduce a
second byte, capacity is `s + 256(256 - s)`; covering all 4097 values requires
`s <= 240`.

## Core components

- `Opcode` and `Status` define stable wire identifiers.
- `Request` and `Response` validate and encode complete stream frames.
- `encode_compact_length` and `decode_compact_length` implement the canonical
  4 KiB length primitive.
- Fixed request and response headers report the next complete frame length for
  persistent-lane readers.
- `ProtocolError` reports malformed, unsupported, and oversized frames.

## Configuration

The v2 limits and ALPN identifier are compile-time constants. There are no
environment variables or runtime configuration files.
