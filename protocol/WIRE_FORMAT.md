# OpenKache wire format

This is the short entry point for the transport-neutral byte contract. The
normative sections remain in [`SPEC.md`](SPEC.md) until the v1 draft is frozen.

- [Unsigned `vu128`](SPEC.md#unsigned-vu128)
- [Common limits](SPEC.md#common-limits)
- [Request frames](SPEC.md#request-frames)
- [Response frames](SPEC.md#response-frames)
- [Validation and malformed frames](SPEC.md#validation-and-malformed-frames)

The wire format is independent of whether a connection uses QUIC or
TLS-over-TCP. Values are opaque bytes; key mapping and value envelopes are
client contracts.
