# OpenKache Client Key Contract — Version 1 Draft

> **Status:** Draft `draft-2026-08-19`; not released or finalized.
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
key bytes into the final Item ID. The initial profiles are `Hash` and
`UnisolatedKeyOrHash`. A mapping profile is client-local configuration,
optionally scoped per namespace by the client, not a wire-visible field or a
server-enforced namespace policy. A client that addresses more than one
namespace MUST keep its selected profile stable for each namespace it
addresses, but the server does not store or interpret that choice.

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
  -> Hash
  -> ItemId

application key (typed input)
  -> typed key (TypedKey)
  -> canonical key encoding
  -> canonical key bytes (canonical_key_bytes)
  -> UnisolatedKeyOrHash
  -> Item ID (canonical bytes when 0..=32 bytes; public hash otherwise)

exact Item ID
  -> Exact Item ID API
  -> wire
```

Both formatted profiles use the same typed key and canonical encoding.
`Hash` always produces a root- and namespace-bound 32-byte Item ID.
`UnisolatedKeyOrHash` exposes short canonical key bytes directly and uses an
unkeyed hash only when the encoding does not fit. It is intended for
deployments that trust the server and do not require client-side key
confidentiality or profile isolation. The selected mapping profile is part of
item identity and MUST remain stable for addressable data.

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

The selected Item ID mapping profile and canonicalization algorithm MUST remain
fixed for all formatted operations addressing a namespace unless the
application is intentionally performing an identity migration. The inferred
`KeyType` MAY vary between operations because it is determined by the native
input. The server does not know which profile or native key type produced an
Item ID; a mismatch can therefore appear only as misses or collisions. Whether
maintained clients later persist a local profile record or use server-side
metadata remains a client-policy TODO.

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
| Exact integer value | `Integer` |
| Valid UTF-8 text/string | `Text` |
| Explicit byte sequence (`Bytes`, `byte[]`, `Buffer`, or equivalent) | `Bytes` |

The inference is language-specific only at the boundary; the resulting
`TypedKey` and canonical bytes are language-independent. An adapter MUST
reject a value when it cannot determine one of these categories without
coercion. Examples include floating-point values, booleans, null, arrays,
objects, maps, and custom objects.

JavaScript `number` follows §3.2: a finite safe integer infers `Integer`, and
all other numbers are rejected. JavaScript `bigint` infers `Integer`.
Python `bool` MUST be rejected before Python's integer-subclass behavior is
considered; Python `int` infers `Integer`, `str` infers `Text`, and an
explicit bytes-like value infers `Bytes`.

An adapter MAY expose an explicit typed constructor for FFI or generic
containers, but that constructor must produce the same three `TypedKey`
variants and must not introduce a fourth key type. Both mapping profiles accept
all three variants.

The normative `Integer` value is not limited to `i64`: Python integers and
JavaScript `bigint` values outside native machine ranges remain valid
integers, subject to `MAX_CANONICAL_KEY_BYTES`. A binding MAY provide a narrower
native convenience API, but that convenience limit MUST NOT change the
cross-language `Integer` contract. The maintained API still has an explicit
pre-freeze TODO: choose between no additional native limit, an `i64` limit, or
a different bounded representation for ergonomic typed-language overloads.

The maintained-client default mapping profile is `Hash`.
`UnisolatedKeyOrHash` MUST be selected explicitly. It is suitable for
benchmarks and for deployments that fully trust the server and accept public,
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
other numeric spellings MUST be rejected. The decimal text is only a transport
representation; key identity remains the mathematical integer encoded under
§3.1.

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

- The accepted subset contains only major types 0/1 integers, tag 2/3
  canonical bignums, major type 2 byte strings, and major type 3 text strings.
- Indefinite-length items, tags other than 2 and 3, sequences, and trailing
  bytes are invalid.
- Integer encoding uses RFC 8949 preferred serialization, including standard
  bignum tags `2` and `3` when the basic integer types cannot represent the
  value. A non-negative integer greater than `2^64 - 1` uses tag `2` with its
  minimal unsigned big-endian magnitude. A negative integer less than `-2^64`
  uses tag `3` with the minimal unsigned magnitude of `-1 - value`, as required
  by RFC 8949. The magnitude is non-empty for a bignum and MUST NOT contain
  leading zero bytes. Values representable by the basic CBOR major types MUST
  use those preferred major-type encodings rather than a bignum tag.
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

Item ID mapping profiles are client-local choices. They have no wire or server
field.
Every profile MUST produce an Item ID accepted by the wire protocol.

### 4.1 Hash profile (`Hash`)

Under `Hash`, the adapter infers one supported `TypedKey`, encodes it, and
derives a 32-byte Item ID. Equal typed keys in different namespaces produce
different Item IDs.

```text
profile_header = f8 01
tag =
  BLAKE3-KEYED-HASH(
    key   = item_id_derivation_key[32],
    input = profile_header[2] | namespace_id:u64be | canonical_key_bytes
  )[0..30]
item_id = profile_header[2] | tag[30]
```

The header identifies this mapping revision and is authenticated by the keyed
hash input. `f8` cannot begin a canonical key accepted by §3, so a `Hash` Item
ID cannot collide with the preserved path below. Truncating BLAKE3 to 30 bytes
retains 240-bit collision resistance. The server treats all 32 bytes as
opaque.

### 4.2 Unisolated preserve-or-hash profile (`UnisolatedKeyOrHash`)

`UnisolatedKeyOrHash` accepts any supported typed application key and encodes it
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
uses the hash fallback. Basic CBOR integers occupy at most nine bytes and are
preserved. Larger integers may use a hash fallback when their tagged canonical
encoding exceeds 32 bytes.

The fallback uses a distinct header and a 30-byte unkeyed BLAKE3 tag:

```text
profile_header = f8 02
tag = BLAKE3-HASH(profile_header[2] | canonical_key_bytes)[0..30]
hash_fallback_item_id = profile_header[2] | tag[30]
```

The header is included in the hash input and cannot begin an accepted canonical
key. It is not added to preserved Item IDs. Neither path uses `namespace_id`,
`item_id_root_key`, or `item_id_derivation_key`. Consequently, equal typed
keys produce equal Item IDs across namespaces and client configurations under
this profile; the wire-level `(namespace_id, item_id)` pair still identifies a
distinct server item.

This profile provides no client-side key confidentiality, namespace binding,
or root-key isolation. It is useful when the server is fully trusted, when
cross-client public identity is desired, or when avoiding hashing for short
keys matters. Applications requiring those omitted guarantees MUST use
`Hash`.

The headers prevent accidental collisions between the two formatted hashed
paths. The Exact Item ID API can still intentionally supply any byte sequence,
including either header. Future mapping revisions MUST use a new header and
MUST NOT reinterpret existing Item IDs.

### 4.3 Exact Item ID API

The Exact Item ID API accepts the final opaque `0..=32`-byte Item ID directly.
It performs no typed-key conversion, canonical encoding, hashing, or namespace
derivation. It still enforces the protocol Item ID length limit.

This API is appropriate when an application already owns a wire identity. It
is a dangerous escape hatch: the caller, not the client profile, owns identity,
collision avoidance, and isolation.

## 5. Derivation parameters

### 5.1 Namespace binding

`namespace_id` is a positive, server-assigned identity. Clients MUST NOT
synthesize or recycle it. `Hash` binds it after the profile header:

```text
Hash profile:
  f8 01 | namespace_id:u64be | canonical_key_bytes
```

Including the namespace makes equal typed keys produce different `Hash` Item
IDs in different namespaces. Consequently, a client MUST resolve the namespace
ID before deriving a `Hash` Item ID.

`UnisolatedKeyOrHash` deliberately omits the namespace from both paths. The
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

`UnisolatedKeyOrHash` ignores `item_id_root_key` and the resulting
`item_id_derivation_key`. Changing either has no effect on its Item IDs.

Maintained clients SHOULD persist a non-secret, client-local profile record
keyed by namespace identity. It SHOULD contain the mapping-profile name and
revision, canonicalization revision, namespace ID, and a fingerprint of the
root-key configuration. Native key-type inference is per input and is never a
namespace configuration. A future client MAY use opaque server metadata for
profile discovery, but the v1 wire protocol does not require the server to
interpret or validate it. Until that design is finalized, a client MUST NOT
silently change these settings for an existing namespace.

### 5.3 Hash algorithm and profile compatibility

`Hash` uses BLAKE3 keyed hashing with a 32-byte derived key.
`UnisolatedKeyOrHash` uses standard unkeyed BLAKE3 only for canonical
encodings longer than 32 bytes. The assigned hashed-path headers are:

| Header | Profile path |
|---:|---|
| `f8 01` | `Hash` |
| `f8 02` | `UnisolatedKeyOrHash` hash fallback |

The header identifies the mapping path and revision, not the root-key epoch.
A profile that changes identity, namespace input, hash context, derivation
material, or input framing MUST use a new header and MUST NOT reinterpret
existing Item IDs. There is no in-band root-key mismatch detector or rotation
protocol: the wrong root normally appears as a miss. Changing
`item_id_root_key` is an identity migration, not a value-key rotation.

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

Bindings MAY use a lower documented local resource limit. The maintained
typed-language API still has a pre-freeze TODO to choose whether native
integers are unbounded, limited to `i64`, or subject to another ergonomic
bound; any such limit is separate from canonical encoding.

Every client path MUST enforce the wire protocol's `0..=32` Item ID limit.
Empty logical keys and empty Exact Item IDs are valid. `UnisolatedKeyOrHash`
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

`Hash` vectors use header `f8 01`. `UnisolatedKeyOrHash` fallback vectors use
header `f8 02`; they ignore the namespace and Item ID root parameters above.

These vectors are normative fixtures, not hand-maintained implementation
examples. A generator MUST:

1. construct the typed key and canonical key bytes using the rules in §§2–3;
2. derive the root key with BLAKE3 `DERIVE_KEY` using the exact context string;
3. compute the profile-header input byte-for-byte; and
4. compare the resulting Item ID with this document.

The machine-readable subset in
[`fixtures/key_format_v1.json`](fixtures/key_format_v1.json) MUST agree with
these tables. Before freeze, the complete set SHOULD be generated and checked
by at least two independent implementations.

### 7.1 Hash Item IDs

| Vector | Canonical key bytes | `item_id` |
|---|---|---|
| `Text("abc")` | `63 61 62 63` | `f8 01 b7 7c be 5f 62 bf b4 0c 38 a0 b7 d2 f3 5e 85 b0 e2 b2 27 ef 70 14 a1 f2 b9 a4 47 68 71 71` |
| `Bytes([00,ff])` | `42 00 ff` | `f8 01 76 47 72 4a 35 af aa 05 28 d2 16 5d 39 0d 99 9a 70 45 82 9f af 52 bb 17 77 dd 3a 99 2c 0c` |
| `Integer(1)` | `01` | `f8 01 10 2d 2c bf 0e 5d 25 4b 18 05 0a 98 3c 6f 8e 58 a5 97 57 3e f8 9e 61 e4 f2 d6 56 0c a3 cc` |
| `Integer(-1)` | `20` | `f8 01 bc e2 98 b0 5e c1 45 19 7e 1b cd d9 2d 38 be 1c 29 36 e9 3c 69 6d 3e 58 f0 32 f5 11 0d 68` |
| `Text("")` | `60` | `f8 01 3c a2 db dd 41 ec 46 b6 56 f7 8f 11 6c db 1e 9b c1 72 96 e0 a2 d0 a0 c2 77 9d 7c 32 fa a8` |
| `Bytes([])` | `40` | `f8 01 ec e0 6c e6 b6 77 36 b6 5a 39 9a a3 15 ec 59 60 05 e9 e0 3a c2 62 33 a3 71 9d 28 4c 8f ce` |
| `Text("abc")`, namespace `2` | `63 61 62 63` | `f8 01 86 f9 ef 86 57 f1 a8 1e 59 c6 99 5f 00 79 e4 54 f6 7c 89 89 f4 da 70 59 b6 8e 4e b5 cb 41` |

### 7.2 UnisolatedKeyOrHash boundary vectors

The first four vectors follow the canonical preservation path. The final
vector is a 31-byte logical byte string whose 33-byte canonical encoding uses
the public hash fallback.

| Typed key | Canonical key bytes | `item_id` |
|---|---|---|
| `Integer(1)` | `01` | `01` |
| `Text("abc")` | `63 61 62 63` | `63 61 62 63` |
| `Bytes([])` | `40` | `40` |
| `Bytes([00,01,...,1d])` | `58 1e 00 01 02 ... 1d` | `58 1e 00 01 02 ... 1d` |
| `Bytes([00,01,...,1e])` | `58 1f 00 01 02 ... 1e` | `f8 02 8e af 52 3b b3 19 cf a4 62 12 ea 4b cc 89 79 12 45 67 3e f3 0f 93 eb b7 49 cc 48 e0 da 99` |

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
  f8 01 68 85 41 d0 df 74 d4 61 57 8b c2 4f 51 88
  fb ca 01 00 02 29 c3 07 cb 8d 3a ee 91 d1 7c f1
```

### 7.4 Rejection cases

Clients MUST reject:

- a native input that cannot be inferred as exactly one of `Integer`, `Text`,
  or `Bytes`;
- a non-canonical CBOR key, including trailing bytes or an unsupported type;
- canonical key bytes larger than `MAX_CANONICAL_KEY_BYTES`;
- an Item ID longer than 32 bytes.
