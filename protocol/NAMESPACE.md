# OpenKache Namespace Lifecycle (WIP Draft)

> **Status:** Work in progress. This feature is not part of stable protocol v1,
> has no assigned stable opcodes, and is not an implementation requirement.

Stable v1 data operations carry a provisioned `namespace_id`. This document
preserves the proposed lifecycle design for later revision.

## Open questions

- Define an `identity_domain_id` that lets clients distinguish independent
  deployments, restores, and allocator histories.
- Decide whether it is returned with every namespace descriptor, discovered
  once per deployment, or both.
- Define namespace discovery and profile-mismatch handling without making
  client-owned key profiles visible to the server.
- Assign wire operations only after those identity rules are settled.

## Proposed identity rules

A namespace has a server-assigned, nonzero `namespace_id` that is stable within
one identity domain. An ID is never reused for another namespace in that
domain. Recreating a deleted name creates a new namespace identity.

Names are `0..=255` bytes of valid UTF-8, compared by exact bytes without case
folding or Unicode normalization. The empty name is valid.

## Proposed lifecycle

The draft operations are:

- **Open:** resolve a name and optionally create it with an initial policy.
- **Update policy:** replace policy when `expected_revision` matches.
- **Delete:** remove the namespace only when its revision matches and it has no
  live items.

A namespace starts at revision `1`; each successful policy update increments
the revision without wrapping. Lifecycle changes for one name are serialized.
Data mutations racing with deletion linearize either before the empty check or
after deletion.

The proposed descriptor contains:

```text
identity_domain_id | namespace_id | revision | namespace_policy
```

The representation and size of `identity_domain_id` remain TODO.

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

This section records design intent only. It does not reserve opcodes, statuses,
or a finalized frame layout.
