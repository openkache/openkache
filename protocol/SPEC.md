# OpenKache Wire Protocol v1 (Draft)

## Status

This document is revision `draft-2026-08-19.4` of the draft OpenKache wire
protocol version 1. Version 1 has not been released or finalized. The requirements
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
- the use of server-assigned namespace identities;
- item ID, value, expiration, eviction, and payload constraints;
- request-ID correlation, lane ordering, and out-of-order responses;
- malformed-frame handling and admission rejection;
- mutation error outcomes.

`STATS` and `EXPERIMENTAL_SYNC` are experimental operations. They are not part
of the stable v1 operation conformance surface. A server MAY enable them with
an `enable_experimental_api` server setting. Their current contracts are
documented separately in [`EXPERIMENTAL.md`](EXPERIMENTAL.md). An implementation
that does not enable the setting treats their opcodes as unassigned.

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
Namespace assignment and lifecycle are outside stable v1. Server recovery,
clock, and eviction obligations are summarized below and detailed in
[`SERVER_SEMANTICS.md`](SERVER_SEMANTICS.md).

This document has four normative layers:

1. **Wire grammar:** transport-independent frame bytes, `vu128`, field order,
   limits, and status assignments.
2. **Stable operation semantics:** `PING`, `GET`, `SET`, and `DELETE`.
3. **Item semantics:** server-assigned namespace identity, TTL, and eviction
   behavior carried by stable data frames.
4. **Server semantics:** TTL recovery and eviction behavior detailed in
   [`SERVER_SEMANTICS.md`](SERVER_SEMANTICS.md). Namespace lifecycle remains a
   WIP draft feature.

The wire grammar is the compatibility boundary. A server implementation MAY
organize the semantic layers differently, but MUST preserve the wire rules and
public operation behavior when claiming v1 conformance.

This document is the single normative source for wire grammar and stable
operation semantics during the draft. `SERVER_SEMANTICS.md` owns only the
runtime and optional persistence obligations referenced here.

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
- **Account**: A server-defined authenticated identity. Version 1 does not
  make an account the owner or scope of a namespace.
- **Namespace**: A server-wide collection of Item IDs with default expiration
  and eviction policies. A namespace is not nested under an account.
- **Namespace ID**: A server-assigned positive 64-bit namespace identity used in
  wire frames.
- **Namespace identity**: A server-assigned positive 64-bit namespace ID. Lifecycle
  and revision rules are outside stable v1.
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

An implementation MUST support at least one transport profile and MUST identify
which profile it supports. OpenKache-maintained servers and clients support both
profiles.

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
failure. Explicit fallback to a lower version is an application choice and MUST
be configured by the application.

TLS authenticates the negotiated ALPN as part of the handshake transcript. The
minimum-version rule protects a client from an authenticated endpoint that
deliberately selects an older protocol.

Every conforming transport MUST use TLS 1.3 and an approved
post-quantum/traditional hybrid key agreement. The approved-group registry for
this revision contains `X25519MLKEM768`, which every v1 transport
implementation MUST support. Classical-only fallback is not permitted.
Additional groups may be registered only when they provide at least the
classical and post-quantum security strength of the mandatory group. A peer
with no mutually supported approved group MUST fail the handshake. Every
registry revision MUST be published with the protocol revision so operators
can pin it; registry changes do not change frame bytes.

Protocol v1 does not require a post-quantum certificate signature. The server
certificate signature remains server-configurable because certificate
algorithm support and trust infrastructure vary independently from the
handshake key agreement. A server that requires post-quantum
authentication MUST select an approved PQ signature profile. ML-DSA under
FIPS 204 is the current intended candidate, subject to TLS certificate-profile
and backend support; v1 does not assign a certificate signature registry.
Future approved PQ signature profiles MAY be added without changing common
frame bytes or v1 operation semantics.

The ALPN negotiation selects the connection's frame version. Frames contain no
version field. Once v1 negotiation succeeds, every OpenKache frame on the
connection uses this specification. During the pre-freeze draft period,
implementations using the provisional `openkache/1` identifier coordinate the
active draft revision out of band. An implementation claiming this revision
MUST NOT use an older common-header layout. The draft deliberately keeps
`openkache/1`; after v1 is finalized, an incompatible framing or field meaning
requires a different ALPN identifier.

Peers without a common ALPN identifier MUST fail negotiation.

The server MUST present a certificate during the TLS handshake. Whether the
client verifies the certificate chain and server identity is
client-configurable. Disabling client-side verification still provides
encryption and passive eavesdropping protection, but does not provide active
MITM protection. A client MUST NOT treat such a connection as an authenticated
server endpoint. A server need not require users to provide a certificate
file; system trust or an automatically generated development identity may be
used.

Client certificate authentication is optional and server-configurable.
Ordinary data operations do not require mTLS by default. A server MAY
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
   have no lane-order relationship. Stable item operations remain linearizable
   across all lanes and connections as described below.
10. A lane request direction may be closed only by the transport-specific
    half-close rules below. After a valid request-direction close, the server
    MUST admit no further requests and MUST complete responses for requests
    already admitted.

### QUIC transport profile

Only client-initiated bidirectional QUIC streams carry protocol frames.
Server-initiated unidirectional streams have no protocol meaning and MUST be
rejected with `STOP_SENDING` without consuming their body. The server MUST NOT
initiate a bidirectional protocol stream; a client that receives one MUST reset
it without parsing protocol frames.

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
server MUST enter `DrainingResponses`, admit no later request, and continue
sending responses for every already admitted request. After those responses
are emitted, the server MUST send its response-direction `close_notify` and
enter `Closed`. A `close_notify` in the middle of a frame is a
truncated/malformed frame. Bytes received after request-direction
`close_notify` are a protocol error and MUST NOT be parsed as another frame.

TCP FIN/EOF without TLS `close_notify` is an unclean transport failure even
when it happens to coincide with a frame boundary. TCP RST, a TLS error alert,
and any other unclean TLS close terminate the TLS connection. Outstanding mutations
without responses have an `unknown` outcome.

The TLS-over-TCP lane state transitions are:

```text
Open
  -> RequestHalfClosed       # peer sends request-direction close_notify
  -> DrainingResponses       # server completes admitted responses
  -> ResponseHalfClosed      # server sends response-direction close_notify
  -> Closed
  -> UncleanFailure          # EOF/FIN without close_notify, RST, or TLS alert
```

The client MUST continue reading the response direction in
`RequestHalfClosed` and `DrainingResponses`. A reconnect creates a new lane;
request IDs may be reused there only as new correlation tokens and do not
retry or deduplicate an earlier request.

If a receiver detects malformed framing, it MUST stop processing the affected
connection. On QUIC it closes the connection with application error code
`0x01` (`MALFORMED_FRAME`). On TLS-over-TCP it closes the TLS/TCP connection
without an error response. The receiver MUST NOT scan for a possible next frame. A
A complete request frame whose fields are well-delimited but fail operation
validation is not malformed framing; it MUST receive the applicable error
response once the server has consumed the frame and can preserve the next
boundary.

`0x01` (`MALFORMED_FRAME`) is the QUIC application error code for a
connection-fatal framing or response-meaning failure. TLS-over-TCP reports the
same condition by closing the connection because it has no QUIC application error
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
maximum TTL, a 32-byte Item ID, and a 64 MiB value. The conservative
`MAX_REQUEST_FRAME_BYTES` receive bound is 67,108,934 bytes; it reserves the
maximum nine bytes for `request_id`, TTL, and `value_len` while delimiting a
frame.

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

operation_fields = ping | get | set | delete

ping                     = (empty)
get                      = namespace_id:u64be |
                           item_id_len:u8 | item_id:item_id_len
set                      = namespace_id:u64be | set_flags:u8 |
                           item_id_len:u8 | value_len:vu128 |
                           [ttl_ms:vu128] | item_id:item_id_len |
                           value:value_len
delete                   = namespace_id:u64be |
                           item_id_len:u8 | item_id:item_id_len
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

`u64be` means one fixed eight-byte unsigned integer in network byte order
(most-significant byte first). It is not a `vu128` field and has no alternate
or shorter encoding.

Opcodes `05` and `06` are not stable v1 operations. A server with
`enable_experimental_api` may recognize their current experimental layouts.
Otherwise they are unassigned and malformed. See
[`EXPERIMENTAL.md`](EXPERIMENTAL.md).

Every opcode not assigned above or enabled by that experimental setting is
unassigned. A server receiving one MUST terminate the connection without a
response and MUST NOT scan for a possible next frame.

### Opcodes

<!-- openkache:generated-protocol-operation-table:start -->
| Opcode | Name | Request layout | Response payload |
|---|---|---|---|
| `01` | `PING` | opcode + request ID | `PONG` |
| `02` | `GET` | opcode + request ID + namespace ID + Item ID | opaque value or empty |
| `03` | `SET` | opcode + request ID + namespace ID + flags + lengths + optional TTL + Item ID + value | empty |
| `04` | `DELETE` | opcode + request ID + namespace ID + Item ID | empty |
<!-- openkache:generated-protocol-operation-table:end -->

During this pre-freeze migration, this document is the source of truth and the
machine-readable model may lag. After migration, the model will own stable
assignments, field order, wire widths, and generated constants; this document
will continue to own semantic and rejection rules. A release conformance check
MUST fail when the finalized model and table differ.

### `SET` flags

`SET` carries one flags byte containing three independent two-bit selections.
Other stable request layouts have no flags byte.

| Bits | Mask | Values |
|---:|---:|---|
| 0–1 | `03` | `00` = `Any`; `01` = `IfAbsent`; `10` = `IfPresent`; `11` = invalid in v1 |
| 2–3 | `0C` | `00` = `Inherit`; `01` = `NoExpiry`; `10` = `ExplicitTtl`; `11` = invalid in v1 |
| 4–5 | `30` | `00` = `Inherit`; `01` = `Evictable`; `10` = `EvictionProtected`; `11` = invalid in v1 |
| 6–7 | `C0` | Invalid in v1; MUST be zero |

The expiration selection controls the presence of `ttl_ms`: `ExplicitTtl`
requires one, while `Inherit` and `NoExpiry` omit it. A receiver MUST reject a
`SET` with any invalid value or invalid bit set. Policy conflicts are
described in [Namespace policy](#namespace-policy).

An expiration-mode value selects whether the following `ttl_ms` field is
present. Therefore an invalid expiration-mode value makes the request shape
undecidable and is a malformed request: the receiver MUST close the connection
without a response. Invalid condition, eviction, or upper flag bits do
not change the request shape; once the complete frame is delimited, they MUST
receive `InvalidRequest`.

Invalid v1 values have no forward-compatible meaning. Assigning meaning to
one of them requires a new protocol version and ALPN; a v1 implementation MUST
not accept it based on a local extension.

### Namespace

`namespace_id` is a fixed eight-byte `u64be` in the numeric range
`1..=2^64 - 1`. It identifies a namespace assigned by the server outside the stable v1
data protocol. Zero is invalid. `GET`, `SET`, and `DELETE` carry the ID per
request, so one lane may address multiple namespaces. An item is identified by
the pair `(namespace_id, item_id)`.

Stable v1 does not define namespace creation, lookup by name, policy update, or
deletion. The previous lifecycle proposal remains as a WIP draft in
[`NAMESPACE.md`](NAMESPACE.md) and is not an implementation requirement.
How a client receives a server-assigned namespace ID is outside stable v1.

### Namespace policy

A namespace has a default expiration policy, a default eviction policy, and an
independent rule for whether each default may be overridden by an item request.
The server assigns either `NoExpiry` or a positive `FixedTtl`,
and with either `Evictable` or `EvictionProtected`. Neither default may be
`Inherit`. The assignment interface is outside stable v1.

`SET` carries the item-level `ExpirationMode` and `EvictionMode` selections in
its flags. `Inherit` resolves to the server-assigned namespace default. An explicit
item selection is accepted only when that override is allowed by the
server-assigned policy; otherwise the server returns `PolicyConflict` and makes no
mutation. A successful `SET` resolves both policies at its
mutation linearization point and stores the resolved item metadata.

On the wire, `ttl_ms` MUST be present exactly when
`expiration_mode == ExplicitTtl` and MUST be positive. It MUST be absent for
`Inherit` and `NoExpiry`. A receiver MUST reject any other combination.

`EvictionProtected` protects an item from capacity eviction only. It does not
prevent expiration, explicit `DELETE`, or replacement. The namespace's
eviction algorithm chooses only among resolved `Evictable` items. If a write
cannot be admitted without evicting a protected item, the server returns
`NoCapacity` and makes no mutation.

Namespace policy is immutable for the lifetime of a namespace in v1. A server
MUST NOT change its defaults or override permissions in place. Each successful
replacement applies the policies resolved from that `SET`; `Inherit` therefore
always resolves against the same namespace defaults.

### Item ID

An Item ID is an opaque byte sequence from `0` through `32` bytes. Empty,
short, and 32-byte Item IDs are all valid on the wire; no byte value has
special meaning. A high-level client that guards against accidental empty IDs
MAY require an explicit opt-in without changing wire conformance.
The wire protocol does not define an application key, an application-key
validity rule,
or a hash algorithm. `GET`, `SET`, and `DELETE` carry `item_id_len:u8`
followed by exactly `item_id_len` Item ID bytes.

Servers MUST compare both the Item ID length and every Item ID byte.
`PING` carries no Item ID. The namespace and Item ID pair is the cache
identity; an Item ID is not a server-generated identifier.

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

Persistence, clock, snapshot, and recovery requirements are defined in
[`SERVER_SEMANTICS.md`](SERVER_SEMANTICS.md). Physical deletion remains an
implementation detail; logical presence and conditional checks follow the
deadline rule even when cleanup is deferred.

## Operation semantics

The examples below use request ID `0`, whose canonical encoding is the single
byte `00`. Every response includes that same request ID between the status and
payload length.

Every stable item operation has one linearization point between admission and
response. Operations addressing the same `(namespace_id, item_id)` MUST be
linearizable across all lanes and connections:

- if one operation completes before another is invoked, the later operation
  observes the earlier operation;
- concurrent operations may linearize in either order;
- lane order constrains that order for operations received on the same lane;
  and
- condition evaluation and mutation occur at the same linearization point.

No total order is required between operations on different items.

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
evaluation and the mutation MUST be atomic at the operation's linearization
point.

### `DELETE`

`DELETE` has the request layout
`04 | request_id:vu128 | namespace_id:u64be | item_id_len:u8 |
item_id:item_id_len`.

- Live item removed: `Deleted` with an empty payload.
- Missing, expired, already deleted, or evicted: `NotFound` with an empty
  payload.

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
| `02` | `Created` | `SET` created a logical item |
| `03` | `Replaced` | `SET` replaced a live item |
| `04` | `Deleted` | `DELETE` removed a live item |
| `05` | `NotStored` | A conditional `SET` made no change |
| `80` | `InvalidRequest` | A complete, well-delimited request has invalid namespace ID, flags, lengths, TTL, or semantics |
| `82` | `TooLarge` | A validly bounded item exceeds a server-local limit |
| `83` | `Overloaded` | The server temporarily lacks admission capacity |
| `85` | `Forbidden` | The authenticated identity is not authorized |
| `86` | `InternalError` | The server could not complete the operation |
| `87` | `NoCapacity` | The write cannot be admitted without evicting protected items |
| `88` | `PolicyConflict` | The request selects an item policy disallowed by the server-assigned namespace policy |
| `8A` | `NamespaceNotFound` | The server-assigned namespace does not exist |

Statuses `06` through `7F`, `81`, `84`, `89`, `8B` through `FF` are unassigned.
A client MUST treat an unassigned status as a malformed response and close the
connection. Assigning meaning to an unassigned status requires a new protocol
version and ALPN; v1 has no unknown-status extension rule.

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

For `SET` or `DELETE`, an error response MUST guarantee that the mutation did
not take effect. Otherwise the server MUST close the connection without an
error response, leaving the operation outcome unknown.

## Response contract by request

For a valid request, the following are the domain success and result statuses:

| Request | Allowed domain statuses | Payload |
|---|---|---|
| `PING` | `Ok` | Exactly `PONG` |
| `GET` | `Ok`, `NotFound` | Hit: exact value; miss: empty |
| `SET` | `Created`, `Replaced`, `NotStored` | Always empty |
| `DELETE` | `Deleted`, `NotFound` | Always empty |

Common error statuses MAY be returned only when their stated condition applies:

| Status | Applicable requests |
|---|---|
| `InvalidRequest` | Any complete, well-delimited request with invalid semantics |
| `TooLarge` | `SET` whose wire-valid value exceeds a server-local limit |
| `Overloaded` | Any request rejected before its operation begins |
| `Forbidden` | Any request rejected by server authorization |
| `InternalError` | Any request known to have failed without taking effect |
| `NoCapacity` | `SET` that cannot be admitted without evicting protected items |
| `PolicyConflict` | `SET` that selects a disallowed item-policy override |
| `NamespaceNotFound` | `GET`, `SET`, or `DELETE` addressing a missing namespace |

The effect guarantees are:

| Result | Mutation effect |
|---|---|
| `Created`, `Replaced`, `Deleted` | The mutation took effect. |
| `NotStored`, `NotFound` | The requested mutation did not take effect. |
| Any error response | The requested mutation did not take effect. |

The protocol exposes effect guarantees, not an automatic retry policy:

| Status | Effect guarantee |
|---|---|
| `Overloaded` | The operation did not begin. |
| `InvalidRequest`, `PolicyConflict` | No effect; the unchanged request remains invalid or conflicting. |
| `NamespaceNotFound` | No effect; namespace or application state must change before the request can succeed. |
| `InternalError` | The server definitively determined that no externally visible effect occurred. |

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
5. the presence of `item_id_len` and the `0..=32` Item ID length limit;
6. complete and canonical `vu128` fields;
7. the presence and value of operation flags;
8. the operation-specific layout;
9. TTL presence, canonical encoding, and positive value;
10. item-policy override rules;
11. exactly `item_id_len` Item ID bytes when present;
12. the exact remaining `SET` value length.

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

The first prefix is the `SET` prefix; `GET` and `DELETE` use
`opcode | request_id | namespace_id | item_id_len | item_id:item_id_len` and
have no value body. Brackets indicate the field selected by the `SET`
expiration policy.

A receiver MUST enforce the 64 MiB wire ceiling and any smaller server limit
before allocating or reading the value body. A `value_len` greater than the
64 MiB wire ceiling is outside the v1 frame contract and MUST terminate the
connection without a response; the receiver MUST NOT wait for or discard
the declared unbounded body. A value within the wire ceiling but above a
server-local operational limit MUST receive `TooLarge` when the receiver can
consume exactly that bounded body and preserve the next frame boundary. If it
cannot preserve the boundary, it MUST close the connection without a response.

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
reading the value body. A declared size above the wire ceiling is terminal
without a response. A declared size above a server-local limit MAY enter
`TooLarge` only when the parser can consume exactly the bounded body and
preserve the next frame boundary; otherwise it MUST terminate the connection as
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
a complete, well-delimited request MUST receive `InvalidRequest` or the
applicable domain error once the receiver has consumed the bounded frame and
can preserve the next frame boundary. If it cannot preserve that boundary, it
MUST close the connection without a response. If a complete operation cannot
finish and its outcome
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
client must treat the outcome of an outstanding mutation as unknown.
Whether to issue a new request is an application decision. A new request on a
new lane is independent even when it reuses the same request ID.

## Security and resource handling

TLS 1.3 protects frames in transit on both conforming transports.
`X25519MLKEM768` is the current mandatory approved hybrid group;
classical-only fallback is not allowed. Opaque values are not automatically
confidential from the server or storage. Certificate-chain and server-identity
verification is client policy; disabling it removes active MITM
protection. Optional mTLS may authenticate clients for privileged operations.

Receivers MUST parse lengths incrementally and enforce the 64 MiB ceiling before
allocating or reading a complete `SET` value or response payload. Servers
MUST bound aggregate in-flight frame bytes and MAY apply implementation-local
backpressure or reject requests with `Overloaded` under resource pressure. The
protocol does not define a `max_inflight_requests_per_lane` limit; a server
may choose an admission limit, but it MUST preserve request body boundaries and
return a correlated response for any request it admits. Version 1 does not
specify a congestion-control algorithm beyond the selected transport's
behavior.

Canonical integer enforcement is security-relevant: it prevents multiple wire
representations of one logical frame and simplifies bounded incremental
parsing.

## Conformance profiles

A conformance claim identifies the implementation role, transport profile, and
optional operational profiles:

- **Client**: emits valid requests, validates responses, correlates lane-local
  request IDs, and preserves unknown mutation outcomes.
- **Server**: implements stable operation semantics, cross-lane item
  linearizability, namespace policy, runtime TTL behavior, and bounded
  admission.
- **QUIC transport** or **TLS-over-TCP transport**: implements the selected
  transport binding. At least one is required; maintained OpenKache clients and
  servers implement both.
- **Persistent TTL**: persists, restarts, snapshots, or restores expiring
  items according to [`SERVER_SEMANTICS.md`](SERVER_SEMANTICS.md). A server
  that does not persist expiring items does not claim this profile.

Client key mapping and formatted-value profiles are separate client
conformance claims and do not change wire conformance.

## Version evolution

Protocol v1 assigns no meaning to unassigned opcodes, statuses, or flag bits.
Senders MUST NOT use them, and receivers MUST reject them as described above.

Before v1 is finalized, this draft MAY make incompatible changes while
retaining its provisional `openkache/1` identifier. Draft implementations
therefore interoperate only when they implement the same revision of this
document.

After v1 is finalized, any change that reinterprets an existing field, assigns
meaning to an invalid flag/value, changes frame order, adds or removes
mandatory fields, changes canonical integer encoding, or changes the meaning
of existing assignments requires a new ALPN identifier. Finalized protocol
versions MUST NOT reuse `openkache/1` for incompatible frames.

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

Because its frame boundary is known, the server MUST return `InvalidRequest`
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

### `DELETE`

```text
04 00 00 00 00 00 00 00 00 07 03 11 22 33
```

The hexadecimal examples above are normative boundary fixtures. A protocol
implementation SHOULD verify them with an independent frame encoder/decoder
and MUST include additional cases for split reads, pipelined frames, oversized
`SET` bodies, truncated bodies, TLS `close_notify`, TCP EOF without
`close_notify`, and QUIC directional cancellation. Before freeze, generated
fixtures SHOULD be checked against the machine-readable protocol model so
opcode/status/layout drift is detected.

## Implementation conformance checklist

Every protocol v1 implementation:

- identifies its role and supported transport profile;
- uses `openkache/1`, TLS 1.3, an approved hybrid key agreement, and the common
  frame bytes;
- implements the lane, half-close, cancellation, canonical `vu128`, assignment,
  layout, and limit rules for its role; and
- closes the connection for malformed or undecidable frames.

A client additionally validates response meaning and correlation and preserves
unknown mutation outcomes. A server additionally:

- accepts every `0..=32`-byte Item ID and preserves values opaquely;
- implements stable operation semantics, cross-lane item linearizability,
  server-assigned namespace policy, runtime TTL, and eviction eligibility;
- bounds aggregate in-flight frame bytes; and
- returns a correlated error only for a complete request whose mutation is
  known not to have taken effect.

A server claiming Persistent TTL also satisfies the recovery requirements in
[`SERVER_SEMANTICS.md`](SERVER_SEMANTICS.md).

## Reference

The `vu128` encoding was designed by John Millikin and is documented by the
[`rust-vu128` project](https://github.com/jmillikin/rust-vu128). This
specification uses only its canonical unsigned 64-bit encoding.
