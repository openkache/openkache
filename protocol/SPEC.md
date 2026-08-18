# OpenKache Wire Protocol v1 (Draft)

## Status

This document is the draft design specification for OpenKache wire protocol
version 1. Version 1 has not been released or finalized. The requirements
below describe the current intended wire contract and may change before
finalization. Within this draft, an implementation conforms only when its
transport, framing, validation, operation behavior, and outcome rules satisfy
this document.

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
- request-ID correlation, stream ordering, and out-of-order responses;
- malformed-frame handling and admission rejection;
- mutation error outcomes and persistence-barrier semantics.

Client-side application-key derivation, serialization, compression,
application-level encryption, and value containers are outside this protocol
and belong to the [client key](../clients/KEY_FORMAT.md) and
[value-format](../clients/VALUE_FORMAT.md) specifications. The shared
implementation choices used by OpenKache-maintained language bindings are
described by the [Client Implementation Guide](../clients/CLIENT.md); they are
not additional wire requirements for third-party clients. The physical storage
layout and the namespace eviction algorithm are outside this wire protocol.
Item expiration and eviction eligibility are part of the `SET` contract below.
Namespace lifecycle and policy administration are carried by the
namespace-management requests defined below.

## Terminology

- **Byte**: Exactly 8 bits.
- **Connection**: One QUIC connection negotiated for OpenKache protocol v1.
- **Lane**: One client-initiated bidirectional QUIC stream.
- **Frame**: One complete request or response encoded as specified below.
- **Logical request**: One request frame and its correlated response frame.
- **Request ID**: A client-selected canonical `vu128` token carried in a
  request and echoed in its response. The server treats its value as opaque.
- **Stream order**: The order in which complete request frames occur on one
  lane. It is independent of request-ID values and response order.
- **In-flight request**: A complete request that has not yet received its
  response.
- **Item ID**: An opaque identifier of `0..=32` bytes used for cache equality.
- **Account**: A deployment-defined authenticated identity. Version 1 does not
  make an account the owner or scope of a namespace.
- **Namespace**: A named server-wide collection of Item IDs with default
  expiration and eviction policies. A namespace is not nested under an
  account.
- **Namespace ID**: The server-assigned positive 64-bit identity of a
  namespace used in wire frames.
- **Namespace revision**: The positive 64-bit version of a namespace's
  policy, used for optimistic concurrency on policy updates and deletion.
- **Value**: An uninterpreted sequence of bytes stored for an item ID.
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
- **Unknown outcome**: The client cannot determine from the protocol whether a
  mutation or `SYNC` took effect because no response was received.

All lengths count bytes, not characters or code points. Hexadecimal bytes are
written as two uppercase digits, such as `7F` or `E0`.

## Transport and version negotiation

Protocol v1 runs over QUIC and therefore uses TLS 1.3 for transport security.
The exact ALPN protocol identifier is the 11-byte ASCII string:

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
connection uses this specification. During the pre-freeze draft period,
implementations using the provisional `openkache/1` identifier MUST coordinate
the same specification revision out of band. An implementation claiming this
revision MUST NOT use an older common-header layout. After v1 is finalized, an
incompatible framing or field meaning requires a different ALPN identifier.

Peers without a common ALPN identifier MUST fail negotiation.

Authentication policy is deployment-specific. Production deployments may
require mutual TLS and may use the authenticated client identity to authorize
administrative operations. No authentication field appears in a v1 frame.

## Stream model

Only client-initiated bidirectional QUIC streams carry protocol frames.
Server-initiated unidirectional streams have no protocol meaning and MUST be
consumed and discarded by the client without parsing protocol frames. The
server MUST NOT initiate a bidirectional protocol stream; a client that
receives one MUST reset it without parsing protocol frames.

QUIC stream read and write boundaries have no protocol meaning. A frame MAY be
split across any number of reads or writes, and one read MAY contain bytes from
more than one frame.

Version 1 supports request pipelining (multiple outstanding requests) and
request/response multiplexing within one lane. Each lane carries a sequence of
logical requests. A request and its response share the request ID:

```text
client                                  server
   |                                      |
   |-- request(id=A) -------------------->|
   |-- request(id=B) -------------------->|
   |<-- response(id=B) -------------------|  (may complete first)
   |<-- response(id=A) -------------------|
   |                                      |
   |-------------- more requests -------->|  ...
```

The following rules apply:

1. A client MAY send multiple complete requests without waiting for earlier
   responses.
2. A request ID is a canonical `vu128` value in the range
   `0..=2^64 - 1`. Zero is valid. The client chooses the value; the server
   MUST echo its canonical bytes and MUST NOT assign ordering, uniqueness,
   deduplication, replay-protection, or idempotency meaning to it.
3. The client owns request-ID allocation and MAY reuse an ID after receiving
   its response. The wire protocol imposes no server-side request-ID
   uniqueness rule; duplicate IDs do not make an otherwise well-formed frame
   malformed. The client contract requires lane-local uniqueness while a
   request is outstanding so a multiplexed client can correlate out-of-order
   responses. That client policy does not add a server validation rule.
4. Each complete request frame receives an internal sequence position in its
   lane. The server MUST produce results equivalent to serial execution in
   that sequence order. It MAY execute non-conflicting requests concurrently,
   but condition checks, mutations, and other externally visible effects MUST
   remain indistinguishable from that serial order. Request-ID values MUST NOT
   be used as an ordering key. For example, a `DELETE` followed by a `SET` for
   the same item on one lane MUST take effect as delete-then-set even if the
   `SET` response is sent first.
5. The server MAY send responses in any order relative to stream order. For
   every complete, well-formed request it admits, it MUST send exactly one
   response, including an admission or semantic error, unless the lane or
   connection fails before a response can be sent. A malformed frame is not a
   parsed request and receives no response.
6. Response frames MUST be emitted as contiguous byte sequences. Response
   bytes from two frames MUST NOT be interleaved.
7. A server MUST NOT send unsolicited responses.
8. After a response, the lane MAY continue carrying requests while both stream
   directions remain open. A client that finishes its send direction MUST NOT
   send another request on that lane; the server continues responses for
   already admitted requests and then finishes its send direction.
9. A client MAY use multiple lanes concurrently. Requests on different lanes
   have no client-visible relative ordering guarantee, except where an
   operation explicitly establishes a namespace-scoped barrier such as
   `SYNC`, namespace policy update, or namespace deletion.

10. A client FIN is valid only at a request-frame boundary. After receiving a
    valid FIN, the server MUST admit no further requests on that lane, MUST
    complete responses for requests already admitted, and MUST then finish its
    send direction. A FIN that arrives in the middle of a frame is malformed.
11. QUIC stream cancellation is directional:
    - A client `RESET_STREAM` terminates the request direction. The server
      MUST admit no further requests, MUST complete responses for requests
      already admitted, and MUST then finish its response direction.
    - A server `RESET_STREAM` terminates the response direction. The client
      MUST send no further requests on that lane. Outstanding requests without
      responses have an `unknown` outcome, and no further protocol response is
      guaranteed.
    - A client `STOP_SENDING` asks the server to stop the response direction.
      The server MUST stop sending protocol responses, and the client MUST
      send no further requests on that lane.
    - A server `STOP_SENDING` asks the client to stop the request direction.
      The client MUST stop sending requests, and the server MUST complete
      responses for requests already admitted.
    A response-direction reset or stop makes every outstanding mutation or
    `SYNC` outcome unknown. The protocol does not prescribe replay. If
    cancellation interrupts a frame, the receiver MUST discard the incomplete
    frame; this is not malformed framing, and other lanes remain usable.

If a receiver detects malformed framing, it MUST stop processing the
connection and close it with application error code `0x01`
(`MALFORMED_FRAME`). It MUST NOT scan for a possible next frame, and it MUST
NOT send an error response for the malformed frame. All lanes on that
connection become unusable. A client MUST discard all in-flight requests on
every lane that terminates this way. A complete frame whose fields are
well-delimited but fail operation validation is not malformed framing; it MAY
receive the applicable error response.

`0x01` (`MALFORMED_FRAME`) is the QUIC application error code for a
connection-fatal framing or response-meaning failure. It does not carry a
protocol response frame.

The request ID is a correlation token only. It is not a nonce, ordering value,
deduplication key, replay-protection token, or idempotency key. If a lane fails
after a mutating request is sent but before its response is received, the
client cannot determine from the protocol alone whether the mutation took
effect.

## Unsigned `vu128`

Every variable-width integer in this specification uses the unsigned 64-bit
subset of [`vu128`](https://github.com/jmillikin/rust-vu128). This section is
self-contained; implementations do not need that library.

`vu128` stores low-order value bits first. Encodings from one through four
bytes place low-order bits in the first byte after a unary length prefix.
Encodings from five through nine bytes use the first byte only as a length
prefix and store the value in little-endian order in the remaining bytes.

| Encoded bytes | Canonical value range | First byte | Value reconstruction |
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

For a first byte of at least `F0`, the encoded length is
`(first_byte & 0x0F) + 2`. Prefixes `F0`, `F1`, and `F2` are not emitted by
the canonical unsigned 64-bit encoding; values they can represent use one of
the compact prefix forms in no more bytes. Prefixes `F8` through `FF` require
more than nine bytes and exceed the unsigned 64-bit range.

A sender MUST emit the unique canonical encoding in the table. A receiver MUST
decode the value, re-encode it according to the table, and reject the input
unless the bytes are identical. This rejects:

- compact or length-prefix alternatives such as `F0`, `F1`, and `F2`;
- a value encoded with more bytes than its canonical representation;
- a first byte from `F8` through `FF`;
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
| Namespace ID | exactly 8 bytes; numeric value `1..=2^64 - 1` |
| Namespace name | `0..=255` UTF-8 bytes; zero is a valid empty name |
| Item ID | `0..=32` bytes |
| Request ID | canonical `vu128`; `0..=2^64 - 1`; at most 9 bytes |
| `SET` request value | `0..=67,108,864` bytes |
| Response payload | `0..=67,108,864` bytes |
| `vu128` integer | `0..=2^64 - 1` |
| TTL | `1..=2^64 - 1` milliseconds |

The 64 MiB value and payload limit is a wire ceiling. A server MAY configure a
smaller operational item limit. A request within the wire ceiling but above
the server limit receives `TooLarge`, and the server MUST reject it before
applying a mutation.

The largest valid `SET` request is 67,108,929 bytes: an opcode, a nine-byte
maximum `request_id`, an eight-byte `namespace_id`, one flags byte, one
`item_id_len` byte, a four-byte canonical `value_len` for 64 MiB, a nine-byte
maximum TTL, a 32-byte Item ID, and a 64 MiB value. The largest valid
`NAMESPACE_OPEN` request is 277 bytes: an opcode, a nine-byte maximum
`request_id`, two flag/length bytes, a 255-byte name, and a ten-byte maximum
namespace policy. The conservative `MAX_REQUEST_FRAME_BYTES` receive bound is
67,108,934 bytes; it reserves the maximum nine bytes for `request_id`, TTL,
and `value_len` while delimiting a frame.

The largest valid response is 67,108,878 bytes: a status byte, a nine-byte
maximum `request_id`, the four-byte canonical `payload_len` for 64 MiB, and a
64 MiB payload. The conservative `MAX_RESPONSE_FRAME_BYTES` bound is
67,108,883 bytes because it reserves the maximum nine-byte `vu128` header for
both variable-width response fields.

## Request frames

Every request starts with the common header `opcode:u8 | request_id:vu128`.
The operation layout after this header is selected by `opcode`. There is no
common request `flags` or `value_len` field. Operations that carry an Item ID
encode a one-byte length followed by that many opaque bytes.

The fixed-width opcode intentionally precedes the variable-width request ID.
This lets a receiver dispatch to the operation parser before decoding the
correlation token, and lets an unassigned opcode terminate the connection
without guessing an ID or body layout. The response uses the analogous
fixed-width `status` first, followed by the variable-width request ID.

```text
request = opcode:u8 | request_id:vu128 | operation_fields

operation_fields =
    ping | get | set | delete | stats | sync |
          namespace_open | namespace_update_policy | namespace_delete

ping                     = (empty)
get                      = namespace_id:u64be |
                           item_id_len:u8 | item_id:item_id_len
set                      = namespace_id:u64be | set_flags:u8 |
                           item_id_len:u8 | value_len:vu128 |
                           [ttl_ms:vu128] | item_id:item_id_len |
                           value:value_len
delete                   = namespace_id:u64be |
                           item_id_len:u8 | item_id:item_id_len
stats                    = namespace_id:u64be
sync                     = namespace_id:u64be
namespace_open           = open_flags:u8 | name_len:u8 |
                           name:name_len | [namespace_policy]
namespace_update_policy  = namespace_id:u64be |
                           expected_revision:u64be | namespace_policy
namespace_delete         = delete_flags:u8 | namespace_id:u64be |
                           expected_revision:u64be
```

`request_id` is a canonical `vu128` field with a maximum encoded width of
nine bytes. It is client-selected and opaque to server operation logic. A
server MUST decode enough of the field to find the opcode-specific body, but
MUST NOT compare request IDs for ordering, deduplication, or idempotency.
The wire protocol imposes no request-ID uniqueness rule.

`item_id_len` appears in `GET`, `SET`, and `DELETE`. It is one fixed byte with
a value from `0` through `32`, including zero for the valid empty Item ID.
`value_len` appears only in `SET`, including when the value is empty. It is
encoded immediately after `item_id_len`; the optional TTL follows `value_len`,
and the Item ID bytes follow the optional TTL. A receiver can therefore reject
an oversized value before allocating or reading any Item ID or value body
after reading only the bounded request metadata.
`namespace_id` is present in every namespace-scoped request and is always
encoded before the operation-specific fields. Operations that carry an Item ID
encode its `item_id_len` and then exactly that many Item ID bytes.
Namespace-management requests use the fixed-width `name_len:u8` and
`expected_revision:u64be` fields defined below.

`u64be` means one fixed eight-byte unsigned integer in network byte order
(most-significant byte first). It is not a `vu128` field and has no alternate
or shorter encoding.

Every other opcode is unassigned. A server receiving an unassigned opcode
MUST treat the request as malformed and terminate the connection without a
response. Because an unassigned opcode has no defined body layout, the server
MUST NOT scan for a possible next frame.

### Opcodes

<!-- openkache:generated-protocol-operation-table:start -->
| Opcode | Name | Request layout | Response payload | Request codecs | Response codecs |
|---|---|---|---|---|---|
| `01` | `PING` | opcode + request ID | opaque payload | — | — |
| `02` | `GET` | opcode + request ID + namespaceId (8 bytes) + u8 itemId length + itemId | opaque payload | `raw_bytes` | — |
| `03` | `SET` | opcode + request ID + namespaceId (8 bytes) + packed(condition, expirationMode, evictionMode) + u8 itemId length + vu128 value length + if expirationMode=explicit_ttl: vu128(ttlMilliseconds) + itemId + value | empty | `raw_bytes` | — |
| `04` | `DELETE` | opcode + request ID + namespaceId (8 bytes) + u8 itemId length + itemId | empty | `raw_bytes` | — |
| `05` | `STATS` | opcode + request ID + namespaceId (8 bytes) | opaque payload | — | — |
| `06` | `SYNC` | opcode + request ID + namespaceId (8 bytes) | empty | — | — |
| `07` | `NAMESPACE_OPEN` | opcode + request ID + packed(createIfMissing) + u8 length + name + if createIfMissing=true: packed(policy.defaultExpiration, policy.expirationOverride, policy.defaultEviction, policy.evictionOverride) + if policy.defaultExpiration=fixed_ttl: vu128(policy.defaultTtlMilliseconds) | opaque payload | — | — |
| `08` | `NAMESPACE_UPDATE_POLICY` | opcode + request ID + namespaceId (8 bytes) + expectedRevision (8 bytes) + packed(policy.defaultExpiration, policy.expirationOverride, policy.defaultEviction, policy.evictionOverride) + if policy.defaultExpiration=fixed_ttl: vu128(policy.defaultTtlMilliseconds) | opaque payload | — | — |
| `09` | `NAMESPACE_DELETE` | opcode + request ID + constant 0x00 + namespaceId (8 bytes) + expectedRevision (8 bytes) | empty | — | — |
<!-- openkache:generated-protocol-operation-table:end -->

### `SET` flags

`SET` carries one flags byte containing three independent two-bit selections.
`NAMESPACE_OPEN` and `NAMESPACE_DELETE` also carry operation-specific flags;
their layouts are defined in [Namespace management flags and
revisions](#namespace-management-flags-and-revisions). Other request layouts
have no flags byte.

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

The wire protocol addresses a namespace by its server-wide name. Namespace
management uses the same request/response protocol as data operations; it does
not require a separate control-plane transport.

`namespace_id` is a fixed eight-byte `u64be` in the numeric range
`1..=2^64 - 1`. The server assigns it; it is an opaque, stable server-wide
identity. Once assigned, a `namespace_id` MUST NOT be reused for a different
namespace, including after deletion, restart, recovery, or replica replacement
within the lifetime of the persistent deployment. A server MUST persist the
allocation tombstones or monotonic allocator state needed to enforce this
rule. If that state cannot be recovered, the server MUST NOT start as a v1
endpoint. `0` is reserved and MUST be rejected. The ID is carried per request
rather than bound to a lane, so a lane may be reused for different namespaces.
Clients do not allocate namespace IDs; they treat the server-returned ID as
opaque and MUST NOT synthesize or recycle one.

The wire protocol has no default namespace concept. A namespace ID of zero is
never a selector.

The `NAMESPACE_OPEN` name field has a one-byte length:

```text
name_len:u8 | name:name_len
```

`name_len = 0` is the valid empty namespace name and carries no name bytes.
It may be used with either value of `CreateIfMissing`.

For any namespace name, `name_len` MUST be `0..=255` and `name` MUST be valid
UTF-8. The length is the UTF-8 byte count, not the number of Unicode scalar
values. Names are compared by their exact UTF-8 bytes, are case-sensitive,
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

These rules are protocol rules, not cloud-provider resource-name rules. The
wire protocol does not require a narrower cloud-portable naming profile.

`GET`, `SET`, `DELETE`, `STATS`, and `SYNC` are namespace-scoped and carry a
`namespace_id`. `PING` is connection-scoped and carries none. `NAMESPACE_OPEN`
resolves a name to a namespace descriptor and can create a missing named
namespace. `NAMESPACE_UPDATE_POLICY` changes a namespace policy with an
optimistic-concurrency check. `NAMESPACE_DELETE` removes a named namespace
identity with an optimistic-concurrency check. Any namespace, including the
empty-name namespace, may be deleted once its deletion barrier confirms that
it contains no live items.
Recreating a deleted name, if allowed by the deployment, creates a new
namespace identity and therefore receives a new `namespace_id`.

`NAMESPACE_OPEN`, `NAMESPACE_UPDATE_POLICY`, and `NAMESPACE_DELETE` for one
namespace name participate in a server-side namespace operation sequence.
`NAMESPACE_OPEN` resolves or creates the name at its sequence position;
`CreateIfMissing` creation is atomic with that resolution. Requests that
address a namespace ID use the sequence position of the namespace identity
resolved at admission.

An item is identified by the pair `(namespace_id, item_id)`. The same Item ID
bytes and length in two namespaces denote two independent items.

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

Version 1 accepts only the `IfEmpty` wire value. `IfEmpty` means that no live
items remain at the delete linearization point. The server MUST establish a
namespace deletion barrier that admits no later namespace operation, drains
only requests admitted before the barrier, checks the revision and live-item
count, and then either removes the namespace identity or returns
`NamespaceNotEmpty` without changing it. Requests that address the deleted
namespace ID after a successful deletion receive `NamespaceNotFound`. Requests
that arrive while the barrier is pending are not admitted until it resolves.
If the deletion returns `NamespaceNotEmpty`, those requests may then proceed
normally. If it succeeds, namespace-ID requests receive `NamespaceNotFound`;
`NAMESPACE_OPEN` without `CreateIfMissing` does the same, while
`NAMESPACE_OPEN` with `CreateIfMissing` may atomically create a new namespace
identity for the name.
Namespace IDs are never reused within the persistent deployment lifetime.

`revision` and `expected_revision` are fixed eight-byte `u64be` values, not
`vu128` fields. A namespace revision is positive, starts at `1`, and increases
by exactly one for every successful policy update. `NAMESPACE_UPDATE_POLICY`
and `NAMESPACE_DELETE` require a non-zero `expected_revision` equal to the
current revision. A mismatch returns `Conflict` and makes no change. Revision
values MUST NOT wrap; an update that would overflow the revision range fails
without changing the policy.

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

The `policy_flags` byte has this layout:

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

On the wire, `ttl_ms` MUST be present exactly when
`expiration_mode == ExplicitTtl` and MUST be positive. It MUST be absent for
`Inherit` and `NoExpiry`. A receiver MUST reject any other combination.

`EvictionProtected` protects an item from capacity eviction only. It does not
prevent expiration, explicit `DELETE`, or replacement. The namespace's
eviction algorithm chooses only among resolved `Evictable` items. If a write
cannot be admitted without evicting a protected item, the server returns
`NoCapacity` and makes no mutation.

Each successful replacement applies the policies resolved from that `SET`.
Thus, `Inherit` on a replacement uses the current namespace defaults; retaining
protection requires the request to select `EvictionProtected` explicitly.

### Item ID

An Item ID is an opaque byte sequence from `0` through `32` bytes. Empty,
short, and 32-byte Item IDs are all valid; no byte value is reserved. The wire
protocol does not define an application key, an application-key validity rule,
or a hash algorithm. `GET`, `SET`, and `DELETE` carry `item_id_len:u8`
followed by exactly `item_id_len` Item ID bytes.

Servers MUST compare both the Item ID length and every Item ID byte.
`PING`, `STATS`, and `SYNC` carry no Item ID. The namespace and Item ID pair is
the cache identity; an Item ID is not a server-generated identifier.

The mapping from an application key to an Item ID is client-owned and is
specified in the [Client Key Format](../clients/KEY_FORMAT.md). It does not add
a wire field or change the opaque Item ID contract above.

### Value

The `SET` value is exactly `value_len` opaque bytes. Empty `SET` values are
valid. A server MUST NOT interpret any value prefix or maintain protocol
metadata for:

- serialization format;
- compression state or algorithm;
- application-level encryption state or algorithm;
- client envelope version.

A successful `GET` MUST return the same value bytes accepted by `SET`, unless
the item was subsequently replaced, deleted, expired, or evicted.

### TTL

`ttl_ms` exists only when the `SET` expiration-policy bits select
`ExplicitTtl`. It follows `value_len` and precedes the Item ID bytes. It is a
canonical unsigned `vu128` count of milliseconds. A `NoExpiry` or `Inherit`
selection has no TTL field; an inherited fixed TTL comes from the namespace
policy.

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

The examples below use request ID `0`, whose canonical encoding is the single
byte `00`. Every response includes that same request ID between the status and
payload length.

### `PING`

`PING` has the request layout `01 | request_id:vu128`.

The success response is `Ok` with exactly the four ASCII bytes `PONG`.

### `GET`

`GET` has the request layout
`02 | request_id:vu128 | namespace_id:u64be | item_id_len:u8 |
item_id:item_id_len`.

- Found: `Ok` with the exact opaque value as payload.
- Missing, expired, deleted, or evicted: `NotFound` with an empty payload.

### `SET`

`SET` has the request layout
`03 | request_id:vu128 | namespace_id:u64be | set_flags | item_id_len |
value_len | [ttl_ms] | item_id | value`.

- Stored over no live item: `Created` with an empty payload.
- Stored over a live item: `Replaced` with an empty payload.
- Condition not satisfied: `NotStored` with an empty payload, with no change
  to the existing item.

`Any` is unconditional. `IfAbsent` succeeds only when the item is logically
absent. `IfPresent` succeeds only when the item is logically present. Condition
evaluation and the mutation MUST be atomic with respect to that namespace and
Item ID.

### `DELETE`

`DELETE` has the request layout
`04 | request_id:vu128 | namespace_id:u64be | item_id_len:u8 |
item_id:item_id_len`.

- Live item removed: `Deleted` with an empty payload.
- Missing, expired, already deleted, or evicted: `NotFound` with an empty
  payload.

### `STATS`

`STATS` has the request layout `05 | request_id:vu128 | namespace_id:u64be`.

- Authorized success: `Ok` with a UTF-8 JSON object containing a `storage`
  string and a `workers` array of strings.
- Unauthorized: `Forbidden` with an optional diagnostic payload.

The JSON object is a point-in-time diagnostic snapshot of the server and its
storage workers. Version 1 does not require the `storage` or `workers` values
to be filtered to the requested namespace; `namespace_id` scopes the request,
checks that the namespace exists for an authorized request, and provides the
authorization boundary.
Per-namespace diagnostic members MAY be added in future responses. Clients
MUST ignore unknown object members so diagnostics can grow without changing
the frame protocol. The JSON payload remains subject to the 64 MiB response
limit.

### `SYNC`

An implementation MAY perform the authorization check before namespace lookup.
For an unauthorized caller, `Forbidden` MAY therefore mask whether the supplied
namespace ID exists. An authorized request for a missing namespace returns
`NamespaceNotFound`.

`SYNC` has the request layout `06 | request_id:vu128 | namespace_id:u64be`.

`SYNC` is a namespace-wide storage persistence barrier. Its linearization point
is the point at which the namespace's operation sequence admits the barrier.
All mutations to that namespace that linearized before that point are covered
by the barrier. Mutations that linearize after that point are not required to be
included.

A successful response is sent only after the configured persistence operation
for the namespace completes for every mutation covered by the barrier. The
configured persistence operation is deployment-defined, but a successful
response MUST mean that the deployment's configured durability guarantee for
those mutations has been established. The stream-order rule prevents an
earlier mutation on the same lane from overtaking `SYNC` in execution. An
earlier mutation's response may still be emitted after the `SYNC` response.
Requests on other lanes are ordered by the namespace's server-side operation
sequence; requests concurrent with the barrier may linearize on either side of
it. A successful `SYNC` response is not a response ordering fence for later
requests.

If the persistence operation cannot establish that guarantee, the server MUST
close the connection without an error response; the outcome of `SYNC` is then
unknown.

- Authorized success: `Ok` with an empty payload, sent only after the barrier
  completes.
- Unauthorized: `Forbidden` with an optional diagnostic payload.

Protocol v1 does not express selectable durability levels. The storage and
deployment durability contract is outside the frame protocol.

As with `STATS`, an implementation MAY authorize before looking up the
namespace, so `Forbidden` may mask a missing namespace. An authorized request
for a missing namespace returns `NamespaceNotFound`.

### `NAMESPACE_OPEN`

`NAMESPACE_OPEN` has the request layout:

```text
07 | request_id:vu128 | open_flags:u8 | name_len:u8 |
name:name_len | [namespace_policy]
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
08 | request_id:vu128 | namespace_id:u64be | expected_revision:u64be |
namespace_policy
```

The server checks namespace existence and `expected_revision`, then atomically
replaces the namespace policy and increments the revision. Authorization is
deployment-specific because v1 has no owner or account field. A successful
response is `Ok` with the updated `namespace_descriptor` payload. A revision
mismatch returns `Conflict` and makes no policy change.

`NAMESPACE_UPDATE_POLICY` participates in the namespace operation sequence.
Requests admitted before the policy update retain their earlier sequence
position; requests admitted after it observe the new policy. A request
concurrent with the update may linearize on either side according to that
sequence. The existence check, `expected_revision` check, policy replacement,
and revision increment are one atomic action at the policy update's
linearization point.

Policy changes apply only to future `SET` operations. Existing items retain the
expiration deadline and resolved eviction policy that were stored when they
were written.

### `NAMESPACE_DELETE`

`NAMESPACE_DELETE` has the request layout:

```text
09 | request_id:vu128 | delete_flags:u8 | namespace_id:u64be |
expected_revision:u64be
```

The only valid v1 `delete_flags` value is `00` (`IfEmpty`). The server applies
the namespace deletion barrier defined above, checks the revision and live-item
count, and removes the namespace identity only when the namespace is empty.
Authorization is deployment-specific because v1 has no owner or account field.
A successful deletion returns `Deleted` with an empty payload. There is no
reserved default namespace exception in v1.

## Response frames

Every response has this layout:

```text
+------------+-------------------+---------------------+------------------------+
| status:u8  | request_id:vu128  | payload_len:vu128   | payload:payload_len    |
+------------+-------------------+---------------------+------------------------+
```

In compact notation:

```text
response = status | request_id | payload_len | payload
```

`request_id` is present in every response and MUST be the exact canonical
request-ID bytes from the corresponding request. `payload_len` is present for
every response, including responses with an empty payload. Responses have no
version, flags, Item ID, or TTL.

The status byte is first so a client can classify a normal result or an
admission error before decoding the response body. A valid response still
requires the request ID and payload length before its frame boundary is known.
Receivers MUST validate the status assignment, canonical `request_id` and
`payload_len`, exact payload boundary, and status-specific payload contract.
An unknown status, truncated response, non-canonical response integer, or
payload/status mismatch is malformed and requires connection close with
`MALFORMED_FRAME`; it receives no response.

### Status codes

| Status | Name | Meaning |
|---:|---|---|
| `00` | `Ok` | Operation succeeded and may carry a payload |
| `01` | `NotFound` | The requested live item does not exist |
| `02` | `Created` | `SET` created a logical item or `NAMESPACE_OPEN` created a namespace |
| `03` | `Replaced` | `SET` replaced a live item |
| `04` | `Deleted` | `DELETE` removed a live item or `NAMESPACE_DELETE` removed a namespace |
| `05` | `NotStored` | A conditional `SET` made no change |
| `80` | `InvalidRequest` | A complete, well-delimited request has invalid namespace ID, flags, lengths, TTL, or semantics |
| `82` | `TooLarge` | A declared or actual item exceeds a wire or server limit |
| `83` | `Overloaded` | The server temporarily lacks admission capacity |
| `85` | `Forbidden` | The authenticated identity is not authorized |
| `86` | `InternalError` | The server could not complete the operation |
| `87` | `NoCapacity` | The write cannot be admitted without evicting protected items |
| `88` | `PolicyConflict` | The request selects an item policy disallowed by the namespace |
| `89` | `Conflict` | `expected_revision` does not match the current namespace revision |
| `8A` | `NamespaceNotFound` | The requested namespace does not exist |
| `8B` | `NamespaceNotEmpty` | `IfEmpty` deletion found one or more live items at its barrier |

Statuses `06` through `7F`, `81`, `84`, and `8C` through `FF` are unassigned. A
client MUST treat an unassigned status as a malformed response and close the
connection.

Assigned statuses `80` and above are errors. Unassigned status values in that
range are not implicitly accepted as errors; they remain malformed. Error
payloads MAY be empty or MAY contain an operator-facing diagnostic. If present,
the diagnostic SHOULD be UTF-8. Diagnostic text is not a stable programmatic
interface; clients MUST branch on the status byte rather than parsing error
text.

Every response, including an error response, carries the request ID for its
complete request. `Overloaded` is a request-level rejection: the server MUST
not begin the operation or mutation. The server MUST consume or discard the
complete rejected body before sending the response, and the lane MAY continue
afterward. If it cannot preserve the next frame boundary, it MUST close the
connection without sending an error response.

For a mutating operation (`SET`, `DELETE`, `SYNC`, namespace creation, policy
update, or namespace deletion), an error response MUST guarantee that the
mutation or persistence barrier did not take effect. If the server cannot
establish that guarantee, it MUST close the connection without an error
response, leaving the operation outcome unknown.

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

Common error statuses MAY be returned only when their stated condition applies:

| Status | Applicable requests |
|---|---|
| `InvalidRequest` | Any complete, well-delimited request with invalid semantics |
| `TooLarge` | `SET` whose value exceeds a wire or server limit |
| `Overloaded` | Any request rejected before its operation begins |
| `Forbidden` | Any request rejected by deployment authorization |
| `InternalError` | Any request known to have failed without taking effect |
| `NoCapacity` | `SET` that cannot be admitted without evicting protected items |
| `PolicyConflict` | `SET` that selects a disallowed item-policy override |
| `Conflict` | `NAMESPACE_UPDATE_POLICY` and `NAMESPACE_DELETE` revision mismatch |
| `NamespaceNotFound` | Any request addressing a missing namespace |
| `NamespaceNotEmpty` | `NAMESPACE_DELETE` whose deletion barrier finds live items |

`NamespaceNotFound` includes `GET`, `SET`, `DELETE`, `STATS`, and `SYNC`, as
well as namespace-management operations that address a missing namespace.
These domain and common errors guarantee that the requested mutation or
barrier was not applied.

A client receiving a response whose request ID does not identify one of its
outstanding requests on that same lane, or whose status is neither an allowed
domain status nor an applicable common error for that request, MUST treat the
response as malformed and close the connection.

## Validation and malformed frames

A conforming receiver MUST validate, in order where practical:

1. the opcode assignment;
2. the complete and canonical `request_id:vu128`;
3. the presence and fixed eight-byte encoding of a namespace ID for
   namespace-scoped requests;
4. the numeric namespace ID range;
5. fixed-width namespace flags, name length, and revision fields;
6. the presence of `item_id_len` and the `0..=32` Item ID length limit;
7. complete and canonical `vu128` fields;
8. the presence and value of operation flags;
9. field-specific length and UTF-8 limits;
10. the operation-specific layout;
11. TTL presence, canonical encoding, and positive value;
12. namespace-policy encoding and item-policy override rules;
13. exactly `item_id_len` Item ID bytes when present;
14. the exact remaining `SET` value length.

For a request, a receiver parses the following prefix before reading a `SET`
value body:

```text
opcode
request_id
namespace_id:u64be
set_flags
item_id_len
value_len
[ttl_ms]
item_id:item_id_len
```

For namespace-management requests, the bounded prefix is:

```text
NAMESPACE_OPEN:
    opcode | request_id | open_flags | name_len | name | [namespace_policy]
NAMESPACE_UPDATE_POLICY:
    opcode | request_id | namespace_id | expected_revision | namespace_policy
NAMESPACE_DELETE:
    opcode | request_id | delete_flags | namespace_id | expected_revision
```

The first prefix is the `SET` prefix; `GET` and `DELETE` use
`opcode | request_id | namespace_id | item_id_len | item_id:item_id_len` and
have no value body.
The brackets indicate fields selected by the `SET` expiration policy or by
`NAMESPACE_OPEN` with `CreateIfMissing`. The namespace ID and revision occupy
eight bytes whenever present. `name_len = 0` is a valid empty name; every name
must satisfy the UTF-8 and name rules above.

A receiver MUST enforce the 64 MiB value ceiling and any smaller server limit
before allocating or reading the value body. A declared value above either
limit maps to `TooLarge` for a complete, well-delimited request when the server
can consume or discard its body without losing the next frame boundary.

Receiving end-of-stream before a frame is complete is a truncated-frame error.
The receiver MUST NOT scan for a possible next frame after malformed framing.
A body shorter than its declared length is malformed. A client that sends a
second request is allowed to do so before receiving the first response, but
the second request MUST begin exactly at the first frame's boundary.
If `RESET_STREAM` or `STOP_SENDING` explicitly cancels a direction while a
frame is incomplete, the receiver MUST discard that incomplete frame under the
stream-cancellation rules; it is not malformed framing.

Malformed framing, an unassigned opcode, a non-canonical integer, or a
truncated body is terminal for the connection: the receiver MUST close the
connection without sending an error response. A semantic validation failure in
a complete, well-delimited request MAY instead receive `InvalidRequest` or the
applicable domain error. If a complete operation cannot finish and its outcome
is known to be unsuccessful, the server MAY return `InternalError`. If a
mutation or `SYNC` outcome becomes unknown because the server cannot determine
whether the operation took effect, the server MUST terminate the connection
without an error response. An unknown outcome caused only by directional stream
cancellation follows the stream-cancellation rules and does not require
connection termination.

## Unknown outcomes

The request ID provides response correlation only. It does not provide replay
protection, deduplication, idempotency, or a mutation identifier. The protocol
does not prescribe retry behavior.

If transport or connection failure occurs before a response is received, the
client must treat the outcome of an outstanding mutation or `SYNC` as unknown.
Whether to issue a new request is an application decision. A new request on a
new lane is independent even when it reuses the same request ID.

## Security and resource handling

QUIC protects frames in transit. Opaque values are not automatically
confidential from the server or from storage; application-level value
encryption remains a client concern.

Receivers MUST parse lengths incrementally and enforce the 64 MiB ceiling before
allocating or reading a complete `SET` value or response payload. Servers
SHOULD bound aggregate in-flight value memory and MAY apply implementation-local
backpressure or reject requests with `Overloaded` under resource pressure. The
protocol does not define a `max_inflight_requests_per_stream` limit; a server
may choose an admission limit, but it MUST preserve request body boundaries and
return a correlated response for any request it admits. Version 1 does not
specify a congestion-control algorithm beyond QUIC's transport behavior.

Canonical integer enforcement is security-relevant: it prevents multiple wire
representations of one logical frame and simplifies bounded incremental
parsing.

## Version evolution

Protocol v1 reserves all unassigned opcodes, statuses, and flag bits. Senders
MUST NOT use them, and receivers MUST reject them as described above.

Before v1 is finalized, this draft MAY make incompatible changes while
retaining its provisional `openkache/1` identifier. Draft implementations
therefore interoperate only when they implement the same revision of this
document.

After v1 is finalized, any change that reinterprets an existing field, changes
frame order, adds or removes mandatory fields, changes canonical integer
encoding, or changes the meaning of existing assignments requires a new ALPN
identifier. Finalized protocol versions MUST NOT reuse `openkache/1` for
incompatible frames.

When a client supports multiple versions, it MUST use the ALPN ordering and
minimum-version rules in the transport section. A server MUST select the
highest mutually supported version.

## Conformance examples

### `PING`

Request:

```text
01 00
```

Response:

```text
00 00 04 50 4F 4E 47
```

This is `Ok`, request ID `0`, `payload_len = 4`, and ASCII `PONG`.

### `GET` miss

For namespace ID `7` and an empty Item ID:

```text
02 00 00 00 00 00 00 00 00 07 00
```

A miss response is:

```text
01 00 00
```

### Request-ID and Item ID boundaries

For request ID `128`, namespace ID `7`, and the maximum-length Item ID
`00 01 02 ... 1F`, a `GET` request is:

```text
02 80 02 00 00 00 00 00 00 00 07 20
00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F
10 11 12 13 14 15 16 17 18 19 1A 1B 1C 1D 1E 1F
```

`80 02` is the canonical two-byte `vu128` encoding of request ID `128`, and
`20` is `item_id_len = 32`.

The following complete request declares 33 Item ID bytes and is semantically
invalid:

```text
02 00 00 00 00 00 00 00 00 07 21 [AA × 33]
```

Because its frame boundary is known, the server MAY return `InvalidRequest`
with the same request ID:

```text
80 00 00
```

By contrast, this request followed by end-of-stream is truncated:

```text
02 00 00 00 00 00 00 00 00 07 03 11 22
```

It declares three Item ID bytes but supplies only two. The server MUST close
the connection with `MALFORMED_FRAME` and MUST NOT send a response or search
for a later opcode. Similarly, an oversized `SET` is a complete,
well-delimited `TooLarge` request only when all bytes declared by `value_len`
are present or can be discarded without losing the next frame boundary.

### Conditional `SET` with TTL

For namespace ID `7`, the three-byte Item ID `11 22 33`, `IfAbsent`, an
explicit 5,000 millisecond TTL, `EvictionProtected`, and the ASCII value
`value`:

```text
03 00 00 00 00 00 00 00 00 07 29 03 05 88 4E 11 22 33 76 61 6C 75 65
```

- `03`: `SET`
- `00`: request ID 0
- `00 00 00 00 00 00 00 07`: namespace ID 7 (`u64be`)
- `29`: `IfAbsent` + `ExplicitTtl` + `EvictionProtected`
- `03`: three-byte Item ID length
- `05`: five-byte value length
- `88 4E`: canonical `vu128` encoding of 5,000
- `11 22 33`: exact three-byte Item ID

A created response is:

```text
02 00 00
```

### Unconditional `SET` with an empty value

For namespace ID `7`, an empty Item ID, and an empty value:

```text
03 00 00 00 00 00 00 00 00 07 00 00 00
```

This is an unconditional `SET` inheriting both namespace policies, with
an empty Item ID, no TTL field, and `value_len = 0`.

### `DELETE`, `STATS`, and `SYNC`

```text
04 00 00 00 00 00 00 00 00 07 03 11 22 33 # DELETE
05 00 00 00 00 00 00 00 00 07             # STATS
06 00 00 00 00 00 00 00 00 07             # SYNC
```

### Namespace management

Resolve the empty-name namespace:

```text
07 00 00 00
```

This is `NAMESPACE_OPEN` with request ID `0`, `open_flags = 00`, and
`name_len = 00`. It has no name or policy bytes; the empty name is the
namespace being resolved.

Create or open the named namespace `cache`:

```text
07 00 01 05 63 61 63 68 65 [namespace_policy]
```

`00` is request ID 0, `01` sets `CreateIfMissing`, and `05` is the UTF-8 byte
length of `cache`. The policy bytes are required when the create flag is set,
even if the namespace already exists; an existing policy is not overwritten.

Update a namespace policy:

```text
08 00 [namespace_id:u64be] [expected_revision:u64be] [namespace_policy]
```

Delete an empty namespace at an expected revision:

```text
09 00 00 [namespace_id:u64be] [expected_revision:u64be]
```

The first `00` is request ID 0 and the second `00` is the only valid v1
`delete_flags` value.

## Implementation conformance checklist

A protocol v1 implementation is not complete unless it:

- negotiates `openkache/1` for these frames;
- supports the documented multi-version ALPN selection and minimum-version
  rules when it supports more than one protocol version;
- emits and accepts no frame-level version byte;
- uses client-initiated bidirectional lanes and permits multiple outstanding
  requests on each lane;
- decodes a canonical client-selected `request_id:vu128`, accepts zero and
  every other unsigned 64-bit value, and echoes its exact bytes in the
  response;
- does not assign request IDs ordering, deduplication, replay, or idempotency
  semantics, and treats uniqueness as a client-side choice;
- executes complete requests on one lane in stream order while allowing
  correlated responses in a different order;
- emits each response as one contiguous frame and does not interleave response
  bytes;
- accepts FIN only at a request-frame boundary and completes responses for
  admitted requests before finishing its send direction;
- applies the directional `RESET_STREAM` and `STOP_SENDING` rules, with no
  guaranteed responses and an `unknown` outcome for outstanding operations
  after the response direction stops;
- derives request layout from the opcode;
- carries a positive server-assigned eight-byte `u64be` `namespace_id` on
  every namespace-scoped request;
- supports `NAMESPACE_OPEN`, `NAMESPACE_UPDATE_POLICY`, and
  `NAMESPACE_DELETE` on the same protocol lanes;
- sequences namespace-name management operations and allocates a new namespace
  ID when a deleted name is recreated;
- treats `name_len = 0` as a valid empty namespace name and validates all
  UTF-8 names as specified;
- uses a one-byte namespace name length and enforces the 255-byte ceiling;
- starts namespace revisions at one and enforces
  `expected_revision` on policy updates and deletion;
- never reuses a previously assigned `namespace_id` for a different namespace,
  including after namespace deletion, restart, recovery, or replica replacement
  within the persistent deployment lifetime;
- encodes namespace policies and descriptors exactly as specified;
- accepts only `IfEmpty` for the v1 namespace-delete flags;
- includes the request ID in every request and response envelope;
- includes `item_id_len` and exactly that many Item ID bytes in every `GET`,
  `SET`, and `DELETE` request, accepting every length from `0` through `32`;
- places the `SET` `item_id_len` and `value_len` before the optional TTL and
  Item ID bytes;
- includes `value_len` only in `SET`;
- validates operation-specific flags for `SET`, `NAMESPACE_OPEN`, and
  `NAMESPACE_DELETE`;
- encodes `Any`/`IfAbsent`/`IfPresent`, `ExpirationMode`, and `EvictionMode` as
  specified in the `SET` flags;
- rejects non-canonical, truncated, wider-than-`u64`, and overflowing `vu128`;
- compares Item IDs by length and complete byte sequence;
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
- preserves all value bytes without interpretation;
- rejects reserved `SET` flag bits and unassigned status values;
- enforces the 64 MiB wire ceiling before unbounded allocation or body reads;
- terminates the connection without an error response for malformed framing,
  unassigned opcodes, non-canonical integers, and truncated bodies;
- returns `Overloaded` only as a correlated request-level rejection when the
  request boundary can be preserved, and does not require a
  `max_inflight_requests_per_stream` protocol limit;
- returns `InternalError` only when a complete operation is known to have
  failed without taking effect; an unknown mutation or `SYNC` outcome
  terminates the connection without an error response;
- guarantees that a mutating error response means no mutation or barrier
  completion;
- discards the connection after framing or response-status meaning cannot be
  determined;
- treats mutation outcomes as unknown when transport fails before a response;
- implements `SYNC` as the documented storage persistence barrier.

## Reference

The `vu128` encoding was designed by John Millikin and is documented by the
[`rust-vu128` project](https://github.com/jmillikin/rust-vu128). This
specification uses only its canonical unsigned 64-bit encoding.
