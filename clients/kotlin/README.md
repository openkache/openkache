# OpenKache Kotlin client

This package is an experimental Kotlin adapter for the complete generated
OpenKache Smithy API. It calls the shared Rust client-core C ABI through JNA
and moves the blocking native call to `Dispatchers.IO`.

## Commands

Run from the public repository root:

```bash
gradle --project-dir clients/kotlin build
```

Gradle regenerates the Smithy-derived DTOs, complete operation signatures,
native constants, JNA declarations, and the namespace descriptor layout before
compiling the adapter. The generated sources live under
`src/main/kotlin/io/openkache/client/generated_local/` and are not edited
manually.

Build the native library with the `ffi` feature before connecting:

```bash
cargo build --manifest-path clients/core/Cargo.toml --features ffi
export OPENKACHE_CLIENT_NATIVE="$PWD/target/debug/libopenkache_client_core.so"
```

## Usage

```kotlin
val client = Client.connect(
    address = "cache.example.com:4433",
    serverName = "cache.example.com",
    certificate = certificateDerOrPem,
    dataProtectionKey = dataProtectionKey,
)
try {
    // Invoke generated Smithy operations here.
} finally {
    client.close()
}
```

`dataProtectionKey` must contain 32 bytes. All cache operations use the same
generated DTOs and ABI boundary. `Client` implements every generated
`SmithyOpenKacheApi` method, including exact-item data operations and namespace
management.
