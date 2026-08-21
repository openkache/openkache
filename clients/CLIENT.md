# OpenKache Maintained Client Implementation Guide — Version 1 Draft

> **Status:** Draft `draft-2026-08-19.4`; not released or finalized.
>
> This guide describes the target implementation for OpenKache-maintained
> client SDKs. Implementations may temporarily lag during migration.

This guide explains how the maintained language bindings share one
language-independent client implementation. It does not redefine the
[Wire Protocol](../protocol/SPEC.md), the [Client Key Format](KEY_FORMAT.md),
or the [Client Value Format](VALUE_FORMAT.md) and
[Security Model](../SECURITY_MODEL.md). Those documents are the sources
of truth for interoperable bytes, identity, formatted values, and protection.

Third-party clients do not have to copy the local API design, retry defaults,
compression policy, runtime integration, or shared-core architecture described
here. A client that claims compatibility with a wire, key, or value profile
must still implement that profile exactly.

The format specifications in this draft describe the target contract. The
key and Item ID boundary is implemented in `clients/core`; the generated value
model and envelope remain transitional until their respective migrations land.

Package READMEs may expose those generated/core artifacts for compatibility, but
they must label legacy fixed-width-ID assumptions, legacy JSON envelopes,
threshold-based compression, and QUIC-only transport as current transitional
behavior.

The normative terms **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and
**MAY** apply only to OpenKache-maintained clients in this guide.

## 1. Scope and document ownership

This guide owns the common implementation decisions that sit above the format
specifications:

- the boundary between the shared Rust core and language adapters;
- request state, response dispatch, retries, cancellation, and error mapping;
- the public distinction between formatted and Exact Item ID operations;
- native key and value conversion without silent semantic loss;
- maintained-client defaults and per-operation overrides;
- native runtime, FFI, memory, and resource ownership; and
- generated contract and cross-language verification requirements.

The following subjects remain owned elsewhere and are referenced rather than
restated here:

| Subject | Source of truth |
|---|---|
| QUIC/TLS-over-TCP negotiation, frames, operations, statuses, limits, and protocol outcomes | [Wire Protocol](../protocol/SPEC.md) |
| Typed keys, canonical key bytes, mapping profiles, and Item ID derivation | [Client Key Format](KEY_FORMAT.md) |
| Payload formats, compression framing, envelope selectors, and value limits | [Client Value Format](VALUE_FORMAT.md) |
| Security goals, threat model, protection profiles, key selection, KDF, and AAD | [Security Model](../SECURITY_MODEL.md) |
| Cross-language logical values, native mappings, representations, and the initial structured-value codec profile | [Client Value Model](value/SPEC.md) |
| Rust core APIs, features, commands, and source layout | [Client core README](core/README.md) |
| Native API names, package configuration, and platform requirements | Each language package's README |

## 2. Shared implementation architecture

Maintained clients use this logical stack:

```text
language-native API
  -> language adapter
  -> generated client contract and native ABI, when applicable
  -> shared client core
  -> shared protocol implementation
  -> OpenKache server
```

The shared core owns behavior that must remain consistent across maintained
bindings:

- connection, TLS, lane, request, and response state;
- protocol request construction and response validation;
- safe retry classification and unknown-outcome tracking;
- namespace resolution needed by formatted operations;
- key validation and Item ID mapping through the key format;
- formatted-value serialization and compression through the value format;
- value-key selection and cryptographic protection through the security model;
  and
- common configuration validation and stable error categories.

The target core uses one connection/request engine. Mapped versus Exact
addressing and formatted versus Raw versus caller-owned-v0 values are
operation choices, not separate transport clients. Bindings may add convenience
facades without coupling the two axes.

A language adapter owns only the language-facing boundary:

- native type conversion;
- idiomatic synchronous, asynchronous, actor, future, promise, or callback
  shape;
- native cancellation integration;
- exception, result, and status projection;
- object, handle, buffer, and runtime lifetime;
- package construction and artifact loading; and
- documentation of package-specific capabilities or deviations.

An adapter MUST NOT introduce an independent implementation of wire framing,
Item ID derivation, formatted-value protection, or retry outcome
classification. Shared behavior needed by more than one binding belongs in the
core or generated client contract.

## 3. Common request engine

### 3.1 Lane and request state

The core maintains one outstanding-request table per protocol lane. Each
admitted operation records enough state to dispatch and interpret exactly one
response, including:

- the request correlation token assigned under the wire protocol;
- the operation kind and expected response shape;
- the completion owner used by the calling runtime;
- whether a transport retry can remain safe; and
- whether loss of the response can produce an unknown outcome.

The core reserves the correlation entry before the request can receive a
response and releases it only after completion or terminal lane failure. The
allocator and table are core implementation details; adapters do not allocate
request IDs or correlate response frames themselves.

Each lane also owns bounded admission capacity, its request-direction state,
its response parser, and terminal failure state. A saturated lane applies
backpressure or rejects new local work according to configured limits. It does
not overwrite outstanding state.

### 3.2 Response dispatch and terminal failure

The shared protocol implementation validates response framing and
operation-specific status and payload rules. The core then resolves the
lane-local outstanding entry and completes only its recorded operation.

An unmatched, duplicate, or operation-incompatible response terminates the
affected protocol connection as specified by the wire protocol. Adapters MUST
NOT guess a destination, reinterpret the payload, or expose a partially parsed
result.

When a lane or connection becomes terminal, the core:

- stops admitting new work to the affected state;
- removes and completes every affected outstanding entry;
- reports read-only operations using the applicable transport category; and
- preserves an unknown outcome for a mutation or experimental maintenance
  barrier that may have taken effect without returning a response.

### 3.3 Cancellation and shutdown

Language cancellation requests the core to stop local waiting and, where
supported, cancel transport work. Cancellation does not prove that a request
was never sent or that a mutation did not occur. The core determines the final
outcome from request progress and protocol state before the adapter maps it to
the language runtime.

Client shutdown prevents new admission, cancels or drains owned transport
tasks according to the selected shutdown mode, and completes every pending
caller exactly once. Native adapters MUST keep the underlying handle and
runtime alive until those completions no longer reference them.

## 4. Retries, outcomes, and errors

### 4.1 Common outcome model

Maintained adapters preserve these language-independent distinctions even when
their public type names differ:

- a successful operation result;
- a definitive server status;
- local configuration or input rejection;
- transport failure for an operation known not to have an unknown mutation
  outcome;
- an unknown mutation outcome;
- malformed or unsupported protocol or formatted-value input; and
- value authentication, decompression, or decoding failure.

An adapter MUST NOT collapse an unknown mutation outcome into an ordinary
transport failure that applications are likely to retry automatically.

### 4.2 Retry policy

The shared core, not each adapter, classifies retry safety. The maintained
default may retry a request rejected locally before transmission and may retry
read-only operations after a retryable transport failure within configured
attempt and deadline limits.

The maintained clients do not automatically replay a mutation or experimental
maintenance barrier after an unknown outcome. A caller may issue a new
operation explicitly, but that is not a continuation or deduplicated retry of
the first request. Unknown outcome is a distinct public result category, not a
generic transport error.

Adapters MAY expose retry count, backoff, and deadline controls. An override
changes only the selected operation or client instance; it does not change the
wire protocol or the definition of an unknown outcome.

### 4.3 Language error mapping

Bindings map common errors into idiomatic exceptions, error values, result
types, or status objects. The mapping MUST preserve the common category,
retry-safety information, and unknown-outcome distinction. Package
documentation lists the concrete language types and any retained server status
or diagnostic fields.

The maintained transport retry boundary is:

| Situation | Read-only operation | Mutation |
|---|---|---|
| Local validation or configuration failure | Do not retry | Do not retry |
| Failure known to occur before transmission | MAY retry within the configured budget | MAY retry within the configured budget |
| Request transmitted but response not received | MAY retry if the operation is otherwise retry-safe | MUST surface an unknown outcome; do not automatically replay |
| `Overloaded` response | MAY retry with bounded backoff | MAY retry with bounded backoff; the server guarantees the operation did not begin |
| Local cancellation | Complete cancellation according to adapter policy | MUST preserve unknown-outcome information if transmission may have occurred |

Conditional mutations such as `SET IfAbsent` are not automatically replayed
after an unknown outcome. A caller that chooses to issue a new request accepts
that it is an independent operation.

Server statuses have separate retry meaning:

| Status | Maintained-client guidance |
|---|---|
| `Overloaded` | The operation did not begin. Retry with bounded backoff when the deadline permits. |
| `InvalidRequest`, `TooLarge`, `PolicyConflict` | Do not retry unchanged. |
| `Forbidden` | Retry only after credentials or authorization policy changes. |
| `NoCapacity` | Retry only after capacity or eviction state changes. |
| `NamespaceNotFound` | Retry only after server namespace state or application state changes. |
| `InternalError` | The server reports no externally visible effect; retry remains a caller or configured-client decision. |

## 5. Public API and native values

### 5.1 Operation families

Address and value representation are independent API axes:

| Address | Value representation | Client behavior |
|---|---|---|
| Mapped key | Formatted v1 | Map the typed key; encode or decode the v1 envelope. |
| Exact Item ID | Formatted v1 | Use the Item ID unchanged; encode or decode the v1 envelope. |
| Mapped key | Raw | Map the typed key; preserve server value bytes. |
| Exact Item ID | Raw | Use the Item ID and value bytes unchanged. |
| Mapped or Exact | Caller-owned v0 | Resolve the address; validate only the leading canonical version `0` on write and otherwise pass the envelope through. |

An adapter MAY use overloads, options, or distinct method names, but its
documentation MUST identify both axes. `exact` means only “bypass key mapping”;
`raw` means only “bypass value encoding and decoding.”

Maintained high-level Exact APIs reject an empty Item ID unless the caller
explicitly enables it. Low-level wire-operation APIs accept the complete
`0..=32` wire range.

| Value mode | Client ownership |
|---|---|
| Formatted v1 | Encode, validate, and decode the OpenKache envelope. |
| Raw | Send and return stored bytes unchanged. |
| Caller-owned v0 | Check only canonical leading version `0`; otherwise pass through unchanged. |

### 5.2 Native key conversion

An adapter converts a supported native key into the explicit typed-key model
defined by the key format. It MUST preserve the selected type and exact
contents and MUST NOT infer a key type through reflection, stringification, or
lossy numeric conversion. The type is inferred independently for each
operation; a client or namespace does not impose one `KeyType` on all keys,
and the server has no key-type policy to enforce.

Bindings may expose different native types or only a subset of the common
typed-key model. Unsupported inputs fail locally before request construction.
Package documentation records supported native mappings and escape hatches to
the language-independent typed-key or canonical-key representation.

Dynamic bindings SHOULD dispatch directly from unambiguous native inputs:

```text
get("user:1")  -> Text("user:1")
get(1)          -> Integer(1)
get(b"\x01")    -> Bytes(01)
```

Static bindings SHOULD use overloads or an explicit `TypedKey` value rather
than weakening the API to an unconstrained `Any`/`Object` parameter. An
explicit typed-key escape hatch is useful for FFI and generic containers, but
it must produce only `Integer`, `Text`, or `Bytes`. Composite keys remain an
application concern in v1 and should be encoded explicitly as `Text` or
`Bytes`.

### 5.3 Native value conversion

Opaque byte operations preserve exact bytes. Logical structured-value
operations use the portable value model in [`value/SPEC.md`](value/SPEC.md)
and convert to native values where that conversion is lossless and
unsurprising.

An adapter MUST NOT stringify, coerce, reorder with semantic loss, or silently
drop a value or map key that its native container cannot represent. It follows
the value model's representation options: `lossless` returns the complete
generic model, while a strict `native` view returns a conversion error when
the language's ordinary containers cannot represent it. The value
specification is the normative source for these representations; each package
documents only its language-specific names and syntax.

Maintained bindings SHOULD expose one `get` operation with a representation
option equivalent to:

```text
get(key, representation="lossless")
get(key, representation="native")
```

The default for dynamic bindings SHOULD be `lossless`. An adapter MUST report
ambiguous native lookups rather than silently selecting or merging an entry.
Exact Item ID and raw operations separately return caller-owned opaque bytes;
they are not structured-value representation modes.

Typed languages SHOULD preserve compile-time distinctions with overloads or
distinct methods such as `set_native` and `set_value`, rather than one
unconstrained `Any` parameter. Overloads are an API-shape choice: all forms
MUST map to the same value-model semantics and MUST reject an
unsupported cross-language decode. A package MAY instead use one generic
method with a typed input parameter when its language can express that
contract without weakening type checking.

A binding MAY offer JSON helpers as language API convenience. JSON has no v1
payload selector: a target `set_json`/`get_json` helper serializes canonical
UTF-8 JSON and carries it as `OpaqueBytes`, with its JSON interpretation
documented by that binding. `StructuredValue-CBOR-v1` is a separate target
operation family; a binding must not silently substitute the legacy JSON
envelope or Raw bytes when exposing structured operations.

### 5.4 Runtime shape

Bindings use the concurrency model expected by their language. A Rust future,
JavaScript promise, Swift actor call, Go context operation, Python coroutine,
or synchronous native wrapper may expose the same core operation differently.
Those shapes do not change request semantics, retry classification, or value
conversion rules.

## 6. Configuration and maintained-client policies

### 6.1 Configuration boundaries

After migration, the generated client contract will be the derived common
source for configuration fields, identifiers, limits, and maintained defaults.
The draft format documents remain the source of truth until then. Adapters
translate native configuration into the generated model and let the shared
core validate combinations; they do not duplicate profile algorithms or derive
new defaults from native type behavior.

Configuration is divided into:

- connection and runtime settings, such as endpoint, transport fallback, trust,
  server-identity verification, mTLS, deadlines, lane capacity, and retry
  policy;
- identity settings consumed by the key format;
- formatted-value settings consumed by the value format; and
- per-operation overrides that do not mutate client-instance defaults.

An adapter MUST keep identity configuration separate from value-protection
configuration even when a language offers a convenience constructor. The key
and value specifications define the actual fields and validity rules.

The shared core's explicit keyring builders accept an Item-ID root and a
separate `ValueKeyring`. `ClientRootKey::public()`/`zero()` deliberately select
publicly derivable Item IDs and MUST NOT be documented as application-key
secrets. Existing root-key convenience builders remain available for source
compatibility and retain their derived value-key behavior.

The maintained identity default is `NamespaceHash`. When no value key is configured,
formatted values use `Unprotected`; this does not change key mapping.
`PublicKeyOrHash` is an explicit choice for applications that trust the
server and do not need client-side key confidentiality, namespace binding, or
root-key isolation. It is also useful for direct-key benchmarks. It remains
independent of value protection and ignores any Item ID root key.

Server certificate and identity verification is enabled by default using
system trust or configured trust roots. Disabling it requires an explicit
insecure option and never occurs as transport or version fallback.

The security properties of representative configurations are:

| Configuration | Key privacy from server | Value privacy from server | Active MITM protection |
|---|---|---|---|
| Public Item ID root, no value key, verification off | No | No | No |
| Verified TLS only | No | No | Yes |
| Secret Item ID root and protected value | Yes | Yes | Only with server verification |

### 6.2 Open design points

The following design points remain outside the stable v1 data contract:

- **Profile metadata:** the key format currently leaves profile discovery and
  mismatch handling to client policy. A future revision may define an optional
  client-local record or opaque server metadata; it MUST NOT turn `KeyType`
  into a server-enforced namespace schema. Until then, clients expose an
  explicit per-operation profile override when mixing profiles.
- **Namespace lifecycle:** stable v1 consumes server-assigned namespace IDs.
  The assignment and lifecycle interface remains in the
  [namespace WIP draft](../protocol/NAMESPACE.md).

### 6.3 Transport and server-authentication policy

Maintained clients support both protocol v1 transport bindings:

- QUIC over TLS 1.3, with one client-initiated bidirectional stream per lane;
- TLS 1.3 over TCP, with one TLS connection per lane.

Both bindings use the same `openkache/1` ALPN and exactly the same request and
response frame bytes. The maintained client may try its configured transport
fallback order, but it MUST NOT invent a transport or lane identifier in a
frame. TCP plaintext is not a conforming transport.

The TLS 1.3 handshake MUST negotiate an approved post-quantum/traditional
hybrid key agreement. The current maintained profile requires
`X25519MLKEM768`; classical-only X25519 fallback is not permitted. This is a
key-agreement requirement, not a post-quantum certificate-signature
requirement.

The approved-group registry currently contains `X25519MLKEM768`. Maintained
clients implement both transports and may use configured fallback. A
third-party implementation may conform to one transport profile without
implementing the other.

Server certificate presentation is always part of the TLS handshake. Whether
the client verifies the certificate chain and server identity is
client-configurable and enabled by default. Disabling verification requires an
explicit insecure option. It still provides passive
eavesdropping protection and encryption, but it does not provide active
MITM protection; such a connection MUST NOT be treated as an authenticated
server endpoint. Requiring a user-supplied certificate file is not a
maintained-client requirement; system trust, generated development identities,
or another configured trust policy may be used.

Client certificate authentication (mTLS) is optional and server-configured.
It is not required for ordinary data operations. A server MAY require it
for administrative or privileged operations. When mTLS is enabled, server
authentication is also required. Omitting mTLS never disables TLS 1.3 or the
hybrid key agreement.

### 6.4 Compression policy

Automatic compression is enabled by default for formatted writes in the
OpenKache-maintained clients. The maintained default policy is:

```text
compression_mode = Automatic
zstd_level = 1
```

The shared value codec attempts one Zstandard level-1 compression and emits the
Zstandard form only when the completed frame is smaller than the original
payload:

```text
zstd_frame_length < payload_length
```

Otherwise it emits the uncompressed form. This is a maintained-client policy,
not a value-format validity or interoperability requirement. Third-party
clients may use another selection policy while emitting valid value envelopes.

All maintained bindings inherit this default from the shared core and
generated client contract; a binding MUST NOT select a language-specific
default. Bindings expose an explicit opt-out. V1 Automatic has no input-size or
minimum-savings threshold. Compression applies to Formatted v1 for either
address type. Raw and caller-owned v0 values are never compressed by the
client.

### 6.5 Protection policy

Write and read policies are separate:

- with no value keys, writes and reads allow only `Unprotected`;
- with an active key ID, writes default to `AES-256-GCM-SIV`;
- a nonempty keyring without an active ID is read-only for protected values;
- a keyed client's default read allowlist accepts both authenticated profiles,
  but not `Unprotected`; and
- an explicit operation override may narrow the read allowlist or select
  `Unprotected`, without mutating the client default.

`Unprotected`, `AES-256-GCM-SIV`, and `AES-SIV-CMAC` are all stable v1 value
profiles. `Unprotected` is never selected implicitly when a value key is
configured; callers must opt in for an individual operation or client.

An authenticated write without an active key fails locally. A protected read
selects only the key ID carried by the envelope and never probes another key or
downgrades.

### 6.6 Value-key rotation

The security model owns key IDs, key selection, and protection algorithms; the
value format owns envelope validation. Maintained clients
implement only the operational read-old/write-new lifecycle around that
format:

1. Add the new immutable key-ID mapping to every reader.
2. Change writers to the new active ID.
3. Keep previous mappings readable while their values may remain.
4. Retire a previous mapping only after its values have expired, been
   replaced, or been invalidated.

Maintained clients do not automatically rewrite a value merely because it was
read under an inactive key. Such a rewrite is an ordinary mutation and can
race with another writer without a generation, compare-and-set, or equivalent
application contract.

A positive value-key ID is immutable. Once it identifies key material, it is
never rebound or reused, including after retirement.

### 6.7 Resource budget

The shared core MUST enforce one aggregate in-flight byte budget across network
bodies, decrypted bodies, decompressed payloads, and encode/decode work. It
acquires budget before reading or allocating a bounded body and releases it
when the owning operation completes. When budget is unavailable, the core
applies backpressure or returns a distinct local resource-limit error; it does
not start unbounded work. Adapters expose the configured limit without
maintaining a separate language-specific budget.

## 7. Adapter and FFI responsibilities

Bindings that use the native ABI treat it as the only boundary to the shared
core. They MUST use generated constants and declarations rather than copying
protocol, key-format, or value-format assignments into package source.

Every native ABI operation documents:

| Concern | Required contract |
|---|---|
| Input ownership | Whether each buffer is borrowed or copied, and for how long. |
| Validation | Which checks occur in the adapter and which occur in the core. |
| Failure | The common error category and whether an output handle exists. |
| Output lifetime | Who owns each result, error, and buffer and which release function ends that ownership. |

Adapters also define runtime initialization, shutdown, cancellation,
completion-thread behavior, linkage, and supported-platform failures.

The adapter must remain thin enough that a shared behavior fix can be made once
in the core. Platform-specific scheduling or memory integration belongs in the
adapter and must not leak into common request or format semantics.

## 8. Conformance and package documentation

The shared core and maintained bindings MUST satisfy the wire, key, and value
conformance vectors. Native conversion, error mapping, cancellation, resource
lifetime, generated contracts, and cross-language round trips are part of the
maintained implementation's conformance obligations.

Each implemented package README documents:

- installation, build, and verification commands;
- supported API families and native type mappings;
- asynchronous or synchronous runtime behavior;
- configuration names and maintained defaults;
- error and unknown-outcome representation;
- resource ownership where it is visible to callers; and
- any deliberate deviation from this common implementation guide.

A maintained binding is complete only when it delegates shared behavior to the
core, preserves the common outcome and value semantics, and documents its
language-specific surface without restating the underlying format
specifications.
