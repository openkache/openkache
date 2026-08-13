# OpenKache Client Key Contract — Version 1 Draft

> **Status:** Draft; this contract has not been released or finalized.
>
> This document specifies client-owned key validation and Item ID mapping. It
> is not a server-required key encoding, and it may change before finalization.

For item identity, the wire protocol carries only an opaque Item ID of
`0..=32` bytes. An application MAY invoke the Exact Item ID API to supply that
identifier directly. That API is separate from the key-mapping profiles
specified below.

All lengths in this document are measured in bytes. A byte is exactly 8 bits;
text lengths count UTF-8 bytes, not characters.

The normative terms **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**,
**SHOULD NOT**, and **MAY** have the meanings specified by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) and
[RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when, and only when, they
appear in uppercase.

## 1. Scope and terminology

This specification defines how a client converts an application key into an
Item ID. It uses the following terms:

- **Application key:** The key value supplied by the application before client
  conversion.
- **Typed key:** A language-neutral key value of type `Integer`, `Text`, or
  `Bytes`. `TypedKey` is the API type that represents it.
- **Direct byte key:** A byte-valued application key supplied to
  `ByteKeyOrHash`. Both mapping paths use this byte sequence directly, without
  a CBOR wrapper.
- **Canonical key encoding:** The deterministic encoding rule for a typed key.
  This contract uses deterministic CBOR.
- **Canonical key bytes:** The byte sequence produced by the canonical key
  encoding. It is exactly one complete canonical CBOR item.
  `canonical_key_bytes` is the API and ABI identifier for these bytes.
- **Item ID:** The final opaque identifier carried by the protocol.

`KeyType` selects the one typed-key variant accepted by a client-side
configuration. It is not a wire-protocol namespace.

The specification defines three distinct client-side conversion paths:

```text
application key (typed input)
  -> typed key (TypedKey)
  -> canonical key encoding
  -> canonical key bytes (canonical_key_bytes)
  -> Hash profile (Hash)
  -> ItemId

application key (direct byte key)
  -> byte-key preserve-or-hash profile (ByteKeyOrHash)
  -> Item ID (preservation path for 0..=32 bytes; hash-fallback path otherwise)

exact Item ID
  -> Exact Item ID API
  -> wire
```

These paths are not interchangeable. In particular, the typed key
`TypedKey::Bytes` is CBOR-encoded before hashing, whereas the preservation
path of `ByteKeyOrHash` uses the direct byte key without a CBOR wrapper.
Identical application bytes may therefore produce different Item IDs under
the two profiles.

## 2. Typed key

### 2.1 Typed-key variants (`TypedKey`)

`TypedKey` is the API type for a typed key defined by this contract. Its
variants are `Integer`, `Text`, and `Bytes`:

```text
TypedKey = Integer | Text | Bytes
```

`Text` is a length-delimited sequence of valid UTF-8 bytes. It is not
NUL-terminated; an embedded U+0000 is ordinary text content. `Bytes` is an
exact sequence of bytes, including empty and NUL-containing values.
`Integer` is an exact mathematical value. Native integer width and signedness
are not part of key identity.

For example, `Text("abc")` contains exactly the three UTF-8 bytes `61 62 63`;
it does not contain a trailing `00`. Its canonical key bytes are
`63 61 62 63`. A C or C++ binding MUST pass the text buffer together with its
explicit length.

Objects, arrays, maps, booleans, nulls, decimal values, and custom objects are
not valid typed keys. JSON, reflection, stringification, and implicit coercion
MUST NOT be used.

The typed key `Bytes` is CBOR-encoded and then included in the `Hash` input. It
is not the `OpaqueBytes` value format and is not a direct byte key or an Item
ID.

### 2.2 Typed-key type selection

Every typed-key configuration MUST declare exactly one `KeyType`:

```text
KeyType::Integer
KeyType::Text
KeyType::Bytes
```

Every typed key MUST match the configured `KeyType`. A type mismatch MUST be
rejected before encoding or hashing. `KeyType` adds no wire field and has no
fixed-width integer variants.

`ByteKeyOrHash` is permitted only when the configured type is `KeyType::Bytes`.
It is a mapping policy, not a fourth typed-key type.

### 2.3 Language binding requirements

| Binding | `Text` | `Bytes` | `Integer` | Floating-point |
|---|---|---|---|---|
| JavaScript / TypeScript | `string` → UTF-8 | `Uint8Array`, `Buffer` | `bigint` | safe integer-valued `number` → `Integer` under §3.2; otherwise reject |
| Python | `str` → UTF-8 | `bytes`, buffer types | `int` | `float` rejected |
| Rust | `String`, `&str` → UTF-8 | `&[u8]`, `Vec<u8>` | signed/unsigned integer types | `f32`, `f64` rejected |
| C | caller supplies a canonical Text item | caller supplies a canonical Bytes item | caller supplies a canonical integer item | binary float types rejected |
| C++ | `string_view` convenience overload | `span`/byte string view convenience overload | not exposed by the convenience API | `float`, `double` rejected |
| Go | not exposed by the convenience API | `[]byte` → `Bytes` | not exposed by the convenience API | `float32`, `float64` rejected |
| Java / Kotlin | package scaffold | package scaffold | package scaffold | package scaffold |
| C# / .NET | exact Item ID API only | exact Item ID API only | exact Item ID API only | exact Item ID API only |
| Swift | `String` → `Text` | `Data` → `Bytes` | not exposed by the convenience API | `Float`, `Double` rejected |
| Dart | package scaffold | package scaffold | package scaffold | package scaffold |

All text bindings MUST reject strings that cannot encode to valid UTF-8,
including unpaired surrogates. Text and direct byte-key inputs are
length-delimited; neither representation is NUL-terminated. In particular, C
bindings MUST pass an explicit buffer length and MUST NOT use a NUL-terminated
C string as the key representation.

The legacy shared C ABI `openkache_client_execute` uses
`canonical_key_bytes` as its canonical-key operation input. Its
`application_key` buffer MUST contain exactly one complete canonical CBOR key
item; that ABI does not infer whether arbitrary bytes represent `Text`, `Bytes`,
or an integer. The typed ABI entry points (`openkache_client_execute_typed*`)
instead take logical key bytes plus an explicit `FfiKeySpec`; language adapters
MUST use those entry points for native typed values. Exact Item ID operations
remain separate and accept opaque Item IDs directly.

## 3. Canonical key encoding

### 3.1 Deterministic CBOR

```text
canonical_key_bytes = deterministic_cbor(typed key)
```

The canonical key encoding follows deterministic CBOR
[RFC 8949 §4.2](https://www.rfc-editor.org/rfc/rfc8949#section-4.2):

- Integer encoding uses RFC 8949 preferred serialization, including standard
  bignum tags `2` and `3` when the basic integer types cannot represent the
  value.
- `Text` is exact valid UTF-8. `Bytes` is an exact CBOR byte string.
- Canonical key bytes are exactly one complete CBOR item. Sequences, trailing
  bytes, unknown tags, and non-canonical encodings MUST be rejected.
- There is no floating-point wire type. Accepted JavaScript `number` values
  encode as `Integer`.

Decoders MUST reconstruct the typed key, re-encode it, and compare the result
with the received bytes before accepting them.

### 3.2 JavaScript number normalization

JavaScript `number` is the binding type that represents both integers and
floating-point values. This algorithm is normative:

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

### 3.3 Canonical key bytes: examples

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

## 4. Item ID mapping profiles

Mapping profiles are client-local choices. They have no wire or server field.
Every profile MUST produce an Item ID accepted by the wire protocol.

### 4.1 Hash profile (`Hash`)

Under `Hash`, the client accepts an application key that matches the configured
`KeyType`, encodes it as a typed key, and derives a 32-byte Item ID from its
canonical key bytes. The namespace is bound into the hash input; equal typed
keys in different namespaces therefore produce different Item IDs.

```text
item_id =
  BLAKE3-KEYED-HASH(
    key   = item_id_derivation_key[32],
    input = namespace_id:u64be | canonical_key_bytes
  )
```

### 4.2 Byte-key preserve-or-hash profile (`ByteKeyOrHash`)

`ByteKeyOrHash` applies to direct byte keys when the configured type is
`KeyType::Bytes`:

- A direct byte key whose length is `0..=32` bytes, including empty and
  exactly 32 bytes, follows the preservation path and becomes the Item ID
  byte-for-byte.
- A direct byte key longer than 32 bytes follows the hash-fallback path and
  becomes a 32-byte Item ID.

The preservation path intentionally omits the CBOR `Bytes` wrapper to keep
short Item IDs compact. The hash-fallback path is part of this profile and
MUST remain stable:

```text
hash_fallback_item_id =
  BLAKE3-KEYED-HASH(
    key   = item_id_derivation_key[32],
    input = namespace_id:u64be | direct_byte_key
  )
```

The profile MUST document the hash input and derivation key. The default
profile uses the same namespace-bound input and derivation key as the `Hash`
profile, but does not prepend a CBOR type or length marker to
`direct_byte_key`. A future profile that changes this input MUST use a distinct
profile identifier and MUST NOT reinterpret existing Item IDs.

### 4.3 Exact Item ID API

The Exact Item ID API accepts the final opaque `0..=32`-byte Item ID directly.
It performs no typed-key conversion, canonical encoding, hashing, or namespace
derivation. It still enforces the protocol Item ID length limit.

This API is appropriate when an application already owns a wire identity or
when a benchmark must compare the server against a Redis-style direct-key path.

## 5. Derivation parameters

### 5.1 Namespace binding

`namespace_id` is a positive, server-assigned identity. Clients MUST NOT
synthesize or recycle it. The `Hash` profile and the hash-fallback path bind it
as the first eight bytes of the keyed-hash input:

```text
Hash profile:
  namespace_id:u64be | canonical_key_bytes

Hash-fallback path:
  namespace_id:u64be | direct_byte_key
```

Including the namespace prevents equal keys in different namespaces from
colliding. Consequently, the client MUST resolve the namespace ID before
deriving an Item ID. Omitting the namespace would make cross-namespace IDs
stable and simpler to precompute, but would weaken domain separation and make
accidental cross-namespace reuse easier.

### 5.2 Root key and derivation visibility

`client_root_key` is an application-selected key of exactly 32 bytes. It MAY
be generated randomly or supplied directly. No text-to-key conversion is
defined.

If it is omitted, the default derivation key is derived from 32 zero bytes.
Item IDs remain publicly derivable in this default profile. Supplying a root
key selects a root-bound derivation profile; changing the root changes Item
IDs and requires migration or repopulation.

The root key's use for Item ID derivation is independent from value handling.
Whether values are enveloped, compressed, or encrypted is defined in the
[Client Value Format](VALUE_FORMAT.md), not here. A client MAY use a root-bound
Item ID profile with a value profile that does not encrypt payloads, subject to
the value-format contract.

```text
item_id_derivation_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache item ID derivation root v1",
    material = client_root_key[32]
  )
```

The all-zero root is valid for the default derivation profile. It MUST NOT be
used as a substitute for an explicitly configured secret when a deployment
requires non-public Item IDs.

### 5.3 Hash algorithm and profile compatibility

The derivation algorithm is BLAKE3 keyed hashing with a 32-byte derived key.
The Item ID has no separate version field. A profile that changes key identity,
namespace input, hash context, or derivation material MUST use a distinct
profile identifier and MUST NOT reinterpret Item IDs from this contract. There
is no key-rotation protocol.

## 6. Limits and validation

Every conforming SDK MUST enforce:

```text
MAX_KEY_INPUT_BYTES = 1,048,576  // 1 MiB
```

`MAX_KEY_INPUT_BYTES` limits the key input before canonical encoding or
hashing. Its measured input is:

- `Text`: the exact UTF-8 byte sequence after UTF-8 validation;
- typed `Bytes`: the exact application byte sequence;
- `ByteKeyOrHash`: the exact direct byte-key sequence; and
- `Integer`: the minimal unsigned big-endian magnitude of the mathematical
  value; zero has a magnitude length of zero.

The integer sign is part of the value but is not included in the magnitude
length. No Unicode normalization, NUL terminator, native integer-width
padding, CBOR header, CBOR tag, namespace prefix, hash output, or other
encoding/profile framing contributes to this limit. Oversized key inputs MUST be
rejected before canonical encoding or hashing. Bindings MAY use a lower local
resource limit, but that limit is not part of the shared key contract.

For an input supplied through the canonical key ABI, the binding MUST decode and
validate the complete canonical item, reconstruct the typed key, measure its
key input length, and apply `MAX_KEY_INPUT_BYTES`. It MUST NOT use the encoded
`canonical_key_bytes` length as the contract limit. A binding MAY reject an
encoded buffer earlier under a separately documented local resource guard.

Every client path MUST enforce the wire protocol's `0..=32` Item ID limit.
Empty keys and empty Item IDs are valid.

## 7. Conformance vectors

All byte values below are shown as space-separated hexadecimal bytes. Unless
noted:

```text
namespace_id = 1
client_root_key =
  00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f
  10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f

item_id_derivation_key =
  ef 08 6a 3f 6e 66 3e df 41 08 98 2d 51 2a 4a 1b
  92 22 5d c5 82 e1 06 a0 f0 29 f9 e6 3b 91 7b 4b
```

### 7.1 Hash Item IDs

| Vector | Canonical key bytes | `item_id` |
|---|---|---|
| `Text("abc")` | `63 61 62 63` | `42 7b da c9 1f 3b 4a 91 e6 84 4e df 91 5f de 24 6a 8c 5f fd c6 53 8a b2 73 d9 b3 8a c7 6e d3 b5` |
| `Bytes([00,ff])` | `42 00 ff` | `37 8f 5b c4 75 ef 94 54 49 58 3a e5 a5 34 16 45 35 28 1a 63 44 63 6b 63 ec 88 70 57 6e 0e 7e 41` |
| `Integer(1)` | `01` | `cf 3c 3f db 98 f7 ce 3f 81 a7 04 a3 9b 88 2d e2 a0 58 37 d3 d1 c0 56 81 84 38 23 74 11 88 54 7c` |
| `Integer(-1)` | `20` | `63 3b 3c a8 10 66 f7 25 0c 6a f1 1f 14 62 d3 a1 48 4f 12 16 a0 6f 1e f5 65 46 78 65 c3 28 81 d6` |
| `Text("")` | `60` | `4a 09 7c c0 5f b5 4d 3b e2 1a f5 83 dc b4 b3 d9 e8 d7 6f f6 f6 5b 14 78 5f dd f7 56 d1 aa 13 bc` |
| `Bytes([])` | `40` | `6a 45 bd 6f 9f a7 94 a0 08 9b ed 46 b1 ee f3 af 0f 69 12 ec 08 b5 0a 02 c0 f4 f3 9f be c0 5e cc` |
| `Text("abc")`, namespace `2` | `63 61 62 63` | `8d be e5 54 99 ed f7 43 4c f2 b9 02 42 e7 d3 60 94 05 d2 08 06 a7 e5 27 5b ce 83 b7 94 f8 50 35` |

### 7.2 ByteKeyOrHash boundary vectors

The first two direct byte keys follow the preservation path. The last vector
follows the hash-fallback path for the 33-byte direct byte key.

| Input | Result |
|---|---|
| empty direct byte key | empty Item ID |
| 32-byte direct byte key `00 01 02 ... 1f` | same 32 bytes |
| 33-byte direct byte key `00 01 02 ... 20` | `ee 7e 7b 63 94 f2 96 bc 45 1c 0c 25 50 0f ce e8 37 e4 de 6d 23 88 84 57 5d fb 0f 9f f2 27 07 bc` |

### 7.3 Root separation

For `namespace_id = 1` and `Text("abc")`:

```text
client_root_key =
  ff fe fd fc fb fa f9 f8 f7 f6 f5 f4 f3 f2 f1 f0
  ef ee ed ec eb ea e9 e8 e7 e6 e5 e4 e3 e2 e1 e0

item_id_derivation_key =
  d8 9c 53 52 71 5e 33 2f a0 6b 52 f1 32 52 b5 fb
  d1 b7 dd 5c 75 43 f8 cc 07 cb 9b 18 ad 0e 2b cd

item_id =
  cc f7 df e4 e2 d9 f4 65 4d 00 44 e4 eb 2c 9b 5b
  fc 52 2a 8d 33 0e 7b 4e 9e 30 9a 7d b5 cf b4 80
```

### 7.4 Rejection cases

Clients MUST reject:

- a typed key whose `KeyType` does not match the typed-key configuration;
- a non-canonical CBOR key, including trailing bytes or an unsupported type;
- a key input larger than `MAX_KEY_INPUT_BYTES`;
- an Item ID longer than 32 bytes.
