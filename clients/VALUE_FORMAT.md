# OpenKache Client Value Encoding Profile v1 (Draft)

> **Status: Draft — pre-freeze**

This document defines the client-side v1 value encoding before it is handed to
the server. The server stores the resulting bytes opaquely and does not
interpret payload formats, compression, or cryptographic protection.

The cryptographic key schedule, associated data, protection constructions, and
security properties are summarized here for envelope interop and defined in
the dedicated [Value Security Profiles](VALUE_SECURITY.md). That document is
the source of truth for cryptographic details.

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

Structured values use the codec-neutral model and initial profile in
[`value/SPEC.md`](value/SPEC.md). This envelope document does not redefine
language-native numeric conversion, map equality, or lossless value views.

Address selection and value representation are independent client concerns.
The address is either a mapped typed key or an already resolved exact
`0..=32`-byte Item ID. The value representation is either raw bytes, a
caller-owned version-0 envelope, or this version-1 envelope. A server never
interprets either representation.

Maintained clients MAY expose only safe combinations in ergonomic APIs, but
the shared core contract MUST permit an exact Item ID to carry a v1 envelope.
The conceptual operation families are distinct:

```text
set(mapped_key, value)
set_exact(item_id, raw_bytes)
set_exact_formatted(item_id, structured_value)
put_raw_v0_envelope(address, complete_bytes)
```

The exact Item ID API remains caller-owned and provides no isolation or key
derivation. Its being suitable for benchmarks does not prevent the caller from
selecting a formatted value representation when explicitly requested.

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
bytes. `StructuredValue-CBOR-v1` accepts one value-profile data item under the rules in
[`value/SPEC.md`](value/SPEC.md). `Json` is an API convenience type, not a v1
payload selector: it is serialized as canonical RFC 8785 UTF-8 and carried
using `OpaqueBytes` (selector `0`). The payload format is selected by the
caller; the encoding does not infer a format from the payload bytes.

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
application limits. The maintained client exposes it only through an explicit
`put_raw_v0_envelope` operation:

```text
put_raw_v0_envelope(address, complete_bytes)
get_raw_bytes(address) -> complete_bytes
```

The client passes complete version-0 bytes through as caller-owned data. It
MUST validate that the first canonical version field is exactly `0`, but MUST
NOT parse or interpret the remaining body. It MUST NOT apply version-1
transforms or rewrite the bytes. The outer protocol value-size limit still
applies, but version-1
expanded-payload, Zstandard, selector, keyring, and cryptographic validation
do not. Validation and interpretation of version `0` belong to the
application or to its separately configured profile.

Versions other than `0` and `1` MUST be rejected. A structured-value decoder
MUST reject version `0` with a raw-envelope-required error rather than
returning a partially interpreted value. `get_raw_bytes` returns stored bytes
without selecting a grammar. A decoder that is asked to interpret a value
MUST dispatch version `0` to the caller-owned pass-through path or select the
version-1 grammar before interpreting any version-body byte.

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
| `2` | `AES-SIV-CMAC` ([RFC 5297](https://www.rfc-editor.org/rfc/rfc5297); two AES-256 keys) | Deterministic authenticated encryption with no random nonce. |

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
  `AES-SIV-CMAC`, but not `Unprotected`;
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
protection profiles defined in [VALUE_SECURITY.md](VALUE_SECURITY.md).
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
   through the `get_raw_bytes` path and has no v1 body validation.
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

`MAX_VALUE_ENVELOPE_BYTES` applies to the complete byte string stored and
returned opaquely by the server, including version, selector, value-key ID,
nonce or synthetic IV, ciphertext, authentication tag, and any Zstandard frame
overhead. It equals the wire protocol's maximum `SET` value and response
payload.

`MAX_EXPANDED_PAYLOAD_BYTES` applies to the payload after decryption and
decompression but before structured-value parsing. `MAX_ZSTD_WINDOW_BYTES` is
the largest declared decoder window. All three limits are independent:
satisfying one does not waive either of the others.

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
available only through the explicit `put_raw_v0_envelope` API for a complete
application-defined envelope profile configured out of band; the maintained
client passes it through without interpreting it.

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
