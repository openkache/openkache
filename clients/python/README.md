# OpenKache Python client

`openkache` is a synchronous Python client for the OpenKache cache server. It
uses the shared Rust client core for QUIC, TLS 1.3 over TCP, retries,
compression, authenticated value protection, and the wire protocol. The
Python layer provides validation, Python value conversion, and deterministic
resource management.

This is an experimental preview. The protocol and generated Smithy API are
still transitional, so pin the package version and coordinate upgrades with
the server version you operate.

## Install

The normal installation is:

```bash
python -m pip install openkache
```

The package requires Python 3.11 or newer. Published wheels contain the
platform-native Rust adapter and do not require a C compiler or a Python
extension toolchain. If pip falls back to the source distribution, the build
also needs Rust/Cargo, a C/C++ compiler with CMake (for the AWS-LC TLS
dependency), Bun, and the Smithy CLI used by the shared client core.
Maintainers should publish one wheel per supported target platform so normal
installation does not need those build tools.

## Quick start

The verified transports require a CA certificate that trusts the server
certificate. `OPENKACHE_CA_CERT` may be a PEM/DER file path or the equivalent
bytes passed directly to `Client.connect`.

```python
import os
from pathlib import Path

from openkache import Client, SetOptions


def main() -> None:
    address = os.environ.get("OPENKACHE_ADDRESS", "127.0.0.1:4433")
    ca_certificate = Path(os.environ["OPENKACHE_CA_CERT"])

    with Client.connect(
        address,
        certificate=ca_certificate,
    ) as client:
        client.ping()
        outcome = client.set(
            "profile",
            {"name": "Kim", "visits": 42, "active": True},
            SetOptions(condition="if_absent", ttl_ms=300_000),
        )
        profile = client.get("profile")
        print(outcome.value, profile)


main()
```

The same lifecycle can be written with explicit `client.close()`; close is
idempotent. See [`examples/basic.py`](examples/basic.py) for a runnable version
with environment-variable handling.

## Values and keys

The high-level `Client` alias is the same class as `OpenKacheClient`.

- `set` and `get` use the core's canonical UTF-8 JSON value profile.
- `set_raw` and `get_raw` store and retrieve exact decrypted bytes, including
  empty values.
- `set_structured` and `get_structured` use
  `StructuredValue-CBOR-v1`. The default `"lossless"` representation preserves
  `Undefined`, arbitrary-precision integers, float width/bits, byte strings,
  text strings, arrays, and ordered maps. `"native"` projects to ordinary
  Python values and rejects information-losing values.
- `set_v0` and `get_v0` accept a complete caller-owned version-0 value envelope
  and preserve its body unchanged.

Mapped operations infer a typed key from each call: `str` is text, bytes-like
values are bytes, and signed 64-bit `int` values are integers. Boolean keys
are rejected. Empty and NUL-containing keys are valid. `key_spec`/`KeySpec`
remain accepted for source compatibility with older clients, but do not select
a namespace policy.

JSON values must contain finite numbers. Integers must be exactly representable
by the shared IEEE-754 binary64 JSON model. Use the structured or raw APIs when
that restriction is not appropriate.

## API surface

The high-level client exposes:

| Method | Result |
| --- | --- |
| `ping()` | Verifies that the server is reachable. |
| `get()` / `set()` | Reads or writes canonical JSON values. |
| `get_raw()` / `set_raw()` | Reads or writes exact bytes. |
| `get_structured()` / `set_structured()` | Reads or writes lossless StructuredValue-CBOR-v1 values. |
| `get_v0()` / `set_v0()` | Reads or writes complete caller-owned v0 envelopes. |
| `delete()` | Deletes a mapped key and reports whether it existed. |
| `experimental_stats()` / `experimental_stats_json()` | Returns transitional server statistics or the original response text. |
| `experimental_sync()` / `reconnect()` | Flushes the core or requests a reconnect. |
| `connection_state()` | Returns the best-effort native connection state. |
| `close()` | Releases the native client; safe to call more than once. |

## Connection configuration

`Client.connect(address, ...)` accepts these important options:

| Option | Meaning |
| --- | --- |
| `certificate` | Trusted CA certificate or PEM/DER path. Required for verified peers. |
| `server_name` | TLS server name; defaults to the hostname in `address`. |
| `transport` | `Transport.QUIC` (default), `Transport.TLS_TCP`, or an explicit insecure enum member for local/test deployments. |
| `identity` | Optional `ClientIdentity` containing a client certificate chain and private key for mutual TLS. |
| `data_protection_key` | Optional 32-byte application-managed secret shared by clients addressing the same protected entries. |
| `compression` | `CompressionOptions`; automatic level-1 Zstandard is the default. |
| `encryption` | `Encryption.ROBUST` (default) or `Encryption.COMPACT`. All clients sharing protected entries must use the same profile. |
| `timeouts` | `ClientTimeouts(connect_ms, request_ms)`. |
| `max_in_flight` | Maximum number of concurrent requests admitted by the native core. |
| `retry_max_attempts` | Maximum attempts for retryable failures. |
| `native_path` | Explicit path to a compatible native adapter. |

`OPENKACHE_CLIENT_NATIVE` is an equivalent environment-variable override for
`native_path`. Keep native artifacts produced by the same package release; the
Python wrapper checks the generated ABI version before using one.

If a native failure leaves a mutation outcome uncertain, the client raises
`OpenKacheUnknownMutationError` instead of pretending that the mutation did
not happen.

## Raw Smithy API

`client.raw` exposes the generated exact Item ID API for transitional control
and interoperability work. Item IDs are `bytes` values of at most 32 bytes:

```python
from openkache import SmithyGetInput, SmithySetInput

namespace_id = 1  # Replace with the server-assigned namespace ID.
client.raw.set(
    SmithySetInput(namespace_id=namespace_id, item_id=b"item", value=b"opaque")
)
result = client.raw.get(
    SmithyGetInput(namespace_id=namespace_id, item_id=b"item")
)
```

The namespace-open/update/delete operations are currently out-of-band WIP
control-plane operations, not a stable v1 wire contract. Use the server's
assigned namespace ID for stable data operations.

## Build and verify

Run these commands from `clients/python` in a checkout that has the repository
development tools available:

The maintenance operations are explicitly experimental in the current
protocol. The Python package currently exposes a blocking API only; use
`experimental_stats()` and `experimental_sync()` directly and coordinate the
server's draft revision before relying on them.

```bash
python -m compileall src examples
python -m build --sdist --wheel --outdir dist
python -m twine check dist/*
# After CI has verified every platform wheel:
python -m twine upload dist/*
```

The wheel build copies one target-native library into the package. In the
monorepo, release automation supplies it through `OPENKACHE_CLIENT_NATIVE`
(normally from the Bazel `openkache_client_python_native` target). A source
distribution intentionally contains no host-native library; it bundles the
shared core, protocol, Smithy models, and all generator modules needed to
rebuild one for the target platform.

Upload a version only once: update `project.version`, build into a clean
directory, run the package-content verification, and use PyPI Trusted
Publishing or a short-lived token for the final upload. Never commit `dist/`,
`build/`, native libraries, or generated Smithy modules.

Generated Python modules are kept out of version control and are regenerated
from the Smithy models for a checkout build. They are included in every sdist,
so rebuilding a wheel from an sdist does not need to regenerate the Python
facade. To deliberately regenerate an existing tree, run:

```bash
python setup.py generate_smithy
```

## Package layout

- `src/openkache/_client.py` — public synchronous API, validation, and result mapping.
- `src/openkache/_native.py` — ctypes ownership and native ABI conversion.
- `src/openkache/_value.py` — lossless StructuredValue-CBOR-v1 conversion.
- `src/openkache/_generated/` — generated Smithy API, operations, constants, and
  native ABI declarations.
- `native/` — thin Rust `cdylib` adapter over `clients/core`.
- `examples/basic.py` — minimal end-to-end usage example.

The shared protocol, security, key-format, and value-format documents live in
the OpenKache repository. The generated API is an implementation surface, not
a replacement for those protocol specifications.

## License

The Python client and its native adapter are distributed under the Apache
License 2.0. The package artifacts include the license text.
