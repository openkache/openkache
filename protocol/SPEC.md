# OpenKache Wire Protocol Version 1

## Status

This document is the normative specification for OpenKache wire protocol
version 1. An implementation conforms to version 1 only when its transport,
framing, validation, operation behavior, and outcome rules satisfy this
document.

Client-owned formatted values are specified separately by the
[OpenKache value format](../clients/VALUE_FORMAT.md).

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) and
[RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when they appear in
uppercase.

## Scope

Version 1 specifies:

- QUIC application-protocol negotiation;
- the request/response stream state machine;
- operation-specific request frame layouts;
- canonical unsigned `vu128` integers;
- response frame layout;
- opcode, flag, and status assignments;
- namespace lifecycle, name, policy, and revision contracts;
- item ID, value, expiration, eviction, and payload constraints;
- malformed-frame handling, early rejection, and retry ambiguity;
- mutation error outcomes and persistence-barrier semantics.

Client-side application-key derivation, serialization, compression,
application-level encryption, and value containers are outside this protocol
and belong to the value-format specification. Storage layout and the namespace
eviction algorithm are outside this protocol. Item expiration and eviction
eligibility are part of the `SET` contract below. Namespace lifecycle and
policy administration are carried by the namespace-management requests defined
below.

## Terminology

- **Octet**: An 8-bit byte.
- **Connection**: One QUIC connection negotiated for OpenKache protocol v1.
- **Lane**: One client-initiated bidirectional QUIC stream.
- **Frame**: One complete request or response encoded as specified below.
- **Item ID**: The exact 32-octet identifier used for cache equality.
- **Account**: A deployment-defined authenticated identity. Version 1 does not
  make an account the owner or scope of a namespace.
- **Namespace**: A named server-wide collection of Item IDs with default
  expiration and eviction policies. A namespace is not nested under an
  account.
- **Namespace ID**: The server-assigned positive 64-bit identity of a
  namespace used in wire frames.
- **Namespace revision**: The positive 64-bit version of a namespace's
  policy, used for optimistic concurrency on policy updates and deletion.
- **Value**: An uninterpreted sequence of octets stored for an item ID.
- **Payload**: The uninterpreted response body. Its operation-specific meaning
  is defined by this document.
- **Canonical `vu128`**: The unique encoding selected by the unsigned 64-bit
  rules in this document.
- **Expiration mode**: The item-level choice among inheriting a namespace
  default, never expiring by TTL, or using an explicit TTL.
- **Eviction mode**: The item-level choice among inheriting a namespace
  default, being eligible for capacity eviction, or being protected from
  capacity eviction.
- **Eviction algorithm**: The namespace-level selection algorithm (for example,
  LRU or LFU) applied only to items whose eviction policy is `Evictable`.
- **Mutation linearization point**: The instant at which a `SET`, `DELETE`, or
  namespace mutation takes effect atomically.
- **SYNC linearization point**: The instant at which a `SYNC` operation fixes
  the set of preceding mutations covered by its persistence barrier.

All lengths count octets, not characters or code points. Hexadecimal octets are
written as two uppercase digits, such as `7F` or `E0`.

## Transport and version negotiation

Protocol v1 runs over QUIC and therefore uses TLS 1.3 for transport security.
The exact ALPN protocol identifier is the 11-octet ASCII string:

```text
openkache/1
```

A client that supports only v1 MUST offer `openkache/1`. A client that supports
multiple protocol versions MAY offer multiple ALPN identifiers in descending
preference, with the highest version first. A server MUST select the highest
version that it supports and that the client offered. A server MUST NOT select
an older version when a mutually supported newer version was offered.

Every client has a configured minimum acceptable protocol version. A client
MUST abort the connection if the negotiated ALPN is below that minimum. A
client MUST NOT silently lower its minimum in response to a negotiation
failure. Explicit fallback to a lower version is a deployment choice and MUST
be configured by the application.

TLS authenticates the negotiated ALPN as part of the handshake transcript. The
minimum-version rule protects a client from an authenticated endpoint that
deliberately selects an older protocol.

The ALPN negotiation selects the connection's frame version. Frames contain no
version field. Once v1 negotiation succeeds, every OpenKache frame on the
connection uses this specification. A framing or field meaning that is
incompatible with this document requires a different ALPN identifier. An
implementation MUST NOT use an older common-header layout with `openkache/1`.

Peers without a common ALPN identifier MUST fail negotiation.

Authentication policy is deployment-specific. Production deployments may
require mutual TLS and may use the authenticated client identity to authorize
administrative operations. No authentication field appears in a v1 frame.

## Stream model

Only client-initiated bidirectional QUIC streams carry protocol frames.
Unidirectional streams have no protocol v1 meaning.

QUIC stream read and write boundaries have no protocol meaning. A frame MAY be
split across any number of reads or writes, and one read MAY contain bytes from
more than one frame.

Each lane follows this state machine:

```text
client                                  server
   |                                      |
   |-------------- request -------------->|
   |          no second request            |
   |<------------- response ---------------|
   |                                      |
   |-------------- request -------------->|  ...
```

The following rules apply:

1. A client MUST send exactly one complete request before waiting for its
   response.
2. A client MUST NOT send a second request, or any bytes of a second request,
   before the response for the first request has been received.
3. A server MUST send exactly one response for each request it accepts, unless
   the lane or connection fails before a response can be sent.
4. A server MUST NOT send unsolicited responses.
5. After a complete response, the lane returns to the request state and MAY be
   reused if both stream directions remain open.
6. A client that finishes its send direction after a request MUST NOT expect
   that lane to be reusable. The server MAY still send the one response.
7. A client MAY use multiple lanes concurrently. Ordering exists only within
   one lane.

If a server can determine a request error from a prefix before the complete
request body arrives, it MAY send one error response immediately. After that
response, the lane is terminal: the server MUST close or reset the lane, and
the client MUST stop transmitting that request and MUST NOT reuse the lane.

Version 1 has no request identifier because lane order provides correlation.
It also has no deduplication token. If a lane fails after a mutating request is
sent but before its response is received, the client cannot determine from the
protocol alone whether the mutation took effect.

## Unsigned `vu128`

Every variable-width integer in this specification uses the unsigned 64-bit
subset of [`vu128`](https://github.com/jmillikin/rust-vu128). This section is
self-contained; implementations do not need that library.

`vu128` stores low-order value bits first. Encodings from one through four
octets place low-order bits in the first octet after a unary length prefix.
Encodings from five through nine octets use the first octet only as a length
prefix and store the value in little-endian order in the remaining octets.

| Encoded octets | Canonical value range | First octet | Value reconstruction |
|---:|---:|---|---|
| 1 | `0` through `2^7 - 1` | `0xxxxxxx` | `b0` |
| 2 | `2^7` through `2^14 - 1` | `10xxxxxx` | `(b0 & 0x3F) \| (b1 << 6)` |
| 3 | `2^14` through `2^21 - 1` | `110xxxxx` | `(b0 & 0x1F) \| (b1 << 5) \| (b2 << 13)` |
| 4 | `2^21` through `2^28 - 1` | `1110xxxx` | `(b0 & 0x0F) \| (b1 << 4) \| (b2 << 12) \| (b3 << 20)` |
| 5 | `2^28` through `2^32 - 1` | `F3` | little-endian `b1..b4` |
| 6 | `2^32` through `2^40 - 1` | `F4` | little-endian `b1..b5` |
| 7 | `2^40` through `2^48 - 1` | `F5` | little-endian `b1..b6` |
| 8 | `2^48` through `2^56 - 1` | `F6` | little-endian `b1..b7` |
| 9 | `2^56` through `2^64 - 1` | `F7` | little-endian `b1..b8` |

For a first octet of at least `F0`, the encoded length is
`(first_octet & 0x0F) + 2`. Prefixes `F0`, `F1`, and `F2` are not emitted by
the canonical unsigned 64-bit encoding; values they can represent use one of
the compact prefix forms in no more octets. Prefixes `F8` through `FF` require
more than nine octets and exceed the unsigned 64-bit range.

A sender MUST emit the unique canonical encoding in the table. A receiver MUST
decode the value, re-encode it according to the table, and reject the input
unless the octets are identical. This rejects:

- compact or length-prefix alternatives such as `F0`, `F1`, and `F2`;
- a value encoded with more octets than its canonical representation;
- a first octet from `F8` through `FF`;
- a truncated encoding;
- any decoded value that exceeds a field-specific limit.

The following boundary vectors are normative:

| Value | Canonical encoding |
|---:|---|
| `0` | `00` |
| `127` | `7F` |
| `128` | `80 02` |
| `16,383` | `BF FF` |
| `16,384` | `C0 00 02` |
| `2^21 - 1` | `DF FF FF` |
| `2^21` | `E0 00 00 02` |
| `2^28 - 1` | `EF FF FF FF` |
| `2^28` | `F3 00 00 00 10` |
| `2^32 - 1` | `F3 FF FF FF FF` |
| `2^32` | `F4 00 00 00 00 01` |
| `2^40` | `F5 00 00 00 00 00 01` |
| `2^48` | `F6 00 00 00 00 00 00 01` |
| `2^56` | `F7 00 00 00 00 00 00 00 01` |
| `5,000` | `88 4E` |
| `67,108,864` (64 MiB) | `E0 00 00 40` |
| `2^64 - 1` | `F7 FF FF FF FF FF FF FF FF` |

For example, `81 00` decodes numerically to `1` but is invalid because `01` is
the canonical encoding.

## Common limits

| Field | Limit |
|---|---:|
| Namespace ID | exactly 8 octets; numeric value `1..=2^64 - 1` |
| Namespace name | `0..=255` UTF-8 octets; zero is a valid empty name |
| Item ID | exactly 32 octets when present |
| `SET` request value | `0..=67,108,864` octets |
| Response payload | `0..=67,108,864` octets |
| `vu128` integer | `0..=2^64 - 1` |
| TTL | `1..=2^64 - 1` milliseconds |

The 64 MiB value and payload limit is a wire ceiling. A server MAY configure a
smaller operational item limit. A request within the wire ceiling but above
the server limit receives `TooLarge`, and the server MUST reject it before
applying a mutation.

The largest valid `SET` request is 67,108,924 octets: an opcode, an
eight-octet `namespace_id`, one flags octet, a 32-octet Item ID, a nine-octet
TTL, a nine-octet maximum `value_len`, and a 64 MiB value. The largest valid
`NAMESPACE_OPEN` request is 268 octets: an opcode, two flag/length octets, a
255-octet name, and a ten-octet maximum namespace policy. The
`MAX_REQUEST_FRAME_BYTES` receive limit is the larger of those operation
limits. The largest valid response is 67,108,874 octets: one status octet, a
nine-octet maximum `payload_len`, and a 64 MiB payload.

## Request frames

The request layout is selected by `opcode`. There is no common request
`flags`, `item_id_len`, or `value_len` field.

```text
request = ping | get | set | delete | stats | sync |
          namespace_open | namespace_update_policy | namespace_delete

ping                     = opcode:01
get                      = opcode:02 | namespace_id:u64be | item_id:32
set                      = opcode:03 | namespace_id:u64be | set_flags:u8 |
                           item_id:32 | [ttl_ms:vu128] |
                           value_len:vu128 | value:value_len
delete                   = opcode:04 | namespace_id:u64be | item_id:32
stats                    = opcode:05 | namespace_id:u64be
sync                     = opcode:06 | namespace_id:u64be
namespace_open           = opcode:07 | open_flags:u8 | name_len:u8 |
                           name:name_len | [namespace_policy]
namespace_update_policy  = opcode:08 | namespace_id:u64be |
                           expected_revision:u64be | namespace_policy
namespace_delete         = opcode:09 | delete_flags:u8 | namespace_id:u64be |
                           expected_revision:u64be
```

`value_len` appears only in `SET`, including when the value is empty. It is
encoded immediately before the value, after the fixed Item ID and optional TTL.
A receiver can therefore reject an oversized value before allocating or
reading any value body after reading only the bounded request metadata.
`namespace_id` is present in every namespace-scoped request and is always
encoded before the operation-specific fields. The Item ID is always exactly 32
octets for operations that carry one. Namespace-management requests use the
fixed-width `name_len:u8` and `expected_revision:u64be` fields defined below.

`u64be` means one fixed eight-octet unsigned integer in network byte order
(most-significant octet first). It is not a `vu128` field and has no alternate
or shorter encoding.

Every other opcode is unassigned. A server receiving an unassigned opcode
MUST respond with `UnsupportedOpcode` when it can send a response. Because an
unassigned opcode has no defined body layout, the server MUST NOT scan for a
possible next frame; it MUST terminate the lane after the error response.

### Opcodes

| Opcode | Name | Request layout | Value |
|---:|---|---|---|
| `01` | `PING` | opcode only | no Item ID, no value |
| `02` | `GET` | opcode + namespace ID + Item ID | no value |
| `03` | `SET` | opcode + namespace ID + flags + Item ID + optional TTL + value length + value | value is `0..=64 MiB` |
| `04` | `DELETE` | opcode + namespace ID + Item ID | no value |
| `05` | `STATS` | opcode + namespace ID | no Item ID, no value |
| `06` | `SYNC` | opcode + namespace ID | no Item ID, no value |
| `07` | `NAMESPACE_OPEN` | opcode + flags + name length + name + optional policy | namespace descriptor |
| `08` | `NAMESPACE_UPDATE_POLICY` | opcode + namespace ID + expected revision + policy | namespace descriptor |
| `09` | `NAMESPACE_DELETE` | opcode + flags + namespace ID + expected revision | no value |

### `SET` flags

`SET` carries one flags octet containing three independent two-bit selections.
`NAMESPACE_OPEN` and `NAMESPACE_DELETE` also carry operation-specific flags;
their layouts are defined in [Namespace management flags and
revisions](#namespace-management-flags-and-revisions). Other request layouts
have no flags octet.

| Bits | Mask | Values |
|---:|---:|---|
| 0–1 | `03` | `00` = `Any`; `01` = `IfAbsent`; `10` = `IfPresent`; `11` = reserved |
| 2–3 | `0C` | `00` = `Inherit`; `01` = `NoExpiry`; `10` = `ExplicitTtl`; `11` = reserved |
| 4–5 | `30` | `00` = `Inherit`; `01` = `Evictable`; `10` = `EvictionProtected`; `11` = reserved |
| 6–7 | `C0` | Reserved; MUST be zero |

The expiration selection controls the presence of `ttl_ms`: `ExplicitTtl`
requires one, while `Inherit` and `NoExpiry` omit it. A receiver MUST reject a
`SET` with any reserved value or reserved bit set. Policy conflicts are
described in [Namespace policy](#namespace-policy).

### Namespace

The public API addresses a namespace by its server-wide name. Namespace
management uses the same request/response protocol as data operations; it does
not require a separate control-plane transport. A client MAY obtain a namespace
handle and use it for all operations:

```text
cache = client.namespace("cache")       # resolves or opens by name
cache.set(item_id, value, options)
```

`namespace_id` is a fixed eight-octet `u64be` in the numeric range
`1..=2^64 - 1`. The server assigns it; it is an opaque, stable server-wide
identity. `0` is reserved and MUST be rejected. The ID is carried per request
rather than bound to a lane, so a lane may be reused for different namespaces
and retries can be encoded deterministically.

The wire protocol has no default namespace concept. An SDK MAY expose
`client.set(item_id, value, options)` as a convenience shorthand, but it MUST
resolve a configured namespace name through `NAMESPACE_OPEN` and then send the
returned non-zero ID. A namespace ID of zero is never a selector.

The `NAMESPACE_OPEN` name field has a one-octet length:

```text
name_len:u8 | name:name_len
```

`name_len = 0` is the valid empty namespace name and carries no name octets.
It may be used with either value of `CreateIfMissing`.

For any namespace name, `name_len` MUST be `0..=255` and `name` MUST be valid
UTF-8. The length is the UTF-8 octet count, not the number of Unicode scalar
values. Names are compared by their exact UTF-8 octets, are case-sensitive,
and are not case-folded or Unicode-normalized. The wire protocol imposes no
path, shell, or cloud-provider naming profile. The empty name is an ordinary
namespace name. Namespace names are unique within one server.

Namespace ownership and authorization are deployment concerns, not namespace
identity rules. A deployment MAY designate an administrative owner and MAY
grant other authenticated accounts access through an ACL, but those identities
and grants are not fields in a v1 frame. For every account authorized to use a
namespace, the same name resolves to the same server-wide namespace ID and the
same `(namespace_id, item_id)` data. Sharing access never creates an
account-local namespace or changes the namespace name or ID.

These rules are protocol rules, not cloud-provider resource-name rules. An SDK
MAY offer an additional cloud-portable validator (for example, lowercase ASCII
`a-z0-9-` with a 3–63 octet limit), but the wire protocol does not require that
narrower profile.

`GET`, `SET`, `DELETE`, `STATS`, and `SYNC` are namespace-scoped and carry a
`namespace_id`. `PING` is connection-scoped and carries none. `NAMESPACE_OPEN`
resolves a name to a namespace descriptor and can create a missing named
namespace. `NAMESPACE_UPDATE_POLICY` changes a namespace policy with an
optimistic-concurrency check. `NAMESPACE_DELETE` deletes an empty named
namespace with an optimistic-concurrency check. Any namespace, including the
empty-name namespace, may be deleted when it is empty.

An item is identified by the pair `(namespace_id, item_id)`. The same 32-octet
Item ID in two namespaces denotes two independent items.

### Namespace management flags and revisions

`NAMESPACE_OPEN` has this flag layout:

| Bit | Mask | Meaning |
|---:|---:|---|
| 0 | `01` | `CreateIfMissing` |
| 1–7 | `FE` | Reserved; MUST be zero |

When `CreateIfMissing` is clear, the request contains no
`namespace_policy` and only resolves an existing namespace. When it is set,
the request MUST contain one `namespace_policy`. If the named namespace
already exists, the supplied policy is validated but MUST NOT overwrite the
existing policy. A newly created namespace starts at revision `1`.

`NAMESPACE_DELETE` has this flag layout:

| Bits | Mask | Values |
|---:|---:|---|
| 0–1 | `03` | `00` = `IfEmpty`; `01`–`11` = reserved |
| 2–7 | `FC` | Reserved; MUST be zero |

Version 1 supports only `IfEmpty`. The namespace is considered empty at the
delete linearization point when it contains no logically present item; expired
items do not prevent deletion. A live item causes `NamespaceNotEmpty` and no
deletion. Force deletion and asynchronous purge are not defined by v1.

`revision` and `expected_revision` are fixed eight-octet `u64be` values, not
`vu128` fields. A namespace revision is positive, starts at `1`, and increases
by exactly one for every successful policy update. `NAMESPACE_UPDATE_POLICY`
and `NAMESPACE_DELETE` require a non-zero `expected_revision` equal to the
current revision. A mismatch returns `Conflict` and makes no change. Revision
values MUST NOT wrap; an update that would overflow the revision range fails
without changing the policy.

The public operations map to the wire requests as follows:

```text
namespace.resolve(name)
    -> NAMESPACE_OPEN with CreateIfMissing clear

namespace.open_or_create(name, policy)
    -> NAMESPACE_OPEN with CreateIfMissing set (empty name is allowed)

namespace.update_policy(id, expected_revision, policy)
    -> NAMESPACE_UPDATE_POLICY

namespace.delete_if_empty(id, expected_revision)
    -> NAMESPACE_DELETE with delete mode IfEmpty
```

`NAMESPACE_OPEN` returns `Ok` with the existing namespace descriptor, or
`Created` with the newly created descriptor. A missing namespace without
`CreateIfMissing` returns `NamespaceNotFound`. `NAMESPACE_UPDATE_POLICY`
returns `Ok` with the descriptor containing the incremented revision.
`NAMESPACE_DELETE` returns `Deleted` with an empty payload on success.

### Namespace policy

A namespace has a default expiration policy, a default eviction policy, and an
independent rule for whether each default may be overridden by an item request.

```rust
enum ExpirationMode {
    Inherit,       // use the namespace default
    NoExpiry,      // no TTL-based expiration
    ExplicitTtl,   // SET carries ttl_ms
}

enum EvictionMode {
    Inherit,            // use the namespace default
    Evictable,          // eligible for the namespace eviction algorithm
    EvictionProtected,  // never selected for capacity eviction
}

enum OverridePolicy {
    Allowed,
    Disallowed,
}

enum ExpirationDefault {
    NoExpiry,
    FixedTtl { ttl_ms: u64 },
}

enum EvictionDefault {
    Evictable,
    EvictionProtected,
}

enum Condition {
    Any,
    IfAbsent,
    IfPresent,
}

struct SetOptions {
    condition: Condition,
    expiration_mode: ExpirationMode,
    ttl_ms: Option<u64>, // required iff expiration_mode == ExplicitTtl
    eviction_mode: EvictionMode,
}

struct NamespacePolicy {
    default_expiration: ExpirationDefault,
    expiration_override: OverridePolicy,
    default_eviction: EvictionDefault,
    eviction_override: OverridePolicy,
}
```

The wire encoding of `NamespacePolicy` is:

```text
namespace_policy = policy_flags:u8 | [default_ttl_ms:vu128]
```

The `policy_flags` octet has this layout:

| Bits | Mask | Values |
|---:|---:|---|
| 0–1 | `03` | `00` = `NoExpiry`; `01` = `FixedTtl`; `10`–`11` = reserved |
| 2 | `04` | `0` = expiration `Disallowed`; `1` = expiration `Allowed` |
| 3 | `08` | `0` = default `Evictable`; `1` = default `EvictionProtected` |
| 4 | `10` | `0` = eviction `Disallowed`; `1` = eviction `Allowed` |
| 5–7 | `E0` | Reserved; MUST be zero |

`default_ttl_ms` is present exactly when the default expiration bits select
`FixedTtl`. It is a canonical positive `vu128` count of milliseconds and MUST
be absent for `NoExpiry`. A receiver MUST reject a policy with a reserved bit,
a reserved default value, an unexpected TTL field, or a zero TTL.

Namespace descriptors returned by `NAMESPACE_OPEN` and
`NAMESPACE_UPDATE_POLICY` have this payload layout:

```text
namespace_descriptor = namespace_id:u64be |
                       revision:u64be |
                       namespace_policy
```

`ExpirationDefault` is either `NoExpiry` or a positive fixed TTL. Its fixed
TTL is encoded in the namespace configuration, not in a request. `EvictionDefault`
is either `Evictable` or `EvictionProtected`; neither namespace default may be
`Inherit`.

`SET` carries the item-level `ExpirationMode` and `EvictionMode` selections
in its flags. `Inherit` resolves to the namespace default. An explicit item
selection is accepted only when the corresponding namespace
`OverridePolicy` is `Allowed`; otherwise the server returns `PolicyConflict`
and makes no mutation. A successful `SET` resolves both policies at its
mutation linearization point and stores the resolved item metadata. Later
namespace policy changes do not retroactively change existing items.

Namespace policy changes are future-write-only. In particular, changing an
override from `Allowed` to `Disallowed` does not rewrite or invalidate existing
items that were stored with an explicit policy. It only rejects future `SET`
requests that select that explicit override. An inherited policy is resolved
against the namespace policy current at the `SET` linearization point.

At the public API, `ttl_ms` MUST be present exactly when
`expiration_mode == ExplicitTtl` and MUST be positive. It MUST be absent for
`Inherit` and `NoExpiry`; a client MUST reject that invalid combination before
encoding a frame.

`EvictionProtected` protects an item from capacity eviction only. It does not
prevent expiration, explicit `DELETE`, or replacement. The namespace's
eviction algorithm chooses only among resolved `Evictable` items. If a write
cannot be admitted without evicting a protected item, the server returns
`NoCapacity` and makes no mutation.

Each successful replacement applies the policies resolved from that `SET`.
Thus, `Inherit` on a replacement uses the current namespace defaults; a client
that must retain protection MUST request `EvictionProtected` explicitly.

### Item ID

An Item ID is exactly 32 opaque octets. Every 32-octet sequence is a valid Item
ID, including an all-zero sequence. The wire protocol does not define an
application key, an application-key validity rule, or how an application key
becomes an Item ID. Clients may use raw 32-octet identifiers, a digest, a keyed
derivation, or another application policy.

Servers MUST compare Item IDs by their complete 32-octet identity. `PING`,
`STATS`, and `SYNC` carry no Item ID. `GET`, `SET`, and `DELETE` carry exactly
32 Item ID octets after their namespace ID.

### Value

The `SET` value is exactly `value_len` opaque octets. Empty `SET` values are
valid. A server MUST NOT interpret any value prefix or maintain protocol
metadata for:

- serialization format;
- compression state or algorithm;
- application-level encryption state or algorithm;
- client envelope version.

A successful `GET` MUST return the same value octets accepted by `SET`, unless
the item was subsequently replaced, deleted, expired, or evicted.

### TTL

`ttl_ms` exists only when the `SET` expiration-policy bits select
`ExplicitTtl`. It follows the complete 32-octet Item ID and precedes
`value_len`. It is a canonical unsigned `vu128` count of milliseconds. A
`NoExpiry` or `Inherit` selection has no TTL field; an inherited fixed TTL comes
from the namespace policy.

Zero is invalid. A server MUST reject a TTL that cannot be converted into its
supported monotonic absolute-time range. For `ExplicitTtl`, the TTL deadline is
calculated from the `SET` mutation linearization point, not from connection
receipt time or value-read start time:

```text
deadline = mutation_linearization_time + ttl_ms
```

For an inherited namespace `FixedTtl`, the same calculation uses the fixed TTL
from the namespace policy. `NoExpiry` has no TTL deadline.

An item is logically absent when the server's monotonic time satisfies
`now >= deadline`. Expired items therefore produce `NotFound` for `GET` and
`DELETE`, satisfy `IfAbsent`, and do not satisfy `IfPresent`.

If a successful mutation expires before its response is delivered, the server
still reports the mutation's success outcome. If a conditional `SET` fails,
its TTL is not applied.

## Operation semantics

### `PING`

`PING` has the one-octet request `01`.

The success response is `Ok` with exactly the four ASCII octets `PONG`.

### `GET`

`GET` has the request layout `02 | namespace_id:u64be | item_id:32`.

- Found: `Ok` with the exact opaque value as payload.
- Missing, expired, deleted, or evicted: `NotFound` with an empty payload.

### `SET`

`SET` has the request layout
`03 | namespace_id:u64be | set_flags | item_id | [ttl_ms] | value_len | value`.

- Stored over no live item: `Created` with an empty payload.
- Stored over a live item: `Replaced` with an empty payload.
- Condition not satisfied: `NotStored` with an empty payload, with no change
  to the existing item.

`Any` is unconditional. `IfAbsent` succeeds only when the item is logically
absent. `IfPresent` succeeds only when the item is logically present. Condition
evaluation and the mutation MUST be atomic with respect to that namespace and
Item ID.

### `DELETE`

`DELETE` has the request layout `04 | namespace_id:u64be | item_id:32`.

- Live item removed: `Deleted` with an empty payload.
- Missing, expired, already deleted, or evicted: `NotFound` with an empty
  payload.

### `STATS`

`STATS` has the request layout `05 | namespace_id:u64be`.

- Authorized success: `Ok` with a UTF-8 JSON object containing a `storage`
  string and a `workers` array of strings.
- Unauthorized: `Forbidden` with an optional diagnostic payload.

The JSON object is a point-in-time diagnostic snapshot for the requested
namespace. Clients MUST ignore unknown object members so diagnostics can grow
without changing the frame protocol. The JSON payload remains subject to the
64 MiB response limit.

### `SYNC`

`SYNC` has the request layout `06 | namespace_id:u64be`.

`SYNC` is a namespace persistence barrier. Its linearization point fixes the
set of mutations for that namespace covered by the barrier. A successful
response is sent only after all mutations in that namespace whose mutation
linearization points precede that `SYNC` linearization point have completed the
server's configured persistence operation across all storage workers. Mutations
linearized after that point need not be included.

- Authorized success: `Ok` with an empty payload, sent only after the barrier
  completes.
- Unauthorized: `Forbidden` with an optional diagnostic payload.

Protocol v1 does not express selectable durability levels. The storage and
deployment durability contract is outside the frame protocol.

### `NAMESPACE_OPEN`

`NAMESPACE_OPEN` has the request layout:

```text
07 | open_flags:u8 | name_len:u8 | name:name_len | [namespace_policy]
```

`name_len = 0` resolves the empty-name namespace. A non-empty name resolves the
matching server-wide namespace. With `CreateIfMissing` set, an absent name
(including the empty name) is created using the supplied policy.
Existing names are never overwritten by `CreateIfMissing`.

- Existing namespace: `Ok` with a `namespace_descriptor` payload.
- Newly created namespace: `Created` with a `namespace_descriptor` payload.
- Missing name without `CreateIfMissing`: `NamespaceNotFound` with an empty
  payload.

The returned descriptor contains the server-assigned ID, current revision, and
effective namespace policy. The response does not repeat the name because the
client already supplied it.

### `NAMESPACE_UPDATE_POLICY`

`NAMESPACE_UPDATE_POLICY` has the request layout:

```text
08 | namespace_id:u64be | expected_revision:u64be | namespace_policy
```

The server checks namespace existence and `expected_revision`, then atomically
replaces the namespace policy and increments the revision. Authorization is
deployment-specific because v1 has no owner or account field. A successful
response is `Ok` with the updated `namespace_descriptor` payload. A revision
mismatch returns `Conflict` and makes no policy change.

Policy changes apply only to future `SET` operations. Existing items retain the
expiration deadline and resolved eviction policy that were stored when they
were written.

### `NAMESPACE_DELETE`

`NAMESPACE_DELETE` has the request layout:

```text
09 | delete_flags:u8 | namespace_id:u64be | expected_revision:u64be
```

The only valid v1 `delete_flags` value is `00` (`IfEmpty`). The server checks
the revision and atomically tests whether the namespace has any logically
present items. Authorization is deployment-specific because v1 has no owner or
account field. A successful deletion returns `Deleted` with an empty payload.
The namespace is deleted when it is empty; there is no reserved default
namespace exception in v1.

## Response frames

Every response has this layout:

```text
+------------+---------------------+------------------------+
| status:u8  | payload_len:vu128   | payload:payload_len    |
+------------+---------------------+------------------------+
```

In compact notation:

```text
response = status | payload_len | payload
```

`payload_len` is present for every response, including responses with an empty
payload. Responses have no version, request identifier, flags, Item ID, or TTL.

### Status codes

| Status | Name | Meaning |
|---:|---|---|
| `00` | `Ok` | Operation succeeded and may carry a payload |
| `01` | `NotFound` | The requested live item does not exist |
| `02` | `Created` | `SET` created a logical item or `NAMESPACE_OPEN` created a namespace |
| `03` | `Replaced` | `SET` replaced a live item |
| `04` | `Deleted` | `DELETE` removed a live item or `NAMESPACE_DELETE` removed a namespace |
| `05` | `NotStored` | A conditional `SET` made no change |
| `80` | `InvalidRequest` | Request framing, namespace ID, flags, lengths, TTL, or semantics are invalid |
| `81` | `UnsupportedOpcode` | The opcode is not assigned in v1 |
| `82` | `TooLarge` | A declared or actual item exceeds a wire or server limit |
| `83` | `Overloaded` | The server temporarily lacks admission capacity |
| `84` | `Timeout` | Reading, admission, execution, or response preparation timed out |
| `85` | `Forbidden` | The authenticated identity is not authorized |
| `86` | `InternalError` | The server could not complete the operation |
| `87` | `NoCapacity` | The write cannot be admitted without evicting protected items |
| `88` | `PolicyConflict` | The request selects an item policy disallowed by the namespace |
| `89` | `Conflict` | `expected_revision` does not match the current namespace revision |
| `8A` | `NamespaceNotFound` | The requested namespace does not exist |
| `8B` | `NamespaceNotEmpty` | `IfEmpty` deletion found a logically present item |

Statuses `06` through `7F` and `8C` through `FF` are unassigned. A client MUST
treat an unassigned status as a malformed response and discard the lane.

Assigned statuses `80` and above are errors. Unassigned status values in that
range are not implicitly accepted as errors; they remain malformed. Error
payloads MAY be empty or MAY contain an operator-facing diagnostic. If present,
the diagnostic SHOULD be UTF-8. Diagnostic text is not a stable programmatic
interface; clients MUST branch on the status octet rather than parsing error
text.

For a mutating operation (`SET`, `DELETE`, `SYNC`, namespace creation, policy
update, or namespace deletion), an error response MUST guarantee that the
mutation or persistence barrier did not take effect. If the server cannot
establish that guarantee, it MUST close the lane without an error response,
leaving the operation outcome ambiguous.

## Response contract by request

For a valid request, the following are the domain success and result statuses:

| Request | Allowed domain statuses | Payload |
|---|---|---|
| `PING` | `Ok` | Exactly `PONG` |
| `GET` | `Ok`, `NotFound` | Hit: exact value; miss: empty |
| `SET` | `Created`, `Replaced`, `NotStored` | Always empty |
| `DELETE` | `Deleted`, `NotFound` | Always empty |
| `STATS` | `Ok`, `Forbidden` | `Ok`: required JSON object; `Forbidden`: optional diagnostic |
| `SYNC` | `Ok`, `Forbidden` | `Ok`: empty; `Forbidden`: optional diagnostic |
| `NAMESPACE_OPEN` | `Ok`, `Created` | Namespace descriptor |
| `NAMESPACE_UPDATE_POLICY` | `Ok` | Updated namespace descriptor |
| `NAMESPACE_DELETE` | `Deleted` | Always empty |

Common error statuses MAY be returned when their stated condition applies. A
client receiving a status that is neither an allowed domain status nor an
applicable common error for the outstanding request MUST treat the response as
malformed and discard the lane.

`PolicyConflict` and `NoCapacity` apply to `SET`. `Conflict` applies to
`NAMESPACE_UPDATE_POLICY` and `NAMESPACE_DELETE`. `NamespaceNotFound` applies
to namespace operations that address a missing namespace. `NamespaceNotEmpty`
applies to `NAMESPACE_DELETE`. These errors guarantee that the requested
mutation was not applied.

## Validation and malformed frames

A conforming receiver MUST validate, in order where practical:

1. the opcode or status assignment;
2. the presence and fixed eight-octet encoding of a namespace ID for
   namespace-scoped requests;
3. the numeric namespace ID range;
4. fixed-width namespace flags, name length, and revision fields;
5. complete and canonical `vu128` fields;
6. the presence and value of operation flags;
7. field-specific length and UTF-8 limits;
8. the operation-specific fixed layout;
9. the complete 32-octet Item ID when present;
10. TTL presence, canonical encoding, and positive value;
11. namespace-policy encoding and item-policy override rules;
12. the exact remaining `SET` value length;
13. the response status/payload combination.

For a request, a receiver parses the following prefix before reading a `SET`
value body:

```text
opcode
[namespace_id:u64be]
[set_flags]
[item_id]
[ttl_ms]
[value_len]
```

For namespace-management requests, the bounded prefix is:

```text
NAMESPACE_OPEN:
    opcode | open_flags | name_len | name | [namespace_policy]
NAMESPACE_UPDATE_POLICY:
    opcode | namespace_id | expected_revision | namespace_policy
NAMESPACE_DELETE:
    opcode | delete_flags | namespace_id | expected_revision
```

The brackets indicate fields present only for `SET` or
`NAMESPACE_OPEN` with `CreateIfMissing`. The namespace ID and revision occupy
eight octets whenever present. `name_len = 0` is a valid empty name; every
name must satisfy the UTF-8 and name rules above.

A receiver MUST enforce the 64 MiB value ceiling and any smaller server limit
before allocating or reading the value body. A declared value above either
limit maps to `TooLarge` when the server can send a response.

Receiving end-of-stream before a frame is complete is a truncated-frame error.
A receiver MUST NOT scan for a possible next frame after malformed framing.

When a server can respond to a malformed request:

- an unknown opcode maps to `UnsupportedOpcode`;
- a zero or otherwise invalid `namespace_id` maps to `InvalidRequest`;
- a value above the 64 MiB wire ceiling or server limit maps to `TooLarge`;
- a disallowed item-policy override maps to `PolicyConflict`;
- a capacity failure caused by protected items maps to `NoCapacity`;
- a revision mismatch maps to `Conflict`;
- a missing namespace maps to `NamespaceNotFound`;
- a non-empty namespace deletion maps to `NamespaceNotEmpty`;
- other protocol validation failures map to `InvalidRequest`.

The server MAY send that error response before the request body is complete
when the error is established by the parsed prefix. It MUST send no success
response in that case, and it MUST close or reset the lane after the error
response. The client MUST stop writing and MUST discard the lane.

If a server cannot determine an error until it reads the body, it waits for the
declared body length. A body shorter than the declared length is truncated. A
client that sends bytes belonging to another request before receiving the
response violates the lane state; the server MUST terminate the lane rather
than interpreting those bytes as a second in-flight request.

After a framing error, the server SHOULD send one error response and close the
lane because the next frame boundary may be ambiguous. The QUIC connection and
other lanes MAY remain usable. A transport failure may prevent the error
response.

After a malformed response, a client MUST discard the lane. It MAY keep the
connection and use other lanes.

## Retry and outcome rules

The protocol provides no replay protection or mutation identifier.

- `PING`, `GET`, and `STATS` are safe to retry after reconnecting.
- A client SHOULD NOT automatically replay `SET`, `DELETE`, or `SYNC` after a
  transport failure that occurs before a response is received.
- `NAMESPACE_OPEN` without `CreateIfMissing` is safe to retry. With
  `CreateIfMissing`, retrying is state-safe but the response may change from
  `Created` to `Ok`; clients that need to distinguish those outcomes must
  treat a lost response as ambiguous.
- A client SHOULD NOT automatically replay `NAMESPACE_UPDATE_POLICY` or
  `NAMESPACE_DELETE` after a transport failure before a response is received.
- A received mutation error response guarantees no mutation or barrier
  completion and MAY be retried by application policy.
- `Created`, `Replaced`, `Deleted`, and `NotStored` are successful domain
  outcomes, not transport errors.

If a mutating operation may have taken effect but the server cannot send its
success response, the server closes the lane without a definitive error
status. The client must treat the outcome as ambiguous.

Applications that require stronger mutation retry semantics must provide them
above protocol v1.

## Security and resource handling

QUIC protects frames in transit. Opaque values are not automatically
confidential from the server or from storage; application-level value
encryption remains a client concern.

Receivers MUST parse lengths incrementally and enforce the 64 MiB ceiling before
allocating or reading a complete `SET` value or response payload. Servers
SHOULD bound aggregate in-flight value memory and MAY reject or time out
requests under resource pressure.

Canonical integer enforcement is security-relevant: it prevents multiple wire
representations of one logical frame and simplifies bounded incremental
parsing.

## Version evolution

Protocol v1 reserves all unassigned opcodes, statuses, and flag bits. Senders
MUST NOT use them, and receivers MUST reject them as described above.

Any change that reinterprets an existing field, changes frame order, adds or
removes mandatory fields, changes canonical integer encoding, or changes the
meaning of existing assignments requires a new ALPN identifier. New protocol
versions MUST NOT reuse `openkache/1` for incompatible frames.

When a client supports multiple versions, it MUST use the ALPN ordering and
minimum-version rules in the transport section. A server MUST select the
highest mutually supported version.

## Conformance examples

### `PING`

Request:

```text
01
```

Response:

```text
00 04 50 4F 4E 47
```

This is `Ok`, `payload_len = 4`, and ASCII `PONG`.

### `GET` miss

For namespace ID `7` and an Item ID containing 32 `AA` octets:

```text
02 00 00 00 00 00 00 00 07 [AA × 32]
```

A miss response is:

```text
01 00
```

### Conditional `SET` with TTL

For namespace ID `7`, an Item ID containing 32 `11` octets, `IfAbsent`, an
explicit 5,000 millisecond TTL, `EvictionProtected`, and the ASCII value
`value`:

```text
03 00 00 00 00 00 00 00 07 29 [11 × 32] 88 4E 05 76 61 6C 75 65
```

- `03`: `SET`
- `00 00 00 00 00 00 00 07`: namespace ID 7 (`u64be`)
- `29`: `IfAbsent` + `ExplicitTtl` + `EvictionProtected`
- `[11 × 32]`: exact 32-octet Item ID
- `88 4E`: canonical `vu128` encoding of 5,000
- `05`: five-octet value

A created response is:

```text
02 00
```

### Unconditional empty `SET`

For namespace ID `7` and an Item ID containing 32 `22` octets:

```text
03 00 00 00 00 00 00 00 07 00 [22 × 32] 00
```

This is an unconditional `SET` inheriting both namespace policies, with no TTL
field and an empty value.

### `DELETE`, `STATS`, and `SYNC`

```text
04 00 00 00 00 00 00 00 07 [item_id × 32]  # DELETE
05 00 00 00 00 00 00 00 07                 # STATS
06 00 00 00 00 00 00 00 07                 # SYNC
```

### Namespace management

Resolve the empty-name namespace:

```text
07 00 00
```

This is `NAMESPACE_OPEN` with `open_flags = 00` and `name_len = 00`. It has no
name or policy bytes; the empty name is the namespace being resolved.

Create or open the named namespace `cache`:

```text
07 01 05 63 61 63 68 65 [namespace_policy]
```

`01` sets `CreateIfMissing`; `05` is the UTF-8 octet length of `cache`. The
policy bytes are required when the create flag is set, even if the namespace
already exists; an existing policy is not overwritten.

Update a namespace policy:

```text
08 [namespace_id:u64be] [expected_revision:u64be] [namespace_policy]
```

Delete an empty namespace at an expected revision:

```text
09 00 [namespace_id:u64be] [expected_revision:u64be]
```

The `00` is the only valid v1 `delete_flags` value.

## Implementation conformance checklist

A protocol v1 implementation is not complete unless it:

- negotiates `openkache/1` for these frames;
- supports the documented multi-version ALPN selection and minimum-version
  rules when it supports more than one protocol version;
- emits and accepts no frame-level version byte;
- uses client-initiated bidirectional lanes in request/response lockstep;
- derives request layout from the opcode;
- carries a positive server-assigned eight-octet `u64be` `namespace_id` on
  every namespace-scoped request;
- supports `NAMESPACE_OPEN`, `NAMESPACE_UPDATE_POLICY`, and
  `NAMESPACE_DELETE` on the same protocol lanes;
- treats `name_len = 0` as a valid empty namespace name and validates all
  UTF-8 names as specified;
- uses a one-octet namespace name length and enforces the 255-octet ceiling;
- starts namespace revisions at one and enforces
  `expected_revision` on policy updates and deletion;
- encodes namespace policies and descriptors exactly as specified;
- accepts only `IfEmpty` for the v1 namespace-delete flags;
- omits `item_id_len` from every request;
- includes `value_len` only in `SET`;
- validates operation-specific flags for `SET`, `NAMESPACE_OPEN`, and
  `NAMESPACE_DELETE`;
- encodes `Any`/`IfAbsent`/`IfPresent`, `ExpirationMode`, and `EvictionMode` as
  specified in the `SET` flags;
- rejects non-canonical, truncated, wider-than-`u64`, and overflowing `vu128`;
- validates the fixed 32-octet Item ID shape;
- validates expiration-mode/TTL correspondence before reading a large
  value;
- computes TTL from the mutation linearization point using a monotonic clock;
- treats `now >= deadline` as expired;
- resolves inherited namespace policy at `SET` linearization time;
- enforces namespace override rules and returns `PolicyConflict` without a
  mutation;
- never evicts an item resolved as `EvictionProtected`;
- returns `NoCapacity` rather than evicting a protected item when admission
  cannot proceed;
- keeps compression and application-encryption metadata out of frames;
- preserves all value octets without interpretation;
- rejects reserved `SET` flag bits and unassigned status values;
- enforces the 64 MiB wire ceiling before unbounded allocation or body reads;
- permits early error responses for prefix-detectable failures and then
  terminates the lane;
- guarantees that a mutating error response means no mutation or barrier
  completion;
- discards a lane after framing or response-status meaning becomes ambiguous;
- treats mutation outcomes as ambiguous when transport fails before a response;
- implements `SYNC` as the documented namespace persistence barrier.

## Reference

The `vu128` encoding was designed by John Millikin and is documented by the
[`rust-vu128` project](https://github.com/jmillikin/rust-vu128). This
specification uses only its canonical unsigned 64-bit encoding.
