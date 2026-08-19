# OpenKache Value Security Profiles (Draft)

> **Status:** Draft `draft-2026-08-19.4`; not released or finalized.
>
> This document owns the v1 value-key schedule, associated data, cryptographic
> profiles, and security properties. Envelope grammar and selector assignment
> remain in [`VALUE_FORMAT.md`](VALUE_FORMAT.md).

## Ownership boundary

`VALUE_FORMAT.md` defines how an envelope selects protection and compression.
This document defines what those protection profiles mean. The server stores
the resulting bytes opaquely and never interprets these fields.

## Key schedule

The enclosing client profile supplies a keyring mapping each positive unsigned
64-bit `value_key_id` to one 32-byte value key. A protected write uses the
configured active ID. A protected read selects exactly the ID present in the
envelope; unknown and retired IDs fail without key probing or fallback.
Once an ID identifies key material, it MUST NOT be rebound to different key
material. Retired IDs are never reused.

An all-zero value key is invalid. Secret keys MUST be generated independently
with a cryptographically secure random source.

For every protected envelope:

```text
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
```

`value_key_id` uses fixed-width `u64be` in the KDF. The exact canonical
`vu128` bytes are used in the envelope and AAD. `namespace_id` is the
nonzero server identity; `item_id_length` is the exact `0..=32` byte length,
including zero.

The profile-specific keys are derived with these exact BLAKE3 contexts:

```text
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

## Authenticated data

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

The version and key-ID bytes MUST be the exact canonical bytes emitted in the
envelope. Request ID, lane, opcode, namespace name or revision, TTL, eviction
policy, nonce, and envelope length are not AAD inputs.

## Protection profiles

| Profile | Construction | Body | Overhead |
|---|---|---|---:|
| `AES-256-GCM-SIV` | RFC 8452 | `nonce[12] \| ciphertext \| tag[16]` | 28 bytes |
| `AES-SIV-CMAC` | RFC 5297 with two independent AES-256 keys | `synthetic_iv[16] \| ciphertext` | 16 bytes |
| `Unprotected` | no cryptographic transform | transformed body | 0 bytes |

`AES-SIV-CMAC` uses 32 bytes for the MAC key and 32 bytes for the encryption
key, for 64 bytes of total derived key material. GCM-SIV MUST use a fresh
12-byte OS-random nonce for every write; nonce-generation failure rejects the
write. Because `gcm_siv_key` is derived from the value-key ID, namespace ID,
and Item ID, RFC 8452 usage bounds apply independently to each derived item
key, not to the root value key across all items. With random nonces and this
profile's 64 MiB value ceiling, the security analysis assumes fewer than
`2^32` writes per derived item key. This is an operational security bound, not
an envelope-validity rule or a client-wide counter. Applications approaching
that bound for one item rotate to a new value-key ID. SIV-CMAC is deterministic
and therefore reveals equality for equal key, AAD, and payload inputs.

The transform order is:

```text
Protect(Compress(payload_bytes))
```

Authentication MUST complete before decompression or structured-value parsing.
Authentication failures use one generic error and MUST NOT return partially
decrypted or decompressed bytes.

## Security properties

Protected profiles provide confidentiality and authentication only when the
value key is secret. They do not provide freshness, replay protection, TTL
integrity, eviction integrity, ordering, or availability. Envelope length and
compression-dependent length leakage remain observable.

Changing namespace ID, Item ID length or bytes, selector, version, key ID, or
authenticated payload MUST cause authentication failure. The value-key ID is
public metadata and may reveal the key epoch.

Reading a protected value with an unknown or retired key ID fails with a
distinct key-unavailable category; it MUST NOT be reported as authentication
failure or trigger key probing.

## Interoperability vectors

[`fixtures/value_format_v1.json`](fixtures/value_format_v1.json) contains
complete AES-SIV-CMAC and AES-GCM-SIV vectors, including derived keys, AAD,
nonce, and envelope bytes. It covers an empty Item ID, a three-byte Item ID,
the multi-byte `vu128` encoding of key ID `128`, namespace substitution, and
tag alteration. Before freeze, at least two independent implementations SHOULD
reproduce every positive vector and reject every negative vector.
