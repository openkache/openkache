# OpenKache Cross-Language Value Model — Version 1 Gate 0

> **Status:** Frozen Gate 0 (`v1-gate0`, 2026-08-24).

This document defines the language-independent value model used by every
maintained client's `get` and `set`. It is intentionally separate from the
cache value envelope: the envelope carries payload bytes, while this
specification defines what those bytes mean to clients.

Gate 0 exposes lossless model values by default. Package documentation may
choose different wrapper names, but it must preserve the same tags and
distinctions. The concrete profile identifier and selector assignment are
normative in [`../VALUE_FORMAT.md`](../VALUE_FORMAT.md).

`StructuredValue-CBOR-v1` is the only Gate 0 payload profile. The model owns
logical semantics; the profile owns its CBOR bytes. A future profile may use
another codec only after a new selector and contract revision are published.
An implementation MUST NOT claim Gate 0 conformance until it satisfies the
model, profile, validation, and native-conversion rules in this document.

The normative terms **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**,
**SHOULD NOT**, and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) and
[RFC 8174](https://www.rfc-editor.org/rfc/rfc8174).

## 1. Scope and design goals

The value project provides a reusable cross-language serialization layer for
client applications. It is not part of the OpenKache server and it does not
define Item IDs, namespaces, transport frames, compression, encryption, or
cache expiry.

The design priorities are:

1. preserve the source language's observable value semantics on a same-language
   put/get round trip;
2. provide predictable conversions between maintained language bindings;
3. avoid silent numeric, text, byte, or map-key loss;
4. keep common values compact and fast;
5. keep the shared implementation and conformance surface small; and
6. allow the codec profile to evolve independently of the cache envelope.

The model intentionally does not preserve language-specific object identity,
class names, tuple-versus-list identity, set implementations, cycles,
functions, or custom collection classes. Applications that require those
properties MUST use an explicit application codec or a future opaque-value
profile; Gate 0 does not expose an opaque-byte operation.

## 2. Value model

The public model is codec-independent:

```text
Value =
    Undefined
  | Null
  | Boolean(value)
  | Integer(arbitrary_precision_signed_value)
  | Float16(raw_bits)
  | Float32(raw_bits)
  | Float64(raw_bits)
  | ByteString(bytes)
  | TextString(valid_utf8)
  | Array<Value>
  | Map<(Value, Value)>[]
```

`Undefined` is distinct from `Null`. A binding that has no native undefined
value MUST expose it through its lossless value representation or report a
conversion error in strict native mode.

`Float16`, `Float32`, and `Float64` are separate model kinds, not one native
language float with an inferred width. Each stores the raw IEEE-754 bits for
that width. `Integer` is arbitrary precision even though the key contract
uses signed `i64`.

`Map` is an ordered sequence of key/value entries in the generic model:

```text
Map([(key1, value1), (key2, value2), ...])
```

The entry sequence preserves source order for lossless forwarding and native
client convenience. Map order has no semantic meaning for equality or
duplicate-key validation. An application that gives order semantic meaning
MUST use an `Array` of pairs instead of a `Map`.

### 2.1 Integers

`Integer` is an arbitrary-precision mathematical signed integer. It does not
preserve the source language's fixed-width integer type, signedness, or
platform-dependent width. A typed API MAY convert an `Integer` to a requested
native integer after checking its range and sign.

The model has one integer semantic type even when the codec uses more than one
physical representation. In the first profile, ordinary codec integers and
large-integer representations both decode to `Integer`.

Integer conversions MUST be exact. A binding MUST reject overflow, unsigned
conversion of a negative value, and any conversion that would round or
otherwise change the integer.

### 2.2 Floating-point values

The three floating-point kinds contain their IEEE-754 width and raw bits:

```text
Float16(raw_bits)
Float32(raw_bits)
Float64(raw_bits)
```

The model distinguishes `Integer(1)` from `Float64(1.0)`, and distinguishes
positive and negative zero. Float width and raw bits MUST be preserved by the
generic value representation and by an encoder that receives those bits.

An encoder receiving one of these model variants MUST preserve its width and
raw bits exactly.
Native runtime mappings have weaker guarantees when the runtime cannot expose
all of those bits. Python `float` mappings preserve the runtime-observable
binary64 bits. A JavaScript adapter MUST encode the binary64 bits observable
from its `number` value and MUST NOT intentionally normalize a representation
that the runtime exposes. Applications that require a particular NaN payload or
original float width MUST use the generic model returned by `lossless`.

Native decoding MAY map Float16 and Float32 to the language's ordinary
binary64 floating-point type. Such a conversion preserves the numeric value
but not the original width in the native object; the generic representation
remains available when width or raw bits matter.

### 2.3 Strings and bytes

`TextString` is well-formed UTF-8. `ByteString` is an uninterpreted byte
sequence. They are distinct values even when their byte contents happen to be
the same.

Bindings MUST NOT decode arbitrary bytes as text, encode text as bytes, or
stringify a value as a fallback conversion.

### 2.4 Equality and map keys

The model defines structural equality for duplicate-key detection:

- integers compare by mathematical value; physical integer representation is
  ignored;
- floats compare by width and raw bits; `+0.0` and `-0.0` are distinct, and
  distinct NaN payloads are distinct;
- byte strings and text strings compare by type and exact contents;
- arrays compare by length and ordered recursive equality;
- maps compare without regard to entry order after their own duplicate checks;
- booleans, `Null`, and `Undefined` compare only to the same value kind; and
- a map key MUST NOT be compared by a language-native equality operation that
  collapses distinct model values.

Only scalar values MAY be map keys:

```text
MapKey = Null | Undefined | Boolean | Integer
       | Float16 | Float32 | Float64
       | ByteString | TextString
```

`Array` and `Map` MUST NOT be used as map keys in this profile. Applications
that need a compound key MUST use an `Array` of key/value pairs or an explicit
application codec. A decoder MUST reject a map containing a non-scalar key or
duplicate scalar keys under these rules. If a binding cannot represent a map
without losing scalar key identity, it MUST return a lossless map view or a
conversion error; it MUST NOT silently overwrite, merge, stringify, or drop an
entry.

## 3. Maintained language mappings

The following mappings are the defaults for OpenKache-maintained bindings.
Third-party clients MAY expose a different local API while implementing the
same value profile and conversion rules.

### 3.1 Python

The adapter MUST test `bool` before `int`, because `bool` is an `int` subclass
in Python.

| Python value | Model value |
|---|---|
| `None` | `Null` |
| `bool` | `Boolean` |
| `int` | `Integer` |
| `float` | `Float64` |
| `bytes`, `bytearray`, or documented byte buffer | `ByteString` |
| `str` | `TextString` |
| `list` or `tuple` | `Array` |
| `dict` | `Map` |

Python `int` is encoded as an exact `Integer` regardless of magnitude. Python
`float` is encoded as `Float64`, including `-0.0`, infinities, and NaN.
Python `list` and `tuple` both map to `Array`; a same-language round trip
preserves element order and values but does not preserve list-versus-tuple
identity. Python `dict` maps to `Map` in insertion order. Its keys MUST be
scalar model values after conversion. Python's own equality and hashing rules
apply while constructing a native `dict`; callers that need both keys from a
native collision such as `True` and `1` MUST use the generic ordered-entry
constructor instead.

The generic Python escape hatch is an ordered sequence of model pairs, for
example:

```python
MapValue([
    (Boolean(True), TextString("bool")),
    (Integer(1), TextString("integer")),
])
```

### 3.2 JavaScript and TypeScript

JavaScript `number` is always IEEE-754 binary64. The adapter MUST NOT inspect
whether a number happens to be integral and change its model type.

| JavaScript value | Model value |
|---|---|
| `null` | `Null` |
| `undefined` | `Undefined` |
| `boolean` | `Boolean` |
| `number` | `Float64` |
| `bigint` | `Integer` |
| `string` | `TextString` |
| `Uint8Array` or documented byte buffer | `ByteString` |
| `Array` | `Array` |
| `Map` | `Map` |
| documented plain-object mapping | `Map` with text keys |

Consequently:

```text
1       -> Float64(+1.0)
1n      -> Integer(1)
1.5     -> Float64(1.5)
-0      -> Float64(-0.0)
```

An adapter MUST NOT silently convert an unsafe `number` into an integer
meaning. JavaScript callers that explicitly need a compact `Integer` MAY use a
documented integer helper or `bigint`; that opt-in is a value-construction
choice, not automatic inference.

JavaScript `Map` maps to `Map` in iteration order. Its keys MUST convert to
scalar model values; array, object, function, and other non-scalar keys MUST
be rejected rather than stringified. A documented plain-object mapping maps
own enumerable string properties to `TextString` keys in the language's
observable property iteration order. A same-language round trip preserves
entries and order, but a plain object may be returned as a generic/lossless
map when its key/value shape cannot be represented by a plain object. A plain
object projection MUST define properties without invoking inherited setters;
adapters SHOULD use a null-prototype object or equivalent safe property
definition for names such as `__proto__`.

On decode, maintained JavaScript clients return:

```text
Integer -> bigint
Float   -> number
```

An explicit convenience option MAY request safe integers as `number`, but it
MUST reject values outside the exact safe-integer range instead of rounding
them. The default MUST preserve the distinction between JavaScript `number`
and `bigint`.

### 3.3 Other maintained languages

Native arbitrary-precision integer types map to `Integer`. Fixed-width
integer types also map to `Integer`; the width is a local type constraint and
is restored only by a checked typed decode. Native `f32`/`float` and
`f64`/`double` types map to Float32 and Float64 respectively. Platform-width
types such as `isize`, `usize`, `int`, and `uint` MUST NOT become wire-level
types.

Native sequence types map to `Array` and preserve element order. Native map
types map to `Map` and preserve entry order when their runtime exposes one.
Because this profile permits only scalar map keys, a maintained binding MUST
either use a native map with an explicitly documented scalar-key projection or
use its generic ordered-entry representation. It MUST reject a non-scalar
model key or a native-key collision rather than stringify or overwrite it.
Rust, Go, Java, .NET, and C/C++ bindings MUST document the concrete native
collection and generic-entry types they return; they MUST not claim to
preserve source-only container classes that the model does not represent.

Each maintained package MUST document the native type returned for `Integer`,
the behavior of Float16/Float32 decoding, and any value that cannot be
returned in its ordinary native representation. A same-language round trip
guarantees model values and observable entry order, not source-only container
identity such as Python tuple versus list or a JavaScript plain object versus
`Map`.

## 4. Representation options

The client API SHOULD expose one `get` operation with a representation option
rather than forcing callers to learn separate method names. Language syntax
may differ, but the semantics are shared:

```text
get(key, representation="lossless")
get(key, representation="native")
```

`lossless` SHOULD be the default for dynamic maintained bindings.

Encoding SHOULD accept every value expressible by the binding's documented
native mapping. Decoding into a different language's native representation
MAY fail when that language cannot represent the value without semantic loss.
The adapter MUST then return a conversion error for `native`; `lossless` is
the cross-language fallback that returns the complete generic model without
semantic loss.

### 4.1 Lossless representation

`lossless` returns the complete generic model. It MUST preserve value kinds,
numeric distinctions, scalar map-key identity, and map entry order. A binding
MAY provide native-like wrappers or convenience accessors over that model, but
those are not a separate representation contract. If a wrapper is provided, it
MUST support ordinary value access:

- indexing where the model is indexed;
- lookup and membership;
- length;
- iteration; and
- map keys, values, and entries.

The wrapper is not required to pass every native reflection or serialization
test. For example, it need not be an actual Python `dict` or JavaScript plain
object. A lookup that is ambiguous under native equality MUST report an
ambiguity error rather than choose an entry.

The following read operations are common wrapper requirements; language syntax
may differ:

| Operation | Required behavior |
|---|---|
| Array indexing | Return the indexed child, preserving a wrapper when needed. |
| Map lookup | Match a complete model key when the caller supplies one; a native lookup that maps to zero or multiple model keys MUST return missing or an ambiguity error respectively. |
| Map iteration | Iterate model-preserving keys and values in entry order. |
| `keys`, `values`, and `entries` | Expose the same model-preserving view as map iteration. |
| Length | Return the number of array elements or map entries. |

Mutation, hashability, reflection, and native serialization of wrappers are
language-specific and MUST be documented by each adapter. A wrapper MUST NOT
silently mutate a map in a way that drops or merges a model key. A binding
that does not provide a wrapper still returns the generic model in `lossless`;
it MUST NOT silently discard information.

### 4.2 Native representation

`native` requires the complete value to be representable by the binding's
documented native types. It MUST return a conversion error for an unsupported
value, an unrepresentable map key, or a map that would collapse entries.

This mode is intended for APIs that require a real `dict`, plain object, or
other native container. It is not allowed to silently lose information.

The maintained default native projections are:

| Model value | Python | JavaScript/TypeScript |
|---|---|---|
| `Null` | `None` | `null` |
| `Undefined` | conversion error | `undefined` |
| `Boolean` | `bool` | `boolean` |
| `Integer` | `int` | `bigint` |
| `Float16`/`Float32`/`Float64` | documented float wrapper/`float` | documented float wrapper/`number` |
| `ByteString` | `bytes` | `Uint8Array` |
| `TextString` | `str` | `string` |
| `Array` | `list` | `Array` |
| `Map` | `dict` when representable | `Map` when representable |

`native` MUST return a conversion error instead of rounding an integer,
normalizing a float distinction, collapsing scalar map keys, or silently
changing a map's order. A binding MAY provide a separate checked convenience
conversion, such as JavaScript safe `Integer` values to `number`, only when it
rejects values outside the exact target range.

Gate 0 has no opaque-byte representation or raw/Exact Item ID operation.
Byte-exact forwarding is a future feature, not a representation option on
structured-value `get`; callers MUST NOT decode and re-encode a structured
value when exact envelope bytes are required.

The structured-value API guarantees semantic round trips, not byte identity.
Reading with `lossless` and writing again MAY produce different codec bytes
because map ordering, preferred integer encodings, or codec implementation
details may differ. Exact stored-envelope forwarding, replication, and backup
are outside this representation contract and require a future low-level
stored-bytes API if they become necessary.

## 5. Initial codec profile: StructuredValue-CBOR-v1

The first profile maps the model to one complete CBOR data item. It delegates
the byte grammar to [RFC 8949](https://www.rfc-editor.org/rfc/rfc8949) and the
floating-point representation to IEEE-754.

The profile accepts exactly the following CBOR values:

| CBOR encoding | Logical value | Acceptance rule |
|---|---|---|
| `null` | `Null` | MUST be accepted. |
| `true`, `false` | `Boolean` | MUST be accepted. |
| `undefined` | `Undefined` | MUST be accepted. |
| Major type 0 or 1 | `Integer` | MUST represent the mathematical value exactly. Preferred and valid non-preferred integer encodings MAY both be accepted; encoders MUST emit preferred serialization. |
| Tag 2 or 3 over a definite byte string | `Integer` | MUST use a minimal, non-empty big-endian magnitude. The sign is selected by the tag. Values representable as ordinary CBOR integers MUST still be decoded as the same `Integer`; encoders MUST prefer ordinary integers. |
| Half-, single-, or double-precision float | `Float16`/`Float32`/`Float64` | Width and raw bits MUST be retained by the generic representation. |
| Definite byte string | `ByteString` | Bytes are uninterpreted. |
| Definite, valid UTF-8 text string | `TextString` | Invalid UTF-8 MUST be rejected. |
| Definite array | `Array` | Elements are decoded recursively. |
| Definite map | `Map` | Keys must be scalar and duplicates are rejected under §2.4. |

Indefinite-length arrays, maps, byte strings, and text strings MUST be
rejected. Simple values other than `null`, booleans, and `undefined`, and tags
other than 2 and 3, MUST be rejected. Exactly one complete CBOR data item MUST
be present; trailing bytes and CBOR sequences MUST be rejected.

For duplicate-key detection, a decoder MUST decode each key to the logical
model, compare keys using §2.4 structural equality, and reject a duplicate
before exposing the map. It MUST NOT use native language equality as a
substitute for this comparison. Map entry order does not affect the result.

The profile does not require deterministic encoding of the complete payload.
Maintained Python and JavaScript encoders MUST preserve the observable
iteration order of their input `dict`, `Map`, or lossless entry view, and
decoders MUST preserve that order in their lossless result. This is a
same-language round-trip guarantee and does not make map order part of wire
equality or require third-party implementations to preserve it.

The profile does not impose additional wire-level depth, entry-count, or
bignum-size limits. A conforming decoder MUST nevertheless use stack-safe
parsing or an equivalent bounded traversal strategy. An implementation MAY
apply lower local structural budgets and MUST report a local resource
rejection without returning a partially decoded value.

The profile's parser MUST apply the common payload limits supplied by the
enclosing client value format. Gate 0 does not provide a raw/opaque or
caller-owned-v0 structured operation; those are unsupported client features,
even though the model can be used by a future profile.

## 6. Standards delegation and future profiles

The value project delegates well-defined low-level behavior instead of
re-specifying it:

| Concern | Delegated reference |
|---|---|
| Initial self-describing codec | RFC 8949 CBOR |
| Floating-point representation | IEEE-754 |
| Structured payload compression | OpenKache Client Value Format |
| Schema-bound protobuf or similar payloads | Their respective standards |

Delegation does not remove the need to define cross-language behavior. This
specification owns native numeric mappings, semantic equality, duplicate-key
rejection, lossless/native representations, and optional view behavior because the
underlying codec standards do not define those product-level contracts.

Schema-bound formats such as protobuf are future payload profiles, not
transparent replacements for the generic `Value` model. A protobuf profile
requires a schema and preserves protobuf's own field, integer-width, and
unknown-field semantics.

## 7. Implementation boundary

Maintained bindings may share a value implementation, but the public Gate 0
facade owns the lossless model and `StructuredValue-CBOR-v1` profile described
here. The client value envelope prefixes the payload with version `1` and
selector `0x10` and performs no value-level compression or protection in Gate
0. The server continues to treat the complete envelope as opaque bytes.

This specification is deliberately independent of Item ID derivation and
formatted-key behavior, which remain defined by
[`../KEY_FORMAT.md`](../KEY_FORMAT.md).

## 8. Conformance examples

The following mappings are normative:

| Source value | Logical model |
|---|---|
| Python `1` | `Integer(1)` |
| JavaScript `1` | `Float64(+1.0)` |
| JavaScript `1n` | `Integer(1)` |
| Python or JavaScript `-0.0`/`-0` | `Float64(-0.0)` |
| `TextString("x")` | distinct from `ByteString(78)` |
| `Integer(1)` map key | distinct from `Boolean(true)` and `Float64(1.0)` |
| `Array([Integer(1)])` as a map key | Reject as a non-scalar map key |

A map containing keys that are distinct in the model but collide in a native
container MUST be returned through a lossless representation or rejected by
strict native conversion.

The following initial codec vectors are normative:

| Logical value or case | `StructuredValue-CBOR-v1` bytes | Result |
|---|---|---|
| `Integer(1)` | `01` | Decode as `Integer(1)`. |
| `Integer(2^64)` | `c2 49 01 00 00 00 00 00 00 00 00` | Decode as `Integer(18446744073709551616)`. |
| `Float16(+1.0)` | `f9 3c 00` | Preserve width 16 and raw bits `0x3c00`. |
| `Float32(+1.0)` | `fa 3f 80 00 00` | Preserve width 32 and raw bits `0x3f800000`. |
| `Float64(-0.0)` | `fb 80 00 00 00 00 00 00 00` | Preserve the negative-zero bit pattern. |
| `TextString("x")` | `61 78` | Distinct from `ByteString([78])`. |
| `ByteString([78])` | `41 78` | Distinct from `TextString("x")`. |
| `Map([(Integer(1), Null)])` | `a1 01 f6` | Decode as one map entry. |
| Duplicate logical key `Integer(1)` | `a2 01 f6 01 f7` | Reject before exposing the map. |
| Non-preferred integer encoding `Integer(1)` | `18 01` | MAY be accepted by a decoder; encoders MUST emit `01`. |
