# OpenKache .NET client

The OpenKache package is a managed .NET adapter over the shared Rust client
core's C ABI.

## Purpose

The package provides binary-safe cache operations over the authenticated
transport selected by `ClientOptions.Transport` (verified QUIC by default or
verified TLS-over-TCP). It accepts exact opaque `0..=32`-byte item IDs and
plaintext values. Framing, TLS, retries, stream lanes, and protocol validation
remain in `clients/core`. `QuicInsecure` and `TlsTcpInsecure` are explicit
TLS-preserving opt-outs that disable certificate and server-identity
verification.

The [client status table](../README.md#sdk-status) describes this package's
implementation and migration status.

## Build and package

Run from the public repository root:

```bash
dotnet build clients/dotnet/OpenKache/OpenKache.csproj --configuration Release
dotnet pack clients/dotnet/OpenKache/OpenKache.csproj --configuration Release
```

The package targets .NET 8. It loads the native core library from
`OPENKACHE_CLIENT_NATIVE` or the platform's normal native-library search path.
Build the shared core with the `ffi` feature before running the package:

```bash
cargo build --manifest-path clients/core/Cargo.toml --features ffi
export OPENKACHE_CLIENT_NATIVE="$PWD/target/debug/libopenkache_client_core.so"
```

Building from source also requires Bun and Smithy CLI on `PATH`; the project
generates ignored wire values and Smithy API contracts before compilation.

## Connect and use

Pass the DER bytes of a server or CA certificate for verified transports. The
insecure selectors explicitly opt out of certificate and server-identity
verification and may omit that buffer.

```csharp
using OpenKache;
using Smithy = OpenKache.Smithy;

var certificate = await File.ReadAllBytesAsync(
    "target/openkache-local/certificate.local.der");
await using var client = await Client.ConnectAsync(
    "127.0.0.1",
    4433,
    "localhost",
    certificate);

await client.PingAsync();
var itemId = new byte[32];
itemId[^1] = 1;
var outcome = await client.SetAsync(
    itemId,
    "hello"u8.ToArray(),
    new SetOptions
    {
        Condition = Smithy.SetCondition.IfAbsent,
        TimeToLive = TimeSpan.FromMinutes(5),
    });
var value = await client.GetAsync(itemId);
var statisticsJson = await client.StatsAsync();
await client.SyncAsync();
var deleted = await client.DeleteAsync(itemId);
```

`SetAsync` returns `NotStored` when a condition fails and `Created` or
`Replaced` after a write. `GetAsync` returns `null` for a missing item ID.
`DeleteAsync` reports whether the item ID existed. Every item-ID-taking operation
accepts and sends exact opaque `0..=32`-byte IDs unchanged.

## Protocol and configuration

This package requires TLS 1.3 and ALPN `openkache/1`. The complete wire
contract is defined by [`protocol/SPEC.md`](../../protocol/SPEC.md).

`ClientOptions` controls connection and request deadlines plus maximum reusable
request lanes. Defaults come from the Smithy client-defaults contract: 5
seconds, 2 seconds, and 256 lanes. Formatted-value writes use automatic level-1
Zstandard compression by default; set `ClientOptions.CompressionEnabled` to
`false` for an explicit uncompressed opt-out. `OperationTimeout` remains as a
legacy compatibility alias for callers that need one deadline for both phases.

The generated Smithy operation, input, output, and enum types under
`OpenKache.Smithy` are the current transitional .NET API types. `StatsAsync`
and `SyncAsync` are transitional experimental maintenance operations and are
disabled by default. Enable `enable_experimental_api = true` explicitly and
coordinate exact revision `draft-2026-08-19.4` out of band as described in
[`protocol/EXPERIMENTAL.md`](../../protocol/EXPERIMENTAL.md) before calling
them; the revision is not negotiated on the wire. Generated
namespace-management shapes are out-of-band WIP control-plane operations.
The
`OpenKache.SetCondition` and `OpenKache.SetOutcome` members remain compatibility
aliases for earlier callers, and their values have those generated types.
`SetOptions.Condition` and `SetAsync` return types use the generated shapes
directly.

The package reads and writes plaintext values. It does not implement the
[shared formatted value contract](../VALUE_FORMAT.md).

The client exposes the raw Smithy API. Protected value handling remains in the
shared Rust core and is not part of this package.
