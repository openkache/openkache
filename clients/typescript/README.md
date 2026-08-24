# OpenKache TypeScript client

`@openkache/client` is the maintained Promise-based TypeScript and JavaScript
client for Node.js, Bun, and Deno. Gate 0 keeps the application surface small:
`connect`, `get`, `set`, `delete`, and `close`. The package delegates transport,
key mapping, and `StructuredValue-CBOR-v1` envelope handling to the shared Rust
core.

## Install

```bash
npm install @openkache/client
# or
bun add @openkache/client
```

The package contains generated declarations and platform Node-API adapters. It
has no runtime JavaScript dependencies. Deno uses its Node compatibility layer
with `--allow-ffi`.

## Quick start

Gate 0 uses a fixed TLS 1.3 development profile:

- QUIC-over-TLS 1.3 with `openkache/1` and `X25519MLKEM768`;
- server certificate and hostname verification disabled (`DevelopmentTrust`);
- namespace `1`, the public `NamespaceHash` Item-ID root, and
  uncompressed/unprotected `StructuredValue-CBOR-v1` values.

The server still presents a certificate and TLS encrypts traffic. This profile
provides passive confidentiality but no active man-in-the-middle protection:
**development only — do not use this trust profile in production**. The client
does not accept certificate, trust-root, identity, transport, retry, timeout,
TTL, compression, or value-protection options.

```typescript
import { OpenKache_Client, type Found_Result } from "@openkache/client"

const client = await OpenKache_Client.connect("127.0.0.1:4433")

try {
  const created = await client.set("profile", {
    name: "Ada",
    visits: 42n,
    tags: ["ssd", "v1"],
    metadata: undefined,
  })
  console.log(created) // "created"

  const profile = await client.get("profile")
  if (profile.kind === "found") {
    const value = (profile as Found_Result).value
    console.log(value)
  }

  console.log(await client.delete("profile")) // "deleted"
} finally {
  await client.close()
}
```

`connect({ address: "127.0.0.1:4433" })` is accepted as an equivalent
endpoint-only shape. Any other connection field is rejected.

## The five-operation facade

The package intentionally exports exactly these cache operations:

| Operation | TypeScript shape | Result |
| --- | --- | --- |
| `connect` | `OpenKache_Client.connect(endpoint)` | `Promise<OpenKache_Client>` |
| `get` | `client.get(key)` | `Promise<Get_Result<Structured_Value>>` |
| `set` | `client.set(key, value)` | `Promise<"created" \| "replaced">` |
| `delete` | `client.delete(key)` | `Promise<"deleted" \| "not_found">` |
| `close` | `client.close()` | `Promise<void>` |

`close` is idempotent. It drains or completes accepted work before releasing
the native connection; calls after close reject with `OpenKache_Error`.

`get` never uses JavaScript `undefined` as a missing sentinel:

```typescript
type Get_Result<Value> = Missing_Result | Found_Result<Value>

class Missing_Result {
  readonly kind: "missing"
}

class Found_Result<Value> {
  readonly kind: "found"
  readonly value: Value
}
```

`Missing_Result` means the item is absent. `Found_Result` wraps every stored
value, including `null` and `Undefined_Value`. `MISSING` is a shared singleton
for callers that want identity comparison.

If a mutation may have crossed server admission but its response is lost, the
promise rejects with `OpenKache_Unknown_Mutation_Error`. The client never
automatically replays that mutation.

## Keys

Each operation infers one typed key independently:

- `string` → valid UTF-8 `Text`; empty and NUL-containing strings are valid.
- `Uint8Array` → exact `Bytes`, including empty and zero bytes.
- `bigint` → signed `i64` `Integer` (`-2^63..=2^63-1`).

JavaScript `number`, booleans, `null`, arrays, objects, invalid UTF-16
surrogates, and out-of-range integers are rejected. A number is never guessed
to be an integer. The adapter canonicalizes each key and derives its Item ID
with the fixed Gate 0 `NamespaceHash` profile.

## Structured values

`set` and `get` use `StructuredValue-CBOR-v1` exclusively. The lossless model
contains:

```text
Null | Undefined | Boolean | Integer | Float16/32/64 |
ByteString | TextString | Array | Map
```

Native JavaScript mappings are:

| JavaScript value | Model value |
| --- | --- |
| `null` | `Null` |
| `undefined` | `Undefined` |
| `boolean` | `Boolean` |
| `bigint` | arbitrary-precision `Integer` |
| `number` | `Float64` (raw binary64 bits) |
| `Uint8Array` | `ByteString` |
| `string` | `TextString` |
| `Array` | ordered `Array` |
| `Map` or plain object | ordered scalar-key `Map` |

Use the lossless constructors when Float16/32 width, raw bits, or a scalar map
key must survive a round trip:

```typescript
import {
  Float_Value,
  Integer_Value,
  Map_Value,
  UNDEFINED_VALUE,
} from "@openkache/client"

const value = new Map_Value([
  ["ratio", new Float_Value(16, 0x3c00n)],
  ["count", new Integer_Value(9007199254740993n)],
  ["optional", UNDEFINED_VALUE],
])
await client.set("lossless", value)
```

Only scalar values may be map keys. Duplicate keys, arrays or maps as keys,
cycles, sparse arrays, functions, classes, and arbitrary object graphs are
rejected. Map order is retained for lossless forwarding; map equality is
order-independent.

The runtime-neutral codec helpers are also available from
`@openkache/client/value-codec` and perform no network I/O:

```typescript
import {
  decode_structured_value,
  encode_structured_value,
} from "@openkache/client/value-codec"

const bytes = encode_structured_value(1n)
const model = decode_structured_value(bytes)
```

`to_native` is an explicit convenience projection. It maps integers to
`bigint`, floats to JavaScript `number`, bytes to copied `Uint8Array`, and
maps to `Map`; use the lossless result from `get` when width or raw bits matter.

## Errors

All operation failures reject with `OpenKache_Error` or a subclass. The stable
`kind` values are:

| Error | `kind` | Meaning |
| --- | --- | --- |
| `OpenKache_Error` | `openkache_error` | Validation, transport, protocol, server, or value failure |
| `OpenKache_Unknown_Mutation_Error` | `unknown_mutation` | A mutation may have reached the server without a definitive response |
| `OpenKache_Error` | `incompatible_server_outcome` | A server returned a non-Gate-0 outcome such as conditional `NotStored` |
| `Structured_Value_Error` | codec-specific | Local structured-value conversion or parsing failure |

## Commands

Run these commands from `openkache/clients/typescript` in the repository Nix
development shell:

```bash
bun install --frozen-lockfile
bun run build
bun run typecheck
bun run pack:check
```

`build` generates ignored contract declarations and `pack:check` builds the
host Node-API adapter before checking package contents. Private integration
tests live in the monorepo rather than this public package.
