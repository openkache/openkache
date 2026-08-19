# OpenKache Client Key Contract — Version 1 Draft

> **Status:** Draft `draft-2026-08-19.4`; not released or finalized.
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

Deterministic CBOR in this document is limited to the client key-to-Item-ID
contract. It does not define structured value serialization; value semantics
and the initial value codec profile are specified separately in
[`value/SPEC.md`](value/SPEC.md).

An **Item ID mapping profile** selects the algorithm that converts canonical
key bytes into the final Item ID. The initial profiles are `NamespaceHash` and
`PublicKeyOrHash`. A mapping profile is client-local configuration, not a
wire-visible field or a server-enforced namespace policy. A client may select a
profile per operation, so one namespace may contain Item IDs produced by
different profiles. The same logical item remains addressable only when later
operations select the same profile and identity settings.

`KeyType` is the language-neutral name of the inferred `TypedKey` variant:
`Integer`, `Text`, or `Bytes`. A language adapter MUST infer this variant from
each native input when the input has an unambiguous mapping. It MUST reject
inputs that cannot be represented by one of these variants rather than
stringifying or coercing them. `KeyType` is not a namespace setting, schema,
or wire-protocol field.

The specification defines three distinct client-side conversion paths:

```text
application key (typed input)
  -> typed key (TypedKey)
  -> canonical key encoding
  -> canonical key bytes (canonical_key_bytes)
  -> NamespaceHash
  -> Item ID

application key (typed input)
  -> typed key (TypedKey)
  -> canonical key encoding
  -> canonical key bytes (canonical_key_bytes)
  -> PublicKeyOrHash
  -> Item ID (canonical bytes when 0..=32 bytes; public hash otherwise)

exact Item ID
  -> Exact Item ID API
  -> wire
```

Both formatted profiles use the same typed key and canonical encoding.
`NamespaceHash` always produces a root- and namespace-bound 32-byte Item ID.
`PublicKeyOrHash` exposes short canonical key bytes directly and uses an
unkeyed hash only when the encoding does not fit. It is intended for
applications that trust the server and do not require client-side key
confidentiality or profile isolation. The selected mapping profile is part of
item identity.

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
`Integer` is a signed 64-bit value. Bindings MUST reject values outside
`-2^63..=2^63-1`.

For example, `Text("abc")` contains exactly the three UTF-8 bytes `61 62 63`;
it does not contain a trailing `00`. Its canonical key bytes are
`63 61 62 63`. A C or C++ binding MUST pass the text buffer together with its
explicit length.

Objects, arrays, maps, booleans, nulls, decimal values, and custom objects are
not valid typed keys. JSON, reflection, stringification, and implicit coercion
MUST NOT be used.

Both formatted mapping profiles encode `Bytes` as a CBOR byte string. It is not
the `OpaqueBytes` value format and does not mean an Exact Item ID.

The selected mapping profile and `KeyType` MAY vary between operations,
including within one namespace. The canonicalization algorithm remains fixed
by this specification. The server does not know which profile or native key
type produced an Item ID; selecting different identity inputs may appear as a
miss or collision.

There is deliberately no namespace-level key-type policy in v1. A namespace
does not declare an allowed key type, and the server does not reject an item
because its client used `Integer`, `Text`, or `Bytes`. This keeps ordinary
cache namespaces free-form: applications may use a string key for one item
and a byte or integer key for another. Applications that need a schema or a
single key convention enforce it in their own client wrapper.

### 2.2 Native type inference

The language adapter MUST infer exactly one `KeyType` from the native input
before canonical encoding:

| Native input category | Inferred `KeyType` |
|---|---|
| Signed 64-bit integer value | `Integer` |
| Valid UTF-8 text/string | `Text` |
| Explicit byte sequence (`Bytes`, `byte[]`, `Buffer`, or equivalent) | `Bytes` |

The inference is language-specific only at the boundary; the resulting
`TypedKey` and canonical bytes are language-independent. An adapter MUST
reject a value when it cannot determine one of these categories without
coercion. Examples include floating-point values, booleans, null, arrays,
objects, maps, and custom objects.

JavaScript `number` follows §3.2: a finite safe integer infers `Integer`, and
all other numbers are rejected. JavaScript `bigint` infers `Integer` only when
it fits signed `i64`.
Python `bool` MUST be rejected before Python's integer-subclass behavior is
considered; Python `int` infers `Integer`, `str` infers `Text`, and an
explicit bytes-like value infers `Bytes`.

An adapter MAY expose an explicit typed constructor for FFI or generic
containers, but that constructor must produce the same three `TypedKey`
variants and must not introduce a fourth key type. Both mapping profiles accept
all three variants.

The normative `Integer` value is exactly signed `i64`. Python integers and
JavaScript `bigint` values outside that range MUST be rejected. A binding MAY
offer a narrower convenience API, but it MUST NOT accept values that another
conforming binding cannot represent.

The maintained-client default mapping profile is `NamespaceHash`.
`PublicKeyOrHash` MUST be selected explicitly. It is suitable for
benchmarks and for applications that fully trust the server and accept public,
cross-client key identity. Value protection is independent: either mapping
profile may carry protected or unprotected values.

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
other numeric spellings MUST be rejected. The value MUST fit signed `i64`. The
decimal text is only a transport representation; key identity remains the
integer encoded under §3.1.

An interface that accepts `canonical_key_bytes` MUST validate exactly one
complete canonical CBOR key item. An interface that accepts a logical typed key
MUST infer or receive an explicit discriminator and perform canonical encoding
itself.
Exact Item ID operations remain separate and accept opaque Item IDs directly.

## 3. Canonical key encoding

### 3.1 Deterministic CBOR

```text
canonical_key_bytes = deterministic_cbor(typed key)
```

The canonical key encoding follows deterministic CBOR
[RFC 8949 §4.2](https://www.rfc-editor.org/rfc/rfc8949#section-4.2):

- The accepted subset contains only major types 0/1 integers, major type 2 byte
  strings, and major type 3 text strings.
- Indefinite-length items, tags, sequences, and trailing bytes are invalid.
- Integer encoding uses RFC 8949 preferred serialization for signed `i64`.
  A decoder MUST reject major-type values outside the signed `i64` range.
  CBOR bignum tags are not part of v1.
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
JavaScript bigint outside signed i64 -> rejected
```

### 3.3 Canonical key bytes: examples

```text
Text("abc")     -> 63 61 62 63
Bytes([00,ff])  -> 42 00 ff
Integer(1)      -> 01
Integer(-1)     -> 20
Integer(i64::MAX) -> 1b 7f ff ff ff ff ff ff ff
Integer(i64::MIN) -> 3b 7f ff ff ff ff ff ff ff
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
Integer(9223372036854775808)  // outside signed i64
```

## 4. Item ID mapping profiles

Item ID mapping profiles are client-local choices. They have no wire or server
field.
Every profile MUST produce an Item ID accepted by the wire protocol.

### 4.1 Namespace hash profile (`NamespaceHash`)

Under `NamespaceHash`, the adapter infers one supported `TypedKey`, encodes it, and
derives a 32-byte Item ID. Equal typed keys in different namespaces produce
different Item IDs.

```text
item_id =
  BLAKE3-KEYED-HASH(
    key   = item_id_derivation_key[32],
    input = "openkache/item-id/namespace-hash/v1"
          | namespace_id:u64be
          | canonical_key_bytes
  )
```

The domain string identifies the mapping revision. The full 32-byte BLAKE3
output is the Item ID. The server treats it as opaque.

### 4.2 Public preserve-or-hash profile (`PublicKeyOrHash`)

`PublicKeyOrHash` accepts any supported typed application key and encodes it
exactly as specified in §3:

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
uses the hash fallback. Signed `i64` values occupy at most nine bytes and are
preserved.

The fallback uses a domain-separated unkeyed BLAKE3 hash:

```text
hash_fallback_item_id =
  BLAKE3-HASH(
    "openkache/item-id/public-key-or-hash/v1"
    | canonical_key_bytes
  )
```

Neither path uses `namespace_id`,
`item_id_root_key`, or `item_id_derivation_key`. Consequently, equal typed
keys produce equal Item IDs across namespaces and client configurations under
this profile; the wire-level `(namespace_id, item_id)` pair still identifies a
distinct server item.

This profile provides no client-side key confidentiality, namespace binding,
or root-key isolation. It is useful when the server is fully trusted, when
cross-client public identity is desired, or when avoiding hashing for short
keys matters. Applications requiring those omitted guarantees MUST use
`NamespaceHash`.

The distinct domain strings separate the two hash inputs. There is no
profile marker in the 32-byte output: a `PublicKeyOrHash` preserved encoding
can intentionally or accidentally equal a hash-profile output, and the server
does not distinguish those cases. Future revisions MUST use new domain strings
and MUST NOT reinterpret existing Item IDs.

This profile does not provide collision resistance between its preserve and
hash branches, or between mapping profiles, against a caller that can choose
keys. That is not a server security boundary: the same caller may use the Exact
Item ID API to select any wire-valid Item ID. An application that exposes only
mapped keys to less-trusted callers must enforce its own key namespace and
profile boundary or use `NamespaceHash` exclusively.

### 4.3 Exact Item ID API

The Exact Item ID API accepts the final opaque `0..=32`-byte Item ID directly.
It performs no typed-key conversion, canonical encoding, hashing, or namespace
derivation. It still enforces the protocol Item ID length limit.

This API is appropriate when an application already owns a wire identity. It
is a dangerous escape hatch: the caller, not the client profile, owns identity,
collision avoidance, and isolation.

The wire accepts an empty Exact Item ID. Maintained high-level APIs reject it
by default and require an explicit opt-in; low-level wire-operation APIs
preserve the full wire range.

## 5. Derivation parameters

### 5.1 Namespace binding

`namespace_id` is a positive, server-assigned identity. `NamespaceHash` binds it
after the domain string:

```text
NamespaceHash input:
  "openkache/item-id/namespace-hash/v1"
  | namespace_id:u64be
  | canonical_key_bytes
```

Including the namespace makes equal typed keys produce different
`NamespaceHash` Item IDs in different namespaces.

`PublicKeyOrHash` deliberately omits the namespace from both paths. The
server already scopes item identity by namespace, so the same public Item ID in
two namespaces addresses two distinct items.

### 5.2 Item ID root key and derivation visibility

For `NamespaceHash`, `item_id_root_key` is an application-selected key of exactly 32
bytes. It MAY be generated randomly or supplied directly. No text-to-key
conversion is defined.

If it is omitted, the default derivation key is derived from 32 zero bytes.
Item IDs remain publicly derivable in this default profile. Supplying a root
key selects a root-bound derivation profile; changing the root changes Item
IDs and requires migration or repopulation.

For `NamespaceHash`, the Item ID root key is an identity setting, not a value-protection
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
used as a substitute for an explicitly configured secret when an application
requires non-public Item IDs.

`PublicKeyOrHash` ignores `item_id_root_key` and the resulting
`item_id_derivation_key`. Changing either has no effect on its Item IDs.

Profile persistence and mismatch detection remain intentionally unspecified in
v1. APIs MUST make per-operation profile selection explicit when it differs
from the client default; a mismatch may still appear as a miss.

### 5.3 Mapping algorithm and profile compatibility

`NamespaceHash` uses BLAKE3 keyed hashing with a 32-byte derived key.
`PublicKeyOrHash` uses standard unkeyed BLAKE3 only for canonical encodings
longer than 32 bytes. Each hashed path uses its exact domain string from §4.

A profile that changes identity, namespace input, hash context, derivation
material, or input framing MUST use a new domain string and MUST NOT
reinterpret existing Item IDs. There is no in-band root-key mismatch detector:
the wrong root normally appears as a miss. Changing `item_id_root_key` is an
identity migration, not a value-key rotation.

## 6. Limits and validation

Every conforming SDK MUST enforce:

```text
MAX_CANONICAL_KEY_BYTES = 1,048,576  // 1 MiB
```

This limit applies to the complete canonical CBOR item, including its type,
tag, and length bytes. It therefore measures the bytes actually validated and
hashed uniformly across all key types. An adapter SHOULD reject obviously
oversized native inputs before allocating an encoding and MUST verify the
final encoded length. A canonical-key ABI MUST reject an oversized input
buffer before decoding it.

Bindings MAY use a lower documented local resource limit. The signed `i64`
range is part of the cross-language v1 contract, not a local limit.

Every client path MUST enforce the wire protocol's `0..=32` Item ID limit.
Empty logical keys and empty Exact Item IDs are valid. `PublicKeyOrHash`
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

`NamespaceHash` vectors use the namespace-hash domain string.
`PublicKeyOrHash` fallback vectors use the public domain string and ignore the
namespace and Item ID root parameters above.

These vectors are normative fixtures, not hand-maintained implementation
examples. A generator MUST:

1. construct the typed key and canonical key bytes using the rules in §§2–3;
2. derive the root key with BLAKE3 `DERIVE_KEY` using the exact context string;
3. compute the domain-separated input byte-for-byte; and
4. compare the resulting Item ID with this document.

The machine-readable subset in
[`fixtures/key_format_v1.json`](fixtures/key_format_v1.json) MUST agree with
these tables. Before freeze, the complete set SHOULD be generated and checked
by at least two independent implementations.

### 7.1 NamespaceHash Item IDs

| Vector | Canonical key bytes | `item_id` |
|---|---|---|
| `Text("abc")` | `63 61 62 63` | `c0 ba 3f dd 18 76 04 64 59 a2 58 a7 1f 39 68 d5 d4 5a ff c9 57 d1 44 22 e1 d2 f5 34 79 53 6e 53` |
| `Bytes([00,ff])` | `42 00 ff` | `42 da fe 0f 70 fa 71 5c f0 bf a7 86 d4 5b ca 94 c8 39 21 07 aa dd 69 10 83 6d 37 05 ef 51 e3 e1` |
| `Integer(1)` | `01` | `12 59 ba c3 1d 03 e5 cc b2 1b 4e fc a5 eb c2 c2 5d 04 7c bc 85 63 0d 74 5d a8 75 20 a0 c0 22 d6` |
| `Integer(-1)` | `20` | `9c 75 9f cb 96 09 33 9e 5f 1f 74 ab ae 13 3d de d5 a2 3c 1d 55 29 e0 27 8c 00 b8 09 7a 95 14 dc` |
| `Text("")` | `60` | `84 65 0c 6d 68 0d a8 87 5c c9 0c 2f ba e3 62 48 ff bd 34 ac 33 88 b4 01 64 3c 31 18 63 91 96 e2` |
| `Bytes([])` | `40` | `a8 7e 7a a5 78 1c da 45 c4 a9 eb 1b aa 98 4e ba 0c 78 98 b7 ae a4 59 f4 bd 8f bb ec c3 a3 e0 f7` |
| `Text("abc")`, namespace `2` | `63 61 62 63` | `1b a7 0c ae 30 66 fb b7 bc e8 d7 f4 75 c6 ea 84 ca 27 6e e2 d1 cb 4c 3d 2a f9 bf 85 25 6b c9 58` |

### 7.2 PublicKeyOrHash boundary vectors

The first four vectors follow the canonical preservation path. The final
vector is a 31-byte logical byte string whose 33-byte canonical encoding uses
the public hash fallback.

| Typed key | Canonical key bytes | `item_id` |
|---|---|---|
| `Integer(1)` | `01` | `01` |
| `Text("abc")` | `63 61 62 63` | `63 61 62 63` |
| `Bytes([])` | `40` | `40` |
| `Bytes([00,01,...,1d])` | `58 1e 00 01 02 ... 1d` | `58 1e 00 01 02 ... 1d` |
| `Bytes([00,01,...,1e])` | `58 1f 00 01 02 ... 1e` | `93 71 1b 6b 8d 8a 89 30 39 bc 33 6e 29 99 51 05 d8 3c ae 35 cc 3f 4e 08 5d 51 8c 54 12 7f 67 21` |

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
  28 e6 6d 25 3b 2a 93 e5 db d3 68 d4 21 04 6e 12
  10 c7 76 14 6f d5 ec 21 9f 4e 00 88 f9 8c 24 0f
```

### 7.4 Rejection cases

Clients MUST reject:

- a native input that cannot be inferred as exactly one of `Integer`, `Text`,
  or `Bytes`;
- a non-canonical CBOR key, including trailing bytes or an unsupported type;
- canonical key bytes larger than `MAX_CANONICAL_KEY_BYTES`;
- an Item ID longer than 32 bytes.
