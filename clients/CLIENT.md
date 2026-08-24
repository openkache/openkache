# OpenKache Maintained Client Contract — Version 1 Gate 0

> **Status:** Frozen Gate 0 (`v1-gate0`, 2026-08-24).
>
> This document is the public source of truth for the first maintained client
> surface. A binding MUST NOT claim v1 compatibility while it disagrees with
> this document, [`KEY_FORMAT.md`](KEY_FORMAT.md),
> [`VALUE_FORMAT.md`](VALUE_FORMAT.md), or
> [`value/SPEC.md`](value/SPEC.md).

The contract is intentionally small. It defines one connection lifecycle, three
data operations, one lossless value profile, and one typed-key mapping. The
server wire grammar remains in [`../protocol/SPEC.md`](../protocol/SPEC.md);
this document defines what a maintained client exposes above that grammar.

The normative terms **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and
**MAY** have their RFC 2119 meanings when they appear in uppercase.

## 1. Public surface

Every maintained binding exposes these operations, with an idiomatic sync,
future, promise, coroutine, or callback projection:

| Operation | Input | Successful result |
|---|---|---|
| `connect` | endpoint and the fixed development profile | an open client |
| `get` | one [`TypedKey`](KEY_FORMAT.md) | `GetResult<Value>` |
| `set` | one `TypedKey` and one `Value` | `Created` or `Replaced` |
| `delete` | one `TypedKey` | `Deleted` or `NotFound` |
| `close` | an open client | completion with no value |

`connect` MUST establish TLS before returning a usable client. A failed
connection is an error and does not produce a partially usable client.
`get`, `set`, and `delete` use the mapped typed-key path and the
`StructuredValue-CBOR-v1` value profile only. Bindings MAY expose constructors
or checked accessors around these types, but they MUST NOT add a second meaning
to one of the five operation names.

`close` is idempotent. The first call stops new admission, drains or completes
already admitted work according to the binding's normal async/sync lifetime,
and releases the transport. Later calls complete successfully. No public
cancellation handle or cancellation operation exists in v1; callers wait for
an accepted operation or close the client.

## 2. Deliberately unsupported features

The following are outside Gate 0 and MUST NOT be advertised as maintained v1
operations or defaults:

- `get_json`, `set_json`, JSON auto-detection, and legacy metadata envelopes;
- raw byte reads/writes, Exact Item ID reads/writes, and caller-owned v0
  envelopes;
- conditional writes, TTL/expiration options, eviction options, namespace
  creation/lookup/update/deletion, and other control-plane operations;
- experimental operations such as `EXPERIMENTAL_STATS` and
  `EXPERIMENTAL_SYNC`;
- caller-visible retry-policy, timeout, lane, concurrency, or cancellation
  controls;
- plaintext transport, transport-specific public operation variants, and
  protocol-version downgrade;
- certificate-file, custom trust-root, hostname-verification, or mTLS
  configuration; and
- caller-selected compression, value protection, value-key rotation, or
  payload-format selectors.

Implementations may retain internal compatibility code while migrating. That
code is not part of this public contract and MUST NOT be reachable through the
maintained five-operation facade. An unknown or legacy stored format is a
format error; it is never silently treated as JSON, raw bytes, or a structured
value.

## 3. Development connection profile

Gate 0 defines one development profile so examples and all maintained
bindings address the same server:

```text
transport       = TLS 1.3 over a supported v1 transport
ALPN            = openkache/1
server trust    = DevelopmentTrust (certificate verification disabled)
client identity = none (the server does not require a client certificate)
value profile   = StructuredValue-CBOR-v1
value selector  = payload-format 1, uncompressed, unprotected
key profile     = NamespaceHash with the shared development Item-ID root
namespace ID    = 1
Item-ID root    = 000102030405060708090a0b0c0d0e0f
                  101112131415161718191a1b1c1d1e1f
```

The server still presents a certificate and the TLS 1.3 handshake still
encrypts traffic. `DevelopmentTrust` deliberately disables client-side
certificate-chain and hostname verification, so it provides passive transport
confidentiality but no active man-in-the-middle protection. Every example that
uses it MUST say **development only — do not use this trust profile in
production**.

No plaintext fallback is permitted. Production certificate verification,
custom trust roots, and mutual TLS are follow-up profiles, not configuration
knobs hidden behind the Gate 0 API. A binding MUST reject attempts to select
those profiles rather than silently weakening or changing the connection.

The development key profile is also fixed: all maintained clients use the
documented public root and namespace setup so that a text key written by one
binding can be read by another. This root is a fixture value, not a
production secret. The root is an Item-ID identity setting; it is not a
value-protection key. Gate 0 values are unprotected inside the envelope, while
TLS protects them in transit.

## 4. Typed keys

The key contract is:

```text
TypedKey =
    Integer(signed i64)
  | Text(valid UTF-8)
  | Bytes(byte sequence)
```

`Integer` is exactly the signed 64-bit range. `Text` is length-delimited UTF-8
and may be empty or contain U+0000. `Bytes` preserves every byte, including
empty and zero bytes. The canonical CBOR encoding, key-size bound, and
`NamespaceHash` mapping are normative in [`KEY_FORMAT.md`](KEY_FORMAT.md).

Adapters MUST infer one unambiguous variant or require an explicit typed-key
constructor. They MUST reject booleans, floating-point values, null, arbitrary
objects, collections, invalid UTF-8, and integers outside signed `i64`.
Stringification, reflection, and lossy numeric coercion are forbidden.

Examples use text keys:

```text
get(Text("user:1"))
set(Text("user:1"), TextString("Ada"))
delete(Text("user:1"))
```

`Bytes` and signed integers remain distinct typed identities; a binding MUST
not stringify either one. There is no namespace-wide key-type setting.

## 5. Structured values

Every successful `get` and `set` uses `StructuredValue-CBOR-v1`. The envelope
payload-format ID is `1`; the complete selector byte also carries protection
and compression bits, which are fixed by the development profile and are not
application-level byte arguments. The profile is specified in
[`value/SPEC.md`](value/SPEC.md) and the envelope in
[`VALUE_FORMAT.md`](VALUE_FORMAT.md).

The complete model is:

```text
Value =
    Undefined
  | Null
  | Boolean(true | false)
  | Integer(arbitrary-precision signed integer)
  | Float16(raw IEEE-754 bits)
  | Float32(raw IEEE-754 bits)
  | Float64(raw IEEE-754 bits)
  | ByteString(bytes)
  | TextString(valid UTF-8)
  | Array<Value>
  | Map<(Value, Value)>[]
```

`Float16`, `Float32`, and `Float64` preserve width and raw bits in the
lossless model, including signed zero and NaN payloads. `Integer` is not a
floating-point value, even when its magnitude is small. `ByteString` is not
text. Arrays preserve order. Maps preserve entry order for lossless access,
but map order is not equality.

Map keys MUST be scalar model values (`Undefined`, `Null`, `Boolean`,
`Integer`, any `Float`, `ByteString`, or `TextString`). Arrays and maps are
not map keys. Duplicate keys are rejected using model equality:
`Integer(1)`, `Boolean(true)`, and `Float64(1.0)` are three different keys;
`+0.0` and `-0.0` are different; distinct NaN raw bits are different.

Cycles, functions, classes, arbitrary object graphs, and language-specific
collection identity are not model values. Callers must construct an explicit
model value or receive a local conversion error.

### 5.1 Lossless and native projections

`get` returns a lossless model wrapper (or the language's equivalent tagged
value) by default. Rust and C++ bindings expose the tagged `Value` variant;
Python and TypeScript expose wrappers that retain all kinds and map keys.
Lossless access MUST preserve arbitrary integers, float width/bits, bytes/text,
`Undefined`, and ordered map entries.

An explicit `to_native`/equivalent helper MAY project to ordinary language
containers. It MUST fail instead of rounding integers, normalizing float
distinctions, collapsing map keys, or dropping `Undefined`. Typical mappings
are:

| Model | Python | TypeScript/JavaScript |
|---|---|---|
| `Undefined` | conversion error | `undefined` |
| `Null` | `None` | `null` |
| `Boolean` | `bool` | `boolean` |
| `Integer` | arbitrary-precision `int` | `bigint` |
| `Float*` | documented float wrapper/`float` | documented float wrapper/`number` |
| `ByteString` | `bytes` | `Uint8Array` |
| `TextString` | `str` | `string` |
| `Array` | `list` | `Array` |
| `Map` | ordered model map | `Map` |

The native projection is explicit and strict; it never changes the wire
profile or the meaning of `get`.

## 6. Lookup and mutation outcomes

### 6.1 `GetResult`

The result of `get` is a tagged value, never a nullable sentinel:

```text
GetResult<T> = Missing | Found(T)
```

The four observable cases are distinct:

| Stored state | Result |
|---|---|
| no live item, expired item, deleted item, or evicted item | `Missing` |
| live value `Null` | `Found(Null)` |
| live value `Undefined` | `Found(Undefined)` |
| any other live model value | `Found(value)` |

Python MUST NOT use `None` as the missing marker. TypeScript MUST NOT use
JavaScript `undefined` as the missing marker. Rust and C++ MUST preserve the
tagged distinction in their result types. `Missing` has no value payload.

### 6.2 Set and delete

An unconditional `set` has exactly two successful outcomes:

```text
SetOutcome = Created | Replaced
DeleteOutcome = Deleted | NotFound
```

`Created` means no live value existed at the mutation point. `Replaced` means
one live value was replaced. `Deleted` means a live value was removed.
`NotFound` means delete made no change. `NotStored` is a conditional-server
outcome and is not reachable through the unconditional Gate 0 API; a server
that returns it to a Gate 0 request is incompatible and the client MUST
surface a stable contract error.

If a transport failure occurs after a mutation may have been admitted but
before its response arrives, the client MUST return an explicit
`UnknownMutation` error category. It MUST NOT turn that result into
`NotFound`, `Created`, `Replaced`, or an automatically replayed mutation.
Read-only `get` failures may be retried internally when safe, but retry
controls are not public API and no mutation is automatically replayed.

Other stable errors include local invalid-key/value errors, unsupported or
malformed value-format errors, TLS/connection errors, resource-limit errors,
and server statuses such as `InvalidRequest`, `TooLarge`, `Forbidden`,
`NoCapacity`, `NamespaceNotFound`, or `InternalError`. Bindings map them to
idiomatic errors while preserving the category and any unknown-mutation
information.

## 7. Language projections

The operation names are stable even when syntax differs:

| Language family | `connect` | `get` result | `set`/`delete` | `close` |
|---|---|---|---|---|
| Rust | async constructor | tagged `GetResult<Value>` | `SetOutcome`/`DeleteOutcome` | async idempotent |
| Python | coroutine/factory | tagged wrapper result | enum/string outcome | coroutine idempotent |
| TypeScript | promise factory | tagged discriminated union | discriminated outcomes | promise idempotent |
| C++ | RAII/synchronous convenience | tagged `Value` result | `SetOutcome`/`DeleteOutcome` | idempotent RAII close |

These are projections, not separate semantics. A binding MUST document its
concrete names, ownership, and runtime behavior without adding unsupported
operations or changing outcome distinctions.

## 8. Conformance and fixtures

The canonical machine-readable vectors live in
[`fixtures/`](fixtures/). They are part of the public contract, not tests:

- `client_contract_v1.json` — operation, lookup, mutation, trust-profile, and
  unsupported-feature vectors;
- `structured_value_cbor_v1.json` — every model kind, scalar-key distinction,
  and malformed-value rejection;
- `key_format_v1.json` — typed-key inference, canonical bytes, and mapping
  boundaries;
- `value_format_v1.json` — the fixed Gate 0 envelope selector and rejection
  of unsupported transforms; and
- `protocol_v1.json` — the stable GET/SET/DELETE response and framing
  boundaries owned by the wire protocol.

Each vector declares `spec_revision = "v1-gate0"`. A binding claiming Gate 0
compatibility MUST agree with the documents and fixtures semantically; a
decode/re-encode operation need not reproduce map-order choices byte-for-byte.

No tests, benchmarks, private CI, or development infrastructure belong in the
public submodule. Cross-language tests consume these public vectors from the
private monorepo.
