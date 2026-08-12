# OpenKache Client Value Encoding Profile v1 (Draft)

> **Status: Draft — v1 pre-freeze**
>
> This document defines OpenKache's default managed client value encoding
> profile. It is a client-side convention, not a server-required value
> encoding.

The server stores values as opaque bytes. It does not interpret serialization,
compression, cryptographic protection, or client metadata. Applications MAY
use the opaque client API with an opaque `0..=32`-byte Item ID and opaque value
bytes instead. Opaque operations bypass this document's managed value path but
still obey protocol framing, namespace scoping, and size limits.
The protected managed path uses the 32-byte Item ID produced by the client Item
ID derivation profile; variable-length opaque Item IDs are outside this
envelope.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

## 1. Scope and profile selection

The managed value path is:

```text
source value
  -> selected payload format or exact OpaqueBytes
  -> payload bytes
  -> optional Zstandard compression
  -> optional cryptographic protection
  -> ValueEnvelope
  -> opaque server value
```

Key conversion and Item ID derivation are defined separately in [Key Format](KEY_FORMAT.md);
they are not part of this document. Payload formats have their own value model
and conversion rules. The key input restrictions do not apply to values.

V1 provides three payload format selectors:

| Payload format | Meaning |
|---|---|
| `OpaqueBytes` | Exact application bytes, including zero-length input and embedded `00` bytes. |
| `CBOR` | One CBOR value subject to the v1 acceptance rules in §3.5. |
| `ApplicationDefined` | An application-defined format identified inside the payload. |

`OpaqueBytes` describes a value payload, not a key type or an Item ID. Use
`OpaqueBytes` when the format is fixed or identified out of band. Use
`ApplicationDefined` when one keyspace must carry multiple application formats
and the format identity must travel with each value.

There is no separate client-only metadata envelope. Applications that need
application metadata MAY use the `ApplicationDefined` payload format,
`OpaqueBytes`, or the opaque client API. A client that does not register the
referenced application format MUST reject the value.

## 2. Protection profile selection

The managed client has an instance-wide
`default_protection_profile`. Each managed operation MAY provide a
`protection_profile` override. When an operation does not provide an override,
it uses the instance default. The selected profile applies to that operation
only; changing a per-operation selection MUST NOT mutate the instance default.

| Initialization | Default protection profile | Permitted per-operation profiles |
|---|---|---|
| No `client_root_key` | `Unprotected` | `Unprotected` only |
| Explicit `client_root_key` | `AES-256-GCM-SIV` | `Unprotected`, `AES-256-GCM-SIV`, or `AES-256-SIV-CMAC` |

An explicit initialization profile MAY replace the default, subject to the
same key requirements. When protection is selected, the client MUST require an
explicitly supplied 32-byte key and MUST reject the all-zero key. A client
MUST reject a per-operation protection selection that is incompatible with its
configured key material.

On writes, the selected operation profile determines the `protection_id`
emitted in the ValueEnvelope. On reads, the selected operation profile is the
expected `protection_id`; a stored value with a different ID MUST be rejected.
Callers that intentionally mix profiles in one keyspace MUST select the
matching profile when reading each value. A client MUST NOT silently downgrade
or fall back to another profile after a mismatch.

This policy applies only to the managed client path. Opaque APIs continue to
accept exact Item IDs and opaque value bytes; callers own any protection for
opaque values.

With `Unprotected`, no value-protection key is derived.

The client selects immutable key-conversion rules, available protection
profiles, and the instance-wide default at initialization. The value-envelope
version is also selected per operation from the client's configured version
policy; it is not an additional Item ID input and does not perform key
conversion or Item ID derivation.

The no-key profile still writes a complete `ValueEnvelope`. Its
`protection_id` is `0`, while the payload-format and compression IDs continue
to identify the payload. This preserves self-describing, cross-language
decoding; only the opaque client API bypasses the envelope.

## 3. ValueEnvelope

### 3.1 Layout

```text
value_envelope = value_envelope_version:vu128 | version_body
```

For v1, the version body is:

```text
value_envelope_v1 =
    value_envelope_version:vu128(1)
  | profile_byte:u8
  | envelope_body
```

| Field | Size | Meaning |
|---|---:|---|
| `value_envelope_version` | 1–9 bytes | Common canonical version discriminator; v1 is `01`. |
| `version_body` | version-dependent | Complete body grammar selected by the version. |

### 3.2 Version assignments

| Version | Assignment | Interpretation |
|---:|---|---|
| `0` | Application-defined | Complete application-defined envelope profile configured out of band; OpenKache does not define or interpret it. |
| `1` | OpenKache-defined Value Envelope v1 | The profile byte, body, transforms, and limits defined in this document. |

The envelope has no magic prefix and no body-length field. The enclosing
protocol frame supplies the exact value boundary; `value_len` is the canonical
[`vu128`](../protocol/SPEC.md#unsigned-vu128) length of the complete envelope.

The common `value_envelope_version` field MUST use the shortest canonical
`vu128`. Decoders MUST reject truncation, overflow, reserved prefixes, and
overlong encodings before interpreting the version body. The v1
maximum encoded width is nine bytes.

`profile_byte` and `envelope_body` are v1 fields; they are not
version-independent envelope fields. A future value-envelope version MAY change
their width,
encoding, assigned identifiers, framing, transform order, authentication
inputs, or body grammar. A decoder MUST select the version-body grammar before
parsing any version-body bytes.

Version `0` is the **application-defined envelope version**. OpenKache does
not define, assign, or interpret its body grammar. An application MAY use
version `0` for a complete application-defined envelope profile, including its
selectors, body framing, transform order, authentication inputs, and limits,
when all communicating parties configure that profile out of band. Version `0`
has no OpenKache interoperability or compatibility guarantee. A client without
the matching application-defined profile MUST reject version `0` and MUST NOT
interpret it as the v1 grammar.

### 3.3 Packed profile byte

```text
bits 0..1 = protection_id
bits 2..3 = compression_id
bits 4..5 = payload_format_id
bits 6..7 = reserved_bits

profile_byte =
    protection_id
  | (compression_id << 2)
  | (payload_format_id << 4)
```

Encoders MUST emit only assigned IDs with zero `reserved_bits`. Decoders MUST
reject unassigned IDs and nonzero `reserved_bits`; they MUST NOT guess.

#### Protection IDs

The ID selects the complete algorithm profile. The profile name is the full
algorithm name; nonce and repetition behavior are normative properties of that
profile. Numeric IDs are wire assignments, not a security ranking.

| ID | Protection profile | Behavior |
|---:|---|---|
| `0` | `Unprotected` | No confidentiality or authentication; selected when no key is configured. |
| `1` | `AES-256-GCM-SIV` ([RFC 8452](https://www.rfc-editor.org/rfc/rfc8452)) | Fresh random nonce per write; default when protection is enabled. |
| `2` | `AES-256-SIV-CMAC` ([RFC 5297](https://www.rfc-editor.org/rfc/rfc5297)) | Deterministic and nonce-free; explicit opt-in. |

#### Compression IDs

| ID | Meaning |
|---:|---|
| `0` | Uncompressed |
| `1` | Zstandard |

#### Payload format IDs

| ID | Meaning |
|---:|---|
| `0` | `OpaqueBytes` |
| `1` | CBOR |
| `2` | `ApplicationDefined` (application-defined format) |
| `3` | Unassigned; reject |

### 3.4 Envelope body

```text
payload_bytes =
    opaque_bytes
  | cbor_payload
  | application_defined_payload

application_defined_payload =
    application_format_id:vu128 | application_payload

envelope_body = protect(compress(payload_bytes))
```

`payload_format_id` is in `profile_byte`, not in the payload. `OpaqueBytes`
has no terminator, sentinel, embedded length, or metadata header. A zero-length,
uncompressed, unprotected `OpaqueBytes` value is exactly:

```text
01 00
```

For `payload_format_id = 2` (`ApplicationDefined`), `application_format_id`
MUST be the shortest canonical `vu128`. It identifies an entry in the
configured application format registry; it is not a globally assigned
OpenKache format number. The registry maps the ID to the application payload
grammar and decoder. The remaining `application_payload` bytes belong to that
format and MAY be empty. Participants that exchange application-defined values
MUST configure the same ID-to-format mapping.
Unknown IDs, malformed `vu128` encodings, and payloads rejected by the
selected application format MUST be rejected.

An ID-to-format mapping MUST remain stable for the lifetime of values that use
it. A change to the application payload grammar MUST use a new
`application_format_id`; reassigning an existing ID is not a migration
mechanism.

The application format ID is part of `payload_bytes`, so selected compression
and cryptographic protection cover it without changing the v1 envelope grammar
or AAD. An application-defined format that needs different selectors, framing,
transforms, or AAD requires a new value-envelope version rather than a new
application format ID.

The payload format is independent from the key format. Its language-level
value mapping rules are not inherited from [Key Format](KEY_FORMAT.md).

### 3.5 CBOR acceptance profile

This is an acceptance profile, not a deterministic or canonical CBOR profile. It
defines which CBOR structures a decoder accepts; it does not require one
byte representation for one logical value. Applications that require exact
byte-for-byte reproducibility MUST use `OpaqueBytes` or another explicitly
defined payload format.

The CBOR payload MUST contain exactly one complete CBOR data item. A
decoder MUST consume the entire payload. A CBOR sequence, a second item after
the first item, or any other trailing bytes MUST be rejected.

Only definite-length encodings are supported. Indefinite-length arrays and
maps, and chunked indefinite-length byte strings and text strings, MUST be
rejected. This keeps the value boundary explicit and permits bounded parsing
without streaming or chunk-count rules.

Integer values MAY use any valid CBOR integer encoding supported by the
profile. Preferred serialization is not required, and a decoder MUST
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
unassigned tags MUST be rejected. Applications that need application-defined
typed payloads MUST use the `ApplicationDefined` payload format, `OpaqueBytes`,
or the opaque client API.

## 4. Compression

Compression ID `1` is one standard Zstandard (Zstd) frame under
[RFC 8878](https://www.rfc-editor.org/rfc/rfc8878). Encoders MUST use a
declared content size, no external dictionary, no skippable frame, and no
trailing bytes. Decoders MUST reject missing content sizes, multiple frames,
dictionary requirements, oversized windows, trailing bytes, and decompressed
output above the payload limit.

The initial SDK policy is Zstd level 1, no compression below 1,024 payload
bytes, and no compression unless it saves at least 64 bytes. An
encoder MAY select compression ID `0` when compression is not beneficial; it
MUST NOT label an uncompressed body as compression ID `1`.

When a value combines secret or otherwise sensitive material with
attacker-influenced data, compression SHOULD be disabled or the components
SHOULD be stored separately. Compression can create a side channel through
final ciphertext length; encryption does not hide that length.

## 5. Protection

### 5.1 Key material and AAD

Protected values derive an independent value root:

```text
value_protection_root_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 root key",
    material = client_root_key[32]
  )

per_item_key_material = value_protection_root_key[32] | item_id[32]
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
  | profile_byte
```

The version and profile byte are the exact bytes stored in the envelope. The
namespace is included separately in AAD even though it is also part of Item ID
derivation. AAD binds the ciphertext to its namespace, Item ID, profile,
payload format, and transforms; it does not provide freshness.

### 5.2 AES-256-GCM-SIV (ID 1)

```text
gcm_siv_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-GCM-SIV key",
    material = per_item_key_material[64]
  )

gcm_siv_payload = nonce[12] | ciphertext | tag[16]
```

The encoder MUST obtain a fresh 12-byte nonce from the operating-system
cryptographic random source for every write, including repeated writes. It
MUST fail if randomness fails and MUST NOT substitute a timestamp, Item ID,
plaintext, or process-local counter. Overhead is 28 bytes; the nonce is public.

### 5.3 AES-256-SIV-CMAC (ID 2)

Derive independent 32-byte AES-SIV-CMAC keys:

```text
siv_authentication_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-SIV-CMAC MAC key",
    material = per_item_key_material[64]
  )

siv_encryption_key =
  BLAKE3-DERIVE-KEY(
    context  = "OpenKache value format v1 AES-256-SIV-CMAC encryption key",
    material = per_item_key_material[64]
  )

siv_key_material =
  siv_authentication_key[32] | siv_encryption_key[32]
```

Pass the complete AAD as exactly one RFC 5297 associated-data component; do
not split it into multiple S2V components or add a nonce component.

```text
siv_payload = synthetic_iv[16] | ciphertext
```

Overhead is 16 bytes and there is no random nonce. Repeated identical writes
to one Item ID therefore repeat the stored bytes; per-item keys prevent
cross-Item-ID plaintext equality comparison.

### 5.4 Unprotected (ID 0)

The transformed body is stored directly. This profile provides no
confidentiality or authentication; parser bounds and compression validation
remain mandatory. Clients that require protection MUST reject it.

### 5.5 Size comparison

| Profile | Envelope header | Protection overhead | Zero-length uncompressed `OpaqueBytes` |
|---|---:|---:|---:|
| Unprotected | 2 bytes | 0 bytes | 2 bytes |
| AES-256-GCM-SIV | 2 bytes | 28 bytes | 30 bytes |
| AES-256-SIV-CMAC | 2 bytes | 16 bytes | 18 bytes |

AES-256-GCM-SIV is 12 bytes larger than AES-256-SIV-CMAC. Compression adds
its own frame overhead and may enlarge small values.

## 6. Processing requirements

### 6.1 Encoding

An encoder MUST:

1. Convert a structured source value using the selected payload format, or
   accept exact `OpaqueBytes`.
2. Encode the selected payload format (including the `application_format_id`
   prefix for the `ApplicationDefined` format) and enforce the expanded
   payload limit.
3. Apply compression only when policy permits.
4. Encode the selected value-envelope version and its version body. For v1,
   encode version `1` and the packed `profile_byte`.
5. Construct AAD from the exact namespace, Item ID, and header bytes.
6. Apply the selected protection profile.
7. Enforce the complete ValueEnvelope limit before sending.

For v1, the encoder MUST NOT emit a magic prefix, body length, unassigned ID,
non-canonical `vu128`, or a protection/compression/payload-format profile
inconsistent with the profile byte. An application-defined encoder owns the
corresponding rules for version `0`.

### 6.2 Decoding

A decoder MUST:

1. Parse one canonical `value_envelope_version:vu128` from the exact envelope
   slice.
2. Dispatch version `1` to the OpenKache v1 grammar. Dispatch version `0`
   only when a matching application-defined profile is explicitly configured;
   otherwise reject it.
3. Apply the selected version's version-body grammar. For v1, parse and
   validate `profile_byte`; an application-defined version uses its configured
   grammar.
4. Enforce the selected profile's protection policy.
5. Construct AAD from the selected profile's rules.
6. Check the selected profile's minimum body sizes before slicing.
7. Authenticate and decrypt before decompression or payload-format parsing.
8. Validate and bounded-decompress one Zstandard frame when selected.
9. For v1 `ApplicationDefined`, parse `application_format_id`, resolve the
   configured application format, and pass the remaining bytes to it. For v1
   CBOR, decode exactly one complete payload item. Reject unknown application
   format IDs, format-invalid payloads, and trailing bytes where the selected
   format disallows them. For v1 `OpaqueBytes`, return the exact payload. An
   application-defined profile owns the equivalent payload parsing rules.

Authentication failures MUST use one generic error. Unauthenticated plaintext
MUST be zeroized before returning that error.

### 6.3 Limits

- Complete stored `ValueEnvelope`: at most 64 MiB.
- Expanded payload: at most 64 MiB.

Implementations MAY use lower limits, but MUST check declared sizes, Zstandard
windows, produced output, and all arithmetic before allocation.

## 7. Versioning and compatibility

`value_envelope_version` selects the complete value-envelope grammar. It is
not a version of `profile_byte` or `envelope_body`: each version defines its
own version-body grammar, selector layout and assignments, body framing,
transform order, authentication inputs, and applicable limits. V1 readers
accept only `1`, reject unsupported versions, and never guess a version-body
grammar. The old
magic-prefixed envelope `4F 4B 56 01` is not v1. A default v1 reader treats
version `0` as unsupported; a reader with a matching application-defined
profile may dispatch it before applying the v1 grammar.

The `ApplicationDefined` payload format is an extension point inside v1. Its
`application_format_id` selects a registered payload grammar, but does not
change the v1 envelope grammar. An application-defined format that needs a
different envelope grammar MAY use the application-defined version `0` with an
out-of-band application-defined profile, or MUST use a future OpenKache-defined
version when cross-application interoperability is required. Version `0` is a
complete application-defined profile selector, not a wildcard or a partially
specified extension slot.

Value-envelope versioning is independent from key conversion and Item ID
derivation. The version is not an Item ID input. A client MAY store a different
value-envelope version under the same Item ID when the key profile is
unchanged and the client can read the selected versions. Changing the key
contract or root key remains a separate migration that changes Item IDs.

Future OpenKache profiles use additive profile evolution: newer profiles add
values or policies that older profiles cannot represent. A newer client SHOULD
use the oldest supported OpenKache profile that represents the selected value
and security policy at initialization. A reader MUST reject a version it does
not support rather than guessing its grammar. A default OpenKache v1 reader
supports version `1`; it supports version `0` only when a matching
application-defined profile is explicitly configured.

| Policy | Meaning |
|---|---|
| `Exact(vN)` | Require `vN`; fail if it cannot represent the client configuration. |
| `OldestRepresentable` | Default; choose the oldest complete profile that works. |
| `NewestSupported` | Explicit opt-in; may make older clients unable to read. |

## 8. Freshness and responsibility boundaries

V1 binds the item context and transform selection, not time or write sequence.
Valid older values can be replayed to the same Item ID. Generation/version and
server CAS are intentionally deferred; freshness MUST NOT be inferred from an
Item ID, nonce, synthetic IV, TTL, or namespace revision.

The protocol owns frame lengths, namespace IDs, Item IDs, and server-side
behavior. The common client core owns `vu128`, payload formats, compression,
protection, AAD, bounded parsing, and stable error categories. Language
bindings own language-level value mapping, memory ownership, runtime
integration, and error wrappers; they MUST NOT duplicate wire or cryptographic
logic.
