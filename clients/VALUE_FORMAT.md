# OpenKache Client Value Encoding Profile v1 (Draft)

> **Status:** Draft `draft-2026-08-19.4`; not released or finalized.

This document defines the client-side v1 value encoding before it is handed to
the server. The server stores the resulting bytes opaquely and does not
interpret payload formats, compression, or cryptographic protection.

Cryptographic behavior is defined only in the
[Security Model](../SECURITY_MODEL.md).

This is the target contract for the pre-freeze draft. Client implementations
may temporarily lag while the draft is being completed, but an implementation
MUST NOT claim conformance to this profile until it implements the complete
grammar, key schedule, and validation rules.

The shared implementation and local policies used by OpenKache-maintained
language bindings are described by the
[Client Implementation Guide](CLIENT.md). Binding-specific method names and
documented deviations belong in each binding's documentation.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) and
[RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when they appear in
uppercase.

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

The enclosing client supplies the namespace, resolved Item ID, and value
keyring. This document defines the envelope grammar and payload transforms.
Item ID derivation and value protection are separate contracts.

Structured values use the codec-neutral model and initial profile in
[`value/SPEC.md`](value/SPEC.md). This envelope document does not redefine
language-native numeric conversion, map equality, or lossless value views.

Addressing and value representation are independent:

| Axis | Choices | Meaning |
|---|---|---|
| Address | Mapped, Exact | Map a typed key through `KEY_FORMAT.md`, or use a caller-owned Item ID. |
| Value | Formatted v1, Raw, Caller-owned v0 | Build and interpret this envelope, preserve server value bytes, or pass through a version-0 envelope. |

Every address choice can be combined with every value choice. `Exact` bypasses
only key mapping. `Raw` bypasses only value encoding and decoding. The server
stores every choice as opaque bytes.

## 2. Processing model

```text
source value
  -> selected payload format
  -> payload bytes
  -> optional compression
  -> optional cryptographic protection
  -> ValueEnvelope
```

`OpaqueBytes` preserves the supplied payload before optional compression and
protection. `StructuredValue-CBOR-v1` accepts one item defined by
[`value/SPEC.md`](value/SPEC.md). The caller selects the payload format; the
encoder does not infer it from the bytes.

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
selected protection profile's body. Zero is invalid for a protected envelope
and MUST be rejected.
The key ID is public selection metadata, not secret key material. Its canonical
encoding is at most nine bytes.

`value_envelope_version = 0` is an opaque application-envelope escape hatch,
not a second OpenKache value grammar. OpenKache does not define or interpret
its body grammar, selectors, transform order, authentication inputs, or
application limits. Maintained clients expose it only through an explicit
caller-owned v0 representation.

The client passes complete version-0 bytes through as caller-owned data. It
MUST validate that the first canonical version field is exactly `0`, but MUST
NOT parse or interpret the remaining body. It MUST NOT apply version-1
transforms or rewrite the bytes. The outer protocol value-size limit still
applies, but version-1 expanded-payload, Zstandard, selector, keyring, and
cryptographic validation do not. The application owns all other version-0
validation and interpretation.

Versions other than `0` and `1` MUST be rejected. A structured-value decoder
MUST reject version `0` with a caller-owned-v0-required error rather than
returning a partially interpreted value. A Raw read returns stored bytes
without selecting a grammar. Any interpreting decoder MUST dispatch the
version before reading a version-body byte.

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

`protection_id` selects one profile defined by `SECURITY_MODEL.md`.

| ID | Protection profile |
|---:|---|
| `0` | `Unprotected` |
| `1` | `AES-256-GCM-SIV` |
| `2` | `AES-SIV-CMAC` |

Profile behavior and key selection are defined in
[SECURITY_MODEL.md](../SECURITY_MODEL.md). Maintained-client write defaults,
read allowlists, and per-operation overrides are defined in
[CLIENT.md](CLIENT.md).

### 4.2 Compression profiles

| ID | Compression profile |
|---:|---|
| `0` | Uncompressed |
| `1` | Zstandard |

### 4.3 Payload formats

| ID | Payload format |
|---:|---|
| `0` | `OpaqueBytes` |
| `1` | `StructuredValue-CBOR-v1` (initial profile: [`value/SPEC.md`](value/SPEC.md)) |

Only the payload format IDs listed above are supported by this profile. Any
other payload format ID MUST be rejected. Payload format ID `1` is specifically
the CBOR v1 profile; it is not a generic alias for every future structured
codec. A future OpenKache-defined structured codec MUST use a new payload
format ID or a new envelope version and MUST NOT reinterpret ID `1`. This
profile has no in-band application-format registry. Applications that need
another format MUST select a supported profile or encode it as `OpaqueBytes`
and agree on its interpretation out of band.

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

### 5.2 StructuredValue-CBOR-v1

The structured payload MUST conform to the initial codec profile in
[`value/SPEC.md`](value/SPEC.md). That specification defines the complete
CBOR item boundary, allowed encodings, numeric behavior, map-key equality, and
duplicate-key rejection.

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
[Client Implementation Guide](CLIENT.md#64-compression-policy) for maintained
bindings, not an envelope validity rule. An encoder MUST accurately identify
the emitted body as either `Uncompressed` or `Zstandard`.

When secret data is compressed together with attacker-influenced data,
compression SHOULD be disabled or the components SHOULD be stored separately.
Compression can create a side channel through the resulting ciphertext length;
cryptographic protection does not hide that length.

## 7. Cryptographic protection

The selector and envelope grammar in this document select one of the
protection profiles defined in
[SECURITY_MODEL.md](../SECURITY_MODEL.md).
That document is the sole normative source for value-key selection, KDF
inputs, AAD, cryptographic constructions, protection overhead, and security
properties. This document intentionally keeps no duplicate key schedule or
cryptographic test vectors.

For implementation and interoperability work, use the security document and
the public [client fixtures](fixtures/README.md). The transform order remains:

```text
envelope_body = Protect(Compress(payload_bytes))
```

Authentication MUST complete before decompression or structured-value parsing.
An authentication failure MUST NOT return partially decrypted or decompressed
bytes. `Unprotected` is wire-valid but provides neither confidentiality nor
authentication. Compression-dependent length leakage remains observable for
protected values.

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
2. Dispatch the version before parsing its version body. Version `0` returns
   through the caller-owned v0 path and has no v1 body validation.
3. Parse and validate the selector byte.
4. Enforce the caller's expected protection policy.
5. For a protected profile, parse one canonical nonzero value-key ID, look up
   exactly that key, and derive the item-specific protection key or keys.
6. Construct the authenticated data from the selected profile's rules.
7. Check minimum protected-body sizes before slicing.
8. Authenticate and decrypt before decompression or payload parsing.
9. Validate and bounded-decompress one Zstandard frame when selected.
10. Decode exactly one structured-value item using the selected profile, or
    return the exact `OpaqueBytes` payload.

Unknown versions other than caller-owned version `0`, selector values,
value-key IDs, compression profiles, payload formats, malformed payloads, and
disallowed trailing bytes MUST be rejected. The decoder MUST NOT probe other
keys, silently downgrade, fall back, or reinterpret an unknown value as
another payload format. Version `0` is returned through the caller-owned
pass-through path without parsing.

## 9. Limits and rejection rules

```text
MAX_VALUE_ENVELOPE_BYTES = 67,108,864  // 64 MiB
MAX_EXPANDED_PAYLOAD_BYTES = 67,108,864
MAX_ZSTD_WINDOW_BYTES = 67,108,864
```

These are per-value limits. Implementations SHOULD also configure an aggregate
in-flight byte budget for concurrent encode/decode operations; that operational
budget is not part of the wire format. OpenKache-maintained clients require
that budget as specified in [`CLIENT.md`](CLIENT.md#67-resource-budget).

`MAX_VALUE_ENVELOPE_BYTES` applies to the complete byte string stored and
returned opaquely by the server, including version, selector, value-key ID,
nonce or synthetic IV, ciphertext, authentication tag, and any Zstandard frame
overhead. It equals the wire protocol's maximum `SET` value and response
payload.

`MAX_EXPANDED_PAYLOAD_BYTES` applies after decryption and decompression but
before structured-value parsing. `MAX_ZSTD_WINDOW_BYTES` applies to the
declared decoder window. A Zstandard decoder MUST independently enforce:

```text
declared_content_size <= MAX_EXPANDED_PAYLOAD_BYTES
declared_window_size  <= MAX_ZSTD_WINDOW_BYTES
produced_size         <= MAX_EXPANDED_PAYLOAD_BYTES
produced_size         == declared_content_size
```

Satisfying one check does not waive another.

The two 64 MiB limits are independent: a value is valid only when both its
expanded payload and complete envelope satisfy their limits. Protection and
envelope metadata consume part of the envelope limit, so the maximum logical
payload can be smaller than 64 MiB.

Implementations MAY use lower local limits, but MUST check the complete
envelope, declared Zstandard content size and window size, produced output, and
all arithmetic before allocation. A decoder MUST reject a frame with no
declared content size, more than one frame, any dictionary ID, a skippable
frame, trailing bytes, a declared or produced size above its limit, or a
produced size different from the declared content size.

Malformed `vu128` encodings, truncated headers, zero or unknown protected
value-key IDs, nonzero selector zero bits, unsupported selector values,
structured-value violations, invalid Zstandard frames, authentication
failures, and payloads exceeding limits MUST be rejected.

## 10. Versioning and extension

`value_envelope_version` selects a complete envelope grammar. It is not a
version of the selector byte or envelope body. A future OpenKache-defined
version MAY change selector layout and assignments, body framing, transform
order, key-selection framing, authentication inputs, and limits.

This profile uses `value_envelope_version = 1`. A reader MUST reject a version
other than `0` or `1` rather than guessing its grammar. Version `0` is
available only through the explicit caller-owned v0 representation for a
complete application-defined envelope profile configured out of band; the
maintained client passes it through without interpreting it.

Future OpenKache versions are additive: a newer version may represent values or
policies that this profile cannot. Version selection is independent of payload
bytes.
