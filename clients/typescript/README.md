# OpenKache TypeScript client

`@openkache/client` is the TypeScript and JavaScript SDK for Node.js, Bun, and
Deno. A packaged Node-API adapter delegates network and protection behavior to
`openkache-client-core`; applications need no helper process or runtime npm
dependencies.

## Purpose

The package converts JavaScript values and configuration into shared-core
operations and exposes a Promise-based API. It does not implement QUIC,
protocol framing, compression, or encryption in JavaScript.

Shared SDK status and native-binding boundaries live in the
[client index](../README.md). Formatted value bytes belong to the
[value-format specification](../VALUE_FORMAT.md), and server-visible behavior
belongs to the [wire protocol specification](../../protocol/SPEC.md).

## Commands

From `clients/typescript`:

```bash
bun install --frozen-lockfile
bun run build
bun run typecheck
bun run pack:check
```

`build` requires Smithy CLI on `PATH` and generates the service API plus the
cross-language value-format layout, identifiers, cryptographic metadata, and
legacy envelope limits before compiling ignored JavaScript and declaration
files under `dist/`. The generated values are not committed.
`pack:check` cross-compiles the Rust Node-API adapter and previews the complete
npm package. Generated output is not committed.

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

`set` accepts nested objects, dense arrays, strings, finite numbers, booleans,
and null through the built-in JSON codec. Its optional generic parameter
documents the expected result shape. Object properties whose value is
`undefined` are omitted.

Use `{ condition: "if_absent" }` to create without overwriting or
`{ condition: "if_present" }` to update only an existing item. Use `set_raw`
and `get_raw` when the complete value is binary or already serialized.

Big integers should use an application-defined decimal string representation.
Binary object fields should use an application-defined textual representation,
such as Base64.

Custom `Value_Codec` implementations can register Protobuf, FlatBuffers, or
application formats. The codec registry is a package API, not the shared
value-format registry. The
[client status table](../README.md#sdk-status) tracks format migration.

The runtime-neutral codec layer is available from
`@openkache/client/value-codec`.

## Configuration

- `address` is the server UDP address.
- `server_name` is the certificate identity for a pre-resolved address.
- `certificate` is one trusted DER or PEM server or CA certificate.
- `identity` contains the DER or PEM client certificate chain and private key
  used for mutual TLS.
- `data_protection_key` is a persistent application-managed 32-byte random
  secret. Clients sharing protected values must use the same key.
- `compression` controls Zstandard level, minimum input size, and required
  savings.
- `timeouts.connect_ms` and `timeouts.request_ms` bound connection and complete
  request operations.
- `value_codecs` registers current package codecs.
- `native_path` overrides Node-API adapter discovery for custom packaging.

Generate the data-protection key once with a cryptographically secure random
source and store it as a secret. Rotating it makes existing protected entries
unreachable.

## Runtime and lifecycle

Published applications can use Node.js 20 or newer, Bun's Node-API support, or
Deno's Node compatibility layer with `--allow-ffi`. Release packages support
Linux x64 and ARM64 (glibc 2.17 or newer) plus Apple Silicon macOS.

The browser cannot load the native adapter or open the UDP-based QUIC
transport. The `value-codec` subpath is runtime-neutral, but the client
connection API is not a browser transport.

Every connection and cache method returns a Promise. The adapter runs native
networking outside the JavaScript event loop and maintains one reusable
connection. Call and await `close()` when finished.

Protocol limits, operation outcomes, and retry safety follow the
[wire protocol specification](../../protocol/SPEC.md).
