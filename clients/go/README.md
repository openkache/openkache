# OpenKache Go client

The Go package exposes context-aware cache operations while delegating
TLS 1.3 over QUIC or TCP, retries, key derivation, compression, encryption,
and value limits to the shared `openkache-client-core` native ABI. The package
contains no duplicate wire or cryptographic implementation. Set
`Options.Transport` to `TransportTlsTcp` for verified TLS-over-TCP or to an
explicit insecure selector when certificate and server-identity verification
is intentionally disabled; the zero value remains verified QUIC.

This package is a compatibility adapter over the internal native ABI; it is not
the maintained five-operation Gate 0 facade. Its generated Smithy, JSON, and
exact-item-ID/raw methods are retained for integrations that already use this
adapter and MUST NOT be presented as the public Rust `openkache` API. The
frozen protocol, key, value, and security specifications remain normative.

## Commands

From `clients/go`, generate the Smithy Go API and contract before compiling the
package. The generated files are build artifacts and are intentionally ignored
by Git:

```bash
go generate
```

Then from `clients/go`:

```bash
go vet ./...
go build ./...
```

The package supports both CGO builds and `CGO_ENABLED=0` cross-compilation.
Runtime connections require CGO and a native library built from the Rust
client with the `ffi` feature. The current native runtime uses Compio's
io_uring backend on Linux; `CGO_ENABLED=0` remains useful for packaging and
cross-compilation but returns an explicit unsupported-runtime error at connect
time:

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
    DataProtectionKey: dataProtectionKey, // optional; exactly 32 bytes when supplied
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

`Get` and `Set` treat the `[]byte` key as a v1 `Bytes` PortableKey and pass
its logical bytes with the generated key discriminator; the shared core
performs canonical deterministic CBOR encoding. `Get`
returns `found` separately so an empty stored value is not confused with a
cache miss. `Close` is idempotent and waits for in-flight native operations.
`Reconnect` explicitly replaces the connection without replaying an operation,
and `ConnectionState` returns a best-effort lifecycle snapshot.
`GetJSON` and `SetJSON` delegate JSON parsing and RFC 8785 canonicalization to
the shared core; they accept and return complete JSON documents as bytes.
`NewItemID`, `GetItem`, `SetItem`, and `DeleteItem` expose the exact wire
item-ID/raw-value layer when an application already owns protocol IDs.
`ItemID` preserves the exact `0..=32` opaque bytes supplied by the caller.
Use `client.Smithy()` when an application needs the generated
`SmithyOpenKacheAPI` operation structures shared with other bindings.

Protected and legacy raw operations use the ABI v1 request handle lifecycle
when an async entry point carries their options. A context cancellation before
native admission returns the context error; cancellation after a mutation has
started preserves `ErrUnknownMutation` and never replays the mutation. Complete
raw SET policy flags, structured calls, scoped calls, and namespace control
operations do not have request-handle entry points in ABI v1, so the adapter
drains a safe synchronous completion boundary before returning.

`Close` is the normative lifecycle boundary: call it explicitly and observe
its returned error after admitted operations drain. If a `Client` becomes
unreachable first, the Go runtime may invoke a best-effort finalizer that
discards shutdown errors; finalization is nondeterministic and may not run, so
it is not a replacement for `Close`.

## Configuration

- `Certificate` accepts one DER certificate or a PEM trust chain.
- `Identity` optionally supplies a DER/PEM client certificate chain and private
  key for mutual TLS.
- `Compression`, `Timeouts`, `Retry`, and `MaxInFlight` map directly to core
  settings; the zero `CompressionOptions` value selects automatic level-1
  Zstandard with no input-size or minimum-savings threshold. Set
  `CompressionOptions.Disabled` to opt out explicitly.
- `EncryptionCompact` selects deterministic AES-256-SIV-CMAC protection;
  `EncryptionRobust` (the default) selects randomized AES-256-GCM-SIV.
- An empty `DataProtectionKey` selects unprotected values while retaining
  client-side Item ID derivation.
- `OPENKACHE_CLIENT_LIBRARY` or `Options.NativeLibrary` selects the native
  artifact. The native artifact must have ABI version 1 and the extended
  connect symbol when `Identity` is used.

Protocol operations, Smithy models, and value-format identifiers are generated
from [`../model/openkache.smithy`](../model/openkache.smithy) and
[`../../protocol/model/openkache.smithy`](../../protocol/model/openkache.smithy).
`go generate` writes `smithy_api.go` and `smithy_contract.go` beside the
handwritten adapter sources; both files are ignored and must never be staged.
The C contract header is emitted into `core/generated_local/` and is supplied
to CGO via the package include path.

When using a pre-ABI-extension native library, `Identity`, `EncryptionCompact`,
non-default `Retry.MaxAttempts`, and non-default `MaxInFlight` require upgrading
the native library to one that exports `openkache_client_connect_ex`.
