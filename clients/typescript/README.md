# OpenKache TypeScript Client

`@openkache/client` is the TypeScript and JavaScript SDK for Node.js, Bun, and
Deno applications. It delegates QUIC, mutual TLS, compression, and value encryption
to `openkache-client-core` through a packaged Node-API adapter. Applications
do not need runtime npm dependencies, Bun-specific APIs, or a helper process.

## Purpose

Applications use regular JavaScript objects while OpenKache keeps envelope
framing, transport, and security behavior in one Rust implementation. JavaScript
codecs produce metadata and payload bytes; the adapter passes them to the shared
core's [OpenKache value envelope](../VALUE_FORMAT.md) implementation and delegates
the resulting plaintext envelope to `ProtectedClient`. The core then compresses
beneficial values and encrypts every value before it leaves the client. The
server stores opaque bytes without parsing, decrypting, or decompressing them.

## Commands

From `clients/typescript`:

```bash
bun install --frozen-lockfile
bun run build
bun run typecheck
bun run pack:check
```

`build` generates the JavaScript and declaration files under the ignored
`dist/` directory. `pack:check` also cross-compiles the Rust Node-API adapter and
previews the complete npm package. Generated output is not committed.

The repository uses Bun only for development and release tooling. Published
applications can use Node.js 20 or newer, Bun's Node-API support, or Deno's Node
compatibility layer with `--allow-ffi`. Release packages contain Linux x64 and
ARM64 adapters under `target/native/` and require glibc 2.17 or newer.

## Usage

```typescript
import { readFile } from "node:fs/promises"
import { OpenKache_Client } from "@openkache/client"

const client = await OpenKache_Client.connect({
  address: "203.0.113.10:4433",
  server_name: "cache.example.com",
  certificate: await readFile("client-bundle/ca.crt"),
  identity: {
    certificate_chain: [await readFile("client-bundle/client.crt")],
    private_key: await readFile("client-bundle/client.key"),
  },
  data_protection_key: await readFile("client-bundle/data-protection.key"),
})

await client.set("profile", {
  name: "Kim",
  visits: 42,
  labels: ["subscriber", "beta"],
  active: true,
})
const profile = await client.get<{
  name: string
  visits: number
  labels: string[]
  active: boolean
}>("profile")

const stats = await client.stats()
console.log(stats.storage, stats.workers)

await client.set_raw("opaque", Uint8Array.of(1, 2, 3))
const bytes = await client.get_raw("opaque")
await client.close()
```

`set` accepts regular objects without a positional schema argument. Registered
codecs are checked first; otherwise the built-in JSON codec accepts nested
objects, dense arrays, strings, finite numbers, booleans, and null. The stored
envelope records the encoding and logical type so `get` can select the decoder.
Its optional generic parameter documents the expected application shape.
Properties whose value is `undefined` are omitted by the JSON codec.

Use `{ condition: "if_absent" }` to create without overwriting an existing key,
or `{ condition: "if_present" }` to update without creating a missing key.
Unsatisfied conditions return `"not_stored"`.

Big integers should use decimal strings. Binary fields should use an
application-selected base64 string representation. Use `set_raw` and `get_raw`
when the entire value is binary or uses another shared serialization format.

Custom `Value_Codec` implementations can add Protobuf, FlatBuffers, or an
application format. A codec owns its schema registry, while each stored envelope
carries its encoding and type name. Schemas are therefore registered once
instead of being passed to every `get` and `set`.

The codec registry and JSON fallback use only web-standard APIs and contain no
Node.js imports. Binary envelope framing stays in the Rust core instead of
duplicating its constants in TypeScript. A separate browser configuration
typechecks this subpath against DOM and ES declarations without loading Node
declarations. Empty raw values are valid.

The runtime-neutral layer is available from
`@openkache/client/value-codec`. The package can be installed on any platform;
the current native transport supports Linux x64 and ARM64. `native_path` can
select another compatible Node-API build.

The browser cannot open the UDP-based QUIC transport or load a native adapter.
A future browser client can instead use the browser `WebTransport` API against a
WebTransport/HTTP3 server endpoint. Rust compiled to WebAssembly can reuse the
canonical framing, codecs, compression, and encryption, while JavaScript owns
the WebTransport streams. The runtime-neutral value-codec subpath preserves that
boundary without adding a WebAssembly or browser transport dependency now.

Every connection and cache method returns a promise. The adapter owns one
reusable QUIC connection and runs native networking outside the JavaScript event
loop. Call and await `close()` when finished.

## Configuration

- `address` is the server UDP address.
- `certificate` is one trusted DER or PEM server/CA certificate.
- `identity` contains the DER or PEM client certificate chain and private key
  required by production mutual TLS. An administrator identity is required for
  `stats()` and `sync()`.
- `data_protection_key` is an application-managed 32-byte secret. Clients sharing
  values must use the same key. Generate it once with a cryptographically secure
  random source and persist it in secret storage; do not generate a new key for
  each connection. OpenKache never sends it to the server.
- `compression` controls Zstandard level, minimum input size, and required
  savings. Defaults are level 1, 1 KiB, and 64 bytes.
- `timeouts.connect_ms` bounds endpoint setup and the QUIC/TLS handshake;
  the default is 5000 ms.
- `timeouts.request_ms` bounds each complete request/response operation;
  the default is 2000 ms.
- `value_codecs` registers optional Protobuf, FlatBuffers, or application codecs.
- `native_path` overrides Node-API adapter discovery for custom packaging.

`data_protection_key` replaces the previous `encryption_key` option and changes both key and value
derivation. Reusing the same 32 bytes does not preserve old cache entries: item keys now use a
derived HMAC-SHA-256 key and values use an independently derived encryption key. Repopulate entries
when migrating or rotating this secret.

`stats()` validates the server response and returns
`{ storage: string, workers: readonly string[] }`.

`PING`, `GET`, and `STATS` may reconnect and retry once after a transport
failure. Mutating operations are not retried automatically. Encoded values are
limited to the 64 MiB wire ceiling; servers may enforce a smaller operational
limit.

Stored encrypted values contain a 24-byte nonce, ciphertext, and a 16-byte
authentication tag. Existing request, response, and on-disk metadata fields
carry the compression and encryption bits without adding bytes to the value.
Encryption therefore adds exactly 40 bytes. The flags are authenticated with
the exact item key.
