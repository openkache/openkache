# OpenKache Cross-Language Value Model v1 (Draft)

> **Status:** Draft — pre-freeze

This document defines the language-independent value model used by
OpenKache-maintained clients when they exchange structured values. It is
intentionally separate from the cache value envelope: the envelope carries
encoded bytes, while this specification defines what those bytes mean to
clients.

The first structured-value profile uses CBOR as its internal codec. CBOR is an
implementation profile, not the public value API. A future profile may use
another self-describing codec, or a schema-bound codec such as protobuf, while
retaining a separate compatibility contract.

This document is a target specification. The shared implementation and
language adapters may temporarily lag while the draft is completed. An
implementation MUST NOT claim conformance until it satisfies the model,
profile, validation, and native-conversion rules in this document.

The normative terms **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**,
**SHOULD NOT**, and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

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
properties MUST use an explicit application codec or `OpaqueBytes`.

## 2. Value model

The public model is codec-independent:

```text
Value =
    Null
  | Undefined
  | Boolean(value)
  | Integer(value)
  | Float(width, raw_bits)
  | ByteString(bytes)
  | TextString(utf8)
  | Array(values)
  | Map(entries)
```

`Undefined` is distinct from `Null`. A binding that has no native undefined
value MUST expose it through its lossless value representation or report a
conversion error in strict native mode.

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

`Float` contains both its IEEE-754 width and the raw bits available from the
source representation:

```text
Float16(width=16, raw_bits)
Float32(width=32, raw_bits)
Float64(width=64, raw_bits)
```

The model distinguishes `Integer(1)` from `Float64(1.0)`, and distinguishes
positive and negative zero. Float width and raw bits MUST be preserved by the
generic value representation and by an encoder that receives those bits.

Native runtimes may expose weaker guarantees. In particular, JavaScript does
not provide a portable way to observe or construct every NaN payload. A
JavaScript adapter MUST encode the IEEE-754 bits observable from its runtime
value and MUST NOT intentionally normalize a value that the runtime exposes.
Applications that require a particular NaN payload MUST use the generic or
encoded representation.

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

All model values MAY be map keys, including arrays and maps. A decoder MUST
reject a map containing duplicate keys under these rules. If a binding cannot
represent a map without losing key identity, it MUST return a lossless map
view or a conversion error; it MUST NOT silently overwrite, merge, stringify,
or drop an entry.

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

Each maintained package MUST document the native type returned for `Integer`,
the behavior of Float16/Float32 decoding, and any value that requires a
lossless wrapper.

## 4. Representation options

The client API SHOULD expose one `get` operation with a representation option
rather than forcing callers to learn separate method names. Language syntax
may differ, but the semantics are shared:

```text
get(key, representation="lossless")
get(key, representation="native")
get(key, representation="encoded")
```

`lossless` SHOULD be the default for dynamic maintained bindings.

### 4.1 Lossless representation

`lossless` uses native values whenever the conversion is exact and wraps only
the smallest subtree that cannot be represented by the language's native
containers. A lossless wrapper MUST support ordinary value access:

- indexing where the model is indexed;
- lookup and membership;
- length;
- iteration; and
- map keys, values, and entries.

The wrapper is not required to pass every native reflection or serialization
test. For example, it need not be an actual Python `dict` or JavaScript plain
object. A lookup that is ambiguous under native equality MUST report an
ambiguity error rather than choose an entry.

### 4.2 Native representation

`native` requires the complete value to be representable by the binding's
documented native types. It MUST return a conversion error for an unsupported
value, an unrepresentable map key, or a map that would collapse entries.

This mode is intended for APIs that require a real `dict`, plain object, or
other native container. It is not allowed to silently lose information.

### 4.3 Encoded representation

`encoded` returns the complete structured payload bytes produced by the
selected value profile. It does not expose CBOR types or tags as the public
value model. A caller that needs byte-exact forwarding MUST use this mode
instead of decoding and re-encoding the value.

`OpaqueBytes` operations bypass this model and return the exact application
bytes directly.

## 5. Initial codec profile

The first profile maps the model to one complete CBOR data item. It delegates
the byte grammar to [RFC 8949](https://www.rfc-editor.org/rfc/rfc8949) and the
floating-point representation to IEEE-754.

The profile rules are:

- exactly one complete CBOR data item MUST be present;
- trailing bytes and CBOR sequences MUST be rejected;
- arrays, maps, byte strings, and text strings MUST use definite lengths;
- text strings MUST contain well-formed UTF-8;
- null, booleans, undefined, and supported simple values map to their
  corresponding logical value kinds;
- ordinary integer values use CBOR major type 0 or 1;
- values outside the ordinary CBOR integer range use the standard bignum
  representation (tags 2 and 3) and decode to the same logical `Integer`;
- encoders MUST use the ordinary integer representation whenever it can
  represent the value and MUST NOT use bignum as a public source-type marker;
- valid non-preferred integer encodings MAY be accepted by decoders, but they
  MUST have the same logical value;
- Float16, Float32, and Float64 retain their selected width and raw bits;
- map entries are emitted in their logical order, but map order has no
  semantic meaning; and
- tags other than those required internally for bignum are rejected in this
  first profile.

The profile does not require deterministic encoding of the complete payload.
Maintained Python and JavaScript encoders SHOULD preserve the iteration order
of their input `dict`, `Map`, or lossless entry view. This is a client
convenience and does not make map order part of wire equality or require
third-party implementations to preserve it.

The profile's parser MUST apply the common payload and nesting limits supplied
by the enclosing client value format. This document does not add a separate
bignum limit.

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
rejection, lossless/native representations, and wrapper behavior because the
underlying codec standards do not define those product-level contracts.

Schema-bound formats such as protobuf are future payload profiles, not
transparent replacements for the generic `Value` model. A protobuf profile
requires a schema and preserves protobuf's own field, integer-width, and
unknown-field semantics.

## 7. Implementation boundary

The reusable implementation is intended to live under `clients/value/` as a
small shared value package. It may expose a Rust core and language adapters,
but the logical model and profile conformance remain one specification.

The OpenKache client core uses this package to produce structured payload
bytes. The client value envelope then applies compression, cryptographic
protection, key selection, and storage binding as specified by
[`../VALUE_FORMAT.md`](../VALUE_FORMAT.md). The server continues to treat the
complete envelope as opaque bytes.

This specification is deliberately independent of Item ID derivation and
formatted-key behavior, which remain defined by
[`../KEY_FORMAT.md`](../KEY_FORMAT.md).
