# OpenKache Client Behavioral Contract — Version 1 Draft

> **Status:** Draft; this contract has not been released or finalized.
>
> This document specifies the target behavior for conforming OpenKache v1
> clients. Implementations may temporarily lag while the draft is completed,
> but they MUST NOT claim conformance until they satisfy the applicable
> requirements.

This contract covers client-owned behavior shared across language bindings.
The [Wire Protocol](../protocol/SPEC.md) defines bytes exchanged with the
server, the [Client Key Contract](KEY_FORMAT.md) defines application-key to
Item ID mapping, and the [Client Value Encoding Profile](VALUE_FORMAT.md)
defines formatted values.

The normative terms **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**,
**SHOULD NOT**, and **MAY** have the meanings specified by
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) and
[RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when they appear in
uppercase.

## 1. Scope

This contract defines:

- request-ID allocation and response correlation;
- lane-local request lifecycle and cancellation behavior;
- retry and unknown-outcome handling;
- the distinction between formatted-key and Exact Item ID APIs; and
- shared configuration behavior that must remain consistent across bindings.

Language-native type names, constructors, package layout, asynchronous API
shape, and binding-specific defaults are outside this contract. Each binding
documents those surfaces while preserving the behavior below.

## 2. Request lifecycle

### 2.1 Correlation identity

A client correlates a response by the pair:

```text
(lane, request_id)
```

Request IDs are client-selected unsigned 64-bit values encoded as canonical
`vu128`. They are not server-assigned identifiers, ordering keys, mutation
identifiers, replay tokens, or idempotency keys.

The wire protocol deliberately accepts duplicate request IDs because request-ID
allocation is client-owned and the server does not interpret the value. A
multiplexed client, however, MUST NOT have two outstanding requests with the
same request ID on the same lane. Responses may arrive out of stream order, so
such a duplicate would make client-side correlation ambiguous.

The same request ID MAY be outstanding on different lanes. A client MAY reuse
an ID on one lane after the complete response for its previous use has been
received and removed from the outstanding-request table.

### 2.2 Allocation

An allocator MAY use a counter, random selection, a free list, or another local
strategy. Regardless of strategy, it MUST:

1. select a value in `0..=2^64 - 1`;
2. exclude values currently outstanding on the selected lane;
3. reserve the `(lane, request_id)` entry before the request can receive a
   response;
4. encode the value canonically; and
5. release the entry only after a complete response or terminal lane failure.

Counter wraparound MUST search for a free lane-local value rather than
overwriting an outstanding entry. If no value is available, the client MUST
apply backpressure or fail the new operation locally.

### 2.3 Response dispatch

For every admitted request, the client records the operation kind and the
state needed to interpret its response. On receipt, it MUST parse the complete
response frame, find the outstanding entry by `(lane, request_id)`, validate
that the status and payload are allowed for that operation, and complete
exactly that operation.

A response ID with no outstanding entry on the same lane, a second response
for an already completed entry, or a status/payload combination invalid for
the recorded operation is a malformed response. The client MUST close the
connection as required by the wire protocol; it MUST NOT guess which request
the response belongs to.

## 3. Lane termination and unknown outcomes

A normal response establishes the operation outcome. Transport failure,
connection close, or termination of the response direction before a response
leaves an outstanding mutation or `SYNC` with an unknown outcome.

When a lane becomes terminal, the client MUST:

- prevent new requests from entering that lane;
- remove every outstanding lane-local correlation entry;
- complete read-only operations with a transport failure;
- report mutations and `SYNC` as unknown when no definitive response was
  received; and
- avoid treating request-ID reuse on another lane as a continuation of the
  failed operation.

Request IDs provide no retry safety. A binding MUST preserve the distinction
between a known server rejection and an unknown outcome.

## 4. Retry behavior

A client MAY automatically retry only when both the operation semantics and
the observed failure make the retry safe. In particular:

- a request rejected locally before any bytes are sent MAY be retried;
- `PING`, `GET`, and diagnostic reads MAY be retried after transport failure;
- a received error response guarantees the mutation did not occur and MAY be
  retried according to application policy; and
- `SET`, `DELETE`, `SYNC`, and namespace mutations MUST NOT be automatically
  replayed after an unknown outcome.

An application MAY explicitly issue a new mutation after an unknown outcome,
but the client MUST expose that as a new independent request. Reusing the same
request ID does not turn it into a protocol retry or deduplicated operation.

## 5. Client API families

### 5.1 Formatted-key APIs

A formatted-key operation performs this pipeline:

```text
application key
  -> key validation and mapping profile
  -> exact Item ID
  -> value encoding profile, when applicable
  -> wire request
```

It MUST apply the selected key contract and, for formatted writes and reads,
the selected value-format policies. Namespace resolution occurs before any key
mapping that binds the namespace ID.

### 5.2 Exact Item ID APIs

An Exact Item ID operation accepts the final `0..=32`-byte Item ID and bypasses
typed-key conversion, CBOR key encoding, Item ID hashing, and formatted-value
processing. Values are sent and returned as exact opaque bytes.

Exact Item ID APIs do not bypass wire framing, namespace identity, Item ID
length validation, request correlation, authorization, expiration, eviction,
or mutation semantics.

A binding MUST make the formatted-key and Exact Item ID families
distinguishable in its public documentation. Names containing `raw` are not
assumed to mean either family; the binding MUST state whether a method accepts
a logical application key and formatted value or an exact Item ID and opaque
value.

## 6. Configuration

Configuration that affects identity or stored bytes MUST be stable for the
lifetime of the affected data:

- key type and Item ID mapping profile;
- client root key;
- write protection and compression policy; and
- read protection allowlist.

Changing an identity setting changes which item is addressed. Changing a write
setting changes newly stored bytes but MUST NOT silently mutate the read
policy. The value profile defines the authenticated-profile migration rules
and the explicit opt-in required to read unprotected values with a configured
root key.

Bindings MAY expose different configuration syntax, but defaults and explicit
overrides MUST resolve to the same language-neutral behavior. A per-operation
override MUST NOT mutate connection-wide configuration.

## 7. Conformance checklist

A conforming client:

- keeps request correlation lane-local;
- never creates an ambiguous duplicate outstanding request ID on one lane;
- accepts out-of-order responses and dispatches them by request ID;
- validates response status and payload against the recorded operation;
- distinguishes definitive errors from unknown outcomes;
- does not automatically replay mutations with unknown outcomes;
- keeps formatted-key and Exact Item ID behavior distinct;
- resolves namespaces before namespace-bound Item ID derivation;
- preserves explicit configuration overrides without changing defaults; and
- documents binding-specific API names and defaults.
