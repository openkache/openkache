# OpenKache Client Key Contract — Version 1 Draft

> **Status:** Draft; this contract has not been released or finalized.
>
> This document specifies client-owned key validation and Item ID mapping. It
> is not a server-required key encoding, and it may change before finalization.

This is the target contract for the pre-freeze draft. Client implementations
may temporarily lag while the draft is being completed, but an implementation
MUST NOT claim conformance until it follows this complete mapping contract.

The shared implementation used by OpenKache-maintained language bindings is
described by the [Client Implementation Guide](CLIENT.md).

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
- **Canonical key encoding:** The deterministic encoding rule for a typed key.
  This contract uses deterministic CBOR.
- **Canonical key bytes:** The byte sequence produced by the canonical key
  encoding. It is exactly one complete canonical CBOR item.
  `canonical_key_bytes` is the API and ABI identifier for these bytes.
- **Item ID:** The final opaque identifier carried by the protocol.

`KeyType` selects the one typed-key variant accepted by a typed-key operation.
An adapter MAY select it globally at connection time or per operation. It is
not a wire-protocol namespace.

The specification defines three distinct client-side conversion paths:

```text
application key (typed input)
  -> typed key (TypedKey)
  -> canonical key encoding
  -> canonical key bytes (canonical_key_bytes)
  -> Hash
  -> ItemId

application key (typed input)
  -> typed key (TypedKey)
  -> canonical key encoding
  -> canonical key bytes (canonical_key_bytes)
  -> CanonicalKeyOrHash
  -> Item ID (canonical bytes when 0..=32 bytes; public hash otherwise)

exact Item ID
  -> Exact Item ID API
  -> wire
```

Both formatted profiles use the same typed key and canonical encoding.
`Hash` always produces a root- and namespace-bound 32-byte Item ID.
`CanonicalKeyOrHash` exposes short canonical key bytes directly and uses a
public hash only when the encoding does not fit. The selected mapping profile
is therefore part of item identity and MUST remain stable for addressable data.

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

Both formatted mapping profiles encode `Bytes` as a CBOR byte string. It is not
the `OpaqueBytes` value format and does not mean an Exact Item ID.

### 2.2 Typed-key type selection

Every typed-key operation MUST select exactly one `KeyType`:

```text
KeyType::Integer
KeyType::Text
KeyType::Bytes
```

Every typed key MUST match the `KeyType` selected for that operation. A type
mismatch MUST be rejected before encoding or hashing. `KeyType` adds no wire
field and has no fixed-width integer variants. When a connection-level
`KeyType` is configured, it is the default operation selection; a typed ABI
operation MAY supply its own explicit discriminator.

`Hash` and `CanonicalKeyOrHash` both accept every `KeyType`. A mapping profile
is not a fourth typed-key type.

### 2.3 Binding projection requirements

A language binding MAY expose any subset of `Integer`, `Text`, and `Bytes`, but
it MUST document the supported native types and map them to the language-neutral
variants above without implicit stringification or cross-type coercion.
Binding-specific type names and current capability inventories belong in the
binding documentation, not this format contract.

Every text projection MUST reject input that cannot encode to valid UTF-8,
including unpaired surrogates. Text and byte-string inputs are length-delimited;
neither representation is NUL-terminated. Foreign-function interfaces MUST
carry an explicit buffer length and discriminator rather than inferring a type
or boundary from the byte contents.

Floating-point inputs are not typed keys. A binding MUST reject them except for
the JavaScript safe-integer normalization defined in §3.2. An ABI that
transports an integer as text MUST use canonical signed decimal ASCII:
optional leading `-`, one or more digits, no leading zeroes unless the value is
exactly `0`, and no `-0`. Whitespace, a leading `+`, non-ASCII digits, and
other numeric spellings MUST be rejected. The decimal text is only a transport
representation; key identity remains the mathematical integer encoded under
§3.1.

An interface that accepts `canonical_key_bytes` MUST validate exactly one
complete canonical CBOR key item. An interface that accepts a logical typed key
MUST receive an explicit `KeyType` and perform canonical encoding itself.
Exact Item ID operations remain separate and accept opaque Item IDs directly.

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
    input = 01 | namespace_id:u64be | canonical_key_bytes
  )
```

The leading `01` is the one-byte `Hash` profile domain. It is part of the hash
input, not the resulting Item ID and not a wire field.

### 4.2 Canonical preserve-or-hash profile (`CanonicalKeyOrHash`)

`CanonicalKeyOrHash` accepts an application key that matches the configured
`KeyType` and encodes it exactly as specified in §3:

- Canonical key bytes whose encoded length is `1..=32` become the Item ID
  byte-for-byte.
- Canonical key bytes longer than 32 bytes use the public hash-fallback path
  and become a 32-byte Item ID.

A valid typed key always has a nonempty canonical encoding, including
`Text("")` as `60` and `Bytes([])` as `40`. This formatted profile therefore
does not produce an empty Item ID.

The length decision applies to the complete canonical CBOR encoding, not the
logical payload alone. A `Text` or `Bytes` payload of 30 bytes encodes to
exactly 32 bytes and is preserved; a 31-byte payload encodes to 33 bytes and
uses the hash fallback. Basic CBOR integers occupy at most nine bytes and are
preserved. Larger integers may use a hash fallback when their tagged canonical
encoding exceeds 32 bytes.

The fallback is an unkeyed standard BLAKE3 hash with the profile domain
prepended to its input:

```text
hash_fallback_item_id =
  BLAKE3-HASH(
    input = 02 | canonical_key_bytes
  )
```

The leading `02` is the one-byte `CanonicalKeyOrHash` hash-fallback domain. It
is not added to preserved Item IDs. Neither path uses `namespace_id`,
`item_id_root_key`, or `item_id_derivation_key`. Consequently, equal typed
keys produce equal Item IDs across namespaces and client configurations under
this profile; the wire-level `(namespace_id, item_id)` pair still identifies a
distinct server item.

This profile provides no key confidentiality. Its purpose is a compact,
language-neutral mapping whose short-key path avoids per-key hashing, including
for workloads that compare OpenKache with servers that accept public
application keys. Applications requiring non-public or namespace-dependent
Item ID bytes MUST use `Hash`.

Every hashed path MUST document its exact algorithm, input framing, domain, and
whether it uses secret key material. Future hashed profiles MUST use a new
nonzero domain byte and MUST NOT reinterpret existing Item IDs. Domain bytes
are client-profile identifiers; they are not negotiated with or interpreted by
the server.

### 4.3 Exact Item ID API

The Exact Item ID API accepts the final opaque `0..=32`-byte Item ID directly.
It performs no typed-key conversion, canonical encoding, hashing, or namespace
derivation. It still enforces the protocol Item ID length limit.

This API is appropriate when an application already owns a wire identity or
when a benchmark must compare the server against a Redis-style direct-key path.

## 5. Derivation parameters

### 5.1 Namespace binding

`namespace_id` is a positive, server-assigned identity. Clients MUST NOT
synthesize or recycle it. The `Hash` profile binds it as the first eight bytes
after the profile domain:

```text
Hash profile:
  01 | namespace_id:u64be | canonical_key_bytes
```

Including the namespace makes equal typed keys produce different `Hash` Item
IDs in different namespaces. Consequently, a client MUST resolve the namespace
ID before deriving a `Hash` Item ID.

`CanonicalKeyOrHash` deliberately omits the namespace from both paths. The
server already scopes item identity by namespace, so the same public Item ID in
two namespaces addresses two distinct items.

### 5.2 Item ID root key and derivation visibility

For `Hash`, `item_id_root_key` is an application-selected key of exactly 32
bytes. It MAY be generated randomly or supplied directly. No text-to-key
conversion is defined.

If it is omitted, the default derivation key is derived from 32 zero bytes.
Item IDs remain publicly derivable in this default profile. Supplying a root
key selects a root-bound derivation profile; changing the root changes Item
IDs and requires migration or repopulation.

For `Hash`, the Item ID root key is an identity setting, not a value-protection
key. It MUST remain stable for the lifetime of addressable data.
Value-protection keys rotate independently and are selected from the value
envelope as defined by the [Client Value Format](VALUE_FORMAT.md). A value-key
rotation MUST NOT change Item ID derivation. A client MAY use root-bound Item
IDs with unprotected values, or publicly derivable Item IDs with protected
values.

```text
item_id_derivation_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache item ID derivation root v1",
    material = item_id_root_key[32]
  )
```

The all-zero root is valid for the default derivation profile. It MUST NOT be
used as a substitute for an explicitly configured secret when a deployment
requires non-public Item IDs.

`CanonicalKeyOrHash` ignores `item_id_root_key` and the resulting
`item_id_derivation_key`. Changing either has no effect on its Item IDs.

### 5.3 Hash algorithm and profile compatibility

`Hash` uses BLAKE3 keyed hashing with a 32-byte derived key.
`CanonicalKeyOrHash` uses standard unkeyed BLAKE3 only for canonical encodings
longer than 32 bytes. The currently assigned hashed-profile domains are:

| Domain byte | Profile path |
|---:|---|
| `01` | `Hash` |
| `02` | `CanonicalKeyOrHash` hash fallback |

`00` and `03..=FF` are unassigned. The Item ID has no separate version field.
A profile that changes key identity, namespace input, hash context, derivation
material, or input framing MUST use a distinct nonzero domain byte and MUST
NOT reinterpret Item IDs from this contract. There is no in-band Item ID key
rotation protocol. Changing `item_id_root_key` changes only `Hash` Item IDs and
is an identity migration, not a value-key rotation.

## 6. Limits and validation

Every conforming SDK MUST enforce:

```text
MAX_KEY_INPUT_BYTES = 1,048,576  // 1 MiB
```

`MAX_KEY_INPUT_BYTES` limits the key input before canonical encoding or
hashing. Its measured input is:

- `Text`: the exact UTF-8 byte sequence after UTF-8 validation;
- typed `Bytes`: the exact application byte sequence;
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
Empty logical keys and empty Exact Item IDs are valid. `CanonicalKeyOrHash`
represents an empty `Text` or `Bytes` key by its nonempty one-byte CBOR encoding.

## 7. Conformance vectors

All byte values below are shown as space-separated hexadecimal bytes. Unless
noted:

```text
namespace_id = 1
item_id_root_key =
  00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f
  10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f

item_id_derivation_key =
  ef 08 6a 3f 6e 66 3e df 41 08 98 2d 51 2a 4a 1b
  92 22 5d c5 82 e1 06 a0 f0 29 f9 e6 3b 91 7b 4b
```

`Hash` vectors include domain byte `01`. `CanonicalKeyOrHash` fallback vectors
use an unkeyed BLAKE3 input beginning with domain byte `02`; they ignore the
namespace and Item ID root parameters above.

### 7.1 Hash Item IDs

| Vector | Canonical key bytes | `item_id` |
|---|---|---|
| `Text("abc")` | `63 61 62 63` | `5b 04 c6 3d ba fe 77 0b 24 9d 2b 0f 9b bf 29 b1 ed c7 da b6 41 79 7f 5a 0c 9a 70 62 6c 5e 9c d5` |
| `Bytes([00,ff])` | `42 00 ff` | `20 1c f1 4c 7b c6 4d ab aa e2 c5 7e 7a 62 b8 14 21 f6 27 2d f4 c0 79 7d 92 c4 4a 22 1c de 64 c3` |
| `Integer(1)` | `01` | `fa 6e 82 8f 07 42 ab 6f c6 5d 5e 74 53 6c a9 86 22 45 b3 2e 55 d7 cb aa 73 c6 7c 5c 3e 8c 54 4c` |
| `Integer(-1)` | `20` | `5d de e7 b9 d9 01 97 4c 70 f5 9b ac 00 92 99 30 8d 22 39 ec e6 5b 28 3c e3 cc 8f 63 93 45 f9 6a` |
| `Text("")` | `60` | `6a 67 03 b8 32 5e fc 87 ef 67 ba 94 92 08 18 1b 31 69 c6 8d 66 57 d6 b5 3b 10 6d 5e 6f 62 49 a3` |
| `Bytes([])` | `40` | `6d 0d 3c a0 c4 49 b9 0f 8b 18 fa 50 b1 6f d3 3e 9c 70 d9 10 94 ab 3f b2 60 64 22 92 ee eb fc 2f` |
| `Text("abc")`, namespace `2` | `63 61 62 63` | `9b 5e ed be c7 7a 5a 57 c0 a9 df cf 40 3d cf 98 80 fa 9a ae 54 23 e1 c0 54 d3 79 53 28 1f 8e 7b` |

### 7.2 CanonicalKeyOrHash boundary vectors

The first four vectors follow the canonical preservation path. The final
vector is a 31-byte logical byte string whose 33-byte canonical encoding uses
the public hash fallback.

| Typed key | Canonical key bytes | `item_id` |
|---|---|---|
| `Integer(1)` | `01` | `01` |
| `Text("abc")` | `63 61 62 63` | `63 61 62 63` |
| `Bytes([])` | `40` | `40` |
| `Bytes([00,01,...,1d])` | `58 1e 00 01 02 ... 1d` | `58 1e 00 01 02 ... 1d` |
| `Bytes([00,01,...,1e])` | `58 1f 00 01 02 ... 1e` | `1c 7f 2d 3e a0 59 98 86 a8 45 19 6d d7 59 bf 6d e6 6e 10 8f aa 22 72 d0 a9 14 f8 17 15 77 6c e2` |

### 7.3 Item ID root separation

For `namespace_id = 1` and `Text("abc")`:

```text
item_id_root_key =
  ff fe fd fc fb fa f9 f8 f7 f6 f5 f4 f3 f2 f1 f0
  ef ee ed ec eb ea e9 e8 e7 e6 e5 e4 e3 e2 e1 e0

item_id_derivation_key =
  d8 9c 53 52 71 5e 33 2f a0 6b 52 f1 32 52 b5 fb
  d1 b7 dd 5c 75 43 f8 cc 07 cb 9b 18 ad 0e 2b cd

item_id =
  ab 54 07 91 c1 a7 86 db 25 70 be 9b 4d 7b 71 7c
  40 9c 85 5c dd fa c3 02 7b 18 f0 48 25 ab cd 85
```

### 7.4 Rejection cases

Clients MUST reject:

- a typed key whose `KeyType` does not match the typed-key configuration;
- a non-canonical CBOR key, including trailing bytes or an unsupported type;
- a key input larger than `MAX_KEY_INPUT_BYTES`;
- an Item ID longer than 32 bytes.
