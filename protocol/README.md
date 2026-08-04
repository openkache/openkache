# OpenKache protocol

`openkache-protocol` is the Rust implementation of the binary contract shared
by OpenKache protocol v1 clients and servers.

## Purpose

The crate provides validated request and response types so implementations do
not duplicate framing, opcode, status, or size checks.

The [wire protocol specification](SPEC.md) defines transport negotiation, frame
bytes, operation semantics, limits, malformed input handling, and retry
ambiguity. This README covers crate usage, implementation structure, and
project status.

## Commands

From `protocol`:

```bash
./generate.ts                 # Generate the Rust wire contract
cargo build
cargo check
cargo fmt --check
```

Generation and Rust builds require Bun and Smithy CLI on `PATH`. Cargo invokes
the wire generator automatically before compiling the protocol crate. To
regenerate language-client artifacts, run `../clients/generate.ts` from this
directory or use `just generate-protocol-contract` from the repository root.

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

- `model/openkache.smithy` is the canonical source for server-visible wire
  values: operation and status assignments, shared limits, fixed widths, and
  version-specific frame and flag layout.
- `wire.ts` owns wire-model AST extraction and deterministic rendering of the
  Rust and language-neutral wire contract.
- `generate.ts` validates the wire Smithy model and emits only the Rust wire
  definitions used by this crate. The client generator consumes the same
  extracted wire contract for C, C#, Go, Python, Swift, and TypeScript
  constants; client defaults, API shapes, native ABI identifiers, and
  value-format metadata belong to
  [`../clients/model/openkache.smithy`](../clients/model/openkache.smithy) and
  [`../clients/generate.ts`](../clients/generate.ts).
- `Opcode`, `Status`, and `SetOptions` represent assigned protocol values.
- `Request` and `Response` validate and encode complete frames.
- `RequestHeader` and `ResponseHeader` support bounded incremental reads.
- `ProtocolError` classifies malformed, unsupported, and oversized frames.
- `SPEC.md` defines the implementation-independent contract.

## Implementation status

The shared Rust client and server use protocol v1 through this crate.
Production durability guarantees for `SYNC` remain deployment and storage
policy; protocol v1 specifies only when a successful response may be sent.

## Configuration

Protocol identifiers, operation assignments, fixed field widths, flag masks,
status boundaries, and wire ceilings are compile-time definitions sourced from
the wire Smithy model. The crate has no runtime configuration files. Change
these values in `model/openkache.smithy`, regenerate the contract, and let the
generation/conformance tests catch any implementation that still carries a
stale literal.
