# OpenKache Experimental Protocol Operations (Draft)

> **Status:** Experimental. These operations are not part of the stable v1
> conformance surface. Their names, layouts, status behavior, and semantics
> may change or disappear without a protocol-version change.

This document defines optional operations for benchmark, internal, and other
non-production tooling. A production client MUST NOT depend on an
`EXPERIMENTAL_*` operation.

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
of relying on a pending-write memory buffer. This is a benchmark and
maintenance visibility barrier, not a public durability-level negotiation API.

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
| Barrier failure or transport failure | lane close; outcome unknown |

The operation is not a server capability requirement. A server that does not
expose it MAY treat opcode `06` as an unassigned experimental operation and
close the lane without a response. Such behavior does not make the server
non-conforming to stable protocol v1.
