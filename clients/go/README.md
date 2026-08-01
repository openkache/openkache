# OpenKache Go client

The Go package exposes context-aware cache operations while delegating QUIC,
TLS, retries, key derivation, compression, encryption, and value limits to the
shared `openkache-client-core` native ABI. The package contains no duplicate
wire or cryptographic implementation.

## Commands

From the repository root, regenerate the native contract header before a CGO
build. The header is a build artifact, not a hand-maintained Go constant file:

```bash
OPENKACHE_GENERATION_TARGET=c-contract ./openkache/clients/generate.ts
```

Then from `clients/go`:

```bash
go vet ./...
go test ./...
go build ./...
```

The package supports both CGO builds and `CGO_ENABLED=0` cross-compilation.
Runtime connections require CGO and a native library built from the Rust
client with the `ffi` feature. The default native runtime uses portable
Tokio/Quinn on Linux, macOS, and Windows; the optional Compio/io_uring feature
remains available for Linux fast-path builds. `CGO_ENABLED=0` remains useful
for packaging and cross-compilation but returns an explicit unsupported-runtime
error at connect time:

```bash
cargo build --manifest-path ../core/Cargo.toml \
  --no-default-features --features ffi
```

Set `OPENKACHE_CLIENT_LIBRARY` to the resulting
`libopenkache_client.so`/platform equivalent, or set
`Options.NativeLibrary` explicitly.

## Usage

```go
import (
    "context"

    openkache "github.com/openkache/openkache/clients/go"
)

ctx := context.Background()
client, err := openkache.Connect(ctx, openkache.Options{
    Address:           "cache.example.com:4433",
    ServerName:        "cache.example.com",
    Certificate:       caCertificateDER,
    DataProtectionKey: dataProtectionKey, // exactly 32 bytes
})
if err != nil {
    return err
}
defer client.Close()

outcome, err := client.Set(ctx, []byte("profile"), []byte("value"), openkache.SetOptions{
    Condition: openkache.IfAbsent,
})
if err != nil {
    return err
}
_ = outcome

value, found, err := client.Get(ctx, []byte("profile"))
```

`Get` returns `found` separately so an empty stored value is not confused with
a cache miss. `Close` is idempotent and waits for in-flight native operations.
`Reconnect` explicitly replaces the connection without replaying an operation,
and `ConnectionState` returns a best-effort lifecycle snapshot.
`GetJSON` and `SetJSON` delegate JSON parsing and RFC 8785 canonicalization to
the shared core; they accept and return complete JSON documents as bytes.
`NewItemID`, `GetItem`, `SetItem`, and `DeleteItem` expose the exact wire
item-ID/raw-value layer when an application already owns protocol IDs.
Use `client.Smithy()` when an application needs the generated
`SmithyOpenKacheAPI` operation structures shared with other bindings.

## Configuration

- `Certificate` accepts one DER certificate or a PEM trust chain.
- `Identity` optionally supplies a DER/PEM client certificate chain and private
  key for mutual TLS.
- `Compression`, `Timeouts`, `Retry`, and `MaxInFlight` map directly to core
  settings; zero values select documented core defaults.
- `EncryptionCompact` selects deterministic AES-256-SIV-CMAC protection;
  `EncryptionRobust` (the default) selects randomized AES-256-GCM-SIV.
- `OPENKACHE_CLIENT_LIBRARY` or `Options.NativeLibrary` selects the native
  artifact. The native artifact must have ABI version 3 and the extended
  connect symbol when `Identity` is used.

Protocol operations, Smithy models, and value-format identifiers are generated
from [`../model/openkache.smithy`](../model/openkache.smithy) and
[`../../protocol/model/openkache.smithy`](../../protocol/model/openkache.smithy).
The generated Go files are checked in for module consumers; the C contract
header is emitted into `core/generated_local/` and is supplied to CGO via the
package include path.

When using a pre-ABI-extension native library, `Identity`, `EncryptionCompact`,
non-default `Retry.MaxAttempts`, and non-default `MaxInFlight` require upgrading
the native library to one that exports `openkache_client_connect_ex`.
