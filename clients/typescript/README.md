# OpenKache TypeScript client

OpenKache is a super-fast, open-source SSD cache server. Use this TypeScript
or JavaScript client to store, read, and delete values in a few lines. It runs
on Node.js, Bun, and Deno; it is not a browser client.

[npm package](https://www.npmjs.com/package/openkache) ·
[GitHub source](https://github.com/openkache/openkache/tree/main/clients/typescript)

## Install

```bash
# npm
npm install openkache

# pnpm
pnpm add openkache

# Yarn
yarn add openkache

# Bun
bun add openkache

# Deno
deno add npm:openkache
```

Node.js users need Node.js 20 or newer. The published native adapters currently
support Linux x64/ARM64 (glibc) and Apple Silicon macOS.

## Quick start

The example below assumes a local OpenKache server at `127.0.0.1:4433`.
Save it as `index.mjs`; the same code can be used from a TypeScript file.

```javascript
import { OpenKacheClient } from "openkache"

const client = await OpenKacheClient.connect("127.0.0.1:4433")

await client.set("greeting", "hello")
const result = await client.get("greeting")
console.log(result.kind === "found" ? result.value : "missing")
await client.delete("greeting")
await client.close()
```

Run it with Node.js or Bun:

```bash
node index.mjs
# or
bun index.mjs
```

For Deno, change the import to `from "npm:openkache"` and run:

```bash
deno run --allow-net --allow-ffi index.ts
```

The local development TLS profile does not verify the server certificate. Use
this example only with a local development server.

## Reference

### `OpenKacheClient.connect(endpoint)`

Opens a connection and returns a client.

- **Input:** a non-empty `host:port` string or `{ address }`. IPv6 endpoints
  use `[host]:port`.
- **Returns:** `Promise<OpenKacheClient>`.
- **Throws:** `OpenKacheError` when validation or connection setup fails.

```javascript
const client = await OpenKacheClient.connect("127.0.0.1:4433")
```

### `client.get(key)`

Reads one structured value.

- **Input:** a `ClientKey`.
- **Returns:** `FoundResult` when the key exists, or `MissingResult` when it
  does not. A stored `undefined` is still returned as `FoundResult`.
- **Throws:** `OpenKacheError` when validation, transport, or decoding fails.

```javascript
const result = await client.get("greeting")
if (result.kind === "found") {
  console.log(result.value)
}
```

`MISSING` is a shared `MissingResult` instance.

### `client.set(key, value)`

Stores one value with an unconditional write.

- **Input:** a `ClientKey` and a native or lossless structured value.
- **Returns:** `"created"` for a new key or `"replaced"` for an existing key.
- **Throws:** `OpenKacheError` when validation, encoding, transport, or
  storage fails.

```javascript
const outcome = await client.set("greeting", "hello")
```

### `client.delete(key)`

Deletes one key. Repeating the operation is safe.

- **Input:** a `ClientKey`.
- **Returns:** `"deleted"` when a value was removed, or `"not_found"` when no
  value existed.
- **Throws:** `OpenKacheError` when validation, transport, or storage fails.

```javascript
const outcome = await client.delete("greeting")
```

### `client.close()`

Closes the connection. Repeated calls complete successfully.

- **Returns:** `Promise<void>`.

```javascript
await client.close()
```

### Keys

Each operation accepts one of these key types:

- `string` — UTF-8 text, including empty strings.
- `Uint8Array` — exact bytes, including empty bytes.
- `number` — a finite safe integer other than `-0`.
- `bigint` — a signed 64-bit integer.

Fractions, `NaN`, infinities, unsafe numbers, objects, arrays, booleans, and
strings containing unpaired UTF-16 surrogates are rejected.

```javascript
await client.get("text-key")
await client.get(new Uint8Array([1, 2, 3]))
await client.get(42)
await client.get(42n)
```

### Values

Native values accepted by `set` are converted as follows:

- `null` becomes Null.
- `undefined` becomes Undefined.
- `boolean` becomes Boolean.
- `bigint` becomes an exact Integer.
- `number` becomes an IEEE-754 binary64 Float.
- `string` becomes UTF-8 TextString.
- `Uint8Array` becomes Bytes.
- `Array` becomes an ordered Array.
- `Map` and plain objects become ordered Maps with scalar keys.

```javascript
await client.set("profile", { name: "Ada", active: true })
```

Use the lossless model when float width, raw bits, or exact map keys matter:
`UNDEFINED_VALUE`/`UndefinedValue`, `IntegerValue`, `FloatValue`,
`ByteStringValue`, `TextStringValue`, `ArrayValue`, and `MapValue`.

### Value helpers

- `toValue(value)` converts a native value to the lossless model.
- `encodeStructuredValue(value)` returns one encoded value as
  `Uint8Array`.
- `decodeStructuredValue(bytes)` decodes one complete value.
- `modelEqual(left, right)` compares model values without native coercion.
- `toNative(value)` projects safe lossless values to JavaScript values.
- `decodeNativeValue(bytes)` decodes and projects in one step.
- `toPlainObject(map)` converts a text-keyed lossless map to a
  null-prototype object.
- `ValueLimits` bounds bytes, depth, item count, and integer magnitude.

```javascript
import {
  decodeStructuredValue,
  encodeStructuredValue,
} from "openkache/value-codec"

const encoded = encodeStructuredValue({ count: 1n })
const decoded = decodeStructuredValue(encoded)
```

`toNative` and `decodeNativeValue` preserve integers as `bigint` and bytes
as `Uint8Array`, but reject `UndefinedValue` and `FloatValue` when a native
JavaScript value would lose their distinctions.

### Errors

- `OpenKacheError` — validation, connection, protocol, server, or value
  failure. Inspect its `kind` property for the stable category.
- `OpenKacheUnknownMutationError` — a mutation may have reached the server
  without a confirmed result; do not replay it automatically.
- `StructuredValueError` — invalid structured-value input or a local codec
  failure. Its `kind` property identifies the category.

The public type aliases are `ClientKey`, `NativeValue`, `GetResult`,
`SetOutcome`, `DeleteOutcome`, `StructuredValue`, and `OpenKacheErrorKind`.
The result classes are `FoundResult` and `MissingResult`.

## More information

- [OpenKache on npm](https://www.npmjs.com/package/openkache)
- [OpenKache repository](https://github.com/openkache/openkache)
