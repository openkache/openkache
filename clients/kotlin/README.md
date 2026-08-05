# OpenKache Kotlin ECHO client

This package is an experimental Kotlin adapter for the Smithy `ECHO`
operation. It calls the shared Rust client-core C ABI through JNA and moves
the blocking native call to `Dispatchers.IO`.

## Commands

Run from the public repository root:

```bash
gradle --project-dir clients/kotlin build
```

Gradle regenerates the Smithy-derived native constants before compiling the
adapter.

Build the native library with the `ffi` feature before connecting:

```bash
cargo build --manifest-path clients/core/Cargo.toml --features ffi
export OPENKACHE_CLIENT_NATIVE="$PWD/target/debug/libopenkache_client_core.so"
```

## Usage

```kotlin
val client = EchoClient.connect(
    address = "cache.example.com:4433",
    serverName = "cache.example.com",
    certificate = certificateDerOrPem,
    dataProtectionKey = dataProtectionKey,
)
try {
    val echoed = client.echo("single-source-of-truth")
} finally {
    client.close()
}
```

`dataProtectionKey` must contain 32 bytes. The remaining cache operations will
reuse this ABI boundary as they are added to the Kotlin package.
