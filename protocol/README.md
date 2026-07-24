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

Protocol v1 uses the QUIC ALPN identifier `openkache/1`. Each bidirectional
stream carries one request and one response.

```text
request  = opcode:u8 | key_len:u32be | value_len:u32be | key | value
response = status:u8 | payload_len:u32be | payload
```

Supported operations are `PING`, `GET`, `SET`, `DELETE`, `STATS`, and `SYNC`.
Keys are limited to 65,535 bytes and values or response payloads to 16 MiB.

## Core components

- `Opcode` and `Status` define stable wire identifiers.
- `Request` and `Response` validate and encode complete stream frames.
- `ProtocolError` reports malformed, unsupported, and oversized frames.

## Configuration

The v1 limits and ALPN identifier are compile-time constants. There are no
environment variables or runtime configuration files.
