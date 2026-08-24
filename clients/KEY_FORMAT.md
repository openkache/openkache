# OpenKache Client Key Contract — Version 1 Gate 0

> **Status:** Frozen Gate 0 (`v1-gate0`, 2026-08-24).

This document defines how a maintained client converts a typed application key
to the opaque Item ID carried by the wire protocol. It is the source of truth
for key identity; value semantics are in
[`value/SPEC.md`](value/SPEC.md), and the five-operation API is in
[`CLIENT.md`](CLIENT.md).

The normative terms **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and
**MAY** have their RFC 2119 meanings when they appear in uppercase.

## 1. Typed keys

The only mapped key type is:

```text
TypedKey =
    Integer(signed i64)
  | Text(valid UTF-8)
  | Bytes(byte sequence)
```

`Integer` accepts exactly `-2^63..=2^63-1`. `Text` is a length-delimited,
well-formed UTF-8 sequence; it may be empty and may contain U+0000. `Bytes`
is an uninterpreted, length-delimited byte sequence; empty and zero-containing
values are valid. Neither text nor bytes is NUL-terminated.

Adapters MUST infer exactly one variant or require an explicit typed-key
constructor. They MUST reject booleans, null, floating-point values, arrays,
maps, arbitrary objects, invalid UTF-8, and integers outside signed `i64`.
Stringification, reflection, and lossy numeric conversion are not key
inference.

Maintained-language examples:

```text
Python:      str -> Text, bytes-like -> Bytes, int (not bool) -> Integer
TypeScript:  string -> Text, Uint8Array -> Bytes, bigint in i64 -> Integer
Rust/C++:    explicit TypedKey::Text/Bytes/Integer variants
```

JavaScript `number` is not an unambiguous typed key and MUST be rejected by
the maintained facade, even when it happens to be integral. JavaScript callers
use `bigint` for `Integer`. Python `bool` MUST be rejected before its
`int`-subclass behavior is considered.

There is no namespace-level key type or schema. Each operation carries its
own explicit type, and a `Text("1")`, `Integer(1)`, and `Bytes([31])` are
different identities.

## 2. Canonical key bytes

```text
canonical_key_bytes = deterministic_cbor(typed_key)
```

The encoding follows deterministic CBOR
[RFC 8949 §4.2](https://www.rfc-editor.org/rfc/rfc8949#section-4.2), limited
to one complete item:

| Typed key | CBOR |
|---|---|
| `Integer` | preferred major type 0 or 1 integer |
| `Bytes` | definite byte string |
| `Text` | definite valid UTF-8 text string |

Tags, arrays, maps, booleans, null, floats, indefinite-length items, CBOR
sequences, trailing bytes, and non-preferred encodings are rejected. Integer
values MUST fit signed `i64`; CBOR bignum tags are not part of Gate 0.

An interface that accepts canonical bytes MUST decode one complete item,
re-encode the typed key, and compare the bytes before accepting them. This
prevents overlong integers and alternate encodings from creating a second
identity.

Normative examples:

```text
Text("abc")        -> 63 61 62 63
Text("")           -> 60
Bytes([00,ff])     -> 42 00 ff
Bytes([])          -> 40
Integer(1)         -> 01
Integer(-1)        -> 20
Integer(i64::MAX)  -> 1b 7f ff ff ff ff ff ff ff
Integer(i64::MIN)  -> 3b 7f ff ff ff ff ff ff ff
```

The complete canonical key is bounded to
`MAX_CANONICAL_KEY_BYTES = 1,048,576` bytes (1 MiB), including type and
length bytes. A binding MAY use a lower local resource limit but MUST preserve
the signed-`i64` and Item-ID limits.

## 3. Gate 0 Item-ID mapping

The maintained development profile uses `NamespaceHash` with:

```text
namespace_id       = 1
item_id_root_key   = 00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f
                     10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f
```

These settings are shared by the maintained examples so the same typed key
addresses the same item across bindings. This is a public development
fixture, not a production secret. The root is an identity setting; it is not a
value-encryption key.

The mapping first derives a keyed BLAKE3 key:

```text
item_id_derivation_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache item ID derivation root v1",
    material = item_id_root_key[32]
  )
```

It then emits the complete 32-byte Item ID:

```text
item_id =
  BLAKE3-KEYED-HASH(
    key   = item_id_derivation_key,
    input = "openkache/item-id/namespace-hash/v1"
          | namespace_id:u64be
          | canonical_key_bytes
  )
```

The domain strings, field order, byte order, and key encoding are all
normative. Changing the root, namespace, domain, or canonical bytes addresses
a different item and is an application migration.

## 4. Unsupported identity paths

The following are not Gate 0 maintained-client features:

- `PublicKeyOrHash` or any other public preserve/hash mapping;
- Exact Item ID APIs that bypass typed-key mapping;
- legacy `Hash`/`ByteKeyOrHash` profiles;
- caller-provided canonical bytes as a public operation argument; and
- per-operation profile, root, namespace, or identity overrides.

The wire protocol still accepts an opaque `0..=32` byte Item ID, and internal
compatibility code may retain other profiles, but the five-operation facade
MUST use only the fixed `NamespaceHash` mapping above. A legacy or unknown
Item ID is not reinterpreted as a Gate 0 key.

## 5. Validation and errors

Clients MUST reject:

- an input that cannot be inferred as exactly `Integer`, `Text`, or `Bytes`;
- invalid UTF-8, booleans, nulls, floats, collections, or arbitrary objects;
- signed integers outside the `i64` range;
- non-canonical CBOR, unsupported CBOR types, or trailing bytes;
- canonical key bytes larger than 1 MiB; and
- an Item ID outside the wire protocol's `0..=32` byte range.

Errors MUST be reported before a request is sent when the invalidity is local.
Bindings may use language-specific exception/result names, but a caller MUST
be able to distinguish invalid key input from a missing stored value.

## 6. Canonical vectors

[`fixtures/key_format_v1.json`](fixtures/key_format_v1.json) contains the
machine-readable vectors for every key variant, boundary, and rejection. Each
vector declares `spec_revision = "v1-gate0"` and uses explicit type fields;
fixtures never infer a type from a JSON value. A fixture consumer MUST treat
`Integer("1")`, `Text("1")`, and `Bytes("31")` as different inputs.

The public fixture is a contract corpus, not a test suite. Cross-language
validation belongs in the private monorepo.
