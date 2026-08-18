# OpenKache Client Value Encoding Profile v1 (Draft)

> **Status: Draft — pre-freeze**

This document defines the client-side v1 value encoding before it is handed to
the server. The server stores the resulting bytes opaquely and does not
interpret payload formats, compression, or cryptographic protection.

This is the target contract for the pre-freeze draft. Client implementations
may temporarily lag while the draft is being completed, but an implementation
MUST NOT claim conformance to this profile until it implements the complete
grammar, key schedule, and validation rules.

Shared API-family, request lifecycle, and retry behavior is specified by the
[Client Behavioral Contract](CLIENT.md). Binding-specific method names and
defaults belong in each binding's documentation.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

## 1. Scope and goals

This profile is designed to provide:

- predictable decoding across language implementations;
- explicit, bounded parsing and rejection of malformed input;
- exact preservation of opaque application bytes;
- a compact self-describing header for the supported transforms; and
- authenticated protection when an enclosing client profile supplies a key.

This profile does not define:

- application-key conversion or Item ID derivation;
- server freshness or CAS;
- language-native type conversion;
- application registries for arbitrary payload formats; or
- transport framing.

The enclosing client profile supplies a value keyring and the exact namespace
and Item ID. This profile defines value-key selection and derivation, the value
envelope, its authenticated binding to the key ID, namespace, and exact Item
ID, and the transforms applied to its payload. Item ID derivation uses an
independent identity key defined by the client key contract.

An exact Item ID API is a separate client operation: it accepts an already
resolved `0..=32`-byte Item ID and an opaque value, and does not apply this
value envelope. Formatted-key APIs use this profile after the client has
resolved the Item ID. A server never interprets either representation.

## 2. Processing model

```text
source value
  -> selected payload format
  -> payload bytes
  -> optional compression
  -> optional cryptographic protection
  -> ValueEnvelope
```

`OpaqueBytes` treats the source as an exact byte string: before optional
compression and protection, the payload bytes are identical to the supplied
bytes. `CBOR` accepts one CBOR data item under the rules in §5. `Json` is an
API convenience type, not a v1 payload selector: it is serialized as canonical
RFC 8785 UTF-8 and carried using `OpaqueBytes` (selector `0`). The payload
format is selected by the caller; the encoding does not infer a format from the
payload bytes.

## 3. Envelope grammar

```text
ValueEnvelope = value_envelope_version:vu128 | version_body
version_body  = selector_byte:u8 | selected_body
```

The profile uses `value_envelope_version = 1`:

```text
value_envelope_v1 =
    value_envelope_version:vu128(1)
  | selector_byte:u8
  | selected_body

selected_body =
    transformed_body                                      # Unprotected
  | value_key_id:vu128 | protected_body                   # protected
```

The `|` operator denotes byte concatenation. The `value_envelope_version`
selects the complete grammar for the remaining bytes.

`vu128` is the protocol's canonical unsigned 64-bit variable-length integer
encoding defined in
[Wire Protocol](../protocol/SPEC.md#unsigned-vu128). It is at most nine bytes
wide. The version field MUST use its shortest canonical encoding.

The enclosing value boundary supplies the complete envelope length. This
profile has no magic prefix and no body-length field. An encoder MUST emit one
complete envelope; a decoder MUST consume the complete supplied envelope and
reject truncation, overflow, overlong `vu128` encodings, and trailing bytes.

The `protection_id` in `selector_byte` selects the `selected_body` grammar.
`Unprotected` has no `value_key_id`; all bytes after the selector are its
transformed body. Every protected body begins with one canonical
`value_key_id:vu128` in the numeric range `1..=2^64 - 1`, followed by the
selected protection profile's body. Zero is reserved and MUST be rejected.
The key ID is public selection metadata, not secret key material. Its canonical
encoding is at most nine bytes.

`value_envelope_version = 0` is an application-defined envelope profile.
OpenKache does not define or interpret its body grammar, selectors, transform
order, authentication inputs, or limits. Communicating parties MAY use it only
when they configure the complete profile out of band. A reader without the
matching profile MUST reject it and MUST NOT parse it using this profile.

Unsupported versions MUST be rejected. A decoder MUST select the version
grammar before interpreting any version-body byte.

## 4. Selector byte

```text
bits 0..1 = protection_id
bits 2..3 = compression_id
bits 4..5 = payload_format_id
bits 6..7 = zero

selector_byte =
    protection_id
  | (compression_id << 2)
  | (payload_format_id << 4)
```

Encoders MUST emit only supported selector values and zero bits 6..7. Decoders
MUST reject any unsupported selector value or nonzero bit 6 or 7. They MUST
NOT guess an unknown selector.

### 4.1 Protection profiles

`protection_id` selects the complete cryptographic protection behavior. The
name, nonce policy, authentication behavior, and repetition behavior are
normative properties of the selected profile.

| ID | Protection profile | Behavior |
|---:|---|---|
| `0` | `Unprotected` | Store the transformed body directly; provide no confidentiality or authentication. |
| `1` | `AES-256-GCM-SIV` ([RFC 8452](https://www.rfc-editor.org/rfc/rfc8452)) | Randomized authenticated encryption with a fresh 12-byte nonce per write. |
| `2` | `AES-256-SIV-CMAC` ([RFC 5297](https://www.rfc-editor.org/rfc/rfc5297)) | Deterministic authenticated encryption with no random nonce. |

Encoding and decoding use separate protection policies. The write profile
selects the protection applied to a new value. Each formatted write MAY
override the instance write default; an override MUST NOT mutate that default.
The selected write profile MUST be represented by `protection_id`.

With an empty value keyring, the default and only valid write profile is
`Unprotected`. With a configured `active_value_key_id`, the default write
profile is `AES-256-GCM-SIV`; the active ID MUST resolve to a keyring entry at
configuration time. A nonempty read keyring MAY omit an active ID, but then an
omitted write profile MUST fail instead of selecting `Unprotected`. Such a
client can read protected values without being able to emit one accidentally.

The client MAY explicitly select an authenticated profile where its connection
API exposes that choice. Selecting one without an active ID and its associated
key MUST fail; a binding MUST preserve that explicit selection so the shared
core can reject it rather than silently downgrading to `Unprotected`.

An explicit `Unprotected` connection or operation profile MAY be used while a
value keyring is configured. Item ID derivation is independent, so enabling or
disabling value protection MUST NOT change the Item ID. A protected write MUST
use `active_value_key_id`; it MUST NOT select an inactive read key through a
per-operation protection override.

The read policy is an allowlist, not a second write default:

- without any configured value keys, it contains only `Unprotected`;
- with a nonempty value keyring, its default contains `AES-256-GCM-SIV` and
  `AES-256-SIV-CMAC`, but not `Unprotected`;
- a caller MAY explicitly narrow the allowlist or add `Unprotected`; and
- an operation MAY require one exact protection ID for a stricter read without
  mutating the instance allowlist.

This separation permits values written under either authenticated profile to
remain readable while a client changes its write default. Excluding
`Unprotected` from the keyed default prevents an attacker or stale value from
causing a silent downgrade. A decoder MUST reject a value whose
`protection_id` is outside the effective read allowlist and MUST NOT silently
downgrade or fall back.

### 4.2 Compression profiles

| ID | Compression profile |
|---:|---|
| `0` | Uncompressed |
| `1` | Zstandard |

### 4.3 Payload formats

| ID | Payload format |
|---:|---|
| `0` | `OpaqueBytes` |
| `1` | `CBOR` |

Only the payload format IDs listed above are supported by this profile. Any
other payload format ID MUST be rejected. This profile has no in-band
application-format registry. Applications that need a format other than CBOR
MUST encode it as `OpaqueBytes` and agree on its interpretation out of band.

## 5. Payload formats

### 5.1 OpaqueBytes

`OpaqueBytes` is the exact application byte string. Before optional compression
and protection, its payload is byte-for-byte identical to the supplied input.
It has no terminator, sentinel, embedded length, or metadata prefix.
Zero-length input and embedded zero bytes (`0x00`) are valid.

For an uncompressed, unprotected, zero-length `OpaqueBytes` value, the complete
envelope is:

```text
01 00
```

This is two bytes, not a one-byte length encoding:

```text
01 = value_envelope_version 1
00 = selector_byte
     protection_id    = 0  (Unprotected)
     compression_id   = 0  (Uncompressed)
     payload_format_id = 0 (OpaqueBytes)
```

The second `00` is therefore a selector, not a payload length. The empty
payload is represented by the absence of body bytes after that selector.

The selected payload is empty, so the envelope has no body bytes after the
selector. The enclosing protocol frame supplies the envelope length; the
envelope does not contain a separate payload-length field.

### 5.2 CBOR

The CBOR payload MUST contain exactly one complete CBOR data item. The decoder
MUST consume the entire payload. A CBOR sequence, a second item, or any other
trailing bytes MUST be rejected.

Only definite-length encodings are accepted. Indefinite-length arrays and
maps, and chunked indefinite-length byte strings and text strings, MUST be
rejected.

Integer values MAY use any valid CBOR integer encoding. Preferred serialization
is not required. A decoder MUST NOT reject a valid integer solely because it is
not preferred; malformed encodings and invalid additional-information values
remain invalid.

Map order has no semantic meaning. Encoders MAY emit entries in any order, and
decoders MUST accept any order. Map keys are restricted to untagged CBOR
integers, text strings, and byte strings. Arrays, maps, floating-point values,
booleans, null, and other simple values MUST NOT appear as map keys.

A map MUST NOT contain duplicate keys. To determine uniqueness, a decoder MUST
decode each permitted key, re-encode that key using RFC 8949 preferred
serialization, and compare the resulting bytes. Integer encodings of the same
mathematical value therefore compare equal even when one input did not use
preferred serialization. Text and byte-string keys compare by their exact
bytes, and their distinct CBOR major types keep them separate. No Unicode
normalization is applied.

CBOR text strings MUST contain well-formed UTF-8. Other character encodings
MUST be represented as CBOR byte strings, with their interpretation defined by
the application.

CBOR tags are not supported by this profile. Tagged items MUST be rejected.
Nesting depth MUST NOT exceed 128 levels.

## 6. Compression

Compression profile `1` is one standard Zstandard (Zstd) frame under
[RFC 8878](https://www.rfc-editor.org/rfc/rfc8878).

Encoders MUST use a declared content size, no external dictionary, no
skippable frame, and no trailing bytes. Decoders MUST reject missing content
sizes, multiple frames, dictionary requirements, trailing bytes, and any frame
whose declared content size, declared window size, or produced output exceeds
the corresponding limit in §9. For a single-segment frame, the frame content
size is also its window size and is subject to both limits. These checks MUST
reject an excessive declared size or window before allocating the output
buffer or beginning decompression. The decoder MUST bound produced output
during decompression and verify the exact produced size and frame boundary
afterward.

Whether and when an encoder chooses Zstandard is client policy defined by the
[Client Behavioral Contract](CLIENT.md#62-compression-policy), not an envelope
validity rule. An encoder MUST accurately identify the emitted body as either
`Uncompressed` or `Zstandard`.

When secret data is compressed together with attacker-influenced data,
compression SHOULD be disabled or the components SHOULD be stored separately.
Compression can create a side channel through the resulting ciphertext length;
cryptographic protection does not hide that length.

## 7. Cryptographic protection

### 7.1 Value-key schedule

The enclosing client profile supplies a value keyring that maps each positive
unsigned 64-bit `value_key_id` to one immutable `value_key` of exactly 32
bytes. A protected write uses the configured active ID. A protected read parses
the ID before selecting its key. An unknown or retired ID MUST be rejected
without trying another key and without falling back to `Unprotected`.

Value keys are application-managed secrets. The all-zero key MUST NOT be used
as protected key material. Independently rotated keys SHOULD be generated
independently with a cryptographically secure random source. Keyring lifecycle
and rotation ordering are defined by the
[Client Behavioral Contract](CLIENT.md#63-value-key-rotation).

For protected values, implementations MUST derive the value keys exactly as
follows. BLAKE3 `DERIVE_KEY` uses the context strings as UTF-8 bytes and
returns 32 bytes. The `|` operator denotes byte concatenation without
delimiters:

```text
value_key = value_keyring[value_key_id]

value_derivation_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 root key",
    material = value_key[32]
  )

item_material =
    value_derivation_key[32]
  | value_key_id:u64be
  | namespace_id:u64be
  | item_id_length:u8
  | item_id:item_id_length

siv_mac_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-SIV-CMAC MAC key",
    material = item_material
  )

siv_encryption_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-SIV-CMAC encryption key",
    material = item_material
  )

gcm_siv_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-GCM-SIV key",
    material = item_material
  )
```

`value_key_id` is encoded as its numeric value in fixed-width big-endian form
inside the KDF, independent of its canonical `vu128` envelope representation.
`namespace_id` is the exact nonzero server-assigned identity.
`item_id_length` MUST be the exact Item ID length, including zero; it MUST NOT
be replaced by a fixed-width 32-byte buffer.

The key ID, namespace, and exact Item ID intentionally appear in both the KDF
and AAD. The KDF separates cryptographic key domains; the AAD authenticates
the envelope's declared storage identity and key selection. Version and
algorithm separation are already supplied by the BLAKE3 context strings.
Compression and payload-format selection belong only in the authenticated
selector. Request IDs, lanes, opcodes, namespace names and revisions, TTL,
eviction policy, nonce, and value lengths MUST NOT be added to `item_material`;
they are mutable transport or storage metadata, algorithm inputs, or already
authenticated by the selected AEAD.

The key schedule is deterministic, but AES-256-GCM-SIV protection remains
randomized because each write uses a fresh nonce.

### 7.2 Authenticated data

For a protected envelope, the authenticated data MUST include the exact
encoded `value_envelope_version` and `value_key_id` bytes and the exact
`selector_byte`.
This v1 OpenKache profile defines no additional caller-supplied associated-data
component.

For the OpenKache client profile, that associated data is the following
unambiguous byte sequence (all concatenated without delimiters):

```text
aad =
    "openkache/value-format/aad/v1"
  | namespace_id:u64be
  | item_id_length:u8
  | item_id:item_id_length
  | value_envelope_version:vu128
  | selector_byte:u8
  | value_key_id:vu128
```

`namespace_id` MUST be a nonzero server-assigned namespace identity and
`item_id_length` MUST be the exact number of Item ID bytes (0 through 32).
The version and key-ID bytes in `aad` MUST be the same canonical bytes emitted
in the envelope; an implementation MUST NOT re-encode either numeric value
through a different integer representation. This binds a protected value to
its namespace, exact variable-length Item ID, version, selector, and selected
value key.

The AAD MUST be exactly the sequence above. It does not include request ID,
lane, opcode, namespace name or revision, TTL, eviction policy, nonce, or
payload, ciphertext, or envelope length. Transport and mutable server metadata
are not available as stable decryption inputs; the selected AEAD already binds
its nonce when present, ciphertext, and ciphertext length.

The transform order is:

```text
envelope_body = Protect(Compress(payload_bytes))
```

Protection MUST authenticate before any decompression or payload parsing. An
authentication failure MUST use one generic error. Any decrypted working
buffer MUST be zeroized before that error is returned.

### 7.3 AES-256-GCM-SIV

```text
gcm_siv_body = nonce[12] | ciphertext | tag[16]
```

The encoder MUST obtain a fresh 12-byte nonce from the operating-system
cryptographic random source for every write, including repeated writes. It MUST
fail if randomness fails and MUST NOT substitute a timestamp, payload, or
process-local counter. The protection overhead is 28 bytes; the nonce is
public.

### 7.4 AES-256-SIV-CMAC

RFC 5297 SIV uses independent 32-byte authentication and encryption keys:

```text
siv_key_material =
    siv_mac_key[32]
  | siv_encryption_key[32]

siv_body = synthetic_iv[16] | ciphertext
```

Pass the complete associated data as one RFC 5297 associated-data component.
Do not split it into multiple S2V components or add a nonce component. The
protection overhead is 16 bytes. Repeated identical writes with the same key,
associated data, and payload produce identical protected bodies.

### 7.5 Unprotected

The transformed body is stored directly. This profile provides no
confidentiality or authentication; parser bounds and compression validation
remain mandatory.

### 7.6 Size comparison

Let `E = MAX_VALUE_ENVELOPE_BYTES` and let `k` be the one- to nine-byte
canonical encoded width of `value_key_id`. For an uncompressed payload, the
largest payload that can fit in the complete envelope is:

| Protection profile | Envelope overhead | Maximum uncompressed payload |
|---|---:|---:|
| `Unprotected` | 2 bytes | `E - 2` |
| `AES-256-GCM-SIV` | `2 + k + 28` bytes | `E - 2 - k - 28` |
| `AES-256-SIV-CMAC` | `2 + k + 16` bytes | `E - 2 - k - 16` |

With a one-byte key ID and the v1 64 MiB envelope limit, those maximums are
`67,108,862`, `67,108,833`, and `67,108,845` bytes respectively. Compression
replaces the uncompressed payload with a Zstandard frame, so the complete frame
plus the same envelope overhead MUST fit within `E`; the expanded payload
remains independently limited.

### 7.7 Conformance vectors

All vector bytes are hexadecimal. Unless a vector says otherwise, the
parameters are:

```text
value_key_id = 1

value_key =
  00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f
  10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f

namespace_id = 1
payload_format = OpaqueBytes
compression = Uncompressed
payload = 76 61 6c 75 65  # ASCII "value"
```

The common root derivation is:

```text
value_derivation_key =
  58 56 36 33 1c 40 9f 74 c3 6b 0c 46 c2 e3 3f 45
  92 f8 72 37 34 e6 21 15 eb cf 4b 0f 7b 71 0c 33
```

The following vectors cover the empty, short, and maximum-length Item ID
boundaries. These are intermediate key-schedule outputs, not envelope bytes:

| Item ID | `siv_mac_key` | `siv_encryption_key` | `gcm_siv_key` |
|---|---|---|---|
| empty | `14 29 85 a0 bc e1 44 ca 51 7b 29 1f e9 9b 85 e7 a0 96 ce 46 c8 64 8d a9 4b cd 6d db b0 20 6b fa` | `2a 74 4b a3 e8 bf af 94 5c e9 39 48 7e bf e8 7f 7c 88 86 3e a2 af 65 9f b0 bb 0b c6 89 ff 9f 5b` | `42 4f 35 85 9d 43 b7 42 51 af d7 57 5d ec 9f 3e 85 06 b1 1b f1 9c 91 e0 b3 08 f1 8f 5c 52 77 27` |
| `11 22 33` | `a6 08 ed 12 65 fc 4f 6e 06 78 4e 1b 32 eb fb 31 e1 40 9d a8 9d 20 08 ae 09 ac 3e bc ce c3 e0 c4` | `f8 85 ea f7 0a 9b df 3b 9b b7 c6 22 fe 13 c6 35 6c dd e8 fb a0 88 69 ba 0a b9 70 06 c9 c0 b6 c5` | `12 b0 b7 a6 ee 6f d8 c3 2a 6f 38 4c 11 4a 49 07 76 1f a6 fe 55 a3 d2 72 27 49 67 a8 13 e6 b4 98` |
| `00 01 02 ... 1f` | `90 eb 89 f2 d5 53 a4 da e8 8c b2 8c 7f 5d d9 76 b7 dd c9 fb 22 3e b7 0b 21 eb 02 97 ce b5 96 d8` | `82 9b 71 3d a1 96 b9 16 ed 33 7b 93 0a 4e 5a af 61 87 b4 ee 89 17 3c 9c f9 dc ea 86 0e d1 45 7a` | `be b0 2c a6 0e fc 5f 33 ce 58 55 c2 df d7 13 2c b7 58 46 b6 61 03 7f f6 85 05 e2 02 cd 09 0e 4e` |

#### Empty Item ID with AES-256-SIV-CMAC

The selector is `02`; the following `01` in the envelope is the canonical
encoding of `value_key_id = 1`. The AAD is:

```text
6f 70 65 6e 6b 61 63 68 65 2f 76 61 6c 75 65 2d
66 6f 72 6d 61 74 2f 61 61 64 2f 76 31
00 00 00 00 00 00 00 01 00 01 02 01
```

The complete envelope is:

```text
01 02 01 77 52 d8 b3 d8 79 fd 36 a4 fe 88 4e ad
66 f4 f2 82 21 13 d9 f3
```

#### Three-byte Item ID with AES-256-GCM-SIV

This test fixes the nonce to `00 01 02 03 04 05 06 07 08 09 0a 0b`. Production
encoders MUST still generate a fresh random nonce as required by §7.3. The
selector is `01`, followed by value-key ID `01`. The AAD is:

```text
6f 70 65 6e 6b 61 63 68 65 2f 76 61 6c 75 65 2d
66 6f 72 6d 61 74 2f 61 61 64 2f 76 31
00 00 00 00 00 00 00 01 03 11 22 33 01 01 01
```

The complete envelope is:

```text
01 01 01 00 01 02 03 04 05 06 07 08 09 0a 0b
02 09 d6 19 33 dc 4d ca 6f 44 e4 81 20 8a 68 f4
82 7e c0 71 ad
```

For the same Item ID, nonce, and payload with `namespace_id = 2`, the complete
envelope is:

```text
01 01 01 00 01 02 03 04 05 06 07 08 09 0a 0b
c6 b9 22 82 1c 1a 45 09 9d eb 85 00 c8 bc ee 23
37 58 28 95 94
```

The derived `gcm_siv_key`, ciphertext, and tag all change because the namespace
is included in both the KDF and AAD.

#### Maximum-length Item ID with AES-256-SIV-CMAC

For Item ID `00 01 02 ... 1f`, the selector is `02`, followed by value-key ID
`01`. The AAD is:

```text
6f 70 65 6e 6b 61 63 68 65 2f 76 61 6c 75 65 2d
66 6f 72 6d 61 74 2f 61 61 64 2f 76 31
00 00 00 00 00 00 00 01 20
00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f
10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f
01 02 01
```

The complete envelope is:

```text
01 02 01 44 25 17 06 34 6f a1 25 04 3a 5a 8e b0
66 7b 7f 77 2f a0 03 8c
```

#### Rotated value key and multi-byte key ID

This vector uses the same namespace, three-byte Item ID, nonce, and payload as
the GCM-SIV vector above, but selects:

```text
value_key_id = 128
canonical value_key_id = 80 02

value_key =
  ff fe fd fc fb fa f9 f8 f7 f6 f5 f4 f3 f2 f1 f0
  ef ee ed ec eb ea e9 e8 e7 e6 e5 e4 e3 e2 e1 e0

value_derivation_key =
  a9 99 6e bc 85 bb e9 64 9a 7f 73 8a b3 c4 ac a9
  48 a4 2c 8d eb 30 6b fc a6 35 9b 13 ce 21 98 00

gcm_siv_key =
  77 94 bd 3b b3 a5 56 90 b9 de f4 e0 0e 11 c2 59
  5c 47 3e 24 30 f9 fe a1 a0 3b e5 d7 d0 c5 7f b1
```

The AAD is:

```text
6f 70 65 6e 6b 61 63 68 65 2f 76 61 6c 75 65 2d
66 6f 72 6d 61 74 2f 61 61 64 2f 76 31
00 00 00 00 00 00 00 01 03 11 22 33 01 01 80 02
```

The complete envelope is:

```text
01 01 80 02 00 01 02 03 04 05 06 07 08 09 0a 0b
98 a1 6f 15 1e 84 5b 9a e1 f4 cb cf 48 41 1a 92
a5 2d 6a bf 49
```

This vector proves that the key ID uses canonical `vu128` in the envelope and
AAD, fixed-width `u64be` in the KDF, and an independently selected value key.

#### Unprotected and rejection vectors

The uncompressed, unprotected envelope for the common payload is independent
of the value keyring, namespace, and Item ID. It has no value-key ID:

```text
01 00 76 61 6c 75 65
```

For any protected vector above, changing the namespace ID, Item ID length,
Item ID bytes, encoded version, selector, value-key ID, ciphertext, or
authentication tag MUST cause rejection. Zero, non-canonical, truncated, and
unknown value-key IDs MUST be rejected without trying another key. Truncating
a nonce, synthetic IV, or tag MUST be rejected before decryption. A decoder
MUST NOT return partially decrypted or decompressed bytes for any rejection
case.

## 8. Encoding and decoding

An encoder MUST:

1. Convert the source value using the selected payload format.
2. Enforce the expanded payload limit.
3. Compress the payload only when the selected policy permits.
4. Encode the value-envelope version and selector byte.
5. For a protected profile, resolve and encode the active nonzero value-key ID,
   select its exact key, and derive the item-specific protection key or keys.
6. Construct the profile-defined AAD, including the exact version, selector,
   and key-ID bytes and the namespace and Item ID fields.
7. Apply the selected protection profile.
8. Enforce the complete envelope limit before returning the bytes.

For this profile, the encoder MUST NOT emit a magic prefix, body length,
unsupported selector, zero or non-canonical protected value-key ID, or a
transform inconsistent with the selector byte. It MUST NOT emit a value-key ID
for `Unprotected`.

A decoder MUST:

1. Parse one canonical `value_envelope_version:vu128` from the complete
   envelope.
2. Dispatch the version before parsing its version body.
3. Parse and validate the selector byte.
4. Enforce the caller's expected protection policy.
5. For a protected profile, parse one canonical nonzero value-key ID, look up
   exactly that key, and derive the item-specific protection key or keys.
6. Construct the authenticated data from the selected profile's rules.
7. Check minimum protected-body sizes before slicing.
8. Authenticate and decrypt before decompression or payload parsing.
9. Validate and bounded-decompress one Zstandard frame when selected.
10. Decode exactly one CBOR item or return the exact `OpaqueBytes` payload.

Unknown versions, selector values, value-key IDs, compression profiles, payload
formats, malformed payloads, and disallowed trailing bytes MUST be rejected.
The decoder MUST NOT probe other keys, silently downgrade, fall back, or
reinterpret an unknown value as another payload format.

## 9. Limits and rejection rules

```text
MAX_VALUE_ENVELOPE_BYTES = 67,108,864  // 64 MiB
MAX_EXPANDED_PAYLOAD_BYTES = 67,108,864
MAX_ZSTD_WINDOW_BYTES = 67,108,864
```

`MAX_VALUE_ENVELOPE_BYTES` applies to the complete byte string stored and
returned opaquely by the server, including version, selector, value-key ID,
nonce or synthetic IV, ciphertext, authentication tag, and any Zstandard frame
overhead. It equals the wire protocol's maximum `SET` value and response
payload.

`MAX_EXPANDED_PAYLOAD_BYTES` applies to the payload after decryption and
decompression but before CBOR parsing. `MAX_ZSTD_WINDOW_BYTES` is the largest
declared decoder window. All three limits are independent: satisfying one does
not waive either of the others.

Implementations MAY use lower local limits, but MUST check the complete
envelope, declared Zstandard content size and window size, produced output, and
all arithmetic before allocation. A decoder MUST reject a frame with no
declared content size, more than one frame, any dictionary ID, a skippable
frame, trailing bytes, a declared or produced size above its limit, or a
produced size different from the declared content size.

Malformed `vu128` encodings, truncated headers, zero or unknown protected
value-key IDs, nonzero selector zero bits, unsupported selector values, invalid
UTF-8 text strings, duplicate CBOR map keys, tagged or indefinite-length CBOR
items, invalid Zstandard frames, authentication failures, and payloads
exceeding limits MUST be rejected.

## 10. Versioning and extension

`value_envelope_version` selects a complete envelope grammar. It is not a
version of the selector byte or envelope body. A future OpenKache-defined
version MAY change selector layout and assignments, body framing, transform
order, key-selection framing, authentication inputs, and limits.

This profile uses `value_envelope_version = 1`. A reader MUST reject a version
it does not support rather than guessing its grammar. Version `0` is available
only for a complete application-defined envelope profile configured out of
band.

Future OpenKache versions are additive: a newer version may represent values or
policies that this profile cannot. A client MAY choose the oldest supported
OpenKache version that represents its selected value and policy. That version
selection is independent of the payload bytes.

## 11. Security properties

Protected profiles provide confidentiality and ciphertext authentication for
the supplied key and associated data. `Unprotected` provides neither.

For a protected envelope, authentication covers the envelope version,
selector, value-key ID, namespace ID, exact Item ID and its length, nonce when
present, and the complete protected payload. It does not authenticate TTL,
expiration behavior, eviction behavior, namespace policy or revision,
existence, freshness, replay, ordering, or availability.

The value-key ID is visible and leaks which configured key epoch wrote a
protected envelope. Changing it to another configured ID changes both the
derived protection key and AAD and therefore causes authentication failure.
Changing it to an unknown ID causes rejection without key probing.

This profile does not provide freshness or replay protection. An older valid
envelope can be accepted again unless the enclosing client or server protocol
adds a generation, CAS, expiry, or equivalent freshness mechanism.
Applications that require freshness, version, or expiry integrity SHOULD place
the required generation, timestamp, or policy data inside the protected payload
and validate it after decryption.

The value keyring defines the cryptographic portability domain. Reusing an
identical key-ID mapping in another deployment permits a protected envelope to
authenticate there when the namespace ID and Item ID are also preserved.
Deployments requiring isolation must use independent value keys. Namespace
names, accounts, client identities, and deployment identifiers are not
cryptographic inputs in this profile.

All protected profiles leak the encoded envelope length. Compression can add
content-dependent length leakage; callers that cannot tolerate that side
channel SHOULD disable compression or separate secret and attacker-influenced
data.
