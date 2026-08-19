# OpenKache Experimental Protocol Operations (Draft)

> **Status:** Experimental revision `draft-2026-08-19.4`. These operations are not part of the stable v1
> conformance surface. Their names, layouts, status behavior, and semantics
> may change or disappear without a protocol-version change.

This document defines optional diagnostic and maintenance operations. A server
recognizes them only when its `enable_experimental_api` server setting is
enabled. Otherwise their opcodes are unassigned and therefore malformed under
the stable protocol. An unaware server closes the connection without a
response.

Experimental layouts are not a compatibility surface. Clients MUST enable
their use explicitly and MUST match the server's documented experimental
revision.

## `STATS`

Opcode `05` currently carries:

```text
05 | request_id:vu128 | namespace_id:u64be
```

An authorized success returns `Ok` with an implementation-defined diagnostic
payload. Unauthorized requests may return `Forbidden`. An authorized request
for a missing namespace returns `NamespaceNotFound`.

The payload is operator-facing and not a stable programmatic schema. It MUST
fit the response payload limit.

## `EXPERIMENTAL_SYNC`

Opcode `06` currently carries:

```text
06 | request_id:vu128 | namespace_id:u64be
```

The operation is a namespace-wide storage visibility barrier. Its
linearization point is where the namespace operation sequence admits the
barrier. Mutations to that namespace that linearized before that point are
covered; later mutations are not required to be included.

A successful response is sent only after all covered pending writes have been
sent to disk. A later read MUST be able to use durable storage state instead
of relying on a pending-write memory buffer. This is a maintenance visibility
barrier, not a public durability-level negotiation API.

An implementation MAY perform authorization before namespace lookup. An
unauthorized request may therefore receive `Forbidden` without revealing
whether the namespace exists. An authorized request for a missing namespace
receives `NamespaceNotFound`.

The current response behavior is:

| Condition | Response |
|---|---|
| Authorized barrier completed | `Ok` with an empty payload |
| Unauthorized | `Forbidden` with an optional diagnostic |
| Authorized namespace missing | `NamespaceNotFound` |
| Barrier failure or transport failure | connection close; outcome unknown |

Neither operation is a stable server capability requirement.
