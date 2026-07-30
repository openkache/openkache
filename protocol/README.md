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
           [key[32]] | [ttl_ms:u64be] | value
response = status:u8 | payload_len_and_flags:u32be | payload
```

Supported operations are `PING`, `GET`, `SET`, `DELETE`, `STATS`, and `SYNC`.
KV keys are exact 32-byte opaque item keys. A high-level client may derive
them from arbitrary application keys with SHA-256 or keyed HMAC-SHA-256. The
server rejects every other key length. `PING`, `STATS`, and `SYNC` carry no key.
Values and response payloads are limited to 64 MiB. Servers may enforce a smaller
operational item limit.
`SET` uses request length flag bits for an optional positive millisecond TTL
and the mutually exclusive `if_absent` and `if_present` conditions.
`STATS` and `SYNC` return `Forbidden` when the authenticated client lacks
administrator authorization.
The 8-byte relative TTL appears immediately before the value when present.

## Core components

- `Opcode` and `Status` define stable wire identifiers.
- `Request` and `Response` validate and encode complete stream frames.
- Fixed request and response headers report the next complete frame length for
  persistent-lane readers.
- `ProtocolError` reports malformed, unsupported, and oversized frames.

## Configuration

The v2 limits and ALPN identifier are compile-time constants. There are no
environment variables or runtime configuration files.
