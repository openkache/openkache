# OpenKache Security Model (Draft)

> **Status:** Draft `draft-2026-08-19.4`; not released or finalized.
>
> This document answers a practical question: what can OpenKache protect, and
> what remains visible or outside its security boundary? It starts with
> user-visible goals, then defines the assumptions and technical rules needed
> to provide them.

The [Client Key Format](clients/KEY_FORMAT.md) and
[Client Value Format](clients/VALUE_FORMAT.md) define the bytes on the wire.
The [Wire Protocol](protocol/SPEC.md) defines
transport security and server operations. This document defines the security
goals, threat model, key assumptions, value-protection profiles, and
cryptographic details that connect those formats.

## At a glance

**Zero-trust server, end-to-end encrypted data.** When encryption is enabled,
the client encrypts data before sending it to OpenKache and decrypts it after
receiving it. The server stores and returns ciphertext and never receives the
encryption keys. TLS protects the connection; client-side encryption keeps the
plaintext outside the server's trust boundary.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) and
[RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when they appear in
uppercase.

## Security goals

For encrypted cache data, OpenKache aims to provide:

- **End-to-end encrypted data:** data remains unreadable to the cache service
  because it is encrypted before leaving the client and decrypted only by a
  client with the corresponding secret.
- **Tamper detection:** clients can detect when encrypted data has been changed
  or corrupted.

The non-goals, threat model, and key assumptions below define when these
properties apply. The remaining sections define the mechanisms used to provide
them.

## Non-goals and observable information

These profiles do not provide:

- freshness, anti-replay, or rollback detection;
- protection against deletion, withholding, or denial of service;
- integrity of server-side TTL, eviction, ordering, or mutation semantics;
- confidentiality of namespace IDs, Item IDs, value-key IDs, envelope lengths,
  compression choices, or access patterns;
- protection after the client host or keyring has been compromised; or
- confidentiality or integrity for an `Unprotected` value.

The server may therefore retain an older valid envelope, return it later, or
refuse to return a value. A successful authentication proves that the
envelope is valid for its authenticated context; it does not prove that the
envelope is the newest value.

## Threat model

This model considers every attacker category below. They may act independently
or together. The positive guarantees apply while the client trust boundary
remains uncompromised; client compromise is an explicit failure boundary.

### Server-side attacker

This category includes a server operator and anyone who can control the server
process, server configuration, storage, logs, or response path. They may:

- read stored envelopes and visible metadata;
- modify, delete, withhold, copy, or replay envelopes;
- move an envelope to another namespace or Item ID; and
- observe request timing, result timing, lengths, and access patterns.

For the server and storage guarantees, this attacker does not control the
client host, client process, or client keyring. If that boundary is also
compromised, the [Client compromise](#client-compromise) limitation applies.
Transport TLS does not protect against the legitimate server endpoint;
client-side value protection protects a protected payload from this category.

### Network attacker

This attacker may observe, delay, drop, alter, or replay bytes in transit.
TLS 1.3 with server-identity verification is required for protection against
endpoint impersonation. A connection that disables server-identity
verification does not provide that protection.

### Client compromise

If the client host, client process, application, value keyring, Item ID root
key, or TLS trust configuration is compromised, these profiles provide no
guarantee for data handled by that compromised boundary.

### Attacker-controlled input

An application-level attacker may choose some keys or value contents and
observe repeated results or envelope lengths. This category matters when
evaluating compression side channels and application-specific key isolation.

## Protection matrix

The table describes the intended properties of common configurations. “No”
means that the configuration intentionally does not provide that property;
“N/A” means that the configuration does not map an application key.

| Configuration | Application-key privacy | Value confidentiality | Value integrity and association |
|---|---:|---:|---:|
| `NamespaceHash` + secret root + protected value | Yes | Yes | Yes |
| `NamespaceHash` + public/default root + protected value | No | Yes | Yes |
| `PublicKeyOrHash` + protected value | No | Yes | Yes |
| Exact Item ID + protected value | N/A | Yes | Yes |
| Any address + `Unprotected` value | Address-dependent | No | No |

`PublicKeyOrHash` and Exact Item ID are deliberate escape hatches. They do not
claim application-key privacy. A protected value still has value
confidentiality and integrity when either address mode is used, provided the
value-key assumptions hold.

## Trust and key assumptions

- A value key is a 32-byte secret known to the clients that are allowed to
  read the corresponding values. The value key is not sent as part of the
  value envelope or wire operation.
- An Item ID root key is secret when application-key privacy is required.
  The all-zero root selects a publicly derivable mapping and does not provide
  that privacy.
- A positive `value_key_id` identifies one key mapping for its entire lifetime.
  It MUST NOT be rebound to different key material or reused after retirement.
- Secret keys MUST be generated and stored using an application-controlled
  cryptographically secure process.
- Maintained clients enable TLS server-identity verification by default.
  Disabling it is an explicit insecure choice and does not change the
  client-side value guarantees.

## Application-key protection

Application-key mapping is defined by
[clients/KEY_FORMAT.md](clients/KEY_FORMAT.md). Its
security properties are:

- **`NamespaceHash`:** may provide application-key privacy when its root key
  remains secret. The root key and namespace are part of the client-owned
  identity derivation, but the server sees only the resulting Item ID.
- **`PublicKeyOrHash`:** provides no application-key privacy. Short canonical
  key encodings are exposed as Item IDs, and longer encodings use a public
  unkeyed hash.
- **Exact Item ID:** accepts a caller-owned identity and makes no application
  key privacy claim.

Changing an Item ID root changes the address of mapped data. It is an identity
migration, not value-key rotation. Value-protection keys rotate independently.

## Value protection profiles

The value envelope selects one of these profiles:

| Profile | Construction | Body | Overhead |
|---|---|---|---:|
| `AES-256-GCM-SIV` | RFC 8452 | `nonce[12] \| ciphertext \| tag[16]` | 28 bytes |
| `AES-SIV-CMAC` | RFC 5297 with two independent AES-256 keys | `synthetic_iv[16] \| ciphertext` | 16 bytes |
| `Unprotected` | no cryptographic transform | transformed body | 0 bytes |

`AES-256-GCM-SIV` uses a fresh 12-byte OS-random nonce for every write;
nonce-generation failure rejects the write. Because the encryption key is
derived separately for each item, the RFC 8452 usage bound applies to each
derived item key. The security analysis assumes fewer than `2^32` writes to
one derived item key. This is an operational assumption, not an envelope
validity rule or a client-wide counter. Applications approaching that bound
for one item rotate to a new value-key ID.

`AES-SIV-CMAC` is deterministic. Equal key, authenticated data, and payload
inputs produce equal protected bodies, so this profile reveals equality.
It does not require nonce generation.

The transform order is:

```text
Protect(Compress(payload_bytes))
```

Authentication MUST complete before decompression or structured-value parsing.
An authentication failure MUST NOT return partially decrypted or decompressed
bytes.

## Key lifecycle

The enclosing client supplies a keyring that maps each positive
`value_key_id` to exactly one 32-byte value key. A protected write uses the
configured active ID. A protected read selects exactly the ID present in the
envelope; unknown and retired IDs fail without key probing or fallback.

An all-zero value key is invalid. Once an ID identifies key material, it MUST
NOT be rebound to different material or reused, including after retirement.

The operational read-old/write-new sequence is:

1. Add the new immutable key-ID mapping to every reader.
2. Change writers to the new active ID.
3. Keep previous mappings readable while their values may remain.
4. Retire a previous mapping only after its values have expired, been
   replaced, or been invalidated.

A client does not rewrite a value merely because it was read under an inactive
key. Such a rewrite is an ordinary mutation and may race with another writer.

## Key schedule

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
`vu128` bytes are used in the envelope and authenticated data. `namespace_id`
is the nonzero server identity; `item_id_length` is the exact `0..=32` byte
length, including zero.

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

`AES-SIV-CMAC` uses 32 bytes for the MAC key and 32 bytes for the encryption
key, for 64 bytes of total derived key material. The Item ID root key and
value-protection key are independent key classes. Rotating one MUST NOT
silently change the other.

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
policy, nonce, and envelope length are not authenticated-data inputs.

Changing namespace ID, Item ID length or bytes, selector, version, key ID, or
authenticated payload MUST cause authentication failure. The value-key ID is
public metadata and may reveal the key epoch.

Reading a protected value with an unknown or retired key ID fails with a
distinct key-unavailable category. It MUST NOT be reported as authentication
failure or trigger key probing.

## Compression and side channels

Compression can leak information when an attacker controls part of the
plaintext and can repeatedly observe resulting envelope lengths. Merely
storing one protected, compressed value does not by itself create a
compression oracle.

When secret data is compressed together with attacker-influenced data,
compression SHOULD be disabled or the components SHOULD be stored separately.
Cryptographic protection does not hide envelope length or other compression
dependent metadata.

Compression selection is a maintained-client policy in
[clients/CLIENT.md](clients/CLIENT.md#64-compression-policy), not a
cryptographic property of the protection profiles.

## Interoperability vectors

[`clients/fixtures/value_format_v1.json`](clients/fixtures/value_format_v1.json)
contains
complete AES-SIV-CMAC and AES-GCM-SIV vectors, including derived keys,
authenticated data, nonce, and envelope bytes. It covers an empty Item ID, a
three-byte Item ID, the multi-byte `vu128` encoding of key ID `128`, namespace
substitution, and tag alteration. Before freeze, at least two independent
implementations SHOULD reproduce every positive vector and reject every
negative vector.
