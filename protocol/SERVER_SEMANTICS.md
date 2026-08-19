# OpenKache Server Semantics (Draft)

> **Status:** Draft `draft-2026-08-19.2`. This document defines server behavior that is required
> for a useful implementation but is not additional request/response framing.

The stable wire grammar remains in [`SPEC.md`](SPEC.md). This document owns
expiration recovery and eviction behavior that requires server state.
Namespace lifecycle is a separate [WIP draft](NAMESPACE.md).

## Namespace identity domain

Stable v1 consumes provisioned namespace IDs but does not define their
lifecycle. `identity_domain_id`, restore behavior, and client-visible identity
discovery remain TODOs in [`NAMESPACE.md`](NAMESPACE.md).

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

## Operational metadata

The future administration interface should expose:

- the namespace identity-domain ID;
- the clock-domain ID;
- the restore policy; and
- the namespace allocator epoch.

This is a pre-freeze requirement, not yet a conforming API: the administration
surface and field encodings remain undefined. These identifiers must not
expose or replace client-owned key profiles.
