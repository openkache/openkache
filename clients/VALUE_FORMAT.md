# OpenKache Client Value Encoding Profile — Version 1 Gate 0

> **Status:** Frozen Gate 0 (`v1-gate0`, 2026-08-24).

This document defines the bytes used by the maintained `get` and `set`
operations. The server stores these bytes opaquely; it does not interpret
CBOR model values. Logical values and their cross-language semantics are
specified in [`value/SPEC.md`](value/SPEC.md), while the request/response
frames are specified in [`../protocol/SPEC.md`](../protocol/SPEC.md).

The normative terms **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and
**MAY** have their RFC 2119 meanings when they appear in uppercase.

## 1. Gate 0 profile

Every successful maintained `get` and `set` uses exactly:

```text
value_envelope_version = 1
payload_format         = StructuredValue-CBOR-v1 (ID 1)
compression            = Uncompressed (ID 0)
protection             = Unprotected (ID 0)
selector               = 0x10
```

The selector is protocol metadata, not an application-level byte argument.
Bindings MUST select it internally and MUST NOT let callers choose a raw
selector, compression mode, key ID, or protection profile.

Gate 0 deliberately does not expose opaque/raw values, JSON, caller-owned v0
envelopes, compressed values, or protected values. Selector IDs for those
formats remain reserved for a future contract. A stored value with any
unsupported version, selector, or payload format produces a format error; it
is never guessed or silently reinterpreted.

TLS protects the unprotected envelope in transit. “Unprotected” means only
that the value body has no value-level encryption or authentication; it does
not add a plaintext transport.

## 2. Envelope grammar

The complete envelope is one canonical version field, one selector byte, and
the selected body:

```text
ValueEnvelopeV1 =
    value_envelope_version:vu128(1)
  | selector_byte:u8(0x10)
  | payload_bytes
```

`vu128` is the protocol's canonical unsigned 64-bit variable-length integer
encoding. Version `1` MUST use the shortest encoding (`01`). The enclosing
wire frame supplies the complete envelope length; there is no nested body
length or terminator. A decoder MUST consume exactly one complete envelope and
reject truncation, overlong `vu128`, and trailing bytes.

The selector layout is retained for future profile registration:

```text
bits 0..1 = protection_id
bits 2..3 = compression_id
bits 4..5 = payload_format_id
bits 6..7 = zero

selector = protection_id
         | (compression_id << 2)
         | (payload_format_id << 4)
```

Gate 0 accepts only `0x10`. Bits 6 and 7 MUST be zero. A decoder MUST reject
all other selectors rather than probing another profile.

## 3. StructuredValue-CBOR-v1 payload

`payload_bytes` is exactly one CBOR data item from
[`value/SPEC.md`](value/SPEC.md):

| CBOR item | Model value |
|---|---|
| `undefined` | `Undefined` |
| `null` | `Null` |
| `false`, `true` | `Boolean` |
| major type 0 or 1, or minimal tag 2/3 integer | arbitrary-precision `Integer` |
| half/single/double float | `Float16`/`Float32`/`Float64` with raw bits |
| definite byte string | `ByteString` |
| definite valid UTF-8 text string | `TextString` |
| definite array | `Array<Value>` |
| definite map with scalar unique keys | ordered `Map` |

Indefinite-length items, invalid UTF-8, unknown simple values or tags,
non-scalar map keys, duplicate model keys, and CBOR sequences MUST be
rejected. A decoder MUST reject trailing bytes and MUST NOT return a partial
value. The model's equality rules distinguish integer, boolean, and float
keys; native language equality is not sufficient.

Encoders MUST emit preferred CBOR integer encodings, preserve float width and
raw bits, and encode one complete model item. Map order is retained for
lossless access but is not semantic equality, so a decode/re-encode need not
produce identical bytes when only map order differs.

Examples:

```text
Undefined                         -> 01 10 f7
Null                              -> 01 10 f6
Boolean(true)                     -> 01 10 f5
Integer(1)                        -> 01 10 01
Float16(+1.0)                     -> 01 10 f9 3c 00
Float32(+1.0)                     -> 01 10 fa 3f 80 00 00
Float64(-0.0)                     -> 01 10 fb 80 00 00 00 00 00 00 00
TextString("x")                   -> 01 10 61 78
ByteString([78])                  -> 01 10 41 78
Map([(Integer(1), Null)])         -> 01 10 a1 01 f6
```

## 4. Encoding and decoding

An encoder MUST:

1. convert the caller's value to the lossless model;
2. reject unsupported native values, cycles, invalid text, and non-scalar or
   duplicate map keys;
3. encode one `StructuredValue-CBOR-v1` item;
4. prefix `01 10`; and
5. enforce the envelope limit before returning bytes.

A decoder MUST:

1. parse canonical version `1` and selector `0x10`;
2. decode exactly one complete `StructuredValue-CBOR-v1` item;
3. enforce the payload and envelope limits; and
4. return the lossless model or a stable format/resource error.

The decoder MUST NOT fall back to JSON, raw bytes, another selector, or a
different value profile. Authentication/decompression steps do not exist in
the Gate 0 profile; adding them requires a new contract revision and selector.

## 5. Limits and errors

The wire protocol bounds one value envelope to 67,108,864 bytes (64 MiB).
Implementations MAY use a lower documented local limit, but MUST reject an
envelope or decoded payload that exceeds the configured limit before
unbounded allocation. A resource error MUST identify the limit category and
MUST NOT return a partial model.

Stable format errors include:

- `UnsupportedVersion` — version is not canonical `1`;
- `UnsupportedSelector` — selector is not `0x10` or has reserved bits set;
- `MalformedCbor` — truncated, overlong, indefinite, unknown, or trailing CBOR;
- `InvalidUtf8` — a text string is not valid UTF-8;
- `DuplicateMapKey` — two scalar keys compare equal under model equality;
- `NonScalarMapKey` — an array or map is used as a key; and
- `ResourceLimit` — envelope, payload, depth, or item budget is exceeded.

Bindings map these categories into idiomatic errors without replacing a format
error with `Missing`, `Null`, or an ordinary transport failure.

## 6. Versioning

The version field selects a complete envelope grammar. A future profile MUST
use a new selector or envelope version and MUST NOT reinterpret payload-format
ID `1` or selector `0x10`. Gate 0 has no compatibility reader for legacy JSON,
raw, protected, compressed, or caller-owned-v0 envelopes.

The canonical machine-readable vectors in
[`fixtures/value_format_v1.json`](fixtures/value_format_v1.json) and
[`fixtures/structured_value_cbor_v1.json`](fixtures/structured_value_cbor_v1.json)
are part of this frozen profile. Both declare `spec_revision = "v1-gate0"`.
