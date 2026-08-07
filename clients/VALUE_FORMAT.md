# OpenKache v1 client value and key encoding

> **Status: Draft — v1 pre-freeze**
>
> This document is the normative candidate for the formatted client APIs.
> Previous draft encodings are not compatibility targets. The contract becomes
> wire-stable only when v1 is frozen.

This document specifies two boundaries:

1. how a language-native key becomes canonical bytes before Item ID derivation;
2. how a structured or RawBytes client value becomes the opaque byte string
   stored by the server.

The server stores that byte string opaquely; it never parses serialization,
compression, encryption, or client metadata.

The design is still pre-freeze. The current Rust core and generated language
packages contain the previous JSON/derivation implementation and must not be
treated as v1-interoperable until they are migrated to this document. This
document is the contract to implement before the v1 freeze.

The [wire protocol specification](../protocol/SPEC.md) defines frame boundaries,
the 64 MiB wire limit, namespace operations, and the server's opaque-value
contract. It also defines the one canonical unsigned `vu128` encoding used by
both documents.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

## Contract at a glance

For structured values and formatted keys, the conversion contract has one
strict shape:

```text
native language value
  -> to_portable_value(native_value): PortableValue
  -> OpenKache deterministic CBOR
  -> exact canonical bytes
```

| Input | Canonical boundary |
|---|---|
| Formatted key | Canonical bytes are prefixed with `namespace_id:u64be` and hashed into the 32-byte Item ID. |
| Structured value | The same `PortableValue` conversion is encoded by the selected deterministic-CBOR codec. |
| RawBytes value | The native-to-portable mapping is bypassed; application bytes are copied exactly, including empty input and embedded NUL bytes. |

The five invariants every implementation must preserve are:

1. **Same portable value, same bytes.** Every supported language produces the
   same bytes for the same explicit `PortableValue`.
2. **No silent loss.** Ambiguous, unsupported, cyclic, out-of-range, or
   non-canonical input fails before hashing or storage.
3. **Type identity is intentional.** Text, bytes, and Integer remain distinct.
   Unsupported numeric, boolean, object, and serialization forms fail rather
   than being guessed or silently coerced.
4. **No serializer luck.** JSON, `toString()`, reflection, locale formatting,
   object insertion order, and library-specific serializer choices are not
   part of the contract.
5. **One rule for keys and values.** Key canonicalization and structured-value
   canonicalization share the same native-to-portable mapping.

This is a typed binary contract, not a promise to serialize every native
language object. An adapter supports the portable model explicitly and rejects
everything else.

## Scope and non-goals

In scope:

- canonical key bytes and namespace-bound Item ID input;
- RawBytes and deterministic CBOR structured values;
- canonical Integer identity and checked numeric normalization;
- compression, encryption, authenticated headers, and bounded decoding;
- language-neutral conformance vectors.

Out of scope for v1:

- server-side interpretation of client values;
- generic JSON or MessagePack interoperability;
- reflection-based object serialization;
- lossy numeric coercion and floating-point wire identity;
- replay/freshness semantics or server CAS;
- fixed-width integer identity on the wire;

## Decision register

The binary contract is explicit about what is fixed and what remains a
pre-freeze product decision:

| Area | v1 state |
|---|---|
| Structured codec | Fixed: OpenKache deterministic CBOR. |
| PortableValue scope | Fixed: `Integer`, `Text`, and `Bytes` only; containers, `Null`, and `Bool` are unsupported. |
| Integer identity | Fixed: one mathematical `Integer` type using canonical CBOR major types `0`/`1` and standard bignum tags `2`/`3`. |
| Integer width | Not represented on the v1 wire; fixed-width identity is deferred. |
| Floating point | No v1 wire type; only checked exact-integral inputs normalize to `Integer`; all other floating-point inputs are rejected. |
| Language mapping | Fixed: native integers and exactly integral floating-point values map to `Integer` under the checked normalization rule. |
| JavaScript `BigInt` | Fixed: maps exactly to `Integer`; non-integral, unsafe, or special `number` values are rejected. |
| Formatted key resource policy | Fixed SDK policy: `MAX_CANONICAL_KEY_BYTES = 1 MiB`; oversized keys are rejected before hashing. |
| Profile compatibility | Fixed: immutable profiles; multi-version clients may read/write supported profiles. |
| Replay/freshness | Open P1; no freshness claim in the v1 ValueEnvelope. |
| SDK profile default | `OldestCompatible`, subject to security and type requirements. |

## Normative decisions

The v1 design fixes the following choices:

- `namespace_id` is bound independently into both Item ID derivation and value
  AAD.
- The 32-byte root secret is named `client_root_key`.
- Every formatted key uses one native-to-portable mapping followed by
  deterministic CBOR. There is no separate raw-byte-key or structured-key
  codec.
- All native integer types map to one mathematical `Integer` portable type.
  Integer width and signed/unsigned representation are not part of v1 wire
  identity. Canonical CBOR major types `0`/`1` and standard bignum tags `2`/`3`
  provide the complete v1 integer encoding.
- V1 has no floating-point portable type. A native floating-point value MAY be
  normalized to `Integer` only when it is finite, not negative zero, exactly
  integral without rounding, and within the safe-integer range
  `-(2^53 - 1)` through `2^53 - 1`. Every other floating-point value MUST be
  rejected before hashing or storage.
- Values provide RawBytes and deterministic CBOR. There is no separate
  client-only metadata envelope.
- Compression, encryption, and value codec selection are independent options.
  The wire format supports no compression, Zstandard, no protection,
  DeterministicSIV protection, RandomizedGCM-SIV protection, RawBytes, and
  deterministic CBOR.
- Standalone variable-width integers use the protocol's canonical unsigned
  64-bit `vu128` encoding. Value-format transform and codec identifiers are
  fixed-width bit fields in the packed `format_flags` byte.
- Each profile version is immutable. A client MAY support multiple complete
  profiles and MUST select one profile for both key derivation and value
  encoding; it MUST NOT mix rules from different versions.
- Replay/freshness protection is intentionally not frozen in v1.

## Processing model

A formatted write performs these stages:

```text
native value or exact RawBytes
  -> core logical value when structured
  -> selected value codec
  -> optional Zstandard compression
  -> optional authenticated encryption
  -> v1 ValueEnvelope
  -> opaque server value
```

A read reverses the stages. Compression always precedes encryption. The
one-byte `format_flags` field identifies the selected codec and transforms
before the body is decoded. `format_flags` is visible to the server but is
authenticated as associated data by protected profiles.

The format has no magic bytes and no body-length field. The surrounding
protocol frame supplies the exact value boundary. `value_len` in the `SET`
frame is the canonical `vu128` length of the complete ValueEnvelope.

## Canonical unsigned `vu128`

The value format reuses the protocol's canonical unsigned 64-bit `vu128`
encoding verbatim. See
[Unsigned `vu128`](../protocol/SPEC.md#unsigned-vu128) for the normative
bit layout and vectors.

All variable-width integers in this document:

- MUST use the shortest canonical encoding;
- MUST be decoded with checked arithmetic;
- MUST reject truncation, overflow, reserved prefixes, and overlong encodings;
- are limited to the unsigned 64-bit range in v1.

The maximum encoded width is therefore 9 bytes. In this document,
`profile_version` is the only variable-width header field. Value codec,
compression, and encryption identifiers are fixed-width subfields of the
packed `format_flags` byte and do not have separate `vu128` encodings.

## ValueEnvelope layout

```text
value_envelope = profile_version:vu128 | format_flags:u8 | body
```

| Field | Size | Meaning |
|---|---:|---|
| `profile_version` | 1–9 bytes | Canonical Wire Profile Version; v1 is `01`. |
| `format_flags` | 1 byte | Packed encryption, compression, and codec identifiers. |
| `body` | remaining bytes | Codec payload, optionally compressed and encrypted. |

The wire has one packed `format_flags` byte rather than a separate header byte
for each transformation. Each layer owns its fixed-width identifier field,
while the body still applies the transformations in codec, compression, then
encryption order.

The `format_flags` byte is packed from three fixed-width two-bit identifiers:

```text
bits 0..1 = encryption_id
bits 2..3 = compression_id
bits 4..5 = codec_id
bits 6..7 = unassigned in v1; MUST be zero

format_flags =
  encryption_id
  | (compression_id << 2)
  | (codec_id << 4)
```

### Compression identifiers

| ID | Name |
|---:|---|
| `0` | None |
| `1` | Zstandard |
| `2..3` | Unassigned in v1 |

### Encryption identifiers

| ID | Name | Profile |
|---:|---|---|
| `0` | None | Unprotected |
| `1` | AES-256-SIV-CMAC | DeterministicSIV |
| `2` | AES-256-GCM-SIV | RandomizedGCM-SIV |
| `3` | Unassigned in v1 |

A decoder MUST reject nonzero unassigned bits and every identifier not assigned
by v1.
It MUST NOT guess an algorithm or try several interpretations.

Common `format_flags` bytes are:

| Byte | Encryption | Compression | Codec |
|---:|---|---|---|
| `00` | Unprotected | None | RawBytes |
| `04` | Unprotected | Zstandard | RawBytes |
| `10` | Unprotected | None | Deterministic CBOR |
| `14` | Unprotected | Zstandard | Deterministic CBOR |
| `01` | DeterministicSIV | None | RawBytes |
| `05` | DeterministicSIV | Zstandard | RawBytes |
| `11` | DeterministicSIV | None | Deterministic CBOR |
| `15` | DeterministicSIV | Zstandard | Deterministic CBOR |
| `02` | RandomizedGCM-SIV | None | RawBytes |
| `06` | RandomizedGCM-SIV | Zstandard | RawBytes |
| `12` | RandomizedGCM-SIV | None | Deterministic CBOR |
| `16` | RandomizedGCM-SIV | Zstandard | Deterministic CBOR |

### Codec identifiers

| ID | Name |
|---:|---|
| `0` | RawBytes |
| `1` | Deterministic CBOR |
| `2..3` | Unassigned in v1 |

An unknown codec identifier is an unsupported-codec error. Language adapters
MUST NOT assign identifiers or write a serializer that is not part of the core
contract.

The body before compression or encryption is exactly the selected codec
payload:

```text
codec_payload =
  exact_application_bytes                  // RawBytes
  | deterministic_cbor(core_logical_value) // deterministic CBOR
transformed_body = encrypt(compress(codec_payload))
```

The `codec_id` is in `format_flags`, not in `codec_payload` or
`transformed_body`.

### RawBytes

```text
raw_codec_payload = exact_application_bytes
```

An empty application byte string is valid and has a zero-byte codec payload.
Embedded `00` octets are ordinary payload bytes. There is no NUL terminator,
text sentinel, codec marker, or implicit application-value length inside
RawBytes.

Consequently, an empty RawBytes value occupies 2 bytes in the uncompressed,
unprotected profile (`profile_version=01 | format_flags=00`).

RawBytes is also the escape hatch for application-defined binary data. If an
application wants to put client-only metadata next to its value, it can define
that metadata inside its own RawBytes payload. v1 does not add a second envelope
or length field for such data.

### CBOR codec

The structured-value codec is CBOR as specified by
[RFC 8949](https://www.rfc-editor.org/rfc/rfc8949). RFC 8949 is normative for
the CBOR data model, major types, tags, preferred serialization, and encoding
validity. Its deterministic-encoding rules in
[§4.2](https://www.rfc-editor.org/rfc/rfc8949#section-4.2) are the v1 baseline.
This document does not restate those rules; the remainder of this section lists
only OpenKache's restrictions and deliberate differences.

The v1 portable logical model is deliberately small:

```text
PortableValue =
    Integer
  | Text
  | Bytes
```

A language adapter converts a supported native value to this model; it MUST NOT
call a language JSON/CBOR serializer and pass the resulting bytes through as if
they were canonical. `RawBytes` remains available when an application needs to
store a value outside this structured model.

OpenKache adds the following restrictions or overrides:

- **Closed model.** Only `Integer`, `Text`, and `Bytes` are structured values in
  v1. Unsupported native values MUST be rejected rather than stringified,
  omitted, coerced, or assigned an implementation-defined extension. Objects,
  maps, arrays, booleans, nulls, custom class instances, and JavaScript
  `undefined` are not v1 `PortableValue`s.
- **Integer identity.** All native integer types become one mathematical
  `Integer`. Use the RFC's major types `0` and `1`, and standard bignum tags `2`
  and `3` only when the magnitude does not fit the corresponding basic type.
  OpenKache adds no width or signedness tags. Bignum magnitudes are big-endian
  and shortest; leading-zero and overlong forms are rejected. An Integer whose
  magnitude exceeds the configured v1 resource limit MUST be rejected before
  allocation or hashing.
- **No floating-point wire type.** Binary16, binary32, binary64, decimal,
  extended-precision, and arbitrary-precision floating values are not in the
  v1 model. A finite floating value that is not negative zero, exactly integral,
  and within the implicit safe-integer range is normalized to `Integer` before
  encoding. Float items, non-integral values, NaN, infinity, and negative zero
  MUST be rejected before hashing or storage. This keeps v1 numeric identity
  exact and language-neutral; applications that need floating-point data can
  use RawBytes or a future versioned profile.
- **Text and bytes.** `Text` is valid UTF-8 with no Unicode normalization,
  locale encoding, NUL terminator, or replacement of malformed input. `Bytes`
  is an exact byte string; empty bytes and embedded `00` octets are valid.
- **Tags.** Tags `2` and `3` are reserved for `Integer`. Unknown and
  unassigned tags are rejected by v1 readers, even though a general CBOR
  implementation may preserve them.
- **Single item.** A ValueEnvelope codec payload contains exactly one complete
  CBOR item. CBOR sequences, trailing bytes, and a valid item followed by
  another item are rejected.

The core decoder MUST re-encode a decoded logical value and reject input that
is not the exact OpenKache deterministic representation.

### CBOR extensions

Standard bignum tags `2` and `3` are assigned to `Integer` as described above.
Future profiles MAY add floating-point, UUID, Decimal, or timestamp logical
types with an explicit canonical CBOR encoding (primitive, tagged, or a new
codec).
Each assigned tag MUST define one canonical payload encoding and any required
validation. Unknown or unassigned tags MUST be rejected by v1 readers.

Adding an extension does not change the encoding of an existing portable type.
Changing an existing type's meaning or canonical bytes requires a new value
codec or Wire Profile Version.

## Item ID conversion philosophy

An Item ID is a stable logical identity, not an incidental result of a
language's serializer. The conversion contract therefore follows these
principles:

- The same portable value MUST produce the same canonical bytes and Item ID in
  every supported language and runtime.
- An unsupported or ambiguous native value MUST fail before hashing. Silent
  `toString()`, `JSON.stringify()`, locale formatting, field omission, map
  iteration order, or lossy numeric conversion is forbidden.
- Semantic distinctions that matter to identity are preserved: Text is
  different from Bytes, and Integer is different from either. Floating-point
  values are either rejected or normalized to `Integer` only by the checked
  exact-integral rule.
- Native integer width and signed/unsigned representation do not become
  identity. Native types that are not supported by the model are rejected.
- Item ID conversion is independent of value compression, encryption, and the
  ValueEnvelope's `format_flags`. It is performed once, before the keyed hash.
- The keyed hash makes the identity opaque and binds it to the
  `(client_root_key, namespace_id)` security domain; it does not provide
  freshness, versioning, or a reversible encoding of the application key.
- Any change to the native-to-portable mapping, canonical bytes, or derivation
  context requires a new versioned contract. A v1 reader MUST NOT reinterpret
  v1 bytes under a different rule.

This philosophy intentionally resembles a typed tuple layer, but the v1
portable representation is deterministic CBOR and not a language-specific tuple
serializer.

### Native-to-Portable Mapping

Language bindings MAY provide ergonomic overloads, but convenience MUST only
cover a one-to-one native mapping or the explicit checked exact-integral
conversion defined below. The binding MUST reject an input when it would have
to infer intent:

- A native string is always `Text`; it is never parsed as JSON, decoded as
  Base64, or treated as UTF-8 `Bytes`.
- A byte buffer is always `Bytes`; it is never guessed to be text.
- A native integer maps to `Integer` regardless of its native width or
  signed/unsigned representation. It MUST be represented exactly and MUST
  never be routed through a floating-point or decimal conversion. An
  implementation MUST enforce a checked resource limit before allocating or
  hashing an oversized arbitrary-precision integer.
- A native floating-point value has no v1 floating-point representation. It
  MAY be normalized to `Integer` only when it is finite, not negative zero,
  exactly integral without rounding, and within the safe-integer range
  `-(2^53 - 1)` through `2^53 - 1`. Non-integral, special, or out-of-range
  values MUST be rejected rather than rounded, truncated, or stringified.
- Objects, lists/arrays, dictionaries/maps, custom class instances, reflection-
  based serialization, `toJSON()`/`__str__()` hooks, symbols, hidden fields,
  and JavaScript `undefined` MUST be rejected for the v1 structured mapping.

JavaScript and TypeScript have one `number` runtime type. A `number` maps to
`Integer` only when all of the following hold:

- it is finite;
- it is not negative zero;
- `Number.isSafeInteger(number)` is true.

Such a value maps to `Integer` exactly. Every other JavaScript/TypeScript
`number`, including non-integral values, unsafe integers, `NaN`, infinity, and
negative zero, MUST be rejected. `BigInt` maps to `Integer` without narrowing
or width inference. The safe-integer range is
`-(2^53 - 1)` through `2^53 - 1`.

Bindings SHOULD expose explicit constructors or a `PortableValue`/variant API
for the three v1 types. Choosing `Text("0012")` versus
`Bytes([30, 30, 31, 32])` must be visible in the call site. A broad
`Any`/`object` parameter is acceptable only when it performs the same strict
validation and returns a deterministic unsupported-value error; it MUST NOT
silently fall back to a string representation.

### Exact-integral numeric normalization

The following language-neutral algorithm is normative. It converts a native
numeric input to the only numeric v1 type, `Integer`; it is not a general
numeric coercion routine:

```text
normalize_numeric_input(x):
    if x is a native integer:
        n = exact_integer_value(x)
        enforce_integer_resource_limit(n)
        return Integer(n)

    if x is a native binary floating-point value:
        if not finite(x):
            reject
        if is_negative_zero(x):
            reject
        if abs(x) > (2^53 - 1):
            reject
        if x is not mathematically integral:
            reject
        n = exact_integer_value_without_rounding(x)
        enforce_integer_resource_limit(n)
        return Integer(n)

    reject
```

`exact_integer_value_without_rounding` extracts the integer represented by the
floating-point value; it MUST NOT call decimal string formatting, JSON
serialization, rounding, truncation, or an intermediate numeric type that can
lose information. The finite, integral, and range checks occur before the
conversion. The negative-zero check is semantic, not merely `x == 0`: an IEEE
754 value whose sign bit is set and whose magnitude is zero is rejected.
`x is mathematically integral` means that the represented binary floating-point
real value has no fractional component; it MUST NOT be established by rounding
or truncating a non-integral value.
`enforce_integer_resource_limit(n)` rejects when the shortest canonical CBOR
encoding of `Integer(n)` would exceed `MAX_CANONICAL_KEY_BYTES` for a formatted
key (or the configured structured-value limit for a value).

For JavaScript `number`, `Number.isSafeInteger(x)` supplies the finite,
integral, and range checks, but the explicit negative-zero check still applies.
`BigInt` follows the native-integer branch and is never narrowed to `number`.
The same algorithm applies to Rust `f32`/`f64` and Python `float`; the
floating-point format does not become part of the resulting identity.

### Language mapping matrix

The adapter mapping is part of the protocol contract. The following defaults
are normative; the checked JavaScript-number exception is value-validated as
specified above:

| Native type | Portable type | Rule |
|---|---|---|
| Rust `f32`/`f64` | `Integer` or rejected | Normalize only exact integral finite values that are not negative zero and are within the safe range. |
| Rust fixed-width integer | `Integer` | Native width and signedness are not wire identity. |
| Python `float` | `Integer` or rejected | `1.0` maps to `Integer(1)`; non-integral, special, negative-zero, or unsafe values are rejected. |
| Python `int` | `Integer` | Preserve the exact mathematical value; enforce resource limits. |
| JavaScript `number` | `Integer` or rejected | Accept only finite safe integers that are not negative zero; reject all other values. |
| JavaScript `BigInt` | `Integer` | Preserve the exact mathematical value; use standard bignum encoding when needed. |

The same checked exact-integral rule applies to all native binary32 and
binary64 values. Decimal and arbitrary-precision numeric types MUST NOT be
implicitly converted; callers need an explicit, checked conversion before
calling the portable API.

For JavaScript/TypeScript, the mapping is therefore:

```text
number 1                         -> Integer(1)
number 1.0                       -> Integer(1)
number Number.MAX_SAFE_INTEGER   -> Integer(Number.MAX_SAFE_INTEGER)
number 1.5                       -> rejected
number -0                        -> rejected
number NaN or Infinity           -> rejected
number outside the safe range   -> rejected
bigint 1n                        -> Integer(1)
```

## Key Canonicalization Contract

The formatted key pipeline is:

```text
native_key
  -> to_portable_value(native_key): PortableValue
  -> deterministic_cbor(PortableValue): canonical_key_bytes
  -> namespace_id:u64be | canonical_key_bytes
  -> BLAKE3 keyed Item ID derivation
```

`canonical_key_bytes` is exactly one complete deterministic-CBOR item. It has no
key codec identifier, extra length, NUL terminator, or textual encoding. CBOR's
self-delimiting item structure and the fixed-width namespace prefix provide the
unambiguous boundary.

### Key specification and assertions

The default key specification is semantic: supported native integers become
`Integer`, strings become `Text`, and byte buffers become `Bytes`. A binding MAY
expose a `KeyCanonicalizationSpec` (short form: `KeySpec`) to assert the
expected portable type:

```text
KeySpec::Integer
KeySpec::Text
KeySpec::Bytes
```

The specification is an assertion and canonicalization contract, not a license
to coerce values. A value that does not match the requested portable type MUST
be rejected. V1 has no wire-level fixed-width integer assertion;
fixed-width integer assertions are deferred to a future profile. A key
specification MAY be fixed when a client, namespace handle, or keyspace is
created so every key operation uses the same contract. The specification adds
no wire field; it only selects the canonical `PortableValue` mapping before
hashing.

Canonical key-byte vectors:

```text
Text("abc")              -> 63 61 62 63
Bytes([00, ff])          -> 42 00 ff
Integer(1)               -> 01
Integer(-1)              -> 20
Text("")                -> 60
Bytes([])                -> 40
```

These are conformance vectors, not illustrative serializer output. A
conforming implementation MUST produce the exact bytes shown.
A checked native floating-point input with the value `1.0` normalizes to
`Integer(1)` and therefore produces `01`; it never emits a CBOR float item.

### Item ID conformance vectors

The following vectors are normative end-to-end tests of the Item ID pipeline.
All hex values are octets separated by spaces. Unless a vector says otherwise,
it uses this exact root and derived key:

```text
client_root_key =
  00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f
  10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f

item_id_derivation_key =
  ef 08 6a 3f 6e 66 3e df 41 08 98 2d 51 2a 4a 1b
  92 22 5d c5 82 e1 06 a0 f0 29 f9 e6 3b 91 7b 4b
```

| Vector | `namespace_id` | `canonical_key_bytes` | `item_id_input` | final `item_id` |
|---|---:|---|---|---|
| `Text("abc")` | `1` | `63 61 62 63` | `00 00 00 00 00 00 00 01 63 61 62 63` | `42 7b da c9 1f 3b 4a 91 e6 84 4e df 91 5f de 24 6a 8c 5f fd c6 53 8a b2 73 d9 b3 8a c7 6e d3 b5` |
| `Bytes([00, ff])` | `1` | `42 00 ff` | `00 00 00 00 00 00 00 01 42 00 ff` | `37 8f 5b c4 75 ef 94 54 49 58 3a e5 a5 34 16 45 35 28 1a 63 44 63 6b 63 ec 88 70 57 6e 0e 7e 41` |
| `Integer(1)`; JavaScript `number 1`, Python `float 1.0`, and JavaScript `BigInt 1n` | `1` | `01` | `00 00 00 00 00 00 00 01 01` | `cf 3c 3f db 98 f7 ce 3f 81 a7 04 a3 9b 88 2d e2 a0 58 37 d3 d1 c0 56 81 84 38 23 74 11 88 54 7c` |
| `Integer(-1)` | `1` | `20` | `00 00 00 00 00 00 00 01 20` | `63 3b 3c a8 10 66 f7 25 0c 6a f1 1f 14 62 d3 a1 48 4f 12 16 a0 6f 1e f5 65 46 78 65 c3 28 81 d6` |
| `Text("")` | `1` | `60` | `00 00 00 00 00 00 00 01 60` | `4a 09 7c c0 5f b5 4d 3b e2 1a f5 83 dc b4 b3 d9 e8 d7 6f f6 f6 5b 14 78 5f dd f7 56 d1 aa 13 bc` |
| `Bytes([])` | `1` | `40` | `00 00 00 00 00 00 00 01 40` | `6a 45 bd 6f 9f a7 94 a0 08 9b ed 46 b1 ee f3 af 0f 69 12 ec 08 b5 0a 02 c0 f4 f3 9f be c0 5e cc` |
| `Text("abc")`, namespace separation | `2` | `63 61 62 63` | `00 00 00 00 00 00 00 02 63 61 62 63` | `8d be e5 54 99 ed f7 43 4c f2 b9 02 42 e7 d3 60 94 05 d2 08 06 a7 e5 27 5b ce 83 b7 94 f8 50 35` |

Changing either the root key or namespace must change the resulting Item ID.
For the root-key separation vector, keep `namespace_id = 1` and
`canonical_key_bytes = 63 61 62 63`, and replace the common fixture with:

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

The following inputs are rejected by the v1 structured codec:

```text
18 01                      // overlong Integer(1); canonical form is 01
f9 3c 00                   // binary16 Float(1.0); floating point unsupported
fa 3f 80 00 00             // binary32 Float(1.0); floating point unsupported
fb 3f f0 00 00 00 00 00 00 // binary64 Float(1.0); floating point unsupported
c2 41 01                   // bignum for 1; canonical form is 01
c2 42 00 01                // leading zero in a bignum magnitude
d8 18 01                   // disallowed tag 24 at the value root
80                         // array; containers are outside the v1 PortableValue model
a0                         // map; containers are outside the v1 PortableValue model
f4                         // boolean; not a v1 PortableValue
f6                         // null; not a v1 PortableValue
60 00                      // valid item followed by trailing bytes
```

The same `to_portable_value(...)` rules apply to a structured value before its
selected value codec. A byte-oriented native key is therefore represented as a
CBOR byte string in the formatted-key contract; the raw protocol client's
explicit 32-byte Item ID API remains the escape hatch for callers that already
own an Item ID.

Language bindings SHOULD expose the portable model directly or provide
constructors that make the conversion explicit. They MUST NOT use reflection or
custom object hooks to invent additional key types.

Formatted keys are hashed and are not stored in a protocol frame, so this is
not a server wire limit. Every v1 SDK MUST apply the following shared resource
policy before hashing:

```text
MAX_CANONICAL_KEY_BYTES = 1,048,576  // 1 MiB
```

The limit applies to the exact `canonical_key_bytes` length, including its
canonical CBOR type/length header and any bignum magnitude, but excluding the
8-octet `namespace_id` prefix. An oversized Text, Bytes, or Integer key MUST be
rejected before hashing. For arbitrary-precision native integers, the adapter
MUST check the encoded-size budget before materializing an unbounded canonical
buffer. Bindings MAY expose a lower deployment-specific limit, but MUST NOT
silently allocate or hash beyond this common v1 budget. The 64 MiB value limit
does not become a larger key limit.

## Conformance checklist

Before calling a language adapter interoperable, verify all of the following:

- It maps every supported native type through the published language matrix.
- It has explicit constructors for every portable type the native language
  cannot express directly.
- It passes the canonical vectors above byte-for-byte.
- It rejects every invalid vector before hashing or storage.
- It normalizes only finite, exact, safe integral floating-point values to
  `Integer`; it rejects non-integral values, NaN, infinity, negative zero, and
  out-of-range values.
- It preserves the exact Integer value and empty byte strings.
- It does not invoke JSON serialization, locale formatting, reflection, or
  user-defined stringification as a fallback.
- It applies the same `to_portable_value(...)` mapping to key and
  structured-value inputs.
- It enforces decoder limits before allocating from untrusted lengths.

## `client_root_key` and Item ID derivation

`client_root_key` is exactly 32 bytes with 256 bits of cryptographic
randomness. A password, human-readable string, padded string, or truncated
secret is not a valid root key. The key remains inside the client.

The root key SHOULD be unique to one application security domain. Sharing
entries intentionally requires sharing the same root key and namespace
identity. Unrelated applications or environments SHOULD use different keys.

All context strings below are exact, case-sensitive UTF-8 bytes. They have no
terminator, implicit length, or implementation-specific prefix.

```text
item_id_derivation_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache item ID derivation root v1",
    material = client_root_key[32]
  )
```

The v1 `profile_version` is not an additional prefix in the Item ID input; the
versioned derivation context above is the v1 binding. A future profile that
changes key identity or derivation material MUST use its own assigned context
and complete profile contract, and MUST NOT reinterpret a v1 Item ID under the
changed rules.

The namespace identity is fixed-width big-endian:

```text
namespace_id_bytes = namespace_id:u64be
```

`namespace_id` MUST be a positive server-assigned namespace ID. The wire
protocol's zero value is not a selector and MUST NOT be used by a formatted
client.

The exact Item ID hash input is:

```text
item_id_input = namespace_id_bytes | canonical_key_bytes

item_id =
  BLAKE3-KEYED-HASH(
    key   = item_id_derivation_key[32],
    input = item_id_input
  )
```

The result is the exact 32-byte wire Item ID. There is no additional hash,
length prefix, terminator, or textual encoding. Changing either
`client_root_key` or `namespace_id` intentionally makes the old item
unreachable.

The raw protocol client still accepts a caller-supplied 32-byte Item ID and
does not perform this derivation. It is an explicit escape hatch.

There is no key-rotation protocol in v1. A deployment that changes
`client_root_key` must migrate or repopulate entries under the new identity;
automatic dual-key reads and rotation metadata are out of scope.

## Value encryption root key derivation

Encrypted values derive a value-encryption root from the same client root key:

```text
value_encryption_root_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 root key",
    material = client_root_key[32]
  )

item_id_material = value_encryption_root_key[32] | item_id[32]
```

The two components are fixed-width and therefore unambiguous. Derived keys
MUST be zeroized when their operation or owning object ends. Implementations
SHOULD derive per-item keys on demand instead of retaining a high-cardinality
key cache.

## Associated Data (AAD)

Both protected profiles authenticate the same bytes:

```text
aad =
  ascii("openkache/value-format/aad/v1")
  | namespace_id_bytes
  | item_id[32]
  | encoded_profile_version
  | format_flags
```

`encoded_profile_version` is the exact canonical `vu128` byte sequence stored
in the ValueEnvelope. For v1 it is `01`. `format_flags` is the exact packed
byte stored after the profile version, including its zero-valued unassigned bits.

The namespace is included separately in both `item_id_input` and `aad`.
The complete AAD is not fed into Item ID derivation; doing that would create a
circular dependency because AAD contains the item ID.

Authenticating the namespace and Item ID prevents moving a valid ciphertext to
another `(namespace_id, item_id)` pair. Authenticating the header prevents
changing the profile version, codec, or transform algorithms. AAD does not
provide freshness: an older valid ValueEnvelope can still be replayed to the
same item.

## Protection profiles

### Unprotected (ID 0)

The transformed codec payload is stored directly:

```text
value_envelope = profile_version:vu128 | format_flags:u8 | transformed_body
```

This is the minimum-size option. It provides no confidentiality or value
authentication. Parser bounds and Zstandard validation are still mandatory.
Clients that require protection MUST reject this profile; accepting it must be
an explicit configuration choice.

### DeterministicSIV (ID 1)

DeterministicSIV uses deterministic AES-256-SIV-CMAC as specified by
[RFC 5297](https://www.rfc-editor.org/rfc/rfc5297). Derive independent 32-byte
keys:

```text
deterministic_siv_mac_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-SIV-CMAC MAC key",
    material = item_id_material[64]
  )

deterministic_siv_encryption_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-SIV-CMAC encryption key",
    material = item_id_material[64]
  )

deterministic_siv_key =
  deterministic_siv_mac_key[32] | deterministic_siv_encryption_key[32]
```

Pass the complete AAD as exactly one RFC 5297 associated-data component. Do
not split its fields into several S2V components and do not provide an
optional nonce component.

```text
deterministic_siv_body = synthetic_iv[16] | ciphertext
```

DeterministicSIV has 16 bytes of cryptographic overhead and no random nonce.
Identical transformed plaintext for one item produces identical stored bytes,
so the server can observe repetition of that item's complete value. It cannot
compare the same plaintext across different item IDs because their keys differ.

### RandomizedGCM-SIV (ID 2)

RandomizedGCM-SIV uses AES-256-GCM-SIV as specified by
[RFC 8452](https://www.rfc-editor.org/rfc/rfc8452):

```text
randomized_gcm_siv_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-GCM-SIV key",
    material = item_id_material[64]
  )

randomized_gcm_siv_body = nonce[12] | ciphertext | tag[16]
```

The encoder MUST obtain a fresh 12-byte nonce from the operating-system
cryptographic random source for every write, including repeated writes of the
same value. It MUST fail if the random source fails and MUST NOT use a
timestamp, item ID, plaintext, or process-local counter as a fallback.

RandomizedGCM-SIV has 28 bytes of cryptographic overhead. It is the recommended
protected profile because repeated writes have independent representations and
therefore do not expose DeterministicSIV's deterministic repetition. A nonce
remains public.

### Profile size comparison

For v1, the fixed overhead before the codec payload is:

| Profile | ValueEnvelope/crypto overhead | Empty RawBytes value |
|---|---:|---:|
| Unprotected | 2 bytes | 2 bytes |
| DeterministicSIV | 18 bytes | 18 bytes |
| RandomizedGCM-SIV | 30 bytes | 30 bytes |

RandomizedGCM-SIV is exactly 12 bytes larger than DeterministicSIV.
Compression, when selected, adds the Zstandard frame overhead and can make a
small value larger.

The wire format does not choose a default profile. The SDK policy default is
still a product decision; a protected SDK SHOULD choose RandomizedGCM-SIV, while
a caller optimizing for minimum size MAY choose DeterministicSIV when repetition
leakage is acceptable. Unprotected MUST remain an explicit opt-in.

## Compression

Compression ID `1` is one complete Zstandard frame as specified by
[RFC 8878](https://www.rfc-editor.org/rfc/rfc8878):

```text
compressed_body = zstd(codec_payload)
```

The encoder MUST:

- compress exactly the selected codec payload; the codec identifier is already
  in `format_flags`;
- emit exactly one standard frame with a declared content size;
- use no external dictionary and no skippable frame;
- append no trailing bytes;
- set the compression identifier only when the stored body is actually a
  Zstandard frame.

The decoder MUST reject skippable frames, missing content sizes, dictionary
requirements, multiple frames, trailing bytes, oversized windows, and declared
or produced output above the decoded-value limit.

Compression policy is not wire semantics. An encoder MAY compare compressed and
uncompressed lengths and keep the uncompressed body when compression is not
beneficial. The initial recommendation is Zstandard level 1, no compression
below 1,024 codec-payload bytes, and a minimum saving of 64 bytes.

### Compression and sensitive values

“Sensitive” here means a value that contains a secret (for example a token,
session credential, password-derived material, or private user data) together
with bytes an attacker can influence or repeatedly probe. Observing the
compressed length of chosen inputs can reveal information about the secret,
even when encryption is strong. This is the familiar compression length-oracle
risk.

For such values, compression SHOULD be disabled or the secret and
attacker-controlled data SHOULD be stored in separate items. Compression is
not automatically unsafe for every secret-only value, but disabling it is the
conservative policy when the relationship between the inputs is unknown.

Encryption does not hide the final stored length in any profile.

## Encoding procedure

An encoder MUST:

1. Convert a structured native value to the core logical model. A RawBytes
   value instead supplies its exact application bytes.
2. Encode the value with the selected codec to produce `codec_payload`.
3. Reject a codec payload above the decoded-value limit.
4. Apply Zstandard only when configured and beneficial.
5. Encode profile version `1` canonically and construct the packed
   `format_flags` byte.
6. Construct AAD from the namespace ID, exact Item ID, and exact header bytes.
7. Apply the selected protection profile.
8. Reject a complete ValueEnvelope above the protocol value limit.
9. Send the complete ValueEnvelope as the opaque protocol value.

The encoder MUST NOT emit a magic prefix, body-length field, unassigned
identifier, non-canonical `vu128`, nonzero unassigned `format_flags` bits, or a
protection profile, compression algorithm, or codec inconsistent with
`format_flags`.

## Decoding procedure

A decoder MUST:

1. Parse one canonical `profile_version:vu128` from the exact ValueEnvelope
   slice.
2. Require profile version `1`.
3. Read one `format_flags` byte and reject nonzero unassigned bits or
   encryption, compression, and codec IDs not assigned by v1.
4. Enforce the configured encryption policy.
5. Construct AAD from the supplied namespace ID, exact Item ID, and exact header.
6. Check profile-specific minimum body sizes before slicing.
7. Derive the selected per-item key(s).
8. Authenticate and decrypt before decompression or codec parsing.
9. If compressed, validate and decompress one bounded Zstandard frame.
10. Dispatch the selected codec from `format_flags` over the complete
    decompressed body.
11. Reject unsupported, non-deterministic, truncated, trailing, or oversized
    payloads.

Authentication errors MUST be reported as one generic value-authentication
failure. Implementations MUST zeroize unauthenticated plaintext before
returning that error.

## Limits

Two independent v1 limits apply:

1. The complete stored ValueEnvelope MUST be at most 64 MiB, including
   headers, compression framing, nonce, and authentication tag.
2. The decoded codec payload MUST be at most 64 MiB. The codec identifier is
   already in the one-byte `format_flags` field and is not counted in this
   limit.

Implementations MAY configure lower limits, but MUST apply them consistently
to encoding and decoding. Declared Zstandard content size, requested window
size, produced output, and all size arithmetic MUST be checked before
allocation.

## Replay and freshness (P1, intentionally open)

V1 protection authenticates identity and transform selection, not time or write
order. A server can replay an older valid value to the same namespace and Item
ID.
RandomizedGCM-SIV's random nonce changes the bytes; it does not stop rollback.

The current recommendation is to leave replay/freshness out of the frozen
binary layout until the server CAS contract is settled. Two future designs are
possible:

1. **Application generation**: the application stores a monotonic generation
   or version inside RawBytes/deterministic CBOR and rejects values older than
   its local expectation. This requires no wire change but does not give the
   cache a server-enforced CAS operation.
2. **Protocol generation/CAS**: the wire protocol carries an expected or
   returned generation and the server atomically checks it. The generation
   would then be authenticated as part of the value context or protocol
   operation.

“Generation/version + CAS as a separate protection profile” means the second
design: an opt-in profile would add a generation field and require an atomic
compare-and-set contract. It is not a third protection algorithm, and it is
not part of v1 yet. Do not infer freshness from an Item ID, DeterministicSIV
synthetic IV, RandomizedGCM-SIV nonce, TTL, or namespace revision.

## Protocol integration and responsibilities

The wire protocol owns frame lengths, namespaces, Item IDs, and server
semantics. This document starts at the exact opaque `value` slice in a request
or response and adds no protocol fields.

The shared core owns:

- canonical `vu128` parsing for this format;
- the RawBytes and deterministic-CBOR codec registry;
- the Key Canonicalization Contract and namespace-bound Item ID derivation;
- Zstandard policy and bounded decompression;
- BLAKE3 derivation and secret zeroization;
- DeterministicSIV and RandomizedGCM-SIV protection;
- stable validation and authentication errors.

Language adapters own only conversion between native values and the core
logical model, byte-buffer ownership, asynchronous/runtime APIs, and error
wrappers. They MUST NOT duplicate framing, codec canonicalization,
compression, key derivation, encryption, nonce generation, or ValueEnvelope
parsing.

## Versioning

The Wire Profile Version identifies a complete immutable profile. A v1 writer
emits profile version `1`; a v1 reader accepts profile version `1` only. A
future profile version is required for changes to:

- header order or field meaning;
- assigned algorithm IDs or body layouts;
- AAD construction;
- BLAKE3 context strings or derivation material;
- the Key Canonicalization Contract;
- an assigned codec's byte contract;
- the PortableValue model, including floating-point acceptance;
- canonical Integer and bignum encoding;
- a language-to-portable-type mapping table.

Every change to the v1 binary contract requires a new Wire Profile Version.
Identifiers and flag bits not assigned by v1 are invalid in v1; no
forward-compatible interpretation or future reuse is implied by this
document. A v2 specification may assign different meanings after defining its
own complete versioned contract.

Wire Profile Versions identify wire profiles, not client release numbers. An
editorial clarification that does not change canonical bytes or semantics does
not require a new profile.

The previous magic-prefixed envelope beginning with `4F 4B 56 01` is not v1
and MUST NOT be implicitly unwrapped or migrated. Package-local compatibility
may put that legacy payload inside RawBytes, but it is not a cross-language
codec.

## Profile compatibility and selection

Forward compatibility from an old client to a future profile is not a v1
requirement. A client that does not implement a Wire Profile Version MUST reject
it as an unsupported profile; it MUST NOT guess the codec, ignore encryption
metadata, or treat authenticated bytes as a different value type.

A newer client MAY implement several profiles simultaneously. When it does,
the selected profile covers the complete operation:

```text
profile(profile_version) =
  Key Canonicalization Contract
  + Item ID derivation context
  + ValueEnvelope and codec rules
  + encryption/AAD rules
```

Profile versions are capability extensions: a newer profile adds values or
policies that an older profile cannot represent, while retaining the older
profile as the representation for operations that already fit it. A newer
client MUST therefore use the oldest supported complete profile that can
represent the requested key, value, and security policy. In particular, a
v1-representable operation MUST be written with profile `1` by the normal
compatibility policy, including v1 Item ID derivation and v1 ValueEnvelope
rules. A newer profile is selected only when the operation or an explicit
caller policy requires a feature absent from the older profile.

The client MUST NOT calculate a v1 Item ID and then store a v2 value under it,
or use the v2 Key Canonicalization Contract with a v1 ValueEnvelope. A profile
selection is all-or-nothing: key identity, Item ID derivation context, value
encoding, protection, and AAD MUST come from the same profile.

The write policy has three forms:

| Policy | Behavior |
|---|---|
| `Exact(vN)` | Require profile `vN`; fail if it cannot represent the operation. |
| `OldestCompatible` | Use the oldest supported profile that preserves the value, key identity, and required security properties. |
| `LatestSupported` | An explicit opt-in to the newest supported profile; it MUST NOT be the default and may make older clients unable to read the entry. |

`OldestCompatible` is the v1 SDK default. It lets a new client continue
serving older clients while preserving the complete operation contract. It is
not permission to silently weaken security or lose a type distinction. A client
MUST fail rather than downgrade when the older profile cannot satisfy the
caller's requirements.

Reading an older profile is normal multi-version support. Migrating an entry to
a newer profile is an explicit read/decode/write operation; if the key
contract changes, the migration MUST account for the new Item ID rather than
assuming that the old and new identities are equal.

## Implementation migration checklist

The checked-in implementation is intentionally ahead of this design only in
the old direction. Before declaring v1 frozen, the following production
surfaces must consume this document as their source of truth:

- the Smithy client model and generated value-format constants;
- the shared key module (`client_root_key`, `KeySpec`, and namespace-bound Item
  ID derivation);
- the shared value module (deterministic CBOR Integer/bignum handling, one
  protocol `vu128`, and namespace-bound AAD);
- protected clients and FFI entry points, which must resolve the namespace
  before deriving an Item ID;
- language adapters and package documentation.

Until those changes land, old JSON ValueEnvelopes and old derivation outputs are
implementation artifacts, not v1 wire compatibility.
