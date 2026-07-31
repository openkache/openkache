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
./generate.ts
cargo build
cargo check
cargo fmt --check
```

Generation and Rust builds require Bun and Smithy CLI on `PATH`. Cargo invokes
the generator automatically before compiling the protocol crate.

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

- `model/openkache.smithy` is the canonical source for assigned wire values,
  operation inputs and outputs, shared limits, version-specific layouts, the
  complete cross-language value-format contract, and the legacy TypeScript
  metadata-envelope limits.
- `generate.ts` validates the Smithy AST and emits Rust, TypeScript, C#, and
  native C definitions into ignored `generated_local` directories or Cargo
  output. The CMake client build generates its native header into the CMake
  build tree and installs it with the package.
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

Protocol identifiers, operation shapes, and wire ceilings are compile-time
definitions sourced from the Smithy model. The crate has no runtime
configuration files.
