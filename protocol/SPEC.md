# OpenKache Wire Protocol Version 3

## Status

This document is the normative specification for OpenKache wire protocol
version 3. An implementation conforms to version 3 only when its transport,
framing, validation, and operation behavior satisfy this document.

This is the sole source of truth for server-visible protocol behavior.
Client-owned formatted values are specified separately by the
[OpenKache value format](../clients/VALUE_FORMAT.md).

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) and
[RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when they appear in
uppercase.

## Scope

Version 3 specifies:

- QUIC application-protocol negotiation;
- the request/response stream state machine;
- canonical unsigned `vu128` integers;
- request and response frame layouts;
- opcode, flag, and status assignments;
- key, value, TTL, and payload constraints;
- malformed-frame handling and retry ambiguity.

Client-side key derivation, serialization, compression, application-level
encryption, and value containers are outside this protocol and belong to the
value-format specification. Storage layout and cache eviction policy are also
outside this protocol.

## Terminology

- **Octet**: An 8-bit byte.
- **Connection**: One QUIC connection negotiated for OpenKache protocol v3.
- **Lane**: One client-initiated bidirectional QUIC stream.
- **Frame**: One complete request or response encoded as specified below.
- **Item key**: The exact 32-octet identifier used for cache equality.
- **Value**: An uninterpreted sequence of octets stored for an item key.
- **Payload**: The uninterpreted response body. Its operation-specific meaning
  is defined by this document.
- **Canonical `vu128`**: The unique encoding selected by the unsigned 64-bit
  rules in this document.

All lengths count octets, not characters or code points. Hexadecimal octets are
written as two uppercase digits, such as `7F` or `E0`.

## Transport and version negotiation

Protocol v3 runs over QUIC and therefore uses TLS 1.3 for transport security.
The exact ALPN protocol identifier is the 11-octet ASCII string:

```text
openkache/3
```

A client MUST offer `openkache/3`. A server implementing this version MUST
select `openkache/3` and MUST NOT select it for any incompatible framing.
Peers without a common ALPN identifier MUST fail negotiation.

Frames contain no version field. Once ALPN negotiation succeeds, every
OpenKache frame on the connection uses this specification. A framing or field
meaning that is incompatible with this document requires a different ALPN
version.

Authentication policy is deployment-specific. Production deployments may
require mutual TLS and may use the authenticated client identity to authorize
administrative operations. No authentication field appears in a v3 frame.

## Stream model

Only client-initiated bidirectional QUIC streams carry protocol frames.
Unidirectional streams have no protocol v3 meaning.

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
2. A client MUST NOT have more than one request in flight on one lane.
3. A server MUST send exactly one response for each request it accepts, unless
   the lane or connection fails before a response can be sent.
4. A server MUST NOT send unsolicited responses.
5. After a complete response, the lane returns to the request state and MAY be
   reused.
6. A connection MAY use multiple lanes concurrently. Ordering exists only
   within one lane.

Version 3 has no request identifier because lane order provides correlation.
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

Examples:

| Value | Canonical encoding |
|---:|---|
| `0` | `00` |
| `127` | `7F` |
| `128` | `80 02` |
| `16,383` | `BF FF` |
| `16,384` | `C0 00 02` |
| `5,000` | `88 4E` |
| `67,108,864` (64 MiB) | `E0 00 00 40` |
| `2^64 - 1` | `F7 FF FF FF FF FF FF FF FF` |

For example, `81 00` decodes numerically to `1` but is invalid because `01` is
the canonical encoding.

## Common limits

| Field | Limit |
|---|---:|
| Item key | exactly 32 octets when present |
| Request value | `0..=67,108,864` octets |
| Response payload | `0..=67,108,864` octets |
| `vu128` integer | `0..=2^64 - 1` |
| TTL | `1..=2^64 - 1` milliseconds |

The 64 MiB value and payload limit is a wire ceiling. A server MAY configure a
smaller operational item limit. A request within the wire ceiling but above
the server limit receives `TooLarge`.

The largest valid request is 67,108,912 octets: a `SET` with two fixed octets,
one-octet `key_len`, four-octet maximum `value_len`, a 32-octet item key, a
nine-octet TTL, and a 64 MiB value. The largest valid response is 67,108,869
octets: one status octet, a four-octet maximum `payload_len`, and a 64 MiB
payload.

## Request frame

Every request has this layout:

```text
+------------+----------+----------------+------------------+
| opcode:u8  | flags:u8 | key_len:vu128  | value_len:vu128  |
+------------+----------+----------------+------------------+
| item_key:key_len                                        ...
+----------------------------------------------------------+
| ttl_ms:vu128, present only when SET flag bit 0 is set    ...
+----------------------------------------------------------+
| value:value_len                                         ...
+----------------------------------------------------------+
```

In compact notation:

```text
request = opcode | flags | key_len | value_len |
          item_key | [ttl_ms] | value
```

`key_len` and `value_len` are present for every opcode, including operations
that require zero lengths. The item key immediately follows both lengths. A
present TTL follows the complete item key. The value follows the TTL, or the
item key when no TTL is present.

This ordering lets a server validate the opcode, flags, lengths, key, and TTL
before admitting or reading a large value. A receiver SHOULD perform those
checks before allocating or reading `value_len` octets.

The frame ends exactly after `value_len` value octets. On a correctly sequenced
lane, the next octet is not sent until the response has been received.

### Opcodes

| Opcode | Name | Key length | Value length |
|---:|---|---:|---:|
| `01` | `PING` | `0` | `0` |
| `02` | `GET` | `32` | `0` |
| `03` | `SET` | `32` | `0..=64 MiB` |
| `04` | `DELETE` | `32` | `0` |
| `05` | `STATS` | `0` | `0` |
| `06` | `SYNC` | `0` | `0` |

Every other opcode is unassigned. A server receiving an unassigned opcode
MUST respond with `UnsupportedOpcode` when it can send a response.

### Request flags

The flags octet is operation-specific. All flags MUST be zero for operations
other than `SET`.

| Bit | Mask | `SET` meaning |
|---:|---:|---|
| 0 | `01` | A `ttl_ms` field is present |
| 1 | `02` | Store only if the item is absent (`if_absent`) |
| 2 | `04` | Store only if the item is present (`if_present`) |
| 3–7 | `F8` | Reserved; MUST be zero |

Bits 1 and 2 are mutually exclusive. A receiver MUST reject a `SET` with both
bits set. A receiver MUST also reject any reserved bit or any nonzero flag on
another opcode.

### Item key

An item key is exactly 32 opaque octets. The protocol does not define how an
application key becomes an item key. Clients may use raw 32-octet identifiers,
a digest, a keyed derivation, or another application policy.

Servers MUST compare item keys by their complete 32-octet identity. Servers
MUST reject any opcode whose `key_len` differs from the opcode table.

### Value

The value is exactly `value_len` opaque octets. Empty `SET` values are valid.
A server MUST NOT interpret any value prefix or maintain protocol metadata for:

- serialization format;
- compression state or algorithm;
- application-level encryption state or algorithm;
- client envelope version.

A successful `GET` MUST return the same value octets accepted by `SET`, unless
the item was subsequently replaced, deleted, expired, or evicted.

### TTL

`ttl_ms` exists only when `SET` flag bit 0 is set. It is a canonical unsigned
`vu128` count of milliseconds relative to the time the server processes the
`SET`.

Zero is invalid. A server MUST reject a TTL that cannot be converted into its
supported absolute time range. When the TTL flag is clear, no TTL octets are
present and the value begins immediately after the item key.

An item is logically absent at or after its expiration time. Expired items
therefore produce `NotFound` for `GET` and `DELETE`, satisfy `if_absent`, and do
not satisfy `if_present`.

## Operation semantics

### `PING`

`PING` verifies request/response liveness.

- Valid request: zero flags, key length, and value length.
- Success response: `Ok` with the four ASCII octets `PONG`.

### `GET`

`GET` reads the current logical value for an item key.

- Found: `Ok` with the exact opaque value as payload.
- Missing, expired, deleted, or evicted: `NotFound` with an empty payload.

### `SET`

`SET` stores an opaque value and optionally applies a positive TTL and one
existence condition.

- Stored over no live item: `Created` with an empty payload.
- Stored over a live item: `Replaced` with an empty payload.
- Condition not satisfied: `NotStored` with an empty payload, with no change to
  the existing item.

Without bit 1 or bit 2, `SET` is unconditional. `if_absent` succeeds only when
the item is logically absent. `if_present` succeeds only when the item is
logically present. Condition evaluation and the mutation MUST be atomic with
respect to that item key.

### `DELETE`

`DELETE` removes the current logical item.

- Live item removed: `Deleted` with an empty payload.
- Missing, expired, already deleted, or evicted: `NotFound` with an empty
  payload.

### `STATS`

`STATS` requests server diagnostics.

- Authorized success: `Ok` with a UTF-8 JSON object containing a `storage`
  string and a `workers` array of strings.
- Unauthorized: `Forbidden` with a diagnostic payload.

Clients MUST ignore unknown JSON object members so diagnostics can grow without
changing the frame protocol.

### `SYNC`

`SYNC` requests the server's configured persistence barrier.

- Authorized success: `Ok` with an empty payload, sent only after the
  configured synchronization operation completes.
- Unauthorized: `Forbidden` with a diagnostic payload.

Protocol v3 does not express selectable durability levels. The storage and
deployment durability contract is outside the frame protocol.

## Response frame

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

The frame ends exactly after `payload_len` octets. Responses have no version,
request identifier, flags, key, or TTL.

### Status codes

| Status | Name | Meaning |
|---:|---|---|
| `00` | `Ok` | Operation succeeded and may carry a payload |
| `01` | `NotFound` | The requested live item does not exist |
| `02` | `Created` | `SET` created a logical item |
| `03` | `Replaced` | `SET` replaced a live item |
| `04` | `Deleted` | `DELETE` removed a live item |
| `05` | `NotStored` | A conditional `SET` made no change |
| `40` | `InvalidRequest` | Request framing, flags, lengths, TTL, or semantics are invalid |
| `41` | `UnsupportedOpcode` | The opcode is not assigned in v3 |
| `42` | `TooLarge` | A declared or actual item exceeds a wire or server limit |
| `43` | `Overloaded` | The server temporarily lacks admission capacity |
| `44` | `Timeout` | Reading, admission, execution, or response preparation timed out |
| `45` | `Forbidden` | The authenticated identity is not authorized |
| `7F` | `InternalError` | The server could not complete the operation |

Statuses `06` through `3F`, `46` through `7E`, and `80` through `FF` are
unassigned. A client MUST treat an unassigned status as a malformed response
and discard the lane.

Statuses `40` and above are errors. Their payload SHOULD be a UTF-8 diagnostic
for operators. Diagnostic text is not a stable programmatic interface; clients
MUST branch on the status octet rather than parsing error text.

## Validation and malformed frames

A conforming receiver MUST validate, in order where practical:

1. opcode or status assignment;
2. flags;
3. complete and canonical `vu128` fields;
4. field-specific length limits;
5. opcode-specific key and value lengths;
6. the complete item key;
7. TTL presence, canonical encoding, and positive value;
8. the exact remaining body length.

Receiving end-of-stream before a frame is complete is a truncated-frame error.
A receiver MUST NOT scan for a possible next frame after malformed framing.

When a server can respond to a malformed request:

- an unknown opcode maps to `UnsupportedOpcode`;
- a value above the 64 MiB wire ceiling maps to `TooLarge`;
- other protocol validation failures map to `InvalidRequest`.

After a framing error, the server SHOULD send one error response and close the
lane because the next frame boundary may be ambiguous. The QUIC connection and
other lanes MAY remain usable. A transport failure may prevent the error
response.

After a malformed response, a client MUST discard the lane. It MAY keep the
connection and use other lanes.

## Retry and outcome rules

The protocol provides no replay protection or mutation identifier.

- `PING`, `GET`, and `STATS` are safe to retry after reconnecting.
- A client SHOULD NOT automatically replay `SET`, `DELETE`, or `SYNC` after an
  ambiguous transport failure.
- `Created`, `Replaced`, `Deleted`, and `NotStored` are successful domain
  outcomes, not transport errors.

Applications that require stronger mutation retry semantics must provide them
above protocol v3.

## Security and resource handling

QUIC protects frames in transit. Opaque values are not automatically
confidential from the server or from storage; application-level value
encryption remains a client concern.

Receivers SHOULD parse lengths incrementally and enforce the 64 MiB ceiling
before allocating the complete body. Servers SHOULD bound aggregate in-flight
value memory and MAY reject or time out requests under resource pressure.

Canonical integer enforcement is security-relevant: it prevents multiple wire
representations of one logical frame and simplifies bounded incremental
parsing.

## Version evolution

Protocol v3 reserves all unassigned opcodes, statuses, and flag bits. Senders
MUST NOT use them, and receivers MUST reject them as described above.

Any change that reinterprets an existing field, changes frame order, adds
mandatory fields, changes canonical integer encoding, or changes the meaning
of existing assignments requires a new ALPN identifier. Version negotiation
MUST remain at the connection layer rather than adding a redundant frame
version.

## Conformance examples

### `PING`

Request:

```text
01 00 00 00
```

This is `PING`, zero flags, `key_len = 0`, and `value_len = 0`.

Response:

```text
00 04 50 4F 4E 47
```

This is `Ok`, `payload_len = 4`, and ASCII `PONG`.

### `GET` miss

For an item key containing 32 `AA` octets:

```text
02 00 20 00 [AA × 32]
```

A miss response is:

```text
01 00
```

### Conditional `SET` with TTL

For an item key containing 32 `11` octets, `if_absent`, a 5,000 millisecond
TTL, and the ASCII value `value`:

```text
03 03 20 05 [11 × 32] 88 4E 76 61 6C 75 65
```

- `03`: `SET`
- `03`: TTL present plus `if_absent`
- `20`: 32-octet key
- `05`: 5-octet value
- `88 4E`: canonical `vu128` encoding of 5,000

A created response is:

```text
02 00
```

## Implementation conformance checklist

A protocol v3 implementation is not complete unless it:

- negotiates only `openkache/3` for these frames;
- emits and accepts no frame-level version byte;
- uses client-initiated bidirectional lanes in request/response lockstep;
- rejects non-canonical, truncated, wider-than-`u64`, and overflowing `vu128`;
- validates opcode-specific key and value lengths;
- validates TTL before reading a large value when transport buffering permits;
- keeps compression and application-encryption metadata out of frames;
- preserves all value octets without interpretation;
- rejects reserved request flag bits and unassigned status values;
- enforces the 64 MiB wire ceiling before unbounded allocation;
- discards a lane after framing becomes ambiguous;
- treats mutation outcomes as ambiguous when transport fails before a response.

## Reference

The `vu128` encoding was designed by John Millikin and is documented by the
[`rust-vu128` project](https://github.com/jmillikin/rust-vu128). This
specification uses only its canonical unsigned 64-bit encoding.
