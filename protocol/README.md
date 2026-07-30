# OpenKache Protocol

`openkache-protocol` defines the shared binary wire format used by OpenKache
QUIC clients and servers.

## Purpose

The crate keeps opcode, status, framing, and size validation in one place so
clients and servers cannot silently drift onto incompatible formats.

The normative wire contract is [SPEC.md](./SPEC.md). Implementation status and
follow-up work are recorded in [HANDOVER.md](./HANDOVER.md).

## Commands

From this directory:

```bash
cargo build
cargo check
cargo fmt --check
```

## Wire format

Protocol v3 uses the QUIC ALPN identifier `openkache/3`. The ALPN selects the
wire version for the connection; frames do not repeat a version byte. Each
bidirectional stream is a reusable sequential lane with one request in flight.

```text
request  = opcode:u8 | flags:u8 | key_len:vu128 | value_len:vu128 |
           item_key | [ttl_ms:vu128] | value
response = status:u8 | payload_len:vu128 | payload
```

Lengths and TTLs use canonical shortest unsigned `vu128`. Item keys are exact
32-byte identifiers, values are opaque, and values and response payloads have
a 64 MiB wire ceiling. The TTL appears after the key and before the value so a
server can validate server-owned metadata before reading a large body.

See the [wire protocol specification](./SPEC.md) for the byte-level encoding,
operation semantics, status registry, validation rules, error recovery, and
conformance vectors.

## Core components

- `Opcode` and `Status` define stable wire identifiers.
- `Request` and `Response` validate and encode complete stream frames.
- Incremental request and response header decoders report when enough prefix
  bytes are available to determine the complete frame length.
- `ProtocolError` reports malformed, unsupported, and oversized frames.
- `SPEC.md` defines the normative protocol independent of the Rust API.

## Configuration

The v3 limits and ALPN identifier are compile-time constants. There are no
environment variables or runtime configuration files.
