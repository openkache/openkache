# OpenKache client core target design

> **Status:** Draft `draft-2026-08-19.4`; not released or finalized.

This document describes the pre-freeze target for `openkache-client-core`.
The [README](README.md) documents the current crate surface.

## API model

Address and value representation are independent:

```text
Address = MappedKey | ExactItemId
Value   = FormattedV1 | RawValue | CallerOwnedV0
```

Every address mode supports every value mode.

Mapped operations use `NamespaceHash` by default and may explicitly select
`PublicKeyOrHash` per operation. Different profiles may coexist in one
namespace. Exact operations bypass key mapping.

Typed keys use `Integer`, `Text`, or `Bytes`; `Integer` is signed `i64`.

High-level Exact APIs reject an empty Item ID unless the caller opts in.
Low-level wire-operation APIs accept the full `0..=32` wire range.

## Request outcomes

The core owns request correlation, retries, cancellation, and lane failure.
Unknown mutation outcome is a distinct public result category. It is never
collapsed into a generic transport error or automatically replayed.

## Transport and security

The target core supports QUIC and TLS-over-TCP with identical v1 frame bytes.
Each transport is an independent conformance profile; maintained clients
support both. Both require TLS 1.3 and `X25519MLKEM768`. Server certificate and
identity verification is enabled by default and requires an explicit insecure
option to disable.

Value processing retains all three stable profiles: `Unprotected`,
`AES-256-GCM-SIV`, and `AES-SIV-CMAC`. Key mapping and value protection remain
independent. Automatic compression uses Zstandard level 1 only when the
completed frame is smaller; it has no input-size or minimum-savings threshold.

The core enforces one aggregate in-flight byte budget across network,
cryptographic, compression, and decoding work.

## Sources of truth

- [Client implementation guide](../CLIENT.md)
- [Key format](../KEY_FORMAT.md)
- [Value format](../VALUE_FORMAT.md)
- [Value security](../VALUE_SECURITY.md)
- [Wire protocol](../../protocol/SPEC.md)
