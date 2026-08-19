# OpenKache .NET client

The OpenKache package is a managed .NET adapter over the shared Rust client
core's C ABI.

## Purpose

The package provides binary-safe cache operations over one authenticated QUIC
connection owned by the shared core. It currently accepts the legacy exact
32-byte item-ID API and plaintext values; the draft wire contract targets
`0..=32`-byte Item IDs. Framing, TLS, retries, stream lanes, and protocol
validation remain in `clients/core`.

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

Pass the DER bytes of a server or CA certificate; the client has no
certificate-verification bypass.

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
currently requires exactly 32 bytes and sends them unchanged. The draft wire
contract targets `0..=32`-byte Item IDs; this package has not migrated yet.

## Protocol and configuration

This package requires TLS 1.3 and ALPN `openkache/1`. The complete wire
contract is defined by [`protocol/SPEC.md`](../../protocol/SPEC.md).

`ClientOptions` controls connection and request deadlines plus maximum reusable
request lanes. Defaults come from the Smithy client-defaults contract: 5
seconds, 2 seconds, and 256 lanes. `OperationTimeout` remains as a legacy
compatibility alias for callers that need one deadline for both phases.

The generated Smithy operation, input, output, and enum types under
`OpenKache.Smithy` are the canonical .NET API types. The
`OpenKache.SetCondition` and `OpenKache.SetOutcome` members remain compatibility
aliases for earlier callers, and their values have those generated types.
`SetOptions.Condition` and `SetAsync` return types use the generated shapes
directly.

The package reads and writes plaintext values. It does not implement the
[shared formatted value contract](../VALUE_FORMAT.md).

The client exposes the raw Smithy API. Protected value handling remains in the
shared Rust core and is not part of this package.
