# OpenKache Python client

The `openkache` package is an asyncio-friendly Python binding over the shared
Rust client core. QUIC, TLS, retries, application-key derivation, compression,
encryption, and the v1 value format stay in [`../core`](../core); Python only
converts Python values and owns the async scheduling and resource lifecycle.

The Smithy model in [`../../protocol/model/openkache.smithy`](../../protocol/model/openkache.smithy)
is the source of the generated operation types, wire/value constants, and
native ABI identifiers in `src/openkache/_generated/`. The `RawClient` adapter
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
resulting portable ctypes library in the wheel. A C compiler or Python
extension ABI is not required. Package builds require Bun and the Smithy CLI;
the reproducible development shell supplies all of these tools.

## Usage

```python
from pathlib import Path

from openkache import Client, SetOptions

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
`get_raw` for exact bytes; empty raw values are supported. Keys may be UTF-8
strings or bytes and must not be empty. JSON numbers are finite and limited to
the exact IEEE-754 integer range.

`client.raw` exposes the Smithy-shaped exact item-ID API:

```python
from openkache import (
    SmithyGetInput,
    SmithySetInput,
)

item_id = bytes(32)
await client.raw.set(SmithySetInput(item_id=item_id, value=b"opaque"))
result = await client.raw.get(SmithyGetInput(item_id=item_id))
```

## Configuration

- `certificate` accepts a DER/PEM path or bytes containing one trusted
  certificate or PEM chain.
- `data_protection_key` is an application-managed 32-byte secret shared by
  clients that must address the same protected entries.
- `server_name` defaults to the hostname from `address` and is used for TLS
  verification after DNS resolution.
- `identity` accepts a `ClientIdentity` with a PEM/DER client chain and private
  key for mutual TLS.
- `compression`, `timeouts`, `max_in_flight`, and `retry_max_attempts` map
  directly to shared-core settings.
- `native_path` or `OPENKACHE_CLIENT_NATIVE` selects a custom native artifact.

Call `close()` when finished; it is idempotent. The client also supports
`async with`. `stats()` returns validated `ServerStats`, while
`stats_json()` preserves the Smithy response text.

## Components

- `src/openkache/_client.py` contains the small Python API and validation.
- `src/openkache/_native.py` contains only ctypes ownership and ABI conversion.
- `native/` re-exports the core C ABI without protocol logic.
- `src/openkache/_generated/` is regenerated from Smithy during every package
  build and is intentionally not checked into source control.
