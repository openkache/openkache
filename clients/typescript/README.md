# OpenKache TypeScript client

`@openkache/client` is the TypeScript and JavaScript SDK for Node.js, Bun, and
Deno. A packaged Node-API adapter delegates network, retries, value protection,
and canonical JSON behavior to `openkache-client-core`; applications need no
helper process or runtime npm dependencies.

The package exports a runtime-neutral ``StructuredValue-CBOR-v1`` model from
the ``value-codec`` subpath. JavaScript ``number`` always maps to a binary64
``Float`` (even when integral), ``bigint`` maps to an exact ``Integer``,
``undefined`` is distinct from ``null``, and ``Uint8Array`` maps to
``ByteString``. ``Map`` and documented plain objects preserve entry order.
The default lossless decode returns model wrappers; strict native projection
returns ``bigint`` for integers, ``number`` for floats, ``Uint8Array`` for
bytes, and ``Map`` for maps. A checked safe-integer option rejects values that
would round, and plain-object projection uses null-prototype objects with
defined properties so names such as ``__proto__`` are safe.

Client ``get_structured`` and ``set_structured`` invoke the required generated
native selectors directly; they never reinterpret payloads through legacy JSON
or Raw operations. The native boundary preserves an unknown mutation as
``OpenKache_Unknown_Mutation_Error`` and local cancellation as
``OpenKache_Cancelled_Error``.

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
and null through the backwards-compatible TypeScript metadata envelope. Its
optional generic parameter documents the expected result shape. Object
properties whose value is `undefined` are omitted by this legacy JSON helper;
the structured-value codec preserves them as ``Undefined``.

Use `set_json` and `get_json` for the cross-language value API. These methods
delegate JSON conversion, canonical RFC 8785 serialization, compression, and
authenticated encryption to the shared Rust core, so values written by the
Rust client can be read without a TypeScript-specific envelope:

```typescript
await client.set_json("shared", { z: 1, a: ["core", true] })
const shared = await client.get_json("shared")
```

Canonical JSON accepts only null, booleans, finite numbers, strings, dense
arrays, and regular objects with string keys. Cycles, sparse arrays, binary
objects, `undefined`, `bigint`, and non-finite numbers are rejected.

Keys follow the configured current `key_spec` (`text` by default), which
selects the target `key_type`, not an Item ID mapping profile: text keys are
exact UTF-8 strings, byte keys are exact `Uint8Array` values, and integer keys
are safe integer-valued `number` or `bigint` values. Empty and NUL-containing
keys are valid; floating-point, unsupported native types, and unpaired
surrogates are rejected. A future API may expose the target name `key_type`
directly.

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
- `data_protection_key` is optional. When supplied it is a persistent
  application-managed 32-byte random secret; clients sharing protected values
  must use the same key. When omitted, Item IDs are still derived but values
  are stored unprotected.
- `compression` controls Zstandard level, minimum input size, and required
  savings.
- `timeouts.connect_ms` and `timeouts.request_ms` bound connection and complete
  request operations.
- `retry.max_attempts` controls retries for response-safe operations.
- `max_in_flight` bounds concurrent request lanes on one connection.
- `max_in_flight_bytes` bounds aggregate bytes retained across transport and
  value protection work.
- `encryption` selects the shared core's `compact` or recommended `robust`
  authenticated-encryption profile.
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
connection. `connection_state()` reports `connected`, `reconnecting`,
`disconnected`, `closed`, or `unknown`; `reconnect()` replaces a failed connection without
replaying an operation. Call and await `close()` when finished. The
`client.raw()` view implements the Smithy-generated `Smithy_OpenKache_Api`
contract for the current fixed-width item-ID API and opaque protocol values.
The draft key contract targets `0..=32`-byte Item IDs and is not implemented
by this package yet.

Protocol limits and operation outcomes follow the
[wire protocol specification](../../protocol/SPEC.md); retry policy remains
client-local.
