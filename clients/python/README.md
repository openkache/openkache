# OpenKache Python client

The `openkache` package is an asyncio-friendly Python binding over the shared
Rust client core. TLS 1.3 over QUIC or TCP, retries, application-key
derivation, compression, encryption, and the v1 value format stay in
[`../core`](../core); Python only converts Python values and owns the async
scheduling and resource lifecycle. Pass `Transport.TLS_TCP` for verified
TLS-over-TCP or an explicit insecure enum member to disable certificate and
server-identity verification; the default remains verified QUIC.

The Smithy client model in [`../model/openkache.smithy`](../model/openkache.smithy), together with
the wire model in [`../../protocol/model/openkache.smithy`](../../protocol/model/openkache.smithy),
currently generates the transitional operation types, client constants, and native ABI identifiers
in `src/openkache/_generated/`. The `RawClient` adapter
implements those exact item-ID operations. `Client` adds protected
application-key operations and JSON values.

The package also exposes the lossless ``StructuredValue-CBOR-v1`` model from
``openkache._value`` (and the package root). Python ``bool`` is tested before
``int``; Python integers retain arbitrary precision, bytes-like values map to
byte strings, and both ``list`` and ``tuple`` map to model arrays.
``decode_value`` returns the complete lossless model, while ``decode_native``
performs a strict conversion and rejects ``Undefined`` or map-key collisions
instead of dropping information. Cache methods use canonical UTF-8 JSON as
`OpaqueBytes`; structured payload operations use the generated
StructuredValue-CBOR-v1 ABI without JSON or Raw fallback. The native boundary preserves unknown mutation
and cancellation outcomes as ``OpenKacheUnknownMutationError`` and
``OpenKacheCancelledError``.

Mapped GET/SET/DELETE calls use the ABI v1 request handle
(``poll``/``wait``/``cancel``/``free``), so task cancellation cannot abandon a
native mutation. Cancellation before admission raises
``OpenKacheCancelledError``; cancellation after a mutation starts raises
``OpenKacheUnknownMutationError``. Structured, scoped, namespace, and complete
raw-policy calls have no dedicated request entry point in ABI v1 and therefore
drain a documented safe completion boundary before honoring cancellation.

> **Current implementation:** This package exposes canonical-JSON mapped-key
> operations and exact `0..=32`-byte Item ID operations. Native values are
> converted to logical bytes plus a generated key discriminator at the adapter
> boundary; the shared core performs canonical encoding and applies the default
> `NamespaceHash` profile. `PublicKeyOrHash` remains an explicit core-only
> profile until the binding options are finalized.

## Commands

Run from `clients/python`:

```bash
python -m compileall src
python -m build
```

The package build first regenerates the Smithy modules in
`src/openkache/_generated/`, then runs Cargo for `native/` and places the
resulting target-native ctypes library in the wheel. Source distributions bundle
the shared core and protocol source so a wheel is compiled for the target
platform; they never carry a host native binary. A C compiler or Python
extension ABI is not required. Package builds require Bun and the Smithy CLI;
the reproducible development shell supplies all of these tools.

## Usage

```python
from pathlib import Path

from openkache import Client, KeySpec, SetOptions

client = await Client.connect(
    "cache.example.com:4433",
    certificate="client-bundle/ca.crt",
    data_protection_key=Path("client-bundle/data-protection.key").read_bytes(),
)
try:
    await client.set(
        "profile",
        {"name": "Kim", "visits": 42, "active": True},
        SetOptions(condition="if_absent", ttl_ms=300_000),
    )
    profile = await client.get("profile")
    raw = await client.get_raw("opaque")
finally:
    await client.close()
```

`set` and `get` use the core canonical JSON value format. Use `set_raw` and
`get_raw` for exact bytes; empty raw values are supported. Each operation
infers `Text`, `Bytes`, or signed-i64 `Integer` from the native key and passes
the logical bytes plus the generated key discriminator through the native ABI;
the shared core performs canonical deterministic CBOR encoding. The
deprecated `key_spec` option is accepted for source compatibility but is not a
namespace policy and does not override per-operation inference. Empty and
NUL-containing keys are valid. JSON numbers
are finite, and integers
must be exactly representable as IEEE-754 binary64 values. Python converts a
native value to a UTF-8 JSON input buffer only to cross the ctypes ABI; the
core reparses that input and owns canonical serialization, compression,
encryption, and value framing.

JSON helpers are an explicitly documented convenience surface: they carry
canonical UTF-8 JSON as `OpaqueBytes`, while the structured operation uses
`StructuredValue-CBOR-v1` and never substitutes JSON or Raw payloads.
`set_v0` / `get_v0` accept a complete caller-owned version-0 envelope,
validate only its canonical leading version field, and preserve the remaining
bytes unchanged. The `client.raw` view exposes the same value modes for exact
Item IDs.

`client.raw` exposes the current transitional Smithy-shaped exact item-ID API.
The namespace-open call below is an out-of-band WIP control-plane example, not
a stable v1 wire operation; stable data operations still use a
server-assigned namespace ID.

```python
from openkache import (
    SmithyGetInput,
    SmithyNamespaceOpenInput,
    SmithyNamespacePolicy,
    SmithyExpirationDefault,
    SmithyEvictionDefault,
    SmithyOverridePolicy,
    SmithySetInput,
)

item_id = b"short-id"
namespace = await client.raw.namespace_open(
    SmithyNamespaceOpenInput(
        name="example",
        create_if_missing=True,
        policy=SmithyNamespacePolicy(
            default_expiration=SmithyExpirationDefault.NO_EXPIRY,
            expiration_override=SmithyOverridePolicy.ALLOWED,
            default_eviction=SmithyEvictionDefault.EVICTABLE,
            eviction_override=SmithyOverridePolicy.ALLOWED,
        ),
    )
)
namespace_id = namespace.descriptor.namespace_id
await client.raw.set(
    SmithySetInput(namespace_id=namespace_id, item_id=item_id, value=b"opaque")
)
result = await client.raw.get(
    SmithyGetInput(namespace_id=namespace_id, item_id=item_id)
)
```

## Configuration

- `certificate` accepts a DER/PEM path or bytes containing one trusted
  certificate or PEM chain.
- `data_protection_key` is optional. When supplied it is an
  application-managed 32-byte secret shared by clients that must address the
  same protected entries. When omitted, values are unprotected.
- The deprecated `key_spec`/`KeySpec` names remain accepted for source
  compatibility. They do not select a namespace policy or Item ID mapping
  profile; mapped operations infer the `TypedKey` variant per call.
- `server_name` defaults to the hostname from `address` and is used for TLS
  verification after DNS resolution.
- `identity` accepts a `ClientIdentity` with a PEM/DER client chain and private
  key for mutual TLS.
- `compression`, `encryption`, `timeouts`, `max_in_flight`, and
  `retry_max_attempts` map directly to shared-core settings. Compression
  defaults to automatic level-1 Zstandard with no input-size or
  minimum-savings threshold; pass `CompressionOptions(enabled=False)` for an
  explicit opt-out. `Encryption.ROBUST` is the default; select
  `Encryption.COMPACT` only when every client sharing the protected entries
  uses that profile.
- `native_path` or `OPENKACHE_CLIENT_NATIVE` selects a custom native artifact.

Call `close()` when finished; it is idempotent. The client also supports
`async with`. `experimental_stats()` returns validated `ServerStats`, while
`experimental_stats_json()` preserves the Smithy response text.
`experimental_stats` is transitional
experimental behavior; for a draft-conforming peer, enable
`enable_experimental_api` and coordinate the exact revision in
[`protocol/EXPERIMENTAL.md`](../../protocol/EXPERIMENTAL.md) before calling it.

## Components

- `src/openkache/_client.py` contains the small Python API and validation.
- `src/openkache/_native.py` contains only ctypes ownership and ABI conversion.
- `native/` re-exports the core C ABI without protocol logic.
- `src/openkache/_generated/smithy_*.py` is regenerated from Smithy during
  every package build and is intentionally not checked into source control.
  The small `__init__.py` package facade is handwritten and tracked so the
  generated modules remain importable after a clean checkout.
