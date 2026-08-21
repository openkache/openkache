# OpenKache Swift client

The Swift package is a thin actor-based adapter over the shared Rust client
core. It accepts Foundation `Data`, exposes async cache operations, and does
not duplicate QUIC/TCP framing, TLS validation, retries, key derivation,
compression, encryption, or value parsing. `OpenKacheClientOptions.transport`
selects verified QUIC (the default), verified TLS-over-TCP, or an explicit
TLS-preserving insecure selector.

This README documents the current transitional Swift/FFI API. The target
variable-width Item ID, structured-value, compression, and dual-transport
contracts live in the shared draft documents linked by [`../README.md`](../README.md).

`OpenKacheClient` derives protected item IDs from v1 PortableKey values. `String`
keys use the `Text` type and `Data` keys use the `Bytes` type; both are encoded
as canonical deterministic CBOR before crossing the native ABI. Use
`OpenKacheRawClient` when an integration owns exact protocol item IDs and
opaque value bytes; it implements the generated `Smithy_OpenKache_Api`
contract.

The native library exports the versioned ABI declared in
[`../core/include/openkache/client_abi.h`](../core/include/openkache/client_abi.h).
Build or install the `openkache-client-core` Rust `cdylib` for the target platform
and make it visible to the linker as `openkache_client_core`.

The generated `STATS` and `SYNC` methods are transitional experimental
maintenance operations and are disabled by default. Enable
`enable_experimental_api = true` explicitly and coordinate exact revision
`draft-2026-08-19.4` out of band as described in
[`protocol/EXPERIMENTAL.md`](../../protocol/EXPERIMENTAL.md) before calling
them; the revision is not negotiated on the wire. Namespace lifecycle methods
in the raw example are out-of-band WIP control-plane shapes, not stable-v1
data-plane operations.

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

For an exact protocol item ID, connect the raw adapter:

```swift
let raw = try await OpenKacheRawClient.connect(options: options)
let itemID = Data(repeating: 0x42, count: Smithy_Value_Format.itemIdBytes)
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
Empty values are valid for the protected adapter. Raw item IDs accept
`0...Smithy_Value_Format.itemIdBytes` opaque bytes. `dataProtectionKey` must
contain exactly 32 persistent random bytes.
`certificate` may be one DER certificate or a PEM chain; omit it to use system
roots. A numeric address may provide a separate `serverName` for certificate
verification.

Formatted writes use automatic level-1 Zstandard compression by default and
retain a completed frame only when it is smaller. Pass
`compression: .disabled` to `OpenKacheClientOptions` for an explicit
uncompressed opt-out.

The Smithy operation, value-format, connection-state, and native ABI
declarations are generated into SwiftPM's build directory from
[`../model/openkache.smithy`](../model/openkache.smithy) and the wire model in
[`../../protocol/model/openkache.smithy`](../../protocol/model/openkache.smithy).
They are not checked into source control: the `GenerateSmithy` SwiftPM build
plugin regenerates the current transitional contract for every build. The
draft protocol and client-format documents remain the target sources of truth
until migration is complete. The shared C ABI header consumes the same
generated current contract.
To regenerate the declarations explicitly:

```bash
env OPENKACHE_GENERATION_TARGET=swift bun ../generate.ts
```
