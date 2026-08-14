# OpenKache Client Value Encoding Profile v1 (Draft)

> **Status: Draft — pre-freeze**

This document defines the client-side v1 value encoding before it is handed to
the server. The server stores the resulting bytes opaquely and does not
interpret payload formats, compression, or cryptographic protection.

The Rust `ValueCodec` and TypeScript `get_raw`/`set_raw` convenience methods
use this normative format. TypeScript `get`/`set` and the legacy
`value_envelope` module use a separate migration envelope (`OKV1` magic prefix
plus metadata lengths); that format is not described by the grammar below.
Low-level exact Item ID APIs, such as `RawClient` or native `raw_get`/`raw_set`
operations, bypass this envelope and send the caller's Item ID and opaque
value bytes directly. A binding MUST document whether a method named
`get_raw` or `set_raw` is a logical-key convenience method using this format
or an exact Item ID operation bypassing it.

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

The enclosing client profile supplies the `client_root_key` and the exact
namespace and Item ID. This profile defines the value-key schedule derived
from that root, the value envelope, its authenticated binding to the
namespace and exact Item ID, and the transforms applied to its payload.

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
version_body  = selector_byte:u8 | envelope_body
```

The profile uses `value_envelope_version = 1`:

```text
value_envelope_v1 =
    value_envelope_version:vu128(1)
  | selector_byte:u8
  | envelope_body
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

The shared client core has an instance-wide default protection profile. Each
formatted operation MAY specify a protection-profile override when its language
binding exposes that operation-local option. If no override is supplied, the
operation uses the instance default; an override MUST NOT mutate that default.
The selected operation profile MUST be represented by `protection_id`.

Without a configured client root key, the default and only valid profile is
`Unprotected`. With a configured key, the default profile is
`AES-256-GCM-SIV`. An omitted connection profile therefore means
`Unprotected` without a key and `AES-256-GCM-SIV` with a key. The client MAY
explicitly select an authenticated profile where its connection API exposes
that choice. Selecting an authenticated protection profile without its
required key MUST fail; a binding MUST preserve that explicit selection so the
shared core can reject it rather than silently downgrading to `Unprotected`.
Operation-local APIs MAY additionally select `Unprotected`. A decoder MUST
reject a value whose protection ID is disallowed by the caller's configured
profile and MUST NOT silently downgrade or fall back.

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
decoders MUST accept any order. A map MUST NOT contain duplicate keys as
determined by the decoded key values. A decoder that cannot determine key
uniqueness MUST reject the map.

CBOR text strings MUST contain well-formed UTF-8. Other character encodings
MUST be represented as CBOR byte strings, with their interpretation defined by
the application.

CBOR tags are not supported by this profile. Tagged items MUST be rejected.
The core acceptance implementation additionally limits nesting depth to 128
levels and rejects compound or floating-point map keys when it cannot
determine semantic key uniqueness. These are bounded-parser requirements for
the v1 acceptance profile, not alternate payload semantics.

## 6. Compression

Compression profile `1` is one standard Zstandard (Zstd) frame under
[RFC 8878](https://www.rfc-editor.org/rfc/rfc8878).

Encoders MUST use a declared content size, no external dictionary, no
skippable frame, and no trailing bytes. Decoders MUST reject missing content
sizes, multiple frames, dictionary requirements, oversized windows, trailing
bytes, and decompressed output above the payload limit.

The generated client contract supplies Zstd level `1`, a minimum input size of
`1,024` payload bytes, and a minimum savings threshold of `64` bytes as the
default settings when compression is enabled. Compression enablement is a
language-adapter policy; the current adapters use these defaults:

| Adapter | Compression enabled when omitted |
|---|---:|
| Rust core, C ABI, C++, Go, Swift, CLI | No |
| Python, TypeScript | Yes |
| .NET | Not applicable (exact Item ID API only) |

Callers sharing a workload SHOULD select the same policy. An adapter
documentation MUST state its default explicitly. An encoder MAY select
compression profile `0` when compression is not beneficial. It MUST NOT label
an uncompressed body as Zstandard.
When secret data is compressed together with attacker-influenced data,
compression SHOULD be disabled or the components SHOULD be stored separately.
Compression can create a side channel through the resulting ciphertext length;
cryptographic protection does not hide that length.

## 7. Cryptographic protection

### 7.1 Value-key schedule

The enclosing client profile supplies one `client_root_key` of exactly 32
bytes. The key is application-managed; this document does not define how it is
generated, stored, or rotated. A protected operation MUST reject a missing or
invalid root key. The all-zero root is reserved for an explicitly unprotected
client profile and MUST NOT be used as protected key material.

For protected values, implementations MUST derive the value keys exactly as
follows. BLAKE3 `DERIVE_KEY` uses the context strings as UTF-8 bytes and
returns 32 bytes. The `|` operator denotes byte concatenation without
delimiters:

```text
value_root_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 root key",
    material = client_root_key[32]
  )

item_material =
    value_root_key[32]
  | item_id_length:u8
  | item_id:item_id_length

compact_mac_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-SIV-CMAC MAC key",
    material = item_material
  )

compact_encryption_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-SIV-CMAC encryption key",
    material = item_material
  )

robust_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-GCM-SIV key",
    material = item_material
  )
```

`item_id_length` MUST be the exact length of the Item ID, including when it is
zero; it MUST NOT be replaced by a fixed-width 32-byte buffer. Namespace ID
is not part of `item_material`; it is authenticated through the AAD below.
The key schedule is deterministic, but Robust protection remains randomized
because each write uses a fresh nonce.

### 7.2 Authenticated data

For a protected envelope, the authenticated data MUST include the exact
encoded `value_envelope_version` bytes and the exact `selector_byte`.
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
```

`namespace_id` MUST be a nonzero server-assigned namespace identity and
`item_id_length` MUST be the exact number of Item ID bytes (0 through 32).
The version bytes in `aad` MUST be the same canonical bytes emitted in the
envelope; an implementation MUST NOT re-encode the numeric version through a
different integer representation. This binds a protected value to its
namespace, exact variable-length Item ID, version, and selector.

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
    authentication_key[32]
  | encryption_key[32]

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

| Protection profile | Envelope header | Protection overhead |
|---|---:|---:|
| `Unprotected` | 2 bytes | 0 bytes |
| `AES-256-GCM-SIV` | 2 bytes | 28 bytes |
| `AES-256-SIV-CMAC` | 2 bytes | 16 bytes |

Compression adds its own frame overhead and may enlarge small values.

## 8. Encoding and decoding

An encoder MUST:

1. Convert the source value using the selected payload format.
2. Enforce the expanded payload limit.
3. Compress the payload only when the selected policy permits.
4. Encode the value-envelope version and selector byte.
5. Construct the profile-defined AAD, including the exact version and selector
   bytes and the namespace and Item ID fields.
6. Apply the selected protection profile.
7. Enforce the complete envelope limit before returning the bytes.

For this profile, the encoder MUST NOT emit a magic prefix, body length,
unsupported selector, non-canonical `vu128`, or a transform inconsistent with
the selector byte.

A decoder MUST:

1. Parse one canonical `value_envelope_version:vu128` from the complete
   envelope.
2. Dispatch the version before parsing its version body.
3. Parse and validate the selector byte.
4. Enforce the caller's expected protection policy.
5. Construct the authenticated data from the selected profile's rules.
6. Check minimum protected-body sizes before slicing.
7. Authenticate and decrypt before decompression or payload parsing.
8. Validate and bounded-decompress one Zstandard frame when selected.
9. Decode exactly one CBOR data item or return the exact `OpaqueBytes` payload.

Unknown versions, selector values, compression profiles, payload formats,
malformed payloads, and disallowed trailing bytes MUST be rejected. The
decoder MUST NOT silently downgrade, fall back, or reinterpret an unknown
value as another payload format.

## 9. Limits and rejection rules

- Complete `ValueEnvelope`: at most 64 MiB.
- Expanded payload: at most 64 MiB.

Implementations MAY use lower limits, but MUST check declared sizes, Zstd
windows, decompressed output, and all arithmetic before allocation.

Malformed `vu128` encodings, truncated headers, nonzero selector zero bits,
unsupported selector values, invalid UTF-8 text strings, duplicate CBOR map
keys, tagged or indefinite-length CBOR items, invalid Zstandard frames,
authentication failures, and payloads exceeding limits MUST be rejected.

## 10. Versioning and extension

`value_envelope_version` selects a complete envelope grammar. It is not a
version of the selector byte or envelope body. A future OpenKache-defined
version MAY change selector layout and assignments, body framing, transform
order, authentication inputs, and limits.

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

This profile does not provide freshness or replay protection. An older valid
envelope can be accepted again unless the enclosing client or server protocol
adds a generation, CAS, expiry, or equivalent freshness mechanism.

All protected profiles leak the encoded envelope length. Compression can add
content-dependent length leakage; callers that cannot tolerate that side
channel SHOULD disable compression or separate secret and attacker-influenced
data.
