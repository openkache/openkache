# OpenKache Dart ECHO client

This package is an experimental Dart adapter for the Smithy `ECHO` operation.
It uses `dart:ffi` to call the shared Rust client-core C ABI; Dart does not
reimplement QUIC, TLS, framing, retries, or result ownership.

## Commands

The package source imports the Smithy-derived native constants. Regenerate them
from the canonical model before running the Dart commands:

```bash
OPENKACHE_GENERATION_TARGET=dart ../generate.ts
dart pub get
dart analyze
```

Build the native library with the `ffi` feature before connecting:

```bash
cargo build --manifest-path ../core/Cargo.toml --features ffi
export OPENKACHE_CLIENT_NATIVE="$PWD/../core/target/debug/libopenkache_client_core.so"
```

## Usage

```dart
final client = EchoClient.connect(
  address: 'cache.example.com:4433',
  serverName: 'cache.example.com',
  certificate: certificateDerOrPem,
  dataProtectionKey: dataProtectionKey,
);
try {
  final echoed = await client.echoMessage('single-source-of-truth');
} finally {
  client.close();
}
```

`dataProtectionKey` must contain 32 bytes. The remaining cache operations will
reuse this ABI boundary as they are added to the Dart package.
