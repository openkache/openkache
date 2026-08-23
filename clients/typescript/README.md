# OpenKache TypeScript client

`@openkache/client` is the Promise-based TypeScript and JavaScript SDK for
Node.js, Bun, and Deno. A packaged Node-API adapter delegates networking,
retries, compression, value protection, and canonical JSON behavior to the
shared Rust core; applications need no helper process or runtime dependencies.

## Install

```bash
npm install @openkache/client
# or: bun add @openkache/client
```

The npm package contains generated declarations plus Linux x64/ARM64 GNU and
Apple Silicon Node-API adapters. It has no runtime dependency. Deno uses its
Node compatibility layer with `--allow-ffi`.

## Quick start

The following example assumes an OpenKache server with mutual TLS enabled and a
client bundle containing a trusted CA certificate and client credentials:

```typescript
import { readFile } from "node:fs/promises"
import { OpenKache_Client } from "@openkache/client"

const certificate = await readFile("client-bundle/ca.crt")
const identity = {
  certificate_chain: [await readFile("client-bundle/client.crt")],
  private_key: await readFile("client-bundle/client.key"),
}

const client = await OpenKache_Client.connect({
  address: "127.0.0.1:4433",
  server_name: "localhost",
  certificate,
  identity,
})

try {
  await client.ping()

  // Cross-language canonical JSON.
  const outcome = await client.set_json("profile", {
    name: "Kim",
    visits: 42,
    labels: ["subscriber", "beta"],
    active: true,
  })
  console.log(outcome) // "created" or "replaced"
  console.log(await client.get_json("profile"))

  // Exact binary bytes.
  await client.set_raw("blob", Uint8Array.of(1, 2, 3))
  console.log(await client.get_raw("blob"))

  // Create only when absent, then expire after 60 seconds.
  await client.set_json(
    "lock",
    { owner: "worker-1" },
    { condition: "if_absent", expiration_mode: "explicit_ttl", ttl_ms: 60_000 },
  )
} finally {
  await client.close()
}
```

Use verified QUIC by default, `transport: "tls_tcp"` where UDP is unavailable,
and an explicit `*_insecure` selector only in tests. Production connections
require a trusted certificate and normally a client identity.

## Choose a value API

| API | Value contract | Use when |
|---|---|---|
| `set_json` / `get_json` | Canonical RFC 8785-compatible JSON in the shared Rust core | Values must interoperate across OpenKache clients |
| `set_structured` / `get_structured` | Exact `StructuredValue-CBOR-v1` | Integers, floats, bytes, `undefined`, and ordered maps need distinct types |
| `set_raw` / `get_raw` | Exact bytes after compression and protection | The application already owns serialization |
| `set_v0` / `get_v0` | Complete caller-owned version-0 envelope | Migrating or embedding another envelope format |
| `set` / `get` | Package-local metadata envelope or registered custom codec | Existing package users and codec integrations |

Canonical JSON accepts only null, booleans, finite numbers, strings, dense
arrays, and regular objects with string keys. Cycles, sparse arrays, binary
objects, `undefined`, `bigint`, and non-finite numbers are rejected.

The runtime-neutral structured-value model is available without loading native
networking from `@openkache/client/value-codec`. JavaScript `number` always
encodes as a binary64 float even when integral, while `bigint` encodes exactly
as an integer. The default decode preserves those distinctions with model
wrappers; `get_structured(key, "native")` projects integers to `bigint`,
floats to `number`, bytes to `Uint8Array`, and maps to `Map`.

### Structured values and exact types

Use structured values when binary64, arbitrary-precision integers,
byte strings, `undefined`, and scalar-keyed ordered maps must remain distinct:

```typescript
import {
  ByteString_Value,
  Float_Value,
  Integer_Value,
  Map_Value,
  UNDEFINED_VALUE,
} from "@openkache/client/value-codec"

await client.set_structured("session", {
  id: 9007199254740993n, // Exact integer; number would round.
  ratio: 1.25,
  token: new ByteString_Value(Uint8Array.of(1, 2, 3)),
  metadata: undefined, // Preserved as Undefined.
  attributes: new Map_Value([
    ["region", "ap-northeast-2"],
    [42n, "integer-key"],
  ]),
})

const lossless = await client.get_structured("session")
const native = await client.get_structured("session", "native")
```

For local encoding or browser use, the codec subpath performs no I/O:

```typescript
import {
  decode_structured_value,
  encode_structured_value,
  to_native,
} from "@openkache/client/value-codec"

const bytes = encode_structured_value(
  { width: new Float_Value(64, 0x3ff8000000000000n) },
  { max_bytes: 1024 * 1024 },
)
const model = decode_structured_value(bytes)
console.log(to_native(model))
```

```typescript
import { test } from "node:test"
import assert from "node:assert/strict"
import {
  Float_Value,
  Integer_Value,
  Map_Value,
  Structured_Value_Error,
  decode_structured_value,
  encode_structured_value,
  model_equal,
  to_plain_object,
  to_native,
  to_value,
} from "@openkache/client/value-codec"

test("structured model preserves exact JavaScript type identity", (): void => {
  assert.deepEqual([...encode_structured_value(1)], [
    0xfb, 0x3f, 0xf0, 0, 0, 0, 0, 0, 0,
  ])
  assert.deepEqual([...encode_structured_value(1n)], [0x01])
})

test("ordered maps keep scalar key identity", (): void => {
  const value = new Map_Value([
    [true, "boolean"],
    [1n, "integer"],
  ])
  assert.equal(model_equal(value, decode_structured_value(encode_structured_value(value))), true)
})

test("float width and byte/text distinctions survive a round trip", (): void => {
  assert.equal(
    model_equal(
      to_value(Uint8Array.of(0x78)),
      decode_structured_value(encode_structured_value("x")),
    ),
    false,
  )
})

test("plain-object projection defines __proto__ safely", (): void => {
  const source = Object.create(null) as Record<string, unknown>
  source.__proto__ = 1n
  const projected = to_plain_object(decode_structured_value(encode_structured_value(source)) as Map_Value)
  assert.equal(Object.getPrototypeOf(projected), null)
})

test("malformed values and duplicate map keys are rejected", (): void => {
  const value = new Map_Value([[1n, "first"], [1n, "second"]])
  assert.throws(() => new Map_Value(value.entries), Structured_Value_Error)
})

test("resource limits reject an oversized declared payload", (): void => {
  assert.throws(
    () => decode_structured_value(Uint8Array.from([0x58, 0x05, 0x41]), { max_bytes: 4 }),
    Structured_Value_Error,
  )
})

test("native projection rejects lossy Map key collisions", (): void => {
  const value = new Map_Value([
    [new Float_Value(64, 0n), "positive"],
    [new Float_Value(64, 0x8000000000000000n), "negative"],
  ])
  assert.throws(() => to_native(decode_structured_value(encode_structured_value(value))))
})
```

`encode_structured_value` accepts ordinary arrays and text-keyed objects plus
the lossless wrapper classes. It rejects cycles, sparse arrays, unsupported
native objects, non-scalar map keys, and duplicate map keys.

### Compatibility envelope and codecs

The compatibility `set` / `get` path is package-local. It stores a
TypeScript-specific `{ encoding, type_name, payload }` metadata envelope, not
the shared-core canonical JSON or structured-value format used by
`set_json`, `set_structured`, or other OpenKache clients. Use it when an
application must remain compatible with data already written by this package
or must plug in its own object codec:

- Built-in fallback: plain JSON values encode as the package-local `"json"`
  envelope.
- Custom codecs: objects matching `can_encode` store under the codec's stable
  encoding name.
- Cross-language reads: prefer `set_json` / `set_structured`; this path does
  not interoperate with them.

```typescript
import type { Value_Codec } from "@openkache/client"

const protobuf_codec: Value_Codec = {
  encoding: "acme.protobuf",
  can_encode: (value): boolean =>
    typeof value === "object" &&
    value !== null &&
    "protobuf_type" in value,
  encode: (value): { type_name: string; payload: Uint8Array } => {
    // Call your schema-specific encoder here.
    return { type_name: "acme.Profile", payload: Uint8Array.of(1) }
  },
  decode: (type_name, payload): object => {
    if (type_name !== "acme.Profile") throw new Error("unsupported profile")
    return { protobuf_type: "Profile", payload }
  },
}

// Supply value_codecs: [protobuf_codec] when connecting.
```

## Keys and exact item IDs

Mapped operations infer one typed key per call:

- `string`: exact UTF-8 text; unpaired surrogates are rejected.
- `Uint8Array`: exact bytes.
- `number`: finite safe integer other than `-0`.
- `bigint`: signed-i64 range.

Empty and NUL-containing keys are valid. The adapter converts each mapped key
to deterministic CBOR bytes before deriving an item ID. The deprecated
`key_spec` option remains accepted only for source compatibility and does not
change per-operation inference.

For protocol-level control, use `client.raw()`:

```typescript
import { SMITHY_ITEM_ID_BYTES } from "@openkache/client"

const raw = client.raw()
const namespace = await raw.namespace_open({
  name: "application",
  create_if_missing: true,
  policy: {
    default_expiration: "no_expiry",
    expiration_override: "allowed",
    default_eviction: "evictable",
    eviction_override: "allowed",
  },
})
const namespace_id = namespace.descriptor.namespace_id
const item_id = crypto.getRandomValues(new Uint8Array(SMITHY_ITEM_ID_BYTES))

await raw.set({ namespace_id, item_id, value: new TextEncoder().encode("v1") })
const stored = await raw.get({ namespace_id, item_id })
console.log(stored.value)
```

Raw methods accept exact `0..=32`-byte item IDs and opaque values. Namespace
management operations remain transitional control-plane shapes.

## Errors and lifecycle

All cache operations reject with `OpenKache_Error`; mutations whose outcome may
have reached the server reject with
`OpenKache_Unknown_Mutation_Error`, and cancelled local work rejects with
`OpenKache_Cancelled_Error`. Check the stable `kind` property rather than
parsing messages.

`stats` and `sync` are experimental maintenance operations. Enable them on the
server only with `enable_experimental_api = true` and coordinate revision
`draft-2026-08-19.4` out of band as described in
[protocol/EXPERIMENTAL.md](../../protocol/EXPERIMENTAL.md).

A client owns one reusable connection. `connection_state()` reports
`connected`, `reconnecting`, `disconnected`, `closed`, or `unknown`;
`reconnect()` replaces a failed connection without replaying an operation.
Await `close()`, which is idempotent, before process exit. Operations after
closing reject.

### Error reference

| Error | `kind` | Meaning |
|---|---|---|
| `OpenKache_Error` | `"openkache_error"` | Validation, configuration, transport, server, decoding, or other generic failure |
| `OpenKache_Unknown_Mutation_Error` | `"unknown_mutation"` | A mutation may have reached the server, but its outcome is not known |
| `OpenKache_Cancelled_Error` | `"cancelled"` | Work was cancelled before a definitive result |
| `Structured_Value_Error` | See below | Local structured-value conversion or CBOR parsing failed |

Structured-value errors expose these stable categories:

| `kind` | Meaning |
|---|---|
| `"conversion"` | A model value cannot be represented in the requested native form |
| `"resource_limit"` | Bytes, depth, items, or integer bytes exceeded the configured budget |
| `"truncated"` | The payload ended before completing a value |
| `"trailing_bytes"` | Extra bytes followed one complete root value |
| `"invalid_encoding"` | CBOR tags, lengths, or floating-point forms are not allowed by the contract |
| `"unsupported_type"` | A runtime value cannot be converted to the structured model |
| `"invalid_utf8"` | Text contains unpaired surrogates or malformed UTF-8 |
| `"invalid_integer"` | An integer form violates the contract |
| `"non_scalar_key"` | A map key is not null, boolean, integer, float, byte-string, or text |
| `"duplicate_key"` | Two map keys are structurally equal |

```typescript
try {
  await client.set_json("metrics", value)
} catch (error) {
  if (error instanceof OpenKache_Unknown_Mutation_Error) {
    // Do not blindly retry a non-idempotent mutation.
  } else if (error instanceof OpenKache_Error) {
    console.error(error.kind, error.message)
  } else {
    throw error
  }
}
```

## API reference

### Client lifecycle and maintenance

| Member | Signature | Contract |
|---|---|---|
| `connect` | `static connect(options: Client_Options): Promise<OpenKache_Client>` | Validates settings, loads the platform adapter, opens one connection, and rejects on setup failure |
| `ping` | `ping(): Promise<void>` | Verifies reachability and protocol handling |
| `connection_state` | `connection_state(): Connection_State` | Returns a best-effort snapshot without connecting |
| `reconnect` | `reconnect(): Promise<void>` | Replaces a failed connection without replaying operations |
| `close` | `close(): Promise<void>` | Releases the connection; repeated awaits are safe; later operations reject |

### Mapped key operations

All mapped methods accept `Client_Key`: UTF-8 `string`, `Uint8Array`, safe
integer-valued `number`, or signed-i64 `bigint`.

| Method | Signature | Value contract |
|---|---|---|
| `get` | `get<Value = Json_Value>(key: Client_Key): Promise<Value \| undefined>` | Reads the package-local metadata envelope or registered custom codec |
| `set` | `set<Value>(key: Client_Key, value: Value, options?: Set_Options): Promise<Set_Outcome>` | Encodes with the built-in envelope or first matching custom codec |
| `get_json` | `get_json(key: Client_Key): Promise<Json_Value \| undefined>` | Reads shared-core canonical JSON |
| `set_json` | `set_json(key: Client_Key, value: Json_Value, options?: Set_Options): Promise<Set_Outcome>` | Writes shared-core canonical JSON |
| `get_structured` | `get_structured(key: Client_Key, representation?: "lossless" \| "native"): Promise<unknown \| undefined>` | Lossless decode returns model wrappers; native projection applies strict JavaScript mapping |
| `set_structured` | `set_structured(key: Client_Key, value: unknown, options?: Set_Options): Promise<Set_Outcome>` | Encodes `StructuredValue-CBOR-v1` directly |
| `get_raw` | `get_raw(key: Client_Key): Promise<Uint8Array \| undefined>` | Reads protected transport bytes exactly after core decompression/decryption |
| `set_raw` | `set_raw(key: Client_Key, value: Uint8Array, options?: Set_Options): Promise<Set_Outcome>` | Stores exact application bytes |
| `get_v0` | `get_v0(key: Client_Key): Promise<Uint8Array \| undefined>` | Reads a complete caller-owned version-0 envelope |
| `set_v0` | `set_v0(key: Client_Key, value: Uint8Array, options?: Set_Options): Promise<Set_Outcome>` | Stores a version-0 envelope after leading-version and size checks |
| `delete` | `delete(key: Client_Key): Promise<boolean>` | Returns whether an item existed and was deleted |

### Raw exact-item operations

`client.raw(): OpenKache_Raw_Client` shares the client connection. Its inputs
use generated Smithy shapes with `namespace_id`, `item_id`, opaque `value`,
and optional set behavior. In addition to raw `ping/get/set/delete`,
it provides explicit JSON, structured, v0, namespace-management, stats,
sync, reconnect, close, and state operations. Namespace management remains
transitional.

Set outcomes are `"created"`, `"replaced"`, or `"not_stored"`.

### Value codec exports

Import these from `@openkache/client/value-codec` unless listed as also being
available from the main entry point:

| Export | Kind | Purpose |
|---|---|---|
| `Json_Value`, `Json_Object` | Type | Canonical JSON value model accepted by JSON helpers |
| `assert_json_value` | Function | Narrows unknown data and rejects unsupported JSON constructs |
| `Value_Codec`, `Encoded_Value`, `Value_Envelope` | Type | Compatibility-codec boundary and stored envelope components |
| `Value_Codec_Registry` | Class | Selects codecs on writes and routes envelopes on reads |
| `Structured_Value_Error` | Class | Local codec error with a stable `kind` |
| `Value_Limits` | Type | Bounds for encode and decode budgets |
| `Undefined_Value` through `Map_Value` | Class | Lossless model wrappers preserving exact type identity and order |
| `Structured_Value` | Type | Complete lossless model union |
| `UNDEFINED_VALUE` | Constant | Shared model representation of `undefined` |
| `to_value` | Function | Converts supported native/model data to lossless model form |
| `model_equal` | Function | Compares models structurally, including float width and raw bits |
| `encode_structured_value` | Function | Encodes one bounded `StructuredValue-CBOR-v1` payload |
| `decode_structured_value` | Function | Decodes exactly one bounded payload to lossless model form |
| `to_native` | Function | Projects a model to native JavaScript values with optional safe-integer checking |
| `decode_native_value` | Function | Decodes payload bytes directly to native projection |
| `to_plain_object` | Function | Converts a text-keyed map to a null-prototype object safely |

`Encoded_Value`, `Json_Value`, `Structured_Value_Error_Kind`, and
`Structured_Value` are also re-exported by the main entry point. Generated
Smithy operation and limit names remain available from `@openkache/client`
for protocol-level code.

## Configuration

- `address` is the server transport address.
- `server_name` is the certificate identity for a pre-resolved address.
- `certificate` is one trusted DER or PEM server or CA certificate.
- `identity` contains the DER or PEM client certificate chain and private key
  used for mutual TLS.
- `data_protection_key` is optional. When supplied it is a persistent
  application-managed 32-byte random secret; clients sharing protected values
  must use the same key. When omitted, Item IDs are still derived but values
  are stored unprotected.
- `compression` controls the Zstandard level and optional thresholds. The
  maintained default is automatic level 1 with zero input-size and
  minimum-savings thresholds; it emits the compressed frame only when it is
  smaller. Set `compression.enabled` to `false` for an explicit opt-out.
- `timeouts.connect_ms` and `timeouts.request_ms` bound connection and complete
  request operations.
- `retry.max_attempts` controls retries for response-safe operations.
- `max_in_flight` bounds concurrent request lanes on one connection.
- `max_in_flight_bytes` bounds aggregate bytes retained across transport and
  value-protection work.
- `encryption` selects the shared core's `compact` or recommended `robust`
  authenticated-encryption profile.
- `value_codecs` registers current package codecs.
- `native_path` overrides Node-API adapter discovery for custom packaging.

Defaults come from the generated Smithy contract: 256 request lanes, 5-second
connect timeout, 2-second request timeout, two total attempts, and automatic
level-1 Zstandard compression. Explicit thresholds emit compression only when
the compressed frame is smaller.

Generate the data-protection key once with a cryptographically secure random
source and store it as a secret. Rotating it makes existing protected entries
unreachable.

## Runtime and compatibility

Protocol limits and operation outcomes follow the
[wire protocol specification](../../protocol/SPEC.md); retry policy remains
client-local.

## Development

Build and check the package from `clients/typescript` in the repository Nix
development shell:

```bash
bun install --frozen-lockfile
bun run build        # requires Smithy CLI on PATH
bun run typecheck
bun run pack:check   # also builds host native adapters
```

`build` generates ignored TypeScript contract files under
`src/generated_local/`; release packaging adds platform `.node` adapters under
ignored `target/native/`. Private integration tests live in the monorepo's
`tests/clients/` workspace rather than this public package.
