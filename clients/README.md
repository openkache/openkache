# OpenKache client libraries

OpenKache is a super-fast open-source SSD cache server.

Use a client library to connect to OpenKache and store, read, or delete values.

## Packages

| Language | Package | Install | Reference | Source |
| --- | --- | --- | --- | --- |
| Python | [PyPI `openkache`](https://pypi.org/project/openkache/) | `python -m pip install openkache` | [Python README](python/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/python) |
| Rust | [crates.io `openkache`](https://crates.io/crates/openkache) | `cargo add openkache` | [Rust README](rust/README.md) · [docs.rs](https://docs.rs/openkache/latest/openkache/) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/rust) |
| TypeScript / JavaScript | [npm `openkache`](https://www.npmjs.com/package/openkache) | `npm install openkache` | [TypeScript README](typescript/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/typescript) |

The package READMEs include alternative package-manager commands, a complete
first-use example, and a reference for every public client API.

All three packages use the same logical operations:

| Operation | Python | Rust | TypeScript / JavaScript |
| --- | --- | --- | --- |
| Connect | `Client.connect(address)` | `Client::connect(endpoint)` | `OpenKacheClient.connect(endpoint)` |
| Read | `client.get(key)` | `client.get(key).await` | `client.get(key)` |
| Write | `client.set(key, value)` | `client.set(key, value).await` | `client.set(key, value)` |
| Delete | `client.delete(key)` | `client.delete(key).await` | `client.delete(key)` |
| Close | `client.close()` | `client.close().await` | `client.close()` |

The source-built [CLI](cli/README.md) provides the same Gate 0 key and
structured-value profile by default for Bash scripts and interactive use.
Select its `configured` profile when certificate roots, mTLS, client-side value
protection, TTL, or conditional writes are required. Its maintenance commands
remain available only against a full OpenKache server with the matching
experimental API policy; the prototype does not implement them.

## Values and keys

These packages accept typed keys (text, bytes, and integers) and
preserve structured values such as nulls, booleans, integers, floats, text,
bytes, arrays, and maps. Each language maps these values to native types while
also exposing lossless value classes for applications that need exact
representation.

These clients encode values as `StructuredValue-CBOR-v1`, so a value written by
one language can be read losslessly by the others. The package-specific
READMEs describe the supported operations, value types, and connection
behavior for each language.

The package README is the source for language-facing behavior. Shared formats
and interoperability rules are documented separately:

- [Client implementation guide](CLIENT.md)
- [Key format](KEY_FORMAT.md)
- [Value model](value/SPEC.md)
- [Value format](VALUE_FORMAT.md)
- [Security model](../SECURITY_MODEL.md)
- [Protocol specification](../protocol/SPEC.md)
- [Canonical fixtures](fixtures/)

## Other packages

These packages are available for compatibility, native integration, or future
language support. Their package README is the source for the current status.

| Package | Path | Status |
| --- | --- | --- |
| C | [c/](c/) | C17 native adapter |
| C++ | [cpp/](cpp/) | C++20 adapter over the C API |
| C# / .NET | [dotnet/](dotnet/) | Compatibility adapter |
| Go | [go/](go/) | Compatibility adapter |
| Swift | [swift/](swift/) | Compatibility adapter |
| CLI | [cli/](cli/) | Gate 0 command-line client with a configurable compatibility profile |
| Java | `java/` | Scaffold |
| Kotlin | `kotlin/` | Scaffold |
| Dart | `dart/` | Scaffold |

Java, Kotlin, and Dart currently contain package layouts only; they do not
connect to an OpenKache server yet.
