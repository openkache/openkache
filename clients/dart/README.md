# OpenKache Dart client

This package is an experimental Dart adapter for the complete generated
OpenKache Smithy API. It uses `dart:ffi` to call the shared Rust client-core C
ABI; Dart does not reimplement QUIC, TLS, framing, retries, namespace handling,
or result ownership.

## Commands

The package source imports Smithy-derived DTOs, operation signatures, and
native constants. Regenerate them from the canonical model before running the
Dart commands:

```bash
OPENKACHE_GENERATION_TARGET=dart ../generate.ts
dart pub get
dart analyze
```

Generated sources are under `lib/generated_local/` and must not be edited
manually.

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

`dataProtectionKey` must contain 32 bytes. All cache operations use the same
generated DTOs and ABI boundary. `EchoClient` implements every generated
`SmithyOpenKacheApi` method, including exact-item data operations and namespace
management.
