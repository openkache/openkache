# OpenKache TypeScript client

`@openkache/client` is the Promise-based TypeScript and JavaScript SDK for
Node.js, Bun, and Deno. A packaged Node-API adapter delegates networking,
retries, compression, value protection, and the common structured value format
to the shared Rust core; applications need no helper process or runtime
dependencies.

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

  // The primary API uses the common structured value format.
  const outcome = await client.set("profile", {
    name: "Kim",
    visits: 42,
    labels: ["subscriber", "beta"],
    active: true,
  })
  console.log(outcome) // "created" or "replaced"
  const profile = await client.get("profile")
  if (profile instanceof Map) {
    console.log(profile.get("name"))
  }

  // Create only when absent, then expire after 60 seconds.
  await client.set(
    "lock",
    { owner: "worker-1" },
    { condition: "if_absent", expiration_mode: "explicit_ttl", ttl_ms: 60_000 },
  )

  const stats = await client.experimental_stats()
  console.log(stats.storage, stats.workers)

  await client.set_raw("opaque", Uint8Array.of(1, 2, 3))
  const bytes = await client.get_raw("opaque")
} finally {
  await client.close()
}
```

Use verified QUIC by default, `transport: "tls_tcp"` where UDP is unavailable,
and an explicit `*_insecure` selector only in tests. Production connections
require a trusted certificate and normally a client identity.

`experimental_stats` is a transitional experimental operation and is disabled
by default. Enable `enable_experimental_api = true` explicitly and coordinate
exact revision `draft-2026-08-19.4` out of band as described in
[`protocol/EXPERIMENTAL.md`](../../protocol/EXPERIMENTAL.md) before calling it;
the revision is not negotiated on the wire.

## Primary value API: `set` / `get`

Mapped `set` and `get` are the normal application API. Both use the common
structured value contract described by the shared
[`VALUE_FORMAT.md`](../VALUE_FORMAT.md) and [`value/SPEC.md`](../value/SPEC.md).
The concrete payload profile and selector are chosen by the shared core; they
are not part of the TypeScript application API. `set` and `get` do not use the
former TypeScript-only `{ encoding, type_name, payload }` metadata envelope.

The native JavaScript mapping is intentionally explicit:

| JavaScript value | Common value model | `get` result |
|---|---|---|
| `null` | `Null` | `null` |
| `undefined` | `Undefined` | `undefined` |
| `boolean` | `Boolean` | `boolean` |
| `number` | `Float64` | `number` |
| `bigint` | `Integer` | `bigint` |
| `string` | `TextString` | `string` |
| `Uint8Array` | `ByteString` | copied `Uint8Array` |
| `Array` | `Array` | `Array` |
| `Map` or plain object | ordered `Map` | `Map` |

Plain objects are encoded as text-keyed maps and are returned as `Map` values.
This avoids stringifying scalar map keys, overwriting entries, or silently
changing cross-language map semantics. Use the lossless helpers below when
float width, raw bits, or exact model wrappers are part of the contract.

Values that cannot be represented by the common model—such as functions,
classes, cyclic objects, and sparse arrays—are rejected. `NaN` and infinities
are valid structured values; the advanced JSON helper
rejects them because JSON has no representation for them.

The runtime-neutral structured-value model is available without loading native
networking from `@openkache/client/value-codec`. JavaScript `number` always
uses the model's binary64 representation even when integral, while `bigint`
is an exact integer. `decode_structured_value` and
`get_structured(key)` preserve those distinctions with model wrappers;
`get_structured(key, "native")` projects integers to `bigint`, floats to
`number`, bytes to `Uint8Array`, and maps to `Map`.

### Structured values and exact types

Use structured values when binary64, arbitrary-precision integers,
byte strings, `undefined`, and scalar-keyed ordered maps must remain distinct:

```typescript
await client.set("session", {
  id: 9007199254740993n, // Exact integer; number would round.
  ratio: 1.25,
  token: Uint8Array.of(1, 2, 3),
  metadata: undefined, // Preserved as Undefined.
  attributes: new Map([
    ["region", "ap-northeast-2"],
    [42n, "integer-key"],
  ]),
})

const native = await client.get("session")
```

Use `Map` when a value needs non-text keys. The ordinary `get` result for this
example is also a `Map`, with the nested `id` value returned as `bigint`.
Because both a missing key and a root `Undefined` project to JavaScript
`undefined`, use `get_structured` when those two states must be distinguished.

`get_structured(key)` is an advanced lossless view of the same structured
value. It returns model wrappers such as `Map_Value`, `Integer_Value`, and
`Float_Value`; `get_structured(key, "native")` applies the same strict native
projection as `get`.

For local conversion or browser use, the value-codec subpath performs no I/O:

```typescript
import {
  decode_structured_value,
  encode_structured_value,
  Float_Value,
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
  assert.equal(
    model_equal(
      decode_structured_value(encode_structured_value(1)),
      to_value(1),
    ),
    true,
  )
  assert.equal(
    model_equal(
      decode_structured_value(encode_structured_value(1n)),
      to_value(1n),
    ),
    true,
  )
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

The old package-local metadata-envelope path is not used by `set` or `get` and
is not a cross-language compatibility contract. Applications that need a
schema-bound format should encode it explicitly and use `set_raw`/`get_raw`
until a dedicated shared payload profile is defined.

## Keys and exact item IDs

Mapped operations infer one typed key per call:

- `string`: exact UTF-8 text; unpaired surrogates are rejected.
- `Uint8Array`: exact bytes.
- `number`: finite safe integer other than `-0`.
- `bigint`: signed-i64 range.

Empty and NUL-containing keys are valid. The adapter converts each mapped key
to canonical bytes before deriving an item ID. The deprecated
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

`experimental_stats` and `experimental_sync` are experimental maintenance
operations. Enable them on the server only with `enable_experimental_api = true`
and coordinate revision
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
| `Structured_Value_Error` | See below | Local structured-value conversion or payload parsing failed |

Structured-value errors expose these stable categories:

| `kind` | Meaning |
|---|---|
| `"conversion"` | A model value cannot be represented in the requested native form |
| `"resource_limit"` | Bytes, depth, items, or integer bytes exceeded the configured budget |
| `"truncated"` | The payload ended before completing a value |
| `"trailing_bytes"` | Extra bytes followed one complete root value |
| `"invalid_encoding"` | Payload tags, lengths, or floating-point forms are not allowed by the contract |
| `"unsupported_type"` | A runtime value cannot be converted to the structured model |
| `"invalid_utf8"` | Text contains unpaired surrogates or malformed UTF-8 |
| `"invalid_integer"` | An integer form violates the contract |
| `"non_scalar_key"` | A map key is not null, boolean, integer, float, byte-string, or text |
| `"duplicate_key"` | Two map keys are structurally equal |

```typescript
try {
  await client.set("metrics", value)
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

### Public types

| Type | Purpose |
|---|---|
| `Client_Options` | TLS, transport, protection, compression, timeout, retry, and resource-budget settings |
| `Client_Key` | Mapped key union: UTF-8 `string`, `Uint8Array`, safe integer `number`, or signed-i64 `bigint` |
| `Native_Value` | Recursive native projection returned by `get`, including `bigint`, `Uint8Array`, arrays, and `Map` |
| `Set_Options` | Conditional-write, TTL, expiration, and eviction settings |
| `Set_Outcome` | `"created"`, `"replaced"`, or `"not_stored"` |
| `Connection_State` | `"connected"`, `"reconnecting"`, `"disconnected"`, `"closed"`, or `"unknown"` |

### Mapped key operations

All mapped methods accept `Client_Key`: UTF-8 `string`, `Uint8Array`, safe
integer-valued `number`, or signed-i64 `bigint`.

The first two rows are the primary API. The remaining rows are explicit
advanced selectors and escape hatches; they do not change the `set` / `get`
contract.

| Method | Signature | Value contract |
|---|---|---|
| `get` | `get<Value = Native_Value>(key: Client_Key): Promise<Value \| undefined>` | Reads the common structured value and returns the strict native JavaScript projection |
| `set` | `set(key: Client_Key, value: unknown, options?: Set_Options): Promise<Set_Outcome>` | Writes the common structured value from the documented native value mapping |
| `get_structured` | `get_structured(key: Client_Key, representation?: "lossless" \| "native"): Promise<unknown \| undefined>` | Advanced structured-value read; lossless mode returns model wrappers |
| `set_structured` | `set_structured(key: Client_Key, value: unknown, options?: Set_Options): Promise<Set_Outcome>` | Advanced structured-value write from native/model values |
| `get_json` | `get_json(key: Client_Key): Promise<Json_Value \| undefined>` | Advanced JSON compatibility read |
| `set_json` | `set_json(key: Client_Key, value: Json_Value, options?: Set_Options): Promise<Set_Outcome>` | Advanced JSON compatibility write |
| `get_raw` | `get_raw(key: Client_Key): Promise<Uint8Array \| undefined>` | Reads exact application bytes after core decompression/decryption |
| `set_raw` | `set_raw(key: Client_Key, value: Uint8Array, options?: Set_Options): Promise<Set_Outcome>` | Stores application bytes as an opaque payload |
| `get_v0` | `get_v0(key: Client_Key): Promise<Uint8Array \| undefined>` | Advanced caller-owned version-0 envelope read |
| `set_v0` | `set_v0(key: Client_Key, value: Uint8Array, options?: Set_Options): Promise<Set_Outcome>` | Stores a caller-owned version-0 envelope after leading-version and size checks |
| `delete` | `delete(key: Client_Key): Promise<boolean>` | Returns whether an item existed and was deleted |

### Raw exact-item operations

`client.raw(): OpenKache_Raw_Client` shares the client connection. Its inputs
use generated Smithy shapes with `namespace_id`, `item_id`, opaque `value`,
and optional set behavior. In addition to raw `ping/get/set/delete`,
it provides explicit JSON, structured, v0, namespace-management,
experimental_stats, experimental_sync, reconnect, close, and state operations.
Namespace management remains transitional.

Set outcomes are `"created"`, `"replaced"`, or `"not_stored"`.

### Value codec exports

Import these from `@openkache/client/value-codec` unless listed as also being
available from the main entry point:

| Export | Kind | Purpose |
|---|---|---|
| `Json_Value`, `Json_Object` | Type | Canonical JSON value model accepted by JSON helpers |
| `assert_json_value` | Function | Narrows unknown data and rejects unsupported JSON constructs |
| `Structured_Value_Error` | Class | Local structured-value error with a stable `kind` |
| `Value_Limits` | Type | Bounds for encode and decode budgets |
| `Undefined_Value` through `Map_Value` | Class | Lossless model wrappers preserving exact type identity and order |
| `Structured_Value` | Type | Complete lossless model union |
| `UNDEFINED_VALUE` | Constant | Shared model representation of `undefined` |
| `to_value` | Function | Converts supported native/model data to lossless model form |
| `model_equal` | Function | Compares models structurally, including float width and raw bits |
| `encode_structured_value` | Function | Encodes one bounded structured-value payload |
| `decode_structured_value` | Function | Decodes exactly one bounded payload to lossless model form |
| `to_native` | Function | Projects a model to native JavaScript values with optional safe-integer checking |
| `decode_native_value` | Function | Decodes payload bytes directly to native projection |
| `to_plain_object` | Function | Converts a text-keyed map to a null-prototype object safely |

`Json_Value`, `Structured_Value_Error_Kind`, and `Structured_Value` are also
re-exported by the main entry point. Generated Smithy operation and limit names
remain available from `@openkache/client` for protocol-level code.

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
- `native_path` overrides Node-API adapter discovery for custom packaging.

Defaults come from the generated Smithy contract: 256 request lanes, 5-second
connect timeout, 2-second request timeout, two total attempts, and automatic
level-1 Zstandard compression. Explicit thresholds emit compression only when
the compressed frame is smaller.

Generate the data-protection key once with a cryptographically secure random
source and store it as a secret. Rotating it makes existing protected entries
unreachable.

## Runtime and compatibility

`set` / `get` are the common structured-value API. They are not
backward-compatible with values written by the former TypeScript-only metadata
envelope. Use the explicit `set_json` / `get_json`, `set_raw` / `get_raw`, or
`set_v0` / `get_v0` helpers only when the stored value was written with that
same profile. Protocol limits and operation outcomes follow the
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

Maintainers should follow [`RELEASING.md`](./RELEASING.md) for the versioned
npm publication process. `bun run release:dry-run` validates the complete
multi-platform artifact without publishing; `bun run release:publish` is the
only package-local command that performs the authenticated publication.
