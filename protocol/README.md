# OpenKache protocol

`openkache-protocol` is the small Rust wire-primitives crate shared by clients
and servers using the still-unpublished, evolving protocol-v1 draft profile.

## Purpose

The crate owns transport identifiers and limits, canonical `vu128` helpers,
item-ID bytes, opaque framing, reusable value/layout codecs, and the generated
numeric operation contract. Each modeled operation declares its request and
response fields and an explicit compact request plan. Generation turns that
model into operation metadata and the shared request layout consumed by
operation-neutral encoders and projectors.

API adapters remain responsible for mapping domain values to generated numeric
fields and interpreting semantic results. The protocol crate does not generate
handlers, client methods, or API-family routing.

The [wire protocol specification](SPEC.md) defines transport negotiation, frame
bytes, stable operation semantics, limits, malformed input handling, and
ambiguous operation outcomes. [Server semantics](SERVER_SEMANTICS.md) defines
identity-domain, TTL recovery, and eviction obligations. [Experimental
operations](EXPERIMENTAL.md) defines optional benchmark and internal
operations. This README covers crate usage and implementation structure.

## Commands

From `protocol`:

```bash
./generate.ts                 # Generate wire values and operation contracts
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

- `model/openkache.smithy` is the canonical source for transport-visible
  identifiers, statuses, limits, operation fields, codecs, and explicit compact
  request plans.
- `wire.ts` owns AST extraction and deterministic rendering of wire values,
  numeric request and response field modules, operation metadata, and shared
  request layouts.
- `generate.ts` emits `wire_values.rs`, `operation_contract.rs`, and the
  draft-v1 compatibility projection consumed by the Rust crate. It does not
  generate API handlers or client methods.
- `Opcode`, `Status`, and their generated `ALL`, `COUNT`, `NAMES`, `index()`,
  and `name()` metadata come from the Smithy enums so adapters and metrics do
  not repeat wire labels.
- `encode_request_frame`, `wire_request_layout`, and `OwnedRequestFrame` encode
  generated numeric fields into ordered compact wire segments while retaining
  large field owners instead of coalescing their bytes.
- `OpaqueRequestFrame`, `decode_request_frame_header`, and
  `project_request_frame` delimit and project requests from the same generated
  layouts without interpreting API semantics.
- `Response` and `ResponseFrame` validate and encode complete opaque response
  frames; `ResponseHeader` supports bounded incremental reads.
- `ProtocolError` classifies malformed, unsupported, and oversized wire
  frames.
- `SPEC.md` defines the implementation-independent contract.

## Configuration

Transport identifiers, status assignments, wire ceilings, operation fields,
codec declarations, and compact request plans are compile-time definitions
sourced from the wire Smithy model. Change those values in
`model/openkache.smithy` and regenerate. API adapters own domain-to-field
mapping, semantic validation beyond the wire contract, handler behavior, and
result interpretation; they do not redefine request framing.
