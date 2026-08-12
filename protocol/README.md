# OpenKache protocol

`openkache-protocol` is the small Rust wire-primitives crate shared by
OpenKache protocol v1 clients and servers.

## Purpose

The crate owns generated wire identifiers, canonical `vu128` helpers, item-ID
bytes, and opaque response framing. Request bodies deliberately do not live
here: v1 selects an operation-specific request layout from the opcode, so the
server adapter owns request delimiting and semantic validation while each
client adapter owns request construction and response payload decoding.

The [wire protocol specification](SPEC.md) defines transport negotiation, frame
bytes, operation semantics, limits, malformed input handling, and retry
ambiguity. This README covers crate usage and implementation structure.

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

Encode or decode an opaque response through the shared frame type:

```rust
use openkache_protocol::{Response, Status};

let response_bytes = [Status::Ok as u8, 0x04, b'P', b'O', b'N', b'G'];
let response = Response::decode(&response_bytes)?;
assert_eq!(response.status, Status::Ok);
```

Incremental transports can use `Response::decode_header` and
`Response::frame_len` to determine how many bytes a complete response
requires without duplicating variable-integer parsing.

## Core components

- `model/openkache.smithy` is the canonical source for server-visible wire and
  API values: operation assignments, operation shapes, statuses, shared
  limits, fixed widths, and version-specific frame and flag layout.
- `wire.ts` owns wire-model AST extraction and deterministic rendering of the
  Rust wire contract and the server-owned generated adapter contract.
- `generate.ts` validates the wire Smithy model and emits those Rust
  contracts. The client generator consumes the same protocol model for C, C#,
  Go, Python, Swift, TypeScript, JVM, and Dart API shapes and constants; client
  defaults, native ABI identifiers, and value-format metadata belong to
  [`../clients/model/openkache.smithy`](../clients/model/openkache.smithy) and
  [`../clients/generate.ts`](../clients/generate.ts).
- `Opcode`, `Status`, and their generated `ALL`, `COUNT`, `NAMES`, `index()`,
  and `name()` metadata come from the Smithy enums so adapters and metrics do
  not repeat wire labels.
- `Response` and `ResponseFrame` validate and encode complete opaque response
  frames; `ResponseHeader` supports bounded incremental reads.
- `ProtocolError` classifies malformed, unsupported, and oversized wire
  frames.
- `SPEC.md` defines the implementation-independent contract.

## Configuration

Protocol identifiers, operation assignments, fixed field widths, flag masks,
status boundaries, and wire ceilings are compile-time definitions sourced from
the wire Smithy model. Change these values in `model/openkache.smithy`,
regenerate the contracts, and let the conformance tests catch stale literals.
