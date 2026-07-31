# OpenKache value format

This document specifies the complete client-owned representation of values stored through
OpenKache's formatted APIs. It covers serialization, compression, authenticated encryption, key
derivation, validation, and the boundary between the shared client core and language adapters.

The [client index](README.md) describes implementation and migration status.
The [wire protocol specification](../protocol/SPEC.md) defines server-visible
framing and operation semantics.

The format is a pre-release design. Implementations may replace earlier
envelope and value-protection code without a compatibility path. Format version
`1` is the first specified version.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**, and **MAY** are to be
interpreted as described in [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

## Goals

The value format has five design goals:

1. The server stores one opaque byte string and never parses application values.
2. `openkache-client-core` is the only implementation of serialization, compression, key
   derivation, and encryption.
3. Language packages are thin adapters from native values to core-owned value types.
4. Per-item overhead remains small enough for caches containing many short values.
5. Decoders reject ambiguous, unsupported, authentication-failing, or oversized input before
   exposing it to an application codec.

The format contains no magic bytes and no encoded body length. An OpenKache protocol frame already
provides the exact container boundary. Callers that need to store arbitrary unformatted protocol
bytes use the low-level raw client rather than the formatted API.

## Processing model

A formatted write applies these stages in order:

```text
native language value
  -> core logical value
  -> core serialization
  -> optional Zstandard compression
  -> optional authenticated encryption
  -> value-format container
  -> opaque server storage
```

A read applies the exact reverse order. Compression always precedes encryption. Serialization
metadata is part of the transformed body, so encryption hides both the serialized payload and its
serialization identifier.

## Container

```text
container = version:vu128 | format:u8 | body
```

| Field | Size | Description |
|---|---:|---|
| `version` | 1–17 bytes | Canonical unsigned VU128 format version |
| `format` | 1 byte | Compression and encryption algorithm identifiers |
| `body` | remaining bytes | Serialized, optionally compressed, optionally encrypted value |

Version `1` encodes as the single byte `01`.

### Unsigned VU128

Unsigned integers use John Millikin's revised
[VU128 encoding](https://john-millikin.com/vu128-efficient-variable-length-integers). Implementations
MUST use the shortest canonical encoding and MUST reject truncated, overlong, or overflowing
encodings even if a low-level VU128 library accepts them.

The layouts relevant to this format are:

| Value range | Encoded size |
|---|---:|
| `[0, 2^7)` | 1 byte |
| `[2^7, 2^14)` | 2 bytes |
| `[2^14, 2^21)` | 3 bytes |
| `[2^21, 2^28)` | 4 bytes |
| `[2^28, 2^128)` | 5–17 bytes using a binary length prefix |

Examples:

```text
0          -> 00
1          -> 01
127        -> 7f
128        -> 80 02
16383      -> bf ff
16384      -> c0 00 02
```

Version and serialization identifiers currently fit in one byte, but using one canonical integer
encoding keeps their extension rules uniform.

### Format byte

The low nibble identifies compression. The high nibble identifies encryption.

```text
bits 0..3 = compression algorithm
bits 4..7 = encryption algorithm

format = compression_id | (encryption_id << 4)
```

Compression identifiers:

| ID | Name |
|---:|---|
| `0` | None |
| `1` | Zstandard |
| `2..15` | Reserved |

Encryption identifiers:

| ID | Name | Profile |
|---:|---|---|
| `0` | None | Unprotected |
| `1` | AES-256-SIV-CMAC | Compact |
| `2` | AES-256-GCM-SIV | Robust |
| `3..15` | Reserved |

Common format bytes:

| Byte | Meaning |
|---:|---|
| `0x00` | Serialized value without compression or encryption |
| `0x01` | Zstandard-compressed value |
| `0x10` | Compact AES-SIV value |
| `0x11` | Zstandard followed by Compact AES-SIV |
| `0x20` | Robust AES-GCM-SIV value |
| `0x21` | Zstandard followed by Robust AES-GCM-SIV |

A decoder MUST reject every unassigned compression or encryption identifier. It MUST NOT guess an
algorithm or try multiple algorithms.

For version `1`, fixed container overhead, excluding the serialized value and any Zstandard
framing, is:

| Profile | Version and format | Cryptographic metadata | Total |
|---|---:|---:|---:|
| Unprotected | 2 bytes | 0 bytes | 2 bytes |
| Compact | 2 bytes | 16-byte synthetic IV | 18 bytes |
| Robust | 2 bytes | 12-byte nonce and 16-byte tag | 30 bytes |

These totals are transform overhead, not minimum valid container sizes. A serialized value always
contains at least its one-byte serialization identifier. An empty raw value therefore occupies `3`
bytes when unprotected, `19` bytes with Compact encryption, or `31` bytes with Robust encryption.

## Serialized value

Before compression or encryption, the body has this representation:

```text
serialized_value = serialization_id:vu128 | serialization_payload
```

The serialization identifier is inside the transformed body. It is therefore compressed and
encrypted with the application payload.

Serialization identifiers:

| ID | Name | Payload |
|---:|---|---|
| `0` | Raw bytes | Exact remaining bytes |
| `1` | Canonical JSON | RFC 8785 UTF-8 JSON |
| `2..2^128-1` | Reserved for future core codecs | Not yet valid |

An unknown identifier MUST produce an unsupported-serialization error. A language adapter MUST NOT
assign an identifier or implement a serializer itself.

### Raw bytes

Raw serialization has this form:

```text
00 | exact application bytes
```

An empty application byte string is valid and serializes as the single byte `00`. The formatted raw
API still adds the value-format container. Only the exact-item-ID, exact-value protocol client
bypasses the format entirely.

### Canonical JSON

JSON serialization follows the
[JSON Canonicalization Scheme](https://www.rfc-editor.org/rfc/rfc8785) in RFC 8785. The core handles
both encoding and decoding. Language adapters convert native values into the core's logical value
model and MUST NOT call facilities such as `JSON.stringify` to produce stored bytes.

The common logical JSON model contains:

- null;
- booleans;
- finite IEEE-754 binary64 numbers;
- Unicode strings;
- dense arrays;
- objects with unique string keys.

The following values are invalid:

- NaN or positive or negative infinity;
- sparse arrays;
- cyclic containers;
- duplicate object keys;
- lone Unicode surrogate code points;
- functions, symbols, language-specific objects, or implicit `undefined` values.

The core MUST preserve valid Unicode string data without normalization. It MUST sort object
property names lexicographically by unsigned UTF-16 code units, as required by RFC 8785, rather
than by UTF-8 bytes, Unicode scalar values, locale, or insertion order.

The decoder MUST reject a JSON payload that is valid JSON but is not the exact RFC 8785
serialization of its decoded logical value. This includes payloads with insignificant whitespace,
non-canonical string escapes, non-canonical number spelling, or incorrectly ordered object
properties. Parsing without canonicality validation is insufficient.

Integers outside the interoperable binary64 range `-(2^53 - 1)..=(2^53 - 1)` SHOULD be represented
by an application-defined decimal string contract. Binary fields SHOULD use the raw serialization
or an application-defined textual representation.

Canonical JSON permits any JSON value at the top level. A higher-level API MAY restrict its input
to objects, but that restriction is not part of the stored format.

### Future typed codecs

Protobuf, FlatBuffers, and other schema-aware encodings require separate core-owned codec
specifications before receiving standard serialization identifiers. Those specifications must
define schema registration, type identity, canonical payload production, and native-to-core value
conversion. Passing language-serialized bytes through an adapter and labeling them as a standard
codec is not permitted.

Applications may store independently serialized data as raw bytes, but the format does not claim
cross-language codec compatibility for such data.

## Compression

Compression ID `1` means one complete Zstandard frame as specified by
[RFC 8878](https://www.rfc-editor.org/rfc/rfc8878):

```text
compressed_body = zstd(serialized_value)
```

The encoder:

- MUST compress the complete serialized value, including its serialization identifier;
- MUST emit exactly one standard Zstandard frame and MUST NOT emit a skippable frame;
- MUST NOT use an external dictionary;
- MUST include the frame content size;
- MUST NOT append another standard or skippable frame or any trailing bytes;
- SHOULD omit the optional Zstandard content checksum to minimize overhead;
- MUST set the compression identifier only when it stores an actual Zstandard frame.

The decoder:

- MUST reject a skippable frame;
- MUST reject a frame without a declared content size;
- MUST reject a dictionary requirement;
- MUST reject multiple frames or trailing bytes;
- MUST reject a requested window size above the decoded-value limit;
- MUST reject a declared or produced size above the decoded-value limit;
- MUST verify that the produced byte count equals the declared content size.

Compression level and the decision to compress are encoder policy rather than wire semantics.
The initial recommended policy is Zstandard level `1`, no compression below `1,024` serialized
bytes, and a compressed-frame length at least `64` bytes smaller than the serialized value. The
encoder stores the uncompressed representation when compression does not satisfy its policy.

Encryption does not hide the stored length. An application that combines secrets with
attacker-controlled data and lets the attacker observe value lengths across chosen writes SHOULD
disable compression or separate those inputs to avoid a compression length oracle.

## Data protection key

Encrypted formats require an application-managed 32-byte `DataProtectionKey` with 256 bits of
cryptographic randomness. A password, padded string, truncated string, or unhashed passphrase is
not a valid key.

The key SHOULD be unique to one application security domain. Clients that intentionally share
entries use the same key, but unrelated applications or environments SHOULD use different keys.
Reusing a key across trust domains permits same-item ciphertext replay between those domains.

The data protection key never leaves the client. Encrypted values use per-item encryption keys so
that nonce reuse and deterministic equality are scoped to one exact 32-byte item ID.

All subkeys use the BLAKE3
[derive-key mode](https://github.com/BLAKE3-team/BLAKE3-specs/blob/master/blake3.pdf). Context
strings below are exact, case-sensitive UTF-8 bytes. Implementations MUST NOT add terminators,
lengths, or implementation-specific prefixes. Every `BLAKE3-DERIVE-KEY` and
`BLAKE3-KEYED-HASH` operation below returns the first 32 bytes of BLAKE3's extendable output.

### Protected item ID

Formatted protected clients derive the exact 32-byte wire item ID in the core:

```text
item_id_root =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache client item key root v1",
    material = data_protection_key[32]
  )

item_id =
  BLAKE3-KEYED-HASH(
    key   = item_id_root[32],
    input = application_key
  )
```

`application_key` is one exact byte string supplied to the core. A language adapter that accepts a
native text key MUST reject ill-formed Unicode and encode the scalar sequence as UTF-8 without
normalization. It MUST NOT include a terminator or length prefix. A byte-oriented key API passes
its bytes unchanged.

The keyed hash accepts the exact application-key bytes without a length prefix because it receives
one complete input byte string. Enabling, disabling, or rotating the data protection key changes
the item ID and makes entries written under the previous setting unreachable without an explicit
migration strategy.

The low-level raw client accepts an exact caller-supplied item ID and does not perform this
derivation. An unprotected formatted primitive likewise accepts an exact 32-byte wire item ID; it
MUST NOT invent a separate unkeyed mapping from arbitrary application keys.

### Value root key

First derive a value root:

```text
value_root_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 root key",
    material = data_protection_key[32]
  )
```

Per-item derivations use this fixed 64-byte material:

```text
item_id_material = value_root_key[32] | item_id[32]
```

The item ID is the exact wire item ID, not the original application key. Because both components
have fixed lengths, their concatenation is unambiguous.

Derived secrets MUST be zeroized when their owning object is destroyed or the operation completes.
Implementations SHOULD derive per-item encryption keys on demand rather than retaining a
high-cardinality key cache.

## Associated data

Both encryption profiles authenticate the same canonical associated data:

```text
aad =
  ascii("openkache/value-format/aad/v1")
  | item_id[32]
  | encoded_version
  | format
```

`encoded_version` is the exact canonical VU128 byte sequence stored in the container. For version
`1`, it is `01`. `format` is the exact one-byte field stored after the version.

The domain string is not stored and consumes no capacity. Authenticating the item ID prevents a
server or intermediary from moving a valid ciphertext to another cache item. Authenticating the
header prevents changing the version or transform algorithms without detection.

The associated data does not provide freshness. A server can replay an older valid container for
the same item ID. Applications that require rollback detection must serialize and validate their
own monotonic version or other freshness data.

## Compact encryption

Encryption ID `1` is deterministic AES-256-SIV-CMAC as specified by
[RFC 5297](https://www.rfc-editor.org/rfc/rfc5297). It uses one 256-bit AES-CMAC key and one
256-bit AES-CTR key, equivalent to the RFC's 64-byte AES-SIV-CMAC-512 combined-key variant.

Derive the keys independently:

```text
compact_mac_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-SIV-CMAC MAC key",
    material = item_id_material[64]
  )

compact_encryption_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-SIV-CMAC encryption key",
    material = item_id_material[64]
  )
```

The RFC 5297 combined key is:

```text
compact_key = compact_mac_key[32] | compact_encryption_key[32]
```

Pass the complete canonical `aad` byte string as exactly one RFC 5297 associated-data component.
Do not split its domain, item ID, version, or format fields into separate S2V components, because
component boundaries affect the synthetic IV. Do not supply an optional nonce component. An empty,
fixed, or all-zero nonce component is not equivalent to omitting that component; implementations
MUST NOT use a nonce-requiring AEAD wrapper that cannot express RFC 5297 deterministic mode.

Encrypt the serialized or compressed bytes and store:

```text
compact_body = synthetic_iv[16] | ciphertext

container =
  version:vu128
  | format:u8
  | synthetic_iv[16]
  | ciphertext
```

There is no random nonce. The synthetic IV is also the 16-byte authentication tag. For version
`1`, the complete fixed container overhead is exactly 18 bytes: one version byte, one format byte,
and the synthetic IV. The encrypted serialization identifier remains part of the ciphertext.

Compact encryption is deterministic. Under one per-item encryption key, identical transformed
plaintext and associated data produce identical stored bytes. Because every item has a different
derived key,
the server cannot compare value equality across different item IDs, but it can observe whether
the complete value of one item repeats across writes.

Compact encryption is appropriate only when that per-item equality leakage is acceptable.

## Robust encryption

Encryption ID `2` is AES-256-GCM-SIV as specified by
[RFC 8452](https://www.rfc-editor.org/rfc/rfc8452).

Derive its per-item encryption key:

```text
robust_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-GCM-SIV key",
    material = item_id_material[64]
  )
```

Generate a fresh 12-byte nonce from the operating system cryptographic random source for every
write, including repeated writes of the same value to the same item. The write MUST fail if the
random source fails; an implementation MUST NOT substitute a deterministic fallback.

Store the RFC 8452 ciphertext and tag as:

```text
robust_body = nonce[12] | ciphertext | tag[16]

container =
  version:vu128
  | format:u8
  | nonce[12]
  | ciphertext
  | tag[16]
```

For version `1`, the complete fixed container overhead is exactly 30 bytes: one version byte, one
format byte, a 12-byte nonce, and a 16-byte tag.

The nonce is public and MUST NOT be derived from only the item ID, a timestamp, plaintext, or a
process-local counter. Per-item ID derivation limits the random-nonce collision domain to repeated
writes of one item. AES-GCM-SIV additionally prevents an accidental nonce repeat from causing the
catastrophic failure associated with AES-GCM or ChaCha20-Poly1305. A repeated nonce under one key
reveals whether the corresponding plaintext and associated data also repeat, and repetition
worsens the scheme's security bounds. Implementations MUST NOT reuse nonces intentionally.

Robust encryption is the recommended default because repeated writes of the same value produce
independent stored representations.

## Unencrypted values

Encryption ID `0` stores the serialized or compressed bytes directly:

```text
container = version:vu128 | format:u8 | transformed_body
```

The format provides no value authentication in this mode. Parser bounds and Zstandard validation
remain mandatory, but they are not a substitute for cryptographic integrity.

The format supports unencrypted values so raw and explicitly unprotected APIs can share the same
serialization and compression contract. A client configured to require value protection MUST
reject encryption ID `0`. Accepting unencrypted formatted values requires an explicit unprotected
configuration; a decoder MUST NOT treat a missing encryption profile as an automatic fallback.

## Size limits

Two independent limits apply:

1. The complete stored container MUST NOT exceed the protocol maximum value size of 64 MiB.
2. The decoded serialized value MUST NOT exceed 64 MiB.

The first limit includes the version, format byte, nonce or synthetic IV, ciphertext, and tag. The
second includes the serialization identifier and serialization payload.

An implementation MAY enforce lower configured container and decoded-value limits. It MUST apply
each limit consistently to encoding and decoding, including declared Zstandard content size,
requested Zstandard window size, and produced output.

An implementation MUST use checked arithmetic for every size calculation. It MUST authenticate an
encrypted body before trusting or allocating based on compressed or serialized contents. A
declared Zstandard content size or requested window size above the decoded limit MUST be rejected
before decompression, and decompression MUST remain bounded by that limit even if the frame is
malformed.

## Encoding procedure

An encoder implements these steps:

1. Convert the native language value to a core-owned logical value.
2. Serialize it in the core and prefix the canonical serialization identifier.
3. Reject the value if the complete serialized representation exceeds the decoded-value limit.
4. Apply Zstandard only when configured and beneficial; update the compression nibble to match the
   bytes actually selected.
5. Encode version `1` canonically and construct the format byte.
6. Construct the canonical associated data from the exact item ID and header bytes.
7. Apply the selected encryption profile:
   - none: copy the transformed body;
   - Compact: derive both per-item AES-SIV keys and prepend the 16-byte synthetic IV;
   - Robust: derive the per-item AES-GCM-SIV key, generate a 12-byte nonce, and append the 16-byte
     tag.
8. Reject the result if the complete container exceeds the protocol value limit.
9. Send the complete container as the raw protocol operation's opaque value bytes.

The encoder MUST NOT emit a length field, magic bytes, reserved algorithm identifier, or
non-canonical VU128.

## Decoding procedure

A decoder implements these steps in order:

1. Parse one canonical unsigned VU128 from the start of the exact container slice.
2. Reject a version other than `1`.
3. Read one format byte and reject unassigned compression or encryption identifiers.
4. Reject an encryption identifier that violates the client's configured protection policy.
5. Construct the canonical associated data from the item ID and exact header bytes.
6. Validate the minimum encrypted body size before slicing:
   - Compact requires at least a 16-byte synthetic IV and one encrypted serialization-ID byte;
   - Robust requires a 12-byte nonce, a 16-byte tag, and one encrypted serialization-ID byte.
7. Derive the selected per-item encryption key or keys.
8. Authenticate and decrypt before decompression or serialization parsing.
9. If compressed, validate one bounded Zstandard frame and decompress it.
10. Parse one canonical serialization identifier from the transformed plaintext.
11. Dispatch to the selected core codec and reject an unsupported identifier or invalid payload.
12. Convert the core logical value to the language adapter's native result.

Authentication failure MUST NOT reveal whether the key, nonce, header, tag, ciphertext, or
compressed contents were responsible. The public error MUST identify only a value-authentication
failure. Implementations MUST zeroize any unauthenticated plaintext produced internally before
returning that error.

## Protocol integration

The [wire protocol specification](../protocol/SPEC.md) defines value limits,
server opacity, and `SET` flags. This format begins with the exact opaque value
slice carried by a protocol request or response; it does not add protocol
fields.

The low-level raw API sends exact item IDs and value bytes without this
container. It is an explicit escape hatch and does not claim compatibility with
formatted APIs.

## Core and language responsibilities

`openkache-client-core` handles:

- value-format parsing and production;
- canonical VU128 handling;
- the serialization registry and all serializers;
- Zstandard policy, encoding, and decoding;
- BLAKE3 key derivation and secret zeroization;
- Compact and Robust authenticated encryption;
- format validation and stable error categories;
- the protected application-key and plaintext-value operations used by bindings.

Language adapters handle only:

- conversion between native values and core logical values;
- transfer of raw byte buffers without reformatting them;
- runtime-appropriate asynchronous APIs;
- native configuration and error wrappers;
- deterministic resource cleanup.

Language adapters MUST NOT duplicate VU128, JSON serialization, compression, key derivation,
encryption, nonce generation, or container parsing. A future browser implementation must reuse the
same core through WebAssembly or another shared-core boundary rather than reimplementing the
format in JavaScript.

## Versioning

There is no compatibility requirement for pre-release envelope or flag-based value
representations. Writers emit version `1`; readers accept version `1` only.

The earlier magic-prefixed envelope beginning with `4f 4b 56 01` is not version `1` of this format
and MUST be rejected rather than unwrapped or migrated implicitly.

A future version is required when changing:

- header field order or meaning;
- an assigned algorithm identifier or its byte-level semantics;
- associated-data construction;
- KDF context strings or input material;
- an assigned serializer's byte contract;
- encryption body layout.

Assigning a reserved compression, encryption, or serialization identifier with a complete
specification does not require a container version change. Decoders still reject the identifier
until they implement that specification.

Unknown versions MUST fail explicitly. A decoder MUST NOT search for a magic prefix, guess that an
unknown value is plaintext, or fall back to an earlier format.
