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

Protocol v3 uses the QUIC ALPN identifier `openkache/3`. The ALPN selects the
wire version for the connection; frames do not repeat a version byte. Each bidirectional
stream is a reusable sequential lane carrying any number of request/response
pairs. A lane has at most one in-flight request, so responses need no request
identifier.

```text
request  = opcode:u8 | flags:u8 | key_len:vu128 | value_len:vu128 |
           item_key | [ttl_ms:vu128] | value
response = status:u8 | payload_len:vu128 | payload
```

Supported operations are `PING`, `GET`, `SET`, `DELETE`, `STATS`, and `SYNC`.
KV keys are exact 32-byte opaque item keys. A high-level client may derive
them from arbitrary application keys with SHA-256 or keyed HMAC-SHA-256. The
server rejects every other key length. `PING`, `STATS`, and `SYNC` carry no key.
Values and response payloads are limited to 64 MiB. Servers may enforce a
smaller operational item limit.
For `SET`, flag bit 0 indicates an optional positive millisecond TTL, bit 1 is
`if_absent`, and bit 2 is `if_present`. Bits 3 through 7 are reserved and must
be zero. `if_absent` and `if_present` are mutually exclusive. Flags must be
zero for every other opcode.
`STATS` and `SYNC` return `Forbidden` when the authenticated client lacks
administrator authorization.
The TTL appears after the key and before the value so a server can validate
all server-owned metadata before reading a large opaque value.

Lengths and TTLs use the unsigned [`vu128`](https://github.com/jmillikin/rust-vu128)
format. Protocol v3 accepts only the canonical shortest encoding and only
values representable as `u64`; overlong and wider encodings are invalid.
Compression, application-level encryption, serialization, and other value
metadata are not part of the wire protocol. The server stores value bytes
without interpreting them.

## Core components

- `Opcode` and `Status` define stable wire identifiers.
- `Request` and `Response` validate and encode complete stream frames.
- Incremental request and response header decoders report when enough prefix
  bytes are available to determine the complete frame length.
- `ProtocolError` reports malformed, unsupported, and oversized frames.

## Configuration

The v3 limits and ALPN identifier are compile-time constants. There are no
environment variables or runtime configuration files.
