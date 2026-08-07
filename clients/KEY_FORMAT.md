# OpenKache v1 Client Key Format

> **Status: Draft — v1 pre-freeze**
>
> This is the default formatted-key implementation shipped by OpenKache
> clients. It is not a server-required key encoding.

The server receives only a 32-byte Item ID. Applications MAY bypass this
format with the raw client API and supply an exact Item ID directly.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

## 1. Key contract

`PortableKey` is the v1 key-only subset of deterministic CBOR:
`Integer`, `Text`, or `Bytes`. Native integers and safe integer-valued
JavaScript `number` values map to `Integer`; all other types are rejected. This
logical value is not the 32-byte Item ID and does not restrict value codecs.

```text
PortableKey = Integer | Text | Bytes
```

Every formatted keyspace MUST select exactly one `KeySpec`:

```text
KeySpec::Integer
KeySpec::Text
KeySpec::Bytes
```

Every key MUST match its keyspace's `KeySpec`; a mismatch MUST be rejected
before hashing. `KeySpec` adds no wire field and has no fixed-width integer
variants.

```text
native key -> PortableKey -> deterministic CBOR -> canonical_key_bytes
```

`Text` is exact valid UTF-8, `Bytes` is exact bytes, and `Integer` is an exact
mathematical value. Native integer width and signedness are not identity.
Objects, arrays, maps, booleans, nulls, decimal values, and custom objects are
not key types. JSON, reflection, stringification, and implicit coercion MUST
NOT be used.

Key `Bytes` is a logical key type: it is CBOR-encoded and then included in the
Item ID hash. It is not the `RawBytes` value codec and is not a raw 32-byte
Item ID.

### 1.1 Language mapping

| Binding | `Text` | `Bytes` | `Integer` | Floating-point |
|---|---|---|---|---|
| JavaScript / TypeScript | `string` → UTF-8 | `Uint8Array`, `Buffer` | `bigint` | safe integer-valued `number` → `Integer` under §1.2; otherwise reject |
| Python | `str` → UTF-8 | `bytes`, buffer types | `int` | `float` rejected |
| Rust | `String`, `&str` → UTF-8 | `&[u8]`, `Vec<u8>` | signed/unsigned integer types | `f32`, `f64` rejected |
| C | explicit UTF-8 `uint8_t* + length` | `uint8_t* + length` | exposed signed/unsigned integer types | binary float types rejected |
| C++ | length-delimited UTF-8 string view | `span`/byte string view | standard integer types and exposed extensions | `float`, `double` rejected |
| Go | `string` → UTF-8 | `[]byte` | `int`/`uint` and fixed-width integer types | `float32`, `float64` rejected |
| Java / Kotlin | `String` → UTF-8 | `byte[]` / `ByteArray` | fixed-width types and exposed big-integer types | `Float`, `Double` rejected |
| C# / .NET | `string` → UTF-8 | `byte[]`, `ReadOnlySpan<byte>` | integral types, `Int128`/`UInt128`, exposed `BigInteger` | `float`, `double` rejected |
| Swift | `String` → UTF-8 | `Data`, `[UInt8]` | `Int`/`UInt` and fixed-width integer types | `Float`, `Double` rejected |
| Dart | `String` → UTF-8 | `Uint8List` | `int` when represented exactly | `double` rejected |

All text bindings MUST reject strings that cannot encode to valid UTF-8,
including unpaired surrogates. All byte bindings are length-delimited; C
strings MUST NOT be used as a key representation.

### 1.2 JavaScript number normalization

JavaScript `number` is the one v1 binding type that represents both integers
and floating-point values. This algorithm is normative:

```js
function normalizeJavaScriptNumber(x) {
  if (Object.is(x, -0) || !Number.isSafeInteger(x)) {
    throw new TypeError("key number is not a supported integer")
  }
  return Integer(BigInt(x))
}
```

`Number.isSafeInteger` supplies the finite, integral, and range checks.
`Object.is` rejects negative zero. `BigInt(x)` is exact after those checks.

```text
JavaScript number 1, JavaScript 1n -> Integer(1)
JavaScript number 1.5, -0, NaN, Infinity, unsafe number -> rejected
```

### 1.3 Canonical key bytes

```text
canonical_key_bytes = deterministic_cbor(PortableKey)
```

The key codec follows deterministic CBOR
[RFC 8949 §4.2](https://www.rfc-editor.org/rfc/rfc8949#section-4.2):

- Integer encoding uses RFC 8949 preferred serialization, including standard
  bignum tags `2` and `3` when the basic integer types cannot represent the
  value.
- `Text` is exact valid UTF-8. `Bytes` is an exact CBOR byte string.
- A key is exactly one complete CBOR item. Sequences, trailing bytes,
  unknown tags, and non-canonical encodings MUST be rejected.
- There is no floating-point wire type. Accepted JavaScript `number` values
  encode as `Integer`.

Decoders MUST re-encode the logical key and compare the bytes before accepting
them.

Key-byte vectors:

```text
Text("abc")     -> 63 61 62 63
Bytes([00,ff])  -> 42 00 ff
Integer(1)      -> 01
Integer(-1)     -> 20
Text("")        -> 60
Bytes([])       -> 40
```

The following are rejected:

```text
18 01                         // overlong Integer(1)
fa 3f 80 00 00                // binary32 Float(1.0)
80                            // array
f4                            // boolean
f6                            // null
60 00                         // trailing bytes
```

## 2. Item ID derivation

`client_root_key` is an application-selected key of exactly 32 octets. It MAY
be generated randomly or supplied directly. No text-to-key conversion is
defined.

If it is omitted, the default is 32 zero octets. This selects the unprotected
formatted-value mode; it does not bypass key conversion or Item ID derivation.
Item IDs are publicly derivable in this mode. Supplying a root key selects the
protected value mode; changing the root changes Item IDs and requires
migration or repopulation. See [Value Format](VALUE_FORMAT.md).

```text
item_id_derivation_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache item ID derivation root v1",
    material = client_root_key[32]
  )

item_id =
  BLAKE3-KEYED-HASH(
    key   = item_id_derivation_key[32],
    input = namespace_id:u64be | deterministic_cbor(PortableKey)
  )
```

`namespace_id` is a positive, server-assigned identity. Clients MUST NOT
synthesize or recycle it. The namespace is bound before hashing, so equal
keys in different namespaces have different Item IDs.

The v1 Item ID has no separate version field. A profile that changes key
identity or derivation material MUST use a distinct context and MUST NOT
reinterpret v1 Item IDs. There is no v1 key-rotation protocol.

## 3. Resource policy

Every v1 SDK MUST enforce:

```text
MAX_CANONICAL_KEY_BYTES = 1,048,576  // 1 MiB
```

The limit includes the CBOR header and bignum magnitude and excludes the
8-byte namespace prefix. Oversized keys MUST be rejected before hashing.
Bindings MAY use a lower local limit, but all conforming SDKs share the 1 MiB
interoperability limit.

## 4. Item ID conformance vectors

All values below are octets separated by spaces. Unless noted:

```text
client_root_key =
  00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f
  10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f

item_id_derivation_key =
  ef 08 6a 3f 6e 66 3e df 41 08 98 2d 51 2a 4a 1b
  92 22 5d c5 82 e1 06 a0 f0 29 f9 e6 3b 91 7b 4b
```

| Vector | `namespace_id` | `canonical_key_bytes` | `item_id_input` | 32-byte `item_id` |
|---|---:|---|---|---|
| `Text("abc")` | `1` | `63 61 62 63` | `00 00 00 00 00 00 00 01 63 61 62 63` | `42 7b da c9 1f 3b 4a 91 e6 84 4e df 91 5f de 24 6a 8c 5f fd c6 53 8a b2 73 d9 b3 8a c7 6e d3 b5` |
| `Bytes([00,ff])` | `1` | `42 00 ff` | `00 00 00 00 00 00 00 01 42 00 ff` | `37 8f 5b c4 75 ef 94 54 49 58 3a e5 a5 34 16 45 35 28 1a 63 44 63 6b 63 ec 88 70 57 6e 0e 7e 41` |
| `Integer(1)`; JS `number 1`, JS `1n` | `1` | `01` | `00 00 00 00 00 00 00 01 01` | `cf 3c 3f db 98 f7 ce 3f 81 a7 04 a3 9b 88 2d e2 a0 58 37 d3 d1 c0 56 81 84 38 23 74 11 88 54 7c` |
| `Integer(-1)` | `1` | `20` | `00 00 00 00 00 00 00 01 20` | `63 3b 3c a8 10 66 f7 25 0c 6a f1 1f 14 62 d3 a1 48 4f 12 16 a0 6f 1e f5 65 46 78 65 c3 28 81 d6` |
| `Text("")` | `1` | `60` | `00 00 00 00 00 00 00 01 60` | `4a 09 7c c0 5f b5 4d 3b e2 1a f5 83 dc b4 b3 d9 e8 d7 6f f6 f6 5b 14 78 5f dd f7 56 d1 aa 13 bc` |
| `Bytes([])` | `1` | `40` | `00 00 00 00 00 00 00 01 40` | `6a 45 bd 6f 9f a7 94 a0 08 9b ed 46 b1 ee f3 af 0f 69 12 ec 08 b5 0a 02 c0 f4 f3 9f be c0 5e cc` |
| `Text("abc")`, namespace separation | `2` | `63 61 62 63` | `00 00 00 00 00 00 00 02 63 61 62 63` | `8d be e5 54 99 ed f7 43 4c f2 b9 02 42 e7 d3 60 94 05 d2 08 06 a7 e5 27 5b ce 83 b7 94 f8 50 35` |

Root-key separation vector (`namespace_id = 1`, key `Text("abc")`):

```text
client_root_key =
  ff fe fd fc fb fa f9 f8 f7 f6 f5 f4 f3 f2 f1 f0
  ef ee ed ec eb ea e9 e8 e7 e6 e5 e4 e3 e2 e1 e0

item_id_derivation_key =
  d8 9c 53 52 71 5e 33 2f a0 6b 52 f1 32 52 b5 fb
  d1 b7 dd 5c 75 43 f8 cc 07 cb 9b 18 ad 0e 2b cd

item_id_input =
  00 00 00 00 00 00 00 01 63 61 62 63

item_id =
  cc f7 df e4 e2 d9 f4 65 4d 00 44 e4 eb 2c 9b 5b
  fc 52 2a 8d 33 0e 7b 4e 9e 30 9a 7d b5 cf b4 80
```

The raw API bypasses this conversion and derivation entirely.
