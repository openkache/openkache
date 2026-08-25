# OpenKache Python client

OpenKache is a super-fast open-source SSD cache server. Use this Python client
to store, read, and delete values in a few lines.

[PyPI package](https://pypi.org/project/openkache/) ·
[GitHub source](https://github.com/openkache/openkache/tree/main/clients/python)

## Install

Python 3.11 or newer is required.

```bash
# pip
python -m pip install openkache

# uv
uv add openkache

# Existing uv virtual environment:
uv pip install openkache

# Poetry
poetry add openkache

# PDM
pdm add openkache

# Pipenv
pipenv install openkache
```

Published wheels support Linux x86_64/ARM64, macOS x86_64/ARM64, and Windows
x86_64. Linux wheels use the `manylinux_2_38` policy tag; macOS and Windows
tags come from their native Python packaging toolchain. Windows ARM64 and
other Rust-supported targets can install from the source distribution.

The native Rust adapter supports Linux, macOS, and Windows on Rust-supported
host architectures, including ARM64. Building from source requires Python,
Rust 1.85 or newer, and the platform's normal C linker. The package selects
`.so`, `.dylib`, or `.dll` at import time and does not assume Linux.

On Windows checkouts, run the generator from PowerShell:

```powershell
$env:OPENKACHE_GENERATION_TARGET = "python"
..\generate.ts
```

## Quick start

The example below assumes a local OpenKache server at `127.0.0.1:4433`.

```python
from openkache import Client

client = Client.connect("127.0.0.1:4433")
print(client.set("greeting", "hello"))  # SetOutcome.CREATED
print(client.get("greeting"))           # Found(value='hello')
print(client.delete("greeting"))        # True
client.close()
```

`set` returns `SetOutcome.CREATED` for a new key,
`get` returns `Found(value)`, and `delete` returns `True` when a value was
removed.

> The example uses the local development TLS profile, which does not verify
> the server certificate. Use it only with a local development server.

## Reference

### `Client.connect(address)`

Opens a connection and returns a `Client`.

- **Input:** a non-empty `host:port` string. IPv6 endpoints use `[host]:port`.
- **Returns:** `Client`.
- **Raises:** `OpenKacheError` when the connection cannot be opened.

```python
client = Client.connect("127.0.0.1:4433")
```

`Client` and `OpenKacheClient` refer to the same class.

### `client.get(key)`

Reads one value.

- **Input:** a UTF-8 `str`, signed 64-bit `int`, or bytes-like value
  (`bytes`, `bytearray`, or `memoryview`).
- **Returns:** `Found(value)` when the key exists, or `Missing` when it does
  not. A stored `None` or `UNDEFINED` is still returned as `Found`.
- **Raises:** `OpenKacheValueError` for an invalid key and `OpenKacheError`
  for connection or server failures.

```python
from openkache import Found

result = client.get("greeting")
if isinstance(result, Found):
    print(result.value)
```

`MISSING` is a shared `Missing` instance. `GetResult` is the
`Found | Missing` type alias.

### `client.set(key, value)`

Stores one value, replacing any existing value for the key.

- **Input:** the same key types accepted by `get`, plus a native or lossless
  structured value.
- **Returns:** `SetOutcome.CREATED` for a new key or
  `SetOutcome.REPLACED` for an existing key.
- **Raises:** `OpenKacheValueError` for an invalid key or value and
  `OpenKacheError` for connection or server failures.

```python
outcome = client.set("greeting", "hello")
# outcome is SetOutcome.CREATED or SetOutcome.REPLACED
```

### `client.delete(key)`

Deletes one key. Deleting a missing key is safe.

- **Input:** a key accepted by `get`.
- **Returns:** `True` when a value was removed, or `False` when no value
  existed.
- **Raises:** `OpenKacheValueError` for an invalid key and
  `OpenKacheError` for connection or server failures.

```python
removed = client.delete("greeting")
if removed:
    print("deleted")
```

### `client.close()`

Closes the connection and returns `None`. Calling it more than once is safe.

```python
client.close()
```

The client also supports `with Client.connect(address)`, which closes the
connection automatically when the block exits.

### Keys

Keys are typed. Use:

- `str` for UTF-8 text keys;
- `int` for signed 64-bit integer keys;
- `bytes`, `bytearray`, or `memoryview` for exact byte keys.

```python
client.get("text-key")
client.get(42)
client.get(b"bytes-key")
```

### Values

The client converts common Python values to structured values:

- `None` becomes Null.
- `bool` becomes Boolean.
- `int` becomes an exact Integer.
- `float` becomes an IEEE-754 binary64 Float.
- `str` becomes UTF-8 TextString.
- `bytes`, `bytearray`, and `memoryview` become Bytes.
- `list` and `tuple` become Array.
- `dict` becomes Map.

```python
client.set("profile", {"name": "Ada", "active": True})
```

Use the lossless model when the exact representation matters:
`UNDEFINED`/`UndefinedValue`, `IntegerValue`, `FloatValue`,
`ByteStringValue`, `TextStringValue`, `ArrayValue`, and `MapValue`.
The short names `Undefined`, `Integer`, `Float`, `ByteString`, `TextString`,
`Array`, `Map`, and `Value` are compatibility aliases.

### Value helpers

- `to_value(value, limits=None)` converts a native Python value to the
  lossless model.
- `encode_value(value, limits=None)` returns one encoded
  `StructuredValue-CBOR-v1` item as `bytes`.
- `decode_value(data, limits=None)` decodes one complete item from
  `bytes`-like input.
- `model_equal(left, right)` compares model values without treating
  `True` and `1` as equal.
- `ValueLimits` bounds encoded bytes, nesting depth, item count, and integer
  magnitude.

```python
from openkache import decode_value, encode_value, model_equal, to_value

encoded = encode_value({"count": 1})
decoded = decode_value(encoded)
assert model_equal(decoded, to_value({"count": 1}))
```

`StructuredValueError` reports conversion, encoding, decoding, and resource
limit failures. Its `kind` property is a `ValueErrorKind`.

### Errors

- `OpenKacheError` — connection, protocol, server, or operation failure.
- `OpenKacheValueError` — invalid key or value supplied by the caller.
- `OpenKacheUnknownMutationError` — a mutation may have reached the server
  without a confirmed result; do not replay it automatically.
- `OpenKacheIncompatibleServerError` — the server returned an outcome that
  this client does not support.
- `StructuredValueError` — invalid structured-value data or resource limits.

## More information

- [OpenKache on PyPI](https://pypi.org/project/openkache/)
- [OpenKache repository](https://github.com/openkache/openkache)
