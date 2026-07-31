# OpenKache Swift client

The Swift package is a thin actor-based adapter over the shared Rust client
core. It accepts Foundation `Data`, exposes async cache operations, and does
not duplicate QUIC framing, TLS validation, retries, key derivation,
compression, encryption, or value parsing.

`OpenKacheClient` derives protected item IDs from application keys. Use
`OpenKacheRawClient` when an integration owns exact protocol item IDs and
opaque value bytes; it implements the generated `Smithy_OpenKache_Api`
contract.

The native library exports the versioned ABI declared in
[`../core/include/openkache/client_abi.h`](../core/include/openkache/client_abi.h).
Build or install the `openkache-client` Rust `cdylib` for the target platform
and make it visible to the linker as `openkache_client_core`.

## Commands

From this directory:

```bash
swift build
```

When the Rust library is in a non-standard directory, pass its search path:

```bash
swift build -Xlinker -L/path/to/native-library
```

The Rust library can be built from the public workspace with the `ffi` feature:

```bash
env -u CARGO_BUILD_TARGET cargo build \
  --manifest-path ../../Cargo.toml \
  -p openkache-client-core \
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
    dataProtectionKey: key,
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

For an exact protocol item ID, connect the raw adapter or derive one from a
protected client:

```swift
let raw = try await OpenKacheRawClient.connect(options: options)
let itemID = Data(repeating: 0x42, count: Smithy_Value_Format.itemIdBytes)
let output = try await raw.set(
    itemID,
    value: Data("opaque".utf8),
    options: OpenKacheSetOptions(condition: .ifAbsent)
)
let smithyOutput = try await raw.get(Smithy_Get_Input(itemId: itemID))
_ = (output, smithyOutput)
```

Keys are exact UTF-8 or binary bytes and must not be empty. Empty values are
valid for the protected adapter. Raw item IDs must contain exactly
`Smithy_Value_Format.itemIdBytes` bytes. `dataProtectionKey` must contain
exactly 32 persistent random bytes.
`certificate` may be one DER certificate or a PEM chain; omit it to use system
roots. A numeric address may provide a separate `serverName` for certificate
verification.

The Smithy operation, value-format, connection-state, and native ABI
declarations are generated into `Sources/OpenKache/Generated/SmithyAPI.swift`
from [`protocol/model/openkache.smithy`](../../protocol/model/openkache.smithy).
The generated declarations and shared C ABI header are outputs only; the
Smithy model is the single source of truth for operation, state, result,
limit, and value-format identifiers.
Regenerate them with:

```bash
env OPENKACHE_GENERATION_TARGET=swift bun ../../protocol/generate.ts
```
