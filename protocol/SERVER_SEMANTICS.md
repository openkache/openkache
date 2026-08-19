# OpenKache Server Semantics (Draft)

> **Status:** Draft `draft-2026-08-19.3`. This document defines server behavior that is required
> for a useful implementation but is not additional request/response framing.

The stable wire grammar remains in [`SPEC.md`](SPEC.md). This document owns
expiration recovery and eviction behavior that requires server state.
Namespace lifecycle is a separate [WIP draft](NAMESPACE.md).

## Namespace identity

Stable v1 consumes server-assigned namespace IDs. The server owns allocation and
discovery; clients use the ID supplied to them. Namespace policy is immutable
for the lifetime of a namespace.

## TTL persistence and recovery

The `SET` mutation linearization point determines the expiration deadline:

```text
deadline = mutation_linearization_time + ttl_ms
```

At runtime the server MUST use a monotonic deadline and treat an item as
expired when `now >= deadline`. Expired items are logically absent for `GET`,
`DELETE`, conditional `SET`, and namespace live-item counting even if physical
cleanup is deferred.

Persistence MUST retain enough information to reconstruct the deadline without
extending the item on restart. The recommended representation is an absolute
server-clock expiration timestamp plus a reconstructed monotonic runtime
deadline. A server MUST document its clock source, rollback/forward behavior,
VM suspend behavior, and restore behavior.

Snapshot or replica restore MUST choose one explicit policy:

1. preserve the original server clock domain and deadline;
2. recompute from a documented remaining-duration representation; or
3. expire items whose deadline cannot be trusted.

It MUST NOT silently extend TTLs because a snapshot was restored.

## Eviction

The namespace eviction algorithm is implementation-defined (for example, LRU
or LFU), but it may select only items whose resolved eviction mode is
`Evictable`. `EvictionProtected` items remain protected from capacity eviction,
but may still expire, be explicitly deleted, or be replaced.

If a write cannot be admitted without selecting a protected item, the server
returns `NoCapacity` and makes no mutation. Because namespace policy is
immutable in v1, existing items and future writes use the same namespace
defaults.

The administration surface and its metadata are outside stable v1. They must
not expose or replace client-owned key profiles.
