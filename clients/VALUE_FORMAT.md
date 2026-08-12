# OpenKache v1 Client Value Format

> **Status: Draft — v1 pre-freeze**
>
> This document defines OpenKache's default formatted client value profile. It
> is a client-side convention, not a server-required value encoding.
>
> The packed codec layout below is the target v1 contract. The key-conversion
> change in this branch is implemented; migration of the shared value codec
> from its prior serialization-id body to this layout is a separate change.

The server stores values as opaque bytes. It does not interpret serialization,
compression, encryption, or client metadata. Applications MAY use the raw
client API with an exact 32-byte Item ID and opaque value bytes instead. Raw
operations bypass this document's formatted value path but still obey protocol
framing, namespace scoping, and size limits.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

## 1. Scope and profile selection

The formatted value path is:

```text
native value
  -> selected value codec or exact RawBytes
  -> codec payload
  -> optional Zstandard compression
  -> optional authenticated protection
  -> ValueEnvelope
  -> opaque server value
```

Formatted key conversion and Item ID derivation are defined separately in
[Key Format](KEY_FORMAT.md); they are not part of this document. Value codecs
have their own value model and conversion rules. The key input restrictions do
not apply to values.

V1 provides two value codecs:

| Codec | Meaning |
|---|---|
| `RawBytes` | Exact application bytes, including empty input and embedded `00` octets. |
| `OpenKache CBOR Value v1` | One CBOR value subject to the v1 rules in §3.4. |

`RawBytes` describes a value payload, not a key type or an Item ID.

There is no separate client-only metadata envelope. Applications that need
custom metadata MAY place it inside `RawBytes`, or use the raw client API.

## 2. Client protection mode

The formatted client selects one protection mode when it is initialized. The
mode applies to every formatted value operation; encryption cannot be enabled
or disabled for an individual value.

| Initialization | Formatted writes and reads |
|---|---|
| No `client_root_key` | Use `Unprotected` for every value. The default root is 32 zero octets and affects only the formatted Item ID. |
| Explicit `client_root_key` | Protect every value. `AES-256-GCM-SIV` is the default; `AES-256-SIV-CMAC` is an explicit client-wide alternative. |

When protection is enabled, the client MUST require an explicitly supplied
32-octet key and MUST reject the all-zero key. A client MUST reject a
formatted value whose encryption ID conflicts with its configured mode.

This policy applies only to the formatted client path. Raw APIs continue to
accept exact Item IDs and opaque value bytes; callers own any protection for
raw values.

With `Unprotected`, no value-encryption key is derived.

The client selects one immutable complete client profile at initialization.
All formatted operations use that profile; protection cannot vary per value.
The profile determines both the key rules in [Key Format](KEY_FORMAT.md) and
this value format. `value_envelope_version` identifies the value side of that
profile; it does not perform key conversion or Item ID derivation and is not an
additional Item ID input.

The no-key profile still writes a complete `ValueEnvelope`. Its
`encryption_id` is `0`, while the codec and compression IDs continue to
identify the payload. This preserves self-describing, cross-language decoding;
only the raw client API bypasses the envelope.

## 3. ValueEnvelope

### 3.1 Layout

```text
value_envelope = value_envelope_version:vu128 | format_flags:u8 | body
```

| Field | Size | Meaning |
|---|---:|---|
| `value_envelope_version` | 1–9 bytes | Canonical profile selector; v1 is `01`. |
| `format_flags` | 1 byte | Packed encryption, compression, and codec IDs. |
| `body` | remaining bytes | Codec payload after the selected transforms. |

The envelope has no magic prefix and no body-length field. The enclosing
protocol frame supplies the exact value boundary; `value_len` is the canonical
[`vu128`](../protocol/SPEC.md#unsigned-vu128) length of the complete envelope.

`value_envelope_version` MUST use the shortest canonical `vu128`. Decoders
MUST reject truncation, overflow, reserved prefixes, and overlong encodings.
The v1 maximum encoded width is nine bytes.

### 3.2 Packed flags

```text
bits 0..1 = encryption_id
bits 2..3 = compression_id
bits 4..5 = codec_id
bits 6..7 = zero

format_flags =
    encryption_id
  | (compression_id << 2)
  | (codec_id << 4)
```

Encoders MUST emit only assigned IDs with zero high bits. Decoders MUST
reject unassigned IDs and nonzero high bits; they MUST NOT guess.

#### Encryption IDs

The ID selects the complete algorithm profile. The profile name is the full
algorithm name; nonce and repetition behavior are normative properties of that
profile. Numeric IDs are wire assignments, not a security ranking.

| ID | Algorithm/profile | Policy |
|---:|---|---|
| `0` | `Unprotected` | No confidentiality or authentication; selected when no key is configured. |
| `1` | `AES-256-GCM-SIV` ([RFC 8452](https://www.rfc-editor.org/rfc/rfc8452)) | Fresh random nonce per write; default when protection is enabled. |
| `2` | `AES-256-SIV-CMAC` ([RFC 5297](https://www.rfc-editor.org/rfc/rfc5297)) | Deterministic and nonce-free; explicit opt-in. |

#### Compression IDs

| ID | Meaning |
|---:|---|
| `0` | None |
| `1` | Zstandard |

#### Codec IDs

| ID | Meaning |
|---:|---|
| `0` | `RawBytes` |
| `1` | CBOR |

### 3.3 Body

```text
codec_payload =
    exact_application_bytes
  | openkache_cbor_value_v1

transformed_body = protect(compress(codec_payload))
```

`codec_id` is in `format_flags`, not in the payload. `RawBytes` has no
terminator, sentinel, embedded length, or metadata header. An empty,
uncompressed, unprotected `RawBytes` value is exactly:

```text
01 00
```

The value codec is independent from the key codec. Its native conversion rules
are not inherited from [Key Format](KEY_FORMAT.md).

### 3.4 OpenKache CBOR Value v1

This is a value profile, not a deterministic or canonical CBOR profile. It
defines which CBOR structures a decoder accepts; it does not require one
byte representation for one logical value. Applications that require exact
byte-for-byte reproducibility MUST use `RawBytes` or another explicitly
defined codec.

The CBOR codec payload MUST contain exactly one complete CBOR data item. A
decoder MUST consume the entire payload. A CBOR sequence, a second item after
the first item, or any other trailing bytes MUST be rejected.

Only definite-length encodings are supported. Indefinite-length arrays and
maps, and chunked indefinite-length byte strings and text strings, MUST be
rejected. This keeps the value boundary explicit and permits bounded parsing
without streaming or chunk-count rules.

Integer values MAY use any valid CBOR integer encoding supported by the
profile. Preferred integer serialization is not required, and a decoder MUST
NOT reject a valid non-preferred integer encoding solely because it is
non-preferred. Encoders SHOULD use compact preferred encodings when practical.
Malformed encodings and reserved additional-information values remain invalid.

Map order has no semantic meaning. Encoders MAY emit entries in any order, and
decoders MUST accept any order. A map MUST NOT contain duplicate keys as
determined by the decoded key values; a decoder that cannot determine key
uniqueness MUST reject the map.

CBOR text strings MUST contain well-formed UTF-8. Other character encodings
are not text-string encodings in this profile and MUST be represented as CBOR
byte strings, with their interpretation defined by the application.

The base v1 profile assigns no application-specific CBOR tags. Unknown or
unassigned tags MUST be rejected. Applications that need custom typed payloads
MUST use `RawBytes` or another explicitly negotiated codec.

## 4. Compression

Compression ID `1` is one standard Zstandard frame under
[RFC 8878](https://www.rfc-editor.org/rfc/rfc8878). Encoders MUST use a
declared content size, no external dictionary, no skippable frame, and no
trailing bytes. Decoders MUST reject missing content sizes, multiple frames,
dictionary requirements, oversized windows, trailing bytes, and output above
the decoded-value limit.

The initial SDK policy is Zstandard level 1, no compression below 1,024
codec-payload bytes, and no compression unless it saves at least 64 bytes. An
encoder MAY retain the uncompressed body when compression is not beneficial.

When a value combines secret or otherwise sensitive material with
attacker-controlled or probeable bytes, compression SHOULD be disabled or the
components SHOULD be stored separately. Compression can expose relationships
through final ciphertext length; encryption does not hide that length.

## 5. Protection

### 5.1 Key material and AAD

Protected values derive an independent value root:

```text
value_encryption_root_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 root key",
    material = client_root_key[32]
  )

item_id_material = value_encryption_root_key[32] | item_id[32]
```

Derived secrets MUST be zeroized. Implementations SHOULD derive per-item keys
on demand rather than retain a high-cardinality key cache.

Both protected profiles authenticate:

```text
aad =
  ascii("openkache/value-format/aad/v1")
  | namespace_id:u64be
  | item_id[32]
  | encoded_value_envelope_version
  | format_flags
```

The version and flags are the exact bytes stored in the envelope. The
namespace is included separately in AAD even though it is also part of Item ID
derivation. AAD binds the ciphertext to its namespace, Item ID, profile,
codec, and transforms; it does not provide freshness.

### 5.2 AES-256-GCM-SIV (ID 1)

```text
gcm_siv_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-GCM-SIV key",
    material = item_id_material[64]
  )

gcm_siv_body = nonce[12] | ciphertext | tag[16]
```

The encoder MUST obtain a fresh 12-byte nonce from the operating-system
cryptographic random source for every write, including repeated writes. It
MUST fail if randomness fails and MUST NOT substitute a timestamp, Item ID,
plaintext, or process-local counter. Overhead is 28 bytes; the nonce is public.

### 5.3 AES-256-SIV-CMAC (ID 2)

Derive independent 32-byte AES-SIV-CMAC keys:

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

Pass the complete AAD as exactly one RFC 5297 associated-data component; do
not split it into multiple S2V components or add a nonce component.

```text
deterministic_siv_body = synthetic_iv[16] | ciphertext
```

Overhead is 16 bytes and there is no random nonce. Repeated identical writes
to one Item ID therefore repeat the stored bytes; per-item keys prevent
cross-Item-ID plaintext equality comparison.

### 5.4 Unprotected (ID 0)

The transformed body is stored directly. This profile provides no
confidentiality or authentication; parser bounds and compression validation
remain mandatory. Clients that require protection MUST reject it.

### 5.5 Size comparison

| Profile | Envelope/crypto overhead | Empty uncompressed `RawBytes` |
|---|---:|---:|
| Unprotected | 2 bytes | 2 bytes |
| AES-256-GCM-SIV | 30 bytes | 30 bytes |
| AES-256-SIV-CMAC | 18 bytes | 18 bytes |

AES-256-GCM-SIV is 12 bytes larger than AES-256-SIV-CMAC. Compression adds
its own frame overhead and may enlarge small values.

## 6. Processing requirements

### 6.1 Encoding

An encoder MUST:

1. Convert a structured native value using the selected value codec, or accept
   exact `RawBytes`.
2. Encode the selected codec and enforce the decoded-payload limit.
3. Apply compression only when policy permits.
4. Encode version `1` and the packed `format_flags`.
5. Construct AAD from the exact namespace, Item ID, and header bytes.
6. Apply the selected protection profile.
7. Enforce the complete ValueEnvelope limit before sending.

It MUST NOT emit a magic prefix, body length, unassigned ID, non-canonical
`vu128`, or an algorithm inconsistent with the flags.

### 6.2 Decoding

A decoder MUST:

1. Parse one canonical `value_envelope_version:vu128` from the exact envelope
   slice.
2. Require version `1`.
3. Parse and validate `format_flags`.
4. Enforce the configured protection policy.
5. Construct AAD from the supplied namespace, Item ID, and exact header.
6. Check version-specific minimum body sizes before slicing.
7. Authenticate and decrypt before decompression or codec parsing.
8. Validate and bounded-decompress one Zstandard frame when selected.
9. For a structured codec, decode exactly one complete payload item and reject
   codec-invalid or trailing bytes. For `RawBytes`, return the exact payload.

Authentication failures MUST use one generic error. Unauthenticated plaintext
MUST be zeroized before returning that error.

### 6.3 Limits

- Complete stored `ValueEnvelope`: at most 64 MiB.
- Decoded codec payload: at most 64 MiB.

Implementations MAY use lower limits, but MUST check declared sizes, Zstandard
windows, produced output, and all arithmetic before allocation.

## 7. Versioning and compatibility

`value_envelope_version` is the value-side selector for an immutable complete
client profile. V1 readers accept only `1`, reject unknown versions and IDs,
and never guess. The old magic-prefixed envelope `4F 4B 56 01` is not v1.

Future profiles are capability extensions: newer profiles add values or
policies that older profiles cannot represent. A newer client SHOULD use the
oldest supported complete profile that represents the key, value, and required
security policy at initialization. A v1-representable client therefore uses
the v1 key rules and v1 ValueEnvelope together.

The client MUST NOT calculate a v1 Item ID and store a newer-profile value
under it, or combine a newer key contract with a v1 value envelope. Migration
to a newer profile is an explicit read/decode/write operation and MUST account
for a changed Item ID.

| Policy | Meaning |
|---|---|
| `Exact(vN)` | Require `vN`; fail if it cannot represent the client configuration. |
| `OldestCompatible` | Default; choose the oldest complete profile that works. |
| `LatestSupported` | Explicit opt-in; may make older clients unable to read. |

## 8. Replay and ownership

V1 authenticates identity and transform selection, not time or write order.
Valid older values can be replayed to the same Item ID. Generation/version and
server CAS are intentionally deferred; freshness MUST NOT be inferred from an
Item ID, nonce, synthetic IV, TTL, or namespace revision.

The protocol owns frame lengths, namespace IDs, Item IDs, and server semantics.
The shared client core owns `vu128`, codecs, compression, protection, AAD,
bounded parsing, and stable errors. Language adapters own native conversion,
memory ownership, runtime integration, and error wrappers; they MUST NOT
duplicate wire or cryptographic logic.
