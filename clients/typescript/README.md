# OpenKache TypeScript Client

The TypeScript package is the canonical SDK for TypeScript and JavaScript
applications. It is currently a thin Bun wrapper over the production Rust
client.
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
import { ProfileSchema } from "./gen/profile_pb.ts"

const client = await OpenKache_Client.connect({
  address: "127.0.0.1:4433",
  certificate: await Bun.file("certificate.der").bytes(),
  encryption_key: crypto.getRandomValues(new Uint8Array(32)),
})

await client.set("profile", {
  name: "Kim",
  visits: 42,
  labels: ["subscriber", "beta"],
}, ProfileSchema)
const profile = await client.get("profile", ProfileSchema)

await client.setRaw("opaque", Uint8Array.of(1, 2, 3))
const bytes = await client.getRaw("opaque")
await client.close()
```

`set` and `get` use Protobuf-ES internally. Define values in a shared `.proto`
file, generate TypeScript with `@bufbuild/protoc-gen-es`, and pass the generated
message schema to both methods. The schema infers the TypeScript initializer and
result types and also performs the actual runtime encoding and decoding.
Applications import `OpenKache_Client` and their generated schema; they do not
need to call `create`, `toBinary`, or `fromBinary`.

Generate each language SDK from the same `.proto` definitions. A TypeScript
client can therefore encode a value that Java, C++, Python, or Rust later
decodes with its generated type, and the reverse direction works the same way.
The Rust transport treats those Protobuf bytes as opaque values; it only
compresses, encrypts, and sends them. The server does not parse Protobuf,
decrypt, or decompress stored values.

Use `setRaw` and `getRaw` when the application already owns Protobuf bytes from
another runtime or needs an exact byte-for-byte round trip.

All connection and cache methods return promises. A small Bun worker owns the
synchronous FFI handle so native networking does not block the application's
main JavaScript thread. Bun FFI is currently experimental, so this package is
a preview. The native client reuses one QUIC connection. Call and await
`close()` when finished.

## Configuration

- `address` is the server UDP address.
- `certificate` is the trusted DER certificate.
- `encryption_key` is an application-managed 32-byte secret. Clients sharing
  values must use the same key. OpenKache never sends it to the server.
- `compression` controls Zstandard level, minimum input size, and required
  savings. Defaults favor low memory use with level 1.
- `timeouts.connect_ms` bounds endpoint setup and the QUIC/TLS handshake;
  the default is 5000 ms.
- `timeouts.request_ms` bounds each complete request/response operation;
  the default is 2000 ms.
- `library_path` overrides the native library location for packaged builds.

`PING`, `GET`, and `STATS` may reconnect and retry once after a transport
failure. Mutating operations are not retried automatically. Encoded values are
limited to the 64 MiB wire ceiling; servers may enforce a smaller operational
limit. Operations are serialized through the native worker; `close()` rejects
operations that have not started and waits for at most the current bounded
operation.

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
