# OpenKache Swift client

The Swift package is a thin actor-based adapter over the shared Rust client
core. It accepts Foundation `Data`, exposes async cache operations, and does
not duplicate QUIC framing, TLS validation, retries, key derivation,
compression, encryption, or value parsing.

`OpenKacheClient` derives protected item IDs from v1 typed-key values. `String`
keys use the `Text` type and `Data` keys use the `Bytes` type; the logical bytes
and explicit type discriminator cross the native ABI and the shared core
performs canonical encoding (or the configured `ByteKeyOrHash` mapping). Use
`OpenKacheRawClient` when an integration owns exact protocol item IDs and
opaque value bytes; it implements the generated `Smithy_OpenKache_Api`
contract.

The native library exports the versioned ABI declared in
[`../core/include/openkache/client_abi.h`](../core/include/openkache/client_abi.h).
Build or install the `openkache-client-core` Rust `cdylib` for the target platform
and make it visible to the linker as `openkache_client_core`.

## Commands

From this directory:

```bash
swift build
```

When the Rust library is in a non-standard directory, pass its search path:

```bash
swift build -Xlinker -L./path/to/native-library
```

The Rust core library can be built from the public client workspace with the
`ffi` feature:

```bash
env -u CARGO_BUILD_TARGET cargo build \
  --manifest-path ../core/Cargo.toml \
  --no-default-features \
  --features ffi
```

## Usage

```swift
import Foundation
import OpenKache

let key = Data(repeating: 0x42, count: 32)
let options = OpenKacheClientOptions(
    address: "cache.example.com:4433",
    clientRootKey: key,
    certificate: try Data(contentsOf: certificateURL)
)

let client = try await OpenKacheClient.connect(options: options)
try await client.set(
    "session",
    value: Data("hello".utf8),
    options: OpenKacheSetOptions(
        condition: .ifAbsent,
        expiresAfter: .seconds(300)
    )
)
let value = try await client.get("session")
let statisticsJSON = try await client.stats()
try await client.sync()
await client.close()
```

For an exact protocol item ID, connect the raw adapter:

```swift
let raw = try await OpenKacheRawClient.connect(options: options)
let itemID = Data(repeating: 0x42, count: Smithy_Value_Format.maxItemIdBytes)
let namespace = try await raw.namespaceOpen(
    Smithy_Namespace_Open_Input(
        name: "example",
        createIfMissing: true,
        policy: Smithy_Namespace_Policy(
            defaultExpiration: .noExpiry,
            expirationOverride: .allowed,
            defaultEviction: .evictable,
            evictionOverride: .allowed
        )
    )
)
let output = try await raw.set(
    Smithy_Set_Input(
        namespaceId: namespace.descriptor.namespaceId,
        itemId: itemID,
        value: Data("opaque".utf8),
        condition: .ifAbsent
    )
)
let smithyOutput = try await raw.get(
    Smithy_Get_Input(namespaceId: namespace.descriptor.namespaceId, itemId: itemID)
)
_ = (output, smithyOutput)
```

Keys are exact UTF-8 or binary bytes, including empty and NUL-containing keys.
Empty values are valid for the protected adapter. Raw item IDs may contain zero
through `Smithy_Value_Format.maxItemIdBytes` bytes. `clientRootKey` must contain
exactly 32 persistent random bytes.
Compression is disabled by default (`OpenKacheCompression.disabled`). Pass
`OpenKacheCompression.zstandard()` to enable Zstandard with the shared defaults:
level `1`, minimum input `1,024` bytes, and minimum savings `64` bytes.
When `encryption` is omitted, the shared core selects Robust with a
`clientRootKey` and Unprotected without one. Use `.unprotected` to explicitly
disable value protection while retaining root-key-bound Item IDs. Explicit
Compact or Robust requires a client root key.
`OpenKacheKeyFormat.byteKeyOrHash` preserves `Data` keys up to the 32-byte Item
ID limit and hashes longer keys; use it only with byte-key APIs.
`certificate` may be one DER certificate or a PEM chain; omit it to use system
roots. A numeric address may provide a separate `serverName` for certificate
verification.

The Smithy operation, value-format, connection-state, and native ABI
declarations are generated into SwiftPM's build directory from
[`../model/openkache.smithy`](../model/openkache.smithy) and the wire model in
[`../../protocol/model/openkache.smithy`](../../protocol/model/openkache.smithy).
They are not checked into source control: the `GenerateSmithy` SwiftPM build
plugin regenerates them for every build. These two Smithy models remain the
scoped sources of truth for operation, state, result, limit, and value-format
identifiers. The shared C ABI header consumes the same generated contract.
To regenerate the declarations explicitly:

```bash
env OPENKACHE_GENERATION_TARGET=swift bun ../generate.ts
```
