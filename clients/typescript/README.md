# OpenKache TypeScript client

Store, read, and delete values in an OpenKache server from TypeScript or
JavaScript. The package runs on Node.js, Bun, and Deno; it is not a browser
client.

[npm package](https://www.npmjs.com/package/openkache) ·
[GitHub source](https://github.com/openkache/openkache/tree/main/clients/typescript)

## Install

Choose the package manager used by your project:

| Tool | Command |
| --- | --- |
| npm | `npm install openkache` |
| pnpm | `pnpm add openkache` |
| Yarn | `yarn add openkache` |
| Bun | `bun add openkache` |
| Deno | `deno add npm:openkache` |

Node.js users need Node.js 20 or newer. The published native adapters currently
support Linux x64/ARM64 (glibc) and Apple Silicon macOS. Windows, Linux musl,
and Intel macOS do not have published native adapters yet.

## Quick start

The example below assumes a local OpenKache server at `127.0.0.1:4433`.
Save it as `index.mjs`; the same code can be used from a TypeScript file.

```javascript
import { OpenKache_Client, to_native } from "openkache"

const client = await OpenKache_Client.connect("127.0.0.1:4433")

try {
  console.log("SET:", await client.set("greeting", "hello"))

  const result = await client.get("greeting")
  console.log(
    "GET:",
    result.kind === "found" ? to_native(result.value) : "missing",
  )

  console.log("DELETE:", await client.delete("greeting"))

  const afterDelete = await client.get("greeting")
  console.log(
    "GET after DELETE:",
    afterDelete.kind === "missing" ? "missing" : "found",
  )
} finally {
  await client.close()
}
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

### Client

| API | Description |
| --- | --- |
| `OpenKache_Client.connect(endpoint)` | Connect to a `host:port` endpoint. Pass either a string or `{ address }`; IPv6 endpoints use `[host]:port`. |
| `client.get(key)` | Resolve to `Found_Result` when the key exists, or `Missing_Result` otherwise. |
| `client.set(key, value)` | Resolve to `"created"` or `"replaced"`. |
| `client.delete(key)` | Resolve to `"deleted"` or `"not_found"`. |
| `client.close()` | Close the connection. Repeated calls are safe. |

`MISSING` is a shared `Missing_Result` instance. A stored `undefined` is still
returned as a `Found_Result`; it is never treated as a missing key.

### Keys

Each operation accepts one of these key types:

| Type | Meaning |
| --- | --- |
| `string` | UTF-8 text, including empty strings. |
| `Uint8Array` | Exact bytes, including empty bytes. |
| `number` | A finite safe integer other than `-0`. |
| `bigint` | A signed 64-bit integer. |

Fractions, `NaN`, infinities, unsafe numbers, objects, arrays, booleans, and
invalid UTF-16 strings are rejected as keys.

### Values

Native values accepted by `set` are converted as follows:

| JavaScript value | Stored value |
| --- | --- |
| `null` | Null |
| `undefined` | Undefined |
| `boolean` | Boolean |
| `bigint` | Exact integer |
| `number` | IEEE-754 binary64 float |
| `string` | UTF-8 text |
| `Uint8Array` | Bytes |
| `Array` | Ordered array |
| `Map` or plain object | Ordered map with scalar keys |

Use the lossless model when float width, raw bits, or exact map keys matter:

- `UNDEFINED_VALUE` / `Undefined_Value`
- `Integer_Value`
- `Float_Value`
- `ByteString_Value`
- `TextString_Value`
- `Array_Value`
- `Map_Value`

The value helpers are `to_value`, `model_equal`,
`encode_structured_value`, and `decode_structured_value`. The
`openkache/value-codec` subpath exports the codec helpers without opening a
network connection:

```javascript
import {
  decode_structured_value,
  encode_structured_value,
} from "openkache/value-codec"
```

`to_native` and `decode_native_value` provide a convenient native projection.
They preserve integers as `bigint` and bytes as `Uint8Array`, but reject
`Undefined_Value` and `Float_Value` because JavaScript primitives would lose
their distinctions. Use the lossless model when those values are needed.
`to_plain_object` converts a lossless map when JavaScript object semantics are
safe.

### Errors

- `OpenKache_Error` — validation, connection, protocol, server, or value
  failure. Inspect its `kind` property for the stable category.
- `OpenKache_Unknown_Mutation_Error` — a mutation may have reached the server
  without a confirmed result; do not replay it automatically.
- `Structured_Value_Error` — invalid structured-value input or a local codec
  failure.

The public type aliases are `Client_Key`, `Native_Value`, `Get_Result`,
`Set_Outcome`, `Delete_Outcome`, `Structured_Value`, and
`OpenKache_Error_Kind`. The result classes are `Found_Result` and
`Missing_Result`.

## More information

- [OpenKache on npm](https://www.npmjs.com/package/openkache)
- [OpenKache repository](https://github.com/openkache/openkache)
