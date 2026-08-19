# OpenKache Wire Protocol v1 (Draft)

## Status

This document is the draft design specification for OpenKache wire protocol
version 1. Version 1 has not been released or finalized. The requirements
below describe the current intended wire contract and may change before
finalization. Within this draft, an implementation conforms only when its
transport, framing, validation, operation behavior, and outcome rules satisfy
this document.

Client-owned formatted values are specified separately by the
[OpenKache value model](../clients/value/SPEC.md) and its
[value envelope](../clients/VALUE_FORMAT.md).

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) and
[RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when they appear in
uppercase.

## Scope

Version 1 specifies:

- the common request/response frame contract over QUIC and TLS-over-TCP;
- transport negotiation and transport-specific lane lifecycle;
- the request/response lane state machine;
- operation-specific request frame layouts;
- canonical unsigned `vu128` integers;
- response frame layout;
- opcode, flag, and status assignments;
- namespace lifecycle, name, policy, and revision contracts;
- item ID, value, expiration, eviction, and payload constraints;
- request-ID correlation, lane ordering, and out-of-order responses;
- malformed-frame handling and admission rejection;
- mutation error outcomes.

`SYNC` is a private server-maintenance operation used by benchmark and storage
tooling. It is not part of the public client API or the public v1 operation
conformance surface; its private persistence contract is documented in its
own subsection below.

Client-side application-key derivation, serialization, compression,
application-level encryption, and value containers are outside this protocol
and belong to the [client key](../clients/KEY_FORMAT.md), [value
model](../clients/value/SPEC.md), and [value-format](../clients/VALUE_FORMAT.md)
specifications. The shared
implementation choices used by OpenKache-maintained language bindings are
described by the [Client Implementation Guide](../clients/CLIENT.md); they are
not additional wire requirements for third-party clients. The physical storage
layout and the namespace eviction algorithm are outside this wire protocol.
Item expiration and eviction eligibility are part of the `SET` contract below.
Namespace lifecycle and policy administration are carried by the
namespace-management requests defined below.

This document has four normative layers:

1. **Wire grammar:** transport-independent frame bytes, `vu128`, field order,
   limits, and status assignments.
2. **Public operation semantics:** `PING`, `GET`, `SET`, `DELETE`, `STATS`, and
   namespace management.
3. **Namespace semantics:** namespace identity, policy, revisions, TTL, and
   eviction behavior. These are server behavior contracts carried by public
   frames, not additional frame fields.
4. **Private maintenance semantics:** the optional server-only `SYNC`
   operation. Public clients are not required to expose or implement it.

The wire grammar is the compatibility boundary. A server implementation MAY
organize the semantic layers differently, but MUST preserve the wire rules and
public operation behavior when claiming v1 conformance.

## Terminology

- **Byte**: Exactly 8 bits.
- **Connection**: One negotiated transport connection for OpenKache protocol
  v1. A QUIC connection may contain multiple lanes; a TLS-over-TCP connection
  contains exactly one lane.
- **Lane**: The unit of ordered request processing. On QUIC it is one
  client-initiated bidirectional stream. On TLS-over-TCP it is the entire TLS
  connection.
- **Frame**: One complete request or response encoded as specified below.
- **Logical request**: One request frame and its correlated response frame.
- **Request ID**: A client-selected canonical `vu128` token carried in a
  request and echoed in its response. The server treats its value as opaque.
- **Lane order**: The order in which complete request frames occur on one lane.
  It is independent of request-ID values and response order.
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
- **Unknown outcome**: The client cannot determine from the protocol whether a
  mutation took effect because no response was received.

All lengths count bytes, not characters or code points. Hexadecimal bytes are
written as two uppercase digits, such as `7F` or `E0`.

## Transport and version negotiation

Protocol v1 supports both QUIC and TLS 1.3 over TCP. Both transport bindings
carry exactly the same request and response frame bytes. No frame contains a
transport ID, lane ID, or transport-specific multiplexing header. TCP
segments, TLS records, socket reads, and socket writes have no frame-boundary
meaning. TCP plaintext is not a conforming v1 transport.

Both bindings use the same 11-byte ASCII ALPN identifier:

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

Every conforming transport MUST use TLS 1.3 and MUST negotiate the approved
post-quantum/traditional hybrid key agreement `X25519MLKEM768`. Classical-only
X25519 fallback is not permitted. This is a key-agreement requirement; it does
not require the server certificate signature itself to be post-quantum.

The ALPN negotiation selects the connection's frame version. Frames contain no
version field. Once v1 negotiation succeeds, every OpenKache frame on the
connection uses this specification. During the pre-freeze draft period,
implementations using the provisional `openkache/1` identifier MUST coordinate
the same specification revision out of band. An implementation claiming this
revision MUST NOT use an older common-header layout. After v1 is finalized, an
incompatible framing or field meaning requires a different ALPN identifier.

Peers without a common ALPN identifier MUST fail negotiation.

The server MUST present a certificate during the TLS handshake. Whether the
client verifies the certificate chain and server identity is
deployment-configurable. Disabling client-side verification still provides
encryption and passive eavesdropping protection, but does not provide active
MITM protection. A client MUST NOT treat such a connection as an authenticated
server endpoint. A deployment need not require users to provide a certificate
file; system trust or an automatically generated development identity may be
used.

Client certificate authentication is optional and deployment-configurable.
Ordinary data operations do not require mTLS by default. A deployment MAY
require an authenticated client identity for administrative or privileged
operations. When client authentication is enabled, server authentication is
also required. Omitting mTLS MUST NOT disable TLS 1.3 or the hybrid key
agreement.

## Lane model

Version 1 supports request pipelining (multiple outstanding requests) on every
lane. Each lane carries a sequence of logical requests. A request and its
response share the request ID:

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
5. The server MAY send responses in any order relative to lane order. For
   every complete, well-formed request it admits, it MUST send exactly one
   response, including an admission or semantic error, unless the lane or
   connection fails before a response can be sent. A malformed frame is not a
   parsed request and receives no response.
6. Response frames MUST be emitted as contiguous byte sequences. Response
   bytes from two frames MUST NOT be interleaved.
7. A server MUST NOT send unsolicited responses.
8. After a response, the lane MAY continue carrying requests while both
   directions remain open.
9. A client MAY use multiple lanes concurrently. Requests on different lanes
   have no client-visible relative ordering guarantee. Namespace policy update
   and deletion define only their operation-specific atomic race rules; they do
   not create a general cross-lane response or execution order.
10. A lane request direction may be closed only by the transport-specific
    half-close rules below. After a valid request-direction close, the server
    MUST admit no further requests and MUST complete responses for requests
    already admitted.

### QUIC transport profile

Only client-initiated bidirectional QUIC streams carry protocol frames.
Server-initiated unidirectional streams have no protocol meaning and MUST be
consumed and discarded by the client without parsing protocol frames. The
server MUST NOT initiate a bidirectional protocol stream; a client that
receives one MUST reset it without parsing protocol frames.

QUIC stream read and write boundaries have no protocol meaning. A frame MAY be
split across any number of reads or writes, and one read MAY contain bytes from
more than one frame.

A client FIN received at a request-frame boundary is a normal
request-direction half-close. The server MUST admit no later request, MUST
complete responses for already admitted requests, and MUST then finish its
send direction. A FIN received in the middle of a request frame is malformed.

QUIC `RESET_STREAM` and `STOP_SENDING` are directional lane cancellation:

- a client `RESET_STREAM` terminates the request direction;
- a server `RESET_STREAM` terminates the response direction;
- a client `STOP_SENDING` asks the server to stop the response direction; and
- a server `STOP_SENDING` asks the client to stop the request direction.

The affected endpoint MUST stop using that direction as specified by QUIC.
Outstanding mutations without responses have an `unknown` outcome. If
cancellation interrupts a frame, the receiver MUST discard the incomplete
frame; it is not malformed framing, and other lanes remain usable.

### TLS-over-TCP transport profile

A TLS-over-TCP connection carries exactly one lane. TCP segment, TLS record,
socket read, and socket write boundaries have no protocol meaning. Frames MAY
be split across reads and a read MAY contain more than one frame.

Normal request-direction half-close is expressed only by TLS `close_notify`.
A `close_notify` received at a request-frame boundary ends that direction; the
server MAY continue sending responses for already admitted requests. A
`close_notify` in the middle of a frame is a truncated/malformed frame.

TCP FIN/EOF without TLS `close_notify` is an unclean transport failure even
when it happens to coincide with a frame boundary. TCP RST, a TLS error alert,
and any other unclean TLS close terminate the lane. Outstanding mutations
without responses have an `unknown` outcome.

If a receiver detects malformed framing, it MUST stop processing the affected
connection. On QUIC it closes the connection with application error code
`0x01` (`MALFORMED_FRAME`). On TLS-over-TCP it closes the TLS/TCP lane without
an error response. The receiver MUST NOT scan for a possible next frame. A
complete frame whose fields are well-delimited but fail operation validation
is not malformed framing; it MAY receive the applicable error response.

`0x01` (`MALFORMED_FRAME`) is the QUIC application error code for a
connection-fatal framing or response-meaning failure. TLS-over-TCP reports the
same condition by closing the lane because it has no QUIC application error
code.

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
| `06` | private `SYNC` | opcode + request ID + namespaceId (8 bytes) | empty | — | — |
| `07` | `NAMESPACE_OPEN` | opcode + request ID + packed(createIfMissing) + u8 length + name + if createIfMissing=true: packed(policy.defaultExpiration, policy.expirationOverride, policy.defaultEviction, policy.evictionOverride) + if policy.defaultExpiration=fixed_ttl: vu128(policy.defaultTtlMilliseconds) | opaque payload | — | — |
| `08` | `NAMESPACE_UPDATE_POLICY` | opcode + request ID + namespaceId (8 bytes) + expectedRevision (8 bytes) + packed(policy.defaultExpiration, policy.expirationOverride, policy.defaultEviction, policy.evictionOverride) + if policy.defaultExpiration=fixed_ttl: vu128(policy.defaultTtlMilliseconds) | opaque payload | — | — |
| `09` | `NAMESPACE_DELETE` | opcode + request ID + constant 0x00 + namespaceId (8 bytes) + expectedRevision (8 bytes) | empty | — | — |
<!-- openkache:generated-protocol-operation-table:end -->

The operation table is a generated view of the machine-readable protocol
model. During this pre-freeze migration, this document remains the target
source of truth and the model may temporarily lag. After migration, the model
becomes the source of truth for opcode assignments, field order, wire-width
annotations, and generated client constants; this prose remains the source of
truth for semantic explanations and rejection rules. A release or conformance
check MUST fail when the generated table and finalized model differ.
Hand-editing the generated table alone does not change the protocol contract.

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
`1..=2^64 - 1`. The server assigns it; it is an opaque, stable identity within
the server's **namespace identity domain**. The domain is deployment state,
not a wire field. Within one domain, a server MUST NOT reuse an ID for a
different namespace after deletion, restart, recovery, or replica replacement.
Durable allocator state and snapshots MUST preserve that rule. An operator that
restores an independent fork or snapshot as a new deployment MUST establish a
new identity domain rather than merging its allocator history invisibly.
`0` is reserved and MUST be rejected. The ID is carried per request rather
than bound to a lane, so a lane may be reused for different namespaces.
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
namespace name are serialized for name resolution and lifecycle changes.
`NAMESPACE_OPEN` resolves or creates the name atomically. Requests that address
a namespace ID are ordered only by the lane rules and by the atomic
operation-specific condition they use; v1 does not require one global
cross-lane order for ordinary data operations.

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
items remain at the delete linearization point. The server MUST serialize the
delete with namespace lifecycle operations, check the revision and live-item
count atomically, and either remove the namespace identity or return
`NamespaceNotEmpty` without changing it. Requests that address the deleted
namespace ID after a successful deletion receive `NamespaceNotFound`.
`NAMESPACE_OPEN` without `CreateIfMissing` does the same, while
`NAMESPACE_OPEN` with `CreateIfMissing` may atomically create a new namespace
identity for the name.

A data mutation concurrent with deletion linearizes either before or after the
delete check. If it linearizes before, its live item participates in the
`IfEmpty` result. If deletion succeeds first, the mutation returns
`NamespaceNotFound` and makes no change. This atomic race rule does not require
draining all lanes or assigning every namespace operation one global sequence.
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

For persistence and recovery, a server MUST persist enough information to
reconstruct the expiration deadline without extending the item merely because
the process restarted. The recommended representation is an absolute
deployment-time expiration timestamp plus a monotonic runtime deadline. On
restart, the server reconstructs the monotonic deadline from the persisted
absolute timestamp and current deployment clock. An item whose deadline has
already passed is logically absent immediately after recovery. Snapshot or
replica restore MUST document whether it preserves the original deployment
clock domain; restoring a snapshot into a new identity domain MUST NOT silently
extend TTLs.

Physical deletion of expired items is an implementation detail. Logical
presence, conditional checks, and `NAMESPACE_DELETE` live-item counting MUST
use the deadline rule above even when cleanup is deferred.

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

- Authorized success: `Ok` with an implementation-defined diagnostic payload.
- Unauthorized: `Forbidden` with an optional diagnostic payload.

The diagnostic payload is opaque to the protocol and is intended for
operators. A server MAY use UTF-8 JSON, but v1 does not require a particular
format or member. Clients MUST NOT parse diagnostic fields as a stable
programmatic interface. `namespace_id` scopes the request, checks that the
namespace exists for an authorized request, and provides the authorization
boundary. The payload remains subject to the 64 MiB response limit.

### Private `SYNC` maintenance operation

`SYNC` is not part of the public v1 client API. The server may retain it as a
private benchmark/storage operation, and public clients MUST NOT expose it as a
normal cache method.

An implementation MAY perform the authorization check before namespace lookup.
For an unauthorized caller, `Forbidden` MAY therefore mask whether the supplied
namespace ID exists. An authorized request for a missing namespace returns
`NamespaceNotFound`.

The private request layout is
`06 | request_id:vu128 | namespace_id:u64be`.

The operation is a namespace-wide storage barrier. Its linearization point is
the point at which the namespace's operation sequence admits the barrier. All
mutations to that namespace that linearized before that point are covered.
Mutations that linearize after that point are not required to be included.

A successful response is sent only after all covered pending writes have been
sent to disk. This is a storage visibility barrier for benchmark and server
maintenance use: a later read MUST be able to use the durable storage state
instead of relying on a pending-write memory buffer. It is not a public
durability-level negotiation API.

If the disk barrier cannot be established, the server MUST close the lane
without an error response; the outcome of the private operation is unknown.

- Authorized success: `Ok` with an empty payload, sent only after the barrier
  completes.
- Unauthorized: `Forbidden` with an optional diagnostic payload.

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

`NAMESPACE_UPDATE_POLICY` is serialized with other lifecycle changes for the
same namespace name. The existence check, `expected_revision` check, policy
replacement, and revision increment are one atomic action at the policy
update's linearization point. A concurrent `SET` resolves inherited policy at
its own mutation linearization point; it is not required to participate in a
global cross-lane namespace sequence.

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
text. A client MUST preserve a diagnostic as opaque bytes when it does not
decode as UTF-8 and MUST NOT expose it as a trusted server message without
application policy. Servers SHOULD omit secrets, credentials, certificate
material, request values, and internal filesystem paths from diagnostics.
Diagnostics are subject to the response payload limit.

Every response, including an error response, carries the request ID for its
complete request. `Overloaded` is a request-level rejection: the server MUST
not begin the operation or mutation. The server MUST consume or discard the
complete rejected body before sending the response, and the lane MAY continue
afterward. If it cannot preserve the next frame boundary, it MUST close the
connection without sending an error response.

For a mutating operation (`SET`, `DELETE`, namespace creation, policy update,
or namespace deletion), an error response MUST guarantee that the mutation did
not take effect. If the server cannot establish that guarantee, it MUST close
the connection without an error response, leaving the operation outcome
unknown.

## Response contract by request

For a valid request, the following are the domain success and result statuses:

| Request | Allowed domain statuses | Payload |
|---|---|---|
| `PING` | `Ok` | Exactly `PONG` |
| `GET` | `Ok`, `NotFound` | Hit: exact value; miss: empty |
| `SET` | `Created`, `Replaced`, `NotStored` | Always empty |
| `DELETE` | `Deleted`, `NotFound` | Always empty |
| `STATS` | `Ok` | Opaque diagnostic |
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
| `NamespaceNotFound` | Any public request addressing a missing namespace |
| `NamespaceNotEmpty` | `NAMESPACE_DELETE` whose deletion barrier finds live items |

`NamespaceNotFound` includes `GET`, `SET`, `DELETE`, and `STATS`, as well as
namespace-management operations that address a missing namespace. These
domain and common errors guarantee that the requested mutation was not
applied.

The private `SYNC` operation uses `Ok`, `Forbidden`, `NamespaceNotFound`, and
the common transport/error statuses only when the private server interface
enables it. Public clients MUST NOT depend on its status applicability.

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

### Incremental parser state machine

A conforming parser MUST make the frame boundary explicit in its state, even
when the transport delivers bytes in arbitrary chunks. The minimum states are:

```text
NeedOpcode
  -> NeedRequestId
  -> NeedOperationPrefix
  -> NeedOptionalFields
  -> NeedItemId
  -> NeedValue                 # SET only
  -> Complete
  -> Malformed
```

`NeedOperationPrefix` MUST parse enough bounded metadata to determine the
operation and all declared lengths. For `SET`, the parser MUST validate
`item_id_len`, `value_len`, TTL presence, and policy flags before allocating or
reading the value body. A declared size above a wire or server limit MAY enter
`TooLarge` only when the parser can consume exactly the declared body and
preserve the next frame boundary; otherwise it MUST terminate the lane as
malformed/truncated according to the transport rules. A body that ends before
the declared length is always malformed. The parser MUST never search for an
opcode inside a declared body.

Receiving end-of-stream before a frame is complete is a truncated-frame error.
The receiver MUST NOT scan for a possible next frame after malformed framing.
A body shorter than its declared length is malformed. A client that sends a
second request is allowed to do so before receiving the first response, but
the second request MUST begin exactly at the first frame's boundary.
If `RESET_STREAM` or `STOP_SENDING` explicitly cancels a direction while a
frame is incomplete, the receiver MUST discard that incomplete frame under the
transport-specific cancellation rules; it is not malformed framing.
TLS `close_notify` during an incomplete frame is malformed; TCP EOF without
`close_notify` is always an unclean transport failure.

Malformed framing, an unassigned opcode, a non-canonical integer, or a
truncated body is terminal for the connection: the receiver MUST close the
connection without sending an error response. A semantic validation failure in
a complete, well-delimited request MAY instead receive `InvalidRequest` or the
applicable domain error. If a complete operation cannot finish and its outcome
is known to be unsuccessful, the server MAY return `InternalError`. If a
mutation outcome becomes unknown because the server cannot determine whether
the operation took effect, the server MUST terminate the connection without an
error response. An unknown outcome caused only by a transport-specific
direction cancellation follows that transport's lane rules and does not
require additional protocol error data.

## Unknown outcomes

The request ID provides response correlation only. It does not provide replay
protection, deduplication, idempotency, or a mutation identifier. The protocol
does not prescribe retry behavior.

If transport or connection failure occurs before a response is received, the
client must treat the outcome of an outstanding mutation as unknown. Private
server maintenance operations use the same rule.
Whether to issue a new request is an application decision. A new request on a
new lane is independent even when it reuses the same request ID.

## Security and resource handling

TLS 1.3 protects frames in transit on both conforming transports. The
`X25519MLKEM768` hybrid key agreement is mandatory; classical-only X25519
fallback is not allowed. Opaque values are not automatically confidential from
the server or from storage; application-level value encryption remains a
client concern. Certificate-chain/server-identity verification is a client
deployment policy and, when disabled, does not provide active MITM protection.
Optional mTLS may supply an authenticated client identity for privileged
operations.

Receivers MUST parse lengths incrementally and enforce the 64 MiB ceiling before
allocating or reading a complete `SET` value or response payload. Servers
SHOULD bound aggregate in-flight value memory and MAY apply implementation-local
backpressure or reject requests with `Overloaded` under resource pressure. The
protocol does not define a `max_inflight_requests_per_lane` limit; a server
may choose an admission limit, but it MUST preserve request body boundaries and
return a correlated response for any request it admits. Version 1 does not
specify a congestion-control algorithm beyond the selected transport's
behavior.

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

### `DELETE`, `STATS`, and private `SYNC`

```text
04 00 00 00 00 00 00 00 00 07 03 11 22 33 # DELETE
05 00 00 00 00 00 00 00 00 07             # STATS
06 00 00 00 00 00 00 00 00 07             # private SYNC
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

The hexadecimal examples above are normative boundary fixtures. A protocol
implementation SHOULD verify them with an independent frame encoder/decoder
and MUST include additional cases for split reads, pipelined frames, oversized
`SET` bodies, truncated bodies, TLS `close_notify`, TCP EOF without
`close_notify`, and QUIC directional cancellation. Before freeze, generated
fixtures SHOULD be checked against the machine-readable protocol model so
opcode/status/layout drift is detected.

## Implementation conformance checklist

A protocol v1 implementation is not complete unless it:

- negotiates `openkache/1` for these frames;
- supports both QUIC over TLS 1.3 and TLS-over-TCP over TLS 1.3 with identical
  frame bytes;
- requires the `X25519MLKEM768` hybrid key agreement and does not fall back to
  classical-only X25519;
- presents a TLS server certificate, while treating client-side identity
  verification and mTLS as deployment policies;
- supports the documented multi-version ALPN selection and minimum-version
  rules when it supports more than one protocol version;
- emits and accepts no frame-level version byte;
- maps one client-initiated bidirectional QUIC stream or one TLS-over-TCP
  connection to each lane and permits multiple outstanding requests on it;
- decodes a canonical client-selected `request_id:vu128`, accepts zero and
  every other unsigned 64-bit value, and echoes its exact bytes in the
  response;
- does not assign request IDs ordering, deduplication, replay, or idempotency
  semantics, and treats uniqueness as a client-side choice;
- executes complete requests on one lane in lane order while allowing
  correlated responses in a different order;
- emits each response as one contiguous frame and does not interleave response
  bytes;
- accepts a QUIC request-direction close only at a request-frame boundary and
  completes responses for admitted requests before finishing its send
  direction;
- accepts a TLS-over-TCP request-direction half-close only through TLS
  `close_notify`, treats TCP FIN without it as unclean, and keeps responses
  available for admitted requests;
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
- never reuses a previously assigned `namespace_id` for a different namespace
  within its namespace identity domain, including after namespace deletion,
  restart, recovery, or replica replacement;
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
- computes TTL from the mutation linearization point using a monotonic clock
  and reconstructs persisted deadlines without extending them on restart;
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
  `max_inflight_requests_per_lane` protocol limit;
- returns `InternalError` only when a complete operation is known to have
  failed without taking effect; an unknown mutation outcome terminates the
  connection without an error response;
- guarantees that a mutating error response means no mutation or barrier
  completion;
- discards the connection after framing or response-status meaning cannot be
  determined;
- treats mutation outcomes as unknown when transport fails before a response;
- implements the private `SYNC` storage barrier only when the server exposes
  that benchmark/maintenance API.

## Reference

The `vu128` encoding was designed by John Millikin and is documented by the
[`rust-vu128` project](https://github.com/jmillikin/rust-vu128). This
specification uses only its canonical unsigned 64-bit encoding.
