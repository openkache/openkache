# OpenKache Client Key Contract — Version 1 Draft

> **Status:** Draft; version 1 has not been released or finalized.
>
> This document specifies client-owned key validation and Item ID mapping. It
> is not a server-required key encoding, and it may change before the version 1
> freeze.

For item identity, the wire protocol carries only an opaque Item ID of
`0..=32` octets. An application MAY invoke the Exact Item ID API to supply that
identifier directly. That API is separate from the key-mapping profiles
specified below.

In this document, `octet` denotes an 8-bit unit in an encoded or protocol
representation. `byte` is used for application-facing APIs and profile names;
both terms refer to the same 8-bit unit.

The normative terms **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**,
**SHOULD NOT**, and **MAY** have the meanings specified by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) and
[RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when, and only when, they
appear in uppercase.

## 1. Scope and terminology

This specification separates a logical application key, its canonical
representation, and the final Item ID. `PortableKey` is the logical
application-key model. `KeySpec` selects the one logical type accepted by a
typed keyspace. Here, a keyspace is a client-side key configuration; it is not a
wire-protocol namespace. `canonical_key_bytes` is the deterministic
representation used by the typed-key path. `ItemId` is the final opaque
identifier sent to the server.

The specification defines three distinct client-side conversion paths:

```text
typed application key
  -> PortableKey
  -> canonical_key_bytes
  -> Hash mapping
  -> ItemId

byte-oriented application key
  -> ByteKeyOrHash mapping
  -> ItemId (preserved for 0..=32 octets; hashed otherwise)

exact ItemId
  -> Exact Item ID API
  -> wire
```

These paths are not interchangeable. In particular, `PortableKey::Bytes` is
CBOR-encoded before hashing, whereas the preserved branch of `ByteKeyOrHash`
uses the application bytes without a CBOR wrapper. Identical application bytes
may therefore produce different Item IDs under the two profiles.

## 2. Logical key model

### 2.1 PortableKey types

`PortableKey` denotes the version 1 key-only subset of deterministic CBOR:
`Integer`, `Text`, or `Bytes`.

```text
PortableKey = Integer | Text | Bytes
```

`Text` is a length-delimited sequence of valid UTF-8 octets. It is not
NUL-terminated; an embedded U+0000 is ordinary text content. `Bytes` is an
exact sequence of octets, including empty and NUL-containing values.
`Integer` is an exact mathematical value. Native integer width and signedness
are not part of key identity.

For example, `Text("abc")` contains exactly the three UTF-8 octets
`61 62 63`; it does not contain a trailing `00`. Its canonical CBOR
representation is `63 61 62 63`. A C or C++ binding MUST pass the text buffer
together with its explicit length.

Objects, arrays, maps, booleans, nulls, decimal values, and custom objects are
not key types. JSON, reflection, stringification, and implicit coercion MUST
NOT be used.

`PortableKey::Bytes` is a logical key type. It is CBOR-encoded and then
included in the `Hash` input. It is not the `RawBytes` value codec and is not a
preserved wire Item ID.

### 2.2 KeySpec and mismatch rules

Every typed keyspace MUST declare exactly one `KeySpec`:

```text
KeySpec::Integer
KeySpec::Text
KeySpec::Bytes
```

Every typed key MUST match its keyspace's `KeySpec`. A type mismatch MUST be
rejected before canonicalization or hashing. `KeySpec` adds no wire field and
has no fixed-width integer variants.

`ByteKeyOrHash` is permitted only for a `KeySpec::Bytes` keyspace. It is a
mapping policy, not a fourth `KeySpec`.

### 2.3 Language binding requirements

| Binding | `Text` | `Bytes` | `Integer` | Floating-point |
|---|---|---|---|---|
| JavaScript / TypeScript | `string` → UTF-8 | `Uint8Array`, `Buffer` | `bigint` | safe integer-valued `number` → `Integer` under §3.2; otherwise reject |
| Python | `str` → UTF-8 | `bytes`, buffer types | `int` | `float` rejected |
| Rust | `String`, `&str` → UTF-8 | `&[u8]`, `Vec<u8>` | signed/unsigned integer types | `f32`, `f64` rejected |
| C | caller supplies a canonical Text item | caller supplies a canonical Bytes item | caller supplies a canonical integer item | binary float types rejected |
| C++ | `string_view` convenience overload | `span`/byte string view convenience overload | not exposed by the v1 convenience API | `float`, `double` rejected |
| Go | not exposed by the v1 convenience API | `[]byte` → `Bytes` | not exposed by the v1 convenience API | `float32`, `float64` rejected |
| Java / Kotlin | package scaffold | package scaffold | package scaffold | package scaffold |
| C# / .NET | exact Item ID API only | exact Item ID API only | exact Item ID API only | exact Item ID API only |
| Swift | `String` → `Text` | `Data` → `Bytes` | not exposed by the v1 convenience API | `Float`, `Double` rejected |
| Dart | package scaffold | package scaffold | package scaffold | package scaffold |

All text bindings MUST reject strings that cannot encode to valid UTF-8,
including unpaired surrogates. Text and byte inputs are length-delimited;
neither representation is NUL-terminated. In particular, C bindings MUST pass
an explicit buffer length and MUST NOT use a NUL-terminated C string as the key
representation.

The shared native ABI uses `canonical_key_bytes` as its formatted-operation
input. Its `application_key` buffer MUST contain exactly one complete
canonical CBOR key item; the ABI does not infer whether arbitrary bytes
represent `Text`, `Bytes`, or an integer. Language adapters convert their
native key type before calling the ABI. Exact Item ID operations remain
separate and accept opaque protocol Item IDs directly.

## 3. Canonical representation

### 3.1 Deterministic CBOR

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

### 3.2 JavaScript number normalization

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

### 3.3 Canonical-byte examples

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

### 4.1 Hash

Under `Hash`, the client accepts a `PortableKey` matching the configured
`KeySpec`, canonicalizes it, and derives a 32-byte Item ID. The namespace is
bound into the hash input; equal logical keys in different namespaces
therefore produce different Item IDs.

```text
item_id =
  BLAKE3-KEYED-HASH(
    key   = item_id_derivation_key[32],
    input = namespace_id:u64be | canonical_key_bytes
  )
```

### 4.2 ByteKeyOrHash

`ByteKeyOrHash` applies to byte-oriented `KeySpec::Bytes` keyspaces:

- An input whose length is `0..=32` octets, including empty and exactly
  32 octets, is preserved byte-for-byte as the Item ID.
- An input longer than 32 octets is hashed to a 32-byte Item ID.

The preserved branch intentionally omits the CBOR `Bytes` wrapper to keep short
Item IDs compact. The overflow branch is a separate derivation profile and
MUST remain stable:

```text
overflow_item_id =
  BLAKE3-KEYED-HASH(
    key   = item_id_derivation_key[32],
    input = namespace_id:u64be | byte_key
  )
```

The profile MUST document the hash input and key material. The default v1
profile uses the same namespace-bound input and derivation key as `Hash`, but
does not prepend a CBOR type or length marker to `byte_key`. A future profile
that changes this input MUST use a distinct profile identifier and MUST NOT
reinterpret existing Item IDs.

### 4.3 Exact Item ID API

The Exact Item ID API accepts the final opaque `0..=32`-octet Item ID directly.
It performs no `PortableKey` conversion, canonicalization, hashing, or
namespace derivation. It still enforces the protocol Item ID length limit.

This API is appropriate when an application already owns a wire identity or
when a benchmark must compare the server against a Redis-style direct-key path.

## 5. Derivation parameters

### 5.1 Namespace binding

`namespace_id` is a positive, server-assigned identity. Clients MUST NOT
synthesize or recycle it. `Hash` and the overflow branch of `ByteKeyOrHash`
bind it as the first eight octets of the keyed-hash input:

```text
namespace_id:u64be | key_material
```

Including the namespace prevents equal keys in different namespaces from
colliding. Consequently, the client MUST resolve the namespace ID before
deriving an Item ID. Omitting the namespace would make cross-namespace IDs
stable and simpler to precompute, but would weaken domain separation and make
accidental cross-namespace reuse easier.

### 5.2 Root key and derivation visibility

`client_root_key` is an application-selected key of exactly 32 octets. It MAY
be generated randomly or supplied directly. No text-to-key conversion is
defined.

If it is omitted, the default derivation key is derived from 32 zero octets.
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

The v1 derivation algorithm is BLAKE3 keyed hashing with a 32-byte derived key.
The v1 Item ID has no separate version field. A profile that changes key
identity, namespace input, hash context, or derivation material MUST use a
distinct profile identifier and MUST NOT reinterpret v1 Item IDs. There is no
v1 key-rotation protocol.

## 6. Limits and validation

Every v1 SDK MUST enforce:

```text
MAX_CANONICAL_KEY_BYTES = 1,048,576  // 1 MiB
MAX_BYTE_KEY_BYTES = 1,048,576       // 1 MiB
```

`MAX_CANONICAL_KEY_BYTES` includes the CBOR header and bignum magnitude and
excludes the 8-byte namespace prefix. `MAX_BYTE_KEY_BYTES` applies to the
application byte input and also excludes that prefix. Oversized typed or byte
keys MUST be rejected before hashing. Bindings MAY use a lower local limit.

Every client path MUST enforce the wire protocol's `0..=32` Item ID limit.
Empty keys and empty Item IDs are valid.

## 7. Conformance vectors

All values below are octets separated by spaces. Unless noted:

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

| Vector | `canonical_key_bytes` | `item_id` |
|---|---|---|
| `Text("abc")` | `63 61 62 63` | `42 7b da c9 1f 3b 4a 91 e6 84 4e df 91 5f de 24 6a 8c 5f fd c6 53 8a b2 73 d9 b3 8a c7 6e d3 b5` |
| `Bytes([00,ff])` | `42 00 ff` | `37 8f 5b c4 75 ef 94 54 49 58 3a e5 a5 34 16 45 35 28 1a 63 44 63 6b 63 ec 88 70 57 6e 0e 7e 41` |
| `Integer(1)` | `01` | `cf 3c 3f db 98 f7 ce 3f 81 a7 04 a3 9b 88 2d e2 a0 58 37 d3 d1 c0 56 81 84 38 23 74 11 88 54 7c` |
| `Integer(-1)` | `20` | `63 3b 3c a8 10 66 f7 25 0c 6a f1 1f 14 62 d3 a1 48 4f 12 16 a0 6f 1e f5 65 46 78 65 c3 28 81 d6` |
| `Text("")` | `60` | `4a 09 7c c0 5f b5 4d 3b e2 1a f5 83 dc b4 b3 d9 e8 d7 6f f6 f6 5b 14 78 5f dd f7 56 d1 aa 13 bc` |
| `Bytes([])` | `40` | `6a 45 bd 6f 9f a7 94 a0 08 9b ed 46 b1 ee f3 af 0f 69 12 ec 08 b5 0a 02 c0 f4 f3 9f be c0 5e cc` |
| `Text("abc")`, namespace `2` | `63 61 62 63` | `8d be e5 54 99 ed f7 43 4c f2 b9 02 42 e7 d3 60 94 05 d2 08 06 a7 e5 27 5b ce 83 b7 94 f8 50 35` |

### 7.2 ByteKeyOrHash boundary vectors

The first two vectors are preserved byte-for-byte. The last vector hashes the
33-byte input with the default overflow profile.

| Input | Result |
|---|---|
| empty byte key | empty Item ID |
| 32-byte key `00 01 02 ... 1f` | same 32 octets |
| 33-byte key `00 01 02 ... 20` | `ee 7e 7b 63 94 f2 96 bc 45 1c 0c 25 50 0f ce e8 37 e4 de 6d 23 88 84 57 5d fb 0f 9f f2 27 07 bc` |

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

- a typed key whose `KeySpec` does not match the configured keyspace;
- a non-canonical CBOR key, including trailing bytes or an unsupported type;
- a canonical key larger than `MAX_CANONICAL_KEY_BYTES`;
- a byte key larger than the selected `MAX_BYTE_KEY_BYTES`;
- an Item ID longer than 32 octets.
