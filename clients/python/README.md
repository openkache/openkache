# OpenKache Python client

`openkache` is the maintained synchronous Python client for the frozen
OpenKache v1 Gate 0 (`v1-gate0`) contract. It exposes only the five contract operations:

| Operation | Result |
| --- | --- |
| `Client.connect(address)` | A connected client |
| `client.get(key)` | `Found(value)` or `Missing` |
| `client.set(key, value)` | `SetOutcome` |
| `client.delete(key)` | `DeleteOutcome.DELETED` or `DeleteOutcome.NOT_FOUND` |
| `client.close()` | Idempotent resource release |

Mapped keys accept UTF-8 `str`, signed 64-bit `int`, and bytes-like values.
Every public value read and write uses `StructuredValue-CBOR-v1`; JSON, raw
operations, TTL and retry overrides, cancellation controls, and certificate
configuration are outside this Gate 0 facade.

`SetOutcome` contains only `CREATED` and `REPLACED`. A conditional
`NOT_STORED` response is outside Gate 0 and raises
`OpenKacheIncompatibleServerError`, because the facade never sends conditional
writes. A mutation whose response is lost raises the distinct
`OpenKacheUnknownMutationError`.

Published package: [pypi.org/project/openkache](https://pypi.org/project/openkache/).
Python 3.11 or newer is required.

## Install

```bash
python -m pip install openkache
```

The wheel includes the native client adapter. No compiler or OpenKache source
checkout is needed for a normal installation.

## Quick smoke test

Start a local preview server on `127.0.0.1:4433`, then run:

```bash
python -m pip install --upgrade openkache
python
```

The fixed development TLS profile disables server-certificate verification.
It is suitable for a local preview only and must not be used for production
traffic.

## Quick start

The package-local example is intentionally development-only. It uses
QUIC-over-TLS 1.3 with server-certificate verification disabled.

```bash
OPENKACHE_ADDRESS=127.0.0.1:4433 python examples/basic.py
```

```python
from openkache import Client, DeleteOutcome, Found, Missing

client = Client.connect("127.0.0.1:4433")
try:
    print(client.set("hello", {"from": "python"}))

    result = client.get("hello")
    if isinstance(result, Found):
        print(result.value)
    elif isinstance(result, Missing):
        print("missing")

    if client.delete("hello") is DeleteOutcome.DELETED:
        print("deleted")

    if isinstance(client.get("hello"), Missing):
        print("missing after delete")
finally:
    client.close()
```

`set` returns `SetOutcome.CREATED` or `SetOutcome.REPLACED`. `get` returns a
`Found` wrapper or `Missing`; a stored `None` or `UNDEFINED` value is still
`Found`. `delete` returns `DeleteOutcome.DELETED` or
`DeleteOutcome.NOT_FOUND`.

`client.get` always returns model wrappers that preserve `Undefined`,
`None`/Null, booleans, arbitrary integers, Float16/32/64 width and raw bits,
bytes, text, arrays, and ordered scalar-key maps. This lossless contract keeps
every StructuredValue kind observable to Python callers.

## Configuration

Gate 0 has no certificate, timeout, retry, TTL, or transport configuration. The
development example reads only `OPENKACHE_ADDRESS`.

The lossless constructors and codec helpers are available for applications
that need an explicit model value:

```python
from openkache import FloatValue, IntegerValue, MapValue, UNDEFINED

value = MapValue([
    ("count", IntegerValue(2**80)),
    ("missing", UNDEFINED),
    ("half", FloatValue(16, 0x3C00)),
])
```

## Build and verify

Run these commands from `openkache/clients/python` in a checkout with the
repository development tools:

```bash
python -m compileall src examples
OPENKACHE_GENERATION_TARGET=python ../generate.ts
python -m unittest ../../../tests/clients/python_value_test.py
python -m build --sdist --wheel --outdir dist
```

The generated Smithy modules and platform-native adapter are package build
outputs. Do not commit `dist/`, `build/`, generated modules, or native
libraries.

## Package layout

- `src/openkache/_client.py` — Gate 0 facade and key/result mapping.
- `src/openkache/_native.py` — ctypes ownership and native ABI conversion.
- `src/openkache/_value.py` — lossless StructuredValue-CBOR-v1 codec.
- `src/openkache/_generated/` — generated contract and ABI modules.
- `native/` — thin Rust `cdylib` adapter over the shared client core.
- `examples/basic.py` — clearly labeled development-only TLS example.

## License

The Python client and native adapter are distributed under the Apache License
2.0. Package artifacts include the license text.
