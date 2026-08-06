# OpenKache client-owned value format

This document is the v1 binary-layout specification for values produced by the
formatted client APIs. The server stores the resulting byte string opaquely;
it never parses serialization, compression, encryption, or client metadata.

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

## Design decisions

The v1 design fixes the following choices:

- `namespace_id` is bound independently into both item-ID derivation and value
  AAD.
- The 32-byte root secret is named `client_root_key`.
- Raw byte keys and structured keys have one exact binary input contract.
- Values provide Raw bytes and deterministic CBOR. There is no separate
  client-only metadata envelope.
- Compression and encryption are independent options. The wire format supports
  no compression, Zstandard, no protection, Compact protection, and Robust
  protection.
- Protocol and value-format integers use the same canonical unsigned 64-bit
  `vu128` encoding. The value format does not define a second 128-bit variant.
- Replay/freshness protection is intentionally not frozen in v1.

## Processing model

A formatted write performs these stages:

```text
native value
  -> core logical value
  -> selected value codec
  -> optional Zstandard compression
  -> optional authenticated encryption
  -> version-1 container
  -> opaque server value
```

A read reverses the stages. Compression always precedes encryption. The
serialization discriminator and serialized payload are inside the transformed
body, so encryption hides the value codec choice.

The format has no magic bytes and no body-length field. The surrounding
protocol frame supplies the exact value boundary. `value_len` in the `SET`
frame is the canonical `vu128` length of the complete container.

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

The maximum encoded width is therefore 9 bytes. Version and codec identifiers
currently encode in one byte, but using the shared integer contract keeps
future assignments unambiguous.

## Container layout

```text
container = version:vu128 | format:u8 | body
```

| Field | Size | Meaning |
|---|---:|---|
| `version` | 1–9 bytes | Canonical value-format version; v1 is `01`. |
| `format` | 1 byte | Compression and encryption identifiers. |
| `body` | remaining bytes | Serialized value, optionally compressed and encrypted. |

The format byte is split into two nibbles:

```text
bits 0..3 = compression_id
bits 4..7 = encryption_id
format = compression_id | (encryption_id << 4)
```

### Compression identifiers

| ID | Name |
|---:|---|
| `0` | None |
| `1` | Zstandard |
| `2..15` | Reserved |

### Encryption identifiers

| ID | Name | Profile |
|---:|---|---|
| `0` | None | Unprotected |
| `1` | AES-256-SIV-CMAC | Compact |
| `2` | AES-256-GCM-SIV | Robust |
| `3..15` | Reserved |

A decoder MUST reject every unassigned identifier. It MUST NOT guess an
algorithm or try several interpretations.

Common format bytes are:

| Byte | Meaning |
|---:|---|
| `00` | Raw or CBOR value, uncompressed and unprotected |
| `01` | Raw or CBOR value, Zstandard-compressed and unprotected |
| `10` | Raw or CBOR value, Compact-protected |
| `11` | Raw or CBOR value, Zstandard-compressed and Compact-protected |
| `20` | Raw or CBOR value, Robust-protected |
| `21` | Raw or CBOR value, Zstandard-compressed and Robust-protected |

The serialization identifier is in the transformed body and is not encoded in
the format byte. This keeps compression and encryption assignments independent
of the codec registry.

## Serialized value

Before compression or encryption, the body is:

```text
serialized_value = serialization_id:vu128 | serialization_payload
```

Serialization identifiers:

| ID | Name | Payload |
|---:|---|---|
| `0` | Raw bytes | Exact remaining application bytes. |
| `1` | Deterministic CBOR | Core logical value encoded by the v1 CBOR contract. |
| `2..2^64-1` | Reserved | Invalid until specified and assigned. |

An unknown identifier is an unsupported-serialization error. Language adapters
MUST NOT assign identifiers or write a serializer that is not part of the core
contract.

### Raw bytes

```text
raw_serialized_value = 00 | exact_application_bytes
```

An empty application byte string is valid and is encoded as the single byte
`00` before the outer container is added. Embedded `00` octets are ordinary
payload bytes. There is no NUL terminator, text sentinel, or implicit
application-value length inside Raw.

Consequently, an empty raw value occupies 3 bytes in the uncompressed,
unprotected profile (`version=01 | format=00 | serialization_id=00`).

Raw is also the escape hatch for application-defined binary data. If an
application wants to put client-only metadata next to its value, it can define
that metadata inside its own Raw payload. v1 does not add a second envelope,
flag, or length field for such data.

### Deterministic CBOR

Deterministic CBOR is the structured-value codec for v1. It MUST use the
deterministic encoding rules from
[RFC 8949 §4.2](https://www.rfc-editor.org/rfc/rfc8949#section-4.2):

- definite lengths only;
- shortest integer and length arguments;
- map keys sorted by the encoded-key byte strings;
- no floating-point NaN or infinity;
- no tags, indefinite-length items, duplicate map keys, or trailing bytes;
- UTF-8 text strings without Unicode normalization.

The v1 logical model is null, boolean, finite number, UTF-8 string, array, and
map with unique string keys. A language adapter converts its native object to
this model; it MUST NOT call a language JSON/CBOR serializer and pass the
resulting bytes through as if they were canonical.

Numbers use finite IEEE-754 binary64 values. Deterministic CBOR uses the
shortest representation that preserves the value; an exactly representable
integer MAY use the corresponding integer representation, while a
non-integral value uses the shortest preserving floating-point width.

The core decoder MUST re-encode the decoded logical value and reject the input
when the bytes are not the exact deterministic representation. A valid but
non-deterministic CBOR encoding is not accepted.

Protobuf, FlatBuffers, MessagePack, and other schema or language-specific
formats are not v1 codecs. They MAY be stored as Raw bytes under an
application-owned contract.

## Key input contract

The item-ID hash input is a typed, canonical byte string:

```text
key_input = key_codec_id:vu128 | key_payload
```

The codec identifier is part of the hash input. It prevents a byte key from
colliding with a structured key that happens to have the same payload bytes.

| ID | Key kind | `key_payload` |
|---:|---|---|
| `0` | Raw key | Exact application bytes. |
| `1` | Structured key | Deterministic-CBOR payload produced from the core logical model. |
| `2..2^64-1` | Reserved | Invalid until specified and assigned. |

Rules for language-facing inputs:

- A byte-oriented key API copies the supplied bytes exactly. Empty bytes and
  embedded NUL bytes are valid; neither has a special meaning at the binary
  layer. An empty Raw key is therefore the one-byte input `00` (the Raw codec
  identifier).
- A text key API rejects ill-formed Unicode and uses the UTF-8 bytes of the
  scalar sequence as a Raw key payload. It MUST NOT normalize, append a NUL,
  append a length, or use locale encoding.
- An object key follows the same native-object-to-core-logical-value conversion
  as a structured value, then uses deterministic CBOR with key codec ID `1`.
- Key bytes are not compressed or encrypted. Their only purpose is deterministic
  item identity.

This contract is language-neutral. It intentionally specifies the bytes the
core receives after native conversion, not a particular language's object
reflection rules.

There is no v1 wire maximum for key length: the key is hashed and is never
stored in a frame. BLAKE3 accepts an arbitrary byte string. An implementation
MAY impose a documented, checked resource limit to protect memory and CPU, but
such a limit is an API/resource policy rather than a binary-layout rule. The
64 MiB value limit does not silently become a key limit.

## Client root key and item ID derivation

`client_root_key` is exactly 32 bytes with 256 bits of cryptographic
randomness. A password, human-readable string, padded string, or truncated
secret is not a valid root key. The key remains inside the client.

The root key SHOULD be unique to one application security domain. Sharing
entries intentionally requires sharing the same root key and namespace
identity. Unrelated applications or environments SHOULD use different keys.

All context strings below are exact, case-sensitive UTF-8 bytes. They have no
terminator, implicit length, or implementation-specific prefix.

```text
item_id_root =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache item ID derivation root v1",
    material = client_root_key[32]
  )
```

The namespace identity is fixed-width big-endian:

```text
namespace_id_bytes = namespace_id:u64be
```

`namespace_id` MUST be a positive server-assigned namespace ID. The wire
protocol's zero value is not a selector and MUST NOT be used by a formatted
client.

The exact item-ID hash input is:

```text
item_id_input = namespace_id_bytes | key_input

item_id =
  BLAKE3-KEYED-HASH(
    key   = item_id_root[32],
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

## Value encryption key derivation

Encrypted values derive a value root from the same client root key:

```text
value_root_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 root key",
    material = client_root_key[32]
  )

item_id_material = value_root_key[32] | item_id[32]
```

The two components are fixed-width and therefore unambiguous. Derived keys
MUST be zeroized when their operation or owning object ends. Implementations
SHOULD derive per-item keys on demand instead of retaining a high-cardinality
key cache.

## Associated data

Both protected profiles authenticate the same bytes:

```text
aad =
  ascii("openkache/value-format/aad/v1")
  | namespace_id_bytes
  | item_id[32]
  | encoded_version
  | format
```

`encoded_version` is the exact canonical `vu128` byte sequence stored in the
container. For v1 it is `01`. `format` is the exact byte stored after the
version.

The namespace is included separately in both `item_id_input` and `aad`.
The complete AAD is not fed into item-ID derivation; doing that would create a
circular dependency because AAD contains the item ID.

Authenticating the namespace and Item ID prevents moving a valid ciphertext to
another `(namespace_id, item_id)` pair. Authenticating the header prevents
changing the version or transform algorithms. AAD does not provide freshness:
an older valid container can still be replayed to the same item.

## Encryption profiles

### Unprotected (ID 0)

The transformed serialized body is stored directly:

```text
container = version:vu128 | format:u8 | transformed_body
```

This is the minimum-size option. It provides no confidentiality or value
authentication. Parser bounds and Zstandard validation are still mandatory.
Clients that require protection MUST reject this profile; accepting it must be
an explicit configuration choice.

### Compact (ID 1)

Compact uses deterministic AES-256-SIV-CMAC as specified by
[RFC 5297](https://www.rfc-editor.org/rfc/rfc5297). Derive independent 32-byte
keys:

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

compact_key = compact_mac_key[32] | compact_encryption_key[32]
```

Pass the complete AAD as exactly one RFC 5297 associated-data component. Do
not split its fields into several S2V components and do not provide an
optional nonce component.

```text
compact_body = synthetic_iv[16] | ciphertext
```

Compact has 16 bytes of cryptographic overhead and no random nonce. Identical
transformed plaintext for one item produces identical stored bytes, so the
server can observe repetition of that item's complete value. It cannot compare
the same plaintext across different item IDs because their keys differ.

### Robust (ID 2)

Robust uses AES-256-GCM-SIV as specified by
[RFC 8452](https://www.rfc-editor.org/rfc/rfc8452):

```text
robust_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-GCM-SIV key",
    material = item_id_material[64]
  )

robust_body = nonce[12] | ciphertext | tag[16]
```

The encoder MUST obtain a fresh 12-byte nonce from the operating-system
cryptographic random source for every write, including repeated writes of the
same value. It MUST fail if the random source fails and MUST NOT use a
timestamp, item ID, plaintext, or process-local counter as a fallback.

Robust has 28 bytes of cryptographic overhead. It is the recommended protected
profile because repeated writes have independent representations and therefore
do not expose Compact's deterministic repetition. A nonce remains public.

### Profile size comparison

For v1, the fixed overhead before the serialized payload is:

| Profile | Container/crypto overhead | Empty Raw value |
|---|---:|---:|
| Unprotected | 2 bytes | 3 bytes |
| Compact | 18 bytes | 19 bytes |
| Robust | 30 bytes | 31 bytes |

Robust is exactly 12 bytes larger than Compact. Compression, when selected,
adds the Zstandard frame overhead and can make a small value larger.

The wire format does not choose a default profile. The SDK policy default is
still a product decision; a protected SDK SHOULD choose Robust, while a caller
optimizing for minimum size MAY choose Compact when repetition leakage is
acceptable. Unprotected MUST remain an explicit opt-in.

## Compression

Compression ID `1` is one complete Zstandard frame as specified by
[RFC 8878](https://www.rfc-editor.org/rfc/rfc8878):

```text
compressed_body = zstd(serialized_value)
```

The encoder MUST:

- compress the complete serialized value, including its codec identifier;
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
below 1,024 serialized bytes, and a minimum saving of 64 bytes.

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

1. Convert a native value to the core logical model.
2. Prefix the selected canonical codec ID to produce `serialized_value`.
3. Reject a serialized value above the decoded-value limit.
4. Apply Zstandard only when configured and beneficial.
5. Encode version `1` canonically and construct the format byte.
6. Construct AAD from the namespace ID, exact Item ID, and exact header bytes.
7. Apply the selected encryption profile.
8. Reject a complete container above the protocol value limit.
9. Send the complete container as the opaque protocol value.

The encoder MUST NOT emit a magic prefix, body-length field, reserved
identifier, non-canonical `vu128`, or an encryption profile inconsistent with
the format byte.

## Decoding procedure

A decoder MUST:

1. Parse one canonical `version:vu128` from the exact container slice.
2. Require version `1`.
3. Read one format byte and reject reserved compression/encryption IDs.
4. Enforce the configured encryption policy.
5. Construct AAD from the supplied namespace ID, exact Item ID, and exact header.
6. Check profile-specific minimum body sizes before slicing.
7. Derive the selected per-item key(s).
8. Authenticate and decrypt before decompression or codec parsing.
9. If compressed, validate and decompress one bounded Zstandard frame.
10. Parse one canonical serialization ID and dispatch the core codec.
11. Reject unsupported, non-deterministic, truncated, trailing, or oversized
    payloads.

Authentication errors MUST be reported as one generic value-authentication
failure. Implementations MUST zeroize unauthenticated plaintext before
returning that error.

## Limits

Two independent v1 limits apply:

1. The complete stored container MUST be at most 64 MiB, including headers,
   compression framing, nonce, and authentication tag.
2. The decoded `serialized_value` MUST be at most 64 MiB, including its codec
   identifier.

Implementations MAY configure lower limits, but MUST apply them consistently
to encoding and decoding. Declared Zstandard content size, requested window
size, produced output, and all size arithmetic MUST be checked before
allocation.

## Replay and freshness (P1, intentionally open)

V1 encryption authenticates identity and format, not time or write order. A
server can replay an older valid value to the same namespace and Item ID.
Robust's random nonce changes the bytes; it does not stop rollback.

The current recommendation is to leave replay/freshness out of the frozen
binary layout until the server CAS contract is settled. Two future designs are
possible:

1. **Application generation**: the application stores a monotonic generation
   or version inside Raw/CBOR and rejects values older than its local
   expectation. This requires no wire change but does not give the cache a
   server-enforced CAS operation.
2. **Protocol generation/CAS**: the wire protocol carries an expected or
   returned generation and the server atomically checks it. The generation
   would then be authenticated as part of the value context or protocol
   operation.

“Generation/version + CAS as a separate security profile” means the second
design: an opt-in profile would add a generation field and require an atomic
compare-and-set contract. It is not a third encryption algorithm, and it is
not part of v1 yet. Do not infer freshness from an Item ID, Compact synthetic
IV, Robust nonce, TTL, or namespace revision.

## Protocol integration and responsibilities

The wire protocol owns frame lengths, namespaces, Item IDs, and server
semantics. This document starts at the exact opaque `value` slice in a request
or response and adds no protocol fields.

The shared core owns:

- canonical `vu128` parsing for this format;
- the Raw and deterministic-CBOR codec registry;
- key-input canonicalization and namespace-bound Item-ID derivation;
- Zstandard policy and bounded decompression;
- BLAKE3 derivation and secret zeroization;
- Compact and Robust encryption;
- stable validation and authentication errors.

Language adapters own only conversion between native values and the core
logical model, byte-buffer ownership, asynchronous/runtime APIs, and error
wrappers. They MUST NOT duplicate framing, codec canonicalization,
compression, key derivation, encryption, nonce generation, or container parsing.

## Versioning

Writers emit version `1`; v1 readers accept version `1` only. A future version
is required for changes to:

- header order or field meaning;
- assigned algorithm IDs or body layouts;
- AAD construction;
- BLAKE3 context strings or derivation material;
- the key-input contract;
- an assigned codec's byte contract;
- canonical integer encoding.

Reserved codec, compression, and encryption IDs MAY receive a complete
specification without changing the container version, but readers MUST reject
them until implemented.

The previous magic-prefixed envelope beginning with `4F 4B 56 01` is not v1
and MUST NOT be implicitly unwrapped or migrated. Package-local compatibility
may put that legacy payload inside Raw, but it is not a cross-language codec.

## Implementation migration checklist

The checked-in implementation is intentionally ahead of this design only in
the old direction. Before declaring v1 frozen, the following production
surfaces must consume this document as their source of truth:

- the Smithy client model and generated value-format constants;
- the shared key module (`ClientRootKey`, typed key input, and namespace-bound
  Item-ID derivation);
- the shared value module (deterministic CBOR, one protocol `vu128`, and
  namespace-bound AAD);
- protected clients and FFI entry points, which must resolve the namespace
  before deriving an Item ID;
- language adapters and package documentation.

Until those changes land, old JSON containers and old derivation outputs are
implementation artifacts, not v1 wire compatibility.
