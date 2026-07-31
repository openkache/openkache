# OpenKache .NET client

The OpenKache package is a managed .NET client that uses `System.Net.Quic`
directly.

## Purpose

The package provides binary-safe cache operations over one authenticated QUIC
connection. It accepts exact 32-byte item IDs and plaintext values and uses a
bounded pool of reusable bidirectional streams.

The [client status table](../README.md#sdk-status) describes this package's
implementation and migration status.

## Build and package

Run from the public repository root:

```bash
dotnet build clients/dotnet/OpenKache/OpenKache.csproj --configuration Release
dotnet pack clients/dotnet/OpenKache/OpenKache.csproj --configuration Release
```

The package targets .NET 8 and opts into its preview QUIC APIs. Windows ships
MsQuic with .NET. Linux must make a compatible `libmsquic` available to the
runtime.

## Connect and use

Pass the exact DER bytes of the server's certificate; the client has no
certificate-verification bypass.

```csharp
using OpenKache;

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
        Condition = SetCondition.IfAbsent,
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
requires exactly 32 bytes and sends them unchanged.

## Protocol and configuration

This package requires TLS 1.3 and ALPN `openkache/1`. The complete wire
contract is defined by [`protocol/SPEC.md`](../../protocol/SPEC.md).

`ClientOptions` controls the request timeout and maximum reusable stream lanes.
The defaults are 10 seconds and 256 lanes.

The package reads and writes plaintext values. It does not implement the
[shared formatted value contract](../VALUE_FORMAT.md).

The client currently owns a small raw protocol adapter. Protected value
handling remains in the shared Rust core and is not part of this package.
