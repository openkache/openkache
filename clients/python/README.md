# OpenKache Python client

The `openkache` package is an asyncio-friendly Python binding over the shared
Rust client core. QUIC, TLS, retries, application-key derivation, compression,
encryption, and the v1 value format stay in [`../core`](../core); Python only
converts Python values and owns the async scheduling and resource lifecycle.

The Smithy client model in [`../model/openkache.smithy`](../model/openkache.smithy), together with
the wire model in [`../../protocol/model/openkache.smithy`](../../protocol/model/openkache.smithy),
is the source of the generated operation types, client constants, and native ABI identifiers in
`src/openkache/_generated/`. The `RawClient` adapter
implements those exact item-ID operations. `Client` adds protected
application-key operations and JSON values.

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
    client_root_key=Path("client-bundle/client-root.key").read_bytes(),
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
`get_raw` for exact bytes; empty raw values are supported. A `str` key is the
v1 `Text` typed keys by default. Select `key_spec=KeySpec.BYTES` or
`key_spec=KeySpec.INTEGER` when the keyspace uses exact bytes or arbitrary
precision integers. The selected spec is enforced for every formatted
operation and logical key bytes plus the explicit discriminator are passed to
the native ABI; the shared core performs canonical encoding (or the configured
`ByteKeyOrHash` mapping). Empty and NUL-containing keys are valid. JSON numbers
are finite, and integers
must be exactly representable as IEEE-754 binary64 values. Python converts a
native value to a UTF-8 JSON input buffer only to cross the ctypes ABI; the
core reparses that input and owns canonical serialization, compression,
encryption, and value framing.

`client.raw` exposes the Smithy-shaped exact item-ID API:

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

item_id = bytes(32)  # Any length from zero through 32 bytes is valid.
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
- `client_root_key` is optional. When supplied it is an
  application-managed 32-byte secret shared by clients that must address the
  same protected entries. When omitted, values are unprotected.
- `key_spec` selects `KeySpec.TEXT` (the default), `KeySpec.BYTES`, or
  `KeySpec.INTEGER`. Use the same spec and logical key type in every language
  client that must share entries.
- `key_format` selects `KeyFormat.HASH` (the default) or
  `KeyFormat.BYTE_KEY_OR_HASH`; the latter requires `KeySpec.BYTES` and
  preserves byte keys up to 32 bytes before hashing longer keys.
- `server_name` defaults to the hostname from `address` and is used for TLS
  verification after DNS resolution.
- `identity` accepts a `ClientIdentity` with a PEM/DER client chain and private
  key for mutual TLS.
- `compression`, `encryption`, `timeouts`, `max_in_flight`, and
  `retry_max_attempts` map directly to shared-core settings. When `encryption`
  is omitted, the shared core selects Robust if a client root key is
  supplied and Unprotected otherwise. An explicit Compact or Robust profile
  requires a client root key; select Compact only when every client
  sharing the protected entries uses that profile.
  `Encryption.UNPROTECTED` explicitly selects the unprotected profile and is
  valid even when a client root key is supplied; this retains root-bound Item
  ID derivation while disabling value protection. Operation-local overrides
  use the same profile identifiers.
- `native_path` or `OPENKACHE_CLIENT_NATIVE` selects a custom native artifact.

Call `close()` when finished; it is idempotent. The client also supports
`async with`. `stats()` returns validated `ServerStats`, while
`stats_json()` preserves the Smithy response text.

## Components

- `src/openkache/_client.py` contains the small Python API and validation.
- `src/openkache/_native.py` contains only ctypes ownership and ABI conversion.
- `native/` re-exports the core C ABI without protocol logic.
- `src/openkache/_generated/smithy_*.py` is regenerated from Smithy during
  every package build and is intentionally not checked into source control.
  The small `__init__.py` package facade is handwritten and tracked so the
  generated modules remain importable after a clean checkout.
