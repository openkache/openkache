# OpenKache Server Semantics (Draft)

> **Status:** Draft. This document defines server behavior that is required
> for a useful implementation but is not additional request/response framing.

The stable wire grammar and public operation layouts remain in
[`SPEC.md`](SPEC.md). This document owns namespace identity-domain handling,
expiration recovery, and eviction behavior that require server or operator
state.

## Namespace identity domain

`namespace_id` is stable only within one deployment identity domain. A server
MUST NOT reuse an ID for a different namespace after deletion, restart,
recovery, or replica replacement within that domain. Durable allocator state
and snapshots MUST preserve this rule.

An operator restoring an independent snapshot fork MUST establish a new
identity domain rather than silently merging allocator history. The restore
procedure MUST document whether namespace IDs, namespace revisions, and item
identity are preserved or intentionally remapped.

Recreating a deleted namespace name creates a new namespace identity. A client
MUST NOT assume that the old ID or old client-side key profile addresses the
new namespace.

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
deployment-clock expiration timestamp plus a reconstructed monotonic runtime
deadline. A server MUST document its clock source, rollback/forward behavior,
VM suspend behavior, and restore behavior.

Snapshot or replica restore MUST choose one explicit policy:

1. preserve the original deployment clock domain and deadline;
2. recompute from a documented remaining-duration representation; or
3. expire items whose deadline cannot be trusted.

It MUST NOT silently extend TTLs because a snapshot was restored into a new
deployment.

## Eviction

The namespace eviction algorithm is implementation-defined (for example, LRU
or LFU), but it may select only items whose resolved eviction mode is
`Evictable`. `EvictionProtected` items remain protected from capacity eviction,
but may still expire, be explicitly deleted, or be replaced.

If a write cannot be admitted without selecting a protected item, the server
returns `NoCapacity` and makes no mutation. Namespace policy changes affect
future writes; existing items retain the policy resolved at their own `SET`
linearization point.

## Operational contract

The server implementation MUST expose its chosen identity-domain, clock, and
restore policies to operators. Those policies are not client-owned key
profiles and are not interpreted by the wire protocol.
