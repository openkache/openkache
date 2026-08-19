# OpenKache client core target design

> **Status:** Draft `draft-2026-08-19.3`; not released or finalized.

This document describes the pre-freeze target for `openkache-client-core`.
The [README](README.md) documents the current crate surface.

## API model

Address and value representation are independent:

| Address | Value |
|---|---|
| Mapped typed key | Formatted v1 |
| Exact Item ID | Raw bytes |
|  | Caller-owned v0 envelope |

Mapped operations use `NamespaceHash` by default and may explicitly select
`PublicKeyOrHash` per operation. Different profiles may coexist in one
namespace. Exact operations bypass key mapping.

Typed keys use `Integer`, `Text`, or `Bytes`; `Integer` is signed `i64`.

High-level Exact APIs reject an empty Item ID unless the caller opts in. Raw
protocol APIs accept the full `0..=32` wire range.

## Request outcomes

The core owns request correlation, retries, cancellation, and lane failure.
Unknown mutation outcome is a distinct public result category. It is never
collapsed into a generic transport error or automatically replayed.

## Transport and security

The target core supports QUIC and TLS-over-TCP with identical v1 frame bytes.
Each transport is an independent conformance profile; maintained clients
support both. Both require TLS 1.3 and `X25519MLKEM768`.

Value processing retains all three stable profiles: `Unprotected`,
`AES-256-GCM-SIV`, and `AES-SIV-CMAC`. Key mapping and value protection remain
independent.

## Sources of truth

- [Client implementation guide](../CLIENT.md)
- [Key format](../KEY_FORMAT.md)
- [Value format](../VALUE_FORMAT.md)
- [Value security](../VALUE_SECURITY.md)
- [Wire protocol](../../protocol/SPEC.md)
