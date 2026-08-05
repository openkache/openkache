# OpenKache Java ECHO client

This package is an experimental Java adapter for the Smithy `ECHO` operation.
It uses JNA to call the shared Rust client-core C ABI; QUIC, TLS, framing,
retries, and native result ownership stay in the core.

## Commands

Run from the public repository root:

```bash
mvn -f clients/java/pom.xml package
```

Maven regenerates the Smithy-derived native constants before compiling the
adapter.

Build the native library with the `ffi` feature before connecting:

```bash
cargo build --manifest-path clients/core/Cargo.toml --features ffi
export OPENKACHE_CLIENT_NATIVE="$PWD/target/debug/libopenkache_client_core.so"
```

## Usage

```java
try (var client = EchoClient.connect(
        "cache.example.com:4433",
        "cache.example.com",
        certificateDerOrPem,
        dataProtectionKey)) {
    var echoed = client.echo("single-source-of-truth")
        .toCompletableFuture()
        .join();
    System.out.println(echoed);
}
```

`dataProtectionKey` must contain 32 bytes. The `ECHO` operation itself does
not protect the message, but the shared core requires a valid client key when
opening a protected connection.
