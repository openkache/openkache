# OpenKache Namespace Lifecycle (WIP Draft)

> **Status:** Namespace lifecycle/management is work in progress. It is not part
> of stable protocol v1, has no assigned stable opcodes, and is not an
> implementation requirement. Stable data operations still require the
> server-assigned namespace ID and namespace-policy semantics defined by
> `SPEC.md`.

Stable v1 data operations carry a server-assigned `namespace_id`. This document
preserves the proposed lifecycle design for later revision.

The Smithy model currently retains `NamespaceOpen`,
`NamespaceUpdatePolicy`, and `NamespaceDelete` as `outOfBand` operations so
private or control-plane adapters can describe the draft shapes. They do not
reserve stable v1 opcodes or statuses. The current Rust server keeps
compatibility registrations for these shapes so legacy/control-plane callers
can reach them on the data lane, but that route is transitional and does not
constitute stable-v1 conformance. Their transitional error lists and response
names must not be used to validate a stable v1 frame. A future namespace API
must first receive an explicit assignment in [`SPEC.md`](SPEC.md) before the
Smithy operations can become stable wire-visible. The model's
`NamespaceDescriptor.revision` and
`NamespaceUpdatePolicy.expectedRevision` are legacy optimistic-concurrency
fields for that out-of-band shape; they do not revise the proposed immutable
policy or namespace-replacement rules below.

## Open questions

- Define the server interface that returns a namespace ID and immutable policy.
- Assign wire operations only after lifecycle and authorization rules settle.

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
In particular, the Smithy `NamespaceUpdatePolicy` shape is a WIP control-plane
proposal. Its revision fields are non-normative until a future lifecycle
proposal resolves whether policy changes replace a namespace, and it does not
change that stable-v1 rule.
