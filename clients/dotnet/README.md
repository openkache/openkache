# OpenKache .NET Client

The OpenKache package is a managed .NET client for the OpenKache protocol v2
server. It uses `System.Net.Quic` directly and does not require the Rust client
library or another native OpenKache binding.

## Purpose

The client provides binary-safe cache operations over one authenticated QUIC
connection. Concurrent operations use a bounded pool of reusable bidirectional
streams so packet loss on one request does not block unrelated requests.

## Build and package

Run these commands from the public repository root:

```bash
dotnet build clients/dotnet/OpenKache/OpenKache.csproj --configuration Release
dotnet pack clients/dotnet/OpenKache/OpenKache.csproj --configuration Release
```

The package targets .NET 8 and opts into the .NET 8 preview QUIC APIs. Windows
ships MsQuic with .NET. Linux installations must make a compatible
`libmsquic` available to the .NET runtime.

## Connect and use

The server creates a self-signed certificate for each run and writes it to the
path selected by `--certificate-out`. Pass those exact DER bytes to the client;
there is no certificate-verification bypass.

```csharp
using System.Text;
using OpenKache;

var certificate = await File.ReadAllBytesAsync(
    "target/openkache-local/certificate.local.der");
await using var client = await Client.ConnectAsync(
    "127.0.0.1",
    4433,
    "localhost",
    certificate);

await client.PingAsync();
var outcome = await client.SetAsync(
    "greeting",
    Encoding.UTF8.GetBytes("hello"),
    new SetOptions
    {
        Condition = SetCondition.IfAbsent,
        TimeToLive = TimeSpan.FromMinutes(5),
    });
var value = await client.GetAsync("greeting");
var statisticsJson = await client.StatsAsync();
await client.SyncAsync();
var deleted = await client.DeleteAsync("greeting");
```

`SetAsync` reports `NotStored` when an `IfAbsent` or `IfPresent` condition
fails; otherwise it reports `Created` or `Replaced`. A positive `TimeToLive` is
rounded up to the next millisecond. Expired keys are treated as missing.
`GetAsync` returns `null` for a missing key, and `DeleteAsync` returns whether
the key existed. String keys are encoded as UTF-8; binary overloads preserve
exact key bytes.

## Protocol and configuration

The connection requires TLS 1.3 and ALPN `openkache/2`. Each key is converted
to its 32-byte SHA-256 protocol digest. Request and response payloads are
limited to 16 MiB.

`ClientOptions` controls the request timeout and maximum number of reusable
stream lanes. Both default to the server-oriented values of 10 seconds and 256
lanes.

This release reads and writes plaintext values. If another SDK stores a value
with OpenKache compression or encryption flags, `GetAsync` reports
`UNSUPPORTED_VALUE_ENCODING` instead of returning encoded bytes as plaintext.
