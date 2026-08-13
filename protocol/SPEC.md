# OpenKache Wire Protocol Version 1

## Status

Version 1 is an unpublished, evolving draft. This document specifies only the
transport and reusable framing primitives. API contracts are designed and
implemented by the API modules that register with the transport; they define
their own request/response codes, payload codecs, semantic results, and retry
policy.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) and
[RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when they appear in
uppercase.

## Scope

This draft defines:

- QUIC application-protocol negotiation;
- the request/response stream state machine;
- opaque request and response envelopes;
- canonical unsigned `vu128` integers;
- a shared payload ceiling;
- malformed, truncated, oversized, and trailing-byte handling;
- reusable ordered-field, dense-field, and explicit optional-value layouts.

It does not define API operation names, code assignments, domain types, status
meanings, storage behavior, authorization, client ABI, or retry semantics.

## Terminology

- **Octet**: An 8-bit byte.
- **Connection**: One QUIC connection negotiated for this draft profile.
- **Lane**: One client-initiated bidirectional QUIC stream.
- **Frame**: One complete request or response.
- **Request code**: An API-owned opaque discriminator at the start of a request.
- **Response code**: An API-owned opaque discriminator at the start of a
  response.
- **Payload**: Bytes whose meaning is owned by the API contract.
- **Canonical `vu128`**: The unique encoding selected by the unsigned integer
  rules below.

All lengths count octets, not characters or code points. Hexadecimal octets are
written as two uppercase digits.

## Transport and negotiation

Version 1 runs over QUIC and therefore uses TLS 1.3. The ALPN identifier is the
11-octet ASCII string:

```text
openkache/1
```

A client supporting this draft MUST offer `openkache/1`. A server MUST select
it only when it is mutually supported. Peers without a common ALPN identifier
MUST fail negotiation.

The negotiated ALPN selects the frame version. Frames contain no version
field. An incompatible framing change requires a different ALPN identifier.
Authentication and authorization are deployment concerns; no authentication
field appears in a frame.

The transport constants for this draft are:

<!-- openkache:generated-protocol-contract-snapshot:start -->
| Transport constant | Value |
|---|---|
| ALPN | `openkache/1` |
| Maximum payload bytes | `67108864` |
| Request code bytes | `1` |
| Response code bytes | `1` |
| Minimum varuint bytes | `1` |
| Maximum varuint bytes | `9` |
<!-- openkache:generated-protocol-contract-snapshot:end -->

## Stream model

Only client-initiated bidirectional QUIC streams carry protocol frames.
Unidirectional streams have no meaning in this draft.

QUIC read and write boundaries have no protocol meaning. A frame MAY be split
across reads or writes, and one read MAY contain bytes from more than one
frame.

Each lane is request/response lockstep:

1. A client sends one complete request.
2. The client waits for the matching response.
3. The server sends at most one response for that request.
4. The lane may be reused after the response if both directions remain open.

A client MUST NOT send a second request before receiving the first response.
A server MUST NOT send unsolicited responses. A client MAY use multiple lanes
concurrently; ordering exists only within one lane.

If a server can reject a request from a prefix, it MAY send an error response
before the complete body arrives. The server MUST then close or reset that lane,
and the client MUST stop transmitting the request and MUST NOT reuse the lane.

There is no request identifier. Lane order provides correlation. If a lane
fails after a request is sent but before its response is received, the API
contract determines whether the outcome is retryable or ambiguous.

## Canonical `vu128`

Lengths and variable unsigned integers use canonical unsigned `vu128` values.
This draft accepts values in the unsigned 64-bit range.

The first octet determines the total encoded width. The remaining bits carry the
value in network byte order. An encoder MUST use the shortest valid encoding.
A decoder MUST reject:

- an incomplete prefix as incomplete input rather than a complete value;
- a width greater than the configured maximum;
- a value whose encoding is not the shortest encoding;
- a value outside the supported unsigned 64-bit range.

The shared helpers expose incremental decode (`incomplete`, `complete`, or
malformed), canonical encode, and exact encoded-length calculation.

## Payload limits and validation

The maximum encoded API payload is **67,108,864 octets**. The limit applies to
request payloads, response payloads, and nested reusable layout bodies unless
an API contract selects a smaller bound.

Implementations MUST check arithmetic for overflow before allocation or cursor
advancement. A frame that exceeds the limit MUST be rejected without allocating
the claimed size. A malformed or truncated frame MUST NOT be interpreted as a
different valid frame.

After a complete frame has been decoded, bytes outside the declared frame are
trailing bytes. A lane parser MUST either retain them as the next frame's
prefix or report them to the stream loop; an API decoder MUST NOT silently
consume trailing bytes as part of its payload.

## Request envelope

The first byte(s) are the opaque request code. For this draft the request-code
width is one octet. The API registration owns the code value and supplies a
frame layout to the incremental parser.

An API MAY choose a fixed-width body, a body preceded by a canonical `vu128`
payload length, or a sequence of byte-level steps that it documents. The shared
parser consumes only those steps:

```text
request_code | API-defined framing and payload
```

For a length-delimited request, the common form is:

```text
request_code | payload_len:vu128 | payload[payload_len]
```

The parser returns the opaque code, the prefix length, and the payload slice.
It does not decode fields, domain identifiers, flags, or semantic values.

## Response envelope

The first byte(s) are the opaque response code. For this draft the response-code
width is one octet. Every response carries a canonical payload length:

```text
response_code | payload_len:vu128 | payload[payload_len]
```

The generic response encoder/decoder validates the code width, canonical
length, payload ceiling, exact frame length, and ownership-preserving segment
boundaries. It never maps a code to a success or error meaning.

## Reusable payload layouts

These layouts are byte-level primitives. An API selects one only when its
wire contract calls for it and supplies field count, widths, requiredness, and
semantic codecs.

### Ordered field sequence

An ordered field sequence begins with a compact presence mask. Present fields
before the final present field carry canonical `vu128` lengths; the final
present field consumes the remaining bytes:

```text
presence_mask | len_0:vu128 | field_0 | ... | field_n
```

Absent fields have a cleared mask bit. A present-empty field has a set bit and
a zero length. Unused mask bits MUST be zero. The decoder MUST reject missing
required fields, truncated entries, non-canonical lengths, and trailing bytes.

Nested sequences may be used for repeated groups. The transport treats each
group as opaque bytes and does not infer alignment or domain roles.

### Dense fixed-width fields

A dense layout concatenates required fields with no per-field length prefixes:

```text
field_0[width_0] | field_1[width_1] | ...
```

The API supplies exact widths. The decoder MUST reject truncation, width
mismatch, and trailing bytes.

### Explicit optional-value layout

An API may select a fixed-width length prefix and reserve one representable
length as the missing sentinel:

```text
length_0 | value_0? | length_1 | value_1? | ...
```

The prefix width and missing sentinel are API-owned parameters. A present-empty
value is distinct from a missing value whenever the chosen sentinel is not
zero. The decoder MUST reject a prefix that does not fit the configured width,
an entry that exceeds the payload limit, truncation, and trailing bytes.

This is a reusable primitive, not a built-in operation family or an API route.

## API boundary

An API module owns:

- code assignments and collision checks;
- request and response codecs;
- field order, widths, and layout selection;
- semantic validation and result/status mapping;
- handler/client registration;
- authorization, resource access, and retry/commit policy.

The transport owns only negotiation, frame delimiting, canonical integer
helpers, payload limits, ownership, and reusable byte-level cursors. Adding an
API MUST NOT require adding an operation variant, status variant, route branch,
generated adapter, or domain type to this crate.

An API handler MUST NOT parse QUIC framing or construct transport-specific
buffers. It receives an API-owned decoded request and returns an API-owned
result; its registration adapter projects that result into an opaque response
code and payload.

## Evolution

This draft may change before publication. A change that alters ALPN, common
envelope widths, canonical integer rules, or payload-limit interpretation MUST
use a new ALPN identifier or an explicitly negotiated profile. API modules may
add codes and contracts independently as long as they preserve the transport
invariants.

## Conformance checklist

An implementation of the transport layer:

- negotiates the configured ALPN;
- preserves request/response lockstep;
- decodes only canonical `vu128` values;
- enforces the payload ceiling before allocation;
- distinguishes incomplete, malformed, and complete input;
- rejects invalid layout, truncation, width mismatch, and trailing bytes;
- keeps code and payload values opaque;
- exposes the reusable field/layout primitives without API-specific branches.
