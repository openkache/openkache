# OpenKache Java client

This package is an experimental Java adapter for the complete generated
OpenKache Smithy API. It uses JNA to call the shared Rust client-core C ABI;
QUIC, TLS, framing, retries, namespace handling, and native result ownership
stay in the core.

## Commands

Run from the public repository root:

```bash
mvn -f clients/java/pom.xml package
```

Maven regenerates the Smithy-derived DTOs, complete operation signatures, and
native constants before compiling the adapter. The generated sources live
under `src/main/java/io/openkache/client/generated_local/` and are not edited
manually.

Build the native library with the `ffi` feature before connecting:

```bash
cargo build --manifest-path clients/core/Cargo.toml --features ffi
export OPENKACHE_CLIENT_NATIVE="$PWD/target/debug/libopenkache_client_core.so"
```

## Usage

```java
import io.openkache.client.GetInput;

try (var client = EchoClient.connect(
        "cache.example.com:4433",
        "cache.example.com",
        certificateDerOrPem,
        dataProtectionKey)) {
    var echoed = client.echo("single-source-of-truth")
        .toCompletableFuture()
        .join();
    long namespaceId = 1L;
    byte[] itemId = new byte[32]; // exact protocol item ID
    var value = client.get(new GetInput(namespaceId, itemId))
        .toCompletableFuture()
        .join()
        .value();
    System.out.println(echoed);
}
```

`dataProtectionKey` must contain 32 bytes. The `ECHO` operation itself does
not protect the message, but the shared core requires a valid client key when
opening a protected connection. `EchoClient` implements every generated
`SmithyOpenKacheApi` method, including exact-item data operations and namespace
management.
