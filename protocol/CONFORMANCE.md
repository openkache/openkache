# OpenKache protocol conformance

Use this checklist when reviewing a v1 implementation:

- [Wire format](WIRE_FORMAT.md) and canonical `vu128`
- At least one transport profile; maintained implementations support both
- Stable `PING`, `GET`, `SET`, and `DELETE`
- `0..=32` byte Item IDs and opaque values
- Immutable namespace policy for the lifetime of a namespace
- Bounded incremental parsing and terminal malformed-frame handling
- Explicit unknown mutation outcomes
- [Public protocol fixtures](../clients/fixtures/README.md)

The detailed checklist and normative examples remain in
[`SPEC.md`](SPEC.md#implementation-conformance-checklist) during the draft.
