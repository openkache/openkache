# OpenKache Server Semantics (Draft)

> **Status:** Draft `draft-2026-08-19.4`; not released or finalized.

The stable wire grammar remains in [`SPEC.md`](SPEC.md). Runtime expiration and
eviction rules apply to every server. Recovery rules apply only to the
Persistent TTL conformance profile. Namespace lifecycle is a separate
[WIP draft](NAMESPACE.md).

## Namespace identity

Stable v1 consumes server-assigned namespace IDs. Clients use the ID supplied
through an interface outside stable v1. Namespace policy is immutable for the
lifetime of a namespace.

## TTL persistence and recovery

The `SET` mutation linearization point determines the expiration deadline:

```text
deadline = mutation_linearization_time + ttl_ms
```

At runtime the server MUST use a monotonic deadline and treat an item as
expired when `now >= deadline`. Expired items are logically absent for `GET`,
`DELETE`, conditional `SET`, and namespace live-item counting even if physical
cleanup is deferred.

The Persistent TTL conformance profile MUST retain either an absolute
expiration timestamp or a remaining duration paired with a trustworthy
reference timestamp. The server reconstructs a monotonic runtime deadline
without extending the item and MUST document its clock source,
rollback/forward behavior, VM suspend behavior, and restore behavior.

OpenKache stores expiration deadlines as Unix-epoch milliseconds and captures
the wall-clock reference in the `OKCPV1` checkpoint. During one process,
deadline comparisons use a monotonic `Instant` anchored to that reference.
On restart, a wall clock earlier than the checkpoint reference is treated as
untrusted and startup fails closed; this prevents a restart rollback from
extending every persisted TTL. A forward jump at restart is accepted and may
expire entries early. The monotonic clock's suspend behavior is
platform-defined; operators must restart/revalidate after a suspend or restore
when elapsed time cannot be trusted, and the restart check follows the same
fail-closed policy rather than extending a deadline. The v1 checkpoint carries
this reference; a checkpoint with a different version is rejected instead of
replayed.

Snapshot or replica restore MUST choose one explicit policy:

1. preserve the original server clock domain and deadline;
2. subtract trustworthy elapsed time from a stored remaining duration; or
3. expire items when elapsed time or the original deadline cannot be trusted.

A stored remaining duration MUST NOT restart from its snapshot-time value.
Restart or restore MUST NOT silently extend a TTL.

## Eviction

The namespace eviction algorithm is implementation-defined (for example, LRU
or LFU), but it MUST select only items whose resolved eviction mode is
`Evictable`. `EvictionProtected` items remain protected from capacity eviction,
but may still expire, be explicitly deleted, or be replaced.

If a write cannot be admitted without selecting a protected item, the server
returns `NoCapacity` and makes no mutation. Because namespace policy is
immutable in v1, existing items and future writes use the same namespace
defaults.

The administration surface and its metadata are outside stable v1. They must
not expose or replace client-owned key profiles.
