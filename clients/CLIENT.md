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
- shared key, compression, value-protection, and rotation behavior across
  bindings.

Language-native type names, constructors, package layout, asynchronous API
shape, and configuration syntax are outside this contract. Each binding
documents those surfaces while preserving the language-neutral behavior below.

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
- Item ID root key;
- the immutable mapping from each value-key ID to its value-protection key;
- write protection and compression policy; and
- read protection allowlist.

Changing an identity setting changes which item is addressed. Changing a write
setting changes newly stored bytes but MUST NOT silently mutate the read
policy. The value profile defines the authenticated-profile migration rules
and the explicit opt-in required to read unprotected values with a configured
value keyring.

Bindings MAY expose different configuration syntax, but defaults and explicit
overrides MUST resolve to the same language-neutral behavior. A per-operation
override MUST NOT mutate connection-wide configuration.

### 6.1 Identity and value-key domains

Item ID derivation and value protection use independent key domains:

```text
item_id_root_key
  -> stable Item ID derivation

value_keyring[value_key_id]
  -> rotatable value protection
```

The Item ID root key is resolved before a formatted request can address an
item. It MUST NOT change as part of value-key rotation. Changing it changes
hashed Item IDs and requires an identity migration or cache repopulation.

The value keyring maps a positive unsigned 64-bit `value_key_id` to one exact
32-byte `value_key`. A key ID is public operator-assigned metadata, not key
material. IDs MAY be sparse. Zero is reserved and MUST NOT appear in a
keyring. A client MUST NOT derive an ID from key bytes. Within one keyring:

- each key ID MUST identify exactly one immutable key;
- the same key material MUST NOT appear under multiple IDs;
- a retired key ID MUST NOT be reassigned;
- independently rotated keys SHOULD be generated independently; and
- clients that share protected entries MUST use identical mappings for every
  ID they accept.

A protected writer MUST configure exactly one `active_value_key_id`, and that
ID MUST resolve to a keyring entry. A read-only protected client MAY omit the
active ID. A convenience API that accepts one value key instead of an explicit
keyring MUST normalize it as:

```text
value_keyring = { 1 -> supplied_value_key }
active_value_key_id = 1
```

A client MUST NOT derive Item IDs from a selected value key. Conversely,
omitting or rotating value keys MUST NOT change item addressing. Sharing a
value keyring across deployments intentionally permits a protected envelope to
remain portable when its namespace and Item ID are also preserved. Deployments
that require cryptographic isolation MUST use independent value keys; this
profile adds no deployment, account, or client identifier to the derivation.

### 6.2 Compression policy

Compression is disabled by default for every client. It is an optional
formatted-write policy, not a property inferred from the language binding or
payload bytes.

When a caller enables compression without supplying tuning values, every
binding uses the same convenience defaults:

```text
compression_level = 1
minimum_input_bytes = 1,024
minimum_savings_bytes = 1
```

These values are client defaults, not value-envelope validity requirements.
Callers MAY override them for their workloads without changing format
conformance. With the shared defaults, an encoder selects Zstandard only when
compression has produced any savings:

```text
payload_length >= 1,024
and zstd_frame_length < payload_length
```

Otherwise it emits `Uncompressed`. A per-operation override MUST NOT mutate
the connection-wide compression policy. Language bindings MAY expose different
configuration syntax, but MUST NOT choose different language-specific defaults.

### 6.3 Value-key rotation

A write-capable protected client selects exactly one configured
`active_value_key_id`. Every protected write uses that ID and its associated
key; a per-operation protection-profile override MUST NOT select an inactive
value key. A protected reader selects exactly the ID carried in the envelope;
the value profile includes that ID in the AEAD associated data.

Rotation proceeds in this order:

1. Add the new immutable key-ID mapping to every reader.
2. Change writers to the new active ID.
3. Keep the previous mapping available for reads while old envelopes may
   remain.
4. Retire the previous mapping only after those envelopes have expired, been
   rewritten, or been invalidated.

Values with no expiration prevent time-based retirement; they must be
rewritten or invalidated before their key is removed. A client that reads an
unknown or retired value-key ID MUST reject the value without trying other
keys and without falling back to `Unprotected`. Reusing a request ID, rewriting
an Item ID, or changing the protection allowlist does not migrate a protected
value.

Version 1 clients MUST NOT automatically rewrite a value merely because it was
read under an inactive key. A rewrite is an ordinary `SET`, can race with
another writer, and cannot be made transparent without a compare-and-set,
generation, or equivalent concurrency contract. Rotation therefore reads old
and new key IDs but writes only the active ID. Old envelopes leave the cache
through expiration, eviction, replacement, an application-coordinated rewrite,
explicit invalidation, or namespace flush and repopulation.

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
- keeps Item ID identity keys independent from rotatable value keys;
- selects protected read keys only by the envelope's immutable value-key ID;
- does not perform automatic read-triggered value-key rewrites;
- applies shared compression defaults independently of binding language;
- preserves explicit configuration overrides without changing defaults; and
- documents binding-specific API names and configuration syntax.
