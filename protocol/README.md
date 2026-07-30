# OpenKache protocol

`openkache-protocol` is the Rust implementation of the binary contract shared
by OpenKache protocol v3 clients and servers.

## Purpose

The crate provides validated request and response types so implementations do
not duplicate framing, opcode, status, or size checks.

The [wire protocol specification](SPEC.md) is the sole normative source for
transport negotiation, frame bytes, operation semantics, limits, malformed
input handling, and retry ambiguity. This README owns only crate usage,
implementation structure, and project status.

## Commands

From `protocol`:

```bash
cargo build
cargo check
cargo fmt --check
```

## Usage

Construct or decode complete frames through validated types:

```rust
use openkache_protocol::{Opcode, Request, Response};

let request = Request::new(Opcode::Ping, None, Vec::new())?;
let request_bytes = request.encode()?;

let response_bytes = [0x00, 0x04, b'P', b'O', b'N', b'G'];
let response = Response::decode(&response_bytes)?;
```

Incremental transports can use `Request::decode_header`,
`Request::frame_len`, `Response::decode_header`, and `Response::frame_len` to
determine how many bytes a complete frame requires without duplicating
variable-integer parsing.

## Core components

- `Opcode`, `Status`, and `SetOptions` represent assigned protocol values.
- `Request` and `Response` validate and encode complete frames.
- `RequestHeader` and `ResponseHeader` support bounded incremental reads.
- `ProtocolError` classifies malformed, unsupported, and oversized frames.
- `SPEC.md` defines the implementation-independent contract.

## Implementation status

The shared Rust client and server use protocol v3 through this crate.
Production durability guarantees for `SYNC` remain deployment and storage
policy; protocol v3 specifies only when a successful response may be sent.

## Configuration

Protocol identifiers and wire ceilings are compile-time constants. The crate
has no environment variables or runtime configuration files.
