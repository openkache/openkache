# OpenKache Python client

Store, read, and delete values in an OpenKache server from Python.

[PyPI package](https://pypi.org/project/openkache/) ·
[GitHub source](https://github.com/openkache/openkache/tree/main/clients/python)

## Install

The package requires Python 3.11 or newer.

| Tool | Command |
| --- | --- |
| pip | `python -m pip install openkache` |
| uv | `uv add openkache` |
| Poetry | `poetry add openkache` |
| PDM | `pdm add openkache` |
| Pipenv | `pipenv install openkache` |

For an existing virtual environment managed by uv, use
`uv pip install openkache`.

The current release publishes one Linux x86_64 wheel
(`manylinux_2_38`) and one source distribution. Other platforms do not have
published wheels; install from the source distribution only when a local Rust
toolchain and C linker can build the native adapter.

## Quick start

The example below assumes a local OpenKache server at `127.0.0.1:4433`.

```python
from openkache import Client, Found, Missing

with Client.connect("127.0.0.1:4433") as client:
    print("SET:", client.set("greeting", {"message": "hello"}))

    result = client.get("greeting")
    print("GET:", result.value if isinstance(result, Found) else "missing")

    print("DELETE:", client.delete("greeting"))

    result = client.get("greeting")
    if isinstance(result, Missing):
        print("GET after DELETE: missing")
```

`Client` can also be closed explicitly with `client.close()`.

> The example uses the local development TLS profile, which does not verify
> the server certificate. Use it only with a local development server.

## Reference

### Client

| API | Description |
| --- | --- |
| `Client.connect(address)` | Connect to a `host:port` endpoint and return a client. IPv6 endpoints use `[host]:port`. |
| `client.get(key)` | Return `Found(value)` when the key exists, or `Missing` otherwise. |
| `client.set(key, value)` | Store a value and return `SetOutcome.CREATED` or `SetOutcome.REPLACED`. |
| `client.delete(key)` | Delete a key and return `DeleteOutcome.DELETED` or `DeleteOutcome.NOT_FOUND`. |
| `client.close()` | Close the connection. Repeated calls are safe. |
| `with Client.connect(address)` | Close the client automatically when the block exits. |

`Client` and `OpenKacheClient` refer to the same client class.

Keys can be UTF-8 `str`, signed 64-bit `int`, or `bytes`-like values
(`bytes`, `bytearray`, and `memoryview`).

### Results

- `Found(value)` contains the value returned by `get`.
- `Missing` represents an absent key. `MISSING` is a shared instance.
- `SetOutcome.CREATED` and `SetOutcome.REPLACED` describe `set`.
- `DeleteOutcome.DELETED` and `DeleteOutcome.NOT_FOUND` describe `delete`.
- `GetResult` is the `Found | Missing` result type.

### Values

The normal Python values accepted by `set` are:

| Python value | Stored value |
| --- | --- |
| `None` | Null |
| `bool` | Boolean |
| `int` | Exact integer |
| `float` | IEEE-754 float |
| `str` | UTF-8 text |
| `bytes`, `bytearray`, `memoryview` | Bytes |
| `list`, `tuple` | Array |
| `dict` | Map |

Use the lossless model when the exact value representation matters:

- `UNDEFINED` / `UndefinedValue`
- `IntegerValue`
- `FloatValue`
- `ByteStringValue`
- `TextStringValue`
- `ArrayValue`
- `MapValue`

The value helpers are `to_value`, `encode_value`, `decode_value`, and
`model_equal`. `ValueLimits` controls the resource limits used by conversion
encoding, and decoding, and `ValueErrorKind` identifies value-codec failures.
`Array`, `ByteString`, `Float`, `Integer`, `Map`, `TextString`, `Undefined`,
and `Value` are compatibility aliases for the model types.

### Errors

- `OpenKacheError` — connection, protocol, server, or operation failure.
- `OpenKacheValueError` — invalid key or value supplied by the caller.
- `OpenKacheUnknownMutationError` — a mutation may have reached the server,
  but its result was not confirmed; do not replay it automatically.
- `OpenKacheIncompatibleServerError` — the server returned an outcome that
  this client does not support.
- `StructuredValueError` — invalid structured-value data or resource limits.

## More information

- [OpenKache on PyPI](https://pypi.org/project/openkache/)
- [OpenKache repository](https://github.com/openkache/openkache)
