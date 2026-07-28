# OpenKache TypeScript Client

The TypeScript client is a thin Bun wrapper over the production Rust client.
It uses the same QUIC transport, wire protocol, Zstandard codec, and
XChaCha20-Poly1305 value encryption as Rust instead of maintaining a second
protocol implementation.

## Purpose

Applications can use OpenKache from TypeScript while keeping protocol and
security behavior in one Rust implementation. Values are compressed when that
reduces their size, then encrypted before leaving the client. The server stores
the resulting bytes without parsing, decrypting, or decompressing them.

## Commands

From `clients/typescript`:

```bash
bun run build:native
```

## Usage

```typescript
import { OpenKache_Client } from "@openkache/client"

const client = OpenKache_Client.connect({
  address: "127.0.0.1:4433",
  certificate: await Bun.file("certificate.der").bytes(),
  encryption_key: crypto.getRandomValues(new Uint8Array(32)),
})

client.set("greeting", "hello")
const value = client.get("greeting")
client.close()
```

The current methods are synchronous because Bun's C ABI interface is
synchronous. Bun FFI is currently an experimental Bun interface, so this
package is a preview. The native client keeps one Rust worker thread and one
reusable QUIC connection. Call `close()` when finished.

## Configuration

- `address` is the server UDP address.
- `certificate` is the trusted DER certificate.
- `encryption_key` is an application-managed 32-byte secret. Clients sharing
  values must use the same key. OpenKache never sends it to the server.
- `compression` controls Zstandard level, minimum input size, and required
  savings. Defaults favor low memory use with level 1.
- `library_path` overrides the native library location for packaged builds.

Stored encrypted values contain only a 24-byte nonce, ciphertext, and a 16-byte
authentication tag. Existing request, response, and on-disk metadata fields
carry the compression and encryption bits without adding bytes to the value.
Encryption therefore adds exactly 40 bytes with no magic, version,
original-length field, or padding. The flags are authenticated together with
the cache-key digest.

The native library uses a bounded request queue. Owned Rust buffers are reused
for compression, encryption, request framing, response decoding, and
uncompressed decryption where possible. Compressed reads require a second
buffer sized from the authenticated Zstandard frame content size.
