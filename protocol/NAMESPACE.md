# OpenKache Namespace Lifecycle (WIP Draft)

> **Status:** Work in progress. This feature is not part of stable protocol v1,
> has no assigned stable opcodes, and is not an implementation requirement.

Stable v1 data operations carry a provisioned `namespace_id`. This document
preserves the proposed lifecycle design for later revision.

## Open questions

- Define namespace discovery and profile-mismatch handling without making
  client-owned key profiles visible to the server.
- Assign wire operations only after those identity rules are settled.

## Proposed identity rules

A namespace has a server-assigned, nonzero `namespace_id`. Clients use the
identifier supplied by the server; they do not allocate or derive it. Namespace
ID allocation and lifecycle policy are server responsibilities.

Names are `0..=255` bytes of valid UTF-8, compared by exact bytes without case
folding or Unicode normalization. The empty name is valid.

## Proposed lifecycle

The future lifecycle may include an **Open** operation that resolves a name and
optionally creates it with an initial policy, and a **Delete** operation that
removes a namespace when it has no live items. Policy updates are not part of
the v1 design because namespace policy is immutable. Any future policy change
should create a new namespace.

Lifecycle changes for one name are serialized. Data mutations racing with
deletion linearize either before the empty check or after deletion.

The proposed descriptor contains:

```text
namespace_id | namespace_policy
```

The representation of `namespace_id` is the fixed eight-byte field used by v1.

## Proposed policy encoding

```text
namespace_policy = policy_flags:u8 | [default_ttl_ms:vu128]
```

| Bits | Meaning |
|---:|---|
| `0..1` | `00` = `NoExpiry`; `01` = `FixedTtl`; other values invalid |
| `2` | expiration override allowed |
| `3` | default eviction is `EvictionProtected` |
| `4` | eviction override allowed |
| `5..7` | zero |

`default_ttl_ms` is present only for `FixedTtl` and must be positive.

Namespace policy is immutable for the lifetime of a namespace in v1. A policy
change, if supported later, creates a new namespace identity rather than
changing the meaning of existing items. This section records design intent
only; it does not reserve opcodes, statuses, or a finalized frame layout.
