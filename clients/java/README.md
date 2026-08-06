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

Maven regenerates the Smithy-derived DTOs, complete operation signatures,
native constants, JNA declarations, and the namespace descriptor layout before
compiling the adapter. The generated sources live under
`src/main/java/io/openkache/client/generated_local/` and are not edited
manually.

Build the native library with the `ffi` feature before connecting:

```bash
cargo build --manifest-path clients/core/Cargo.toml --features ffi
export OPENKACHE_CLIENT_NATIVE="$PWD/target/debug/libopenkache_client_core.so"
```

## Usage

```java
import io.openkache.client.Client;
import io.openkache.client.GetInput;

try (var client = Client.connect(
        "cache.example.com:4433",
        "cache.example.com",
        certificateDerOrPem,
        dataProtectionKey)) {
    long namespaceId = 1L;
    byte[] itemId = new byte[32]; // exact protocol item ID
    var value = client.get(new GetInput(namespaceId, itemId))
        .toCompletableFuture()
        .join()
        .value();
}
```

`dataProtectionKey` must contain 32 bytes. The shared core requires a valid
client key when opening a protected connection. `Client` implements every generated
`SmithyOpenKacheApi` method, including exact-item data operations and namespace
management.
