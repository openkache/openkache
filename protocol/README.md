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
are limited to 16 MiB. `SET` uses request length flag bits for an optional
positive millisecond TTL and the mutually exclusive `NX` and `XX` conditions.
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
